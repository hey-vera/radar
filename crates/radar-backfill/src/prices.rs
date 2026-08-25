// SPDX-License-Identifier: Apache-2.0
//! What a token actually traded at, from realised balance deltas.
//!
//! # Why this exists
//!
//! `Outcome` recorded activity and survival — transfers, distinct accounts,
//! whether the token reached an AMM — and **no price at all**. Every signal in
//! the repository was therefore validated against *graduation*, which is a proxy
//! for profit and, on the bundle cohort, an inverted one: an instant graduation
//! means the supply is already held by whoever arranged it, so the graduation is
//! real and the opportunity belongs to somebody else.
//!
//! Without a price path nothing can answer *"would this trade have made
//! money?"*, which is the only question that decides whether Radar is a trading
//! tool.
//!
//! # Realised, not requested
//!
//! [ADR 0002](../../docs/adr/0002-historical-data-comes-from-cryptohouse-not-a-vendor-archive.md)
//! anticipated this: `solana.transactions.balance_changes` carries what actually
//! moved, where an instruction's arguments carry only what the trader asked for.
//! A `buy(tokens, max_sol_cost)` says nothing about what was spent — and
//! LEARNINGS entry 4 is about the two `u64`s that swap meaning between variants,
//! so parsing the request is exactly the wrong place to look for a fill.
//!
//! Verified against mainnet before this module was written. On
//! `Q5QRogEuf…pump`, one transaction moved 2,473,715,481,027 base units against
//! a +80,427,534 lamport delta and a -82,685,094 one — the pool's receipt and
//! the trader's payment, differing by the fees in between. The resulting price
//! agrees with an independent Jupiter quote taken eleven hours later to within
//! the move that happened in between.
//!
//! # One row per mint, which is why this is affordable
//!
//! Same discipline as [`crate::outcomes`]: aggregates return one row per mint
//! however many billions they scan, so a batch is a single query against a cap
//! that counts result rows. A per-fill extraction of the same data would be
//! millions of rows a day and a different plan entirely (ADR 0002).

use std::collections::BTreeMap;

use radar_store::PRICE_SCALE;
use serde::Deserialize;

/// Mints per price query.
///
/// Matched to [`crate::outcomes::MINTS_PER_BATCH`] so one chunk serves both
/// queries, because **the cost of this query barely depends on how many mints it
/// asks about**. That is the opposite of what it looks like, and measuring was
/// the only way to find it. Over a fixed 24-hour window on the live endpoint:
///
/// | mints | read | elapsed |
/// |---|---|---|
/// | 25 | 193.9 GB | 22.9 s |
/// | 200 | 211.8 GB | 51.3 s |
///
/// Nearly all of that is the window scan itself. The lever is [`WINDOW_HOURS`],
/// not the batch.
pub const MINTS_PER_BATCH: usize = 400;

/// One aggregate price row, as the endpoint returns it.
///
/// Every field arrives as a string because the values exceed what JSON numbers
/// carry safely, and a price silently rounded through an `f64` is a price that
/// compares differently on a replay.
#[derive(Debug, Clone, Deserialize)]
pub struct PriceRow {
    /// The mint.
    pub mint: String,
    /// Fills the aggregate was computed from.
    pub fills: String,
    /// Volume-weighted average price, at [`PRICE_SCALE`].
    pub vwap: String,
    /// Price of the earliest fill in the window.
    pub first_price: String,
    /// Price of the latest fill in the window.
    pub last_price: String,
    /// Highest price any fill traded at.
    pub peak_price: String,
    /// Lowest price any fill traded at.
    pub trough_price: String,
}

/// What a batch of mints traded at over a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Prices {
    /// Fills observed.
    pub fills: u64,
    /// Volume-weighted average price.
    pub vwap: Option<u64>,
    /// First fill's price.
    pub first: Option<u64>,
    /// Last fill's price.
    pub last: Option<u64>,
    /// Highest fill price — maximum favourable excursion.
    pub peak: Option<u64>,
    /// Lowest fill price — maximum adverse excursion.
    pub trough: Option<u64>,
}

