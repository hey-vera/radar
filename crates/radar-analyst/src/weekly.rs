// SPDX-License-Identifier: Apache-2.0
//! The weekly post: the week's result, and the winner's coin torn down.
//!
//! Design 0007 B6 and C8, design 0009 §3 L4 and §5 M2, plan 0006 item 6. Once
//! the week has closed and its record is on disk, the account posts the
//! result as a top-level post and, as a reply to it, the full fact sheet of
//! the winning coin -- the attention prize L4 says is the real one. Both go
//! out on X and, when a channel is configured, on Telegram.
//!
//! # Every number is authorised, or the post does not ship
//!
//! A reply's numbers are checked against the fact sheet it was built from. A
//! post has no fact sheet, so it carries its own authorised set: every figure
//! the template writes -- the counts, the score, the pool, the date, the reply
//! id -- is put on that list by the code that wrote it, and [`check`] runs the
//! same fidelity and forbidden checks a reply passes. A template cannot
//! fabricate a number, so this is guarding against the day somebody edits the
//! template and the list separately; the test that re-applies that is below.
//!
//! # Never a price, never a handle
//!
//! ADR 0013 constraint 5: the pool is stated in SOL, the token's price is
//! not stated at all, and the winner is the reply's URL rather than a handle
//! -- the log carries the author's id, which cannot be renamed and is not a
//! name to print.

use std::fmt::Write as _;

use radar_contest::{Record, Vault};
use radar_roast::sheet::FactSheet;
use radar_roast::voice::Reply;
use radar_types::civil::{date_from_days, timestamp_from_seconds};

use crate::log::Entry;
use crate::publish::{Publisher, Undeliverable};

/// One post, with the numbers it is allowed to contain and the evidence it
/// was written from.
#[derive(Clone, Debug, PartialEq)]
pub struct Post {
    /// What is said.
    pub text: String,
    /// Every numeric value the text may contain.
    pub authorised: Vec<f64>,
    /// What it was written from, recorded beside it: the record's JSON, or
    /// the fact sheet as the model saw it.
    pub source: String,
}

/// Lamports in one SOL.
const LAMPORTS_PER_SOL: f64 = 1_000_000_000.0;

/// The public URL of a reply. The `i/web/status` form resolves without the
/// author's handle, which the log does not carry and should not have to.
fn reply_url(reply_id: &str) -> String {
    format!("x.com/i/web/status/{reply_id}")
}

/// Puts a `YYYY-MM-DD` date's three numerals on the list.
fn authorise_date(authorised: &mut Vec<f64>, date: &str) {
    authorised.extend(date.split('-').filter_map(|p| p.parse::<f64>().ok()));
}

/// The post that tells the winner they won and how to claim.
///
/// Design 0007 §6.2 specifies this and it was never built, which had two
/// consequences. The winner was never told: the summary names them by reply
/// URL and nothing arrives in their notifications. And, worse, `try_claim` had
/// nothing to require a claim to be a reply *to*, so it accepted any
/// mint-shaped string in any mention by the winner -- see that function.
///
/// Posted as a reply under the **winner's own summons**, which is the one reply
/// X guarantees to accept (its author mentioned the bot, by definition) and the
/// one that reaches their notifications. `daemon::prompt_claim_if_due` chooses
/// the parent; this function only writes the words.
///
/// Every numeral is authorised: the date's three parts, the score, and the two
/// zeros in "00:00 UTC".
#[must_use]
#[allow(clippy::cast_precision_loss, reason = "a score, far below 2^53")]
pub fn claim_prompt(record: &Record) -> Option<Post> {
    let winner = record.winner.as_ref()?;
    let mut authorised = Vec::new();
    let monday = date_from_days(i64::try_from(record.opened_at / 86_400).unwrap_or(i64::MAX));
    authorise_date(&mut authorised, &monday);
    let until =
        date_from_days(i64::try_from(record.claim_window_closes_at() / 86_400).unwrap_or(i64::MAX));
    authorise_date(&mut authorised, &until);
    authorised.push(winner.score as f64);

    // Trimmed to fit, and the first draft did not: at 281 characters it was one
    // over a standard account's limit, caught by the test below rather than by
    // the platform. What went was " 00:00 UTC" after the date -- the window has
    // always closed at midnight UTC and the date alone says enough -- and one
    // "and". Nothing about what the winner has to do was shortened.
    let text = format!(
        "This reply won the week of {monday}: {} points under the published rule.
         To claim the pool, reply to THIS post with a Solana wallet address, from this          account, before {until}.
         Nothing to connect, nothing to sign. The reply is the proof.",
        winner.score
    );
    Some(Post {
        text,
        authorised,
        source: record.to_json().unwrap_or_default(),
    })
}

