// SPDX-License-Identifier: Apache-2.0
//! The market-tape collector: spends the CryptoHouse budget so `radar-serve`
//! does not have to.
//!
//! # The problem this exists to close
//!
//! CryptoHouse permits 120 queries an hour per IP, shared with `radar-follow`
//! and the hourly `--outcomes` cron on the same box. `radar-serve`'s market
//! routes used to query CryptoHouse live, once or more per HTTP request —
//! measured at 291 queries in one hour from a handful of terminal loads, well
//! past the quota, and every subsequent request failed with `QUOTA_EXCEEDED`
//! until the hour rolled over. No cache TTL fixes this: the coin list alone
//! costs two queries, so a one-minute refresh consumes the whole hourly
//! budget by itself, before a single visitor asks for anything else.
//!
//! # The fix, and its arithmetic
//!
//! Flip the direction. A collector spends the budget on a fixed schedule and
//! writes what it finds to the store; `radar-serve` reads only the store and
//! issues zero CryptoHouse queries on any request path (`radar_serve::market`).
//!
//! A pass issues [`super::market::query::coin_candidates_query`] for the
//! window's active mints, then one batched
//! [`super::market::query::trades_query`] for all of them.
//!
//! **An earlier version of this comment said that was two queries a pass, 60
//! an hour, and it was wrong by a factor of sixteen.** Measured 2026-09-12: one
//! pass issued about thirty-two queries and collected 20,710 trades across 200
//! mints. The second of those two queries is not one query — CryptoHouse caps a
//! result at a thousand rows, [`radar_backfill::narrowing_fetch`] halves a
//! window that exceeds it, and each half may halve again. Fifteen halvings were
//! logged in a single pass. At one pass every two minutes that is roughly 960
//! queries an hour against a 120/hour quota: the collector would have exhausted
//! the budget in under eight minutes and then served nothing, which is the
//! exact failure it was written to prevent.
//!
//! The arithmetic counted the queries the code *writes*, not the ones the row
//! cap turns them into. Nothing counted the real ones, so nothing could notice.
//!
//! # What the budget can actually buy
//!
//! Two hundred mints trade roughly 620,000 times an hour. At a thousand rows a
//! query that is 620 queries to collect completely, against 120 available —
//! **the free quota can carry about a fifth of that tape, and no arrangement of
//! this code changes the ratio.** So the collector does not try. It spends a
//! declared [`Budget`] per pass, stops when the budget is gone, and records
//! what it did not reach as partial coverage rather than letting a truncated
//! window read as a quiet market.
//!
//! Fewer mints, collected completely, beats every mint collected partly: a tape
//! missing four trades in five is not a tape, while a tape for the ten busiest
//! coins is a true statement about those ten. [`SHORTLIST`] is set from that
//! reasoning and [`Budget::PER_PASS`] enforces it whatever the market does.
//!
//! # Its own cursor
//!
//! The follow cursor (`radar_store::cursor::CURSOR_FILE`) is one file per
//! store, and two writers of one file fight over it — `radar-follow` already
//! owns it. This collector never touches that file: it keeps its own cursor
//! in [`SCOPE_DIR`], a subdirectory `radar_store::Reader`/`Writer` never look
//! inside, using the same atomic read/write the follow cursor uses so a torn
//! write here is exactly as safe as it is there.
//!
//! # What a covered window can honestly claim
//!
//! Each pass ranks mints by recent activity and only asks for trades on that
//! shortlist — the same design the live `/v1/market/coins` endpoint always
//! used, just run once and stored rather than run per request. A mint outside
//! the window's activity cut has no row here, and that is **not** the same
//! fact as "this mint had zero trades": it means the shortlist did not reach
//! it. The [`Coverage`](radar_store::Coverage) record this collector writes
//! says the window's shortlist was scanned; it does not attest every mint on
//! Solana. `radar_backfill::coverage`'s own doc comment makes the same kind of
//! disclosure for pump.fun being the store's only venue, and this is that
//! caveat's shape applied to a rank cut instead of a venue.

use radar_store::{Completion, Coverage, MarketSide, MarketTrade, ObservedSlots, Table};

use crate::QueryError;
use radar_types::{Address, Signature, Slot};

use crate::market::fold;

/// The decoder that produced the rows -- this crate's own version, the same
/// convention `crate::coverage::DECODER_VERSION` uses.
const DECODER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// This collector's coverage source.
pub const SOURCE: &str = "cryptohouse:market:tape";

