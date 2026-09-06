// SPDX-License-Identifier: Apache-2.0
//! The published scoring rule, and who is excluded from it.
//!
//! # The rule, in one line
//!
//! `3·reposts + 3·quotes + 1·likes + 1·replies`, over the public metrics of
//! the **bot's own reply**, not the summoner's post. Design 0007 section 6.2
//! says why: it is the cheapest read, it is harder to buy engagement for
//! somebody else's tweet, and it rewards bringing a coin worth roasting. The
//! weights are constants here and printed in full on the public site; changing
//! one is changing the published rule and is recorded as such.
//!
//! # Exclusions are stated, not silent
//!
//! An entry that does not count is returned beside the reason, never dropped.
//! Design 0007 accepts that the contest can be gamed for a few dollars and
//! answers with visibility rather than a claim of impossibility: every
//! exclusion is in the ledger with its reason, so a reader can see the rule was
//! applied and argue with the rule rather than with a missing row.
//!
//! # Unknown is not eligible
//!
//! Rule 9. An account whose age the caller could not establish is excluded
//! with [`Excluded::AccountAgeUnknown`], not admitted on the assumption that it
//! is old enough. The prize is paid from a public vault to a public address;
//! the safe error is a real entrant excluded for a week, which the ledger shows
//! and the next week corrects, not a thirty-minute-old account paid.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::week::Week;

/// Weight of a repost in the score.
pub const REPOST_WEIGHT: u64 = 3;
/// Weight of a quote post in the score.
pub const QUOTE_WEIGHT: u64 = 3;
/// Weight of a like in the score.
pub const LIKE_WEIGHT: u64 = 1;
/// Weight of a reply in the score.
///
/// **Zero under the verified rule, and the reason is the whole point of that
/// rule.** A reply is conversation rather than spread, one account can post
/// unlimited replies to the same post, and the account's own claim prompt is a
/// reply to its own winning reply. It is kept as a constant, and kept in the
/// raw score, because the raw score is published as evidence beside the
/// verified one and a reader comparing the two needs both formulas to be
/// stated.
pub const REPLY_WEIGHT: u64 = 1;

/// What a verified scan found: engagement that cost somebody something.
///
/// # Why the raw metrics cannot be the rule
///
/// On X a single account can quote or reply to the same post **without limit**.
/// Only a repost and a like are one-per-account. So under
/// `3·reposts + 3·quotes + 1·likes + 1·replies` over `public_metrics`, one
/// account can take the top of the leaderboard, and the week's prize, for
/// nothing at all. Research 0029, S16.
///
/// # What this can and cannot do
///
/// It cannot make farming unprofitable. Likes sell from about $2 per hundred
/// and reposts from about $1.50, from aged accounts with bios and posting
/// histories, and the prize is tens of dollars — so no engagement rule prices a
/// determined farm out. What it does is make each point cost **dollars instead
/// of nothing**: count only what is one-per-account, count only accounts old
/// enough to cost money to fake, and publish every number so a bought week is
/// visible to anybody reading the page.
///
/// Design 0011 argued for publishing the measurement and never a verdict, and
/// that is kept exactly: these are counts. Nothing here says "botted".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verified {
    /// Distinct accounts that reposted, old enough to count.
    pub reposts: u64,
    /// Distinct accounts that quoted, old enough to count.
    ///
    /// **Accounts, not quotes.** Ten quotes from one account are one quoter,
    /// which is the difference between this and `Metrics::quotes`.
    pub quoters: u64,
    /// Distinct accounts that liked, old enough to count.
    pub likes: u64,
    /// How many distinct accounts were seen across all three reads.
    pub engagers: u64,
    /// How many of those were younger than the rule's floor.
    ///
    /// Published as a count beside the score. It is evidence a reader weighs,
    /// never a threshold that excludes anybody: design 0011 phase 2 makes a
    /// cluster measurement into a rule only by ADR, after four closed weeks.
    pub engagers_under_age: u64,
}

impl Verified {
    /// The verified score. Replies weigh nothing; see [`REPLY_WEIGHT`].
    #[must_use]
    pub const fn score(&self) -> u64 {
        self.reposts
            .saturating_mul(REPOST_WEIGHT)
            .saturating_add(self.quoters.saturating_mul(QUOTE_WEIGHT))
            .saturating_add(self.likes.saturating_mul(LIKE_WEIGHT))
    }
}

