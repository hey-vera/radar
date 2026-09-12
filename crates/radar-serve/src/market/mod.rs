// SPDX-License-Identifier: Apache-2.0
//! Public market data: the trade tape, candles, the live coin list, a coin's
//! header, and folded holders — tier 1 of
//! [plan 0012](../../../../docs/plans/0012-the-public-trading-panel.md).
//!
//! Every route here is `Audience::Public` in [`crate::access`] and takes no
//! identity. Market facts belong to nobody: they carry no `Tenant`, read no
//! customer store, and must never gain either.
//!
//! # Where the numbers come from now, and why that changed
//!
//! Every route here used to query CryptoHouse live, once or more per HTTP
//! request. CryptoHouse permits 120 queries an hour per IP, shared with
//! `radar-follow` and the hourly `--outcomes` cron — measured at 291 queries
//! in one hour from a handful of terminal loads on 2026-09-11, and every
//! request after that failed with `QUOTA_EXCEEDED` until the hour rolled
//! over. No cache TTL fixes this: the coin list alone cost two queries, so a
//! one-minute refresh consumed the whole hourly budget by itself.
//!
//! So the direction is flipped. [`radar_backfill::market_tape`] is a
//! collector that spends the budget on a fixed schedule (60 queries an hour,
//! see its own doc comment for the arithmetic) and writes what it finds to
//! [`radar_store::Table::MarketTrades`]. **Every route in this module reads
//! only [`crate::AppState::store`] and issues zero CryptoHouse queries on any
//! request path.** [`radar_backfill::market::fold`] — `detect_pool`,
//! `side_and_trader`, `fold_candles`, `fold_holders`, `adjust` — is the same
//! pure code the collector runs; only where its input comes from changed.
//!
//! # Honest degradation
//!
//! A caller asking about a window the collector has not reached yet is told
//! so plainly, distinguishably from a window the collector reached and found
//! quiet — see [`Degradation::NotCollected`], gated by
//! [`market_tape_collected`] against [`radar_store::Table::Coverage`]. A
//! store that cannot be read is a separate fact again, about this build or
//! this disk, never about a coin.
//!
//! Holder balances and token metadata are not collected by
//! [`radar_backfill::market_tape`] at all today — it collects the trade tape
//! only — so [`holders`] and the metadata half of [`token`] say that plainly
//! rather than returning an empty answer that looks like a quiet market.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use radar_asof::AsOf;
use radar_backfill::market::fold as market_fold;
// `now_epoch` is deliberately absent. **No handler in this module may end a
// window at wall-clock time**: the collector runs behind by design, so `now()`
// names a span it has not reached, and every such request answered with an
// empty list marked complete. Windows end at `newest_collected` instead, and
// the missing import is what stops that regressing quietly.
use radar_store::{MarketSide, MarketTrade, Reader, StoreError, Table, from_epoch};
use radar_types::Address;
use serde::Deserialize;
use serde_json::json;

/// A single top-level window's width, when a caller does not name one — the
/// span of trades folded into the tape by default.
const DEFAULT_WINDOW_SECONDS: i64 = 120;

/// How far back a chart reaches when the caller names no range.
///
/// Wider than the tape's window because the two answer different questions: a
/// tape shows what just happened, a chart shows a shape, and a shape needs
/// more than two minutes of it. Bounded rather than unbounded because the
/// response states the range it actually covered, and a caller asking for a
/// day of a coin collected for ten minutes should be told that plainly rather
/// than handed a day-shaped axis with ten minutes drawn on it.
const DEFAULT_CANDLE_WINDOW_SECONDS: i64 = 60 * 60;

/// The window `/v1/market/coins` ranks activity over.
const COINS_WINDOW_SECONDS: i64 = 10 * 60;

/// A chart reaches further back than a tape: a shape needs more than two
/// minutes of itself. Held at compile time so the two windows cannot be
/// reordered by an edit to either.
const _: () = assert!(DEFAULT_CANDLE_WINDOW_SECONDS > DEFAULT_WINDOW_SECONDS);

/// Said when the store holds no market trades at all.
///
/// Distinct from a collected-and-quiet window: this instance has never had the
/// collector run against it, so there is nothing to be quiet about. A caller
/// reading it knows to check the collector rather than the market.
const NOTHING_COLLECTED: &str =
    "this instance has collected no market trades yet; run radar-backfill --market-tape";

/// The public market-data seam.
///
/// Empty today: every route reads [`crate::AppState::store`] directly, and
/// there is nothing left here to hold. Kept as a named type rather than
/// removed outright because [`crate::AppState::market`] is constructed in
/// several integration tests and in `main.rs`, and a zero-sized marker costs
/// nothing to keep those call sites unchanged. The CryptoHouse client and the
/// per-key `TtlCache` this used to hold are gone with the live queries they
/// existed to pace and de-duplicate — a Parquet read against a handful of
/// small local files does not need either.
#[derive(Default)]
pub struct Market;

impl Market {
    /// A fresh market seam. Takes nothing: see the type's own doc comment.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

/// Why a market route could not answer, distinguishably from an empty
/// result.
///
/// **This, not an empty list, is what "the data is not here" looks like.**
/// Rule 9: a missing measurement must never read like a quiet one.
#[derive(Debug)]
enum Degradation {
    /// The store could not be read. A fact about this build or this disk,
    /// never about a coin — the store-backed analogue of what used to be a
    /// CryptoHouse transport or decoding failure.
    StoreUnreadable(String),
    /// The market-tape collector has not covered the range this answer would
    /// need, or does not collect this kind of fact at all. The `&'static
    /// str` says which, because a caller who cannot tell "not yet" from
    /// "never" cannot decide whether to retry.
    NotCollected(&'static str),
}

impl Degradation {
    fn from_store_error(e: &StoreError) -> Self {
        Self::StoreUnreadable(e.to_string())
    }

