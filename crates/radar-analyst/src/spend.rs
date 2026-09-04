// SPDX-License-Identifier: Apache-2.0
//! What the account is allowed to spend, and what it has spent.
//!
//! # The bill is unbounded by strangers
//!
//! Every other spending path in this repository is driven by Radar deciding to
//! do something. This one is driven by **anybody with an X account**: a mention
//! costs a read, an answer costs a model call and a reply, and nothing about
//! that is rate-limited by the asker's willingness to pay. So the meter is not
//! a nicety here, it is the difference between a product and an open invoice.
//!
//! # Deny by default, and prices have none
//!
//! Two separate refusals, and both are rule 8:
//!
//! - **No budget configured** means [`Budget::CLOSED`], which refuses every
//!   call. The loop still starts and still says why, because a bot that exits
//!   looks like a broken deploy while a bot that answers nothing and reports
//!   `unfunded` is legible.
//! - **No prices configured** means the meter cannot be built at all. A default
//!   price is a spending decision made by whoever wrote the code, and this
//!   repository already has an ADR's worth of regret about deciding on an
//!   unverified number. `radar.env.example` takes the same line for the model
//!   provider.
//!
//! That second rule is what makes the two unsettled X billing figures a
//! *configuration* question rather than a blocker: the operator writes down
//! what they were charged, and nothing here has an opinion.

use radar_provider::{Budget, Commitment, Ledger, Meter, Refusal};
use radar_types::MicroUsd;

/// What each billable thing costs.
///
/// Micro-USD, and every field is required. There is no `Default` on purpose.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Prices {
    /// One page of mentions.
    pub mention_read: MicroUsd,
    /// Reading one post that is not the account's own — a parent lookup.
    pub post_read: MicroUsd,
    /// Publishing one reply.
    pub reply: MicroUsd,
    /// One model call for the voice pass.
    pub model_call: MicroUsd,
}

/// What is being paid for.
///
/// An enum rather than a bare amount so a caller cannot pass the wrong price
/// for the right action. The meter's job is to refuse; choosing which number to
/// refuse against is this type's.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cost {
    /// A mentions poll.
    MentionRead,
    /// A parent-post lookup.
    PostRead,
    /// A published reply.
    Reply,
    /// A voice-pass model call.
    ModelCall,
}

impl Prices {
    /// Reads prices from the environment, or `None` when any is missing.
    ///
    /// All or nothing. A partial price list would meter some calls and let
    /// others through free, which is worse than metering none: the total would
    /// look like a budget being respected.
    ///
    /// Values are in **micro-USD** — whole numbers, so a price can be written
    /// exactly. A reply at one cent is `10000`.
    #[must_use]
    pub fn from_vars(get: &impl Fn(&str) -> Option<String>) -> Option<Self> {
        let read = |name: &str| -> Option<MicroUsd> {
            get(name)
                .and_then(|v| v.trim().parse::<u64>().ok())
                .map(MicroUsd)
        };
        Some(Self {
            mention_read: read("RADAR_X_PRICE_MENTION_READ")?,
            post_read: read("RADAR_X_PRICE_POST_READ")?,
            reply: read("RADAR_X_PRICE_REPLY")?,
            model_call: read("RADAR_MODEL_PER_CALL_USD_MICRO")?,
        })
    }

    /// What one action costs.
    #[must_use]
    pub const fn of(&self, cost: Cost) -> MicroUsd {
        match cost {
            Cost::MentionRead => self.mention_read,
            Cost::PostRead => self.post_read,
            Cost::Reply => self.reply,
            Cost::ModelCall => self.model_call,
        }
    }
}

/// The meter, its prices, and where its ledger lives.
#[derive(Debug)]
pub struct Spend {
    meter: Meter,
    prices: Prices,
    path: String,
}

impl Spend {
    /// Opens a meter, restoring today's ledger if there is one.
    ///
    /// A ledger from an earlier day is not carried forward — [`Meter::restore`]
    /// decides that, and its reasoning is that yesterday's spend has no claim on
    /// today's allowance.
    #[must_use]
    pub fn open(budget: Budget, prices: Prices, path: impl Into<String>, day: u64) -> Self {
        let path = path.into();
        let meter = read_ledger(&path).map_or_else(
            || Meter::new(budget, day),
            |ledger| Meter::restore(budget, &ledger, day),
        );
        Self {
            meter,
            prices,
            path,
        }
    }

    /// Reserves the cost of one action **before** it happens.
    ///
    /// # Errors
    ///
    /// [`Refusal`] when the call would breach the per-call ceiling or the day's
    /// cap. A refusal is not an error to work around: it is the answer.
    pub fn authorize(&mut self, cost: Cost, day: u64) -> Result<Commitment, Refusal> {
        self.meter.authorize(self.prices.of(cost), day)
    }