/// Builds the price query for a batch of mints over one window.
///
/// The window is bounded at both ends, unlike the outcomes query. Prices are a
/// *path*, so the same mint is asked about repeatedly over successive windows
/// and the answers are folded together — where an outcome aggregate is a single
/// open-ended "what has happened so far".
///
/// `balance_changes` is a `Decimal(38, 9)` **already denominated in lamports**,
/// which is worth stating because the type invites a conversion that would be
/// wrong by a factor of a billion. It was wrong by exactly that in the first
/// draft, and only cross-checking against an independent quote caught it.
///
/// # Only transfers between accounts are trades
///
/// `transfer_type` must be filtered or the aggregate is meaningless. A launch
/// emits one `MintTo` carrying the **entire supply** — 1e15 base units against
/// whatever SOL happened to move in the creation transaction, which is 44x the
/// average real fill on the same token. Being the earliest event, it became
/// `first_price`, and every excursion was then measured against the supply mint
/// rather than against a trade.
///
/// The effect was not subtle and was still easy to miss, because a wrong price
/// looks like a number rather than like an error. Over 188 tokens the unfiltered
/// aggregate reported a median MFE of **7,054%** and a 90th percentile of
/// **1,081,162%**. Filtered to `Transfer` and `TransferChecked`, the same cohort
/// reports a median MFE of 23.7% and a median held-to-end of **-13.4%** — which
/// is consistent with the base rate `AGENTS.md` opens with, where the first set
/// was consistent with nothing.
///
/// # The signature pushdown is load-bearing
///
/// `signature IN (SELECT tx_signature FROM sigs)` is what makes this affordable.
/// Without it the server materialises `balance_changes` for *every* transaction
/// in the window — a six-hour window failed with `Query memory limit exceeded:
/// would use 18.63 GiB`. With it, six hours reads 12.8 GB and a full day 28.7 GB.
/// Removing it does not change a single returned value, which is exactly why it
/// would be easy to remove.
#[must_use]
pub fn query_for_mints(mints: &[String], from: &str, to: &str) -> String {
    let list = mints
        .iter()
        .map(|m| format!("'{m}'"))
        .collect::<Vec<_>>()
        .join(",");

    format!(
        "WITH sigs AS (\
           SELECT mint, tx_signature, min(block_timestamp) AS ts, \
                  sum(toInt128(value)) AS tok \
           FROM solana.token_transfers \
           WHERE block_timestamp >= '{from}' AND block_timestamp < '{to}' \
             AND mint IN ({list}) AND value > 0              AND transfer_type IN ('Transfer', 'TransferChecked') \
           GROUP BY mint, tx_signature\
         ), per_tx AS (\
           SELECT s.mint AS mint, s.ts AS ts, s.tok AS tok, \
                  arrayMax(arrayMap(x -> toInt128(x.3 - x.2), tx.balance_changes)) AS lam \
           FROM sigs AS s \
           INNER JOIN (SELECT signature, balance_changes FROM solana.transactions \
                       WHERE block_timestamp >= '{from}' AND block_timestamp < '{to}' \
                         AND signature IN (SELECT tx_signature FROM sigs)) AS tx \
             ON s.tx_signature = tx.signature\
         ) \
         SELECT mint, toString(count()) AS fills, \
                toString(toUInt64(sum(lam) / sum(tok) * {PRICE_SCALE})) AS vwap, \
                toString(toUInt64(argMin(lam / tok, ts) * {PRICE_SCALE})) AS first_price, \
                toString(toUInt64(argMax(lam / tok, ts) * {PRICE_SCALE})) AS last_price, \
                toString(toUInt64(max(lam / tok) * {PRICE_SCALE})) AS peak_price, \
                toString(toUInt64(min(lam / tok) * {PRICE_SCALE})) AS trough_price \
         FROM per_tx WHERE tok > 0 AND lam > 0 GROUP BY mint"
    )
}

/// Turns aggregate rows into prices by mint.
///
/// A row that cannot be parsed is dropped rather than defaulted. A price of zero
/// is a claim that the token was worthless, and an unparseable field is not that
/// claim — it is the absence of one (rule 9).
#[must_use]
pub fn to_prices(rows: &[PriceRow]) -> BTreeMap<String, Prices> {
    let mut out = BTreeMap::new();
    for row in rows {
        let parse = |s: &str| s.parse::<u64>().ok().filter(|v| *v > 0);
        out.insert(
            row.mint.clone(),
            Prices {
                fills: row.fills.parse().unwrap_or(0),
                vwap: parse(&row.vwap),
                first: parse(&row.first_price),
                last: parse(&row.last_price),
                peak: parse(&row.peak_price),
                trough: parse(&row.trough_price),
            },
        );
    }
    out
}