    const fn status(&self) -> StatusCode {
        match self {
            Self::StoreUnreadable(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::NotCollected(_) => StatusCode::SERVICE_UNAVAILABLE,
        }
    }

    const fn code(&self) -> &'static str {
        match self {
            Self::StoreUnreadable(_) => "store_unreadable",
            Self::NotCollected(_) => "not_collected",
        }
    }

    fn message(&self) -> String {
        match self {
            Self::StoreUnreadable(detail) => format!(
                "the store could not be read; this is a fact about this build or this disk, not about the coin: {detail}"
            ),
            Self::NotCollected(reason) => format!(
                "not collected -- distinguishable from a collected-and-quiet range: {reason}"
            ),
        }
    }
}

impl IntoResponse for Degradation {
    fn into_response(self) -> Response {
        let status = self.status();
        let code = self.code();
        let message = self.message();
        (status, Json(json!({ "error": code, "message": message }))).into_response()
    }
}

fn bad_request(message: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": "bad_request", "message": message })),
    )
        .into_response()
}

fn parse_mint(raw: &str) -> Result<Address, Box<Response>> {
    raw.parse::<Address>()
        .map_err(|_| Box::new(bad_request("mint must be a base58-encoded Solana address")))
}

/// A `YYYY-MM-DD HH:MM:SS` timestamp, as every query parameter accepting one
/// expects. Fractional seconds are not accepted here — a caller building a
/// cursor from a response's own `ts` field must trim them, since this
/// module's output already carries CryptoHouse's microsecond precision.
fn parse_stamp(raw: &str) -> Result<i64, Box<Response>> {
    radar_store::to_epoch(raw)
        .map_err(|_| Box::new(bad_request("expected a timestamp as 'YYYY-MM-DD HH:MM:SS'")))
}

/// Whether the market-tape collector has ever produced a
/// [`Table::MarketTrades`] coverage record as of `as_of`.
///
/// **This is coarser than "was this exact window collected".** The collector
/// runs a live tape, not a historical backfill, so a fresh deployment has no
/// coverage at all and an established one is asked about recent time almost
/// always inside what it has already reached. A caller naming a window far
/// in the past, before collection began, is not separately detected here —
/// that would need converting the caller's time-native request into the
/// slot-native terms [`radar_store::coverage`] deliberately refuses to
/// invent a conversion for. What this *does* catch, correctly, is the case
/// that actually happens: a store nobody has run the collector against yet.
fn market_tape_collected(store: &Reader, as_of: AsOf) -> Result<bool, StoreError> {
    Ok(store
        .read_coverage(as_of)?
        .iter()
        .any(|c| c.table == Table::MarketTrades))
}

/// Converts a stored, folded row into the shape
/// [`market_fold::fold_candles`] and the tape response already expect.
///
/// The store holds exactly [`market_fold::Trade`] plus the mint it belongs
/// to (`radar_store::MarketTrade`'s own doc comment), so this is a type
/// conversion, not a re-fold: every value on the right already came from
/// [`market_fold::fold_tape`] when the collector wrote it.
fn to_fold_trade(row: &MarketTrade) -> market_fold::Trade {
    market_fold::Trade {
        ts: row.ts.clone(),
        slot: row.slot.get(),
        signature: row.signature.to_string(),
        side: match row.side {
            MarketSide::Buy => market_fold::Side::Buy,
            MarketSide::Sell => market_fold::Side::Sell,
            MarketSide::Unknown => market_fold::Side::Unknown,
        },
        token_amount: row.token_amount,
        quote_amount: row.quote_amount,
        quote_mint: row.quote_mint.map(|m| m.to_string()),
        price: row.price,
        trader: row.trader.map(|a| a.to_string()),
    }
}

/// Every stored trade for one mint, newest first, from the store's rows in
/// `(as_of, mint, window)`.
///
/// # Errors
///
/// Returns [`StoreError`] if the store cannot be read.
/// The newest moment the store actually holds market trades for.
///
/// **The default window ends here, not at `now()`, and that is not a
/// refinement — it is the difference between a working tape and an empty
/// one.** The collector runs behind wall-clock by design: it lags
/// `market_tape::PASS_INTERVAL` seconds so the window it asks for has landed
/// in CryptoHouse, and a pass takes time on top of that. A handler that
/// defaulted its window to the last two minutes of wall-clock time therefore
/// asked for a span the collector had not reached yet, and answered every
/// visitor with an empty list and `"complete": true` — a confident claim that
/// nothing traded, when the truth was that nothing had been collected yet.
/// Observed 2026-09-12 against a store the collector had just filled.
///
/// `None` when the store holds no market trades at all, which is a different
/// answer again: not "the market is quiet", but "this instance has collected
/// nothing", and the caller says so.
/// Where a window of `span` seconds ending at `to` begins.
///
/// A window reaches **back** from its end. Written inline as `to - span` in
/// three handlers, where an addition reaches forward instead -- asking for a
/// span the collector has not reached and answering every caller with an empty
/// list. One name, one subtraction, and a test that holds it.
const fn reaching_back(to: i64, span_seconds: i64) -> i64 {
    to - span_seconds
}

fn newest_collected(store: &Reader, as_of: AsOf) -> Result<Option<i64>, StoreError> {
    let newest = store
        .read_market_trades(as_of)?
        .iter()
        .map(|t| t.ts.clone())
        .max();
    // Stored timestamps carry CryptoHouse's microseconds
    // (`2026-09-12 16:31:52.000000`) and `to_epoch` takes whole seconds, so
    // the fraction is trimmed rather than parsed. Truncating toward the
    // second is the safe direction: the window ends no later than the newest
    // row, so it can never claim to cover a moment nothing was read at.
    let trimmed = newest.map(|ts| ts.split('.').next().unwrap_or(&ts).to_owned());
    Ok(trimmed.and_then(|ts| radar_store::to_epoch(&ts).ok()))
}

fn tape_for(
    store: &Reader,
    as_of: AsOf,
    mint: Address,
    from_s: &str,
    to_s: &str,
) -> Result<Vec<market_fold::Trade>, StoreError> {
    let mut trades: Vec<market_fold::Trade> = store
        .read_market_trades(as_of)?
        .iter()
        .filter(|t| t.mint == mint && t.ts.as_str() >= from_s && t.ts.as_str() < to_s)
        .map(to_fold_trade)
        .collect();
    // The same order `fold_tape` produced when the collector wrote these
    // rows -- newest first, ties broken by signature so the order does not
    // depend on how the rows happened to be laid out across files.
    trades.sort_by(|a, b| b.ts.cmp(&a.ts).then_with(|| b.signature.cmp(&a.signature)));
    Ok(trades)
}

/// `/v1/market/trades/{mint}` query parameters.
#[derive(Debug, Deserialize)]
pub struct TapeParams {
    limit: Option<usize>,
    before: Option<String>,
}

const DEFAULT_TRADE_LIMIT: usize = 100;
const MAX_TRADE_LIMIT: usize = 500;

/// The tape: recent trades for one mint, newest first.
pub async fn trades(
    State(state): State<Arc<crate::AppState>>,
    Path(mint): Path<String>,
    Query(params): Query<TapeParams>,
) -> Response {
    let mint = match parse_mint(&mint) {
        Ok(m) => m,
        Err(r) => return *r,
    };
    let watermark = match crate::watermark_of(&state) {
        Ok(w) => w,
        Err(e) => return e.into_response(),
    };
    let as_of = AsOf::at(watermark);
    match market_tape_collected(&state.store, as_of) {
        Ok(true) => {}
        Ok(false) => {
            return Degradation::NotCollected(
                "the market-tape collector has not produced anything for this store yet",
            )
            .into_response();
        }
        Err(e) => return Degradation::from_store_error(&e).into_response(),
    }

    let to = match params.before.as_deref().map(parse_stamp).transpose() {
        Ok(Some(before)) => before,
        // No cursor: end the window at what the store actually holds, never at
        // wall-clock time. See `newest_collected` -- the collector runs behind
        // by design, so `now()` names a span it has not reached and the answer
        // is an empty tape presented as complete.
        Ok(None) => match newest_collected(&state.store, as_of) {
            Ok(Some(newest)) => newest,
            Ok(None) => return Degradation::NotCollected(NOTHING_COLLECTED).into_response(),
            Err(e) => return Degradation::from_store_error(&e).into_response(),
        },
        Err(r) => return *r,
    };
    let limit = params
        .limit
        .unwrap_or(DEFAULT_TRADE_LIMIT)
        .clamp(1, MAX_TRADE_LIMIT);
    let from = reaching_back(to, DEFAULT_WINDOW_SECONDS);
    let (from_s, to_s) = (from_epoch(from), from_epoch(to));

    match tape_for(&state.store, as_of, mint, &from_s, &to_s) {
        Ok(mut trades) => {
            trades.truncate(limit);
            Json(json!({
                "mint": mint.to_string(),
                "window": { "from": from_s, "to": to_s, "complete": true },
                "trades": trades,
            }))
            .into_response()
        }
        Err(e) => Degradation::from_store_error(&e).into_response(),
    }
}

/// `/v1/market/candles/{mint}` query parameters.
#[derive(Debug, Deserialize)]
pub struct CandleParams {
    interval: Option<String>,
    from: Option<String>,
    to: Option<String>,
}

/// The widest range a single candles request will cover, however far apart
/// `from` and `to` are asked to be. The response's `covered` window says so
/// when this clamps.
const MAX_CANDLE_WINDOW_SECONDS: i64 = 24 * 60 * 60;

fn interval_seconds(name: &str) -> Option<i64> {
    match name {
        "1m" => Some(60),
        "5m" => Some(300),
        "15m" => Some(900),
        "1h" => Some(3_600),
        "4h" => Some(14_400),
        "1d" => Some(86_400),
        _ => None,
    }
}

/// OHLCV, folded from the same trades the tape reads.
pub async fn candles(
    State(state): State<Arc<crate::AppState>>,
    Path(mint): Path<String>,
    Query(params): Query<CandleParams>,
) -> Response {
    let mint = match parse_mint(&mint) {
        Ok(m) => m,
        Err(r) => return *r,
    };
    let Some(interval) = interval_seconds(params.interval.as_deref().unwrap_or("1m")) else {
        return bad_request("interval must be one of 1m, 5m, 15m, 1h, 4h, 1d");
    };
    let watermark = match crate::watermark_of(&state) {
        Ok(w) => w,
        Err(e) => return e.into_response(),
    };
    let as_of = AsOf::at(watermark);
    match market_tape_collected(&state.store, as_of) {
        Ok(true) => {}
        Ok(false) => {
            return Degradation::NotCollected(
                "the market-tape collector has not produced anything for this store yet",
            )
            .into_response();
        }
        Err(e) => return Degradation::from_store_error(&e).into_response(),
    }

    let requested_to = match params.to.as_deref().map(parse_stamp).transpose() {
        Ok(Some(to)) => to,
        // The store's own horizon, not wall-clock -- see `newest_collected`.
        // A chart defaulting to the last two minutes of wall-clock time drew
        // nothing at all, because the collector is always behind it.
        Ok(None) => match newest_collected(&state.store, as_of) {
            Ok(Some(newest)) => newest,
            Ok(None) => return Degradation::NotCollected(NOTHING_COLLECTED).into_response(),
            Err(e) => return Degradation::from_store_error(&e).into_response(),
        },
        Err(r) => return *r,
    };
    let requested_from = match params.from.as_deref().map(parse_stamp).transpose() {
        Ok(v) => v.unwrap_or(requested_to - DEFAULT_CANDLE_WINDOW_SECONDS),
        Err(r) => return *r,
    };
    if requested_from >= requested_to {
        return bad_request("from must be before to");
    }
    // The range actually covered may be narrower than requested -- clamped
    // rather than silently served, per the plan's own rubric for this
    // endpoint.
    let from = requested_from.max(requested_to - MAX_CANDLE_WINDOW_SECONDS);
    let to = requested_to;
    let (from_s, to_s) = (from_epoch(from), from_epoch(to));

    match tape_for(&state.store, as_of, mint, &from_s, &to_s) {
        Ok(trades) => {
            let candles = market_fold::fold_candles(&trades, interval);
            Json(json!({
                "mint": mint.to_string(),
                "interval": params.interval.as_deref().unwrap_or("1m"),
                "requested": { "from": from_epoch(requested_from), "to": from_epoch(requested_to) },
                "covered": { "from": from_s, "to": to_s, "complete": true },
                "candles": candles,
            }))
            .into_response()
        }
        Err(e) => Degradation::from_store_error(&e).into_response(),
    }
}

/// `/v1/market/coins` query parameters.
#[derive(Debug, Deserialize)]
pub struct CoinsParams {
    limit: Option<usize>,
    sort: Option<String>,
}

const DEFAULT_COINS_LIMIT: usize = 50;
const MAX_COINS_LIMIT: usize = 200;

fn sort_coins(coins: &mut [market_fold::Coin], sort: &str) {
    match sort {
        "volume" => coins.sort_by(|a, b| {
            b.quote_volume
                .unwrap_or(0.0)
                .total_cmp(&a.quote_volume.unwrap_or(0.0))
        }),
        "change" => coins.sort_by(|a, b| {
            b.change_pct
                .unwrap_or(f64::MIN)
                .total_cmp(&a.change_pct.unwrap_or(f64::MIN))
        }),
        _ => coins.sort_by_key(|c| std::cmp::Reverse(c.tx_count)),
    }
}

/// Folds a window of stored trades, across every mint the collector saw,
/// into one row per mint.
///
/// The store-backed analogue of [`market_fold::fold_coins`]: that function
/// reads a live `CoinCandidateRow`/`CoinPriceRow` pair this module no longer
/// fetches, so this reads the rows the collector already wrote instead. Both
/// exist because they read differently-shaped inputs, not because one is a
/// draft of the other -- `market_fold::fold_coins` stays exactly as tested,
/// for a caller that still has the live rows to hand.
fn coins_from_trades(trades: &[MarketTrade]) -> Vec<market_fold::Coin> {
    let mut by_mint: std::collections::BTreeMap<Address, Vec<&MarketTrade>> =
        std::collections::BTreeMap::new();
    for t in trades {
        by_mint.entry(t.mint).or_default().push(t);
    }

    by_mint
        .into_iter()
        .map(|(mint, mut group)| {
            // Chronological, so "first" and "last" priced fill below mean
            // what they say rather than whatever order the store happened
            // to return.
            group.sort_by(|a, b| {
                a.ts.cmp(&b.ts)
                    .then_with(|| a.signature.as_bytes().cmp(b.signature.as_bytes()))
            });
            let tx_count = u64::try_from(group.len()).unwrap_or(u64::MAX);
            let token_volume: f64 = group.iter().map(|t| t.token_amount).sum();
            let priced: Vec<&&MarketTrade> = group.iter().filter(|t| t.price.is_some()).collect();
            let last_price = priced.last().and_then(|t| t.price);
            let first_price = priced.first().and_then(|t| t.price);
            let quote_mint = priced
                .last()
                .and_then(|t| t.quote_mint)
                .map(|m| m.to_string());
            let quote_volume = (!priced.is_empty())
                .then(|| priced.iter().filter_map(|t| t.quote_amount).sum::<f64>());
            let change_pct = match (first_price, last_price) {
                (Some(first), Some(last)) if first > 0.0 => Some((last - first) / first * 100.0),
                _ => None,
            };
            market_fold::Coin {
                mint: mint.to_string(),
                tx_count,
                token_volume: Some(token_volume),
                quote_mint,
                quote_volume,
                price: last_price,
                change_pct,
            }
        })
        .collect()
}

/// The live coin list: what has moved recently, ranked and priced.
pub async fn coins(
    State(state): State<Arc<crate::AppState>>,
    Query(params): Query<CoinsParams>,
) -> Response {
    let limit = params
        .limit
        .unwrap_or(DEFAULT_COINS_LIMIT)
        .clamp(1, MAX_COINS_LIMIT);
    let sort = params.sort.clone().unwrap_or_else(|| "activity".to_owned());

    let watermark = match crate::watermark_of(&state) {
        Ok(w) => w,
        Err(e) => return e.into_response(),
    };
    let as_of = AsOf::at(watermark);
    match market_tape_collected(&state.store, as_of) {
        Ok(true) => {}
        Ok(false) => {
            return Degradation::NotCollected(
                "the market-tape collector has not produced anything for this store yet",
            )
            .into_response();
        }
        Err(e) => return Degradation::from_store_error(&e).into_response(),
    }

    // The store's own horizon, not wall-clock -- see `newest_collected`.
    let to = match newest_collected(&state.store, as_of) {
        Ok(Some(newest)) => newest,
        Ok(None) => return Degradation::NotCollected(NOTHING_COLLECTED).into_response(),
        Err(e) => return Degradation::from_store_error(&e).into_response(),
    };
    let from = reaching_back(to, COINS_WINDOW_SECONDS);
    let (from_s, to_s) = (from_epoch(from), from_epoch(to));

    let rows = match state.store.read_market_trades(as_of) {
        Ok(rows) => rows,
        Err(e) => return Degradation::from_store_error(&e).into_response(),
    };
    let windowed: Vec<MarketTrade> = rows
        .into_iter()
        .filter(|t| t.ts.as_str() >= from_s.as_str() && t.ts.as_str() < to_s.as_str())
        .collect();

    let mut coins = coins_from_trades(&windowed);
    sort_coins(&mut coins, &sort);
    coins.truncate(limit);
    Json(json!({
        "window": { "from": from_s, "to": to_s },
        "coins": coins,
    }))
    .into_response()
}

/// `/v1/market/token/{mint}`: a coin's header.
///
/// Metadata (`name`, `symbol`, `creator`, `published_at`) is never collected
/// by [`radar_backfill::market_tape`] -- it collects the trade tape only --
/// so those fields are always `null` with `metadata_reason` saying so,
/// rather than a live `solana.tokens` query this build no longer makes. Price
/// still comes from the collected tape, the same as [`trades`].
///
/// # Why collecting it was not simply scheduled
///
/// **`solana.tokens` is abandoned.** Checked 2026-09-12: its newest row is
/// dated **2026-08-10**, over a month earlier, and not one of the ten busiest
/// mints of that moment appeared in it at all. A batched
/// `WHERE mint IN (...)` metadata query would have cost one query a pass and
/// fitted the budget comfortably -- it was measured for exactly that -- and it
/// would have returned nothing for every coin a trading screen is about.
///
/// So a name is not available on the free lane at any price in queries. It
/// needs the Metaplex metadata account read from chain, one RPC call per mint
/// against an endpoint Solana's own documentation says is not for production
/// use, or a paid provider. Until one of those is a decision somebody has
/// made, the list shows the mint and the header says the name is absent --
/// which is true, and is a smaller lie than a column of "unknown" where a name
/// would go.
///
/// Market cap and liquidity are always `null` for the reason they always
/// were: both need either an unbounded transfer scan or a live account-state
/// read, neither of which this module performs.
pub async fn token(
    State(state): State<Arc<crate::AppState>>,
    Path(mint): Path<String>,
) -> Response {
    let mint = match parse_mint(&mint) {
        Ok(m) => m,
        Err(r) => return *r,
    };
    let watermark = match crate::watermark_of(&state) {
        Ok(w) => w,
        Err(e) => return e.into_response(),
    };
    let as_of = AsOf::at(watermark);
    match market_tape_collected(&state.store, as_of) {
        Ok(true) => {}
        Ok(false) => {
            return Degradation::NotCollected(
                "the market-tape collector has not produced anything for this store yet",
            )
            .into_response();
        }
        Err(e) => return Degradation::from_store_error(&e).into_response(),
    }

    // The store's own horizon, not wall-clock -- see `newest_collected`.
    let to = match newest_collected(&state.store, as_of) {
        Ok(Some(newest)) => newest,
        Ok(None) => return Degradation::NotCollected(NOTHING_COLLECTED).into_response(),
        Err(e) => return Degradation::from_store_error(&e).into_response(),
    };
    let from = reaching_back(to, DEFAULT_WINDOW_SECONDS);
    let (from_s, to_s) = (from_epoch(from), from_epoch(to));
    let recent_trades = match tape_for(&state.store, as_of, mint, &from_s, &to_s) {
        Ok(t) => t,
        Err(e) => return Degradation::from_store_error(&e).into_response(),
    };

    let priced = recent_trades.iter().find(|t| t.price.is_some());
    let (price, price_reason) = match priced {
        Some(t) => (t.price, None),
        None if recent_trades.is_empty() => {
            (None, Some("no recorded trades in the last two minutes"))
        }
        None => (
            None,
            Some("recent trades exist but none paired with a quote leg in this window"),
        ),
    };

    Json(json!({
        "mint": mint.to_string(),
        "name": Option::<String>::None,
        "symbol": Option::<String>::None,
        "creator": Option::<String>::None,
        "published_at": Option::<String>::None,
        "metadata_reason": "token metadata is not collected: the market-tape collector gathers trade data only, and the free source's own token table stopped being updated on 2026-08-10, so a name is not available on this lane at all",
        "price": price,
        "price_reason": price_reason,
        "market_cap": Option::<f64>::None,
        "market_cap_reason": "supply is not computable without an unbounded transfer scan or a live account read, neither of which this build performs",
        "liquidity": Option::<f64>::None,
        "liquidity_reason": "pool reserves require a live account read, which this build does not perform",
    }))
    .into_response()
}

/// `/v1/market/holders/{mint}` query parameters.
///
/// Carries nothing today: [`holders`] refuses before it would ever read a
/// parameter. Kept as a named type, rather than dropped from the route's
/// signature, so a future holder-fold collector can add fields here without
/// changing the router registration in [`crate::app`].
#[derive(Debug, Deserialize)]
pub struct HoldersParams {}

/// Holder balances, folded from observed transfers over a bounded window.
///
/// **Not collected by [`radar_backfill::market_tape`] today.** The collector
/// gathers a batched trade tape (`mint IN (...)` against a shortlist of
/// active mints); a holders fold needs every raw transfer for one mint over a
/// wide window, which is a different query this budget does not have room
/// for yet. Refusing plainly here is rule 9's shape: an empty holder list
/// would look exactly like a token with no holders, which is a different and
/// much more interesting fact than the true one.
pub async fn holders(Path(mint): Path<String>, Query(_params): Query<HoldersParams>) -> Response {
    let _mint = match parse_mint(&mint) {
        Ok(m) => m,
        Err(r) => return *r,
    };
    Degradation::NotCollected(
        "holder-balance collection is out of scope for the market-tape collector; only trade data is collected",
    )
    .into_response()
}

/// Whether a path is one of this module's routes — used only by
/// `access::audience_of`'s own tests, so the two cannot silently disagree
/// about which paths exist.
#[must_use]
pub fn is_market_path(path: &str) -> bool {
    path.starts_with("/v1/market/")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a store holding exactly the trades given, and a `Reader` over it.
    ///
    /// **Every handler in this module is now a pure function of the store**,
    /// which is what makes these tests possible at all. While the routes
    /// queried CryptoHouse, the only way to reach their window arithmetic was
    /// a live request against a rate-limited public endpoint, so none of it was
    /// tested and the mutation shards reported every branch of it as surviving.
    /// A fixture store costs a temporary directory.
    fn store_of(trades: &[MarketTrade]) -> (tempfile::TempDir, Reader) {
        let dir = tempfile::tempdir().expect("a temporary directory");
        {
            let mut writer =
                radar_store::Writer::open(dir.path().to_str().expect("a utf-8 path"), 20_000)
                    .expect("a writer");
            for trade in trades {
                writer
                    .append_market_trade(trade.clone())
                    .expect("a market trade appends");
            }
            writer.flush().expect("the writer flushes");
        }
        let reader = Reader::open(dir.path().to_str().expect("a utf-8 path"));
        (dir, reader)
    }

    fn market_trade(mint: &str, ts: &str, slot: u64) -> MarketTrade {
        MarketTrade {
            mint: mint.parse().expect("a mint"),
            ts: ts.to_owned(),
            slot: radar_types::Slot(slot),
            signature: radar_types::Signature::new([7u8; 64]),
            side: MarketSide::Unknown,
            token_amount: 1.0,
            quote_amount: Some(2.0),
            quote_mint: Some(WSOL.parse().expect("a mint")),
            price: Some(2.0),
            trader: None,
        }
    }

    const WSOL: &str = "So11111111111111111111111111111111111111112";
    const A_MINT: &str = "5NfV2sy8DqXamLvYEE4LcTWzGqZc5Emv4bqqhVDWpump";

    /// The default windows are the spans their names claim.
    ///
    /// Both are written as products -- `60 * 60` and `10 * 60` -- and a mutant
    /// turning either into a sum or a quotient leaves a plausible-looking small
    /// number: 120 seconds for the chart, 70 for the coin list. Neither errors,
    /// and both quietly show a reader a couple of minutes of market while the
    /// interface says an hour.
    #[test]
    fn the_default_windows_are_the_spans_their_names_claim() {
        assert_eq!(DEFAULT_CANDLE_WINDOW_SECONDS, 3_600, "an hour of chart");
        assert_eq!(COINS_WINDOW_SECONDS, 600, "ten minutes of activity");
        assert_eq!(MAX_CANDLE_WINDOW_SECONDS, 86_400, "a day is the ceiling");
        // That a chart reaches further back than a tape is held at compile
        // time beside the constants themselves -- clippy rightly refuses an
        // assertion whose value is already known.
    }

    /// A window reaches back from its end, never forward.
    ///
    /// Kills the mutants turning the subtraction into an addition or a
    /// division. Forward, the window names a span that has not happened, the
    /// store holds nothing in it, and every caller is told the market was
    /// quiet.
    #[test]
    fn a_window_reaches_back_from_its_end() {
        assert_eq!(reaching_back(1_000, 120), 880);
        assert!(
            reaching_back(1_000, 120) < 1_000,
            "a window that starts after it ends is not a window"
        );
        assert_eq!(
            1_000 - reaching_back(1_000, 120),
            120,
            "and it is exactly the span asked for"
        );
    }

    /// Coverage for another table is not coverage for this one.
    ///
    /// `radar-follow` writes coverage for `Launches` and `Graduations` against
    /// the same store, continuously. A check that matched any coverage row at
    /// all would therefore report the market tape as collected on every
    /// established instance -- including one where the market-tape unit was
    /// never installed, which is exactly the deployment this is meant to
    /// catch. Kills the mutant replacing `==` with `!=`.
    #[test]
    fn another_tables_coverage_is_not_the_market_tapes() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        {
            let mut writer =
                radar_store::Writer::open(dir.path().to_str().expect("a path"), 20_000)
                    .expect("a writer");
            writer
                .append_coverage(radar_store::Coverage {
                    recorded_at: radar_types::Slot(100),
                    table: Table::Launches,
                    filter: None,
                    observed: radar_store::ObservedSlots::Nothing,
                    source: "test".to_owned(),
                    decoder_version: "test".to_owned(),
                    status: radar_store::Completion::Complete,
                })
                .expect("coverage appends");
            writer.flush().expect("flush");
        }
        let store = Reader::open(dir.path().to_str().expect("a path"));
        let as_of = AsOf::at(radar_types::Slot(10_000));
        assert!(
            !market_tape_collected(&store, as_of).expect("the store reads"),
            "a Launches coverage row does not mean the market tape ran"
        );
    }

