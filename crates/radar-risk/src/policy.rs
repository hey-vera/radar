// SPDX-License-Identifier: Apache-2.0
//! The limits capital operates under.

use radar_types::{MicroUsd, SlotDelta};
use serde::{Deserialize, Serialize};

/// How much autonomy the system has.
///
/// A policy value read by a pure function, not a code path. The same pipeline
/// runs at every level, so any past decision can be replayed under a different
/// level to ask what it *would* have done — without ever having run it.
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum Autonomy {
    /// Record only. No proposal is ever authorised.
    #[default]
    Observe,
    /// Proposals are raised and surfaced, never authorised.
    Alert,
    /// An authorisation needs an operator signature before it is valid.
    Approve,
    /// Dust round trips authorise themselves; nothing larger does.
    Canary,
    /// Authorises within hard notional and daily-loss bounds.
    Capped,
    /// Capped, with no per-trade ceiling below the portfolio limits.
    Auto,
}

impl Autonomy {
    /// Whether this level can authorise anything at all without a human.
    #[must_use]
    pub const fn can_self_authorise(self) -> bool {
        matches!(self, Self::Canary | Self::Capped | Self::Auto)
    }
}

/// The limits a proposal is judged against.
///
/// Deny-by-default throughout: [`Policy::CLOSED`] refuses everything, and it is
/// what an unconfigured or failed-to-load policy resolves to. A risk engine that
/// defaults to permissive is not a risk engine.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Policy {
    /// How much autonomy is granted.
    pub autonomy: Autonomy,
    /// The most that may be committed to one position.
    pub max_position: MicroUsd,
    /// The most that may be deployed across all positions at once.
    pub max_deployed: MicroUsd,
    /// The most that may be committed to a single creator's tokens, across all
    /// of them.
    ///
    /// Separate from per-position because a creator launching forty-two tokens
    /// in half an hour can otherwise be held forty-two times over while every
    /// individual position looks small.
    pub max_per_creator: MicroUsd,
    /// Realised loss in a day beyond which nothing is authorised.
    pub max_daily_loss: MicroUsd,
    /// The most a round trip may cost in fees, tips and slippage, per ten
    /// thousand of the position.
    ///
    /// Basis points rather than percent, because the measured cost is **850
    /// bps** and a percent field cannot express it. The ceiling could be 8%
    /// (refusing every trade there is) or 9%, and nothing between — a
    /// bps-granularity cost judged against a percent-granularity limit, where
    /// the rounding decides whether the system trades at all.
    pub max_round_trip_cost_bps: u32,
    /// The largest dust round trip the `Canary` level may authorise.
    pub max_canary: MicroUsd,
    /// How stale an input may be before a proposal is refused.
    ///
    /// A decision made on data older than this is a decision about a market that
    /// may no longer exist.
    pub max_input_staleness: SlotDelta,
    /// Consecutive failed transactions after which nothing is authorised until
    /// an operator intervenes.
    pub max_consecutive_failures: u32,
}

impl Policy {
    /// Refuses everything.
    ///
    /// The correct value for a policy that failed to load, and the correct
    /// starting point for one being written. Spending nothing is always
    /// recoverable.
    pub const CLOSED: Self = Self {
        autonomy: Autonomy::Observe,
        max_position: MicroUsd::ZERO,
        max_deployed: MicroUsd::ZERO,
        max_per_creator: MicroUsd::ZERO,
        max_daily_loss: MicroUsd::ZERO,
        max_round_trip_cost_bps: 0,
        max_canary: MicroUsd::ZERO,
        max_input_staleness: SlotDelta(0),
        max_consecutive_failures: 0,
    };

    /// The policy this build decides with.
    ///
    /// # Why this exists beside [`CLOSED`](Self::CLOSED)
    ///
    /// It **is** `CLOSED` today, and the indirection is the point rather than
    /// the value.
    ///
    /// Two places consumed `Policy::CLOSED` directly and independently:
    /// `radar consider`, which decides, and `radar-serve`'s funnel, which tells
    /// a reader whether anything can be authorised. Opening the policy means
    /// changing the first — and the second would have gone on reporting
    /// `closed`, telling a customer nothing can trade while Radar traded. The
    /// interface's strongest safety claim was a compile-time constant that
    /// could not move with the thing it described.
    ///
    /// One constant cannot diverge from itself. Changing what ships is still a
    /// deliberate decision about money; it is now a decision that cannot be
    /// taken in one place and missed in the other.
    ///
    /// **This bounds the deciding policy and nothing else.**
    /// [ADR 0008](https://github.com/hey-vera/radar/blob/main/docs/adr/0008-the-signer-holds-its-own-policy.md)
    /// put a second policy in the signer, loaded from `RADAR_SIGNER_POLICY`,
    /// which no other process can read. The signer clamps against its own copy
    /// unconditionally, so it can refuse what this one permits — never the
    /// reverse.
    pub const SHIPPED: Self = Self::CLOSED;

    /// Whether this policy could ever authorise anything.
    #[must_use]
    pub const fn is_closed(&self) -> bool {
        !self.autonomy.can_self_authorise() || self.max_position.get() == 0
    }
}

impl Default for Policy {
    fn default() -> Self {
        Self::CLOSED
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_policy_refuses_everything() {
        // An unconfigured risk engine that permits trading is not a risk engine.
        assert_eq!(Policy::default(), Policy::CLOSED);
        assert!(Policy::CLOSED.is_closed());
    }

    #[test]
    fn what_ships_is_closed_and_is_one_constant() {
        // `SHIPPED` is what `radar consider` decides with and what the funnel
        // reports. It is `CLOSED` today; the assertion that matters is that
        // there is *one* of it, because the bug this replaced was two call
        // sites reading `CLOSED` independently — so opening the policy in the
        // decider would have left the interface reporting `closed` while Radar
        // traded.
        assert!(Policy::SHIPPED.is_closed());
        assert_eq!(Policy::SHIPPED, Policy::CLOSED);
    }

    #[test]
    fn only_the_top_three_levels_authorise_without_a_human() {
        assert!(!Autonomy::Observe.can_self_authorise());
        assert!(!Autonomy::Alert.can_self_authorise());
        assert!(!Autonomy::Approve.can_self_authorise());
        assert!(Autonomy::Canary.can_self_authorise());
        assert!(Autonomy::Capped.can_self_authorise());
        assert!(Autonomy::Auto.can_self_authorise());
    }

    #[test]
    fn autonomy_orders_from_least_to_most_permissive() {
        // Ordering is load-bearing: config that clamps a level relies on it.
        assert!(Autonomy::Observe < Autonomy::Alert);
        assert!(Autonomy::Approve < Autonomy::Canary);
        assert!(Autonomy::Capped < Autonomy::Auto);
    }

    #[test]
    fn a_policy_with_autonomy_but_no_size_is_still_closed() {
        // Both halves have to be set. Granting autonomy while leaving the size
        // limit at zero should not read as "trade freely".
        let p = Policy {
            autonomy: Autonomy::Auto,
            ..Policy::CLOSED
        };
        assert!(p.is_closed());
    }
}
