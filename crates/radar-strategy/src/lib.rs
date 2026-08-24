// SPDX-License-Identifier: Apache-2.0
//! Strategies: pure functions from evidence to a [`Proposal`].
//!
//! A strategy has no authority. It emits a proposal, which is inert data, and
//! the risk kernel decides whether that proposal becomes an authorization. This
//! separation is the reason an AI can be given a strategy's seat without being
//! given a spending limit — see rule 1 in `AGENTS.md`.
//!
//! Two properties are load-bearing and both are tested:
//!
//! **Purity.** A strategy is `fn(&Candidate) -> Decision` with no clock, no
//! network and no ambient state. The candidate carries its own watermark, so
//! replaying a recorded decision at its original `as_of` must reproduce it
//! exactly. A strategy that reads a clock cannot be evaluated, only trusted.
//!
//! **Falsifiability.** Every refusal names its reasons, and every proposal names
//! the strategy and version that produced it. The research store can then answer
//! "what did this strategy actually reject, and would taking those have made
//! money" — which is the only way to find out whether a rule is an edge or a
//! superstition.

pub mod assemble;
pub mod avoidance;
pub mod creator_edge;

pub use assemble::{Universe, universe};
pub use avoidance::{PassReason, disqualify};
pub use creator_edge::CreatorEdge;

use radar_asof::AsOf;
use radar_risk::Proposal;
use radar_sim::ExitReport;
use radar_types::{Address, MicroUsd, Slot};

/// Everything a strategy is allowed to know.
///
/// Assembled by the caller from recorded data at the watermark. A strategy
/// cannot fetch anything it was not given, which is what makes look-ahead a
/// property of assembly rather than of every strategy independently.
#[derive(Clone, Debug)]
pub struct Candidate {
    /// The token.
    pub mint: Address,
    /// Who launched it.
    pub creator: Address,
    /// The slot the token was created at.
    pub launch_slot: Slot,
    /// The watermark. Nothing in this candidate may postdate it.
    pub as_of: AsOf,
    /// What the exit analysis found, or `None` if none was run.
    ///
    /// `None` is not "probably fine". A strategy that proposes without one is
    /// proposing a position it has no evidence it can close.
    pub exit: Option<ExitReport>,
    /// How this creator's previous launches turned out.
    pub creator_record: CreatorRecord,
    /// The SOL price used to convert exit capacity into notional, or `None` if
    /// unknown.
    pub sol_price_micro_usd: Option<MicroUsd>,
    /// When this token's own mutable facts were last observed.
    ///
    /// Its outcome measurement, or the launch slot if it has never been measured
    /// — "nothing has been measured" is itself a fact as of the launch.
    ///
    /// Fast-moving. A token's liquidity and activity change by the minute, so a
    /// reading an hour old describes a market that may no longer exist.
    pub token_observed_at: Slot,
    /// When this creator's record was last updated.
    ///
    /// Slow-moving, and deliberately budgeted separately. A creator's history is
    /// a count over months; it does not go stale in an hour, and holding it to a
    /// fast budget would refuse every candidate for a reason that is about the
    /// measurement cadence rather than about the creator.
    ///
    /// This distinction is the plan's mutability classes applied where they
    /// actually bite. A single budget across both made 88% of live candidates
    /// read as stale — see `docs/research/0006`.
    pub creator_observed_at: Slot,
}

impl Candidate {
    /// The oldest mutable input this candidate rests on.
    ///
    /// What goes on the proposal, because the risk kernel's staleness check is
    /// about the decision as a whole: a decision is only as current as its
    /// stalest ingredient, whatever the strategy's own per-class budgets said.
    #[must_use]
    pub fn oldest_input_slot(&self) -> Slot {
        self.token_observed_at.min(self.creator_observed_at)
    }
}

/// A creator's measured history, as of the watermark.
///
/// Counts rather than rates. A rate is a float, a float is a rounding decision,
/// and a rounding decision inside a purity test is a flake waiting to happen.
/// Thresholds are applied to the counts directly.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CreatorRecord {
    /// Launches recorded at or before the watermark.
    pub launches: u64,
    /// Launches whose outcome has been measured.
    pub measured: u64,
    /// Measured tokens that showed almost no life.
    pub stillborn: u64,
    /// Measured tokens that reached an AMM, however they got there.
    ///
    /// Kept for the population view, and deliberately **not** what the strategy
    /// gates on. See [`Self::graduated_organic`].
    pub graduated: u64,
    /// Measured tokens whose curve filled over time rather than in a block.
    ///
    /// This is the one the strategy uses, and the distinction is the whole
    /// point. A creator whose tokens graduate instantly is not a creator who
    /// makes tokens people want; they are a creator who bundles. Selecting on
    /// undifferentiated graduations rewards exactly that, which is the opposite
    /// of the intent — so the rate that gates a proposal counts only
    /// [`GraduationMode::Organic`](radar_store::GraduationMode::Organic).
    pub graduated_organic: u64,
}

impl CreatorRecord {
    /// Graduations of any kind, per ten thousand measured launches.
    ///
    /// `None` below any measurement at all — a rate over zero samples is not a
    /// small number, it is not a number.
    ///
    /// Reported, not acted on. Use [`Self::organic_graduation_bps`] for
    /// decisions.
    #[must_use]
    pub const fn graduation_bps(&self) -> Option<u64> {
        if self.measured == 0 {
            return None;
        }
        Some(self.graduated.saturating_mul(10_000) / self.measured)
    }

