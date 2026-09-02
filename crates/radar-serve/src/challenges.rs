// SPDX-License-Identifier: Apache-2.0
//! The nonces this instance has issued and not yet spent.
//!
//! [`radar_customer::siws`] is pure and says so: it checks that a signature
//! carries the nonce it was shown, and it cannot check that the nonce was ever
//! *issued* or that it is being used for the first time. Remembering is state,
//! and that crate has none.
//!
//! This is the remembering.
//!
//! # Single use is the whole point
//!
//! Without it a captured signature logs an attacker in for the life of the
//! challenge. [`Challenges::spend`] removes the nonce as it returns it, so the
//! second attempt with the same signature finds nothing.
//!
//! # In memory, and what a restart costs
//!
//! Nothing is persisted. A restart forgets every outstanding challenge, and the
//! consequence is that sign-ins in flight fail and have to be retried — which is
//! a nuisance rather than a hole, because it fails **closed**: an unknown nonce
//! is refused, never accepted.
//!
//! That direction is the reason this is acceptable where the chat meter's
//! in-memory state was not. There, forgetting granted a fresh allowance;
//! here, forgetting refuses a login.
//!
//! # Bounded, because an unbounded map is a way to exhaust the process
//!
//! Issuing is unauthenticated by necessity — it happens before anyone has
//! proved anything — so anyone can ask for challenges as fast as they like.
//! Expired entries are dropped on every operation, and a hard ceiling refuses
//! rather than growing past [`Challenges::CAPACITY`].

use std::collections::HashMap;
use std::sync::Mutex;

use radar_customer::siws::{Challenge, MAX_AGE_SECONDS};

/// The nonces outstanding on this instance.
#[derive(Debug)]
pub struct Challenges {
    issued: Mutex<HashMap<String, Challenge>>,
    domain: String,
}

/// Why a challenge could not be issued.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Busy {
    /// Too many challenges are outstanding.
    ///
    /// Refused rather than evicting somebody else's: evicting under pressure
    /// would let a flood of requests cancel the sign-ins of real customers,
    /// which turns a denial of service into a *targeted* one.
    TooManyOutstanding,
}

impl core::fmt::Display for Busy {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooManyOutstanding => {
                write!(f, "too many sign-ins are in flight; try again shortly")
            }
        }
    }
}

impl Challenges {
    /// The most challenges that may be outstanding at once.
    ///
    /// Generous against real use — each lives at most five minutes, so this is
    /// roughly thirty-three sign-ins a second sustained — and small enough that
    /// the map cannot grow into a memory problem on a 4GB host.
    pub const CAPACITY: usize = 10_000;

    /// A store for challenges naming `domain`.
    #[must_use]
    pub fn new(domain: impl Into<String>) -> Self {
        Self {
            issued: Mutex::new(HashMap::new()),
            domain: domain.into(),
        }
    }

    /// The site these challenges are bound to.
    #[must_use]
    pub fn domain(&self) -> &str {
        &self.domain
    }

    /// Records and returns a new challenge.
    ///
    /// `nonce` is supplied rather than generated here so the randomness has one
    /// source and this type stays testable without one.
    ///
    /// # Errors
    ///
    /// [`Busy::TooManyOutstanding`] when the ceiling is reached.
    pub fn issue(&self, nonce: String, now: u64) -> Result<Challenge, Busy> {
        let mut issued = self
            .issued
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Self::forget_expired(&mut issued, now);
        if issued.len() >= Self::CAPACITY {
            return Err(Busy::TooManyOutstanding);
        }
        let challenge = Challenge {
            domain: self.domain.clone(),
            nonce: nonce.clone(),
            issued_at: now,
        };
        issued.insert(nonce, challenge.clone());
        Ok(challenge)
    }

    /// Takes a challenge, removing it so it cannot be used twice.
    ///
    /// `None` when the nonce was never issued, has already been spent, or has
    /// expired — three situations that are one answer to a caller, deliberately.
    /// Telling a caller which would say whether a nonce had ever existed.
    pub fn spend(&self, nonce: &str, now: u64) -> Option<Challenge> {
        let mut issued = self
            .issued
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Self::forget_expired(&mut issued, now);
        issued.remove(nonce)
    }

