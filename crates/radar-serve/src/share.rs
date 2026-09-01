// SPDX-License-Identifier: Apache-2.0
//! How much of the day's model budget any one customer may take.
//!
//! # The hole this closes
//!
//! [`chat`](crate::chat) reserves against **one global daily budget** and there
//! is no customer anywhere in the path. That is correct today, because the only
//! reader is the operator: `customer::Mode::Off` ships, so every request falls
//! back to the operator check and there is one of him.
//!
//! It stops being correct the moment `RADAR_PRIVY_APP_ID` is set. From that
//! deploy, every signed-up account shares one allowance first-come-first-served,
//! and the first one to open a loop spends the whole day for everybody. Nothing
//! in the system would report it as anything other than a spent budget.
//!
//! # Why this is in memory, which is a real weakening and is stated as one
//!
//! [ADR 0006](https://github.com/hey-vera/radar/blob/main/docs/adr/0006-radar-records-only-what-it-cannot-recover.md)
//! decides that Radar persists **exactly one** thing about a customer: the
//! signature meter, because the per-customer signature count decides whether
//! Privy's pricing stays acceptable and cannot be taken retroactively. A second
//! durable per-customer artefact is an amendment to that ADR, not a patch, and
//! this does not need to be durable to do its job.
//!
//! So a restart resets these counters, and it is worth being exact about what
//! that does and does not cost:
//!
//! - It **cannot** increase total spend. The global budget is durable through
//!   [`ledger`](crate::ledger) and still binds; that was fixed for exactly this
//!   reason and [`the_budget_survives_a_restart`] holds it.
//! - It **can** let one customer take a larger share across a restart.
//!
//! That is a fairness weakening rather than a spending one, which is materially
//! smaller than the failure [`LEARNINGS`] entries 1 and 9 record — where a
//! restart handed out a fresh *budget*. If it ever matters, the fix is an
//! amendment to ADR 0006 and a second column on the meter that already exists.
//!
//! [`the_budget_survives_a_restart`]: https://github.com/hey-vera/radar/blob/main/crates/radar-serve/tests/the_budget_survives_a_restart.rs
//! [`LEARNINGS`]: https://github.com/hey-vera/radar/blob/main/LEARNINGS.md

use std::collections::HashMap;
use std::sync::Mutex;

use radar_customer::{Subject, SubjectError};

/// The variable that configures the per-customer ceiling.
pub const VAR: &str = "RADAR_CHAT_PER_CUSTOMER_DAILY";

/// How many questions one customer may ask in a day.
///
/// No default, and rule 8's direction when absent: an instance with no
/// per-customer ceiling configured lets **no customer** chat, rather than
/// letting every customer share the operator's budget. Spending nothing is
/// always recoverable.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Allowance(u32);

impl Allowance {
    /// Refuses everything. What an unconfigured allowance resolves to.
    pub const CLOSED: Self = Self(0);

    /// An allowance of `per_day` questions.
    #[must_use]
    pub const fn per_day(per_day: u32) -> Self {
        Self(per_day)
    }

    /// The ceiling.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// Reads the allowance from the environment.
    ///
    /// Absent is [`Self::CLOSED`], not an error and not a permissive default.
    /// An instance that has never thought about this refuses rather than sharing
    /// one budget among strangers.
    ///
    /// # Errors
    ///
    /// Returns a message when the value is set but unreadable. A typo must not
    /// resolve to "closed" silently — that is indistinguishable from a
    /// deliberate zero, and an operator would be looking at the wrong thing.
    pub fn from_vars(get: &impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let Some(raw) = get(VAR) else {
            return Ok(Self::CLOSED);
        };
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Ok(Self::CLOSED);
        }
        trimmed
            .parse()
            .map(Self)
            .map_err(|_| format!("{VAR} is not a whole number: {trimmed}"))
    }
}