/// The public metrics of one of the bot's own replies, read at week close.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Metrics {
    /// Reposts of the reply.
    pub reposts: u64,
    /// Quote posts of the reply.
    pub quotes: u64,
    /// Likes on the reply.
    pub likes: u64,
    /// Replies to the reply.
    pub replies: u64,
    /// What the engager scan found, when this entry was scanned.
    ///
    /// `None` means the scan did not reach this entry -- the walk stops as soon
    /// as arithmetic says nothing below can win (`close_if_due`), so most
    /// entries in a busy week are never scanned and are correctly scored raw.
    /// It is **not** "nobody engaged", which would be a `Some` full of zeroes.
    /// Rule 9, and the distinction decides the ranking.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified: Option<Verified>,
}

impl Metrics {
    /// The score that ranks this entry.
    ///
    /// Verified when the scan reached it, raw otherwise. Mixing the two in one
    /// ranking is sound because **raw is always at least verified**: the raw
    /// formula counts every account and adds replies, the verified one counts a
    /// subset of accounts and drops replies. That inequality is what makes the
    /// walk's stopping rule correct, and it is asserted below rather than
    /// assumed.
    #[must_use]
    pub const fn score(&self) -> u64 {
        match self.verified {
            Some(v) => v.score(),
            None => self.raw_score(),
        }
    }

    /// The published metrics scored as they come off the platform.
    ///
    /// Kept, and published beside the verified score as evidence. This is the
    /// number that can be farmed from one account for nothing, which is why it
    /// no longer decides anything on its own.
    ///
    /// Saturating, because a score is compared and never summed onward, and
    /// an overflow that wrapped to a small number would rank a viral reply
    /// last.
    #[must_use]
    pub const fn raw_score(&self) -> u64 {
        self.reposts
            .saturating_mul(REPOST_WEIGHT)
            .saturating_add(self.quotes.saturating_mul(QUOTE_WEIGHT))
            .saturating_add(self.likes.saturating_mul(LIKE_WEIGHT))
            .saturating_add(self.replies.saturating_mul(REPLY_WEIGHT))
    }
}

/// One summoned reply, as an entry in the week it was posted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// The bot's reply, on the platform.
    pub reply_id: String,
    /// Who summoned it. The entrant, as the numeric account id.
    pub summoner: String,
    /// The entrant's handle, when the week close read one.
    ///
    /// Defaulted for the reason [`crate::ledger::Winner::handle`] gives at
    /// length: a record that fails to parse is skipped rather than reported,
    /// so a field added without a default deletes history quietly.
    #[serde(default)]
    pub handle: Option<String>,
    /// The coin it was about.
    pub mint: String,
    /// When the reply was posted, as seconds since the epoch.
    pub at: u64,
    /// The reply's public metrics at week close.
    pub metrics: Metrics,
}

/// What the caller knows about one account, supplied rather than looked up.
///
/// Pure crates do not read the platform. The week-close job reads each
/// entrant's account once and passes what it found; an entrant it did not look
/// up has no standing here and is excluded as unknown.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Standing {
    /// How old the account was at week close, in days, if it could be read.
    pub account_age_days: Option<u32>,
    /// Whether the admission gate refused this account at any point in the week.
    pub refused_this_week: bool,
    /// The most recent week this account won, if any.
    pub last_win: Option<Week>,
}

/// The published exclusions, as parameters so the rule is one function.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rules {
    /// Every account the operator controls. None of them can win.
    ///
    /// A **set**, because the operator is more than one account: the bot posts
    /// as itself and is managed from a person's own account, and a prize paid
    /// to either is the operator paying themselves out of a pool the public is
    /// told is theirs. Both were reachable before 2026-09-06 -- only the bot's
    /// own id was excluded, so the managing account could have entered and won.
    ///
    /// Ordered, so the published rule renders the same way twice.
    pub operators: BTreeSet<String>,
    /// Accounts younger than this at week close do not count.
    pub min_account_age_days: u32,
    /// An engager younger than this is not counted toward a verified score.
    ///
    /// The same floor entrants already meet, applied symmetrically -- an
    /// account too new to enter is too new to vote. Published, so it is a rule
    /// a reader can check rather than a filter they have to trust.
    ///
    /// `serde(default)` reads a record written before the verified rule
    /// existed; zero there is correct, because those weeks were scored with no
    /// engager floor at all and the record must say what actually happened.
    #[serde(default)]
    pub min_engager_age_days: u32,
    /// A winner cannot win again until this many further weeks have closed.
    ///
    /// Three means one win in any four consecutive weeks: the winner of week
    /// `W` is excluded in `W+1`, `W+2` and `W+3`. Design 0009 section 9 took
    /// this from the forwarded notes as the cheap answer to the obvious farm.
    pub cooldown_weeks: u64,
}