/// The week's result, as the top-level post.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    reason = "counts, a score and lamports, all far below 2^53"
)]
pub fn summary(record: &Record, vault: Option<&Vault>) -> Post {
    let mut authorised = Vec::new();
    let monday = date_from_days(i64::try_from(record.opened_at / 86_400).unwrap_or(i64::MAX));
    authorise_date(&mut authorised, &monday);
    // "00:00 UTC" is two zeros to the scanner.
    authorised.push(0.0);

    let counted = record.ranking.ranked.len();
    let excluded = record.ranking.excluded.len();
    let answered = counted + excluded;
    for n in [answered, counted, excluded] {
        authorised.push(n as f64);
    }

    // Under 280 characters in every branch, which the test pins: a standard X
    // account has that limit and a post cut off mid-figure is a wrong figure.
    let mut text = format!(
        "Week of {monday}: {answered} summoned {}; {counted} counted, {excluded} excluded \
         under the published rule.",
        if answered == 1 { "reply" } else { "replies" }
    );

    match &record.winner {
        Some(w) => {
            authorised.push(w.score as f64);
            if let Ok(id) = w.reply_id.parse::<f64>() {
                authorised.push(id);
            }
            let _ = write!(
                text,
                "\nTop reply: {} {} -- {}",
                w.score,
                if w.score == 1 { "point" } else { "points" },
                reply_url(&w.reply_id)
            );
        }
        None => text.push_str("\nNothing counted, so no winner; the pool rolls over."),
    }

    match vault {
        Some(v) => {
            let sol = v.lamports as f64 / LAMPORTS_PER_SOL;
            let rendered = format!("{sol:.3}");
            authorised.push(sol);
            if let Ok(r) = rendered.parse::<f64>() {
                authorised.push(r);
            }
            let at = timestamp_from_seconds(v.measured_at);
            authorise_date(&mut authorised, &at[..10]);
            authorised.extend(at[11..19].split(':').filter_map(|p| p.parse::<f64>().ok()));
            let _ = write!(text, "\nPrize pool: {rendered} SOL at {at}");
            match (&record.payout, &record.winner) {
                (Some(_), _) => text.push_str("; paid to the claim, transaction on the site."),
                (None, Some(_)) => {
                    // The date, not the timestamp. The window has always closed
                    // at midnight UTC and the date alone says enough -- the same
                    // trim `claim_prompt` already made, for the same reason:
                    // this post is at the platform's limit to the character,
                    // and a post cut off mid-figure is a wrong figure.
                    let until = timestamp_from_seconds(record.claim_window_closes_at());
                    let day = &until[..10];
                    authorise_date(&mut authorised, day);
                    let _ = write!(text, "; claim open until {day}.");
                }
                (None, None) => text.push('.'),
            }
        }
        None => text.push_str("\nPrize pool: no token yet."),
    }
    // The address itself, since 2026-09-06. `forbidden::check` masks this exact
    // literal before scanning, so the account can name its own site and every
    // other "cabal" is still refused. Before that the line read "on the site"
    // with no link, and a reader who wanted to check the rule had to go and
    // find it -- which is a strange thing to ask of somebody you are inviting
    // to check you.
    text.push_str("\nRule and leaderboard: cabalhunter.org/leaderboard");

    Post {
        text,
        authorised,
        source: record.to_json().unwrap_or_default(),
    }
}

