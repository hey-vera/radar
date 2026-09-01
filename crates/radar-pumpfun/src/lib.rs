// SPDX-License-Identifier: Apache-2.0
//! Building pump.fun instructions, and pricing them off the curve.
//!
//! [ADR 0009](https://github.com/hey-vera/radar/blob/main/docs/adr/0009-radar-builds-its-own-pump-fun-swaps.md)
//! decides that Radar builds its own swaps rather than asking an aggregator for
//! one. Research
//! [0021](https://github.com/hey-vera/radar/blob/main/docs/research/0021-the-signer-cannot-read-the-only-venue-that-lists-them.md)
//! is why: Jupiter routes pre-graduation pump.fun liquidity only as a versioned
//! transaction, the signer reads only legacy ones, and Radar selects
//! pre-graduation tokens exclusively. Any two of those are fine.
//!
//! The venue itself was never the obstacle — every capture behind this crate is
//! a **legacy** transaction, so people trade this curve that way every block.
//!
//! # Pure, like the risk kernel and for the same reason
//!
//! No clock, no network, no ambient state. Reserves and addresses come in as
//! arguments; instructions and prices go out. That is what makes a built
//! transaction reproducible from a recording, and it is why this is a crate
//! rather than a module inside `radar-exec`, which holds an HTTP client.
//!
//! # What it does not do
//!
//! It does not sign, submit, or hold a key, and it has no function that takes
//! one. It emits an instruction; turning that into an authorised, signed
//! transaction is `radar-risk`'s and `radar-signer`'s job, in that order, and
//! nothing here shortens that path.

#![forbid(unsafe_code)]

pub mod curve;
pub mod fees;
pub mod instruction;
pub mod pda;
pub mod transaction;

pub use curve::{BondingCurve, Fill};
pub use fees::{FeeConfig, Fees};
pub use instruction::Trade;
pub use transaction::{AccountMeta, Instruction, Unbuildable};
