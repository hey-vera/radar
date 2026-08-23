// SPDX-License-Identifier: Apache-2.0
//! Exit analysis: can this position actually be sold, and at what size.
//!
//! Radar's premise is that most losses in this market are not bad entries but
//! positions that could be opened and not closed. So the exit is analysed before
//! the entry, and [`radar_risk`] refuses any proposal without a measured one —
//! `simulated_exit_capacity: None` is a refusal, not a default.
//!
//! Two halves, and neither substitutes for the other:
//!
//! - [`mint`] reads the mint account: authorities, and the Token-2022 extensions
//!   that can stop or tax a transfer. A price quote will happily price a token
//!   with a transfer hook that reverts on sell.
//! - [`exit`] measures the curve: what a sale actually returns at the intended
//!   size and beyond it. A token can be perfectly transferable and still have ten
//!   dollars of depth.
//!
//! Everything here is a pure function of bytes and quotes. The router lives
//! behind [`exit::Quoter`], so every rule that decides whether a position is
//! allowed can be exercised offline.

#![forbid(unsafe_code)]

pub mod exit;
pub mod jupiter;
pub mod mint;
pub mod rpc;

pub use exit::{Confidence, ExitReport, QuoteError, QuotePoint, Quoter, capacity_table, probe};
pub use jupiter::JupiterQuoter;
pub use mint::{Extension, MintError, MintStructure};
pub use rpc::{FetchError, RpcClient};
