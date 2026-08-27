// SPDX-License-Identifier: Apache-2.0
//! The spend meter: the thing standing between a bug and an unbounded bill.

use radar_types::MicroUsd;

/// Hard ceilings on what Radar may spend. Deny-by-default: a request that would
/// breach any one of these is refused, and no caller can override it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Budget {
    /// The most any single call may cost. Catches a mispriced catalogue entry or
    /// a fat-fingered config before it catches the daily cap.
    pub per_call_max: MicroUsd,
    /// The most that may be spent in one accounting day.
    pub daily_max: MicroUsd,
}

impl Budget {
    /// A budget that refuses everything. The correct default for a meter whose
    /// config has not loaded yet: spending nothing is always recoverable.
    pub const CLOSED: Self = Self {
        per_call_max: MicroUsd::ZERO,
        daily_max: MicroUsd::ZERO,
    };
}

/// Why a spend was refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug, thiserror::Error)]
pub enum Refusal {
    /// The call costs more than the per-call ceiling allows.
    #[error("call would cost {cost}, over the per-call ceiling of {ceiling}")]
    OverPerCallCeiling {
        /// What the call would cost.
        cost: MicroUsd,
        /// The configured ceiling.
        ceiling: MicroUsd,
    },
    /// The call would take the day over its cap.
    #[error("call would cost {cost}, and {spent} of {cap} is already committed today")]
    OverDailyCap {
        /// What the call would cost.
        cost: MicroUsd,
        /// Already spent or committed today.
        spent: MicroUsd,
        /// The configured daily cap.
        cap: MicroUsd,
    },
}

/// An authorisation to spend, issued by the meter and consumed exactly once.
///
/// Carries an id so a caller cannot settle the same commitment twice and quietly
/// halve its recorded spend. Deliberately not `Clone` or `Copy`.
#[derive(PartialEq, Eq, Debug)]
pub struct Commitment {
    id: u64,
    reserved: MicroUsd,
}

impl Commitment {
    /// What the meter set aside for this call.
    #[must_use]
    pub const fn reserved(&self) -> MicroUsd {
        self.reserved
    }
}

/// Tracks spend against [`Budget`] and refuses anything that would breach it.
///
/// Has no clock. The accounting day is passed in by the caller, which is what
/// makes the meter replayable: feeding a recorded sequence of calls back through
/// a fresh meter reproduces the same refusals in the same places, and a test can
/// cross midnight without waiting.
#[derive(Debug)]
pub struct Meter {
    budget: Budget,
    day: u64,
    settled_today: MicroUsd,
    committed: MicroUsd,
    next_id: u64,
    refusals: u64,
}

impl Meter {
    /// A meter starting on `day` with nothing spent.
    #[must_use]
    pub const fn new(budget: Budget, day: u64) -> Self {
        Self {
            budget,
            day,
            settled_today: MicroUsd::ZERO,
            committed: MicroUsd::ZERO,
            next_id: 1,
            refusals: 0,
        }
    }

    /// Total committed or settled today.
    #[must_use]
    pub const fn spent_today(&self) -> MicroUsd {
        self.settled_today.saturating_add(self.committed)
    }

    /// How many spends have been refused. A non-zero count that keeps climbing
    /// means the budget is wrong or something is looping; it belongs on the
    /// ops page.
    #[must_use]
    pub const fn refusals(&self) -> u64 {
        self.refusals
    }

    /// Reserves budget for a call.
    ///
    /// In-flight commitments count against the cap, so a burst of concurrent
    /// requests cannot collectively overshoot while each individually looks
    /// affordable.
    ///
    /// # Errors
    ///
    /// Returns [`Refusal`] if the call breaches the per-call ceiling or would
    /// take the day over its cap.
    pub fn authorize(&mut self, cost: MicroUsd, day: u64) -> Result<Commitment, Refusal> {
        self.roll_to(day);

        if cost > self.budget.per_call_max {
            self.refusals += 1;
            return Err(Refusal::OverPerCallCeiling {
                cost,
                ceiling: self.budget.per_call_max,
            });
        }
        let would_be = self.spent_today().saturating_add(cost);
        if would_be > self.budget.daily_max {
            self.refusals += 1;
            return Err(Refusal::OverDailyCap {
                cost,
                spent: self.spent_today(),
                cap: self.budget.daily_max,
            });
        }

        self.committed = self.committed.saturating_add(cost);
        let id = self.next_id;
        self.next_id += 1;
        Ok(Commitment { id, reserved: cost })
    }