    /// The store's own newest moment, not the wall clock.
    ///
    /// **This is the defect that made every route answer empty.** The
    /// collector runs behind wall-clock by design, so a window ending at
    /// `now()` named a span it had not reached, and every handler returned an
    /// empty list marked complete — a confident claim that nothing traded.
    /// Observed against a store holding 900 trades.
    #[test]
    fn the_default_window_ends_where_the_store_reaches_not_at_the_clock() {
        let (_dir, store) = store_of(&[
            market_trade(A_MINT, "2020-01-01 00:00:01.000000", 100),
            market_trade(A_MINT, "2020-01-01 00:00:59.000000", 200),
        ]);
        let as_of = AsOf::at(radar_types::Slot(10_000));

        let newest = newest_collected(&store, as_of).expect("the store reads");
        let expected = radar_store::to_epoch("2020-01-01 00:00:59").expect("a stamp");
        assert_eq!(
            newest,
            Some(expected),
            "the horizon is the newest row, whatever year the clock says"
        );
    }

    /// Microseconds are trimmed toward the second, never rounded up.
    ///
    /// The window's upper bound is exclusive, so a horizon rounded *up* past
    /// the newest row would claim to cover a moment nothing was read at.
    #[test]
    fn the_horizon_truncates_toward_the_second_rather_than_past_it() {
        let (_dir, store) = store_of(&[market_trade(A_MINT, "2020-01-01 00:00:59.999999", 200)]);
        let as_of = AsOf::at(radar_types::Slot(10_000));
        let newest = newest_collected(&store, as_of).expect("the store reads");
        assert_eq!(
            newest,
            radar_store::to_epoch("2020-01-01 00:00:59").ok(),
            "59.999999 is not 60"
        );
    }