/// The week's best hunters, named, as the third post in the thread.
///
/// Design 0009 item 8. The hunter rank is **status, never money** (design 0009
/// §5, M3): it counts the refusal signals a summoner's replies carried, which
/// is the skill the bot teaches every time it answers. Naming the top three is
/// the whole of the reward, so the post has to exist for the rank to mean
/// anything.
///
/// # Why a third post rather than three more words on the summary
///
/// The summary's fullest branch is already at the platform's limit, to the
/// character. There is no room, and a post cut off mid-figure is a wrong
/// figure -- so this is a reply in the same thread instead of a truncation.
///
/// # Handles only, and `None` when there are none
///
/// A `Placing` carries the numeric account id, and posting that would render
/// `@1234567890` at a reader as though somebody had chosen it -- the site's own
/// S4, in a post this time. Hunters with no handle on the record are skipped,
/// and a board with no named hunters at all produces no post: an empty
/// leaderboard is not worth a reply.
#[must_use]
#[allow(clippy::cast_precision_loss, reason = "a signal count, far below 2^53")]
pub fn hunters(record: &Record, board: &[radar_contest::hunter::Placing]) -> Option<Post> {
    // Every handle the week's record knows, ranked and excluded alike: an
    // entrant excluded from the *prize* still hunted, and the two rules are
    // deliberately different ones.
    let handles: std::collections::BTreeMap<&str, &str> = record
        .ranking
        .ranked
        .iter()
        .map(|r| &r.entry)
        .chain(record.ranking.excluded.iter().map(|(e, _)| e))
        .filter_map(|e| Some((e.summoner.as_str(), e.handle.as_deref()?)))
        .collect();

    let named: Vec<(&str, u64)> = board
        .iter()
        .filter(|p| p.signals > 0)
        .filter_map(|p| Some((*handles.get(p.summoner.as_str())?, p.signals)))
        .take(3)
        .collect();
    if named.is_empty() {
        return None;
    }

    let mut authorised = Vec::new();
    let monday = date_from_days(i64::try_from(record.opened_at / 86_400).unwrap_or(i64::MAX));
    authorise_date(&mut authorised, &monday);
    let mut text = format!("Best hunters, week of {monday}, by refusal signals found:");
    for (handle, signals) in &named {
        authorised.push(*signals as f64);
        let _ = write!(
            text,
            "\n@{handle} -- {signals} {}",
            if *signals == 1 { "signal" } else { "signals" }
        );
    }
    // Status, and it says so. The prize is a separate rule and a reader who
    // confuses the two will think the board decides the money.
    text.push_str("\nStatus, not money. The rule: cabalhunter.org/history");

    Some(Post {
        text,
        authorised,
        source: record.to_json().unwrap_or_default(),
    })
}

/// The winner's coin, torn down: the reply the roaster wrote from its sheet,
/// carried with the sheet's own authorised set.
#[must_use]
pub fn teardown(sheet: &FactSheet, reply: &Reply) -> Post {
    Post {
        text: reply.text.clone(),
        authorised: sheet.authorised(),
        source: sheet.render(),
    }
}

/// The two checks a reply passes, applied to a post.
///
/// # Errors
///
/// Every violation, in words, so the log line says what was refused.
pub fn check(post: &Post) -> Result<(), Vec<String>> {
    let mut why: Vec<String> = radar_roast::forbidden::check(&post.text)
        .into_iter()
        .map(|v| format!("forbidden: {v:?}"))
        .collect();
    why.extend(
        radar_roast::fidelity::check(&post.text, &post.authorised)
            .into_iter()
            .map(|f| format!("unauthorised number: {}", f.literal)),
    );
    if why.is_empty() { Ok(()) } else { Err(why) }
}

