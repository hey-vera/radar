// SPDX-License-Identifier: Apache-2.0
//! Turning CryptoHouse rows into the shapes the market endpoints return.
//!
//! Split from [`super::query`] the way [`crate::extract`] is split from
//! [`crate::cryptohouse`]: everything here is a pure function from a row to a
//! domain type, so every honesty rule this module has to keep — a
//! decimals-unadjusted amount never escaping, a missing quote leg never
//! becoming a zero price — is a fact about a function signature, checked with
//! a fixture, rather than a fact about a live query nobody can run in a test.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Parses a raw base-unit amount, as CryptoHouse's `toString(Decimal(38,9))`
/// renders it.
///
/// Observed live on 2026-09-11 as a bare integer string (`"56626"`), never
/// with a decimal point, because every value these queries touch is a whole
/// number of base units. Still tolerant of an all-zero fractional part in case
/// that ever changes, and refuses — never truncates — a genuinely fractional
/// one, which would mean this column stopped meaning what it means.
fn parse_units(raw: &str) -> Option<i128> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    match raw.split_once('.') {
        None => raw.parse().ok(),
        Some((whole, frac)) if frac.chars().all(|c| c == '0') => whole.parse().ok(),
        Some(_) => None,
    }
}

/// The only path from a raw base-unit amount to a displayable one.
///
/// **This is the fix for the sketch's core bug.** `value` is a raw
/// `Decimal(38,9)` that has not been divided by `decimals`, and every domain
/// type in this module holds only what this function returns — never the raw
/// string and the decimals count side by side — so there is no field an
/// unadjusted amount could reach a response through.
///
/// `f64` rather than a decimal string: every number this module produces goes
/// on a trading-terminal chart, which wants a JS number either way, and this
/// is display precision for a chart, not the ledger precision
/// [`crate::prices::PriceRow`] needs for a replay to compare
/// byte-for-byte. Returns `None` for a value or a decimals count that does not
/// parse, never a guess.
#[must_use]
pub fn adjust(raw_value: &str, decimals: &str) -> Option<f64> {
    let units = parse_units(raw_value)?;
    let places: i32 = decimals.trim().parse().ok()?;
    #[expect(
        clippy::cast_precision_loss,
        reason = "display precision for a chart; see the doc comment"
    )]
    let units = units as f64;
    Some(units / 10f64.powi(places))
}

/// Which way a trade went, when it can be told.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    /// The trader acquired the mint.
    Buy,
    /// The trader disposed of the mint.
    Sell,
    /// The pool side of the trade could not be identified confidently.
    ///
    /// Not an error and not a coin flip: neither end of the trade appeared
    /// more often than the other across the window,
    /// so nothing here claims a direction it did not establish. A wrong
    /// direction is worse than an admitted unknown.
    Unknown,
}

/// One transaction's token leg outer-joined against its quote leg, as
/// [`super::query::trades_query`] returns it.
///
/// A missing quote leg comes back from the `LEFT JOIN` as ClickHouse's default
/// for each column's type — empty string for `quote_mint` and `quote_authority`,
/// `"0"` for `quote_value` — never as JSON `null`, so every field here is a
/// plain (non-`Option`) `String` and an empty `quote_mint` is what "absent"
/// looks like.
#[derive(Debug, Clone, Deserialize)]
pub struct TapeRow {
    /// The mint this row's token leg moved.
    ///
    /// Present because [`super::query::trades_query`] is batched across a
    /// shortlist of mints in one round trip (the change that makes the
    /// collector fit inside CryptoHouse's quota): a caller has to split the
    /// combined result back into one row set per mint before folding, since
    /// [`end_frequencies`] assumes every row it sees belongs to a single mint's
    /// activity.
    pub mint: String,
    /// `block_timestamp`, `YYYY-MM-DD HH:MM:SS.ffffff`.
    pub ts: String,
    /// `block_slot`.
    pub slot: String,
    /// The transaction signature.
    pub sig: String,
    /// The mint's raw transfer sum for this transaction.
    pub token_value: String,
    /// The mint's `decimals`.
    pub token_decimals: String,
    /// A token-account address moving the mint outward, if any leg did.
    pub token_source: String,
    /// A token-account address receiving the mint, if any leg did.
    pub token_destination: String,
    /// Who authorised the mint leg — the trader, on a sell.
    pub token_authority: String,
    /// The quote leg's raw sum. `"0"` when the join found no matching row.
    pub quote_value: String,
    /// The quote leg's `decimals`.
    pub quote_decimals: String,
    /// The quote leg's mint. Empty when the join found no matching row —
    /// **this, not `quote_value`, is what "no quote leg" is tested against**,
    /// because a real trade can genuinely move zero net quote after fees.
    pub quote_mint: String,
    /// Who authorised the quote leg — the trader, on a buy.
    pub quote_authority: String,
}

/// One trade on the tape, as the API returns it.
///
/// Holds only adjusted amounts — see [`adjust`] — and an absent quote leg is
/// `None` on every field it would have filled, never a zero.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Trade {
    /// `block_timestamp`, `YYYY-MM-DD HH:MM:SS.ffffff`.
    pub ts: String,
    /// The slot the transaction landed in.
    pub slot: u64,
    /// The transaction signature.
    pub signature: String,
    /// Buy, sell, or undetermined.
    pub side: Side,
    /// The mint amount, adjusted by its `decimals`.
    pub token_amount: f64,
    /// The quote amount, adjusted by its `decimals`. `None` when no quote leg
    /// was found for this transaction — never `Some(0.0)`.
    pub quote_amount: Option<f64>,
    /// Which quote asset this trade priced against. `None` exactly when
    /// [`Self::quote_amount`] is `None`.
    pub quote_mint: Option<String>,
    /// `quote_amount / token_amount`. `None` whenever either side of that
    /// division is unknown, which is the fix for the sketch reporting a
    /// dropped trade's price as zero.
    pub price: Option<f64>,
    /// The wallet identified as the trader, when the pool side of the trade
    /// could be told apart from it.
    pub trader: Option<String>,
}

