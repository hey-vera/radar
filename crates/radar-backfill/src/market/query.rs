// SPDX-License-Identifier: Apache-2.0
//! The SQL this module sends to CryptoHouse.
//!
//! Split from [`super::fold`] on the same principle [`crate::extract`] already
//! uses: everything here is a pure function from parameters to a query string,
//! so it is tested without a network, and everything that turns a row into a
//! domain type lives on the other side of the split where a fixture can
//! exercise it.
//!
//! # The technique, and the bugs in the sketch it came from
//!
//! A swap on any venue is one transaction in which the target mint moves and a
//! quote asset — wrapped SOL, USDC or USDT — moves too. The sketch this was
//! built from matched wrapped SOL with `LIKE 'So1111...%'`, which also matches
//! `So11111111111111111111111111111111111111111` — a real, active, unrelated
//! mint one character longer than wrapped SOL, confirmed live against
//! CryptoHouse on 2026-09-11. [`crate::extract::QUOTE_MINTS`] already excludes
//! it by exact match, for the same reason, so the quote set here is that list
//! rather than a pattern.
//!
//! It also summed values that had not been divided by `decimals`, dropped a
//! trade outright when the quote leg's join found nothing, and looked only at
//! wrapped SOL. Every one of those is fixed below: the query joins with a
//! **`LEFT JOIN`**, so a transaction whose quote leg is missing is not
//! discarded — [`super::fold`] is the layer that turns "no leg" into a `None`
//! price rather than a zero one — the quote side accepts any of the three
//! assets, and every raw amount keeps its `decimals` beside it so nothing is
//! displayed before it is adjusted.
//!
//! # `any()` picked an arbitrary leg, not the trade's real ends
//!
//! A swap moves the target mint **several times in one transaction** — trader
//! to pool, pool to trader, plus fee and routing legs — so
//! `any(source)`/`any(destination)`/`any(authority)` returned whichever leg
//! ClickHouse happened to visit first, and `token_source` and
//! `token_destination` could come from *different* transfers entirely.
//! [`super::fold::detect_pool`] then saw no account on a consistent side,
//! never cleared its threshold, and every row folded to
//! [`super::fold::Side::Unknown`]. Confirmed live on 2026-09-11: querying
//! `solana.token_transfers` directly for one busy mint returned real
//! `source`, `destination` and `authority` on every row, with one account
//! appearing on every single trade — the pool, which `any()` was discarding
//! in favour of an arbitrary sibling leg.
//!
//! [`trades_query`] fixes this with **net flow per account**, not an
//! arbitrary leg: every transfer of the target mint contributes a negative
//! delta to its source account and a positive delta to its destination, and
//! the two accounts with the most negative and most positive net flow across
//! the whole transaction are its real ends — deterministically, however many
//! intermediate legs it carried. A transaction whose net flows cancel exactly
//! (every account nets to zero) has no real ends, and is reported as `''` for
//! both rather than an arbitrary account picked off an all-zero column —
//! [`super::fold::TapeRow`]'s existing convention for "no leg", which
//! `detect_pool` and `side_and_trader` already read correctly with no change
//! to either.

use crate::extract::QUOTE_MINTS;

/// The quote mints, as a SQL `IN (...)` list.
///
/// Exact match, never `LIKE`. See the module comment for the mint a pattern
/// match would have pulled in.
fn quote_list() -> String {
    QUOTE_MINTS
        .iter()
        .map(|m| format!("'{m}'"))
        .collect::<Vec<_>>()
        .join(",")
}

