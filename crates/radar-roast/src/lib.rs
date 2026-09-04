// SPDX-License-Identifier: Apache-2.0
//! The public analyst's reply.
//!
//! # The pipeline
//!
//! ```text
//!   mint ──▶ radar-onchain ──▶ Dossier ──┐
//!                                        ├──▶ FactSheet ──▶ model ──▶ CHECKS ──▶ reply
//!   docs/research/data/0024 ──▶ BaseRates┘         │                     │
//!                                                  │                     └─ fail ─▶ template
//!                                                  └─ the ONLY thing the model sees
//! ```
//!
//! Four things decide four different questions, and keeping them apart is the
//! whole design:
//!
//! - **What the numbers are** — the instruments, deterministically.
//! - **The verdict** — a rule, so a refusal is reproducible from a recording.
//! - **What the headline is, what matters, the framing, the tone** — the model.
//!   This is real judgement and it is where the product's voice comes from.
//! - **Whether a number in the output is real** — a check, after generation.
//!
//! The model may not introduce a fact. That is a deliberate narrowing of "the
//! model makes judgements", and it is the same shape as `radar-signer`'s
//! `verify::check`, which re-decodes the bytes rather than trusting the
//! caller's description of them. **The signer re-reads the bytes it signs; the
//! roaster re-reads the numbers it posts.**
//!
//! # Its caller
//!
//! `radar roast <mint>`, in `radar-cli`, which prints the reply to stdout. The
//! whole pipeline is exercisable offline and with no X account, no credential
//! and no key — which is the point of building it before the adapter.
//!
//! # What it does not do
//!
//! It does not post anything, hold a credential, or know that X exists. It does
//! not read the store. It does not trade, sign, or touch `Policy::CLOSED`.

#![forbid(unsafe_code)]

pub mod baserates;
pub mod fidelity;
pub mod forbidden;
pub mod render;
pub mod sheet;
pub mod verdict;
pub mod voice;

pub use baserates::BaseRates;
pub use sheet::{Fact, FactSheet};
pub use verdict::{Verdict, template};
pub use voice::{Fellback, Reply, write};

use radar_model::Provider;
use radar_onchain::Dossier;

/// Builds the reply for a dossier.
///
/// The one function a caller needs. `rates` is `None` when the snapshot could
/// not be read, and `provider` is `None` when no model is configured — neither
/// is an error, and both make the reply say less rather than say more.
#[must_use]
pub fn roast(
    dossier: &Dossier,
    rates: Option<&BaseRates>,
    provider: Option<&dyn Provider>,
) -> (FactSheet, Reply) {
    let sheet = FactSheet::build(dossier, rates);
    let reply = write(&sheet, provider);
    (sheet, reply)
}