/// How often each token account appears as an end of a trade in this window.
///
/// **The input to the side decision, and the replacement for a single
/// window-wide pool account.** See [`side_and_trader`] for why one account is
/// not enough.
fn end_frequencies(rows: &[TapeRow]) -> HashMap<&str, u64> {
    let mut seen: HashMap<&str, u64> = HashMap::new();
    for row in rows {
        if !row.token_source.is_empty() {
            *seen.entry(row.token_source.as_str()).or_default() += 1;
        }
        if !row.token_destination.is_empty() {
            *seen.entry(row.token_destination.as_str()).or_default() += 1;
        }
    }
    seen
}

/// One row's side and trader, given the pool this mint's window resolved to.
///
/// The direction the mint moved *relative to the pool* decides the side: the
/// pool is the source on a buy and the destination on a sell. The trader is
/// read off whichever leg they authorised — the quote leg on a buy (they sent
/// the quote asset), the mint leg on a sell (they sent the token) — because a
/// `Transfer`/`TransferChecked` row's `authority` is who authorised moving
/// funds *out* of its source, never who merely received them.
fn side_and_trader(row: &TapeRow, seen: &HashMap<&str, u64>) -> (Side, Option<String>) {
    let non_empty = |s: &str| (!s.is_empty()).then(|| s.to_owned());
    if row.token_source.is_empty()
        || row.token_destination.is_empty()
        || row.token_source == row.token_destination
    {
        return (Side::Unknown, None);
    }
    let from = seen.get(row.token_source.as_str()).copied().unwrap_or(0);
    let to = seen
        .get(row.token_destination.as_str())
        .copied()
        .unwrap_or(0);
    match from.cmp(&to) {
        // The mint left the busier account: a pool paid out, so the trader
        // bought. They are on the quote leg, having sent the quote asset.
        std::cmp::Ordering::Greater => (Side::Buy, non_empty(&row.quote_authority)),
        // The mint arrived at the busier account: a pool took it in, so the
        // trader sold. They authorised the mint leg, having sent the token.
        std::cmp::Ordering::Less => (Side::Sell, non_empty(&row.token_authority)),
        // Neither end is busier. Nothing here separates a pool from a trader,
        // and a direction guessed from a tie is a direction invented.
        std::cmp::Ordering::Equal => (Side::Unknown, None),
    }
}

fn trade_from_row(row: &TapeRow, seen: &HashMap<&str, u64>) -> Option<Trade> {
    let token_amount = adjust(&row.token_value, &row.token_decimals)?;
    let slot: u64 = row.slot.trim().parse().ok()?;
    let (side, trader) = side_and_trader(row, seen);

    // The empty `quote_mint` is what a missing join match looks like -- see
    // `TapeRow`'s doc comment. Everything downstream of "no quote leg" stays
    // `None`, on purpose: a trade the join could not price is still a trade.
    let has_quote_leg = !row.quote_mint.trim().is_empty();
    let quote_amount = has_quote_leg
        .then(|| adjust(&row.quote_value, &row.quote_decimals))
        .flatten();
    let quote_mint = quote_amount.is_some().then(|| row.quote_mint.clone());
    let price = match (quote_amount, token_amount) {
        (Some(quote), token) if token > 0.0 => Some(quote / token),
        _ => None,
    };

    Some(Trade {
        ts: row.ts.clone(),
        slot,
        signature: row.sig.clone(),
        side,
        token_amount,
        quote_amount,
        quote_mint,
        price,
        trader,
    })
}

/// Every row that converts, newest first.
///
/// A row that fails to parse (`token_value`, `decimals` or `slot` malformed)
/// is skipped rather than guessed at — the same discipline
/// [`crate::extract::events_from_rows`] uses, though this is rare
/// enough on CryptoHouse's own `toString` output that no [`Stats`]-style
/// counter is kept for it here.
#[must_use]
pub fn fold_tape(rows: &[TapeRow]) -> Vec<Trade> {
    let seen = end_frequencies(rows);
    let mut trades: Vec<Trade> = rows
        .iter()
        .filter_map(|r| trade_from_row(r, &seen))
        .collect();
    // Lexicographic order agrees with chronological order for this timestamp
    // format, so no epoch parse is needed just to sort.
    trades.sort_by(|a, b| b.ts.cmp(&a.ts).then_with(|| b.signature.cmp(&a.signature)));
    trades
}

/// One OHLCV bucket, folded from priced trades only.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Candle {
    /// The start of the bucket in **seconds since the Unix epoch, UTC**.
    ///
    /// The field a chart plots against, and the reason it exists beside
    /// [`Self::bucket_start`]: that one is `YYYY-MM-DD HH:MM:SS` with **no
    /// zone marker**, and JavaScript's `Date.parse` reads exactly that shape
    /// as *local* time. A browser in New York would have drawn every candle
    /// shifted by four hours and shown no sign of it. Serialised first so the
    /// unambiguous field is the one a reader of the JSON meets first.
    pub time: i64,
    /// The start of the bucket, `YYYY-MM-DD HH:MM:SS` UTC — for a human
    /// reading the response, never for arithmetic. See [`Self::time`].
    pub bucket_start: String,
    /// Price of the earliest priced trade in the bucket.
    pub open: f64,
    /// Highest price in the bucket.
    pub high: f64,
    /// Lowest price in the bucket.
    pub low: f64,
    /// Price of the latest priced trade in the bucket.
    pub close: f64,
    /// Summed quote amount of priced trades in the bucket.
    pub quote_volume: f64,
    /// Summed token amount of priced trades in the bucket.
    pub token_volume: f64,
    /// How many priced trades folded into this bucket.
    pub trade_count: u64,
}

fn ts_to_epoch(ts: &str) -> Option<i64> {
    let without_fraction = ts.split('.').next().unwrap_or(ts);
    radar_store::to_epoch(without_fraction).ok()
}