    /// An empty store is not a quiet market.
    #[test]
    fn a_store_with_no_trades_has_no_horizon_rather_than_a_zero_one() {
        let (_dir, store) = store_of(&[]);
        let as_of = AsOf::at(radar_types::Slot(10_000));
        assert_eq!(
            newest_collected(&store, as_of).expect("the store reads"),
            None,
            "no rows means no horizon, never the epoch"
        );
    }

    /// Nothing past the watermark is visible, including to the horizon.
    ///
    /// AGENTS §4 rule 3. A horizon read past `as_of` would let a replay see a
    /// later moment than the replay is for, and every window derived from it
    /// would inherit that.
    #[test]
    fn the_horizon_never_reaches_past_the_watermark() {
        let (_dir, store) = store_of(&[
            market_trade(A_MINT, "2020-01-01 00:00:01.000000", 100),
            market_trade(A_MINT, "2020-01-01 00:09:00.000000", 900),
        ]);
        let as_of = AsOf::at(radar_types::Slot(500));
        assert_eq!(
            newest_collected(&store, as_of).expect("the store reads"),
            radar_store::to_epoch("2020-01-01 00:00:01").ok(),
            "the row at slot 900 is past the watermark and must not be seen"
        );
    }

    /// The tape is one mint's, and it is newest first.
    #[test]
    fn the_tape_is_one_mints_own_trades_newest_first() {
        let other = "So11111111111111111111111111111111111111112";
        let (_dir, store) = store_of(&[
            market_trade(A_MINT, "2020-01-01 00:00:01.000000", 100),
            market_trade(other, "2020-01-01 00:00:02.000000", 110),
            market_trade(A_MINT, "2020-01-01 00:00:03.000000", 120),
        ]);
        let as_of = AsOf::at(radar_types::Slot(10_000));
        let tape = tape_for(
            &store,
            as_of,
            A_MINT.parse().expect("a mint"),
            "2020-01-01 00:00:00",
            "2020-01-01 00:01:00",
        )
        .expect("the store reads");
        assert_eq!(tape.len(), 2, "the other mint's trade is not this tape's");
        assert_eq!(tape[0].ts, "2020-01-01 00:00:03.000000", "newest first");
    }