/// Publishes a thread: the first post top-level, each later one as a reply to
/// the one before. Every post is recorded before it is said and again after,
/// the way replies are, under `<label>:<n>` -- `weekly:<week>` here,
/// `daily:<date>` for the daily post, which shares this function.
///
/// A post that fails its [`check`] is recorded as refused and not sent, and
/// the thread stops there: the teardown must not go out under a summary that
/// did not. Returns how many were actually sent.
///
/// # Errors
///
/// The I/O error from the log. A post that cannot be recorded is not said.
pub fn publish(
    publisher: &dyn Publisher,
    posts_log: &str,
    label: &str,
    posts: &[Post],
    now: u64,
) -> std::io::Result<usize> {
    publish_under(publisher, posts_log, label, None, posts, now).map(|(sent, _)| sent)
}

/// The same, but starting the thread as a reply to `under` rather than as a
/// top-level post, and returning the id of the first post that landed.
///
/// The claim prompt needs both halves: it belongs under the account's own
/// winning reply so it reaches the winner's thread, and its id has to be
/// written back into the record, because that id is what a claim must reply to.
///
/// # Errors
///
/// Only the log's I/O. A post the platform refuses is recorded and stops the
/// thread; it is not an error here.
pub fn publish_under(
    publisher: &dyn Publisher,
    posts_log: &str,
    label: &str,
    under: Option<&str>,
    posts: &[Post],
    now: u64,
) -> std::io::Result<(usize, Option<String>)> {
    let mut sent = 0;
    let mut first: Option<String> = None;
    let mut parent: Option<String> = under.map(str::to_owned);
    for (n, post) in posts.iter().enumerate() {
        let mut entry = Entry {
            at: now,
            mention_id: format!("{label}:{n}"),
            summoner: "radar".to_owned(),
            mint: None,
            read_at_slot: None,
            fact_sheet: post.source.clone(),
            reply: post.text.clone(),
            fellback: None,
            reply_id: None,
            signals: None,
        };
        if let Err(why) = check(post) {
            entry.fellback = Some(format!("refused: {}", why.join("; ")));
            crate::log::append(posts_log, &entry)?;
            eprintln!(
                "radar-analyst: {label} post {n} refused: {}",
                why.join("; ")
            );
            break;
        }
        crate::log::append(posts_log, &entry)?;
        let result = match &parent {
            None => publisher.post(&post.text),
            Some(id) => publisher.reply(id, &post.text),
        };
        match result {
            Ok(id) => {
                if first.is_none() {
                    first = Some(id.clone());
                }
                parent = Some(id.clone());
                entry.reply_id = Some(id);
                sent += 1;
            }
            Err(why) => {
                entry.fellback = Some(format!("not published: {why}"));
                crate::log::append(posts_log, &entry)?;
                if matches!(why, Undeliverable::Unconfigured) {
                    // A dry run: recorded, nothing said, and nothing further
                    // to say the rest under.
                    eprintln!("radar-analyst: {label} post {n} recorded, not published (dry run)");
                } else {
                    eprintln!("radar-analyst: {label} post {n} failed: {why}");
                }
                break;
            }
        }
        crate::log::append(posts_log, &entry)?;
    }
    Ok((sent, first))
}

#[cfg(test)]
mod tests {
    use super::*;
    use radar_contest::{Entry as ContestEntry, Metrics, Ranked, Ranking, Week};

    const WEEK: Week = Week(2957);

    fn record(winner: bool) -> Record {
        let mut ranking = Ranking::default();
        if winner {
            ranking.ranked.push(Ranked {
                entry: ContestEntry {
                    reply_id: "1963012345678901234".to_owned(),
                    summoner: "9001".to_owned(),
                    mention_id: None,
                    handle: None,
                    mint: "MintOne".to_owned(),
                    at: WEEK.opens_at() + 10,
                    metrics: Metrics {
                        reposts: 4,
                        likes: 3,
                        ..Metrics::default()
                    },
                },
                score: 15,
            });
        }
        ranking.excluded.push((
            ContestEntry {
                reply_id: "r2".to_owned(),
                summoner: "9002".to_owned(),
                mention_id: None,
                handle: None,
                mint: "MintTwo".to_owned(),
                at: WEEK.opens_at() + 20,
                metrics: Metrics::default(),
            },
            radar_contest::Excluded::Unscored,
        ));
        Record::close(WEEK, ranking, &radar_contest::Rules::published(["radar"]))
    }