impl Prices {
    /// Folds a later window's prices into an earlier one's.
    ///
    /// The path is built from successive windows, and which end wins is not
    /// symmetric: `first` belongs to the earliest window that saw a fill and
    /// `last` to the most recent, while `peak` and `trough` are extremes over
    /// everything seen. Getting this backwards would make the excursion figures
    /// describe one window rather than the token's life.
    ///
    /// `self` is the earlier window and `later` the newer one.
    #[must_use]
    pub fn fold(self, later: Self) -> Self {
        Self {
            fills: self.fills.saturating_add(later.fills),
            // Volume-weighting across windows would need per-window volume,
            // which the aggregate does not return. The most recent window's
            // average is the honest answer, and it is not claimed to be a
            // lifetime VWAP.
            vwap: later.vwap.or(self.vwap),
            first: self.first.or(later.first),
            last: later.last.or(self.last),
            peak: max_opt(self.peak, later.peak),
            trough: min_opt(self.trough, later.trough),
        }
    }
}

fn max_opt(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (x, None) | (None, x) => x,
    }
}

fn min_opt(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (x, None) | (None, x) => x,
    }
}

/// How far back a single price query reaches.
///
/// Six hours, and the number is a measurement rather than a preference. Against
/// 200 mints on the live endpoint:
///
/// | window | read | elapsed |
/// |---|---|---|
/// | 1 hour | 18.7 GB | 1.6 s |
/// | 4 hours | 56.7 GB | 3.3 s |
/// | 24 hours | 211.8 GB | 51.3 s |
///
/// Twenty-four hours is not affordable: 51 s against a 60 s server timeout, and
/// a 50-mint run at that width timed out outright while a 200-mint one did not —
/// the endpoint is shared and the variance at that size is larger than the
/// margin. Six hours is a twentieth of the cost with room to spare.
///
/// # What this does and does not cover
///
/// Tokens are measured at [`crate::checkpoints::CHECKPOINTS`] — 1 h, 6 h and
/// 24 h after launch. A six-hour window covers a token's **whole life at the
/// first two**, which are the checkpoints a trade would live inside. At the
/// 24-hour checkpoint it covers hours 18–24, and [`Prices::fold`] carries hours
/// 0–6 forward from the earlier measurement, leaving **hours 6–18 unpriced**.
///
/// So `peak_price` and `trough_price` are extremes over what was *measured*, not
/// over the token's whole life, and `fills` is what says how much that was. That
/// distinction is why the fill count is stored rather than derived.
pub const WINDOW_HOURS: i64 = 6;

/// Where a price window starts: [`WINDOW_HOURS`] before `to`, but never earlier
/// than `since`.
///
/// `since` is the earliest launch the run cares about, so reaching further back
/// would price chain that no token in the batch existed for.
///
/// Both are `YYYY-MM-DD HH:MM:SS`. On anything it cannot parse it returns
/// `since` — the narrower window, which under-measures rather than issuing a
/// query whose cost nobody predicted.
#[must_use]
pub fn window_start(to: &str, since: &str) -> String {
    let Some(start) = shift_hours(to, -WINDOW_HOURS) else {
        return since.to_owned();
    };
    if start.as_str() < since {
        since.to_owned()
    } else {
        start
    }
}

/// Shifts a `YYYY-MM-DD HH:MM:SS` timestamp by whole hours.
///
/// Hand-rolled because the alternative is a date-time dependency in a process
/// that will eventually sit beside a signing key, and the arithmetic needed is
/// days-and-hours rather than calendars.
fn shift_hours(at: &str, hours: i64) -> Option<String> {
    let (date, time) = at.split_once(' ')?;
    let mut d = date.splitn(3, '-');
    let (y, m, day): (i64, i64, i64) = (
        d.next()?.parse().ok()?,
        d.next()?.parse().ok()?,
        d.next()?.parse().ok()?,
    );
    let mut t = time.splitn(3, ':');
    let (hh, mm, ss): (i64, i64, i64) = (
        t.next()?.parse().ok()?,
        t.next()?.parse().ok()?,
        t.next()?.split('.').next()?.parse().ok()?,
    );

    let total = days_from_civil(y, m, day).checked_mul(86_400)? + hh * 3_600 + mm * 60 + ss;
    let shifted = total.checked_add(hours.checked_mul(3_600)?)?;
    let (days, secs) = (shifted.div_euclid(86_400), shifted.rem_euclid(86_400));
    let (y, m, day) = civil_from_days(days);
    Some(format!(
        "{y:04}-{m:02}-{day:02} {:02}:{:02}:{:02}",
        secs / 3_600,
        (secs % 3_600) / 60,
        secs % 60
    ))
}

