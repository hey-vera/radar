// SPDX-License-Identifier: Apache-2.0
//! The venue-agnostic trade tape: the query that builds it and the pure fold
//! that turns rows into domain types.
//!
//! Lives here, not in `radar-serve`, because `radar-backfill`'s market-tape
//! collector (`crate::market_tape`) is what spends the CryptoHouse queries now
//! — `radar-serve` reads only the store it fills. `radar-serve` already depends
//! on this crate and imports these two modules directly rather than holding a
//! second copy that could drift from the one actually run against CryptoHouse.

pub mod fold;
pub mod query;
