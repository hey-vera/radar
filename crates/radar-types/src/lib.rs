// SPDX-License-Identifier: Apache-2.0
//! Domain vocabulary for Radar.
//!
//! This crate holds types and nothing else: no I/O, no network, no clock. Every
//! other crate in the workspace depends on it, so anything that can fail at
//! runtime belongs somewhere further out.
//!
//! Three ideas here carry more weight than the rest and are worth reading before
//! the code that uses them:
//!
//! - [`Slot`] is the only clock Radar has. Wall-clock time is never the basis of
//!   a decision, because a chain does not advance on wall-clock time and a
//!   replay of a decision must land on the same inputs the live run saw.
//! - [`Mutability`] is declared per fact, not per provider. It is what makes
//!   "never fetch this twice" a property the cache can enforce rather than a
//!   convention a caller has to remember.
//! - [`MicroUsd`] is integer money. Costs are summed across millions of calls and
//!   compared against hard budget caps; floating point has no place in that path.

#![forbid(unsafe_code)]

/// Base64, because three crates need it and one of them must not depend on the
/// signer to get it.
///
/// It lived in `radar-signer` and was used from `radar-serve`, which meant the
/// crate holding `Key::sign` was compiled into the internet-facing process. Rule
/// 1's "no network, no listener" was true of the signer *binary* and not of the
/// signer *crate*, and the only thing the web server ever wanted from it was
/// this.
pub mod b64;

mod address;
mod money;
mod mutability;
mod provenance;
mod slot;

pub use address::{Address, AddressParseError, Signature};
pub use money::MicroUsd;
pub use mutability::{Latch, LatchReopened, Mutability, Revalidation};
pub use provenance::{EvidenceTier, Provenance, SourceId, Trust};
pub use slot::{Slot, SlotDelta};