    /// A window that excludes every row returns nothing, without error.
    ///
    /// The bound is half-open — `>= from`, `< to` — so a row exactly at `to`
    /// belongs to the next window, not this one. Pinned because the horizon is
    /// derived from the newest row and an inclusive upper bound would make
    /// every default window double-count its own edge.
    #[test]
    fn the_window_bound_is_half_open_at_the_top() {
        // The stamp is compared as a **string**, so it must equal the bound
        // exactly to sit on the boundary at all. An earlier version of this
        // test used `...00:00:30.000000` against a bound of `...00:00:30`:
        // the longer string sorts after the shorter one under `<` and `<=`
        // alike, so it passed against both and the mutant survived a test
        // written to kill it.
        let (_dir, store) = store_of(&[market_trade(A_MINT, "2020-01-01 00:00:30", 100)]);
        let as_of = AsOf::at(radar_types::Slot(10_000));
        let mint: Address = A_MINT.parse().expect("a mint");

        let excluded = tape_for(
            &store,
            as_of,
            mint,
            "2020-01-01 00:00:00",
            "2020-01-01 00:00:30",
        )
        .expect("the store reads");
        assert!(excluded.is_empty(), "a row at `to` is the next window's");

        let included = tape_for(
            &store,
            as_of,
            mint,
            "2020-01-01 00:00:30",
            "2020-01-01 00:00:31",
        )
        .expect("the store reads");
        assert_eq!(included.len(), 1, "a row at `from` is this window's");
    }