    /// How many are outstanding. For tests and for the operator's screen.
    #[must_use]
    pub fn outstanding(&self, now: u64) -> usize {
        let mut issued = self
            .issued
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Self::forget_expired(&mut issued, now);
        issued.len()
    }

    /// Drops everything past its window.
    ///
    /// Called on every operation rather than by a timer: a timer is a second
    /// thing that can stop, and its stopping would be invisible until the map
    /// was large.
    fn forget_expired(issued: &mut HashMap<String, Challenge>, now: u64) {
        issued.retain(|_, c| now.saturating_sub(c.issued_at) <= MAX_AGE_SECONDS);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_788_000_000;

    fn store() -> Challenges {
        Challenges::new("radar.heyvera.org")
    }

    #[test]
    fn a_challenge_can_be_spent_once() {
        // The property the whole file exists for. Without it, a captured
        // signature logs an attacker in for the life of the challenge.
        let store = store();
        let issued = store.issue("nonce-1".to_owned(), NOW).expect("issued");
        assert_eq!(store.spend("nonce-1", NOW + 1).as_ref(), Some(&issued));
        assert_eq!(
            store.spend("nonce-1", NOW + 2),
            None,
            "a second use must find nothing"
        );
    }

    #[test]
    fn a_nonce_that_was_never_issued_is_not_accepted() {
        // Fail closed, and this is what a restart looks like from outside: an
        // unknown nonce is refused rather than trusted.
        assert_eq!(store().spend("never-issued", NOW), None);
    }

    #[test]
    fn a_challenge_expires_and_the_boundary_is_swept() {
        let store = store();
        store.issue("a".to_owned(), NOW).expect("issued");
        assert!(
            store.spend("a", NOW + MAX_AGE_SECONDS).is_some(),
            "the last valid second"
        );

        store.issue("b".to_owned(), NOW).expect("issued");
        assert!(
            store.spend("b", NOW + MAX_AGE_SECONDS + 1).is_none(),
            "one second past"
        );
    }

    #[test]
    fn expired_challenges_are_forgotten_rather_than_accumulating() {
        // Issuing is unauthenticated by necessity, so the map is reachable by
        // anyone. If nothing dropped, this is how the process runs out of memory.
        let store = store();
        for i in 0..100 {
            store.issue(format!("old-{i}"), NOW).expect("issued");
        }
        assert_eq!(store.outstanding(NOW), 100);
        assert_eq!(
            store.outstanding(NOW + MAX_AGE_SECONDS + 1),
            0,
            "everything past its window is gone"
        );
    }

    #[test]
    fn the_ceiling_refuses_rather_than_evicting_someone_elses_sign_in() {
        // Eviction under pressure would let a flood cancel real customers'
        // sign-ins, which turns a denial of service into a targeted one.
        let store = store();
        for i in 0..Challenges::CAPACITY {
            store.issue(format!("n-{i}"), NOW).expect("under the cap");
        }
        assert_eq!(
            store.issue("one-too-many".to_owned(), NOW),
            Err(Busy::TooManyOutstanding)
        );
        // And the existing ones are untouched.
        assert_eq!(store.outstanding(NOW), Challenges::CAPACITY);
        assert!(store.spend("n-0", NOW).is_some());
    }

    #[test]
    fn pressure_clears_once_the_outstanding_ones_expire() {
        // The ceiling must not be permanent: a full map that never drains would
        // mean one flood disables sign-in forever.
        let store = store();
        for i in 0..Challenges::CAPACITY {
            store.issue(format!("n-{i}"), NOW).expect("under the cap");
        }
        assert!(store.issue("blocked".to_owned(), NOW).is_err());
        assert!(
            store
                .issue("later".to_owned(), NOW + MAX_AGE_SECONDS + 1)
                .is_ok(),
            "the flood has aged out"
        );
    }

    #[test]
    fn an_issued_challenge_carries_this_instances_domain() {
        // The domain is the thing that stops a signature from another site
        // being replayed here, so it must come from the server rather than from
        // whoever asked.
        let issued = store().issue("n".to_owned(), NOW).expect("issued");
        assert_eq!(issued.domain, "radar.heyvera.org");
        assert_eq!(issued.issued_at, NOW);
        assert_eq!(issued.nonce, "n");
    }
}
