// SPDX-License-Identifier: Apache-2.0
//! The risk kernel: a pure function from a proposal to a verdict.

use std::collections::BTreeMap;

use radar_types::{Address, MicroUsd, Slot, SlotDelta};
use serde::{Deserialize, Serialize};

use crate::policy::{Autonomy, Policy};

/// What a strategy or model wants to do.
///
/// Inert data with no authority whatsoever. A fully compromised reasoning layer
/// can emit any `Proposal` it likes and reach nothing: the only thing that turns
/// one into an [`Authorization`] is [`evaluate`], and the only thing that turns
/// an authorisation into a signature is a separate process that re-derives the
/// transaction and checks it against these bounds.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Proposal {
    /// Which token.
    pub mint: Address,
    /// Who created it. Carried on the proposal so exposure can be aggregated by
    /// creator without the kernel needing to look anything up — a lookup would
    /// be ambient state, and the kernel has none.
    pub creator: Address,
    /// What to do.
    pub action: Action,
    /// How much to commit.
    pub notional: MicroUsd,
    /// Expected total cost of getting in and out again: fees, tips, slippage.
    pub estimated_round_trip_cost: MicroUsd,
    /// The oldest input this proposal rests on.
    ///
    /// Not the newest. A decision is only as current as its stalest ingredient,
    /// and taking the newest would let one fresh number launder a set of old ones.
    pub oldest_input_slot: Slot,
    /// The largest notional an exit simulation says could actually be sold, or
    /// `None` if no exit was simulated.
    ///
    /// `None` is refused rather than treated as unlimited. Radar's whole premise
    /// is that most losses are inability to exit, so an unsimulated exit is the
    /// one thing it must not wave through.
    pub simulated_exit_capacity: Option<MicroUsd>,
}

/// What a proposal asks for.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Open or add to a position.
    Buy,
    /// Reduce a position.
    Reduce,
    /// Close a position entirely.
    Exit,
}

impl Action {
    /// Whether this action increases exposure.
    ///
    /// Only these are subject to sizing limits. Refusing an exit because a
    /// position is too large would trap capital in exactly the situation the
    /// limits exist to prevent.
    #[must_use]
    pub const fn increases_exposure(self) -> bool {
        matches!(self, Self::Buy)
    }
}

/// Everything the kernel knows about the world.
///
/// Passed in rather than read. There is no clock here, no network, no ambient
/// state — which is what lets a recorded decision be replayed and produce the
/// same verdict, and what makes a refusal reproducible from a recording rather
/// than a matter of trust.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct PortfolioState {
    /// The current slot. The kernel's only clock, and it is an argument.
    pub now: Slot,
    /// Total currently deployed.
    pub deployed: MicroUsd,
    /// Deployed per creator.
    pub per_creator: BTreeMap<Address, MicroUsd>,
    /// Realised loss so far today.
    pub realised_loss_today: MicroUsd,
    /// Consecutive failed transactions.
    pub consecutive_failures: u32,
    /// Whether an operator has stopped trading.
    ///
    /// Checked first and unconditionally. A kill switch that can be reasoned
    /// past is not a kill switch.
    pub halted: bool,
}

impl PortfolioState {
    /// A flat, unhalted portfolio at a slot.
    #[must_use]
    pub fn flat(now: Slot) -> Self {
        Self {
            now,
            deployed: MicroUsd::ZERO,
            per_creator: BTreeMap::new(),
            realised_loss_today: MicroUsd::ZERO,
            consecutive_failures: 0,
            halted: false,
        }
    }

    /// What is already committed to a creator's tokens.
    #[must_use]
    pub fn creator_exposure(&self, creator: &Address) -> MicroUsd {
        self.per_creator
            .get(creator)
            .copied()
            .unwrap_or(MicroUsd::ZERO)
    }
}

/// Why a proposal was refused.
///
/// Every applicable reason is returned, not the first one found. A caller fixing
/// one limit only to hit the next has learned nothing about whether the trade
/// was ever going to be allowed.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Refusal {
    /// An operator has halted trading.
    Halted,
    /// The policy grants no autonomy to authorise anything.
    NoAutonomy,
    /// The position exceeds the per-position ceiling.
    OverPositionLimit,
    /// The portfolio would exceed its total deployment ceiling.
    OverDeploymentLimit,
    /// Exposure to this creator's tokens would exceed its ceiling.
    OverCreatorLimit,
    /// Today's realised loss is at or beyond the daily cap.
    DailyLossReached,
    /// The round trip costs more than the policy allows as a share of size.
    RoundTripTooExpensive,
    /// No exit was simulated, so nothing is known about whether it can be sold.
    ExitNotSimulated,
    /// The simulated exit cannot absorb the proposed size.
    ExitCapacityTooSmall,
    /// An input is older than the policy tolerates.
    InputsTooStale,
    /// Too many consecutive transaction failures.
    TooManyFailures,
    /// The size exceeds what the canary level may authorise.
    OverCanaryLimit,
}

/// Permission to execute, with the exact bounds it was granted under.
///
/// The signer re-derives the transaction and checks it against these. It does
/// not trust a caller's description of what the transaction does, because a
/// caller that could describe its own transaction could describe it wrongly.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Authorization {
    /// Content hash of the proposal and the state it was judged against.
    ///
    /// Deterministic rather than random: the same decision replayed produces the
    /// same nonce, which is what makes a recorded authorisation checkable.
    pub nonce: String,
    /// The token.
    pub mint: Address,
    /// The action permitted.
    pub action: Action,
    /// The most that may be committed. Not "about this much" — the signer
    /// refuses anything above it.
    pub max_notional: MicroUsd,
    /// The slot after which this is void.
    pub expires_after: Slot,
    /// Whether an operator signature is still required.
    ///
    /// True under `Approve`. The kernel says the trade is *within policy*; a
    /// human still has to say go.
    pub needs_operator_signature: bool,
}