    /// Records what a call actually cost and releases its reservation.
    ///
    /// `actual` may differ from the reservation — a vendor can price a call
    /// differently from the catalogue, and that drift is itself a signal worth
    /// watching. The reservation is released either way, so a vendor overcharging
    /// can push the day over its cap by at most the drift on calls already in
    /// flight; the next `authorize` sees the real number and refuses.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "consuming the commitment is the point: it is a linear token, and making                   it Copy as clippy suggests would allow the double-settle this prevents"
    )]
    pub fn settle(&mut self, commitment: Commitment, actual: MicroUsd) {
        debug_assert!(
            commitment.id < self.next_id,
            "commitment from another meter"
        );
        self.committed = MicroUsd(
            self.committed
                .get()
                .saturating_sub(commitment.reserved.get()),
        );
        self.settled_today = self.settled_today.saturating_add(actual);
    }

    /// Releases a reservation for a call that never happened — a transport
    /// failure before the request was billed, or a cache hit discovered late.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "consuming the commitment is the point; see settle"
    )]
    pub fn release(&mut self, commitment: Commitment) {
        debug_assert!(
            commitment.id < self.next_id,
            "commitment from another meter"
        );
        self.committed = MicroUsd(
            self.committed
                .get()
                .saturating_sub(commitment.reserved.get()),
        );
    }

    fn roll_to(&mut self, day: u64) {
        if day > self.day {
            self.day = day;
            self.settled_today = MicroUsd::ZERO;
            // In-flight commitments deliberately survive the roll. They are real
            // money already owed; zeroing them would let a burst spanning
            // midnight spend twice its cap.
        }
    }
}

/// What a meter has to remember across a restart.
///
/// **A budget that forgets is not a budget.** The daily ceiling is the only
/// thing standing between a bug in a paid call and an unbounded bill, and
/// `radar-serve` runs under `Restart=always` — a process that crashes and comes
/// back with a fresh allowance can spend the day's budget as many times as it
/// can crash.
///
/// Serialisable rather than self-persisting, because this crate is pure policy:
/// no clock, no network, no filesystem. The caller decides where it lives, the
/// same way it decides what "today" means.
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub struct Ledger {
    /// The accounting day this covers.
    pub day: u64,
    /// Micro-USD committed or settled on that day.
    pub spent: u64,
    /// How many calls were refused for want of budget.
    pub refusals: u64,
}

impl Meter {
    /// What this meter would need to be rebuilt.
    ///
    /// Records `spent_today`, which includes **in-flight commitments** as well
    /// as settled spend. That is deliberate and it is the conservative
    /// direction: a process that dies mid-call cannot know whether the call
    /// happened, and assuming it did risks under-spending while assuming it did
    /// not risks paying twice. Spending nothing is always recoverable.
    #[must_use]
    pub const fn ledger(&self) -> Ledger {
        Ledger {
            day: self.day,
            spent: self.spent_today().get(),
            refusals: self.refusals,
        }
    }