/// Buckets trades into candles of `interval_seconds`, in time order.
///
/// **A trade with no price never seeds or touches a candle.** That is the
/// fix for the plan's own warning about `PricePath`: folding an unpriced fill
/// into a bucket's high or low would print a level nothing actually traded
/// at. An interval with no priced trade in it is simply absent from the
/// series — never an invented zero-volume bar.
#[must_use]
pub fn fold_candles(trades: &[Trade], interval_seconds: i64) -> Vec<Candle> {
    if interval_seconds <= 0 {
        return Vec::new();
    }
    // Carry the price alongside the trade rather than filtering on
    // `price.is_some()` and unwrapping it later. The filter-then-expect version
    // was correct and still obliged this function to document a panic it could
    // not actually reach; pairing them in the collect makes the guarantee the
    // type's rather than the reader's.
    let mut ascending: Vec<(f64, &Trade)> = trades
        .iter()
        .filter_map(|t| t.price.map(|p| (p, t)))
        .collect();
    ascending.sort_by(|(_, a), (_, b)| a.ts.cmp(&b.ts).then_with(|| a.signature.cmp(&b.signature)));

    let mut candles: Vec<(i64, Candle)> = Vec::new();
    for (price, trade) in ascending {
        let Some(epoch) = ts_to_epoch(&trade.ts) else {
            continue;
        };
        let bucket_epoch = epoch.div_euclid(interval_seconds) * interval_seconds;
        match candles.last_mut() {
            Some((last_epoch, candle)) if *last_epoch == bucket_epoch => {
                candle.high = candle.high.max(price);
                candle.low = candle.low.min(price);
                candle.close = price;
                candle.quote_volume += trade.quote_amount.unwrap_or(0.0);
                candle.token_volume += trade.token_amount;
                candle.trade_count += 1;
            }
            _ => candles.push((
                bucket_epoch,
                Candle {
                    time: bucket_epoch,
                    bucket_start: radar_store::from_epoch(bucket_epoch),
                    open: price,
                    high: price,
                    low: price,
                    close: price,
                    quote_volume: trade.quote_amount.unwrap_or(0.0),
                    token_volume: trade.token_amount,
                    trade_count: 1,
                },
            )),
        }
    }
    candles.into_iter().map(|(_, c)| c).collect()
}

/// A raw transfer of one mint, for the holder fold.
#[derive(Debug, Clone, Deserialize)]
pub struct HolderRow {
    /// The moving token account, or empty for a mint with no source (a
    /// creation-time `MintTo` has none).
    pub source: String,
    /// The receiving token account, or empty for a `Burn`'s non-existent
    /// destination.
    pub destination: String,
    /// The raw amount moved.
    pub value: String,
    /// The mint's `decimals`.
    pub decimals: String,
    /// `Transfer`, `TransferChecked`, `MintTo`, `Burn`, or one of their
    /// checked/fee variants.
    pub transfer_type: String,
}

/// One token account's folded balance.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Holder {
    /// The token account — **not a wallet**. See [`HoldersFold::granularity`].
    pub account: String,
    /// The account's balance, adjusted by `decimals`.
    pub balance: f64,
}

/// What [`fold_holders`] returns, captioned with what kind of fact it is.
///
/// The three qualifications the plan calls load-bearing, each its own field
/// rather than folded into prose a caller could drop: this is a fold of
/// observed transfers, over token accounts rather than owning wallets, across
/// a stated window.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HoldersFold {
    /// Always `"folded_transfers"`. Named explicitly, and distinct from a read
    /// of current on-chain token-account state — a fact this endpoint does not
    /// have.
    pub fact: &'static str,
    /// Always `"token_account"`. A wallet holding several accounts is counted
    /// once per account, which overstates holder count relative to owners.
    pub granularity: &'static str,
    /// The window's start, as given to the query.
    pub from: String,
    /// The window's end, as given to the query.
    pub to: String,
    /// Balances that survived the fold above zero, richest first.
    pub holders: Vec<Holder>,
}

/// Folds observed transfers into balances, holding at most `limit` accounts.
///
/// `MintTo` and `Burn` (and their checked variants) are conservation events —
/// credited or debited without a symmetric other side — so the fold does not
/// silently drift relative to actual supply. `TransferCheckedWithFee` is
/// folded at its gross `value`: the separate `fee` column is not deducted from
/// the destination here, which is a known approximation for a fee-charging
/// mint and not one this data source resolves without a second pass.
#[must_use]
pub fn fold_holders(rows: &[HolderRow], from: &str, to: &str, limit: usize) -> HoldersFold {
    let mut balances: std::collections::BTreeMap<String, i128> = std::collections::BTreeMap::new();
    let mut decimals: Option<u32> = None;
    for row in rows {
        let Some(units) = parse_units(&row.value) else {
            continue;
        };
        if decimals.is_none() {
            decimals = row.decimals.trim().parse().ok();
        }
        match row.transfer_type.as_str() {
            "MintTo" | "MintToChecked" if !row.destination.is_empty() => {
                *balances.entry(row.destination.clone()).or_default() += units;
            }
            "Burn" | "BurnChecked" if !row.source.is_empty() => {
                *balances.entry(row.source.clone()).or_default() -= units;
            }
            _ => {
                if !row.source.is_empty() {
                    *balances.entry(row.source.clone()).or_default() -= units;
                }
                if !row.destination.is_empty() {
                    *balances.entry(row.destination.clone()).or_default() += units;
                }
            }
        }
    }

    let mut holders: Vec<Holder> = balances
        .into_iter()
        .filter(|(_, raw)| *raw > 0)
        .map(|(account, raw)| Holder {
            account,
            #[expect(clippy::cast_precision_loss, reason = "display precision for a chart")]
            // `i32::try_from` rather than `as`: a decimals count that does not
            // fit an `i32` is not a token with very many decimal places, it is
            // a value this fold should refuse to scale by. Falling back to the
            // unscaled figure would be the "absent becomes a wrong number"
            // failure, so the balance keeps its raw units and the caller is no
            // worse off than for a mint whose decimals were never read.
            balance: decimals
                .and_then(|d| i32::try_from(d).ok())
                .map_or(raw as f64, |d| raw as f64 / 10f64.powi(d)),
        })
        .collect();
    holders.sort_by(|a, b| b.balance.total_cmp(&a.balance));
    holders.truncate(limit);

    HoldersFold {
        fact: "folded_transfers",
        granularity: "token_account",
        from: from.to_owned(),
        to: to.to_owned(),
        holders,
    }
}

/// One candidate mint's activity, from [`super::query::coin_candidates_query`].
#[derive(Debug, Clone, Deserialize)]
pub struct CoinCandidateRow {
    /// The mint.
    pub mint: String,
    /// Distinct transactions moving it in the window.
    pub tx_count: String,
    /// Raw summed transfer value in the window.
    pub token_volume: String,
    /// The mint's `decimals`.
    pub token_decimals: String,
}