/// One transaction's token leg outer-joined against its quote leg, over a
/// bounded time window, for every mint named in `mints` in a single round
/// trip.
///
/// **Batched, not one query per mint.** CryptoHouse permits 120 queries an
/// hour per IP and that budget is shared with `radar-follow` and the hourly
/// `--outcomes` cron, so a per-mint query does not fit any traffic level a
/// collector could run at — this is the change that makes it fit. `mint IN
/// (...)` costs one round trip for the whole shortlist, and the output carries
/// [`fold::TapeRow::mint`](super::fold::TapeRow::mint) so a caller can split
/// the result back into one row set per mint before folding — `super::fold`'s
/// pool detection assumes every row it sees belongs to one mint's activity,
/// so that split has to happen upstream of it, not inside it.
///
/// `LEFT JOIN`: a transaction that moved one of `mints` but has no matching
/// row in the quote CTE still appears, with every `quote_*` column coming back
/// as ClickHouse's default for its type (`''` for the mints and authorities,
/// `'0'` for the summed amounts) rather than being dropped. [`super::fold`]
/// treats an empty `quote_mint` as "no quote leg found", never as a quote leg
/// worth zero.
///
/// Bounded on `block_timestamp` at both ends, which is what the table is
/// partitioned by — an unbounded `mint` filter alone scans the whole table
/// (measured at 16.42 billion rows for one mint's lifetime) and is refused by
/// the endpoint outright.
///
/// # The `ends` CTE
///
/// `flow` turns every transfer of one of `mints` into a signed, per-account
/// delta — negative at its `source`, positive at its `destination` — with an
/// empty `source` or `destination` excluded rather than counted as an
/// account (an empty string is not one, the same guard
/// [`super::fold::fold_holders`] applies to `MintTo`/`Burn`). `net` sums those
/// deltas to one row per `(mint, tx_signature, account)`, carrying the
/// largest authority seen on that account's own outgoing legs (`max`, not
/// `any`, so a real address wins over the empty string a destination-only
/// account contributes). `ends` then picks the two accounts with the most
/// negative and most positive net flow per transaction — the trade's real
/// ends, however many intermediate legs it carried — and reports both as `''`
/// when they cancel exactly, so no account is invented for a column that
/// never had a direction. `argMin`/`argMax` for `token_source`/`token_destination`
/// and `sender_authority` share the same sort key (`net`) inside one `ends`
/// row, so the authority reported is the one belonging to the same account
/// named as the source.
///
/// # Panics
///
/// Never on `mints` content — every entry reaching this function has already
/// been through [`radar_types::Address::from_str`], so it cannot carry a quote
/// or a statement terminator. Panics only if `mints` is empty, the same
/// caller-bug guard [`coin_prices_query`] uses: an empty `IN ()` is invalid SQL
/// and asking CryptoHouse to reject it would waste a round trip finding that
/// out.
///
/// # Why it is bounded per mint
///
/// **Unbounded, this query cannot be collected inside the quota.** Measured
/// 2026-09-12: two hundred mints traded 20,710 times in two minutes, and
/// CryptoHouse returns at most a thousand rows, so
/// [`radar_backfill::narrowing_fetch`] halved the window fifteen times in a
/// single pass — about thirty-two queries where the collector's own comment
/// claimed two. Narrowed to ten mints over five minutes it still exhausted a
/// six-query budget without returning a single trade.
///
/// `LIMIT n BY t.mint` is ClickHouse's per-group limit: the most recent `n`
/// trades **for each mint**, rather than the most recent `n` overall. The
/// distinction is the point — one frantic mint would otherwise fill the whole
/// result and the other nine would appear to have stopped trading, which is a
/// silence that looks exactly like a fact.
///
/// # What this still cannot tell apart
///
/// **A transaction that moves the mint and a quote asset is not necessarily a
/// swap.** A liquidity deposit, a bundled airdrop, a treasury movement paired
/// with a fee, or an arbitrage leg all have that shape, and this query prices
/// every one of them. Netting removed the multiple-counting; it did not add a
/// way to ask *why* the two assets moved, and nothing in `token_transfers`
/// carries that -- it would need the venue's own instruction decoded, which is
/// the per-venue work this technique exists to avoid.
///
/// The visible consequence is that a window's first and last priced fill can
/// be several orders of magnitude apart, so a percentage change computed from
/// them is noisy in a way a real venue's chart is not. That is a property of a
/// venue-agnostic tape, not a defect to be tuned away by discarding outliers:
/// dropping the fills that look wrong would also drop the real ones, and a
/// filtered tape presented as a complete one is the failure this module is
/// most careful about. Callers that need a robust level should say which
/// estimator they used.
///
/// # Why both legs are netted, never summed
///
/// **A swap moves each asset more than once.** The trader sends the quote
/// asset to the pool, the pool sends the mint back, a fee leg moves more of
/// the quote, and a routed swap repeats the whole shape at every hop. Summing
/// a transaction's transfers of one asset therefore counts the same value
/// several times, and the price -- one sum divided by the other -- is wrong by
/// however many legs the route happened to use.
///
/// Worse, the quote side picked its mint and its `decimals` with two
/// independent `any()` aggregates. A transaction touching both USDC and
/// wrapped SOL could report the USDC mint beside wrapped SOL's nine decimals,
/// scaling the amount by a thousand. Observed 2026-09-12 on a live store: a
/// meme coin priced at $2,923 with a stated change of -96,101 per cent, and a
/// chart whose axis those outliers flattened into a single spike.
///
/// So both sides net per account and take the largest single gain, which is
/// the amount that actually changed hands however many legs carried it. The
/// quote side additionally groups by mint before choosing, so `quote_mint`,
/// `quote_value` and `quote_decimals` are guaranteed to describe **the same
/// asset** -- they are selected together by `argMax` over the same ordering,
/// not picked one at a time.
///
/// This makes the result a **recent-trades sample, not a complete window**, and
/// the caller must record it as one. A tape showing the last hundred trades is
/// what a trading screen wants; a window claiming completeness it does not have
/// is what this repository exists not to ship.
#[must_use]
pub fn trades_query(mints: &[String], from: &str, to: &str, per_mint: usize) -> String {
    assert!(!mints.is_empty(), "trades_query needs at least one mint");
    assert!(per_mint > 0, "a tape of zero trades per mint is not a tape");
    let list = mints
        .iter()
        .map(|m| format!("'{m}'"))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "WITH t AS (\
           SELECT mint, block_timestamp, block_slot, tx_signature, \
                  sum(value) AS token_value, any(decimals) AS token_decimals \
           FROM solana.token_transfers \
           WHERE mint IN ({list}) AND block_timestamp >= '{from}' AND block_timestamp < '{to}' \
           GROUP BY mint, block_timestamp, block_slot, tx_signature\
         ), flow AS (\
           SELECT mint, tx_signature, source AS account, -value AS delta, \
                  authority AS leg_authority \
           FROM solana.token_transfers \
           WHERE mint IN ({list}) AND block_timestamp >= '{from}' AND block_timestamp < '{to}' \
             AND source != ''\
           UNION ALL \
           SELECT mint, tx_signature, destination AS account, value AS delta, \
                  '' AS leg_authority \
           FROM solana.token_transfers \
           WHERE mint IN ({list}) AND block_timestamp >= '{from}' AND block_timestamp < '{to}' \
             AND destination != ''\
         ), net AS (\
           SELECT mint, tx_signature, account, sum(delta) AS net, \
                  max(leg_authority) AS authority \
           FROM flow \
           GROUP BY mint, tx_signature, account\
         ), ends AS (\
           SELECT mint, tx_signature, min(net) AS min_net, max(net) AS max_net, \
                  argMin(account, net) AS sender, argMax(account, net) AS receiver, \
                  argMin(authority, net) AS sender_authority \
           FROM net \
           GROUP BY mint, tx_signature\
         ), qflow AS (\
           SELECT mint, tx_signature, source AS account, -value AS delta, \
                  decimals, authority AS leg_authority \
           FROM solana.token_transfers \
           WHERE mint IN ({quotes}) AND block_timestamp >= '{from}' AND block_timestamp < '{to}' \
             AND source != ''\
           UNION ALL \
           SELECT mint, tx_signature, destination AS account, value AS delta, \
                  decimals, '' AS leg_authority \
           FROM solana.token_transfers \
           WHERE mint IN ({quotes}) AND block_timestamp >= '{from}' AND block_timestamp < '{to}' \
             AND destination != ''\
         ), qnet AS (\
           SELECT mint, tx_signature, account, sum(delta) AS net, \
                  any(decimals) AS dec, max(leg_authority) AS authority \
           FROM qflow \
           GROUP BY mint, tx_signature, account\
         ), qmint AS (\
           SELECT mint, tx_signature, max(net) AS gross, any(dec) AS dec, \
                  argMin(authority, net) AS payer \
           FROM qnet \
           GROUP BY mint, tx_signature\
         ), s AS (\
           SELECT tx_signature, argMax(mint, gross) AS quote_mint, max(gross) AS quote_value, \
                  argMax(dec, gross) AS quote_decimals, argMax(payer, gross) AS quote_authority \
           FROM qmint \
           GROUP BY tx_signature\
         ) \
         SELECT t.mint AS mint, toString(t.block_timestamp) AS ts, toString(t.block_slot) AS slot, \
                t.tx_signature AS sig, \
                toString(greatest(ifNull(ends.max_net, 0), 0)) AS token_value, \
                toString(t.token_decimals) AS token_decimals, \
                if(ifNull(ends.min_net, 0) = 0 AND ifNull(ends.max_net, 0) = 0, '', ends.sender) \
                  AS token_source, \
                if(ifNull(ends.min_net, 0) = 0 AND ifNull(ends.max_net, 0) = 0, '', ends.receiver) \
                  AS token_destination, \
                if(ifNull(ends.min_net, 0) = 0 AND ifNull(ends.max_net, 0) = 0, '', ends.sender_authority) \
                  AS token_authority, \
                toString(s.quote_value) AS quote_value, toString(s.quote_decimals) AS quote_decimals, \
                s.quote_mint AS quote_mint, s.quote_authority AS quote_authority \
         FROM t \
         LEFT JOIN ends ON t.mint = ends.mint AND t.tx_signature = ends.tx_signature \
         LEFT JOIN s ON t.tx_signature = s.tx_signature \
         ORDER BY t.mint, t.block_timestamp DESC \n         LIMIT {per_mint} BY t.mint",
        quotes = quote_list()
    )
}

