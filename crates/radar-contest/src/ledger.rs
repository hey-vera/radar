// SPDX-License-Identifier: Apache-2.0
//! One week's record: entries, scores, winner, claim, payout.
//!
//! # Published, one file per week
//!
//! Design 0007 C2 writes one JSON document per week and the public site reads
//! it, never the store. This is that document's shape. Everything a reader
//! needs to check the result is in it: the entries and their metrics, the
//! scores, every exclusion with its reason, the winner, the address they
//! claimed with, and the transaction that paid them.
//!
//! # The payout policy lives here, and it is three lines
//!
//! `radar-payout` signs from a hot key whose blast radius is one week of
//! creator fees (ADR 0013). What keeps it that small is [`Payout::permitted`]:
//! the recipient must be the address the ledger's winner claimed with, the
//! amount may not exceed what was collected, and a week is paid at most once.
//! Pure and tested here, so the binary calls a function rather than carrying
//! its own copy of the rule -- and so that each refusal can be proved by
//! re-applying the bug without a key in the room.

use serde::{Deserialize, Serialize};

use crate::score::{Ranking, Rules};
use crate::week::{SECONDS_PER_DAY, Week};

/// How long the winner has to reply with an address before the prize rolls
/// into the next week. Design 0007 section 6.2: seven days.
pub const CLAIM_WINDOW_SECONDS: u64 = 7 * SECONDS_PER_DAY;

/// The week's winner, as decided by the rule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Winner {
    /// Who, as the numeric account id a mention carries.
    pub summoner: String,
    /// The reply that won.
    pub reply_id: String,
    /// Its score.
    pub score: u64,
    /// The handle, when the close read one.
    ///
    /// `Option` because it arrives from a platform call that can fail while the
    /// rest of the close succeeds, and because every record written before
    /// 2026-09-06 has no such field.
    ///
    /// `serde(default)` is **redundant on an `Option` and kept as a statement of
    /// intent**, which is worth saying plainly because the first draft of this
    /// comment claimed the attribute was load-bearing. It is not: serde already
    /// maps a missing `Option` field to `None`, established by probe rather than
    /// recalled. What it buys is a visible marker that this field is expected to
    /// be absent in older records, and it starts mattering the day somebody
    /// changes the type to a bare `String`.
    ///
    /// The hazard it marks is real, and it is finding S11: [`records_in`] skips
    /// a file it cannot parse, without warning and without failing, so a
    /// **required** field added to a record would make old weeks vanish from the
    /// leaderboard and from the cooldown that reads them — quietly freeing a
    /// past winner to win again. The test named
    /// `the_record_production_wrote_before_these_fields_existed_still_parses` is
    /// the enforcement; this attribute is only the note.
    #[serde(default)]
    pub handle: Option<String>,
}

/// The address the winner asked to be paid at, and the reply that said so.
///
/// The claim is a public reply from the winner's own account, parsed with the
/// same strict rule as a mention (design 0007 C3), which is why it carries the
/// reply id: the link between account and address is on the platform for
/// anyone to see.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claim {
    /// The Solana address, as text. Parsed and checked by the caller before it
    /// is written here; this crate stores what was accepted.
    pub address: String,
    /// The reply it was read from.
    pub reply_id: String,
    /// When, as seconds since the epoch.
    pub at: u64,
}

/// A payment that was made.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Payout {
    /// To whom.
    pub recipient: String,
    /// How much.
    pub lamports: u64,
    /// The transaction signature, so the ledger's claim is checkable on chain.
    pub signature: String,
    /// When, as seconds since the epoch.
    pub at: u64,
}