/// One mint's first and last priced fill, from
/// [`super::query::coin_prices_query`].
#[derive(Debug, Clone, Deserialize)]
pub struct CoinPriceRow {
    /// The mint.
    pub mint: String,
    /// Priced trades folded into this row.
    pub trade_count: String,
    /// The earliest fill's raw token amount.
    pub first_token_value: String,
    /// The earliest fill's `decimals`.
    pub first_token_decimals: String,
    /// The earliest fill's raw quote amount.
    pub first_quote_value: String,
    /// The earliest fill's quote `decimals`.
    pub first_quote_decimals: String,
    /// The latest fill's raw token amount.
    pub last_token_value: String,
    /// The latest fill's `decimals`.
    pub last_token_decimals: String,
    /// The latest fill's raw quote amount.
    pub last_quote_value: String,
    /// The latest fill's quote `decimals`.
    pub last_quote_decimals: String,
    /// Which quote asset priced this mint.
    pub quote_mint: String,
    /// Raw summed quote volume across every priced fill.
    pub quote_volume: String,
    /// The quote volume's `decimals`.
    pub quote_volume_decimals: String,
}

/// One row of the live coin list.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Coin {
    /// The mint.
    pub mint: String,
    /// Distinct transactions moving it in the window.
    pub tx_count: u64,
    /// Summed transfer volume, adjusted by `decimals`. `None` if the raw
    /// amount or its `decimals` did not parse.
    pub token_volume: Option<f64>,
    /// The quote asset the price below is denominated in. `None` exactly when
    /// [`Self::price`] is.
    pub quote_mint: Option<String>,
    /// Summed quote volume across every priced fill in the window.
    pub quote_volume: Option<f64>,
    /// The most recent priced fill's price. `None` when nothing in the
    /// window paired this mint with a quote leg — a capability gap, not a
    /// price of zero.
    pub price: Option<f64>,
    /// Percent change from the window's first priced fill to its last.
    /// `None` whenever [`Self::price`] is, or the first fill priced at zero.
    pub change_pct: Option<f64>,
}

fn priced(
    token_value: &str,
    token_decimals: &str,
    quote_value: &str,
    quote_decimals: &str,
) -> Option<f64> {
    let token = adjust(token_value, token_decimals)?;
    let quote = adjust(quote_value, quote_decimals)?;
    (token > 0.0).then_some(quote / token)
}

/// The percentage move from the window's first priced fill to its last.
///
/// Pulled out of [`fold_coins`] so the decision has a name and a test. Two
/// things it must not do, and both are one character away: divide by a first
/// price of zero -- which yields an infinity that serialises as JSON `null`
/// and so arrives looking exactly like an honest absent change -- and report a
/// ratio where a percentage is meant.
///
/// `None` when either end is missing, which is a different fact from a change
/// of zero: nothing in the window paired that mint with a quote leg, so no
/// move was measured rather than no move having happened.
#[must_use]
fn change_from(first_price: Option<f64>, last_price: Option<f64>) -> Option<f64> {
    match (first_price, last_price) {
        (Some(first), Some(last)) if first > 0.0 => Some((last - first) / first * 100.0),
        _ => None,
    }
}

