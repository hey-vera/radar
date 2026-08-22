// SPDX-License-Identifier: Apache-2.0
//! How often a fact can change, and the one-way latch.

use core::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::slot::SlotDelta;

/// How often a given fact can change.
///
/// Declared per fact rather than per provider, because it is a property of the
/// data. A token's decimals do not become more mutable because a different
/// vendor served them.
///
/// This is the largest cost saving available to Radar and it requires nothing
/// from any vendor: structural facts about a token are fixed within seconds of
/// launch, and they are consulted on every single re-evaluation. Fetching them
/// once per mint instead of once per evaluation is a large multiple on the bill.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Mutability {
    /// Fixed forever once observed: create slot, creator, decimals, the set of
    /// transactions in the launch slot.
    Immutable,
    /// Can change exactly once, in one direction: mint authority revoked, freeze
    /// authority revoked, LP burned. Refetch until the latch closes, then never.
    Latched,
    /// Changes on the order of hours: a creator's prior launches, the tail of a
    /// holder distribution.
    Slow,
    /// Changes on the order of a minute: pool reserves, price, top-holder balances.
    Fast,
    /// Must be live at the moment of use: route quotes, exit simulations. Caching
    /// one of these is not an optimisation, it is a wrong answer.
    Realtime,
}

/// What the cache should do when asked for a value it already holds.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Revalidation {
    /// Serve from cache forever. Never spend money on this again.
    Never,
    /// Serve from cache once the latch has closed; refetch while it is open.
    UntilLatched,
    /// Refetch when the cached value is older than this many slots.
    After(SlotDelta),
    /// Never serve from cache.
    Always,
}

impl Mutability {
    /// The default revalidation policy for this class.
    ///
    /// The two slot budgets are starting points, not findings. `Fast` is about a
    /// minute and `Slow` about five and a half hours at 150 slots per minute.
    /// Both should be tuned against measured change rates once the recorder has
    /// run long enough to measure them — see the cost regression test.
    #[must_use]
    pub const fn revalidation(self) -> Revalidation {
        match self {
            Self::Immutable => Revalidation::Never,
            Self::Latched => Revalidation::UntilLatched,
            Self::Slow => Revalidation::After(SlotDelta(50_000)),
            Self::Fast => Revalidation::After(SlotDelta(150)),
            Self::Realtime => Revalidation::Always,
        }
    }

    /// Whether a value of this class may be written to the cache at all.
    #[must_use]
    pub const fn is_cacheable(self) -> bool {
        !matches!(self, Self::Realtime)
    }
}

/// A one-way boolean.
///
/// Mint authority, once revoked, cannot be restored; the same holds for freeze
/// authority and for burned LP. Modelling these as a plain `bool` loses that,
/// and a provider that reports a revoked authority as live again would silently
/// un-disqualify a token Radar had already rejected.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum Latch {
    /// The thing has not happened yet. Keep watching.
    #[default]
    Open,
    /// The thing has happened and cannot un-happen.
    Closed,
}

/// A latch was observed to reopen, which is impossible on chain.
///
/// This is never a state to reconcile — it means the provider is wrong, is
/// serving another token's data, or is being manipulated. It must surface.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("latch reopened: a one-way transition was observed running backwards")]
pub struct LatchReopened;

impl Latch {
    /// Whether the transition has happened.
    #[must_use]
    pub const fn is_closed(self) -> bool {
        matches!(self, Self::Closed)
    }

    /// Folds a fresh observation into this latch.
    ///
    /// # Errors
    ///
    /// Returns [`LatchReopened`] if the latch is closed and the observation says
    /// it is open. Callers must treat that as a data-integrity failure and raise
    /// it, never as a value to store.
    pub const fn observe(self, observed_closed: bool) -> Result<Self, LatchReopened> {
        match (self, observed_closed) {
            (Self::Closed, false) => Err(LatchReopened),
            (_, true) => Ok(Self::Closed),
            (Self::Open, false) => Ok(Self::Open),
        }
    }
}

impl fmt::Display for Mutability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Immutable => "immutable",
            Self::Latched => "latched",
            Self::Slow => "slow",
            Self::Fast => "fast",
            Self::Realtime => "realtime",
        };
        f.write_str(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn immutable_facts_are_never_refetched() {
        assert_eq!(Mutability::Immutable.revalidation(), Revalidation::Never);
    }

    #[test]
    fn realtime_facts_are_never_cached() {
        assert!(!Mutability::Realtime.is_cacheable());
        assert_eq!(Mutability::Realtime.revalidation(), Revalidation::Always);
        // Everything else may be cached; that is the point of the class.
        for m in [
            Mutability::Immutable,
            Mutability::Latched,
            Mutability::Slow,
            Mutability::Fast,
        ] {
            assert!(m.is_cacheable(), "{m} should be cacheable");
        }
    }

    #[test]
    fn a_latch_closes_and_stays_closed() {
        let l = Latch::Open.observe(false).expect("still open");
        assert_eq!(l, Latch::Open);
        let l = l.observe(true).expect("closes");
        assert_eq!(l, Latch::Closed);
        assert_eq!(l.observe(true), Ok(Latch::Closed));
    }

    #[test]
    fn a_latch_reopening_is_an_error_not_a_state() {
        // A provider claiming a revoked mint authority is live again would
        // otherwise silently un-disqualify a token already rejected.
        assert_eq!(Latch::Closed.observe(false), Err(LatchReopened));
    }
}