impl Rules {
    /// The rule as printed on the public site.
    ///
    /// A function rather than a `Default`, so a caller states that it wants
    /// the published rule and the operator's handle is not invented here.
    ///
    /// Takes anything iterable so the caller cannot accidentally pass one id
    /// where it meant several -- the mistake this signature exists to stop.
    #[must_use]
    pub fn published<I, S>(operators: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            operators: operators.into_iter().map(Into::into).collect(),
            min_account_age_days: 30,
            min_engager_age_days: 30,
            cooldown_weeks: 3,
        }
    }

    /// Whether this account is one the operator controls.
    #[must_use]
    pub fn is_operator(&self, summoner: &str) -> bool {
        self.operators.iter().any(|o| o == summoner)
    }
}

/// Why an entry does not count this week.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Excluded {
    /// An account the operator controls. The bot's own, or one that manages it.
    Operator,
    /// The account was younger than the rule allows at week close.
    AccountTooNew {
        /// Its age, in days.
        days: u32,
    },
    /// The account's age could not be established. Unknown is not eligible.
    AccountAgeUnknown,
    /// The admission gate refused this account during the week.
    RefusedThisWeek,
    /// The account won too recently.
    WonWithinCooldown {
        /// The week it won.
        won: Week,
    },
    /// The platform returned no metrics for the reply at week close -- deleted,
    /// hidden, or not returned -- so it has no score. Not a score of zero: an
    /// entry that could not be read is excluded and says so, rather than
    /// ranked last as if nobody engaged (rule 9). Applied by the week-close
    /// job before ranking, since the rule here scores what it is given.
    Unscored,
}

/// An entry that counts, with its score.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ranked {
    /// The entry.
    pub entry: Entry,
    /// Its score under the published rule.
    pub score: u64,
}

/// The week's entries, ranked, with the exclusions beside them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ranking {
    /// Best first. Ties go to the earlier reply, then to the reply id, so the
    /// order is total and two runs agree.
    pub ranked: Vec<Ranked>,
    /// Every entry that did not count, with the reason.
    pub excluded: Vec<(Entry, Excluded)>,
}

impl Ranking {
    /// The winner: the first ranked entry, if any counted.
    #[must_use]
    pub fn winner(&self) -> Option<&Ranked> {
        self.ranked.first()
    }
}

/// Applies the published rule to one week's entries.
///
/// `standings` is what the caller established about each summoner. An entry
/// whose summoner has no standing is excluded as unknown rather than admitted.
/// Entries from outside `week` are excluded from the input by the caller; this
/// function trusts the week it was given and does not re-derive it, because
/// the reply's timestamp is the caller's evidence and re-checking it here would
/// only hide a caller that passed the wrong week.
#[must_use]
pub fn rank(
    week: Week,
    entries: &[Entry],
    standings: &BTreeMap<String, Standing>,
    rules: &Rules,
) -> Ranking {
    let mut ranking = Ranking::default();
    for entry in entries {
        match exclusion(week, entry, standings.get(&entry.summoner), rules) {
            Some(why) => ranking.excluded.push((entry.clone(), why)),
            None => ranking.ranked.push(Ranked {
                entry: entry.clone(),
                score: entry.metrics.score(),
            }),
        }
    }
    // Highest score first; earlier reply breaks a tie; the id makes the order
    // total. `sort_by` is stable, but a total key is what makes two runs over
    // differently ordered input agree.
    ranking.ranked.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.entry.at.cmp(&b.entry.at))
            .then_with(|| a.entry.reply_id.cmp(&b.entry.reply_id))
    });
    ranking
}