    /// Rebuilds a meter from a saved ledger.
    ///
    /// A ledger from an earlier day is *not* carried forward: the budget is
    /// daily, so yesterday's spend has no claim on today's allowance. Restoring
    /// it as though it were today's would refuse everything until midnight,
    /// which is a different bug and a more visible one.
    ///
    /// The restored spend is treated as settled. There is nothing in flight
    /// after a restart, and a commitment that outlived its process cannot be
    /// released by anyone.
    #[must_use]
    pub const fn restore(budget: Budget, ledger: &Ledger, day: u64) -> Self {
        if ledger.day != day {
            return Self::new(budget, day);
        }
        Self {
            budget,
            day,
            settled_today: MicroUsd(ledger.spent),
            committed: MicroUsd::ZERO,
            next_id: 1,
            refusals: ledger.refusals,
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn spend_survives_a_restart() {
        // The failure this exists to stop. radar-serve runs under
        // Restart=always: a process that crashes and comes back with a fresh
        // allowance can spend the day's budget as many times as it can crash,
        // and nothing about that looks wrong from inside any single process.
        let budget = Budget {
            per_call_max: MicroUsd::from_dollars(1.00),
            daily_max: MicroUsd::from_dollars(5.00),
        };
        let mut before = Meter::new(budget, 42);
        // Five calls of ninety cents, each inside the per-call ceiling: real
        // spend arrives in increments, not in one lump.
        for _ in 0..5 {
            let c = before
                .authorize(MicroUsd::from_dollars(0.90), 42)
                .expect("inside both ceilings");
            before.settle(c, MicroUsd::from_dollars(0.90));
        }

        let mut after = Meter::restore(budget, &before.ledger(), 42);
        assert_eq!(after.spent_today(), MicroUsd::from_dollars(4.50));

        // The remaining allowance is what is left, not the whole budget.
        assert!(
            after.authorize(MicroUsd::from_dollars(0.60), 42).is_err(),
            "a restart must not hand back a spent allowance"
        );
        assert!(after.authorize(MicroUsd::from_dollars(0.40), 42).is_ok());
    }

    #[test]
    fn an_in_flight_commitment_is_restored_as_spent() {
        // A process that dies mid-call cannot know whether the call happened.
        // Assuming it did risks under-spending by that amount; assuming it did
        // not risks paying for it twice. Spending nothing is recoverable.
        let budget = Budget {
            per_call_max: MicroUsd::from_dollars(1.00),
            daily_max: MicroUsd::from_dollars(5.00),
        };
        let mut before = Meter::new(budget, 7);
        let _never_settled = before
            .authorize(MicroUsd::from_dollars(0.75), 7)
            .expect("fits");

        let after = Meter::restore(budget, &before.ledger(), 7);
        assert_eq!(
            after.spent_today(),
            MicroUsd::from_dollars(0.75),
            "the commitment outlived the process and nobody can release it"
        );
    }

    #[test]
    fn yesterdays_ledger_does_not_consume_todays_budget() {
        // The budget is daily. Carrying yesterday's spend forward would refuse
        // everything until midnight, which is a different bug -- louder, but
        // still a bug.
        let budget = Budget {
            per_call_max: MicroUsd::from_dollars(1.00),
            daily_max: MicroUsd::from_dollars(5.00),
        };
        let mut yesterday = Meter::new(budget, 100);
        for _ in 0..5 {
            let c = yesterday
                .authorize(MicroUsd::from_dollars(1.00), 100)
                .expect("inside both ceilings");
            yesterday.settle(c, MicroUsd::from_dollars(1.00));
        }
        assert_eq!(yesterday.spent_today(), MicroUsd::from_dollars(5.00));

        let today = Meter::restore(budget, &yesterday.ledger(), 101);
        assert_eq!(
            today.spent_today(),
            MicroUsd::ZERO,
            "a new day, a new budget"
        );
    }

    #[test]
    fn a_ledger_round_trips_through_json() {
        // It has to survive whatever the caller writes it to, and the caller is
        // outside this crate by design -- no filesystem here.
        let budget = Budget {
            per_call_max: MicroUsd::from_dollars(1.00),
            daily_max: MicroUsd::from_dollars(5.00),
        };
        let mut meter = Meter::new(budget, 9);
        let c = meter
            .authorize(MicroUsd::from_dollars(0.25), 9)
            .expect("fits");
        meter.settle(c, MicroUsd::from_dollars(0.25));
        // Refuse one, so the count is not zero and a dropped field would show.
        let _ = meter.authorize(MicroUsd::from_dollars(99.0), 9);

        let ledger = meter.ledger();
        let json = serde_json::to_string(&ledger).expect("serialises");
        let back: Ledger = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(back, ledger);
        assert_eq!(back.day, 9);
        assert_eq!(back.spent, MicroUsd::from_dollars(0.25).get());
        assert!(back.refusals > 0, "refusals are part of the record");
    }

    use super::*;

    fn budget() -> Budget {
        Budget {
            per_call_max: MicroUsd::from_dollars(0.10),
            daily_max: MicroUsd::from_dollars(5.0),
        }
    }

    #[test]
    fn a_default_meter_refuses_everything() {
        // Config not loaded must mean "spend nothing", never "spend freely".
        let mut m = Meter::new(Budget::CLOSED, 0);
        assert!(m.authorize(MicroUsd(1), 0).is_err());
    }

    #[test]
    fn an_affordable_call_is_authorised_and_settled() {
        let mut m = Meter::new(budget(), 0);
        let c = m
            .authorize(MicroUsd::from_dollars(0.001), 0)
            .expect("affordable");
        assert_eq!(m.spent_today(), MicroUsd::from_dollars(0.001));
        m.settle(c, MicroUsd::from_dollars(0.001));
        assert_eq!(m.spent_today(), MicroUsd::from_dollars(0.001));
    }

    #[test]
    fn an_overpriced_single_call_is_refused_before_the_daily_cap_is_touched() {
        // A mispriced catalogue entry should be caught by the per-call ceiling,
        // not by burning through the day's budget one call at a time.
        let mut m = Meter::new(budget(), 0);
        let err = m
            .authorize(MicroUsd::from_dollars(1.0), 0)
            .expect_err("over ceiling");
        assert!(matches!(err, Refusal::OverPerCallCeiling { .. }));
        assert_eq!(m.spent_today(), MicroUsd::ZERO);
    }

    #[test]
    fn concurrent_calls_cannot_collectively_overshoot() {
        // Each of these is individually affordable. If in-flight commitments did
        // not count, all fifty would be authorised and the day would land at
        // double its cap.
        let mut m = Meter::new(budget(), 0);
        let mut held = Vec::new();
        for _ in 0..50 {
            if let Ok(c) = m.authorize(MicroUsd::from_dollars(0.10), 0) {
                held.push(c);
            }
        }
        assert_eq!(held.len(), 50, "5.00 cap / 0.10 per call");
        assert!(m.authorize(MicroUsd::from_dollars(0.10), 0).is_err());
        assert_eq!(m.spent_today(), MicroUsd::from_dollars(5.0));
    }

    #[test]
    fn releasing_a_failed_call_returns_its_budget() {
        let mut m = Meter::new(budget(), 0);
        let c = m
            .authorize(MicroUsd::from_dollars(0.05), 0)
            .expect("affordable");
        m.release(c);
        assert_eq!(m.spent_today(), MicroUsd::ZERO);
    }

    #[test]
    fn a_new_day_resets_settled_spend_but_not_money_already_owed() {
        let mut m = Meter::new(budget(), 0);
        let inflight = m
            .authorize(MicroUsd::from_dollars(0.10), 0)
            .expect("affordable");
        let done = m
            .authorize(MicroUsd::from_dollars(0.10), 0)
            .expect("affordable");
        m.settle(done, MicroUsd::from_dollars(0.10));

        // Rolling the day clears what was settled, but the in-flight call is real
        // money already owed and must keep counting.
        let c = m
            .authorize(MicroUsd::from_dollars(0.01), 1)
            .expect("new day");
        assert_eq!(m.spent_today(), MicroUsd::from_dollars(0.11));
        m.settle(c, MicroUsd::from_dollars(0.01));
        m.settle(inflight, MicroUsd::from_dollars(0.10));
    }

    #[test]
    fn a_vendor_overcharging_is_recorded_and_then_refused() {
        // The drift is absorbed on the call already in flight, and the next
        // authorisation sees the real total.
        let mut m = Meter::new(
            Budget {
                per_call_max: MicroUsd::from_dollars(1.0),
                daily_max: MicroUsd::from_dollars(1.0),
            },
            0,
        );
        let c = m
            .authorize(MicroUsd::from_dollars(0.01), 0)
            .expect("cheap as catalogued");
        m.settle(c, MicroUsd::from_dollars(0.99));
        assert_eq!(m.spent_today(), MicroUsd::from_dollars(0.99));
        assert!(m.authorize(MicroUsd::from_dollars(0.02), 0).is_err());
    }

    #[test]
    fn refusals_are_counted_for_the_ops_page() {
        let mut m = Meter::new(Budget::CLOSED, 0);
        for _ in 0..3 {
            let _ = m.authorize(MicroUsd(1), 0);
        }
        assert_eq!(m.refusals(), 3);
    }
}