/// Why a question was not charged.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Refused {
    /// This customer has used their allowance for the day.
    Spent {
        /// The ceiling they reached.
        allowance: u32,
    },
    /// No per-customer ceiling is configured, so no customer may chat.
    ///
    /// Distinct from [`Self::Spent`] because they need different words and
    /// different actions: one is a customer waiting for tomorrow, the other is
    /// an operator who has not finished configuring the instance.
    Unconfigured,
    /// The subject could not be derived, so nothing can be counted against it.
    ///
    /// Charging an uncountable customer would be an unmetered call wearing a
    /// meter, so it refuses.
    NoSubject(SubjectError),
}

/// Per-customer question counts for the current day.
///
/// Keyed by a salted hash of the DID rather than the DID, matching
/// [`radar_customer::Subject`]'s reasoning: this map is in memory and short
/// lived, but a heap dump is a copy like any other and there is no reason for it
/// to carry a customer list.
#[derive(Debug)]
pub struct Shares {
    allowance: Allowance,
    inner: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    day: u64,
    counts: HashMap<Subject, u32>,
}

impl Shares {
    /// A meter with this allowance.
    #[must_use]
    pub fn new(allowance: Allowance) -> Self {
        Self {
            allowance,
            inner: Mutex::new(State::default()),
        }
    }

    /// The configured ceiling, for reporting.
    #[must_use]
    pub const fn allowance(&self) -> Allowance {
        self.allowance
    }

    /// How an operator should see this at start.
    ///
    /// Says which state it is in rather than only reporting a number. A line
    /// that reads the same whether the lane is open or shut is the failure
    /// LEARNINGS records repeatedly.
    #[must_use]
    pub fn describe(&self) -> String {
        if self.allowance == Allowance::CLOSED {
            format!("closed — no customer may spend the model budget (set {VAR})")
        } else {
            format!("{} questions per customer per day", self.allowance.get())
        }
    }

