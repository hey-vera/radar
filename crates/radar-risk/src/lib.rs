// SPDX-License-Identifier: Apache-2.0
//! The risk kernel: the only thing that can authorise capital.
//!
//! Radar's central safety invariant, borrowed from GitLocus and applied to
//! money: **model judgement must never authorise capital.**
//!
//! ```text
//! strategy or model  --emits-->  Proposal       inert data, zero authority
//! risk kernel        --emits-->  Authorization  pure fn; nonce, expiry, bounds
//! signer process     --emits-->  Signature      re-derives the tx, trusts nothing
//! ```
//!
//! [`evaluate`] is a pure function. No clock, no network, no ambient state, and
//! no dependence on the order of anything — the current slot is an argument and
//! so is the portfolio. Three things follow, and all three are why it is written
//! this way:
//!
//! - Every past decision can be **replayed** and must produce the same verdict.
//! - Every refusal is **reproducible from a recording**, so "why was this
//!   blocked" is answerable months later rather than a matter of trust.
//! - Any decision can be re-judged under a **different policy** — a different
//!   autonomy level, tighter limits — without ever having run it that way.
//!
//! Deny-by-default throughout. [`Policy::CLOSED`] refuses everything and is what
//! an unloaded or failed policy resolves to, because spending nothing is always
//! recoverable.
//!
//! ```
//! use radar_risk::{Action, Autonomy, MicroUsd, Policy, PortfolioState, Proposal, Slot, evaluate};
//! use radar_risk::{Address, SlotDelta};
//!
//! let policy = Policy {
//!     autonomy: Autonomy::Capped,
//!     max_position: MicroUsd::from_dollars(50.0),
//!     max_deployed: MicroUsd::from_dollars(200.0),
//!     max_per_creator: MicroUsd::from_dollars(50.0),
//!     max_daily_loss: MicroUsd::from_dollars(25.0),
//!     max_round_trip_cost_bps: 900,
//!     max_canary: MicroUsd::from_dollars(1.0),
//!     max_input_staleness: SlotDelta(150),
//!     max_consecutive_failures: 3,
//! };
//!
//! let proposal = Proposal {
//!     mint: Address::new([1; 32]),
//!     creator: Address::new([2; 32]),
//!     action: Action::Buy,
//!     notional: MicroUsd::from_dollars(20.0),
//!     estimated_round_trip_cost: MicroUsd::from_dollars(0.50),
//!     oldest_input_slot: Slot(1_000),
//!     // An unsimulated exit is refused, not assumed fine.
//!     simulated_exit_capacity: Some(MicroUsd::from_dollars(100.0)),
//! };
//!
//! let verdict = evaluate(&proposal, &PortfolioState::flat(Slot(1_050)), &policy);
//! assert!(verdict.authorisation().is_some());
//!
//! // The same proposal under the default policy is refused, because the default
//! // policy refuses everything.
//! assert!(evaluate(&proposal, &PortfolioState::flat(Slot(1_050)), &Policy::default()).is_refused());
//! ```

#![forbid(unsafe_code)]

mod kernel;
mod policy;

pub use kernel::{
    Action, Authorization, PortfolioState, Proposal, Refusal, Verdict, evaluate,
    inevitable_refusals, partition_refusals,
};
pub use policy::{Autonomy, Policy};

// Re-exported so a caller building a proposal does not need three crates.
pub use radar_types::{Address, MicroUsd, Slot, SlotDelta};