    fn coin(
        tx_count: u64,
        quote_volume: Option<f64>,
        change_pct: Option<f64>,
    ) -> market_fold::Coin {
        market_fold::Coin {
            mint: format!("MINT{tx_count}"),
            tx_count,
            token_volume: None,
            quote_mint: None,
            quote_volume,
            price: None,
            change_pct,
        }
    }

    /// Each named sort orders by its own column, and an unknown one falls back
    /// to activity rather than leaving the page as it arrived.
    ///
    /// Kills the mutants that delete the `"volume"` and `"change"` arms: with
    /// either gone the request still answers, the column header still says what
    /// it sorted by, and the rows are simply in a different order -- a screen
    /// lying about its own controls, with nothing failing.
    #[test]
    fn each_sort_orders_by_its_own_column_and_an_unknown_one_falls_back() {
        // The three orderings are deliberately all different. An earlier
        // version of this test had the change ordering coincide with the
        // activity ordering, so deleting the "change" arm -- which falls
        // through to activity -- changed nothing it asserted, and the mutant
        // survived a test written to kill it.
        let sample = || {
            vec![
                coin(5, Some(1.0), Some(50.0)),
                coin(1, Some(9.0), Some(10.0)),
                coin(9, Some(4.0), Some(-5.0)),
            ]
        };

        let mut by_volume = sample();
        sort_coins(&mut by_volume, "volume");
        assert_eq!(
            by_volume.iter().map(|c| c.tx_count).collect::<Vec<_>>(),
            vec![1, 9, 5],
            "volume order is 9.0, 4.0, 1.0"
        );

        let mut by_change = sample();
        sort_coins(&mut by_change, "change");
        assert_eq!(
            by_change.iter().map(|c| c.tx_count).collect::<Vec<_>>(),
            vec![5, 1, 9],
            "change order is 50%, 10%, -5% -- and not the activity order"
        );

        let mut by_activity = sample();
        sort_coins(&mut by_activity, "nonsense");
        assert_eq!(
            by_activity.iter().map(|c| c.tx_count).collect::<Vec<_>>(),
            vec![9, 5, 1],
            "an unrecognised sort is activity, not whatever order arrived"
        );
    }