/// Days since 1970-01-01 from a civil date. Howard Hinnant's algorithm.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The inverse of [`days_from_civil`].
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Attaches measured prices to outcomes, folding them onto what was already
/// known about each mint.
///
/// `prior` is the price path from a mint's most recent earlier measurement.
/// Folding rather than overwriting is what lets a six-hour window describe a
/// token older than six hours: each run contributes the slice it could see, and
/// the extremes accumulate. Overwriting would make every measurement describe
/// only the last window, so a token's peak would fall out of the record as soon
/// as it aged past one.
///
/// An outcome whose mint appears in neither map keeps its prices absent. That is
/// the common case and the correct one: most tokens in a batch never traded, and
/// a token that never traded has no price rather than a price of nothing.
#[must_use]
pub fn apply(
    outcomes: Vec<radar_store::Outcome>,
    priced: &BTreeMap<String, Prices>,
    prior: &BTreeMap<String, Prices>,
) -> Vec<radar_store::Outcome> {
    outcomes
        .into_iter()
        .map(|mut o| {
            let key = o.mint.to_string();
            let folded = match (prior.get(&key), priced.get(&key)) {
                (Some(before), Some(now)) => Some(before.fold(*now)),
                (Some(before), None) => Some(*before),
                (None, Some(now)) => Some(*now),
                (None, None) => None,
            };
            if let Some(p) = folded {
                o.first_price = p.first;
                o.last_price = p.last;
                o.peak_price = p.peak;
                o.trough_price = p.trough;
                o.vwap = p.vwap;
                o.fills = p.fills;
            }
            o
        })
        .collect()
}