/// The kernel's answer.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum Verdict {
    /// Permitted, under these bounds.
    Authorised(Box<Authorization>),
    /// Refused, for all of these reasons.
    Refused {
        /// Every applicable reason, in a fixed order.
        reasons: Vec<Refusal>,
    },
}

impl Verdict {
    /// The authorisation, if there is one.
    #[must_use]
    pub fn authorisation(&self) -> Option<&Authorization> {
        match self {
            Self::Authorised(a) => Some(a),
            Self::Refused { .. } => None,
        }
    }

    /// Whether the proposal was refused.
    #[must_use]
    pub const fn is_refused(&self) -> bool {
        matches!(self, Self::Refused { .. })
    }
}

/// How long an authorisation stays valid.
///
/// Short by design. An authorisation is a statement about a market that existed
/// when it was issued, and a stale one executed later is a decision nobody made.
const AUTHORIZATION_LIFETIME: SlotDelta = SlotDelta(150);

/// Judges a proposal against a policy and the portfolio.
///
/// Pure: no clock, no network, no ambient state, and no dependence on the order
/// of anything. The same inputs always produce the same verdict, which is what
/// makes every past decision replayable and every refusal reproducible.
///
/// This is the only thing that can authorise capital. A model's confidence is
/// not an input.
#[must_use]
pub fn evaluate(proposal: &Proposal, state: &PortfolioState, policy: &Policy) -> Verdict {
    let mut reasons = Vec::new();

    // Checked first and unconditionally: a kill switch that can be reasoned past
    // is not a kill switch.
    if state.halted {
        reasons.push(Refusal::Halted);
    }
    if !policy.autonomy.can_self_authorise() && policy.autonomy != Autonomy::Approve {
        reasons.push(Refusal::NoAutonomy);
    }
    if state.consecutive_failures >= policy.max_consecutive_failures
        && policy.max_consecutive_failures > 0
    {
        reasons.push(Refusal::TooManyFailures);
    }
    if state.realised_loss_today >= policy.max_daily_loss {
        reasons.push(Refusal::DailyLossReached);
    }
    // Staleness applies to every action. Exiting on an hour-old view of
    // liquidity is its own hazard, not a safe default.
    if state.now.saturating_since(proposal.oldest_input_slot) > policy.max_input_staleness {
        reasons.push(Refusal::InputsTooStale);
    }

    if proposal.action.increases_exposure() {
        if proposal.notional > policy.max_position {
            reasons.push(Refusal::OverPositionLimit);
        }
        if state.deployed.saturating_add(proposal.notional) > policy.max_deployed {
            reasons.push(Refusal::OverDeploymentLimit);
        }
        let creator_after = state
            .creator_exposure(&proposal.creator)
            .saturating_add(proposal.notional);
        if creator_after > policy.max_per_creator {
            reasons.push(Refusal::OverCreatorLimit);
        }
        if policy.autonomy == Autonomy::Canary && proposal.notional > policy.max_canary {
            reasons.push(Refusal::OverCanaryLimit);
        }
        if over_cost_ceiling(proposal, policy) {
            reasons.push(Refusal::RoundTripTooExpensive);
        }
        match proposal.simulated_exit_capacity {
            // Unsimulated is refused rather than assumed fine. Radar exists on
            // the premise that most losses are inability to exit.
            None => reasons.push(Refusal::ExitNotSimulated),
            Some(capacity) if capacity < proposal.notional => {
                reasons.push(Refusal::ExitCapacityTooSmall);
            }
            Some(_) => {}
        }
    }

    if !reasons.is_empty() {
        // Sorted so the verdict does not depend on the order the checks happen
        // to run in. Purity has to hold for refusals too, or a replay can
        // disagree with the original about *why*.
        reasons.sort_unstable();
        reasons.dedup();
        return Verdict::Refused { reasons };
    }

    Verdict::Authorised(Box::new(Authorization {
        nonce: nonce_for(proposal, state),
        mint: proposal.mint,
        action: proposal.action,
        max_notional: proposal.notional,
        expires_after: state.now + AUTHORIZATION_LIFETIME,
        needs_operator_signature: policy.autonomy == Autonomy::Approve,
    }))
}

/// Whether the round trip costs more than the policy allows.
fn over_cost_ceiling(proposal: &Proposal, policy: &Policy) -> bool {
    if proposal.notional.get() == 0 {
        // A zero-size buy has an infinite cost ratio. Refusing it here rather
        // than dividing by zero.
        return true;
    }
    let allowed = MicroUsd(
        proposal
            .notional
            .get()
            .saturating_mul(u64::from(policy.max_round_trip_cost_percent))
            / 100,
    );
    proposal.estimated_round_trip_cost > allowed
}

/// A deterministic nonce over everything the verdict depended on.
///
/// Content-addressed rather than random, so an authorisation recorded today can
/// be recomputed tomorrow and shown to be the one that was issued.
fn nonce_for(proposal: &Proposal, state: &PortfolioState) -> String {
    let mut h = blake3::Hasher::new();
    h.update(proposal.mint.as_bytes());
    h.update(proposal.creator.as_bytes());
    h.update(&proposal.notional.get().to_le_bytes());
    h.update(&state.now.get().to_le_bytes());
    h.update(&state.deployed.get().to_le_bytes());
    h.update(match proposal.action {
        Action::Buy => b"buy",
        Action::Reduce => b"red",
        Action::Exit => b"exi",
    });
    h.finalize().to_hex()[..32].to_string()
}