    /// An unmeasured figure sorts last in a descending column, never first.
    #[test]
    fn a_coin_with_no_volume_sorts_below_every_coin_that_has_one() {
        let mut coins = vec![coin(1, None, None), coin(2, Some(0.5), None)];
        sort_coins(&mut coins, "volume");
        assert_eq!(
            coins[0].tx_count, 2,
            "a priced coin outranks an unpriced one"
        );
    }

    /// Every degradation says something substantive, and no two say the same
    /// thing.
    ///
    /// Kills the mutants that replace `message` with `""` or a constant: an
    /// operator acts differently on "the store could not be read" than on
    /// "this range was never collected", and a blank or identical message
    /// takes that distinction away at exactly the moment it is needed. The
    /// live-query-era variants this pinned (`Unreachable`, `Malformed`,
    /// `TimedOut`, `RowCapHit`) are gone with the queries they classified --
    /// see the module doc comment -- and this is their replacement pinning
    /// the two that took their place.
    #[test]
    fn every_degradation_carries_its_own_non_empty_message_and_code() {
        let all = [
            Degradation::StoreUnreadable("boom".to_owned()),
            Degradation::NotCollected("fixture"),
        ];
        let mut messages: Vec<String> = all.iter().map(Degradation::message).collect();
        let mut codes: Vec<&str> = all.iter().map(Degradation::code).collect();
        assert!(
            messages.iter().all(|m| m.len() > 20),
            "a message has to explain, not label"
        );
        assert!(codes.iter().all(|c| !c.is_empty()));
        messages.sort_unstable();
        messages.dedup();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(messages.len(), all.len(), "no two degradations read alike");
        assert_eq!(codes.len(), all.len(), "no two degradations share a code");
    }