    /// A board, best first, the way `hunter::tally` returns one.
    fn placing(summoner: &str, signals: u64) -> radar_contest::hunter::Placing {
        radar_contest::hunter::Placing {
            summoner: summoner.to_owned(),
            signals,
            counted: 1,
            over_cap: 0,
        }
    }

    /// The week's record with handles on both entrants, so the post has names.
    fn record_with_handles() -> Record {
        let mut r = record(true);
        r.ranking.ranked[0].entry.handle = Some("first".to_owned());
        r.ranking.excluded[0].0.handle = Some("second".to_owned());
        r
    }

    #[test]
    fn the_hunters_post_names_handles_and_never_a_bare_account_id() {
        // The site's S4, in a post this time: `@9001` reads as a name somebody
        // chose, and it is a numeric account id. Re-apply by dropping the
        // `handles.get` lookup and posting `p.summoner` -- the second
        // assertion fails.
        let record = record_with_handles();
        let board = [placing("9001", 5), placing("9002", 2), placing("9003", 9)];
        let post = hunters(&record, &board).expect("two named hunters");

        // The week, as a date. CI turned the `/ 86_400` into `%` and `*` and
        // nothing failed, because nothing here read the date -- so the post
        // could have been headed "week of 1970-01-01" at every reader.
        assert!(post.text.contains("week of 2026-08-31"), "{}", post.text);
        assert!(post.text.contains("@first -- 5 signals"), "{}", post.text);
        assert!(post.text.contains("@second -- 2 signals"), "{}", post.text);
        // 9003 has no handle on this record, so it is skipped rather than
        // posted as a number.
        assert!(!post.text.contains("9003"), "{}", post.text);
        // Status, said plainly. A reader who thinks the board decides the money
        // has been told something false about how the contest works.
        assert!(post.text.contains("Status, not money"), "{}", post.text);

        // The same two checks every other post goes through, and the link is
        // the reason the second one is worth running here.
        assert_eq!(check(&post), Ok(()), "{}", post.text);
        assert!(
            post.text.chars().count() <= 280,
            "{} chars: {}",
            post.text.chars().count(),
            post.text
        );
    }

    #[test]
    fn a_board_with_nothing_to_say_produces_no_post_at_all() {
        // An empty leaderboard is not worth a reply, and a post reading "Best
        // hunters:" with nothing under it is worse than silence.
        let record = record_with_handles();
        assert!(hunters(&record, &[]).is_none());
        // A hunter who found no signals is not a hunter. Zero here is a real
        // measurement -- the sheets carried no refusal signal -- not an
        // absence, so it is filtered rather than named.
        assert!(hunters(&record, &[placing("9001", 0)]).is_none());
        // And a board of ids the record has no handle for.
        assert!(hunters(&record, &[placing("9999", 7)]).is_none());
    }

    #[test]
    fn the_hunters_post_names_at_most_three() {
        let mut record = record_with_handles();
        // Four entrants, all named, all with signals.
        for (id, handle) in [("9003", "third"), ("9004", "fourth")] {
            record.ranking.excluded.push((
                ContestEntry {
                    reply_id: format!("r-{id}"),
                    summoner: id.to_owned(),
                    handle: Some(handle.to_owned()),
                    mint: "M".to_owned(),
                    at: WEEK.opens_at() + 30,
                    metrics: Metrics::default(),
                },
                radar_contest::Excluded::Unscored,
            ));
        }
        let board = [
            placing("9001", 9),
            placing("9002", 5),
            placing("9003", 3),
            placing("9004", 1),
        ];
        let post = hunters(&record, &board).expect("a board");
        assert!(post.text.contains("@third"), "{}", post.text);
        assert!(!post.text.contains("@fourth"), "{}", post.text);
    }