    /// Records what an authorised action actually cost.
    pub fn settle(&mut self, commitment: Commitment, actual: MicroUsd) {
        self.meter.settle(commitment, actual);
    }

    /// Gives back a reservation for something that did not happen.
    ///
    /// The call that matters after a failed request: a reply that was refused by
    /// the platform costs nothing, and leaving it reserved would spend the day's
    /// budget on replies nobody received.
    pub fn release(&mut self, commitment: Commitment) {
        self.meter.release(commitment);
    }

    /// What has been committed or settled today.
    #[must_use]
    pub const fn spent_today(&self) -> MicroUsd {
        self.meter.spent_today()
    }

    /// How many calls have been refused for want of budget.
    #[must_use]
    pub const fn refusals(&self) -> u64 {
        self.meter.refusals()
    }

    /// Writes the ledger where a restart will find it.
    ///
    /// # Errors
    ///
    /// The I/O error. **A caller that cannot save its ledger should stop**: a
    /// process under `Restart=always` that forgets its spend can spend the day's
    /// budget as many times as it can crash, which is the failure
    /// [`Ledger`]'s own documentation opens with.
    pub fn save(&self) -> std::io::Result<()> {
        let ledger = self.meter.ledger();
        let json = serde_json::to_string(&ledger)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        // Written beside and renamed, for the reason the cursor is: a truncated
        // ledger that still parses is a spend figure that is quietly too low,
        // and too low is the direction that keeps spending.
        let temp = format!("{}.new", self.path);
        std::fs::write(&temp, json)?;
        std::fs::rename(&temp, &self.path)
    }
}