/// Why a payout is refused.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Refusal {
    /// Nobody won this week.
    NoWinner,
    /// The winner has not claimed, so there is no address to pay.
    Unclaimed,
    /// The recipient is not the address the winner claimed with.
    WrongRecipient,
    /// More than the week collected.
    AboveCollected {
        /// What was collected, in lamports.
        collected: u64,
    },
    /// This week has already been paid.
    AlreadyPaid {
        /// The signature of the payment that was made.
        signature: String,
    },
    /// The operator voided this week. It pays nobody and the pool rolls over.
    Voided {
        /// Their reason, as published.
        reason: String,
    },
    /// The claimed address is not a wallet.
    ///
    /// Defence in depth: `try_claim` already refuses one at claim time, where
    /// the winner can act on it. This is the second check, at the last moment
    /// before a signature, because a mint and a wallet are the same shape and
    /// what is on the other side of this is money leaving.
    NotAWallet {
        /// What owns it, or `None` when the owner could not be read.
        owner: Option<String>,
    },
    /// The week collected less than the floor, so the pool rolls over.
    ///
    /// Design 0007 J2 and design 0009 L4 both say a floor with rollover, and
    /// the code had none until 2026-09-06. Without it a week that collected
    /// a few thousand lamports pays them out: the transaction fee is a
    /// meaningful share of the prize, the winner receives an amount not worth
    /// the click, and the pool that should have been building is spent.
    ///
    /// **Not an error.** The week stays unpaid and claimable-looking, the
    /// prize rolls into the next week, and the history page says so -- which
    /// is the same shape as `Unclaimed` and deliberately so.
    BelowFloor {
        /// The floor in force, in lamports.
        floor: u64,
        /// What the week actually collected.
        collected: u64,
    },
}

/// A week the operator voided, and why.
///
/// # Why a lever exists at all
///
/// Design 0007 §6.2 promised that *"if the first winner is obviously bought,
/// the rule changes and the change is recorded."* Changing a rule takes a
/// deploy and an argument; the prize is claimable in seven days. Without this
/// the only options in that week are pay a farm or let the week look
/// unclaimed, and the second is a lie a reader cannot see through.
///
/// # Why it is visible rather than quiet
///
/// A voided week **says so on the page, with the operator's reason in their own
/// words**. Design 0011's whole argument is that an exclusion at payout is a
/// private correction to a public error; this is the same principle applied to
/// the one action that is unavoidably a judgement. It cannot be used to tidy a
/// week away, because using it publishes the fact that it was used.
///
/// It does not rewrite the ranking. The scores stand, the evidence stands, the
/// winner is still named — the week simply pays nobody and the pool rolls over.
/// A reader who disagrees can see exactly what the operator saw.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Voided {
    /// When the operator voided it, seconds since the epoch.
    pub at: u64,
    /// Why, in the operator's own words. Published verbatim.
    pub reason: String,
}

/// The creator vault, as last read from the chain.
///
/// Written by the timer that reads the vault balance (design 0008 phase 3,
/// which needs the token to exist) and read by the public pool page. Absent
/// means **no token yet**, which the page renders as a sentence and never as
/// a balance of zero.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vault {
    /// The vault's address, base58.
    pub address: String,
    /// Its balance when read.
    pub lamports: u64,
    /// When it was read, seconds since the epoch.
    pub measured_at: u64,
}

impl Vault {
    /// The JSON the site reads.
    ///
    /// # Errors
    ///
    /// Only if a value cannot be serialised, which no value here can fail to be.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// A reading read back.
    ///
    /// # Errors
    ///
    /// When the text is not a vault reading.
    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }
}

/// One week's record, as published.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// Which week.
    pub week: Week,
    /// When it opened, seconds since the epoch. Derived from `week`; written
    /// out so a reader of the JSON does not have to know the arithmetic.
    pub opened_at: u64,
    /// When it closed.
    pub closed_at: u64,
    /// The ranking, entries and exclusions both.
    pub ranking: Ranking,
    /// The winner, if anything counted.
    pub winner: Option<Winner>,
    /// The post the winner must reply to in order to claim.
    ///
    /// Written back after the week closes, once the account has posted it under
    /// its own winning reply (design 0007 §6.2). `None` means the prompt has not
    /// been posted — in a dry run it never is, and no claim can be made, which
    /// is correct, because no winning reply was published for anyone to see
    /// either.
    ///
    /// **This field is what stops a coin's mint address being paid the prize.**
    /// Before it existed, `try_claim` accepted any mint-shaped string in any
    /// mention by the winner inside the claim window, and a mint is such a
    /// string — so a winner who summoned the bot about a coin during their own
    /// claim week had that coin's mint recorded as their payout address, and
    /// `Payout::permitted` would have approved paying it. See `try_claim`.
    #[serde(default)]
    pub claim_prompt: Option<String>,
    /// The winner's claim, once made.
    pub claim: Option<Claim>,
    /// The operator voided this week, and why.
    ///
    /// `None` is the ordinary case. `Some` means the week pays nobody whatever
    /// the ranking says, and the reason is published beside the evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voided: Option<Voided>,
    /// The rule this week was scored under.
    ///
    /// **Written into the record rather than looked up**, because the rule
    /// changes and a closed week has to stay checkable against the rule that
    /// actually decided it. A reader asking why an entry placed where it did
    /// gets an answer from this file alone.
    ///
    /// `None` on a week closed before 2026-09-06, when nothing recorded it.
    /// That is unknown rather than "the current rule", and the history page
    /// says so: rule 9, and the difference matters to somebody disputing a
    /// placing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<Rules>,
    /// The payment, once made.
    pub payout: Option<Payout>,
}