    fn vault() -> Vault {
        Vault {
            address: "VAULT".to_owned(),
            lamports: 1_234_567_890,
            measured_at: WEEK.closes_at() + 60,
        }
    }

    #[test]
    fn the_claim_prompt_tells_the_winner_how_to_claim_and_passes_both_checks() {
        // The post design 0007 section 6.2 asked for and nothing built. Two
        // jobs: it tells the winner they won -- the summary names them only by
        // URL, which reaches nobody's notifications -- and its id becomes the
        // post a claim must reply to, which is what stops a coin's mint being
        // read as a payout address.
        let post = claim_prompt(&record(true)).expect("a week with a winner has a prompt");
        assert!(
            post.text.contains("won the week of 2026-08-31"),
            "{}",
            post.text
        );
        assert!(post.text.contains("15 points"), "{}", post.text);
        // The instruction has to be unmistakable: THIS post, not the summary,
        // not the winning reply, not a fresh mention.
        assert!(post.text.contains("reply to THIS post"), "{}", post.text);
        assert!(post.text.contains("Solana wallet address"), "{}", post.text);
        // Week 2957 closes 2026-09-07, so the seven-day window ends 2026-09-14.
        assert!(post.text.contains("before 2026-09-14"), "{}", post.text);

        // Every numeral on the sheet, and nothing the forbidden list refuses.
        // The same two checks every other post goes through.
        assert_eq!(check(&post), Ok(()), "{}", post.text);
        assert!(
            post.text.chars().count() <= 280,
            "{} chars: {}",
            post.text.chars().count(),
            post.text
        );
    }

    #[test]
    fn a_week_nobody_won_has_no_claim_prompt_to_post() {
        // Not an empty post and not a post saying nobody won -- the summary
        // already says that. There is simply nothing to reply under.
        assert_eq!(claim_prompt(&record(false)), None);
    }

    #[test]
    fn the_summary_states_the_week_and_passes_both_checks() {
        let post = summary(&record(true), Some(&vault()));
        // Week 2957 opens on Monday 2026-08-31 and closes 2026-09-07.
        assert!(post.text.contains("Week of 2026-08-31"), "{}", post.text);
        assert!(
            post.text
                .contains("2 summoned replies; 1 counted, 1 excluded"),
            "{}",
            post.text
        );
        assert!(
            post.text
                .contains("15 points -- x.com/i/web/status/1963012345678901234")
        );
        assert!(
            post.text
                .contains("Prize pool: 1.235 SOL at 2026-09-07T00:01:00Z")
        );
        // The date, not the timestamp: the window has always closed at
        // midnight UTC, and this post is at the limit to the character.
        assert!(
            post.text.contains("claim open until 2026-09-14."),
            "{}",
            post.text
        );
        // The account can name its own site. `forbidden::check` masks this
        // exact literal, so `check` below is the real proof, not this line --
        // until 2026-09-06 the post said "on the site" with no address,
        // because the checks refused the word "cabal" in its own domain.
        assert!(
            post.text.contains("cabalhunter.org/leaderboard"),
            "{}",
            post.text
        );
        // A standard account's limit. The longest branch is this one: a winner,
        // a pool and an open claim.
        assert!(
            post.text.chars().count() <= 280,
            "{} chars: {}",
            post.text.chars().count(),
            post.text
        );
        assert_eq!(check(&post), Ok(()), "{}", post.text);
        assert!(!post.source.is_empty(), "the record travels with the post");
    }

    #[test]
    fn no_winner_and_no_token_are_said_in_words_and_still_pass() {
        let post = summary(&record(false), None);
        assert!(post.text.contains("no winner"), "{}", post.text);
        assert!(post.text.contains("no token yet"), "{}", post.text);
        assert!(!post.text.contains("Top reply"));
        assert_eq!(check(&post), Ok(()), "{}", post.text);
    }