/// The subdirectory this collector's own cursor lives under, so it can never
/// collide with `radar-follow`'s `.follow-cursor` at the store root.
pub const SCOPE_DIR: &str = "market-tape";

/// How many of a window's busiest mints a pass collects.
///
/// Ten, not two hundred, and the difference is the whole design. Two hundred
/// mints produced 20,710 trades in a two-minute window when measured on
/// 2026-09-12, which is twenty-one queries at the endpoint's thousand-row cap
/// before any of them is retried — five times what the quota allows per hour,
/// let alone per pass.
///
/// Ten leaves a pass able to finish inside [`Budget::PER_PASS`] in an ordinary
/// market, so the coins the terminal shows have a **complete** tape rather than
/// whichever fifth of it the budget happened to reach. A partial tape is not a
/// smaller tape; it is a wrong one, because the trades it drops are invisible
/// rather than marked.
pub const SHORTLIST: usize = 10;

/// How many recent trades a pass keeps for each shortlisted mint.
///
/// Ten mints at ninety trades each is 900 rows, under the endpoint's
/// thousand-row cap, so the trades query returns in one request and
/// `narrowing_fetch` never halves it. That is what makes a pass's cost
/// predictable rather than a function of how busy the market happens to be.
///
/// Ninety is also about what a tape panel shows before a reader scrolls, so
/// the bound costs nothing visible while removing the failure mode entirely.
pub const TRADES_PER_MINT: usize = 90;

/// A hard ceiling on the queries one pass may issue.
///
/// **Not a hint.** [`radar_backfill::narrowing_fetch`] halves a window that
/// exceeds the row cap and halves the halves, so the number of queries a single
/// call makes is decided by how busy the market is, not by anything in this
/// file. Without a counter in the one place every query passes through, a busy
/// two minutes silently spends an hour's quota — which is what was measured
/// before this existed.
///
/// Exhaustion is deliberately reported as a [`QueryError::Server`] whose
/// [`QueryError::should_narrow`] is **false**: narrowing further would issue
/// more queries to escape a limit on issuing queries. The pass sees an ordinary
/// failure, keeps whatever it already collected, and records the rest as
/// partial coverage.
pub struct Budget {
    remaining: std::cell::Cell<u32>,
}

impl Budget {
    /// Queries one pass may spend.
    ///
    /// The quota is 120 an hour, shared with `radar-follow` and the hourly
    /// `--outcomes` cron on the same IP. Reserving 40 for those leaves 80, and
    /// at one pass every [`PASS_INTERVAL_SECONDS`] that is this many per pass.
    /// Stated as the division rather than the answer so the two cannot drift.
    pub const PER_PASS: u32 = (80 * PASS_INTERVAL) / 3_600;

    /// A budget for one pass.
    #[must_use]
    pub fn new(queries: u32) -> Self {
        Self {
            remaining: std::cell::Cell::new(queries),
        }
    }

    /// Whether another query may be issued, spending one if so.
    fn take(&self) -> bool {
        let left = self.remaining.get();
        if left == 0 {
            return false;
        }
        self.remaining.set(left - 1);
        true
    }

    /// How many queries are left unspent.
    #[must_use]
    pub fn remaining(&self) -> u32 {
        self.remaining.get()
    }

    /// Wraps a query runner so it refuses once the budget is gone.
    ///
    /// # Errors
    ///
    /// Returns the exhaustion error rather than calling `run`, once
    /// [`Self::PER_PASS`] queries have been issued.
    pub fn guard<T>(
        &self,
        run: impl Fn(&str) -> Result<Vec<T>, QueryError>,
        sql: &str,
    ) -> Result<Vec<T>, QueryError> {
        if self.take() {
            run(sql)
        } else {
            Err(Self::exhausted())
        }
    }

    /// The error a spent budget reports.
    ///
    /// `Server` rather than `Transport` because nothing failed to reach
    /// anywhere, and its text carries no row-cap or timeout marker, so
    /// `should_narrow` is false and the halving stops here.
    #[must_use]
    pub fn exhausted() -> QueryError {
        QueryError::Server(
            "market-tape query budget spent for this pass; the rest of the window is not collected"
                .to_owned(),
        )
    }
}

/// Seconds between passes, as the unsigned count [`Budget::PER_PASS`] divides.
pub const PASS_INTERVAL: u32 = 300;

