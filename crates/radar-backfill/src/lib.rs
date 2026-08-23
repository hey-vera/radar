// SPDX-License-Identifier: Apache-2.0
//! Bulk historical extraction from CryptoHouse into Radar's own store.
//!
//! Reconstructing pump.fun history from RPC would cost roughly $38,900 and 267 TB
//! for six months, and the commercial archive is $200/month for data that mixes
//! labels with forward-looking aggregates. CryptoHouse holds the raw instruction
//! bytes for the whole chain and is free to query, so Radar decodes its own
//! history with the same decoder the live recorder uses. See
//! [ADR 0002](https://github.com/hey-vera/radar/blob/main/docs/adr/0002-historical-data-comes-from-cryptohouse-not-a-vendor-archive.md).
//!
//! The crate splits transport from conversion on purpose: [`extract`] is pure
//! functions from rows to events, so every way an extraction can refuse a row is
//! unit-tested without a network.

#![forbid(unsafe_code)]

pub mod checkpoints;
pub mod cryptohouse;
pub mod extract;
pub mod outcomes;

pub use cryptohouse::{Client, QueryError};
pub use extract::{Row, Scope, Skipped, Stats, events_from_rows, query_for_window};
pub use outcomes::{MINTS_PER_BATCH, outcomes_from_rows};