    #[test]
    fn every_declared_interval_maps_to_a_distinct_number_of_seconds() {
        let names = ["1m", "5m", "15m", "1h", "4h", "1d"];
        let mut seconds: Vec<i64> = names.iter().map(|n| interval_seconds(n).unwrap()).collect();
        seconds.sort_unstable();
        seconds.dedup();
        assert_eq!(seconds.len(), names.len());
        assert_eq!(
            interval_seconds("2m"),
            None,
            "an undeclared interval is refused"
        );
    }

    #[test]
    fn degradation_reasons_are_distinguishable_and_never_look_like_success() {
        let store_error = Degradation::StoreUnreadable("boom".to_owned());
        let not_collected = Degradation::NotCollected("never asked");
        assert!(store_error.status().is_server_error());
        assert!(not_collected.status().is_server_error());
        assert_ne!(store_error.status(), StatusCode::OK);
        assert_ne!(not_collected.status(), StatusCode::OK);
        assert_ne!(store_error.code(), not_collected.code());
    }

    #[test]
    fn not_collected_and_collected_and_quiet_are_different_sentences() {
        // The rule 9 property this module exists to hold: an empty result
        // and "we never looked" must never share a code or a status.
        let not_collected = Degradation::NotCollected("fixture");
        assert_eq!(not_collected.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_ne!(not_collected.status(), StatusCode::OK);
        assert!(not_collected.message().contains("not collected"));
    }

    #[test]
    fn a_malformed_mint_is_refused_before_any_store_read() {
        assert!(parse_mint("not-base58!!").is_err());
        assert!(parse_mint("").is_err());
        assert!(parse_mint("5NfV2sy8DqXamLvYEE4LcTWzGqZc5Emv4bqqhVDWpump").is_ok());
    }

    #[test]
    fn a_malformed_timestamp_is_refused_rather_than_silently_defaulted() {
        assert!(parse_stamp("not a timestamp").is_err());
        assert!(parse_stamp("2026-09-11 17:00:00").is_ok());
    }

    fn mint(n: u8) -> Address {
        Address::new([n; 32])
    }

    fn trade(
        mint: Address,
        ts: &str,
        sig: u8,
        price: Option<f64>,
        quote: Option<f64>,
    ) -> MarketTrade {
        MarketTrade {
            mint,
            ts: ts.to_owned(),
            slot: radar_types::Slot(1),
            signature: radar_types::Signature::new([sig; 64]),
            side: if price.is_some() {
                MarketSide::Buy
            } else {
                MarketSide::Unknown
            },
            token_amount: 1.0,
            quote_amount: quote,
            quote_mint: quote.map(|_| {
                "So11111111111111111111111111111111111111112"
                    .parse()
                    .expect("quote mint")
            }),
            price,
            trader: None,
        }
    }

    #[test]
    fn coins_from_trades_folds_one_row_per_mint() {
        let rows = vec![
            trade(
                mint(1),
                "2026-09-11 17:00:00.000000",
                1,
                Some(1.0),
                Some(1.0),
            ),
            trade(
                mint(1),
                "2026-09-11 17:01:00.000000",
                2,
                Some(1.5),
                Some(1.5),
            ),
            trade(mint(2), "2026-09-11 17:00:00.000000", 3, None, None),
        ];
        let coins = coins_from_trades(&rows);
        assert_eq!(coins.len(), 2, "one row per mint, not one per trade");

        let a = coins
            .iter()
            .find(|c| c.mint == mint(1).to_string())
            .expect("mint 1");
        assert_eq!(a.tx_count, 2);
        assert_eq!(a.price, Some(1.5), "the last priced fill");
        assert!(
            (a.change_pct.unwrap() - 50.0).abs() < 1e-9,
            "50% from the first priced fill to the last"
        );

        let b = coins
            .iter()
            .find(|c| c.mint == mint(2).to_string())
            .expect("mint 2");
        assert_eq!(b.price, None, "never priced in the window");
        assert_eq!(b.change_pct, None);
    }

    #[test]
    fn a_mint_with_no_priced_trade_reports_no_price_not_a_dropped_row() {
        let rows = vec![trade(mint(1), "2026-09-11 17:00:00.000000", 1, None, None)];
        let coins = coins_from_trades(&rows);
        assert_eq!(coins.len(), 1, "the mint is still on the list");
        assert_eq!(coins[0].price, None);
        assert_eq!(coins[0].quote_mint, None);
    }
}
