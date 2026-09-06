// SPDX-License-Identifier: Apache-2.0
//! The week closes: the log becomes a record.
//!
//! Design 0007 C2 and design 0009 M3, plan 0006 item 6. Once a week, at Monday
//! 00:00 UTC, the replies the account posted in the closed week are scored on
//! their public metrics, each entrant's standing is established, the published
//! rule ranks them, and the result is written as `<week>.json` where the public
//! leaderboard reads it. The hunter tally is written beside it.
//!
//! # What is pure and what is not
//!
//! [`close`] is pure: it takes the log, the refusals, the previous records, the
//! metrics and the account ages as arguments and returns the record. Every
//! network read happens in [`close_if_due`], which is a few calls and a file
//! write around it, so the rule can be replayed from the log and a disagreement
//! about a week can be settled by re-running the function on the same inputs.
//!
//! # Rule 9, three times
//!
//! A reply the platform returned no metrics for is [`Excluded::Unscored`], not
//! scored zero. An account whose creation date could not be read has no age
//! and is excluded as unknown, never as old enough. A summoner never refused
//! this week is one whose name is absent from the refusals file -- which is
//! why refusals are written down at all: an absence in a file that exists is
//! evidence, and an absence in a file nobody writes is not.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;

use radar_contest::hunter::{Placing, Sighting, tally};
use radar_contest::{
    Entry as ContestEntry, Excluded, Metrics, Record, Rules, Standing, Week, rank,
};
use serde::{Deserialize, Serialize};

use crate::daemon::Paths;
use crate::log::Entry;
use crate::x::{Mention, X};

/// One refusal by the X gate, as the week-close job needs it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefusalLine {
    /// When, as seconds since the epoch.
    pub at: u64,
    /// Who was refused.
    pub summoner: String,
    /// Why, in the gate's words.
    pub why: String,
}

/// Appends one refusal.
///
/// # Errors
///
/// The underlying I/O error. The caller logs it and carries on: a refusal that
/// could not be written must not stop the reply loop.
pub fn append_refusal(path: &str, line: &RefusalLine) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let mut text = serde_json::to_string(line).map_err(std::io::Error::other)?;
    text.push('\n');
    file.write_all(text.as_bytes())
}