impl Record {
    /// A closed week's record from its ranking, unclaimed and unpaid.
    #[must_use]
    pub fn close(week: Week, ranking: Ranking, rule: &Rules) -> Self {
        let winner = ranking.winner().map(|r| Winner {
            summoner: r.entry.summoner.clone(),
            reply_id: r.entry.reply_id.clone(),
            score: r.score,
            handle: r.entry.handle.clone(),
        });
        Self {
            week,
            opened_at: week.opens_at(),
            closed_at: week.closes_at(),
            ranking,
            winner,
            claim_prompt: None,
            claim: None,
            voided: None,
            rule: Some(rule.clone()),
            payout: None,
        }
    }

    /// The last moment a claim is accepted.
    ///
    /// After this the prize rolls into the next week (design 0007 J2 and
    /// section 6.2). Exclusive, like a week's close.
    #[must_use]
    pub const fn claim_window_closes_at(&self) -> u64 {
        self.closed_at + CLAIM_WINDOW_SECONDS
    }

    /// Whether a claim made at `now` is in time.
    #[must_use]
    pub const fn accepts_claim_at(&self, now: u64) -> bool {
        now >= self.closed_at && now < self.claim_window_closes_at()
    }

    /// The JSON the site reads.
    ///
    /// # Errors
    ///
    /// Only if a value cannot be serialised, which no value here can fail to be.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// A record read back.
    ///
    /// # Errors
    ///
    /// When the text is not a record. A torn or hand-edited file is refused
    /// rather than partly read: the ledger is the evidence, and half of it is
    /// not evidence.
    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }
}

/// Every week record in a directory, in no particular order.
///
/// A record is a file named `<week>.json` where `<week>` is the week number;
/// both halves of the name are required, so a backup copy, a notes file or a
/// numbered file with no extension is not a record. A file that does not parse
/// is skipped rather than failing the read: a torn write is not evidence, and
/// the weeks either side of it still are.
///
/// The one place this crate touches a disk, and only to read what it wrote.
#[must_use]
pub fn records_in(dir: &std::path::Path) -> Vec<Record> {
    let Ok(listing) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    listing
        .flatten()
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.ends_with(".json") && name.trim_end_matches(".json").parse::<u64>().is_ok()
        })
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .filter_map(|text| Record::from_json(&text).ok())
        .collect()
}

