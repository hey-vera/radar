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
//! # Why this is durable, and why the first version was not
//!
//! The first version of this kept the counts in memory, arguing that
//! [ADR 0006](https://github.com/hey-vera/radar/blob/main/docs/adr/0006-radar-records-only-what-it-cannot-recover.md)
//! lets Radar persist **exactly one** thing about a customer and this was not
//! it. That read the ADR's arithmetic instead of its rule.
//!
//! The rule is in its title: *records only the customer state it **cannot
//! recover***. Its table has one decisive column, "recoverable?", and the one
//! row answering **no** is the one that gets written down. "Exactly one" was a
//! count of what passed that test in August, not a ceiling.
//!
//! A per-customer question count is recoverable from nobody. Privy does not know
//! it, Stripe does not know it, the chain does not know it. Radar spent the
//! money. It is the same row as the signature meter, and ADR 0006's amendment of
//! 2026-09-01 says so.
//!
//! Two things the in-memory version got wrong in practice:
//!
//! - **A restart handed back the allowance.** Deploys are routine, and under
//!   `Restart=always` a crash loop returns it per crash. That is
//!   [`LEARNINGS`] entries 1 and 9 in a new costume, which is exactly what
//!   `RADAR_STATE_DIR` was made mandatory to stop.
//! - **It will become a billing fact.** Once a subscription decides who may ask,
//!   what a customer consumed is something Radar has to be able to stand behind,
//!   and a figure that resets on deploy is not one.
//!
//! [`LEARNINGS`]: https://github.com/hey-vera/radar/blob/main/LEARNINGS.md

use std::collections::BTreeMap;
use std::sync::Mutex;

use radar_customer::{Subject, SubjectError};
use serde::{Deserialize, Serialize};

use crate::ledger;

/// The record the counts are written under.
pub const RECORD: &str = "chat-shares";

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

/// What survives a restart.
///
/// A day and a set of counts, keyed by a salted hash of the DID rather than the
/// DID itself — [`radar_customer::Subject`]'s reasoning applies here exactly:
/// the file outlives the request by years and will be copied, and a copy that
/// holds counts cannot be joined against anything, while one holding DIDs can.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct State {
    /// The accounting day these counts cover.
    pub day: u64,
    /// Questions asked per subject, that day.
    pub counts: BTreeMap<Subject, u32>,
}

/// Per-customer question counts, persisted.
#[derive(Debug)]
pub struct Shares {
    allowance: Allowance,
    /// Where the counts are written. `None` only in tests, which say so.
    store: Option<ledger::Store>,
    inner: Mutex<State>,
}

impl Shares {
    /// A meter that forgets on restart.
    ///
    /// **For tests.** Production uses [`Self::restored`], because a meter that
    /// forgets is one a deploy resets — and deploys are routine.
    #[must_use]
    pub fn new(allowance: Allowance) -> Self {
        Self {
            allowance,
            store: None,
            inner: Mutex::new(State::default()),
        }
    }

    /// A meter that reads back what it wrote.
    ///
    /// A record from an earlier day is **not** carried forward: the allowance is
    /// daily, so yesterday's consumption has no claim on today's, and restoring
    /// it would refuse everyone until midnight — a different bug wearing this
    /// one's clothes. The same rule [`radar_customer::Meter::restore`] follows.
    ///
    /// An unreadable record starts empty rather than refusing. That is the one
    /// place here the safe direction is *permissive*, and it is deliberate: the
    /// alternative is that a corrupt file locks every customer out of a product
    /// they are paying for, while the **global** budget still bounds what can be
    /// spent. Losing a day's counts costs fairness for a day; refusing on a
    /// missing file costs the product.
    #[must_use]
    pub fn restored(allowance: Allowance, store: ledger::Store, today: u64) -> Self {
        let state = store
            .read::<State>(RECORD)
            .filter(|saved| saved.day == today)
            .unwrap_or_default();
        Self {
            allowance,
            store: Some(store),
            inner: Mutex::new(State {
                day: today,
                counts: state.counts,
            }),
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
        let count = *used;

        // Written while the lock is held and **before** the caller is told yes,
        // for the reason the model ledger gives: a process that dies between the
        // increment and the write cannot know whether the question was asked, and
        // assuming it was not hands the allowance back.
        //
        // A failed write does not refuse. The count in memory is still correct
        // for this process, the global budget still bounds the spend, and
        // refusing a paying customer because a disk is full is the wrong failure
        // -- but it is logged, because a meter that silently stops being durable
        // is one that looks fine until a restart.
        if let Some(store) = self.store.as_ref()
            && let Err(why) = store.write(RECORD, &*state)
        {
            eprintln!(
                "radar-serve: the chat share meter could not be written ({why}); a restart will forget today's counts"
            );
        }
        Ok(count)
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