/// Raw transfers for one mint, quote legs excluded, over a bounded window —
/// what [`super::fold::fold_holders`] folds into balances.
///
/// Includes `MintTo`, `Burn` and their checked variants so supply is
/// conserved rather than only ever growing: a fold that counted transfers
/// alone would show a burned balance as still held.
#[must_use]
pub fn holder_transfers_query(mint: &str, from: &str, to: &str) -> String {
    format!(
        "SELECT toString(block_timestamp) AS ts, source, destination, \
                toString(value) AS value, toString(decimals) AS decimals, transfer_type \
         FROM solana.token_transfers \
         WHERE mint = '{mint}' AND block_timestamp >= '{from}' AND block_timestamp < '{to}' \
         ORDER BY block_timestamp ASC"
    )
}

/// Which mints traded at all in a window, ranked by transaction count.
///
/// One row per mint however many transfers the window holds — the aggregate
/// is cheap the same way [`radar_backfill::prices`] is, because the cost is
/// the window scan and barely depends on how many mints come back. This is
/// the shortlist [`coin_prices_query`] then prices; pricing every mint that
/// moved without first ranking them would ask CryptoHouse for as many
/// candle-style joins as there are mints in the window, most of which nobody
/// asked to see.
#[must_use]
pub fn coin_candidates_query(from: &str, to: &str, limit: usize) -> String {
    format!(
        "SELECT mint, toString(count(DISTINCT tx_signature)) AS tx_count, \
                toString(sum(value)) AS token_volume, \
                toString(any(decimals)) AS token_decimals \
         FROM solana.token_transfers \
         WHERE block_timestamp >= '{from}' AND block_timestamp < '{to}' \
           AND mint NOT IN ({quotes}) \
         GROUP BY mint \
         ORDER BY tx_count DESC \
         LIMIT {limit}",
        quotes = quote_list()
    )
}