impl Payout {
    /// Whether paying `lamports` to `recipient` for this week is permitted.
    ///
    /// The three lines that bound the hot key's blast radius, in the order
    /// they are cheapest to check. `collected` is what the week's creator fee
    /// amounted to, read from the chain by the caller.
    ///
    /// # Errors
    ///
    /// The refusal, which the caller records and never argues with.
    pub fn permitted(
        record: &Record,
        recipient: &str,
        lamports: u64,
        collected: u64,
        floor: u64,
    ) -> Result<(), Refusal> {
        if let Some(paid) = &record.payout {
            return Err(Refusal::AlreadyPaid {
                signature: paid.signature.clone(),
            });
        }
        // Checked before the winner, the claim and the amount, because a
        // voided week is not a week with a problem to work around -- it is a
        // week the operator has already decided pays nobody. Reporting
        // `Unclaimed` for it would send somebody looking for a claim.
        if let Some(voided) = &record.voided {
            return Err(Refusal::Voided {
                reason: voided.reason.clone(),
            });
        }
        if record.winner.is_none() {
            return Err(Refusal::NoWinner);
        }
        let Some(claim) = &record.claim else {
            return Err(Refusal::Unclaimed);
        };
        if claim.address != recipient {
            return Err(Refusal::WrongRecipient);
        }
        if lamports > collected {
            return Err(Refusal::AboveCollected { collected });
        }
        // Last, and deliberately: every refusal above is about whether this
        // week may be paid at all, and this one is only about whether it is
        // worth paying yet. Checking it earlier would report "below the floor"
        // for a week that is voided, unpaid twice, or claimed by the wrong
        // address -- three answers that are more urgent and more true.
        if collected < floor {
            return Err(Refusal::BelowFloor { floor, collected });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::score::{Entry, Metrics, Ranked};

    /// No floor: what these tests are about is the other five refusals, and a
    /// floor would make every one of them depend on an amount that is not the
    /// thing under test.
    const NO_FLOOR: u64 = 0;

    const WEEK: Week = Week(2958);

    #[test]
    fn a_voided_week_pays_nobody_and_says_so_before_anything_else() {
        // Checked ahead of the winner, the claim and the amount. A voided week
        // is not a week with a problem to work around -- the operator has
        // already decided it pays nobody -- and reporting `Unclaimed` for one
        // would send somebody looking for a claim that is not the point.
        //
        // Re-apply by moving the check below the claim: an unclaimed voided
        // week reports `Unclaimed` and the reason never reaches anybody.
        let mut record = Record::close(WEEK, ranking_with_winner(), &Rules::published(["op"]));
        record.voided = Some(Voided {
            at: 1_788_000_000,
            reason: "every point came from six accounts made that morning".to_owned(),
        });

        // No claim at all: still Voided, not Unclaimed.
        match Payout::permitted(&record, "somebody", 1, 1_000, NO_FLOOR) {
            Err(Refusal::Voided { reason }) => {
                assert!(reason.contains("six accounts"), "{reason}");
            }
            other => panic!("expected Voided, got {other:?}"),
        }

        // And with a perfectly good claim, it still pays nobody.
        record.claim = Some(Claim {
            address: "somebody".to_owned(),
            at: 1_788_000_100,
            reply_id: "r9".to_owned(),
        });
        assert!(matches!(
            Payout::permitted(&record, "somebody", 1, 1_000, NO_FLOOR),
            Err(Refusal::Voided { .. })
        ));
    }

    #[test]
    fn a_week_that_was_already_paid_reports_that_rather_than_the_void() {
        // Order between the two. Money that already left is the more useful
        // fact, and it is the one that cannot be undone by editing a file.
        let mut record = Record::close(WEEK, ranking_with_winner(), &Rules::published(["op"]));
        record.payout = Some(Payout {
            recipient: "somebody".to_owned(),
            lamports: 10,
            signature: "sig".to_owned(),
            at: 1_788_000_000,
        });
        record.voided = Some(Voided {
            at: 1_788_000_100,
            reason: "too late".to_owned(),
        });
        assert!(matches!(
            Payout::permitted(&record, "somebody", 1, 1_000, NO_FLOOR),
            Err(Refusal::AlreadyPaid { .. })
        ));
    }

    #[test]
    fn a_record_written_before_the_veto_existed_is_not_voided() {
        // Rule 9's direction here is the cheap one: absent means the operator
        // never voided it, which is true of every week closed before today.
        let old = Record::close(WEEK, Ranking::default(), &Rules::published(["op"]));
        let json = old.to_json().expect("json");
        assert!(
            !json.contains("voided"),
            "an unvoided week does not carry an empty field: {json}"
        );
        let back: Record = serde_json::from_str(&json).expect("reads");
        assert_eq!(back.voided, None);
    }

    #[test]
    fn only_a_numbered_json_file_in_the_directory_is_a_record() {
        // Both halves of the name rule. CI's mutants turned the `&&` into `||`
        // and a numbered file with no extension, holding a valid record,
        // became a week; the leaderboard would have shown it.
        let dir = std::env::temp_dir().join(format!("radar-ledger-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let write = |name: &str, week: Week| {
            std::fs::write(
                dir.join(name),
                Record::close(week, Ranking::default(), &Rules::published(["op"]))
                    .to_json()
                    .expect("json"),
            )
            .expect("write");
        };
        write("2957.json", Week(2957));
        write("2958", Week(2958));
        write("2959.json.bak", Week(2959));
        std::fs::write(dir.join("notes.json"), "{}").expect("write");
        let weeks: Vec<u64> = records_in(&dir).into_iter().map(|r| r.week.0).collect();
        assert_eq!(weeks, [2957]);
        assert!(
            records_in(&dir.join("nowhere")).is_empty(),
            "a missing directory is no records"
        );
    }

    fn ranking_with_winner() -> Ranking {
        Ranking {
            ranked: vec![Ranked {
                entry: Entry {
                    reply_id: "r1".to_owned(),
                    summoner: "alice".to_owned(),
                    mention_id: None,
                    handle: Some("alice_h".to_owned()),
                    mint: "M".to_owned(),
                    at: WEEK.opens_at() + 10,
                    metrics: Metrics {
                        likes: 4,
                        ..Metrics::default()
                    },
                },
                score: 4,
            }],
            excluded: Vec::new(),
        }
    }

    fn claimed() -> Record {
        let mut record = Record::close(WEEK, ranking_with_winner(), &Rules::published(["op"]));
        record.claim = Some(Claim {
            address: "ADDR".to_owned(),
            reply_id: "r2".to_owned(),
            at: WEEK.closes_at() + 60,
        });
        record
    }

    #[test]
    fn closing_a_week_names_the_winner_and_leaves_it_unclaimed_and_unpaid() {
        let record = Record::close(WEEK, ranking_with_winner(), &Rules::published(["op"]));
        assert_eq!(
            record.winner,
            Some(Winner {
                summoner: "alice".to_owned(),
                reply_id: "r1".to_owned(),
                score: 4,
                // Carried from the entry, so the leaderboard can print a name
                // rather than a numeric id.
                handle: Some("alice_h".to_owned()),
            })
        );
        assert_eq!(record.opened_at, WEEK.opens_at());
        assert_eq!(record.closed_at, WEEK.closes_at());
        assert!(record.claim.is_none());
        assert!(record.payout.is_none());

        // And a week where nothing counted has no winner, not a default one.
        let empty = Record::close(WEEK, Ranking::default(), &Rules::published(["op"]));
        assert!(empty.winner.is_none());
    }

    #[test]
    fn the_claim_window_is_seven_days_from_the_close_and_not_a_second_more() {
        let record = Record::close(WEEK, ranking_with_winner(), &Rules::published(["op"]));
        assert!(
            !record.accepts_claim_at(record.closed_at - 1),
            "the week is still open"
        );
        assert!(record.accepts_claim_at(record.closed_at));
        assert!(record.accepts_claim_at(record.claim_window_closes_at() - 1));
        assert!(
            !record.accepts_claim_at(record.claim_window_closes_at()),
            "rolled over"
        );
        assert_eq!(
            record.claim_window_closes_at() - record.closed_at,
            7 * SECONDS_PER_DAY
        );
    }

    #[test]
    fn the_three_refusals_each_fire_and_a_correct_payout_is_permitted() {
        // Each refusal re-applied as a bug fails exactly one assertion here:
        // drop the recipient comparison and `WrongRecipient` is not returned;
        // change `>` to `>=` on the amount and paying exactly what was
        // collected is refused; drop the paid check and a week pays twice.
        let record = claimed();
        assert_eq!(
            Payout::permitted(&record, "ADDR", 1_000, 1_000, NO_FLOOR),
            Ok(())
        );
        assert_eq!(
            Payout::permitted(&record, "ADDR", 999, 1_000, NO_FLOOR),
            Ok(())
        );
        assert_eq!(
            Payout::permitted(&record, "MALLORY", 1_000, 1_000, NO_FLOOR),
            Err(Refusal::WrongRecipient)
        );
        assert_eq!(
            Payout::permitted(&record, "ADDR", 1_001, 1_000, NO_FLOOR),
            Err(Refusal::AboveCollected { collected: 1_000 })
        );

        let mut paid = claimed();
        paid.payout = Some(Payout {
            recipient: "ADDR".to_owned(),
            lamports: 1_000,
            signature: "SIG".to_owned(),
            at: 1,
        });
        assert_eq!(
            Payout::permitted(&paid, "ADDR", 1_000, 1_000, NO_FLOOR),
            Err(Refusal::AlreadyPaid {
                signature: "SIG".to_owned()
            })
        );
    }

    #[test]
    fn nothing_is_paid_without_a_winner_or_without_a_claim() {
        let unclaimed = Record::close(WEEK, ranking_with_winner(), &Rules::published(["op"]));
        assert_eq!(
            Payout::permitted(&unclaimed, "ADDR", 1, 1, NO_FLOOR),
            Err(Refusal::Unclaimed)
        );

        let nobody = Record::close(WEEK, Ranking::default(), &Rules::published(["op"]));
        assert_eq!(
            Payout::permitted(&nobody, "ADDR", 1, 1, NO_FLOOR),
            Err(Refusal::NoWinner)
        );
    }

    #[test]
    fn the_paid_check_comes_first_so_a_paid_week_is_never_reported_as_anything_else() {
        // A second payout attempt with a wrong recipient must say "already
        // paid", not "wrong recipient": the first is the fact that stops an
        // operator retrying with a corrected address.
        let mut paid = claimed();
        paid.payout = Some(Payout {
            recipient: "ADDR".to_owned(),
            lamports: 1,
            signature: "SIG".to_owned(),
            at: 1,
        });
        assert!(matches!(
            Payout::permitted(&paid, "MALLORY", 1, 1, NO_FLOOR),
            Err(Refusal::AlreadyPaid { .. })
        ));
    }

    #[test]
    fn the_vault_reading_round_trips_and_half_of_it_is_refused() {
        let vault = Vault {
            address: "VAULT".to_owned(),
            lamports: 5_000,
            measured_at: 1_788_000_000,
        };
        let json = vault.to_json().expect("serialises");
        assert_eq!(Vault::from_json(&json).expect("round-trips"), vault);
        assert!(Vault::from_json(&json[..json.len() / 2]).is_err());
    }

    #[test]
    fn the_record_round_trips_through_the_json_the_site_reads() {
        let mut record = claimed();
        record.ranking.excluded.push((
            Entry {
                reply_id: "r9".to_owned(),
                summoner: "radar".to_owned(),
                mention_id: None,
                handle: None,
                mint: "M".to_owned(),
                at: WEEK.opens_at() + 5,
                metrics: Metrics::default(),
            },
            crate::score::Excluded::Operator,
        ));
        let json = record.to_json().expect("serialises");
        assert!(json.contains("\"week\": 2958"), "{json}");
        assert!(
            json.contains("Operator"),
            "the exclusion and its reason are published: {json}"
        );
        let back = Record::from_json(&json).expect("round-trips");
        assert_eq!(back, record);

        // Half a file is not evidence.
        assert!(Record::from_json(&json[..json.len() / 2]).is_err());
    }
    #[test]
    fn the_record_production_wrote_before_these_fields_existed_still_parses() {
        // The exact bytes of `~/radar/data/contest/2956.json`, read off the
        // production box on 2026-09-06. It was written before `claim_prompt`
        // and `handle` existed, and it is the only closed week there is.
        //
        // This is the S11 regression and the failure it guards against is
        // silent. `records_in` SKIPS a file it cannot parse -- it does not warn
        // and it does not fail -- so a *required* field added to this struct
        // would make old weeks disappear from the leaderboard and, worse, from
        // the cooldown that reads every earlier record to decide who is still
        // serving one. A past winner would quietly become eligible again.
        //
        // Re-applied and confirmed: adding a required `probe_required: u64` to
        // `Record` fails exactly this test with
        // `missing field `probe_required``, 28 passed and 1 failed, while
        // nothing else in the crate notices.
        //
        // Deleting the `#[serde(default)]` above does NOT fail it, and finding
        // that out beat assuming it: serde already maps a missing `Option`
        // field to `None`. The attribute is a note about intent. This test is
        // the enforcement.
        const AS_PRODUCTION_WROTE_IT: &str = r#"{
  "week": 2956,
  "opened_at": 1787529600,
  "closed_at": 1788134400,
  "ranking": {
    "ranked": [],
    "excluded": []
  },
  "winner": null,
  "claim": null,
  "payout": null
}"#;
        let record = Record::from_json(AS_PRODUCTION_WROTE_IT)
            .expect("a record written before the new fields existed still parses");
        assert_eq!(record.week, Week(2956));
        assert_eq!(record.opened_at, 1_787_529_600);
        assert_eq!(record.closed_at, 1_788_134_400);
        assert!(record.winner.is_none());
        // Absent, and absent is the value that lets no claim through.
        assert!(record.claim_prompt.is_none());
    }

    #[test]
    fn an_entry_written_before_handles_existed_still_parses() {
        // The same guarantee one level down. An entry is nested inside a
        // record, so a required field added here takes the whole week with it.
        let entry: crate::score::Entry = serde_json::from_str(
            r#"{"reply_id":"r1","summoner":"123","mint":"M","at":10,
                "metrics":{"reposts":0,"quotes":0,"likes":0,"replies":0}}"#,
        )
        .expect("an entry without a handle parses");
        assert_eq!(entry.handle, None);
    }
}