/// Why one entry is excluded, or `None` when it counts.
///
/// The checks run in the order the published rule lists them, so an entry that
/// fails two is reported with the first, and the order is fixed rather than
/// incidental.
fn exclusion(
    week: Week,
    entry: &Entry,
    standing: Option<&Standing>,
    rules: &Rules,
) -> Option<Excluded> {
    if rules.is_operator(&entry.summoner) {
        return Some(Excluded::Operator);
    }
    let Some(standing) = standing else {
        return Some(Excluded::AccountAgeUnknown);
    };
    match standing.account_age_days {
        None => return Some(Excluded::AccountAgeUnknown),
        Some(days) if days < rules.min_account_age_days => {
            return Some(Excluded::AccountTooNew { days });
        }
        Some(_) => {}
    }
    if standing.refused_this_week {
        return Some(Excluded::RefusedThisWeek);
    }
    if let Some(won) = standing.last_win
        && won.0 < week.0
        && week.0 - won.0 <= rules.cooldown_weeks
    {
        return Some(Excluded::WonWithinCooldown { won });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, summoner: &str, at: u64, metrics: Metrics) -> Entry {
        Entry {
            reply_id: id.to_owned(),
            summoner: summoner.to_owned(),
            handle: None,
            mint: "M".to_owned(),
            at,
            metrics,
        }
    }

    fn old_enough() -> Standing {
        Standing {
            account_age_days: Some(400),
            refused_this_week: false,
            last_win: None,
        }
    }

    fn standings(names: &[&str]) -> BTreeMap<String, Standing> {
        names
            .iter()
            .map(|n| ((*n).to_owned(), old_enough()))
            .collect()
    }

    const WEEK: Week = Week(2958);

    #[test]
    fn the_score_is_the_published_weights_and_nothing_else() {
        // Each weight pinned by a metric that is 1 while the rest are 0, so a
        // swapped or dropped weight changes exactly one assertion.
        let one = |reposts, quotes, likes, replies| {
            Metrics {
                reposts,
                quotes,
                likes,
                replies,
                verified: None,
            }
            .raw_score()
        };
        assert_eq!(one(1, 0, 0, 0), 3, "a repost is worth three");
        assert_eq!(one(0, 1, 0, 0), 3, "a quote is worth three");
        assert_eq!(one(0, 0, 1, 0), 1, "a like is worth one");
        assert_eq!(one(0, 0, 0, 1), 1, "a reply is worth one");
        assert_eq!(one(2, 1, 5, 3), 6 + 3 + 5 + 3);
        assert_eq!(Metrics::default().raw_score(), 0);
    }

    #[test]
    fn a_viral_reply_does_not_wrap_to_last_place() {
        let m = Metrics {
            reposts: u64::MAX,
            quotes: u64::MAX,
            likes: 0,
            replies: 0,
            verified: None,
        };
        assert_eq!(m.score(), u64::MAX);
        assert_eq!(
            Verified {
                reposts: u64::MAX,
                quoters: u64::MAX,
                ..Verified::default()
            }
            .score(),
            u64::MAX,
            "and the verified score saturates the same way"
        );
    }

    #[test]
    fn the_highest_score_wins_and_a_tie_goes_to_the_earlier_reply() {
        // Re-apply the bug by comparing `a.score` to `b.score` instead of the
        // reverse: the lowest score then wins. Re-apply the second by dropping
        // the `at` tiebreak: the winner between equals is whichever the input
        // listed first, which is whichever the platform returned first.
        let entries = vec![
            entry(
                "late-high",
                "a",
                200,
                Metrics {
                    likes: 10,
                    ..Metrics::default()
                },
            ),
            entry(
                "early-high",
                "b",
                100,
                Metrics {
                    likes: 10,
                    ..Metrics::default()
                },
            ),
            entry(
                "low",
                "c",
                50,
                Metrics {
                    likes: 1,
                    ..Metrics::default()
                },
            ),
        ];
        let ranking = rank(
            WEEK,
            &entries,
            &standings(&["a", "b", "c"]),
            &Rules::published(["radar"]),
        );
        let order: Vec<&str> = ranking
            .ranked
            .iter()
            .map(|r| r.entry.reply_id.as_str())
            .collect();
        assert_eq!(order, ["early-high", "late-high", "low"]);
        assert_eq!(ranking.winner().expect("a winner").entry.summoner, "b");
        assert!(ranking.excluded.is_empty());
    }

    #[test]
    fn the_order_is_total_so_two_runs_over_shuffled_input_agree() {
        // Same score, same second: the id decides, and the same id wins
        // whichever order the platform returned the replies in.
        let a = entry(
            "a",
            "x",
            100,
            Metrics {
                likes: 1,
                ..Metrics::default()
            },
        );
        let b = entry(
            "b",
            "y",
            100,
            Metrics {
                likes: 1,
                ..Metrics::default()
            },
        );
        let st = standings(&["x", "y"]);
        let rules = Rules::published(["radar"]);
        let first = rank(WEEK, &[a.clone(), b.clone()], &st, &rules);
        let second = rank(WEEK, &[b, a], &st, &rules);
        assert_eq!(first, second);
        assert_eq!(first.winner().expect("a winner").entry.reply_id, "a");
    }

    #[test]
    fn every_published_exclusion_fires_and_says_why() {
        let rules = Rules::published(["radar"]);
        let mut st = standings(&["fine", "young", "refused", "recent-winner", "old-winner"]);
        st.get_mut("young").expect("young").account_age_days = Some(29);
        st.get_mut("refused").expect("refused").refused_this_week = true;
        st.get_mut("recent-winner").expect("recent").last_win = Some(Week(WEEK.0 - 3));
        st.get_mut("old-winner").expect("old").last_win = Some(Week(WEEK.0 - 4));
        st.insert(
            "unknown-age".to_owned(),
            Standing {
                account_age_days: None,
                ..old_enough()
            },
        );
        let big = Metrics {
            reposts: 100,
            ..Metrics::default()
        };
        let entries = vec![
            entry("op", "radar", 1, big),
            entry("young", "young", 2, big),
            entry("refused", "refused", 3, big),
            entry("recent", "recent-winner", 4, big),
            entry("unknown", "unknown-age", 5, big),
            entry("never-looked-up", "stranger", 6, big),
            entry("old", "old-winner", 7, Metrics::default()),
            entry("fine", "fine", 8, Metrics::default()),
        ];
        let ranking = rank(WEEK, &entries, &st, &rules);

        let why = |id: &str| {
            ranking
                .excluded
                .iter()
                .find(|(e, _)| e.reply_id == id)
                .map(|(_, w)| w.clone())
        };
        assert_eq!(why("op"), Some(Excluded::Operator));
        assert_eq!(why("young"), Some(Excluded::AccountTooNew { days: 29 }));
        assert_eq!(why("refused"), Some(Excluded::RefusedThisWeek));
        assert_eq!(
            why("recent"),
            Some(Excluded::WonWithinCooldown {
                won: Week(WEEK.0 - 3)
            })
        );
        // Rule 9 twice: an age the caller could not read, and an account the
        // caller never looked up, are both unknown and neither is eligible.
        assert_eq!(why("unknown"), Some(Excluded::AccountAgeUnknown));
        assert_eq!(why("never-looked-up"), Some(Excluded::AccountAgeUnknown));

        // The ones that count: a winner from four weeks ago is eligible again,
        // and the entries with the biggest scores were all excluded, so the
        // winner is decided among the honest ones.
        let counted: Vec<&str> = ranking
            .ranked
            .iter()
            .map(|r| r.entry.reply_id.as_str())
            .collect();
        assert_eq!(counted, ["old", "fine"], "{counted:?}");
    }

    #[test]
    fn the_cooldown_is_three_further_weeks_exactly() {
        // The winner of week W is excluded in W+1, W+2 and W+3 and eligible
        // in W+4. Both edges pinned: `<=` becoming `<` frees them a week
        // early, and a cooldown counted from the wrong end frees nobody.
        let rules = Rules::published(["radar"]);
        for (weeks_ago, eligible) in [(1, false), (2, false), (3, false), (4, true)] {
            let mut st = standings(&["w"]);
            st.get_mut("w").expect("w").last_win = Some(Week(WEEK.0 - weeks_ago));
            let ranking = rank(WEEK, &[entry("r", "w", 1, Metrics::default())], &st, &rules);
            assert_eq!(
                ranking.ranked.len() == 1,
                eligible,
                "a win {weeks_ago} weeks ago: eligible should be {eligible}"
            );
        }
        // A win recorded in this week or a later one is not a past win and
        // does not exclude: a ledger that is ahead of the clock is a bug
        // elsewhere, and this rule must not turn it into a silent exclusion.
        let mut st = standings(&["w"]);
        st.get_mut("w").expect("w").last_win = Some(WEEK);
        assert_eq!(
            rank(WEEK, &[entry("r", "w", 1, Metrics::default())], &st, &rules)
                .ranked
                .len(),
            1
        );
    }

    #[test]
    fn the_age_bar_is_under_thirty_days_so_thirty_counts() {
        // The site says "accounts under 30 days are excluded". Under, not
        // up to: an account exactly 30 days old counts and one of 29 does
        // not. Both edges pinned because CI's mutants run on 2026-09-05
        // turned `<` into `<=` and every test still passed -- the rule had
        // moved a day and nothing said so.
        let rules = Rules::published(["radar"]);
        for (days, eligible) in [(29, false), (30, true), (31, true)] {
            let mut st = standings(&["w"]);
            st.get_mut("w").expect("w").account_age_days = Some(days);
            let ranking = rank(WEEK, &[entry("r", "w", 1, Metrics::default())], &st, &rules);
            assert_eq!(
                ranking.ranked.len() == 1,
                eligible,
                "an account {days} days old: eligible should be {eligible}"
            );
            if !eligible {
                assert_eq!(ranking.excluded[0].1, Excluded::AccountTooNew { days });
            }
        }
    }

    #[test]
    fn a_week_with_nothing_counted_has_no_winner_rather_than_a_default_one() {
        let ranking = rank(WEEK, &[], &BTreeMap::new(), &Rules::published(["radar"]));
        assert!(ranking.winner().is_none());
        assert!(ranking.ranked.is_empty());
    }

    #[test]
    fn every_account_the_operator_controls_is_excluded_not_just_the_bot() {
        // The bot posts as itself and is managed from a person's own account.
        // Only the bot's own id was excluded before 2026-09-06, so the managing
        // account could have entered and won -- the operator paying themselves
        // out of a pool the public is told is theirs.
        //
        // Re-apply the bug by passing only the bot's id and the second
        // assertion fails.
        let rules = Rules::published(["1889496824328880128", "2005812292693483520"]);
        assert!(rules.is_operator("1889496824328880128"), "the bot itself");
        assert!(
            rules.is_operator("2005812292693483520"),
            "the managing account"
        );
        assert!(!rules.is_operator("999"), "anybody else still enters");

        // And the rule actually applies it, rather than the helper agreeing
        // with itself.
        let entry = entry("r1", "2005812292693483520", 10, Metrics::default());
        let standings = [(
            "2005812292693483520".to_owned(),
            Standing {
                account_age_days: Some(900),
                refused_this_week: false,
                last_win: None,
            },
        )]
        .into_iter()
        .collect();
        let ranking = rank(WEEK, &[entry], &standings, &rules);
        assert!(ranking.ranked.is_empty(), "the managing account cannot win");
        assert_eq!(ranking.excluded[0].1, Excluded::Operator);
    }

    #[test]
    fn the_published_rule_is_what_the_site_prints() {
        // The site says: accounts under 30 days are excluded. A rule that
        // drifted from the page would be a rule nobody agreed to.
        let rules = Rules::published(["radar"]);
        assert_eq!(rules.min_account_age_days, 30);
        assert_eq!(rules.cooldown_weeks, 3);
        assert!(rules.is_operator("radar"));
        assert!(!rules.is_operator("somebody-else"));
        assert_eq!(
            (REPOST_WEIGHT, QUOTE_WEIGHT, LIKE_WEIGHT, REPLY_WEIGHT),
            (3, 3, 1, 1)
        );
    }

    #[test]
    fn one_account_can_farm_the_raw_rule_for_nothing_and_not_the_verified_one() {
        // Research 0029, S16, and the whole reason `Verified` exists. On X a
        // single account can quote or reply to the same post without limit;
        // only a repost and a like are one-per-account. So under the raw rule
        // one account with thirty quotes beats a reply that ten real people
        // reposted, and it costs nothing.
        //
        // Re-apply by making `score()` return `raw_score()` unconditionally:
        // the farm wins and this fails.
        let farm = Metrics {
            reposts: 0,
            quotes: 30,
            likes: 0,
            replies: 30,
            // One account, so one quoter, and it reposted and liked nothing.
            verified: Some(Verified {
                reposts: 0,
                quoters: 1,
                likes: 0,
                engagers: 1,
                engagers_under_age: 0,
            }),
        };
        let real = Metrics {
            reposts: 10,
            quotes: 0,
            likes: 0,
            replies: 0,
            verified: Some(Verified {
                reposts: 10,
                quoters: 0,
                likes: 0,
                engagers: 10,
                engagers_under_age: 0,
            }),
        };

        assert!(
            farm.raw_score() > real.raw_score(),
            "the raw rule is the thing being fixed: {} vs {}",
            farm.raw_score(),
            real.raw_score()
        );
        assert!(
            real.score() > farm.score(),
            "the verified rule ranks ten reposters above one prolific quoter: {} vs {}",
            real.score(),
            farm.score()
        );
    }

    #[test]
    fn a_verified_score_is_never_above_the_raw_one_it_came_from() {
        // The inequality the walk's stopping rule rests on. `close_if_due`
        // scans down the ranking and stops when the best verified score is at
        // least the next entry's RAW score -- which is only sound if raw is an
        // upper bound on verified. It is, structurally: raw counts every
        // account and adds replies, verified counts a subset and drops them.
        //
        // Asserted over a spread of shapes rather than argued, because the day
        // somebody adds a weight to the verified side without adding it to raw
        // is the day the ranking silently stops being correct.
        for (reposts, quotes, likes, replies) in
            [(0, 0, 0, 0), (1, 0, 0, 0), (10, 30, 100, 50), (7, 3, 0, 9)]
        {
            // The most generous scan possible: every raw repost was a distinct
            // old account, every raw quote a distinct old quoter, every like
            // real. Nothing a real scan returns can exceed this.
            let m = Metrics {
                reposts,
                quotes,
                likes,
                replies,
                verified: Some(Verified {
                    reposts,
                    quoters: quotes,
                    likes,
                    engagers: reposts + quotes + likes,
                    engagers_under_age: 0,
                }),
            };
            assert!(
                m.score() <= m.raw_score(),
                "verified {} exceeded raw {} for {reposts}/{quotes}/{likes}/{replies}",
                m.score(),
                m.raw_score()
            );
        }
    }

    #[test]
    fn an_unscanned_entry_is_scored_raw_and_an_empty_scan_is_not_the_same_thing() {
        // Rule 9 in the place it decides a ranking. The walk stops as soon as
        // arithmetic says nothing below can win, so most entries in a busy week
        // are never scanned -- `None` means "not looked at", and scoring those
        // as zero would rank every unscanned entry last.
        //
        // A scan that ran and found nobody is `Some` full of zeroes, and that
        // really is a score of zero.
        let raw = Metrics {
            reposts: 2,
            quotes: 1,
            likes: 5,
            replies: 3,
            verified: None,
        };
        assert_eq!(raw.score(), raw.raw_score());
        assert_eq!(raw.score(), 6 + 3 + 5 + 3);

        let scanned_empty = Metrics {
            verified: Some(Verified::default()),
            ..raw
        };
        assert_eq!(
            scanned_empty.score(),
            0,
            "a scan that found nobody is a zero, not a fallback to raw"
        );
        assert_eq!(
            scanned_empty.raw_score(),
            raw.raw_score(),
            "and the raw numbers are still published beside it as evidence"
        );
    }

    #[test]
    fn the_published_rule_carries_both_age_floors() {
        // An account too new to enter is too new to vote. Applied
        // symmetrically, and published, so it is a rule a reader can check
        // rather than a filter they have to trust.
        let rules = Rules::published(["1"]);
        assert_eq!(rules.min_account_age_days, 30);
        assert_eq!(rules.min_engager_age_days, 30);
    }

    #[test]
    fn a_record_written_before_the_verified_rule_still_reads() {
        // The migration. `min_engager_age_days` is absent from every week
        // closed before 2026-09-06, and zero is the honest value there: those
        // weeks were scored with no engager floor at all, and the record has to
        // say what happened rather than what the rule says now.
        let old = r#"{"operators":["1","2"],"min_account_age_days":30,"cooldown_weeks":3}"#;
        let rules: Rules = serde_json::from_str(old).expect("an old record still reads");
        assert_eq!(rules.min_engager_age_days, 0);
        assert_eq!(rules.min_account_age_days, 30);

        // And a metrics blob from before the scan existed reads as unscanned,
        // not as scanned-and-empty.
        let m: Metrics = serde_json::from_str(r#"{"reposts":2,"quotes":1,"likes":5,"replies":3}"#)
            .expect("an old entry still reads");
        assert_eq!(m.verified, None);
        assert_eq!(m.score(), m.raw_score());
    }
}