    /// Charges one question to a customer, or refuses.
    ///
    /// Counts **before** the call, for the reason
    /// [`radar_customer::Meter::charge`] gives: a process that dies mid-call
    /// cannot know whether the call happened, and a meter that counts afterwards
    /// undercounts exactly the calls that went wrong — which are the ones a
    /// runaway loop is made of.
    ///
    /// # Errors
    ///
    /// Returns [`Refused`] describing which of the three cases applies.
    pub fn charge(&self, did: &str, salt: &[u8], day: u64) -> Result<u32, Refused> {
        if self.allowance == Allowance::CLOSED {
            return Err(Refused::Unconfigured);
        }
        let subject = Subject::derive(did, salt).map_err(Refused::NoSubject)?;

        let Ok(mut state) = self.inner.lock() else {
            // A poisoned lock means a thread died holding it. Refusing is the
            // only answer that cannot overspend: the alternative is treating an
            // unknown count as zero, which is rule 9 pointing at money.
            return Err(Refused::Spent {
                allowance: self.allowance.get(),
            });
        };

        // A new day clears the map rather than ageing entries out of it. The
        // allowance is daily, so yesterday's consumption has no claim on today's.
        if state.day != day {
            state.day = day;
            state.counts.clear();
        }

        let used = state.counts.entry(subject).or_insert(0);
        if *used >= self.allowance.get() {
            return Err(Refused::Spent {
                allowance: self.allowance.get(),
            });
        }
        *used = used.saturating_add(1);
        Ok(*used)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SALT: &[u8] = &[7u8; 32];
    const DID: &str = "did:privy:abc";
    const OTHER: &str = "did:privy:xyz";

    #[test]
    fn an_absent_ceiling_refuses_every_customer_rather_than_sharing_one_budget() {
        // Rule 8, and the whole reason this module exists. The permissive
        // default here is not "no limit" -- it is "every signed-up account
        // shares the operator's budget", which is worse than it sounds because
        // the first one to open a loop spends the day for everybody.
        let none = |_: &str| None;
        assert_eq!(Allowance::from_vars(&none), Ok(Allowance::CLOSED));

        let shares = Shares::new(Allowance::CLOSED);
        assert_eq!(shares.charge(DID, SALT, 1), Err(Refused::Unconfigured));
    }

    #[test]
    fn a_typo_is_an_error_rather_than_a_silent_closure() {
        // "Closed" and "misconfigured" send an operator to different places, and
        // collapsing them means reading the code to tell which happened.
        let bad = |k: &str| (k == "RADAR_CHAT_PER_CUSTOMER_DAILY").then(|| "twenty".to_owned());
        assert!(Allowance::from_vars(&bad).is_err());

        let blank = |k: &str| (k == "RADAR_CHAT_PER_CUSTOMER_DAILY").then(|| "  ".to_owned());
        assert_eq!(Allowance::from_vars(&blank), Ok(Allowance::CLOSED));
    }

    #[test]
    fn one_customer_cannot_spend_another_customers_share() {
        // The leak in one assertion: with a global budget only, the first caller
        // to exhaust it takes everybody else's day with them.
        let shares = Shares::new(Allowance::per_day(2));
        assert_eq!(shares.charge(DID, SALT, 1), Ok(1));
        assert_eq!(shares.charge(DID, SALT, 1), Ok(2));
        assert_eq!(
            shares.charge(DID, SALT, 1),
            Err(Refused::Spent { allowance: 2 })
        );

        // The second customer still has their whole allowance.
        assert_eq!(shares.charge(OTHER, SALT, 1), Ok(1));
    }

    #[test]
    fn a_new_day_clears_the_count() {
        let shares = Shares::new(Allowance::per_day(1));
        assert_eq!(shares.charge(DID, SALT, 1), Ok(1));
        assert!(shares.charge(DID, SALT, 1).is_err());
        assert_eq!(shares.charge(DID, SALT, 2), Ok(1), "tomorrow is a new day");
    }

    #[test]
    fn going_back_a_day_also_clears_it_rather_than_carrying_a_count_forward() {
        // A clock that steps backwards -- an NTP correction, a container with a
        // wrong date -- must not leave a customer refused until the calendar
        // catches up. The counter is per-day, not monotonic.
        let shares = Shares::new(Allowance::per_day(1));
        assert_eq!(shares.charge(DID, SALT, 5), Ok(1));
        assert_eq!(shares.charge(DID, SALT, 4), Ok(1));
    }

    #[test]
    fn a_customer_who_cannot_be_hashed_is_refused_rather_than_waved_through() {
        // An uncountable customer charged anyway is an unmetered call wearing a
        // meter. `Subject::derive` refuses a short salt for its own reasons; the
        // only thing this must not do is proceed.
        let shares = Shares::new(Allowance::per_day(10));
        assert!(matches!(
            shares.charge(DID, b"short", 1),
            Err(Refused::NoSubject(_))
        ));
        assert!(matches!(
            shares.charge("", SALT, 1),
            Err(Refused::NoSubject(_))
        ));
    }

    #[test]
    fn the_count_is_keyed_by_the_hash_and_not_by_the_identifier() {
        // The map is in memory and short lived, but a heap dump is a copy like
        // any other and there is no reason for it to carry a customer list.
        let shares = Shares::new(Allowance::per_day(5));
        shares.charge(DID, SALT, 1).expect("charges");
        let state = shares.inner.lock().expect("not poisoned");
        for subject in state.counts.keys() {
            assert_ne!(format!("{subject:?}"), format!("{DID:?}"));
            assert!(!format!("{subject:?}").contains(DID));
        }
    }

    #[test]
    fn a_configured_ceiling_is_read_as_written() {
        let set = |k: &str| (k == "RADAR_CHAT_PER_CUSTOMER_DAILY").then(|| "25".to_owned());
        assert_eq!(Allowance::from_vars(&set), Ok(Allowance::per_day(25)));
        assert_eq!(Allowance::per_day(25).get(), 25);
    }
}
