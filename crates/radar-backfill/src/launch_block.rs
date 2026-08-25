// SPDX-License-Identifier: Apache-2.0
//! Reading a launch block's shape from CryptoHouse.
//!
//! The fetching half of [`radar_graph`]. It lives here because this crate
//! already owns the CryptoHouse client and the knowledge of that endpoint's
//! sharp edges (ADR 0002), and `radar-graph` stays pure policy.
//!
//! # Why this is a Tier-1 look and not a recorded field
//!
//! The obvious alternative is to record the recipient count for every launch as
//! it is extracted. It was considered and rejected on cost. The recorder's
//! extraction groups token transfers by *transaction*; the shape needs them
//! grouped by (mint, slot), which for the whole window means grouping every
//! token transfer on Solana rather than the pump.fun ones. On 2026-08-24 a
//! two-minute spam burst pushed the existing query past the endpoint's limits
//! and took the recorder down for thirteen hours; making that query heavier for
//! every launch, when the answer is wanted for a handful, is the wrong trade.
//!
//! Asked per candidate the query is cheap, because a mint filter prunes hard.

use radar_graph::{LaunchBlockShape, LaunchBlockSource};
use radar_types::{Address, Slot};
use serde::Deserialize;

use crate::cryptohouse::{Client, QueryError};

/// Assets that are not the subject of a launch. Counting recipients of wrapped
/// SOL would measure the market, not the token.
const QUOTE_MINTS: &[&str] = &[
    "So11111111111111111111111111111111111111112",
    "So11111111111111111111111111111111111111111",
];

/// One aggregate row.
#[derive(Debug, Deserialize)]
struct ShapeRow {
    recipients: String,
    transactions: String,
}

/// Reads launch-block shapes from CryptoHouse.
#[derive(Default)]
pub struct CryptoHouseBlocks {
    client: Client,
}

impl CryptoHouseBlocks {
    /// Builds one against a given client.
    #[must_use]
    pub const fn new(client: Client) -> Self {
        Self { client }
    }
}

/// Builds the query for one mint in one slot.
///
/// Bounded below by a timestamp as well as the slot, because `token_transfers`
/// prunes on `block_timestamp` and nothing else — a slot filter alone reads
/// naturally, prunes nothing, and fails at the ten-billion-row ceiling even for
/// a single mint (ADR 0002). The mint filter is what makes it cheap; the
/// timestamp is what makes it possible.
#[must_use]
pub fn query_for_shape(mint: &Address, slot: Slot, since: &str) -> String {
    let quotes = QUOTE_MINTS
        .iter()
        .map(|m| format!("'{m}'"))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "SELECT toString(uniqExact(destination)) AS recipients, \
                toString(uniqExact(tx_signature)) AS transactions \
         FROM solana.token_transfers \
         WHERE block_timestamp >= '{since}' AND block_slot = {slot} \
           AND mint = '{mint}' AND mint NOT IN ({quotes})",
        slot = slot.get()
    )
}

/// The earliest timestamp a launch-block query is bounded by.
///
/// A launch block is being asked about because the launch was recorded, so the
/// answer is always inside the store's own window. This is deliberately far
/// enough back to cover it and no further: widening it costs scan time on every
/// call for data that cannot be relevant.
pub const SINCE: &str = "2026-08-01 00:00:00";

impl LaunchBlockSource for CryptoHouseBlocks {
    type Error = QueryError;

    fn shape_at(&self, mint: &Address, slot: Slot) -> Result<LaunchBlockShape, Self::Error> {
        let rows: Vec<ShapeRow> = self.client.query(&query_for_shape(mint, slot, SINCE))?;

        // No row means the query ran and found nothing, which is a real
        // observation: the token had no transfers in that slot. An error would
        // have surfaced above.
        let Some(row) = rows.first() else {
            return Ok(LaunchBlockShape {
                recipients: 0,
                transactions: 0,
            });
        };
        Ok(LaunchBlockShape {
            recipients: row.recipients.parse().unwrap_or(0),
            transactions: row.transactions.parse().unwrap_or(0),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mint() -> Address {
        "2Rt18SqHXcgzUU1P94Qr71A9URcpmkwB99cD5SGXpump"
            .parse()
            .expect("a real mainnet mint")
    }

    #[test]
    fn the_query_prunes_on_the_partition_key_as_well_as_the_slot() {
        // `token_transfers` prunes on block_timestamp only. A slot filter alone
        // reads naturally, prunes nothing, and dies at the row ceiling even for
        // one mint -- the trap ADR 0002 records.
        let sql = query_for_shape(&mint(), Slot(441_251_921), SINCE);
        assert!(sql.contains("block_timestamp >="), "must prune");
        assert!(sql.contains("block_slot = 441251921"), "must be one slot");
        assert!(sql.contains(&mint().to_string()), "must name the mint");
    }

    #[test]
    fn the_query_asks_about_exactly_one_slot() {
        // The whole signal is that a bundle is visible in the launch block
        // specifically. A range would dissolve it into ordinary early trading.
        let sql = query_for_shape(&mint(), Slot(441_251_921), SINCE);
        assert!(!sql.contains(">= 441251921"), "no lower-bounded slot range");
        assert!(!sql.contains("BETWEEN"), "no slot range");
    }

    #[test]
    fn quote_assets_are_excluded() {
        // Counting recipients of wrapped SOL would measure the market rather
        // than the token.
        let sql = query_for_shape(&mint(), Slot(1), SINCE);
        assert!(sql.contains("So11111111111111111111111111111111111111112"));
    }
}