/// The first and last priced fill for each of a shortlist of mints.
///
/// Aggregated all the way down to one row per mint with `argMin`/`argMax`
/// over time, so the per-trade rows the two CTEs build are never part of the
/// result the row cap counts — only the final `GROUP BY mint` is. `mints`
/// must already be shortlisted: this is quadratic in nothing, but a query
/// naming every mint that ever moved would be a very long `IN (...)` for no
/// reason [`coin_candidates_query`] does not already serve.
///
/// # Panics
///
/// Never on `mints` content — every entry has already been through
/// [`radar_types::Address::from_str`] by the caller, so it cannot carry a
/// quote or a statement terminator. Panics only if `mints` is empty, which is
/// a caller bug: an empty `IN ()` is invalid SQL and asking CryptoHouse to
/// reject it would waste a round trip finding that out.
#[must_use]
pub fn coin_prices_query(mints: &[String], from: &str, to: &str) -> String {
    assert!(!mints.is_empty(), "coin_prices_query needs a shortlist");
    let list = mints
        .iter()
        .map(|m| format!("'{m}'"))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "WITH t AS (\
           SELECT mint, tx_signature, block_timestamp AS ts, \
                  sum(value) AS token_value, any(decimals) AS token_decimals \
           FROM solana.token_transfers \
           WHERE mint IN ({list}) AND block_timestamp >= '{from}' AND block_timestamp < '{to}' \
           GROUP BY mint, tx_signature, ts\
         ), s AS (\
           SELECT tx_signature, sum(value) AS quote_value, any(decimals) AS quote_decimals, \
                  any(mint) AS quote_mint \
           FROM solana.token_transfers \
           WHERE mint IN ({quotes}) AND block_timestamp >= '{from}' AND block_timestamp < '{to}' \
           GROUP BY tx_signature\
         ), joined AS (\
           SELECT t.mint AS mint, t.ts AS ts, t.token_value AS token_value, \
                  t.token_decimals AS token_decimals, s.quote_value AS quote_value, \
                  s.quote_decimals AS quote_decimals, s.quote_mint AS quote_mint \
           FROM t INNER JOIN s ON t.tx_signature = s.tx_signature\
         ) \
         SELECT mint, toString(count()) AS trade_count, \
                toString(argMin(token_value, ts)) AS first_token_value, \
                toString(argMin(token_decimals, ts)) AS first_token_decimals, \
                toString(argMin(quote_value, ts)) AS first_quote_value, \
                toString(argMin(quote_decimals, ts)) AS first_quote_decimals, \
                toString(argMax(token_value, ts)) AS last_token_value, \
                toString(argMax(token_decimals, ts)) AS last_token_decimals, \
                toString(argMax(quote_value, ts)) AS last_quote_value, \
                toString(argMax(quote_decimals, ts)) AS last_quote_decimals, \
                any(quote_mint) AS quote_mint, \
                toString(sum(quote_value)) AS quote_volume, \
                toString(any(quote_decimals)) AS quote_volume_decimals \
         FROM joined \
         GROUP BY mint",
        quotes = quote_list()
    )
}