/// Every refusal on record. A missing file is no refusals.
#[must_use]
pub fn read_refusals(path: &str) -> Vec<RefusalLine> {
    std::fs::read_to_string(path)
        .map(|text| {
            text.lines()
                .filter(|l| !l.trim().is_empty())
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// The published X replies posted in `week`, with a mint: the week's entries.
///
/// Published means a reply id exists. A dry-run answer and a reply that failed
/// to post are in the log and are not public statements, so they are not
/// entries (design 0007 §6.2: the score reads the reply's public metrics, and
/// an unposted reply has none).
#[must_use]
pub fn entries_in(log: &[Entry], week: Week) -> Vec<&Entry> {
    log.iter()
        .filter(|e| week.contains(e.at) && e.reply_id.is_some() && e.mint.is_some())
        .collect()
}

/// Everything the rule needs, established by the caller.
#[derive(Clone, Debug)]
pub struct Inputs<'a> {
    /// The week being closed.
    pub week: Week,
    /// The reply log, folded.
    pub log: &'a [Entry],
    /// Every refusal on record.
    pub refusals: &'a [RefusalLine],
    /// Every earlier week's record, for the cooldown.
    pub previous: &'a [Record],
    /// Public metrics by reply id, as read at close.
    pub metrics: &'a BTreeMap<String, Metrics>,
    /// What each entrant's account says about itself, as read at close.
    ///
    /// Was a map of creation times alone. It carries the handle now too,
    /// because both arrive from one `/2/users` call and printing a numeric id
    /// on the leaderboard as though it were a handle was finding S4.
    pub accounts: &'a BTreeMap<String, crate::x::Account>,
    /// The published rule.
    pub rules: &'a Rules,
}

/// Closes a week. Pure.
#[must_use]
pub fn close(inputs: &Inputs<'_>) -> Record {
    let week = inputs.week;
    let closes_at = week.closes_at();
    let mut scored = Vec::new();
    let mut unscored = Vec::new();
    for e in entries_in(inputs.log, week) {
        let (Some(reply_id), Some(mint)) = (&e.reply_id, &e.mint) else {
            continue;
        };
        let entry = |metrics| ContestEntry {
            reply_id: reply_id.clone(),
            summoner: e.summoner.clone(),
            // Absent when the account lookup did not return this entrant, and
            // absent is rendered as the id-based link rather than as a blank
            // name.
            handle: inputs
                .accounts
                .get(&e.summoner)
                .and_then(|a| a.username.clone()),
            mint: mint.clone(),
            at: e.at,
            metrics,
        };
        match inputs.metrics.get(reply_id) {
            Some(m) => scored.push(entry(*m)),
            None => unscored.push(entry(Metrics::default())),
        }
    }

    let summoners: BTreeSet<&str> = scored.iter().map(|e| e.summoner.as_str()).collect();
    let standings: BTreeMap<String, Standing> = summoners
        .into_iter()
        .map(|s| {
            let standing = Standing {
                // Whole days at close; an account made this week is 0 days old.
                account_age_days: inputs.accounts.get(s).map(|a| {
                    u32::try_from(closes_at.saturating_sub(a.created_at) / 86_400)
                        .unwrap_or(u32::MAX)
                }),
                refused_this_week: inputs
                    .refusals
                    .iter()
                    .any(|r| r.summoner == s && week.contains(r.at)),
                last_win: inputs
                    .previous
                    .iter()
                    .filter(|r| r.winner.as_ref().is_some_and(|w| w.summoner == s))
                    .map(|r| r.week)
                    .max(),
            };
            (s.to_owned(), standing)
        })
        .collect();

    let mut ranking = rank(week, &scored, &standings, inputs.rules);
    ranking
        .excluded
        .extend(unscored.into_iter().map(|e| (e, Excluded::Unscored)));
    Record::close(week, ranking)
}

/// The week's sightings for the hunter rank: every published X reply whose
/// sheet was counted, with its signal count. A reply from before the count
/// existed has `None` and is not a sighting -- unknown, not zero.
#[must_use]
pub fn sightings(log: &[Entry], week: Week) -> Vec<Sighting> {
    entries_in(log, week)
        .into_iter()
        .filter_map(|e| {
            let signals = e.signals.as_ref()?;
            Some(Sighting {
                summoner: e.summoner.clone(),
                at: e.at,
                signals: u32::try_from(signals.len()).unwrap_or(u32::MAX),
            })
        })
        .collect()
}

/// Reads a winner's claim out of a mention, if that is what it is.
///
/// Design 0007 C3. A claim is a public reply from the winning account, within
/// the claim window, naming an address -- parsed with the same strict rule as
/// a summons, so there is no field an instruction can travel in. When the
/// mention is one, the claim is written into the week's record and the week
/// is returned; the caller then does not answer the mention as a summons,
/// because a wallet address is not a coin.
///
/// The first claim stands. A second address from the same winner in the same
/// window is a summons like any other, and is answered as one -- which is a
/// visible "no record" rather than a silent second claim.
#[must_use]
pub fn try_claim(mention: &Mention, contest_dir: &str, now: u64) -> Option<Week> {
    // A claim is a reply to the account's own claim prompt. Nothing else is a
    // claim, however it is worded and whatever address it contains.
    //
    // Before that rule existed, `try_claim` accepted any mint-shaped string in
    // any mention by the winner inside their claim window -- and a coin's mint
    // address is exactly that shape, 32 bytes of base58, indistinguishable from
    // a wallet by looking at it. A winner who summoned the bot about a coin
    // during their own claim week had that coin's mint written in as their
    // payout address, and `Payout::permitted` would have approved the transfer,
    // because the recipient really did equal the claim. The prize would have
    // gone to a mint account nobody controls.
    //
    // **The guarantee is the `claim_prompt` equality in the predicate below**,
    // not this line. Established by re-applying each separately: deleting that
    // condition fails two tests by name; weakening this `?` to an empty default
    // fails nothing, because an empty parent still does not equal a prompt id.
    //
    // This line is a cheap early return and is honest about being one. A
    // mention that replies to nothing cannot be a claim, and that is most
    // mentions -- so it saves the directory listing below on the common path,
    // which is finding S5.
    let parent = mention.parent.as_deref()?;

    let crate::mention::Asked::Mint(address) = crate::mention::read(&mention.text) else {
        return None;
    };
    if address.parse::<radar_types::Address>().is_err() {
        return None;
    }
    let mut record = radar_contest::records_in(std::path::Path::new(contest_dir))
        .into_iter()
        .find(|r| {
            // `claim_prompt` must be present AND must be what was replied to.
            //
            // Deliberately strict. Design 0007 §6.2 also allows a reply under
            // the *winning reply* to count when the prompt failed to post, and
            // that is rejected here: a winner replying "now do this one" with a
            // new coin under their own winning reply is an ordinary summons,
            // and treating it as a claim would reopen the exact hole this
            // function was fixed to close.
            //
            // The cost of strictness is a week whose prompt never posted being
            // unclaimable, and that is bounded and recoverable -- the daemon
            // retries the prompt on every tick while the window is open, and an
            // unclaimed pool rolls into the next week rather than being lost. A
            // prize paid to a mint account is not recoverable at all.
            r.claim.is_none()
                && r.accepts_claim_at(now)
                && r.claim_prompt.as_deref() == Some(parent)
                && r.winner
                    .as_ref()
                    .is_some_and(|w| w.summoner == mention.author)
        })?;
    record.claim = Some(radar_contest::Claim {
        address,
        reply_id: mention.id.clone(),
        at: now,
    });
    let path = record_path(contest_dir, record.week);
    let text = record.to_json().ok()?;
    if let Err(e) = write_atomically(&path, &text) {
        eprintln!("radar-analyst: cannot write the claim to {path}: {e}");
        return None;
    }
    Some(record.week)
}

/// The week that closed most recently before `now`, if any has.
///
/// The current week is open; the one before it is the one to close. Week zero
/// has no predecessor.
#[must_use]
pub fn due(now: u64) -> Option<Week> {
    Week::of(now).0.checked_sub(1).map(Week)
}

/// Where a week's record lives.
#[must_use]
pub fn record_path(contest_dir: &str, week: Week) -> String {
    format!("{contest_dir}/{}.json", week.0)
}

/// Where a week's hunter tally lives.
#[must_use]
pub fn hunter_path(contest_dir: &str, week: Week) -> String {
    format!("{contest_dir}/hunter-{}.json", week.0)
}

/// Closes the most recent week if its record does not exist yet.
///
/// Called on every tick; almost every call returns `None` at the cost of one
/// `exists`. When a week is due:
///
/// - with no entries, the record is written without reading the platform --
///   an empty week is a fact about the week, and it must be on disk so the
///   leaderboard can say "no winner" rather than "still open";
/// - with entries and no client, nothing is written and the reason is
///   printed; the next tick tries again. A record scored from nothing would
///   exclude every entrant as unscored and crown nobody, and that is a
///   decision, not a failure to read;
/// - with entries and a client, the metrics and the account ages are read,
///   the record and the hunter tally are written, and the record is returned
///   so the caller can post the week.
///
/// # Errors
///
/// The I/O error from writing the record. The platform's refusals are
/// printed and swallowed here, because the retry is the next tick.
pub fn close_if_due(
    x: Option<&X>,
    paths: &Paths,
    now: u64,
    rules: &Rules,
    per_summoner_daily: u32,
) -> std::io::Result<Option<Record>> {
    let Some(week) = due(now) else {
        return Ok(None);
    };
    let path = record_path(&paths.contest_dir, week);
    if std::path::Path::new(&path).exists() {
        return Ok(None);
    }
    let log = crate::log::latest(&paths.log).unwrap_or_default();
    let entries = entries_in(&log, week);

    let (metrics, accounts) = if entries.is_empty() {
        (BTreeMap::new(), BTreeMap::new())
    } else {
        let Some(x) = x else {
            eprintln!(
                "radar-analyst: week {} has {} entries and no X client to score them; not closing",
                week.0,
                entries.len()
            );
            return Ok(None);
        };
        let ids: Vec<String> = entries.iter().filter_map(|e| e.reply_id.clone()).collect();
        let summoners: BTreeSet<String> = entries.iter().map(|e| e.summoner.clone()).collect();
        let metrics = match x.metrics(&ids) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("radar-analyst: cannot read the week's metrics: {e}; not closing");
                return Ok(None);
            }
        };
        let accounts = match x.accounts(&summoners.into_iter().collect::<Vec<_>>()) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("radar-analyst: cannot read the entrants' accounts: {e}; not closing");
                return Ok(None);
            }
        };
        (metrics, accounts)
    };

    let refusals = read_refusals(&paths.refusals);
    let previous = radar_contest::records_in(std::path::Path::new(&paths.contest_dir));
    let record = close(&Inputs {
        week,
        log: &log,
        refusals: &refusals,
        previous: &previous,
        metrics: &metrics,
        accounts: &accounts,
        rules,
    });

    std::fs::create_dir_all(&paths.contest_dir)?;
    let placings: Vec<Placing> = tally(&sightings(&log, week), per_summoner_daily);
    write_atomically(
        &hunter_path(&paths.contest_dir, week),
        &serde_json::to_string_pretty(&placings).map_err(std::io::Error::other)?,
    )?;
    write_atomically(&path, &record.to_json().map_err(std::io::Error::other)?)?;
    Ok(Some(record))
}