/// Joins the candidate list against the shortlist's prices, in Rust rather
/// than in SQL, so a mint absent from `prices` — nothing in the window paired
/// it with a quote leg — has an explicit, testable path to `price: None`
/// rather than a join silently omitting the row.
#[must_use]
pub fn fold_coins(candidates: &[CoinCandidateRow], prices: &[CoinPriceRow]) -> Vec<Coin> {
    let by_mint: HashMap<&str, &CoinPriceRow> =
        prices.iter().map(|p| (p.mint.as_str(), p)).collect();

    candidates
        .iter()
        .map(|c| {
            let tx_count: u64 = c.tx_count.trim().parse().unwrap_or(0);
            let token_volume = adjust(&c.token_volume, &c.token_decimals);
            let Some(p) = by_mint.get(c.mint.as_str()) else {
                return Coin {
                    mint: c.mint.clone(),
                    tx_count,
                    token_volume,
                    quote_mint: None,
                    quote_volume: None,
                    price: None,
                    change_pct: None,
                };
            };
            let last_price = priced(
                &p.last_token_value,
                &p.last_token_decimals,
                &p.last_quote_value,
                &p.last_quote_decimals,
            );
            let first_price = priced(
                &p.first_token_value,
                &p.first_token_decimals,
                &p.first_quote_value,
                &p.first_quote_decimals,
            );
            let change_pct = change_from(first_price, last_price);
            Coin {
                mint: c.mint.clone(),
                tx_count,
                token_volume,
                quote_mint: last_price.is_some().then(|| p.quote_mint.clone()),
                quote_volume: adjust(&p.quote_volume, &p.quote_volume_decimals),
                price: last_price,
                change_pct,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wrapped SOL, the quote asset most of these fixtures price against.
    const WSOL: &str = "So11111111111111111111111111111111111111112";

    fn row(sig: &str, ts: &str, quote_mint: &str, quote_value: &str) -> TapeRow {
        TapeRow {
            mint: "5NfV2sy8DqXamLvYEE4LcTWzGqZc5Emv4bqqhVDWpump".to_owned(),
            ts: ts.to_owned(),
            slot: "441251921".to_owned(),
            sig: sig.to_owned(),
            token_value: "56626".to_owned(),
            token_decimals: "6".to_owned(),
            token_source: "POOL111111111111111111111111111111111111".to_owned(),
            token_destination: "TRADER11111111111111111111111111111111111".to_owned(),
            token_authority: "POOLAUTH1111111111111111111111111111111111".to_owned(),
            quote_value: quote_value.to_owned(),
            quote_decimals: "9".to_owned(),
            quote_mint: quote_mint.to_owned(),
            quote_authority: "TRADERWALLET111111111111111111111111111111".to_owned(),
        }
    }

    /// A genuinely fractional base-unit amount is refused, not truncated.
    ///
    /// `parse_units` accepts `"56626.000"` because an all-zero fraction is the
    /// same integer. It must refuse `"56626.5"`: half a base unit means this
    /// column stopped meaning whole base units, and silently dropping the `.5`
    /// would report a quantity nobody traded. Kills the mutant that flips the
    /// all-zeros test to "any digit differs".
    #[test]
    fn a_fractional_base_unit_is_refused_rather_than_truncated() {
        assert_eq!(parse_units("56626"), Some(56_626));
        assert_eq!(parse_units("56626.000"), Some(56_626));
        assert_eq!(
            parse_units("56626.5"),
            None,
            "half a base unit is not 56626"
        );
        assert_eq!(parse_units("56626.0001"), None);
    }

    /// A mint with no destination credits nobody, and a burn with no source
    /// debits nobody.
    ///
    /// **An empty account is not an account.** `token_transfers` leaves the
    /// far side blank on a mint and on a burn, and crediting `""` would build
    /// a phantom holder that outranks every real one, because every such leg
    /// for the mint collects into the same empty key. Kills the six mutants
    /// that flip or delete the two emptiness guards.
    #[test]
    fn a_mint_or_burn_with_a_blank_side_credits_nobody() {
        for kind in ["MintTo", "MintToChecked", "Burn", "BurnChecked"] {
            let folded = fold_holders(
                &[holder_row("", "", "2000000", kind)],
                "2026-09-11 00:00:00",
                "2026-09-11 01:00:00",
                10,
            )
            .holders;
            assert!(
                folded.iter().all(|h| !h.account.is_empty()),
                "{kind}: a blank side must not become a holder: {folded:?}"
            );
        }
    }

    /// The other half: a real mint credits its destination.
    ///
    /// A guard replaced with `false` makes every mint a no-op, and a test that
    /// only asserted blanks are refused would pass against it.
    #[test]
    fn a_real_mint_credits_its_destination() {
        let to = "DEST111111111111111111111111111111111111111";
        let folded = fold_holders(
            &[holder_row("", to, "2000000", "MintTo")],
            "2026-09-11 00:00:00",
            "2026-09-11 01:00:00",
            10,
        )
        .holders;
        assert_eq!(folded.len(), 1, "the destination holds the minted units");
        assert_eq!(folded[0].account, to);
        assert!((folded[0].balance - 2.0).abs() < 1e-12, "{folded:?}");
    }

    /// A price is quote over token, and zero tokens has no price.
    ///
    /// Kills the mutants that relax `token > 0.0` to `>=` -- which divides by
    /// zero and yields an infinity that serialises as JSON `null`, looking
    /// exactly like an honest absent price -- and the one that turns the
    /// division into a multiplication.
    #[test]
    fn a_priced_pair_divides_quote_by_token_and_refuses_zero_tokens() {
        let p = priced("2000000", "6", "4000000000", "9").expect("both legs parse");
        assert!(
            (p - 2.0).abs() < 1e-12,
            "4 quote over 2 token is 2, not 8: {p}"
        );
        assert_eq!(
            priced("0", "6", "4000000000", "9"),
            None,
            "zero tokens has no price, not an infinite one"
        );
    }

    /// A change is measured against the earlier price, and a zero start has
    /// none.
    ///
    /// Kills the mutants that relax `first > 0.0` to `>=` or to always-true --
    /// both of which divide by zero -- and the one that turns the division
    /// into a remainder.
    #[test]
    fn a_change_is_relative_to_the_first_price_and_a_zero_first_has_none() {
        let doubled = change_from(Some(2.0), Some(3.0));
        assert!(
            doubled.is_some_and(|c| (c - 50.0).abs() < 1e-12),
            "2 to 3 is +50 per cent, not +1 and not a remainder: {doubled:?}"
        );
        assert_eq!(
            change_from(Some(0.0), Some(3.0)),
            None,
            "a change from zero is undefined, never infinite"
        );
        assert_eq!(change_from(None, Some(3.0)), None);
        assert_eq!(change_from(Some(2.0), None), None);
    }

    /// A bucket starts on an interval boundary, whatever second the trade
    /// landed on.
    ///
    /// `epoch.div_euclid(interval) * interval` floors to the boundary. Kills
    /// the mutant turning that multiplication into an addition, which would
    /// put the bucket at `epoch / interval + interval` -- a number near the
    /// epoch rather than near the trade, so every bar on the chart lands in
    /// 1970 and the axis spans fifty years.
    #[test]
    fn a_bucket_starts_on_an_interval_boundary_not_near_the_epoch() {
        let trades = vec![
            trade_from_row(
                &row("a", "2026-09-11 12:34:56", WSOL, "10000"),
                &HashMap::new(),
            )
            .expect("converts"),
        ];
        let candles = fold_candles(&trades, 300);
        assert_eq!(candles.len(), 1);
        let start = candles[0].time;
        assert_eq!(start % 300, 0, "aligned to the five-minute boundary");
        let at = ts_to_epoch("2026-09-11 12:34:56").expect("a stamp");
        assert!(
            start <= at && at - start < 300,
            "the bucket contains its own trade: bucket {start}, trade {at}"
        );
        assert!(
            start > 1_700_000_000,
            "a bucket near the epoch means the arithmetic multiplied nothing"
        );
    }

    /// A burn whose source is blank debits nobody, and one with a source
    /// debits it.
    ///
    /// Kills the `delete !` and `with true` mutants on the burn guard: both
    /// send a blank-source burn into the burn arm, which debits the empty
    /// string and builds a negative phantom holder.
    #[test]
    fn a_burn_debits_its_source_and_never_a_blank_one() {
        let from = "SRC1111111111111111111111111111111111111111";
        let rows = vec![
            holder_row("", from, "5000000", "MintTo"),
            holder_row(from, "", "2000000", "Burn"),
        ];
        let folded = fold_holders(&rows, "2026-09-11 00:00:00", "2026-09-11 01:00:00", 10).holders;
        assert_eq!(folded.len(), 1, "one real account: {folded:?}");
        assert_eq!(folded[0].account, from);
        assert!(
            (folded[0].balance - 3.0).abs() < 1e-12,
            "5 minted less 2 burned is 3: {folded:?}"
        );

        let blank = vec![holder_row("", "", "2000000", "Burn")];
        assert!(
            fold_holders(&blank, "2026-09-11 00:00:00", "2026-09-11 01:00:00", 10)
                .holders
                .is_empty(),
            "a burn with no source debits nobody"
        );
    }

    /// The busier end of a trade is the pool, so a window of real trades
    /// resolves to buys and sells rather than to unknowns.
    ///
    /// **This is the fix for a tape where every row read `unknown`.** The
    /// previous rule looked for one account on at least half of *all* legs in
    /// the window. A coin trading on several venues has no such account —
    /// measured live on 2026-09-12, the busiest account on WEN appeared on
    /// twelve legs of about fifty, so nothing cleared the bar and every trade
    /// in a working tape was reported directionless.
    ///
    /// Comparing the two ends of each trade needs no single pool: a vault
    /// appears on many trades and a trader on one, whichever venue carried it.
    #[test]
    fn a_window_of_real_trades_resolves_to_buys_and_sells() {
        let pool = "VAULT11111111111111111111111111111111111111";
        let mut rows = Vec::new();
        // Three buys: the vault pays the mint out to three different wallets.
        for i in 0..3 {
            let mut r = row("b", "2026-09-11 00:00:01", WSOL, "10000");
            r.token_source = pool.to_owned();
            r.token_destination = format!("BUYER{i:037}");
            rows.push(r);
        }
        // Two sells: two wallets send the mint in.
        for i in 0..2 {
            let mut r = row("s", "2026-09-11 00:00:02", WSOL, "10000");
            r.token_source = format!("SELLER{i:036}");
            r.token_destination = pool.to_owned();
            rows.push(r);
        }

        let trades = fold_tape(&rows);
        assert_eq!(trades.len(), 5);
        let buys = trades.iter().filter(|t| t.side == Side::Buy).count();
        let sells = trades.iter().filter(|t| t.side == Side::Sell).count();
        assert_eq!(buys, 3, "the vault paying out is a buy: {trades:?}");
        assert_eq!(sells, 2, "the vault taking in is a sell: {trades:?}");
        assert!(
            trades.iter().all(|t| t.side != Side::Unknown),
            "no trade in a window with an obvious vault is directionless"
        );
        assert!(
            trades.iter().all(|t| t.trader.is_some()),
            "each resolved side names the wallet on the other end"
        );
    }

    /// Two ends seen equally often name no direction.
    ///
    /// A single trade between two wallets nobody has seen before is exactly
    /// this: one appearance each, nothing to separate a vault from a trader,
    /// and `Unknown` is the honest answer rather than a coin flip.
    #[test]
    fn two_equally_seen_ends_name_no_direction() {
        let mut r = row("a", "2026-09-11 00:00:01", WSOL, "10000");
        r.token_source = "AAAA1111111111111111111111111111111111111111".to_owned();
        r.token_destination = "BBBB1111111111111111111111111111111111111111".to_owned();
        let trades = fold_tape(&[r]);
        assert_eq!(trades.len(), 1);
        assert_eq!(trades[0].side, Side::Unknown);
        assert_eq!(trades[0].trader, None);
    }

    /// A leg that starts and ends at the pool is not a trade in either
    /// direction.
    ///
    /// Kills the four mutants that loosen `side_and_trader`'s guards from
    /// `&&` to `||`: each would let one half of the condition alone decide a
    /// side, and a row whose source *and* destination are the pool satisfies
    /// the source half of a buy and the destination half of a sell at once.
    #[test]
    fn a_leg_from_the_pool_to_itself_has_no_side() {
        let pool = "POOL111111111111111111111111111111111111";
        let mut r = row("a", "2026-09-11 00:00:01", WSOL, "10000");
        r.token_source = pool.to_owned();
        r.token_destination = pool.to_owned();
        let (side, trader) = side_and_trader(&r, &HashMap::from([(pool, 9u64)]));
        assert_eq!(side, Side::Unknown, "pool to pool is not a buy or a sell");
        assert_eq!(trader, None);
    }

    /// A trade that moved no tokens has no price, rather than an infinite one.
    ///
    /// Kills the mutants that relax `token > 0.0` to `>=` or to always-true,
    /// and the ones that turn the division into a remainder or a product.
    /// Dividing a real quote amount by zero tokens yields `f64::INFINITY`,
    /// which serialises as JSON `null` and would arrive on the tape looking
    /// exactly like an honest absent price -- while every other consumer of
    /// `price`, including the candle fold, would have taken it as a number.
    #[test]
    fn a_trade_of_zero_tokens_has_no_price_rather_than_an_infinite_one() {
        let mut r = row("a", "2026-09-11 00:00:01", WSOL, "10000");
        r.token_value = "0".to_owned();
        let t = trade_from_row(&r, &HashMap::new()).expect("a zero-token transfer is still a row");
        assert!((t.token_amount - 0.0).abs() < f64::EPSILON);
        assert_eq!(t.price, None, "no price, not an infinity");

        // And the ordinary case still divides: 0.00001 wSOL for 0.056626 of
        // the mint.
        let ok = trade_from_row(
            &row("b", "2026-09-11 00:00:02", WSOL, "10000"),
            &HashMap::new(),
        )
        .expect("converts");
        let price = ok.price.expect("both legs present");
        assert!(
            (price - (0.000_01 / 0.056_626)).abs() < 1e-12,
            "price is quote over token: {price}"
        );
    }

    /// Two trades inside one interval sum into one bar.
    ///
    /// Kills the mutants that turn the bucket's volume accumulation into a
    /// product or a subtraction, and the one that replaces the bucket-start
    /// multiplication with an addition -- the last of which would scatter
    /// trades across buckets that are not interval-aligned.
    #[test]
    fn two_trades_in_one_interval_make_one_bar_that_sums_them() {
        let trades = vec![
            trade_from_row(
                &row("a", "2026-09-11 00:00:05", WSOL, "10000"),
                &HashMap::new(),
            )
            .expect("a"),
            trade_from_row(
                &row("b", "2026-09-11 00:00:45", WSOL, "20000"),
                &HashMap::new(),
            )
            .expect("b"),
        ];
        let candles = fold_candles(&trades, 60);
        assert_eq!(candles.len(), 1, "both fall in the same minute");
        let bar = &candles[0];
        assert_eq!(bar.trade_count, 2);
        assert_eq!(
            bar.time % 60,
            0,
            "a bucket starts on an interval boundary, not at the first trade"
        );
        let expected_quote = 0.000_01 + 0.000_02;
        assert!(
            (bar.quote_volume - expected_quote).abs() < 1e-12,
            "volumes add: {} vs {expected_quote}",
            bar.quote_volume
        );
        let expected_token = 0.056_626 * 2.0;
        assert!(
            (bar.token_volume - expected_token).abs() < 1e-12,
            "token volumes add too: {}",
            bar.token_volume
        );
    }

    #[test]
    fn decimals_unadjusted_amounts_cannot_reach_a_trade() {
        // The bug the sketch shipped: comparing the two directly is the wrong
        // number by six orders of magnitude for this mint.
        assert_eq!(adjust("56626", "6"), Some(0.056_626));
        assert_eq!(adjust("10000", "9"), Some(0.000_01));
        // `Trade` has no raw field at all -- `trade_from_row` is the only
        // producer, and every numeric field on it comes from `adjust`.
        let t = trade_from_row(
            &row(
                "sig-1",
                "2026-09-11 17:35:00.000000",
                "So11111111111111111111111111111111111111112",
                "10000",
            ),
            &HashMap::new(),
        )
        .expect("converts");
        // Compared within a tolerance rather than exactly: these are `f64`
        // display values, and an exact `==` on a float asserts the bit
        // pattern of an arithmetic result rather than the number meant.
        assert!(
            (t.token_amount - 0.056_626).abs() < 1e-12,
            "{}",
            t.token_amount
        );
        let quote = t.quote_amount.expect("the quote leg is present here");
        assert!((quote - 0.000_01).abs() < 1e-12, "{quote}");
    }

    #[test]
    fn a_trade_with_no_quote_leg_is_reported_with_no_price_never_a_zero_one() {
        // Re-applying the sketch's bug: reading `quote_value` ("0" here, what
        // a LEFT JOIN miss renders as) as though it were a real amount would
        // make this a trade priced at zero rather than a trade this data
        // cannot price.
        let r = row("sig-2", "2026-09-11 17:35:01.000000", "", "0");
        let t = trade_from_row(&r, &HashMap::new()).expect("the mint leg alone still converts");
        assert_eq!(t.quote_amount, None, "no quote leg was found");
        assert_eq!(t.quote_mint, None);
        assert_eq!(t.price, None, "must never be Some(0.0)");
        assert_ne!(t.price, Some(0.0));
    }

    #[test]
    fn fold_tape_keeps_the_quoteless_trade_rather_than_dropping_it() {
        // The other half of the same bug, at the level the sketch's INNER
        // JOIN actually broke: the row must still appear on the tape.
        let rows = vec![
            row(
                "sig-1",
                "2026-09-11 17:35:02.000000",
                "So11111111111111111111111111111111111111112",
                "10000",
            ),
            row("sig-2", "2026-09-11 17:35:01.000000", "", "0"),
        ];
        let trades = fold_tape(&rows);
        assert_eq!(trades.len(), 2, "the quoteless trade must not be dropped");
        assert!(
            trades
                .iter()
                .any(|t| t.signature == "sig-2" && t.price.is_none())
        );
    }

    #[test]
    fn newest_first() {
        let rows = vec![
            row(
                "older",
                "2026-09-11 17:00:00.000000",
                "So11111111111111111111111111111111111111112",
                "10000",
            ),
            row(
                "newer",
                "2026-09-11 17:05:00.000000",
                "So11111111111111111111111111111111111111112",
                "10000",
            ),
        ];
        let trades = fold_tape(&rows);
        assert_eq!(trades[0].signature, "newer");
        assert_eq!(trades[1].signature, "older");
    }

    /// Several trades against the same pool, each a different trader, so the
    /// pool is an unambiguous plurality rather than tied with any one of
    /// them -- a single trade cannot tell a pool apart from a trader by
    /// frequency at all, which is a real limit of the heuristic, not a
    /// property to pin in a test.
    fn rows_against_one_pool(quote_mint: &str) -> Vec<TapeRow> {
        (0..3)
            .map(|i| {
                let mut r = row(
                    &format!("sig-{i}"),
                    "2026-09-11 17:35:00.000000",
                    quote_mint,
                    "10000",
                );
                r.token_destination = format!("TRADER-{i}");
                r.quote_authority = format!("TRADERWALLET-{i}");
                r
            })
            .collect()
    }

    #[test]
    fn a_pool_moving_the_mint_out_is_a_buy_and_the_trader_is_the_quote_authority() {
        let rows = rows_against_one_pool("So11111111111111111111111111111111111111112");
        let trades = fold_tape(&rows);
        assert_eq!(trades.len(), 3);
        for t in &trades {
            assert_eq!(t.side, Side::Buy);
        }
        assert!(
            trades
                .iter()
                .any(|t| t.trader.as_deref() == Some("TRADERWALLET-0"))
        );
    }

    #[test]
    fn a_pool_receiving_the_mint_is_a_sell_and_the_trader_is_the_mint_authority() {
        let mut rows = rows_against_one_pool("So11111111111111111111111111111111111111112");
        for r in &mut rows {
            std::mem::swap(&mut r.token_source, &mut r.token_destination);
        }
        let trades = fold_tape(&rows);
        assert_eq!(trades.len(), 3);
        for t in &trades {
            assert_eq!(t.side, Side::Sell);
            assert_eq!(
                t.trader.as_deref(),
                Some("POOLAUTH1111111111111111111111111111111111")
            );
        }
    }

    #[test]
    fn with_no_dominant_account_the_side_is_unknown_not_guessed() {
        // Every leg touches a different pair of accounts, so nothing clears
        // the pool-share threshold.
        let rows = vec![row(
            "sig-1",
            "2026-09-11 17:00:00.000000",
            "So11111111111111111111111111111111111111112",
            "10000",
        )];
        let mut r2 = row(
            "sig-2",
            "2026-09-11 17:01:00.000000",
            "So11111111111111111111111111111111111111112",
            "5000",
        );
        r2.token_source = "OTHER_A".to_owned();
        r2.token_destination = "OTHER_B".to_owned();
        let all = [rows, vec![r2]].concat();
        let trades = fold_tape(&all);
        // Neither account reaches a 50% share of the four total legs.
        assert!(trades.iter().all(|t| t.side == Side::Unknown));
        assert!(trades.iter().all(|t| t.trader.is_none()));
    }

    #[test]
    fn candles_never_include_an_unpriced_trade() {
        let rows = vec![
            row(
                "priced",
                "2026-09-11 17:00:10.000000",
                "So11111111111111111111111111111111111111112",
                "10000",
            ),
            row("unpriced", "2026-09-11 17:00:20.000000", "", "0"),
        ];
        let trades = fold_tape(&rows);
        let candles = fold_candles(&trades, 60);
        assert_eq!(candles.len(), 1, "one priced trade makes one bucket");
        let only = &candles[0];
        assert_eq!(only.trade_count, 1);
        assert!(
            (only.open - only.close).abs() < f64::EPSILON,
            "the unpriced trade did not touch this bucket: open {} close {}",
            only.open,
            only.close
        );
    }

    #[test]
    fn candles_bucket_by_interval_and_track_high_low() {
        let mut a = row(
            "a",
            "2026-09-11 17:00:05.000000",
            "So11111111111111111111111111111111111111112",
            "10000",
        );
        let mut b = row(
            "b",
            "2026-09-11 17:00:50.000000",
            "So11111111111111111111111111111111111111112",
            "20000",
        );
        let mut c = row(
            "c",
            "2026-09-11 17:01:10.000000",
            "So11111111111111111111111111111111111111112",
            "5000",
        );
        for r in [&mut a, &mut b, &mut c] {
            r.token_value = "56626".to_owned();
            r.token_decimals = "6".to_owned();
        }
        let trades = fold_tape(&[a, b, c]);
        let candles = fold_candles(&trades, 60);
        assert_eq!(
            candles.len(),
            2,
            "a and b share the first minute, c is the next"
        );
        assert_eq!(candles[0].trade_count, 2);
        assert!(
            candles[0].high > candles[0].low,
            "a and b priced differently"
        );
        assert_eq!(candles[1].trade_count, 1);
    }

    #[test]
    fn a_zero_interval_produces_no_candles_rather_than_dividing_by_zero() {
        let rows = vec![row(
            "a",
            "2026-09-11 17:00:05.000000",
            "So11111111111111111111111111111111111111112",
            "10000",
        )];
        let trades = fold_tape(&rows);
        assert!(fold_candles(&trades, 0).is_empty());
    }

    fn holder_row(source: &str, destination: &str, value: &str, kind: &str) -> HolderRow {
        HolderRow {
            source: source.to_owned(),
            destination: destination.to_owned(),
            value: value.to_owned(),
            decimals: "6".to_owned(),
            transfer_type: kind.to_owned(),
        }
    }

    #[test]
    fn a_mint_credits_the_destination_and_a_burn_debits_the_source() {
        let rows = vec![
            holder_row("", "vault-a", "1000000", "MintTo"),
            holder_row("vault-a", "wallet-b", "400000", "Transfer"),
            holder_row("wallet-b", "", "100000", "Burn"),
        ];
        let fold = fold_holders(&rows, "2026-09-11 00:00:00", "2026-09-11 01:00:00", 10);
        assert_eq!(fold.fact, "folded_transfers");
        assert_eq!(fold.granularity, "token_account");
        let by_account: HashMap<&str, f64> = fold
            .holders
            .iter()
            .map(|h| (h.account.as_str(), h.balance))
            .collect();
        assert_eq!(by_account.get("vault-a"), Some(&0.6));
        assert_eq!(by_account.get("wallet-b"), Some(&0.3));
        // Supply is conserved: 1.0 minted, 0.1 burned, 0.9 left across accounts.
        let total: f64 = fold.holders.iter().map(|h| h.balance).sum();
        assert!((total - 0.9).abs() < 1e-9, "{total}");
    }

    #[test]
    fn a_holder_whose_balance_folds_to_zero_or_below_is_not_reported() {
        let rows = vec![
            holder_row("", "vault-a", "1000000", "MintTo"),
            holder_row("vault-a", "wallet-b", "1000000", "Transfer"),
        ];
        let fold = fold_holders(&rows, "2026-09-11 00:00:00", "2026-09-11 01:00:00", 10);
        assert_eq!(fold.holders.len(), 1, "vault-a is fully drained");
        assert_eq!(fold.holders[0].account, "wallet-b");
    }

    #[test]
    fn holders_are_limited_richest_first() {
        let rows = vec![
            holder_row("", "a", "3000000", "MintTo"),
            holder_row("", "b", "1000000", "MintTo"),
            holder_row("", "c", "2000000", "MintTo"),
        ];
        let fold = fold_holders(&rows, "2026-09-11 00:00:00", "2026-09-11 01:00:00", 2);
        assert_eq!(fold.holders.len(), 2);
        assert_eq!(fold.holders[0].account, "a");
        assert_eq!(fold.holders[1].account, "c");
    }

    fn candidate(mint: &str, tx_count: &str) -> CoinCandidateRow {
        CoinCandidateRow {
            mint: mint.to_owned(),
            tx_count: tx_count.to_owned(),
            token_volume: "1000000".to_owned(),
            token_decimals: "6".to_owned(),
        }
    }

    fn price_row(mint: &str, first_quote: &str, last_quote: &str) -> CoinPriceRow {
        CoinPriceRow {
            mint: mint.to_owned(),
            trade_count: "2".to_owned(),
            first_token_value: "1000000".to_owned(),
            first_token_decimals: "6".to_owned(),
            first_quote_value: first_quote.to_owned(),
            first_quote_decimals: "9".to_owned(),
            last_token_value: "1000000".to_owned(),
            last_token_decimals: "6".to_owned(),
            last_quote_value: last_quote.to_owned(),
            last_quote_decimals: "9".to_owned(),
            quote_mint: "So11111111111111111111111111111111111111112".to_owned(),
            quote_volume: "2000000000".to_owned(),
            quote_volume_decimals: "9".to_owned(),
        }
    }

    #[test]
    fn a_coin_never_priced_in_the_window_reports_no_price_not_a_dropped_row() {
        let candidates = vec![candidate("MINT_A", "40")];
        let coins = fold_coins(&candidates, &[]);
        assert_eq!(coins.len(), 1, "the mint is still on the list");
        assert_eq!(coins[0].price, None);
        assert_eq!(coins[0].quote_mint, None);
        assert_eq!(coins[0].change_pct, None);
    }

    #[test]
    fn a_priced_coin_reports_change_from_first_to_last_fill() {
        let candidates = vec![candidate("MINT_A", "40")];
        let prices = vec![price_row("MINT_A", "1000000000", "1500000000")];
        let coins = fold_coins(&candidates, &prices);
        assert_eq!(coins[0].price, Some(1.5));
        assert!((coins[0].change_pct.unwrap() - 50.0).abs() < 1e-9);
        assert_eq!(
            coins[0].quote_mint.as_deref(),
            Some("So11111111111111111111111111111111111111112")
        );
    }
}