/// The price path each mint was last measured with.
///
/// Built from outcomes already in the store, keeping the most recent measurement
/// per mint — which is what [`apply`] folds the new window onto.
#[must_use]
pub fn prior_prices(already: &[radar_store::Outcome]) -> BTreeMap<String, Prices> {
    let mut latest: BTreeMap<String, (radar_types::Slot, Prices)> = BTreeMap::new();
    for o in already {
        if o.fills == 0 && o.first_price.is_none() {
            continue;
        }
        let entry = latest
            .entry(o.mint.to_string())
            .or_insert((o.measured_at, Prices::default()));
        if o.measured_at >= entry.0 {
            *entry = (
                o.measured_at,
                Prices {
                    fills: o.fills,
                    vwap: o.vwap,
                    first: o.first_price,
                    last: o.last_price,
                    peak: o.peak_price,
                    trough: o.trough_price,
                },
            );
        }
    }
    latest.into_iter().map(|(m, (_, p))| (m, p)).collect()
}
#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(mint: u8) -> radar_store::Outcome {
        radar_store::Outcome {
            mint: radar_types::Address::new([mint; 32]),
            measured_at: radar_types::Slot(500_000),
            launch_slot: radar_types::Slot(1_000),
            first_transfer_slot: None,
            last_transfer_slot: None,
            transfers: 0,
            unique_senders: 0,
            unique_receivers: 0,
            graduated_at: None,
            first_price: None,
            last_price: None,
            peak_price: None,
            trough_price: None,
            vwap: None,
            fills: 0,
        }
    }

    fn row(mint: &str, fills: &str, first: &str, last: &str, peak: &str, trough: &str) -> PriceRow {
        PriceRow {
            mint: mint.to_owned(),
            fills: fills.to_owned(),
            vwap: "21002820020797".to_owned(),
            first_price: first.to_owned(),
            last_price: last.to_owned(),
            peak_price: peak.to_owned(),
            trough_price: trough.to_owned(),
        }
    }

    #[test]
    fn the_query_bounds_both_ends_of_the_window() {
        // Unlike the outcomes query, which is deliberately open-ended. A price
        // is a path, so an unbounded upper end would fold every later window's
        // prices into the first one asked for.
        let q = query_for_mints(
            &["MINT1".to_owned()],
            "2026-08-25 05:00:00",
            "2026-08-25 07:00:00",
        );
        assert!(q.contains(">= '2026-08-25 05:00:00'"));
        assert!(q.contains("< '2026-08-25 07:00:00'"));
        assert!(q.contains("'MINT1'"));
    }

    #[test]
    fn the_query_does_not_rescale_lamports() {
        // `balance_changes` is a Decimal(38, 9) already denominated in lamports.
        // The type invites a * 1e9 that would be wrong by a factor of a billion,
        // and the first draft of this query had exactly that.
        let q = query_for_mints(&["M".to_owned()], "a", "b");
        assert!(
            !q.contains("1000000000)"),
            "no lamport rescaling belongs in this query: {q}"
        );
        assert!(q.contains("toInt128(x.3 - x.2)"));
    }

    #[test]
    fn the_query_counts_only_transfers_between_accounts() {
        // A launch emits one `MintTo` carrying the entire supply, and it is the
        // earliest event -- so without this filter it becomes `first_price` and
        // every excursion is measured against the supply mint instead of a
        // trade. Unfiltered, a 188-token cohort reported a median MFE of 7,054%;
        // filtered, 23.7%. The first number looks like a number rather than like
        // an error, which is why this is asserted rather than eyeballed.
        let q = query_for_mints(&["M".to_owned()], "a", "b");
        assert!(
            q.contains("transfer_type IN ('Transfer', 'TransferChecked')"),
            "mint and burn events are not fills: {q}"
        );
    }

    #[test]
    fn the_query_pushes_the_signature_filter_into_the_transactions_scan() {
        // Invisible in the results and decisive for whether the query runs at
        // all: without it a six-hour window fails with `Query memory limit
        // exceeded: would use 18.63 GiB`, because the server materialises
        // `balance_changes` for every transaction in the window rather than for
        // the few thousand that touch these mints.
        //
        // A correctness test cannot catch this — every returned value is
        // identical either way — so it is asserted structurally.
        let q = query_for_mints(&["M".to_owned()], "a", "b");
        assert!(
            q.contains("signature IN (SELECT tx_signature FROM sigs)"),
            "the transactions scan must be filtered to the signatures of interest: {q}"
        );
    }

    #[test]
    fn a_zero_price_is_absent_rather_than_free() {
        // Rule 9. Zero is a claim that the token traded at nothing, which the
        // endpoint returns when a fill rounds below the scale — and a strategy
        // dividing by it would size an infinite position.
        let prices = to_prices(&[row("M", "3", "0", "0", "0", "0")]);
        let p = prices.get("M").expect("row survives");
        assert_eq!(p.first, None);
        assert_eq!(p.peak, None);
        assert_eq!(p.fills, 3, "the fill count is still a real measurement");
    }

    #[test]
    fn a_window_reaches_back_six_hours_but_no_further_than_the_earliest_launch() {
        // Six hours because that is what the endpoint will do: 3.3 s against
        // 51.3 s for twenty-four, on a sixty-second timeout. Reaching past the
        // earliest launch would price chain that no token in the batch existed
        // for -- paid for, and about nothing.
        assert_eq!(WINDOW_HOURS, 6, "the window this test is written against");
        assert_eq!(
            window_start("2026-08-25 16:00:00", "2020-01-01 00:00:00"),
            "2026-08-25 10:00:00",
            "six hours back when the launches are older than that"
        );
        assert_eq!(
            window_start("2026-08-25 16:00:00", "2026-08-25 14:00:00"),
            "2026-08-25 14:00:00",
            "clamped to the earliest launch when that is more recent than six hours back"
        );
        assert_eq!(
            window_start("2026-08-25 16:00:00", "2026-08-25 09:00:00"),
            "2026-08-25 10:00:00",
            "and not widened to reach a launch that is older than the window"
        );
    }

    #[test]
    fn an_unparseable_timestamp_narrows_the_window_rather_than_widening_it() {
        // Rule 8's shape. The failure has to be the cheap one: a narrow window
        // under-measures, where a wrong one issues a query whose cost nobody
        // predicted against a shared endpoint.
        assert_eq!(
            window_start("not a timestamp", "2026-08-25 09:00:00"),
            "2026-08-25 09:00:00"
        );
        assert_eq!(
            window_start("", "2026-08-25 09:00:00"),
            "2026-08-25 09:00:00"
        );
    }

    #[test]
    fn the_hour_shift_crosses_days_months_and_years() {
        // Hand-rolled calendar arithmetic, so the boundaries are the test.
        assert_eq!(
            shift_hours("2026-08-25 16:00:00", -24).unwrap(),
            "2026-08-24 16:00:00"
        );
        assert_eq!(
            shift_hours("2026-08-01 05:00:00", -24).unwrap(),
            "2026-07-31 05:00:00"
        );
        assert_eq!(
            shift_hours("2026-01-01 00:00:00", -1).unwrap(),
            "2025-12-31 23:00:00"
        );
        assert_eq!(
            shift_hours("2026-03-01 00:00:00", -24).unwrap(),
            "2026-02-28 00:00:00"
        );
        // 2024 was a leap year; 2026 is not, and the two must differ.
        assert_eq!(
            shift_hours("2024-03-01 00:00:00", -24).unwrap(),
            "2024-02-29 00:00:00"
        );
        // Fractional seconds are what the endpoint actually returns.
        assert_eq!(
            shift_hours("2026-08-25 05:27:51.000000", -24).unwrap(),
            "2026-08-24 05:27:51"
        );
    }

    #[test]
    fn applying_prices_leaves_unpriced_mints_absent() {
        // The common case: most tokens in a batch never traded, and a token that
        // never traded has no price rather than a price of nothing. Defaulting
        // these to zero would put every dead token at the bottom of every
        // drawdown statistic in the store.
        let traded = radar_types::Address::new([1u8; 32]).to_string();
        let priced = to_prices(&[row(&traded, "45", "100", "200", "300", "50")]);

        let out = apply(vec![outcome(1), outcome(2)], &priced, &BTreeMap::new());
        assert_eq!(
            out[0].first_price,
            Some(100),
            "the priced mint gets its path"
        );
        assert_eq!(out[0].fills, 45);
        assert_eq!(out[1].first_price, None, "the unpriced one stays absent");
        assert_eq!(out[1].fills, 0);
    }

    #[test]
    fn the_measured_excursions_of_a_real_token_reproduce() {
        // Q5QRogEuf…pump over its first day, from the live endpoint on
        // 2026-08-25. It rose 77% and fell 95% inside two minutes of trading,
        // which is the shape an exit rule has to be fit against -- and the
        // reason `graduated_at` alone was never going to answer whether a trade
        // made money.
        let o = radar_store::Outcome {
            mint: radar_types::Address::new([1u8; 32]),
            measured_at: radar_types::Slot(500_000),
            launch_slot: radar_types::Slot(1_000),
            first_transfer_slot: None,
            last_transfer_slot: None,
            transfers: 46,
            unique_senders: 0,
            unique_receivers: 0,
            graduated_at: None,
            first_price: Some(38_876_174_875_644),
            last_price: Some(28_940_978_733_955),
            peak_price: Some(68_758_447_614_858),
            trough_price: Some(1_886_773_264_632),
            vwap: Some(21_002_820_020_797),
            fills: 45,
        };
        assert_eq!(o.mfe_bps(), Some(7_686), "+76.9% at the peak");
        assert_eq!(o.mae_bps(), Some(9_514), "-95.1% at the trough");
        assert_eq!(
            o.held_to_end_gain_bps(),
            Some(-2_555),
            "and -25.6% for anyone who simply held"
        );
    }

    #[test]
    fn folding_keeps_the_earliest_first_and_the_latest_last() {
        // The asymmetry that matters. Reversing it would make the excursion
        // figures describe one window rather than the token's life.
        let early = to_prices(&[row("M", "10", "100", "200", "300", "50")])["M"];
        let late = to_prices(&[row("M", "5", "400", "500", "900", "20")])["M"];

        let folded = early.fold(late);
        assert_eq!(
            folded.first,
            Some(100),
            "first comes from the earlier window"
        );
        assert_eq!(folded.last, Some(500), "last comes from the later window");
        assert_eq!(folded.peak, Some(900), "peak is the extreme over both");
        assert_eq!(folded.trough, Some(20), "trough is the extreme over both");
        assert_eq!(folded.fills, 15);
    }

    #[test]
    fn folding_a_silent_window_changes_nothing_but_is_not_a_reset() {
        // A window in which a token did not trade must not erase what is known.
        // Treating it as a fill at price zero would put the trough at nothing
        // and report a total loss for every token that simply went quiet.
        let seen = to_prices(&[row("M", "10", "100", "200", "300", "50")])["M"];
        let silent = Prices::default();

        let folded = seen.fold(silent);
        assert_eq!(folded.first, Some(100));
        assert_eq!(folded.last, Some(200));
        assert_eq!(folded.peak, Some(300));
        assert_eq!(folded.trough, Some(50), "silence is not a new low");
        assert_eq!(folded.fills, 10);
    }

    #[test]
    fn a_first_sighting_after_silence_is_still_the_first() {
        // The reverse order: nothing known, then a window with fills.
        let folded = Prices::default().fold(to_prices(&[row("M", "4", "7", "9", "11", "5")])["M"]);
        assert_eq!(folded.first, Some(7));
        assert_eq!(folded.last, Some(9));
        assert_eq!(folded.peak, Some(11));
        assert_eq!(folded.trough, Some(5));
    }

    #[test]
    fn a_new_window_folds_onto_what_was_already_known() {
        // The reason a six-hour window can describe a token older than six
        // hours. Overwriting would make each measurement describe only the last
        // window, so a token's peak would drop out of the record the moment it
        // aged past one — and the peak is the whole point of recording MFE.
        let key = radar_types::Address::new([1u8; 32]).to_string();
        let prior = to_prices(&[row(&key, "10", "100", "200", "900", "50")]);
        let now = to_prices(&[row(&key, "5", "400", "500", "300", "20")]);

        let out = apply(vec![outcome(1)], &now, &prior);
        assert_eq!(out[0].first_price, Some(100), "the earliest price survives");
        assert_eq!(
            out[0].last_price,
            Some(500),
            "the latest is the new window's"
        );
        assert_eq!(out[0].peak_price, Some(900), "peak from the earlier window");
        assert_eq!(out[0].trough_price, Some(20), "trough from the later one");
        assert_eq!(out[0].fills, 15, "fills accumulate across windows");
    }

    #[test]
    fn a_mint_that_did_not_trade_this_window_keeps_its_history() {
        // A quiet six hours must not erase a measured path. Dropping to absent
        // here would make an MFE disappear the moment trading stopped, which is
        // exactly when the number matters most.
        let key = radar_types::Address::new([1u8; 32]).to_string();
        let prior = to_prices(&[row(&key, "10", "100", "200", "900", "50")]);

        let out = apply(vec![outcome(1)], &BTreeMap::new(), &prior);
        assert_eq!(out[0].first_price, Some(100));
        assert_eq!(out[0].peak_price, Some(900));
        assert_eq!(out[0].fills, 10);
    }

    #[test]
    fn prior_prices_keeps_the_most_recent_measurement_per_mint() {
        // Outcomes are append-only, so a mint has one row per checkpoint. The
        // path to fold onto is the newest of them; folding onto an older one
        // would re-apply a window that has already been counted.
        let priced_outcome = |measured: u64, first: u64, peak: u64, fills: u64| {
            let mut o = outcome(1);
            o.measured_at = radar_types::Slot(measured);
            o.first_price = Some(first);
            o.peak_price = Some(peak);
            o.fills = fills;
            o
        };
        let already = vec![
            priced_outcome(9_000, 100, 500, 4),
            priced_outcome(54_000, 100, 900, 11),
        ];

        let prior = prior_prices(&already);
        let p = prior
            .get(&radar_types::Address::new([1u8; 32]).to_string())
            .expect("the mint is known");
        assert_eq!(p.peak, Some(900), "the newer measurement wins");
        assert_eq!(p.fills, 11);
    }

    #[test]
    fn an_unpriced_history_contributes_nothing_to_fold_onto() {
        // Every outcome written before prices existed has no path. Carrying one
        // in as a price would invent a first price of nothing for every token
        // already in the store.
        assert!(
            prior_prices(&[outcome(1)]).is_empty(),
            "nothing measured means nothing to fold"
        );
    }
}