/// Reads a saved ledger, or `None`.
///
/// Absent, unreadable and unparseable are all `None`, and that is the safe
/// direction here only because [`Meter::restore`] treats a missing ledger as a
/// **fresh day at the full budget**. That is a real cost — a corrupt ledger buys
/// a second day's allowance — and it is the lesser of the two: refusing to start
/// turns one bad byte into an outage, and a bot that will not start is a bot
/// nobody notices is gone.
fn read_ledger(path: &str) -> Option<Ledger> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prices() -> Prices {
        Prices {
            mention_read: MicroUsd(1_000),
            post_read: MicroUsd(5_000),
            reply: MicroUsd(10_000),
            model_call: MicroUsd(2_000),
        }
    }

    fn budget(daily: u64) -> Budget {
        Budget {
            per_call_max: MicroUsd(50_000),
            daily_max: MicroUsd(daily),
        }
    }

    fn temp(name: &str) -> String {
        let dir = std::env::temp_dir().join(format!("radar-spend-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temp dir");
        let p = dir.join(name);
        let p = p.to_str().expect("a path").to_owned();
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn an_unfunded_account_answers_nothing() {
        // Rule 8, and the shape it takes here: the loop still starts, and every
        // single call is refused. A bot that exits looks like a broken deploy.
        let mut spend = Spend::open(Budget::CLOSED, prices(), temp("closed"), 1);
        for cost in [
            Cost::MentionRead,
            Cost::PostRead,
            Cost::Reply,
            Cost::ModelCall,
        ] {
            assert!(
                spend.authorize(cost, 1).is_err(),
                "{cost:?} must be refused with no budget"
            );
        }
        assert_eq!(spend.refusals(), 4);
    }

    #[test]
    fn each_action_is_charged_its_own_price() {
        // The enum exists so a caller cannot pay the reply price for a read.
        let mut spend = Spend::open(budget(1_000_000), prices(), temp("prices"), 1);
        let c = spend.authorize(Cost::MentionRead, 1).expect("authorised");
        assert_eq!(c.reserved(), MicroUsd(1_000));
        spend.settle(c, MicroUsd(1_000));

        let c = spend.authorize(Cost::Reply, 1).expect("authorised");
        assert_eq!(c.reserved(), MicroUsd(10_000));
        spend.settle(c, MicroUsd(10_000));

        assert_eq!(spend.spent_today(), MicroUsd(11_000));
    }

    #[test]
    fn settling_corrects_the_reservation_rather_than_confirming_it() {
        // The reservation is the list price; the actual is what happened. If
        // settling did nothing the reservation would stand forever, and a day
        // of calls that each cost less than quoted would exhaust a budget it
        // never actually spent.
        //
        // This is the mutant `replace Spend::settle with ()` survived on: every
        // other test settles for exactly what it reserved, so leaving the
        // commitment in flight produced the same total.
        let mut spend = Spend::open(budget(1_000_000), prices(), temp("settled"), 1);
        let c = spend.authorize(Cost::Reply, 1).expect("authorised");
        assert_eq!(
            spend.spent_today(),
            MicroUsd(10_000),
            "reserved at list price"
        );
        spend.settle(c, MicroUsd(3_000));
        assert_eq!(
            spend.spent_today(),
            MicroUsd(3_000),
            "settling must replace the reservation with what it really cost"
        );
    }

    #[test]
    fn the_day_runs_out_and_stays_out() {
        // Twelve replies against a budget for two.
        let mut spend = Spend::open(budget(20_000), prices(), temp("exhausted"), 1);
        for _ in 0..2 {
            let c = spend.authorize(Cost::Reply, 1).expect("within budget");
            spend.settle(c, MicroUsd(10_000));
        }
        for _ in 0..10 {
            assert!(spend.authorize(Cost::Reply, 1).is_err(), "past the cap");
        }
        assert_eq!(spend.spent_today(), MicroUsd(20_000));
        assert_eq!(spend.refusals(), 10);
    }

    #[test]
    fn a_reply_the_platform_refused_gives_its_reservation_back() {
        // Otherwise a day of failing posts spends the whole budget on replies
        // nobody received.
        let mut spend = Spend::open(budget(20_000), prices(), temp("released"), 1);
        let c = spend.authorize(Cost::Reply, 1).expect("authorised");
        assert_eq!(
            spend.spent_today(),
            MicroUsd(10_000),
            "reserved while in flight"
        );
        spend.release(c);
        assert_eq!(spend.spent_today(), MicroUsd::ZERO, "given back");
    }

    #[test]
    fn the_ledger_survives_a_restart() {
        // A budget that forgets is not a budget: a process under
        // `Restart=always` could otherwise spend the day's allowance as many
        // times as it can crash.
        let path = temp("restart");
        let mut spend = Spend::open(budget(30_000), prices(), path.clone(), 7);
        let c = spend.authorize(Cost::Reply, 7).expect("authorised");
        spend.settle(c, MicroUsd(10_000));
        spend.save().expect("saved");

        let restarted = Spend::open(budget(30_000), prices(), path, 7);
        assert_eq!(
            restarted.spent_today(),
            MicroUsd(10_000),
            "the restart must not hand back a fresh allowance"
        );
    }

    #[test]
    fn yesterdays_ledger_does_not_spend_todays_budget() {
        let path = temp("yesterday");
        let mut spend = Spend::open(budget(30_000), prices(), path.clone(), 7);
        let c = spend.authorize(Cost::Reply, 7).expect("authorised");
        spend.settle(c, MicroUsd(10_000));
        spend.save().expect("saved");

        let today = Spend::open(budget(30_000), prices(), path, 8);
        assert_eq!(today.spent_today(), MicroUsd::ZERO);
    }

    #[test]
    fn saving_a_ledger_leaves_no_temporary_file_behind() {
        let path = temp("atomic");
        let spend = Spend::open(budget(30_000), prices(), path.clone(), 1);
        spend.save().expect("saved");
        assert!(!std::path::Path::new(&format!("{path}.new")).exists());
    }

    #[test]
    fn a_corrupt_ledger_starts_a_fresh_day_rather_than_refusing_to_start() {
        // The lesser of two evils, and it costs a second day's allowance. A bot
        // that will not start is a bot nobody notices is gone.
        let path = temp("corrupt");
        std::fs::write(&path, "{not json").expect("written");
        let spend = Spend::open(budget(30_000), prices(), path, 1);
        assert_eq!(spend.spent_today(), MicroUsd::ZERO);
    }

    #[test]
    fn prices_are_all_or_nothing() {
        // A partial price list meters some calls and lets others through free,
        // and the total then looks like a budget being respected.
        let full = |k: &str| -> Option<String> {
            Some(
                match k {
                    "RADAR_X_PRICE_MENTION_READ" => "1000",
                    "RADAR_X_PRICE_POST_READ" => "5000",
                    "RADAR_X_PRICE_REPLY" => "10000",
                    "RADAR_MODEL_PER_CALL_USD_MICRO" => "2000",
                    _ => return None,
                }
                .to_owned(),
            )
        };
        assert_eq!(Prices::from_vars(&full), Some(prices()));

        for missing in [
            "RADAR_X_PRICE_MENTION_READ",
            "RADAR_X_PRICE_POST_READ",
            "RADAR_X_PRICE_REPLY",
            "RADAR_MODEL_PER_CALL_USD_MICRO",
        ] {
            let partial = |k: &str| if k == missing { None } else { full(k) };
            assert_eq!(
                Prices::from_vars(&partial),
                None,
                "a price list missing {missing} must not be usable"
            );
        }
    }

    #[test]
    fn a_price_that_is_not_a_number_is_absent_rather_than_zero() {
        // Zero would be the worst possible reading: every call free, the meter
        // never refusing, and the daily total honestly reporting nothing spent.
        let get = |k: &str| -> Option<String> {
            Some(
                match k {
                    "RADAR_X_PRICE_REPLY" => "one cent",
                    _ => "1000",
                }
                .to_owned(),
            )
        };
        assert_eq!(Prices::from_vars(&get), None);
    }
}
