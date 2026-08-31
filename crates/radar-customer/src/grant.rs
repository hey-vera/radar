// SPDX-License-Identifier: Apache-2.0
//! The bounded signer grant, derived from what the kernel authorised.
//!
//! This is the seam invariant 1 lives on for customer capital. Privy's policy
//! engine is what actually refuses a transaction, so what it is told is the whole
//! of the bound — and it must be told **the kernel's numbers**, not a strategy's,
//! not a model's, and not anything the customer asserted.

use radar_risk::{Action, Authorization};
use radar_types::{Address, MicroUsd, Slot};
use serde::{Deserialize, Serialize};

/// Why a grant could not be derived.
///
/// Every variant is a refusal to hand Privy a bound, and refusing to grant is
/// always recoverable. There is no variant meaning "granted with a default".
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum NotGranted {
    /// The authorisation still needs a human, so a machine signer may not act on
    /// it.
    ///
    /// `Autonomy::Approve` sets `needs_operator_signature`, and the kernel means
    /// it: the trade is *within policy*, and a person still has to say go.
    /// Deriving an unattended grant from it would convert "within policy" into
    /// "authorised", which are the two things the kernel keeps separate.
    NeedsOperatorSignature,
    /// The authorisation permits no notional.
    ///
    /// A zero ceiling is `Policy::CLOSED`'s shape, and a signer granted a zero
    /// bound is a signer that exists for no reason. Refusing here means a closed
    /// policy produces no grant at all rather than an inert one, so the absence
    /// is visible.
    NoNotional,
    /// The authorisation is already void at the slot the grant would start.
    ///
    /// Not merely useless. A grant whose window has passed, handed to a policy
    /// engine that stores it, is a bound nobody can reason about later — and
    /// `expires_after` is the only thing making the grant temporary at all.
    AlreadyExpired {
        /// The slot the authorisation is void after.
        expires_after: Slot,
        /// The slot the grant would have started at.
        at: Slot,
    },
}

/// What Privy's policy engine is told.
///
/// Deliberately a distinct type from [`Authorization`](radar_risk::Authorization) rather than a rename. The
/// authorisation is Radar's internal object and carries a nonce and an action
/// enum; this is the subset that crosses a network boundary to a third party,
/// and keeping them separate means widening one does not silently widen the
/// other.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Bounds {
    /// The only mint this grant may touch.
    pub mint: Address,
    /// The most that may be committed, in micro-USD.
    ///
    /// Carried in Radar's unit and converted at the edge, because the conversion
    /// needs a SOL price and this crate has no clock to fetch one with. A
    /// conversion done here would be a conversion done at an unknown time.
    pub max_notional: MicroUsd,
    /// The slot after which the grant is void.
    pub expires_after: Slot,
    /// Whether this permits acquiring or disposing.
    pub action: Action,
}

/// A derived grant, and the authorisation it came from.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Grant {
    /// The bounds handed to the policy engine.
    pub bounds: Bounds,
    /// The kernel nonce this was derived from.
    ///
    /// Kept so a grant found in a log can be traced back to the decision that
    /// produced it. The nonce is a content hash of the proposal and the state it
    /// was judged against, so this is a link to a *replayable* judgement rather
    /// than to a row that might have changed.
    pub nonce: String,
}

impl Grant {
    /// Derives a grant from an authorisation, at a slot.
    ///
    /// # Errors
    ///
    /// Returns [`NotGranted`] rather than a weakened grant in every case. There
    /// is deliberately no path that returns a grant with a substituted bound —
    /// a caller that wanted a wider one must go back to the kernel for it.
    pub fn derive(authorization: &Authorization, at: Slot) -> Result<Self, NotGranted> {
        if authorization.needs_operator_signature {
            return Err(NotGranted::NeedsOperatorSignature);
        }
        if authorization.max_notional == MicroUsd::ZERO {
            return Err(NotGranted::NoNotional);
        }
        if authorization.expires_after <= at {
            return Err(NotGranted::AlreadyExpired {
                expires_after: authorization.expires_after,
                at,
            });
        }
        Ok(Self {
            bounds: Bounds {
                mint: authorization.mint,
                max_notional: authorization.max_notional,
                expires_after: authorization.expires_after,
                action: authorization.action,
            },
            nonce: authorization.nonce.clone(),
        })
    }
}