    #[test]
    fn a_number_the_template_did_not_authorise_is_refused() {
        // The guard against editing the template and the list separately.
        // Re-applied by deleting `authorised.push(w.score as f64)`: the score
        // is unauthorised and the first assertion below is what fails.
        let mut post = summary(&record(true), Some(&vault()));
        assert_eq!(check(&post), Ok(()));
        post.text.push_str(" Up 420% this week.");
        let why = check(&post).expect_err("a fabricated number");
        assert!(why.iter().any(|w| w.contains("420")), "{why:?}");

        let mut post = summary(&record(true), None);
        post.text.push_str(" This one is safe.");
        let why = check(&post).expect_err("a forbidden phrase");
        assert!(why.iter().any(|w| w.starts_with("forbidden")), "{why:?}");
    }

    #[derive(Debug, Default)]
    struct Records(std::sync::Mutex<Vec<(Option<String>, String)>>);

    impl Publisher for Records {
        fn name(&self) -> &'static str {
            "records"
        }
        fn reply(&self, parent: &str, text: &str) -> Result<String, Undeliverable> {
            self.0
                .lock()
                .expect("lock")
                .push((Some(parent.to_owned()), text.to_owned()));
            Ok(format!("reply-under-{parent}"))
        }
        fn post(&self, text: &str) -> Result<String, Undeliverable> {
            self.0.lock().expect("lock").push((None, text.to_owned()));
            Ok("top-1".to_owned())
        }
    }

    fn temp(name: &str) -> String {
        let dir = std::env::temp_dir().join(format!("radar-weekly-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join(name);
        let _ = std::fs::remove_file(&path);
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn the_thread_is_a_top_level_post_and_replies_under_it_each_recorded_twice() {
        let log = temp("thread.jsonl");
        let publisher = Records::default();
        let posts = [
            summary(&record(true), None),
            Post {
                text: "The winning coin: six token accounts in the launch block.".to_owned(),
                authorised: vec![6.0],
                source: "sheet".to_owned(),
            },
        ];
        let sent = publish(&publisher, &log, "weekly:2957", &posts, 1_788_000_000).expect("logged");
        assert_eq!(sent, 2);
        let calls = publisher.0.lock().expect("lock").clone();
        assert_eq!(calls[0].0, None, "the summary is top-level");
        assert_eq!(
            calls[1].0.as_deref(),
            Some("top-1"),
            "the teardown replies to it"
        );

        let lines = crate::log::read(&log).expect("read");
        assert_eq!(lines.len(), 4, "intent and outcome for each: {lines:?}");
        let folded = crate::log::latest(&log).expect("read");
        assert_eq!(folded[0].mention_id, "weekly:2957:0");
        assert_eq!(folded[0].reply_id.as_deref(), Some("top-1"));
        assert_eq!(folded[1].reply_id.as_deref(), Some("reply-under-top-1"));
    }

    #[test]
    fn a_dry_run_records_the_summary_and_sends_nothing_and_a_refused_post_stops_the_thread() {
        let log = temp("dry.jsonl");
        let posts = [summary(&record(true), None), summary(&record(false), None)];
        let sent =
            publish(&crate::publish::DryRun, &log, "weekly:2957", &posts, 1).expect("logged");
        assert_eq!(sent, 0);
        let folded = crate::log::latest(&log).expect("read");
        assert_eq!(
            folded.len(),
            1,
            "the thread stops when the first post does not go out"
        );
        assert!(
            folded[0]
                .fellback
                .as_deref()
                .is_some_and(|f| f.contains("not published"))
        );

        // Re-applied by removing the `check` before `append`: the fabricated
        // post is sent and `sent` is 1 here.
        let log = temp("refused.jsonl");
        let mut bad = summary(&record(true), None);
        bad.text.push_str(" Up 9000%.");
        let publisher = Records::default();
        let sent = publish(&publisher, &log, "weekly:2957", &[bad], 1).expect("logged");
        assert_eq!(sent, 0);
        assert!(
            publisher.0.lock().expect("lock").is_empty(),
            "nothing reached the platform"
        );
        let folded = crate::log::latest(&log).expect("read");
        assert!(
            folded[0]
                .fellback
                .as_deref()
                .is_some_and(|f| f.starts_with("refused:"))
        );
    }
}