    /// Organic graduations per ten thousand measured launches.
    ///
    /// The rate a proposal is gated on.
    #[must_use]
    pub const fn organic_graduation_bps(&self) -> Option<u64> {
        if self.measured == 0 {
            return None;
        }
        Some(self.graduated_organic.saturating_mul(10_000) / self.measured)
    }

    /// Measured graduations that completed within a few slots of the launch.
    #[must_use]
    pub const fn graduated_instant(&self) -> u64 {
        self.graduated.saturating_sub(self.graduated_organic)
    }

    /// Stillbirths per ten thousand measured launches.
    #[must_use]
    pub const fn stillborn_bps(&self) -> Option<u64> {
        if self.measured == 0 {
            return None;
        }
        Some(self.stillborn.saturating_mul(10_000) / self.measured)
    }
}

/// What a strategy decided.
#[derive(Clone, Debug, PartialEq)]
pub enum Decision {
    /// No proposal, and why not.
    ///
    /// Reasons are sorted and deduplicated so that two runs over the same
    /// candidate compare equal regardless of the order the checks ran in.
    Pass(Vec<PassReason>),
    /// A proposal, carrying no authority.
    Propose(Box<Proposal>),
}

impl Decision {
    /// Builds a pass with its reasons put in a canonical order.
    #[must_use]
    pub fn pass(mut reasons: Vec<PassReason>) -> Self {
        reasons.sort_unstable();
        reasons.dedup();
        Self::Pass(reasons)
    }

    /// Whether this decision produced a proposal.
    #[must_use]
    pub const fn is_proposal(&self) -> bool {
        matches!(self, Self::Propose(_))
    }

    /// The reasons a candidate was passed over, empty for a proposal.
    #[must_use]
    pub fn reasons(&self) -> &[PassReason] {
        match self {
            Self::Pass(r) => r,
            Self::Propose(_) => &[],
        }
    }
}

/// A deterministic strategy.
///
/// # Implementing one
///
/// Take nothing but the candidate. If a rule needs a fact the candidate does not
/// carry, add it to [`Candidate`] so that assembly — and therefore the watermark
/// check — sees it, rather than reaching for it inside `consider`.
pub trait Strategy {
    /// A stable identifier, recorded on every proposal.
    fn name(&self) -> &'static str;

    /// The version of the *rules*, bumped whenever a threshold changes.
    ///
    /// Recorded alongside the name so a result set can be split by the rules
    /// that produced it. Without this, changing a threshold silently pools two
    /// different strategies' results into one indistinguishable pile.
    fn version(&self) -> &'static str;

    /// Considers a candidate.
    fn consider(&self, candidate: &Candidate) -> Decision;
}

/// Converts lamports to notional using a SOL price.
///
/// Integer throughout: the intermediate exceeds `u64` for any real price, so it
/// goes through `u128` and saturates rather than wrapping. A wrapped notional
/// would be a small number describing a large position.
#[must_use]
pub fn lamports_to_micro_usd(lamports: u64, sol_price: MicroUsd) -> MicroUsd {
    let product = u128::from(lamports) * u128::from(sol_price.get());
    MicroUsd(u64::try_from(product / 1_000_000_000).unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rate_over_no_samples_is_absent_rather_than_zero() {
        // Zero would say "this creator never graduates a token", which is a
        // claim. Absent says "nothing is known", which is the truth.
        let empty = CreatorRecord::default();
        assert_eq!(empty.graduation_bps(), None);
        assert_eq!(empty.stillborn_bps(), None);
    }

    #[test]
    fn rates_are_per_ten_thousand() {
        let r = CreatorRecord {
            launches: 10,
            measured: 8,
            stillborn: 6,
            graduated: 2,
            graduated_organic: 2,
        };
        assert_eq!(r.graduation_bps(), Some(2_500));
        assert_eq!(r.stillborn_bps(), Some(7_500));
    }

    #[test]
    fn pass_reasons_are_canonical_regardless_of_check_order() {
        // Two runs that found the same problems must compare equal, or a replay
        // that reorders its checks reads as a divergence.
        let a = Decision::pass(vec![
            PassReason::NoExitSimulated,
            PassReason::CreatorUnproven,
        ]);
        let b = Decision::pass(vec![
            PassReason::CreatorUnproven,
            PassReason::NoExitSimulated,
        ]);
        assert_eq!(a, b);
    }

    #[test]
    fn duplicate_reasons_collapse() {
        let d = Decision::pass(vec![
            PassReason::NoExitSimulated,
            PassReason::NoExitSimulated,
        ]);
        assert_eq!(d.reasons().len(), 1);
    }

    #[test]
    fn notional_conversion_stays_in_integers() {
        // One SOL at $200.
        let sol = MicroUsd::from_dollars(200.0);
        assert_eq!(
            lamports_to_micro_usd(1_000_000_000, sol),
            MicroUsd::from_dollars(200.0)
        );
        // A hundredth of a SOL.
        assert_eq!(
            lamports_to_micro_usd(10_000_000, sol),
            MicroUsd::from_dollars(2.0)
        );
    }

    #[test]
    fn an_absurd_notional_saturates_rather_than_wrapping() {
        // u64 lamports times u64 micro-USD overflows u64 long before it
        // overflows the plausible. A wrapped value would describe a huge
        // position as a small one, which is the direction that loses money.
        let huge = lamports_to_micro_usd(u64::MAX, MicroUsd::from_dollars(1_000_000.0));
        assert_eq!(huge, MicroUsd(u64::MAX));
    }
}