/// The same interval where a signed one is wanted -- window arithmetic works
/// in `i64` epoch seconds. Derived, never a second literal, so the budget and
/// the window cannot come to mean different intervals.
pub const PASS_INTERVAL_SECONDS: i64 = PASS_INTERVAL as i64;

/// A pass cannot run at all on fewer than two queries — one for the window's
/// candidates, one for their trades. Checked at compile time rather than in a
/// test, because it is a property of the constants rather than of any
/// behaviour: if someone shortens [`PASS_INTERVAL_SECONDS`] far enough that
/// the division yields one, the build should stop rather than a pass silently
/// collecting candidates and no tape.
const _: () = assert!(
    Budget::PER_PASS >= 2,
    "a pass needs one query for candidates and one for trades"
);

/// Groups a batched query's rows by mint and folds each group on its own.
///
/// **Grouping has to happen before folding, not inside it.**
/// [`fold::fold_tape`]'s pool detection assumes every row it sees belongs to
/// one mint's activity — mixing two mints' transfers into one call would let
/// one mint's pool compete with another's for the same frequency count, and
/// [`super::market::query::trades_query`] batches exactly that many mints into
/// one result set to fit the query budget. So this is the seam that undoes the
/// batching before the existing, unmodified fold sees it.
///
/// A row whose `mint`, `signature` or `slot` does not parse is dropped rather
/// than guessed at, the same discipline [`crate::extract::events_from_rows`]
/// applies to a chain event — those three are never null by construction in
/// the stored schema, so a bad value in one is not a fact this collector may
/// invent. `quote_mint` and `trader` are optional and a parse failure there is
/// read as absent, never as a reason to drop an otherwise-good trade.
#[must_use]
pub fn fold_market_trades(rows: &[fold::TapeRow]) -> Vec<MarketTrade> {
    let mut by_mint: std::collections::BTreeMap<&str, Vec<fold::TapeRow>> =
        std::collections::BTreeMap::new();
    for row in rows {
        by_mint
            .entry(row.mint.as_str())
            .or_default()
            .push(row.clone());
    }

    let mut out = Vec::new();
    for (mint, group) in by_mint {
        let Ok(mint) = mint.parse::<Address>() else {
            continue;
        };
        for trade in fold::fold_tape(&group) {
            let Ok(signature) = trade.signature.parse::<Signature>() else {
                continue;
            };
            out.push(MarketTrade {
                mint,
                ts: trade.ts,
                slot: Slot(trade.slot),
                signature,
                side: match trade.side {
                    fold::Side::Buy => MarketSide::Buy,
                    fold::Side::Sell => MarketSide::Sell,
                    fold::Side::Unknown => MarketSide::Unknown,
                },
                token_amount: trade.token_amount,
                quote_amount: trade.quote_amount,
                quote_mint: trade.quote_mint.as_deref().and_then(|m| m.parse().ok()),
                price: trade.price,
                trader: trade.trader.as_deref().and_then(|a| a.parse().ok()),
            });
        }
    }
    out
}

/// The highest slot among a pass's trades, or `None` when it collected none.
///
/// The market-tape analogue of `crate::coverage::highest_slot`, kept separate
/// because it walks `&[MarketTrade]` rather than `&[Event]` -- there is no
/// envelope here to read a slot off through a shared accessor.
#[must_use]
pub fn highest_slot(trades: &[MarketTrade]) -> Option<Slot> {
    trades.iter().map(|t| t.slot).max()
}

/// The one coverage record a pass writes.
///
/// One record, not one per mint: the window's shortlist is a single scan of
/// `Table::MarketTrades`, the same way a lifecycle window is a single scan of
/// launches. `filter` stays `None` — see the module doc comment for exactly
/// what that can and cannot be read to claim.
#[must_use]
pub fn coverage_record(status: Completion, trades: &[MarketTrade], recorded_at: Slot) -> Coverage {
    Coverage {
        recorded_at,
        table: Table::MarketTrades,
        filter: None,
        observed: ObservedSlots::over(trades.iter().map(|t| t.slot)),
        source: SOURCE.to_owned(),
        decoder_version: DECODER_VERSION.to_owned(),
        status,
    }
}

#[cfg(test)]
mod tests {