/// The earliest published metadata for a mint — name, symbol, creator.
///
/// `solana.tokens` carries no `decimals` column; that comes back from
/// [`trades_query`] or [`holder_transfers_query`] instead, when either has run
/// recently enough to have seen the mint. Unbounded on time deliberately:
/// metadata is published once, near a token's creation, and a window a caller
/// would have to guess defeats the purpose of asking for it by mint. Measured
/// against the live endpoint on 2026-09-11 at 0.6-3s for a single `mint`
/// lookup — the table is 30 million rows, not the multi-billion-row transfer
/// log the other queries are bounded against.
#[must_use]
pub fn token_metadata_query(mint: &str) -> String {
    format!(
        "SELECT toString(block_timestamp) AS ts, name, symbol, uri, creators, update_authority \
         FROM solana.tokens WHERE mint = '{mint}' ORDER BY block_timestamp ASC LIMIT 1"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINT: &str = "5NfV2sy8DqXamLvYEE4LcTWzGqZc5Emv4bqqhVDWpump";

    #[test]
    fn the_quote_side_is_an_exact_list_never_a_pattern() {
        let sql = trades_query(
            &[MINT.to_owned()],
            "2026-09-11 17:00:00",
            "2026-09-11 17:05:00",
            90,
        );
        // The bug: `LIKE 'So1111...%'` matches this real, unrelated mint too
        // -- one character longer than wrapped SOL, confirmed live.
        assert!(
            !sql.contains("LIKE"),
            "must not pattern-match the quote leg"
        );
        for quote in QUOTE_MINTS {
            assert!(sql.contains(quote), "missing quote mint {quote}: {sql}");
        }
    }

    #[test]
    fn the_trade_query_left_joins_so_a_missing_quote_leg_is_not_dropped() {
        let sql = trades_query(
            &[MINT.to_owned()],
            "2026-09-11 17:00:00",
            "2026-09-11 17:05:00",
            90,
        );
        assert!(
            sql.contains("LEFT JOIN"),
            "an INNER JOIN silently drops a transaction with no quote leg: {sql}"
        );
        assert!(sql.contains(MINT));
        assert!(sql.contains("2026-09-11 17:00:00"));
        assert!(sql.contains("2026-09-11 17:05:00"));
        // Decimals travel with every raw amount, never assumed.
        assert!(sql.contains("token_decimals"));
        assert!(sql.contains("quote_decimals"));
    }

    /// The bug this module's own doc comment names: `any(source)` picked an
    /// arbitrary leg of a multi-leg transaction rather than the trade's real
    /// ends, and every row folded to `Side::Unknown` as a result. The fix
    /// reads net flow per account instead, so the query must no longer carry
    /// the arbitrary form at all.
    #[test]
    fn token_ends_come_from_net_flow_never_from_an_arbitrary_leg() {
        let sql = trades_query(
            &[MINT.to_owned()],
            "2026-09-11 17:00:00",
            "2026-09-11 17:05:00",
            90,
        );
        assert!(
            !sql.contains("any(source)"),
            "the arbitrary-leg bug must not come back: {sql}"
        );
        assert!(
            !sql.contains("any(destination)"),
            "the arbitrary-leg bug must not come back: {sql}"
        );
        assert_eq!(
            sql.matches("any(authority)").count(),
            0,
            "no leg of either side may be picked arbitrarily. An earlier version              of this assertion allowed one, on the grounds that the quote CTE              had \"no multi-leg ambiguity to fix\" -- which was wrong. A routed              swap moves the quote asset at every hop, and picking its mint, its              decimals and its authority with three independent any() aggregates              produced a meme coin priced at $2,923 against another coin's              decimals. Both sides net now: {sql}"
        );
        assert!(
            !sql.contains("any(mint)"),
            "quote_mint, quote_value and quote_decimals must describe the same              asset, chosen together by argMax over one ordering: {sql}"
        );
        assert!(
            sql.contains("UNION ALL"),
            "the signed per-account flow: {sql}"
        );
        assert!(
            sql.contains("argMin(account, net)") && sql.contains("argMax(account, net)"),
            "the two real ends are the most negative and most positive net flow: {sql}"
        );
    }

    /// An empty `source` or `destination` is not an account, the same guard
    /// `fold_holders` applies to a `MintTo` or a `Burn` — otherwise the
    /// biggest "sender" in a transaction with one is the empty string.
    #[test]
    fn an_empty_account_is_excluded_from_the_flow_rather_than_counted() {
        let sql = trades_query(
            &[MINT.to_owned()],
            "2026-09-11 17:00:00",
            "2026-09-11 17:05:00",
            90,
        );
        assert!(sql.contains("AND source != ''"), "{sql}");
        assert!(sql.contains("AND destination != ''"), "{sql}");
    }

    /// A transaction whose net flows cancel exactly has no real ends, and
    /// must not report an arbitrary account picked off an all-zero column --
    /// it is reported as `''`, the existing convention for "no leg", so
    /// `detect_pool` and `side_and_trader` read it correctly with no change
    /// to either.
    #[test]
    fn a_transaction_with_no_net_movement_reports_no_ends_rather_than_a_guess() {
        let sql = trades_query(
            &[MINT.to_owned()],
            "2026-09-11 17:00:00",
            "2026-09-11 17:05:00",
            90,
        );
        assert!(
            sql.contains("ends.min_net, 0) = 0") && sql.contains("ends.max_net, 0) = 0"),
            "cancellation must be checked, not assumed away: {sql}"
        );
    }

    #[test]
    fn the_trade_query_batches_every_named_mint_into_one_in_clause() {
        // The whole point of generalising this query: one round trip for a
        // shortlist of mints rather than one per mint, which is what makes the
        // collector fit inside a shared hourly quota.
        let other = "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM";
        let sql = trades_query(
            &[MINT.to_owned(), other.to_owned()],
            "2026-09-11 17:00:00",
            "2026-09-11 17:05:00",
            90,
        );
        assert!(sql.contains(MINT), "{sql}");
        assert!(sql.contains(other), "{sql}");
        // Every row says which mint it belongs to, so a caller can split the
        // batch back into one row set per mint before folding.
        assert!(sql.contains("t.mint AS mint"), "{sql}");
        assert!(sql.contains("GROUP BY mint"), "{sql}");
    }

    #[test]
    #[should_panic(expected = "at least one mint")]
    fn batching_an_empty_mint_list_is_a_caller_bug_not_a_query() {
        let _ = trades_query(&[], "2026-09-11 17:00:00", "2026-09-11 17:05:00", 90);
    }

    #[test]
    fn the_holder_query_keeps_mints_and_burns_so_supply_is_conserved() {
        let sql = holder_transfers_query(MINT, "2026-09-11 00:00:00", "2026-09-11 01:00:00");
        // No transfer_type filter: excluding MintTo or Burn here would show a
        // burned balance as still held.
        assert!(!sql.contains("transfer_type="), "{sql}");
        assert!(sql.contains("transfer_type"));
    }

    #[test]
    fn the_candidate_query_excludes_the_quote_mints_themselves() {
        let sql = coin_candidates_query("2026-09-11 17:00:00", "2026-09-11 17:05:00", 50);
        assert!(sql.contains("NOT IN"));
        for quote in QUOTE_MINTS {
            assert!(sql.contains(quote));
        }
        assert!(sql.contains("LIMIT 50"));
    }

    #[test]
    #[should_panic(expected = "shortlist")]
    fn pricing_an_empty_shortlist_is_a_caller_bug_not_a_query() {
        let _ = coin_prices_query(&[], "2026-09-11 17:00:00", "2026-09-11 17:05:00");
    }

    #[test]
    fn the_price_query_aggregates_down_to_one_row_per_mint() {
        let sql = coin_prices_query(
            &[MINT.to_owned()],
            "2026-09-11 17:00:00",
            "2026-09-11 17:05:00",
        );
        assert!(sql.contains("GROUP BY mint"));
        assert!(sql.contains("argMin"));
        assert!(sql.contains("argMax"));
        assert!(sql.contains(MINT));
    }

    #[test]
    fn the_metadata_query_has_no_time_bound() {
        // Deliberately unbounded -- see the doc comment for why this table is
        // the one exception, and the measurement backing it.
        let sql = token_metadata_query(MINT);
        assert!(!sql.contains("block_timestamp >="));
        assert!(sql.contains(MINT));
        assert!(sql.contains("LIMIT 1"));
    }
}
