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

#[cfg(test)]
mod tests {
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
