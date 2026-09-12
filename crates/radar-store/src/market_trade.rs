// SPDX-License-Identifier: Apache-2.0
//! One trade on the venue-agnostic market tape, as `radar-backfill`'s
//! market-tape collector records it.
//!
//! # Why this is not `Table::Trades`
//!
//! `Table::Trades` holds pump.fun instructions decoded from raw bytes: it
//! carries `requested_amount`, `limit_amount` and `accepted_any_price` because
//! those come straight off a `buy`/`sell` instruction, and its `side` is read
//! from the instruction's own discriminator. A swap collected here has no
//! instruction and no discriminator — it is a token leg and a quote leg
//! outer-joined from raw transfers, on whichever venue moved them, and its side
//! is inferred from which account looks like the pool
//! (`radar_backfill::market::fold::detect_pool`). Twelve files across four
//! crates read `Table::Trades`' exact shape, and none of them can read this one
//! without also being handed a side they never asked to reason about. A
//! separate table is additive; folding this into `Table::Trades` would not be.
//!
//! # Nullability, and why it is not a compiler guarantee
//!
//! [`MarketTrade::side`] is never null: [`MarketSide::Unknown`] is a genuine
//! value, not an absence, for exactly the case
//! [`radar_backfill::market::fold::detect_pool`] itself documents — no account
//! cleared the pool-share threshold, so nothing here claims a direction it did
//! not establish.
//!
//! [`MarketTrade::quote_amount`], [`MarketTrade::quote_mint`] and
//! [`MarketTrade::price`] are null exactly when no quote leg was found for the
//! transaction, or when the quote leg summed to a size this fold could not
//! price. [`MarketTrade::trader`] is null whenever the side could not be
//! established, since the trader is read off whichever leg they authorised and
//! that leg is only known once the side is. A null in any of these must never
//! read back as a zero or an empty string standing in for "no leg found" — see
//! [`schema::schema_for`](crate::schema::schema_for) and the writer's
//! `append_option` calls, which are the two places this could otherwise leak.

use radar_types::{Address, Signature, Slot};
use serde::{Deserialize, Serialize};

/// Which way a market trade went, when it can be told.
///
/// Three states rather than two: [`Table::Trades`](crate::event::Table::Trades)
/// reads its side off a decoded instruction, which always says buy or sell. A
/// trade folded from raw transfers has no such certainty, and
/// [`Self::Unknown`] is the honest answer when the pool side of the trade could
/// not be identified — never a guessed direction.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarketSide {
    /// The trader acquired the mint.
    Buy,
    /// The trader disposed of the mint.
    Sell,
    /// The pool side of the trade could not be identified confidently.
    Unknown,
}

impl MarketSide {
    /// The name used in the stored column.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Buy => "buy",
            Self::Sell => "sell",
            Self::Unknown => "unknown",
        }
    }

    /// The stored name, back.
    ///
    /// An unrecognised value reads as [`Unknown`](Self::Unknown): a store
    /// written by a later build may have taught this column a value this one
    /// does not recognise, and claiming a direction this build cannot name is
    /// the wrong direction to guess in.
    #[must_use]
    pub fn from_str_or_unknown(s: &str) -> Self {
        match s {
            "buy" => Self::Buy,
            "sell" => Self::Sell,
            _ => Self::Unknown,
        }
    }
}

/// One trade on the venue-agnostic tape.
///
/// The row shape `radar_backfill::market::fold::Trade` returns, plus the mint
/// it belongs to — the one field `Trade` omits because it is folded one mint at
/// a time, and a stored row has to say which mint it is without a caller
/// already knowing.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct MarketTrade {
    /// The mint traded.
    pub mint: Address,
    /// `block_timestamp`, `YYYY-MM-DD HH:MM:SS.ffffff` — the same string
    /// CryptoHouse returned, kept verbatim rather than converted to an epoch
    /// so the tape endpoints can filter on it lexicographically, the same way
    /// the live fold already ordered on it.
    pub ts: String,
    /// The slot the transaction landed in — this table's point-in-time key.
    pub slot: Slot,
    /// The transaction signature.
    pub signature: Signature,
    /// Buy, sell, or undetermined. Never absent; see the type's own doc.
    pub side: MarketSide,
    /// The mint amount, adjusted by its `decimals`.
    pub token_amount: f64,
    /// The quote amount, adjusted by its `decimals`. `None` when no quote leg
    /// was found for this transaction — never `Some(0.0)`.
    pub quote_amount: Option<f64>,
    /// Which quote asset this trade priced against. `None` exactly when
    /// [`Self::quote_amount`] is `None`.
    pub quote_mint: Option<Address>,
    /// `quote_amount / token_amount`. `None` whenever either side of that
    /// division is unknown.
    pub price: Option<f64>,
    /// The wallet identified as the trader, when the pool side of the trade
    /// could be told apart from it. `None` when [`Self::side`] is
    /// [`MarketSide::Unknown`], and independently possible even when a quote
    /// leg was found: a trade can be priced without its trader being known.
    pub trader: Option<Address>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_side_round_trips_through_its_stored_name() {
        for side in [MarketSide::Buy, MarketSide::Sell, MarketSide::Unknown] {
            assert_eq!(MarketSide::from_str_or_unknown(side.as_str()), side);
        }
    }

    #[test]
    fn an_unrecognised_stored_side_reads_as_unknown_not_a_guess() {
        // A future build may write a side this one has never heard of. Reading
        // it as Buy or Sell would be a coin flip wearing a value's clothes.
        assert_eq!(
            MarketSide::from_str_or_unknown("sideways"),
            MarketSide::Unknown
        );
        assert_eq!(MarketSide::from_str_or_unknown(""), MarketSide::Unknown);
    }
}