    /// A budget spends down and then refuses.
    #[test]
    fn a_budget_spends_down_and_then_refuses() {
        let budget = Budget::new(2);
        assert!(
            budget
                .guard(|_| Ok::<Vec<u8>, QueryError>(vec![1]), "a")
                .is_ok()
        );
        assert_eq!(budget.remaining(), 1);
        assert!(
            budget
                .guard(|_| Ok::<Vec<u8>, QueryError>(vec![2]), "b")
                .is_ok()
        );
        assert_eq!(budget.remaining(), 0);
        let refused = budget
            .guard(|_| Ok::<Vec<u8>, QueryError>(vec![3]), "c")
            .expect_err("the third is refused");
        assert!(refused.to_string().contains("not collected"));
    }

    /// A spent budget does not call through.
    ///
    /// The point of the ceiling is that the query is never issued, not that its
    /// result is discarded — a `guard` that ran the closure and then reported
    /// exhaustion would spend the quota it exists to protect.
    #[test]
    fn a_spent_budget_never_calls_the_runner() {
        let budget = Budget::new(0);
        let calls = std::cell::Cell::new(0u32);
        let _ = budget.guard(
            |_| {
                calls.set(calls.get() + 1);
                Ok::<Vec<u8>, QueryError>(Vec::new())
            },
            "sql",
        );
        assert_eq!(calls.get(), 0, "the runner must not be reached");
    }

    /// Exhaustion must not look like a window that needs narrowing.
    ///
    /// **This is the load-bearing one.** `narrowing_fetch` halves on any error
    /// whose `should_narrow` is true, and each half is another query. If a
    /// spent budget said "too many rows", the recursion would issue more
    /// queries to escape a limit on issuing queries — spending far more than
    /// the ceiling, which is the bug this whole type exists to prevent.
    #[test]
    fn an_exhausted_budget_stops_the_halving_rather_than_feeding_it() {
        assert!(
            !Budget::exhausted().should_narrow(),
            "a spent budget must not be mistaken for a wide window"
        );
    }

    /// A budget bounds a real narrowing fetch, however wide the window.
    ///
    /// The fake refuses every window as over the row cap, so without a budget
    /// `narrowing_fetch` would halve until the four-second floor -- hundreds of
    /// queries for a wide enough window. With one, it stops at the ceiling and
    /// reports the failure, and the pass records partial coverage.
    #[test]
    fn a_budget_bounds_a_narrowing_fetch_however_wide_the_window() {
        let budget = Budget::new(5);
        let issued = std::cell::Cell::new(0u32);
        let err = crate::narrowing_fetch(
            &|sql| {
                budget.guard(
                    |_| {
                        issued.set(issued.get() + 1);
                        Err::<Vec<u8>, _>(QueryError::Server(
                            "Code: 396 (TOO_MANY_ROWS_OR_BYTES)".to_owned(),
                        ))
                    },
                    sql,
                )
            },
            0,
            86_400,
            0,
            4,
            std::time::Duration::ZERO,
            &|f, t| format!("{f}..{t}"),
        )
        .expect_err("nothing fits, so the fetch fails");
        assert_eq!(issued.get(), 5, "exactly the budget, and not one more");
        assert_eq!(budget.remaining(), 0);
        assert!(!err.should_narrow());
    }

    /// The per-pass ceiling is the hourly allowance divided by the pass rate.
    ///
    /// Asserted rather than left to arithmetic in a comment, because the
    /// previous version of this collector stated a per-hour figure that was
    /// wrong by a factor of sixteen and nothing contradicted it.
    #[test]
    fn the_per_pass_ceiling_stays_inside_the_hourly_allowance() {
        let passes_per_hour = 3_600 / PASS_INTERVAL_SECONDS;
        let spent = i64::from(Budget::PER_PASS) * passes_per_hour;
        assert!(
            spent <= 80,
            "{spent} queries an hour leaves nothing for radar-follow or --outcomes"
        );
    }
    use super::*;