/// Writes one week's record back, atomically.
///
/// Used after the record has already been written once and something was
/// learned since -- today only the claim prompt's id, which cannot be known
/// until the post lands.
///
/// # Errors
///
/// The underlying I/O error, or a record that will not serialise.
pub fn write_record(contest_dir: &str, record: &Record) -> std::io::Result<()> {
    let text = record.to_json().map_err(std::io::Error::other)?;
    write_atomically(&record_path(contest_dir, record.week), &text)
}

/// The first week whose winner still has no claim prompt posted.
///
/// Retried on every tick rather than only at close, and that matters because
/// [`try_claim`] is strict: with no prompt on the record, no claim can be made
/// at all. A prompt that failed to post once -- a refusal, a 5xx, a budget
/// spent -- would otherwise lock the winner out for the whole window. The
/// window itself bounds the retrying.
#[must_use]
pub fn prompt_due(contest_dir: &str, now: u64) -> Option<Record> {
    radar_contest::records_in(std::path::Path::new(contest_dir))
        .into_iter()
        .find(|r| r.winner.is_some() && r.claim_prompt.is_none() && r.accepts_claim_at(now))
}

/// Writes via a sibling and a rename, so a reader never sees half a record.
fn write_atomically(path: &str, text: &str) -> std::io::Result<()> {
    let tmp = format!("{path}.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WEEK: Week = Week(2957);

    fn entry(
        id: &str,
        summoner: &str,
        at: u64,
        reply: Option<&str>,
        signals: Option<usize>,
    ) -> Entry {
        Entry {
            at,
            mention_id: format!("m-{id}"),
            summoner: summoner.to_owned(),
            mint: Some("MintOne".to_owned()),
            read_at_slot: Some(1),
            fact_sheet: String::new(),
            reply: "measured".to_owned(),
            fellback: None,
            reply_id: reply.map(str::to_owned),
            signals: signals.map(|n| vec![radar_roast::sheet::Signal::CreatorBoughtOwnLaunch; n]),
        }
    }

    fn created(days_before_close: u64) -> u64 {
        WEEK.closes_at() - days_before_close * 86_400
    }

    /// An account of the given age, with no handle.
    ///
    /// No handle by default because that is the case the leaderboard has to
    /// keep rendering: the lookup can return a user without one, and the page
    /// falls back to the id-based link rather than printing a blank name.
    fn account(days_before_close: u64) -> crate::x::Account {
        crate::x::Account {
            created_at: created(days_before_close),
            username: None,
        }
    }

    #[test]
    fn only_published_replies_with_a_mint_in_the_week_are_entries() {
        let open = WEEK.opens_at();
        let log = vec![
            entry("a", "alice", open + 10, Some("ra"), None),
            entry("b", "bob", open + 20, None, None), // dry run: not public
            entry("c", "carol", WEEK.closes_at(), Some("rc"), None), // next week
            entry("d", "dan", open - 1, Some("rd"), None), // last week
        ];
        let mut no_mint = entry("e", "eve", open + 30, Some("re"), None);
        no_mint.mint = None;
        let log = [log, vec![no_mint]].concat();
        let ids: Vec<&str> = entries_in(&log, WEEK)
            .iter()
            .filter_map(|e| e.reply_id.as_deref())
            .collect();
        assert_eq!(ids, ["ra"]);
    }

    #[test]
    fn the_week_closes_from_its_inputs_and_an_unscored_reply_is_excluded_not_zero() {
        // alice: scored, old account. bob: no metrics returned. carol: refused
        // during the week. dan: won last week. eve: account age unreadable.
        let open = WEEK.opens_at();
        let log = vec![
            entry("a", "alice", open + 10, Some("ra"), Some(2)),
            entry("b", "bob", open + 20, Some("rb"), Some(0)),
            entry("c", "carol", open + 30, Some("rc"), None),
            entry("d", "dan", open + 40, Some("rd"), Some(1)),
            entry("e", "eve", open + 50, Some("re"), Some(3)),
        ];
        let big = Metrics {
            reposts: 10,
            ..Metrics::default()
        };
        let small = Metrics {
            likes: 1,
            ..Metrics::default()
        };
        let metrics: BTreeMap<String, Metrics> = [
            ("ra".to_owned(), small),
            ("rc".to_owned(), big),
            ("rd".to_owned(), big),
            ("re".to_owned(), big),
        ]
        .into_iter()
        .collect();
        let accounts: BTreeMap<String, crate::x::Account> = [
            ("alice".to_owned(), account(400)),
            ("bob".to_owned(), account(400)),
            ("carol".to_owned(), account(400)),
            ("dan".to_owned(), account(400)),
        ]
        .into_iter()
        .collect();
        let refusals = vec![RefusalLine {
            at: open + 5,
            summoner: "carol".to_owned(),
            why: "cap".to_owned(),
        }];
        let mut last_week = Record::close(Week(WEEK.0 - 1), radar_contest::Ranking::default());
        last_week.winner = Some(radar_contest::Winner {
            summoner: "dan".to_owned(),
            reply_id: "old".to_owned(),
            score: 9,
            handle: None,
        });
        let rules = Rules::published("radar");
        let record = close(&Inputs {
            week: WEEK,
            log: &log,
            refusals: &refusals,
            previous: &[last_week],
            metrics: &metrics,
            accounts: &accounts,
            rules: &rules,
        });

        // The only counted entry is alice's, so alice wins with her small
        // score; everyone louder was excluded for a stated reason.
        assert_eq!(
            record.winner.as_ref().map(|w| w.summoner.as_str()),
            Some("alice")
        );
        assert_eq!(record.winner.as_ref().map(|w| w.score), Some(1));
        let why = |id: &str| {
            record
                .ranking
                .excluded
                .iter()
                .find(|(e, _)| e.reply_id == id)
                .map(|(_, w)| w.clone())
        };
        assert_eq!(why("rb"), Some(Excluded::Unscored));
        assert_eq!(why("rc"), Some(Excluded::RefusedThisWeek));
        assert_eq!(
            why("rd"),
            Some(Excluded::WonWithinCooldown {
                won: Week(WEEK.0 - 1)
            })
        );
        assert_eq!(why("re"), Some(Excluded::AccountAgeUnknown));
        assert_eq!(record.week, WEEK);
        assert_eq!(record.closed_at, WEEK.closes_at());
    }

    #[test]
    fn a_refusal_outside_the_week_does_not_count_against_this_week() {
        // Re-applied by dropping `week.contains(r.at)`: last week's refusal
        // excludes alice this week and this fails.
        let log = vec![entry("a", "alice", WEEK.opens_at() + 10, Some("ra"), None)];
        let metrics: BTreeMap<String, Metrics> = [("ra".to_owned(), Metrics::default())]
            .into_iter()
            .collect();
        let accounts: BTreeMap<String, crate::x::Account> =
            [("alice".to_owned(), account(100))].into_iter().collect();
        let refusals = vec![RefusalLine {
            at: WEEK.opens_at() - 1,
            summoner: "alice".to_owned(),
            why: "cap".to_owned(),
        }];
        let rules = Rules::published("radar");
        let record = close(&Inputs {
            week: WEEK,
            log: &log,
            refusals: &refusals,
            previous: &[],
            metrics: &metrics,
            accounts: &accounts,
            rules: &rules,
        });
        assert_eq!(
            record.winner.as_ref().map(|w| w.summoner.as_str()),
            Some("alice")
        );
    }

    #[test]
    fn the_age_is_whole_days_at_close_so_the_bar_is_the_published_one() {
        // 30 days minus one second is 29 days: excluded. Exactly 30: counts.
        // Re-applied by rounding up: the first counts and this fails.
        for (secs_before_close, counts) in [(30 * 86_400 - 1, false), (30 * 86_400, true)] {
            let log = vec![entry("a", "alice", WEEK.opens_at() + 10, Some("ra"), None)];
            let metrics: BTreeMap<String, Metrics> = [("ra".to_owned(), Metrics::default())]
                .into_iter()
                .collect();
            let accounts: BTreeMap<String, crate::x::Account> = [(
                "alice".to_owned(),
                crate::x::Account {
                    created_at: WEEK.closes_at() - secs_before_close,
                    username: None,
                },
            )]
            .into_iter()
            .collect();
            let rules = Rules::published("radar");
            let record = close(&Inputs {
                week: WEEK,
                log: &log,
                refusals: &[],
                previous: &[],
                metrics: &metrics,
                accounts: &accounts,
                rules: &rules,
            });
            assert_eq!(
                record.winner.is_some(),
                counts,
                "{secs_before_close}s before close"
            );
        }
    }

    #[test]
    fn sightings_are_the_counted_replies_and_an_uncounted_one_is_not_a_sighting() {
        let open = WEEK.opens_at();
        let log = vec![
            entry("a", "alice", open + 10, Some("ra"), Some(2)),
            entry("b", "bob", open + 20, Some("rb"), None), // pre-count line
            entry("c", "carol", open + 30, None, Some(3)),  // never published
            entry("d", "dan", open + 40, Some("rd"), Some(0)), // counted, clean
        ];
        let got = sightings(&log, WEEK);
        assert_eq!(got.len(), 2, "{got:?}");
        assert_eq!((got[0].summoner.as_str(), got[0].signals), ("alice", 2));
        assert_eq!((got[1].summoner.as_str(), got[1].signals), ("dan", 0));
    }

    #[test]
    fn the_week_due_is_the_one_before_the_current_and_week_zero_has_none() {
        assert_eq!(due(WEEK.opens_at()), Some(Week(WEEK.0 - 1)));
        assert_eq!(due(WEEK.closes_at() - 1), Some(Week(WEEK.0 - 1)));
        assert_eq!(due(WEEK.closes_at()), Some(WEEK));
        assert_eq!(due(0), None);
    }

    #[test]
    fn refusals_round_trip_and_a_missing_file_is_no_refusals() {
        let dir = std::env::temp_dir().join(format!("radar-refusals-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("refusals.jsonl").to_string_lossy().into_owned();
        assert!(read_refusals(&path).is_empty());
        let line = RefusalLine {
            at: 7,
            summoner: "mallory".to_owned(),
            why: "this account has had its 3 replies today".to_owned(),
        };
        append_refusal(&path, &line).expect("append");
        append_refusal(&path, &line).expect("append");
        assert_eq!(read_refusals(&path), vec![line.clone(), line]);
    }

    #[test]
    fn a_reply_to_the_claim_prompt_is_a_claim_and_everything_else_is_a_summons() {
        // The winner replies to the account's claim prompt with an address:
        // written into the record, and the mention is not answered. Everything
        // else is a summons -- somebody else replying to the prompt, the winner
        // replying with no address, the winner after the window, a second
        // claim, and, THE ONE THIS FUNCTION WAS FIXED FOR, the winner summoning
        // the bot about a coin during their own claim week.
        //
        // Re-applied: drop the `claim_prompt` condition from the predicate and
        // `a_summons_naming_a_mint` below is recorded as a claim -- the coin's
        // mint address becomes the winner's payout address, and the payout
        // policy approves it, because the recipient really does equal the
        // claim. Drop `accepts_claim_at` and the late claim is recorded.
        let dir = std::env::temp_dir().join(format!("radar-claim-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let dir = dir.to_string_lossy().into_owned();
        let mut record = Record::close(WEEK, radar_contest::Ranking::default());
        record.winner = Some(radar_contest::Winner {
            summoner: "alice".to_owned(),
            reply_id: "r1".to_owned(),
            score: 3,
            handle: None,
        });
        record.claim_prompt = Some("prompt-1".to_owned());
        write_atomically(&record_path(&dir, WEEK), &record.to_json().expect("json"))
            .expect("write");
        let address = radar_types::Address::new([5u8; 32]).to_string();
        let under = |author: &str, text: &str, parent: Option<&str>| Mention {
            id: format!("m-{author}"),
            author: author.to_owned(),
            text: text.to_owned(),
            parent: parent.map(str::to_owned),
        };
        let inside = WEEK.closes_at() + 3_600;
        let late = record.claim_window_closes_at();

        // Somebody else, replying to the prompt.
        assert_eq!(
            try_claim(
                &under("bob", &format!("pay me {address}"), Some("prompt-1")),
                &dir,
                inside
            ),
            None
        );
        // The winner, replying to the prompt with no address in it.
        assert_eq!(
            try_claim(&under("alice", "thanks!", Some("prompt-1")), &dir, inside),
            None
        );
        // The winner, too late.
        assert_eq!(
            try_claim(
                &under("alice", &format!("here {address}"), Some("prompt-1")),
                &dir,
                late
            ),
            None
        );

        // THE MINT CASE. The winner summons the bot about a coin during their
        // claim week. A mint is 32 bytes of base58 and so is a wallet, so the
        // text alone cannot tell them apart -- only the post being replied to
        // can. This is a top-level mention with no parent at all.
        let a_summons_naming_a_mint = under("alice", "@radar what is HWvHqvfF...pump", None);
        assert_eq!(
            try_claim(&a_summons_naming_a_mint, &dir, inside),
            None,
            "a summons is not a claim, whatever address it happens to contain"
        );
        // And the same, replying under some unrelated post rather than the
        // prompt -- which is how most summonses actually arrive.
        assert_eq!(
            try_claim(
                &under(
                    "alice",
                    &format!("look at {address}"),
                    Some("some-other-post")
                ),
                &dir,
                inside
            ),
            None,
            "a reply to anything but the claim prompt is not a claim"
        );

        // The real claim.
        assert_eq!(
            try_claim(
                &under("alice", &format!("here {address}"), Some("prompt-1")),
                &dir,
                inside
            ),
            Some(WEEK)
        );
        let saved = radar_contest::records_in(std::path::Path::new(&dir));
        let claim = saved[0].claim.as_ref().expect("a claim");
        assert_eq!(
            (claim.address.as_str(), claim.reply_id.as_str(), claim.at),
            (address.as_str(), "m-alice", inside)
        );
        // A second address from the winner is a summons: the first claim stands.
        let other = radar_types::Address::new([6u8; 32]).to_string();
        assert_eq!(
            try_claim(&under("alice", &other, Some("prompt-1")), &dir, inside + 1),
            None
        );
        assert_eq!(
            saved[0].claim.as_ref().map(|c| c.address.as_str()),
            Some(address.as_str())
        );
    }

    #[test]
    fn a_prompt_is_due_only_for_an_unprompted_winner_inside_the_window() {
        // All three conditions, each proved by a case that fails without it.
        // CI reported both `&&` here as surviving mutants -- the function had
        // no test at all -- and each `||` mutation is caught by exactly one of
        // the negative cases below.
        let dir = std::env::temp_dir().join(format!("radar-due-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let dir = dir.to_string_lossy().into_owned();
        let inside = WEEK.closes_at() + 3_600;

        let write = |record: &Record| {
            write_atomically(
                &record_path(&dir, record.week),
                &record.to_json().expect("json"),
            )
            .expect("write");
        };
        let with_winner = || {
            let mut record = Record::close(WEEK, radar_contest::Ranking::default());
            record.winner = Some(radar_contest::Winner {
                summoner: "alice".to_owned(),
                reply_id: "r1".to_owned(),
                score: 3,
                handle: None,
            });
            record
        };

        // Nobody won: there is no reply to post the prompt under.
        write(&Record::close(WEEK, radar_contest::Ranking::default()));
        assert_eq!(prompt_due(&dir, inside), None, "a week nobody won");

        // A winner, no prompt, inside the window: due.
        write(&with_winner());
        assert_eq!(
            prompt_due(&dir, inside).map(|r| r.week),
            Some(WEEK),
            "a winner who has not been told"
        );

        // Already prompted: posting a second one would give the winner two
        // posts to reply to and only one of them would be the claim.
        let mut prompted = with_winner();
        prompted.claim_prompt = Some("prompt-1".to_owned());
        write(&prompted);
        assert_eq!(prompt_due(&dir, inside), None, "already prompted");

        // Past the window: the pool has rolled over, so there is nothing left
        // to claim and no reason to ask.
        write(&with_winner());
        let late = with_winner().claim_window_closes_at();
        assert_eq!(prompt_due(&dir, late), None, "the window has closed");
    }

    #[test]
    fn a_week_whose_prompt_never_posted_accepts_no_claim_at_all() {
        // `claim_prompt: None` is a week where the account never managed to
        // post the prompt -- a dry run, or a failed post. No mention can claim
        // it, including one that replies to the winning reply itself.
        //
        // Deliberate, and the safer half of a real trade-off. Design 0007 §6.2
        // would also accept a reply under the winning reply; that is refused
        // here because a winner replying "now do this one" with a fresh coin
        // under their own winning reply is an ordinary summons, and reading it
        // as a claim would reopen exactly the hole this function was fixed to
        // close. The cost is a week that cannot be claimed until the prompt
        // posts -- the daemon retries it every tick -- and an unclaimed pool
        // rolls over. A prize paid to a mint account does not roll back.
        let dir = std::env::temp_dir().join(format!("radar-noprompt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let dir = dir.to_string_lossy().into_owned();
        let mut record = Record::close(WEEK, radar_contest::Ranking::default());
        record.winner = Some(radar_contest::Winner {
            summoner: "alice".to_owned(),
            reply_id: "r1".to_owned(),
            score: 3,
            handle: None,
        });
        assert!(record.claim_prompt.is_none());
        write_atomically(&record_path(&dir, WEEK), &record.to_json().expect("json"))
            .expect("write");
        let address = radar_types::Address::new([7u8; 32]).to_string();
        let inside = WEEK.closes_at() + 3_600;
        for parent in [None, Some("r1"), Some("anything")] {
            let mention = Mention {
                id: "m1".to_owned(),
                author: "alice".to_owned(),
                text: format!("here {address}"),
                parent: parent.map(str::to_owned),
            };
            assert_eq!(try_claim(&mention, &dir, inside), None, "parent {parent:?}");
        }
    }

    #[test]
    fn an_empty_week_is_closed_without_a_client_and_a_full_one_is_not() {
        // The empty week must be on disk so the leaderboard can say "no
        // winner". The full one needs metrics, and there is no client here, so
        // nothing is written and the next tick will try again. Re-applied by
        // writing the record anyway: a week with entries and a record with
        // nobody in it, and the second assertion fails.
        let dir = std::env::temp_dir().join(format!("radar-close-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let analyst = dir.join("analyst");
        std::fs::create_dir_all(&analyst).expect("mkdir");
        let paths = Paths::under(&analyst.to_string_lossy());
        let rules = Rules::published("radar");
        let now = WEEK.closes_at() + 5;

        let closed = close_if_due(None, &paths, now, &rules, 3).expect("io");
        assert!(closed.is_some_and(|r| r.week == WEEK && r.winner.is_none()));
        assert!(std::path::Path::new(&record_path(&paths.contest_dir, WEEK)).exists());
        assert!(std::path::Path::new(&hunter_path(&paths.contest_dir, WEEK)).exists());
        // Idempotent: the record exists, so the next tick does nothing.
        assert!(
            close_if_due(None, &paths, now, &rules, 3)
                .expect("io")
                .is_none()
        );

        // A later week with an entry and no client: not closed.
        let next = Week(WEEK.0 + 1);
        let e = entry("a", "alice", next.opens_at() + 10, Some("ra"), Some(1));
        crate::log::append(&paths.log, &e).expect("append");
        let closed = close_if_due(None, &paths, next.closes_at() + 5, &rules, 3).expect("io");
        assert!(closed.is_none());
        assert!(!std::path::Path::new(&record_path(&paths.contest_dir, next)).exists());
    }
}