    /// `seed` becomes a valid base58 signature via `Signature::new`, since
    /// `fold_market_trades` parses `sig` and a placeholder string like `"a1"`
    /// is not valid base58 -- it would be dropped as malformed, silently
    /// emptying every test that used one.
    fn row(mint: &str, seed: u8, quote_mint: &str, quote_value: &str) -> fold::TapeRow {
        fold::TapeRow {
            mint: mint.to_owned(),
            ts: "2026-09-11 17:35:00.000000".to_owned(),
            slot: "441251921".to_owned(),
            sig: Signature::new([seed; 64]).to_string(),
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

    const MINT_A: &str = "5NfV2sy8DqXamLvYEE4LcTWzGqZc5Emv4bqqhVDWpump";
    const MINT_B: &str = "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM";
    const SOL: &str = "So11111111111111111111111111111111111111112";

    #[test]
    fn a_batched_result_is_split_by_mint_before_folding() {
        // Re-applying the bug this seam exists to avoid: folding the combined
        // rows in one call would let mint A's pool and mint B's pool compete
        // for the same frequency count, since `detect_pool` has no idea two
        // mints are present.
        let rows = vec![row(MINT_A, 1, SOL, "10000"), row(MINT_B, 2, SOL, "20000")];
        let trades = fold_market_trades(&rows);
        assert_eq!(trades.len(), 2);
        let mints: std::collections::BTreeSet<String> =
            trades.iter().map(|t| t.mint.to_string()).collect();
        assert!(mints.contains(MINT_A));
        assert!(mints.contains(MINT_B));
    }

    #[test]
    fn each_mints_pool_is_detected_from_its_own_rows_only() {
        // Three trades against the same pool for mint A, one lone trade for
        // mint B. If the two mints' rows were folded together, mint A's pool
        // account would dominate the frequency count across both, and mint
        // B's single trade -- which alone cannot establish a pool -- would
        // wrongly inherit a side from an account it never actually traded
        // against.
        let mut a_rows: Vec<fold::TapeRow> = (0..3u8)
            .map(|i| {
                let mut r = row(MINT_A, i, SOL, "10000");
                r.token_destination = format!("TRADER-{i}");
                r
            })
            .collect();
        a_rows.push(row(MINT_B, 200, SOL, "5000"));

        let trades = fold_market_trades(&a_rows);
        let a_trades: Vec<_> = trades
            .iter()
            .filter(|t| t.mint.to_string() == MINT_A)
            .collect();
        let b_trades: Vec<_> = trades
            .iter()
            .filter(|t| t.mint.to_string() == MINT_B)
            .collect();
        assert_eq!(a_trades.len(), 3);
        assert!(
            a_trades.iter().all(|t| t.side == MarketSide::Buy),
            "mint A's own pool is an unambiguous plurality of its own rows"
        );
        assert_eq!(b_trades.len(), 1);
        assert_eq!(
            b_trades[0].side,
            MarketSide::Unknown,
            "a single trade cannot establish a pool on its own"
        );
    }

    #[test]
    fn a_row_naming_an_unparseable_mint_is_dropped_not_guessed() {
        let rows = vec![row("not-a-real-mint", 1, SOL, "10000")];
        assert!(fold_market_trades(&rows).is_empty());
    }

    #[test]
    fn a_quoteless_trade_still_survives_the_grouping_seam() {
        let rows = vec![row(MINT_A, 1, "", "0")];
        let trades = fold_market_trades(&rows);
        assert_eq!(trades.len(), 1);
        assert_eq!(trades[0].quote_amount, None);
        assert_eq!(trades[0].price, None);
    }

    #[test]
    fn a_pass_that_collects_nothing_records_no_slot_at_all() {
        let record = coverage_record(Completion::Complete, &[], Slot(500));
        assert_eq!(
            record.observed,
            ObservedSlots::Nothing,
            "a window that ran and found nothing must be distinguishable from one nobody ran"
        );
        assert_eq!(record.status, Completion::Complete);
        assert_eq!(record.table, Table::MarketTrades);
        assert_eq!(record.filter, None);
    }

    #[test]
    fn a_pass_that_collects_something_spans_exactly_its_own_slots() {
        let rows = vec![row(MINT_A, 1, SOL, "10000"), row(MINT_B, 2, SOL, "20000")];
        let mut trades = fold_market_trades(&rows);
        // Force distinct slots so the span is not vacuously a single point,
        // found by mint rather than by index so this does not depend on
        // whatever internal order grouping happens to produce.
        for t in &mut trades {
            t.slot = if t.mint.to_string() == MINT_A {
                Slot(100)
            } else {
                Slot(400)
            };
        }
        assert_eq!(highest_slot(&trades), Some(Slot(400)));
        let record = coverage_record(Completion::Complete, &trades, Slot(500));
        assert_eq!(
            record.observed,
            ObservedSlots::Span {
                from: Slot(100),
                to: Slot(400)
            }
        );
    }
}
