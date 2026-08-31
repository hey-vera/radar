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

/// One authority seen in a launch block.
#[derive(Debug, Deserialize)]
struct AuthorityOnly {
    authority: String,
}

/// One authority, with how many launch blocks it appears in over the window.
#[derive(Debug, Deserialize)]
struct AuthorityRow {
    /// The signing wallet.
    authority: String,
    /// How many distinct launch blocks it appeared in over the window.
    launch_blocks: String,
}

/// How far back the prevalence window reaches.
///
/// Ninety minutes, because that is the window
/// [`docs/research/0012`](../../docs/research/0012-recipient-sets-cannot-recur-authorities-can.md)
/// measured the bands over, and a count taken over a different window is a
/// different quantity wearing the same unit. Changing this means re-measuring
/// the bands, not adjusting a number.
pub const PREVALENCE_WINDOW_MINUTES: u64 = radar_graph::prevalence::WINDOW_MINUTES;

/// Builds the query for the wallets that signed inside one launch block.
///
/// The same single-block read as [`query_for_shape`], returning the addresses
/// rather than a count, and bounded by `block_timestamp` for the same reason —
/// the table prunes on nothing else, so a slot filter alone reads naturally and
/// prunes nothing (ADR 0002).
///
/// Cheap. Asking the *prevalence* question per candidate instead took 32
/// seconds against the real endpoint, which is why that half is
/// [`query_for_prevalence`] and runs once for the whole pass.
#[must_use]
pub fn query_for_authorities(mint: &Address, slot: Slot, since: &str) -> String {
    format!(
        "SELECT DISTINCT authority AS authority FROM solana.token_transfers WHERE block_timestamp >= '{since}' AND block_slot = {slot} AND mint = '{mint}' AND authority != ''",
        slot = slot.get()
    )
}

/// Builds the window query for every wallet at or above the repeat floor.
///
/// Run once per pass, not once per candidate. The `HAVING` is what keeps the
/// result inside the thousand-row cap: everything below the floor classifies as
/// `Ordinary` anyway, so leaving it out loses nothing, and asking for it would
/// truncate the part that matters.
#[must_use]
pub fn query_for_prevalence() -> String {
    format!(
        "WITH first_slots AS (SELECT mint AS m, min(block_slot) AS launch_slot FROM solana.token_transfers WHERE block_timestamp >= now() - INTERVAL {window} MINUTE AND mint != '' GROUP BY mint) SELECT t.authority AS authority, toString(uniqExact(t.mint)) AS launch_blocks FROM solana.token_transfers t INNER JOIN first_slots f ON t.mint = f.m AND t.block_slot = f.launch_slot WHERE t.block_timestamp >= now() - INTERVAL {window} MINUTE AND t.authority != '' GROUP BY t.authority HAVING uniqExact(t.mint) >= {floor} ORDER BY launch_blocks DESC",
        window = PREVALENCE_WINDOW_MINUTES,
        floor = radar_graph::prevalence::REPEAT_FLOOR
    )
}

/// Reads launch-block shapes from CryptoHouse.
pub struct CryptoHouseBlocks {
    client: Client,
    /// The lower bound every query here carries.
    ///
    /// Held rather than recomputed per call so one run asks one question of the
    /// endpoint, and so a long run cannot drift its own window underneath itself.
    since: String,
}

impl CryptoHouseBlocks {
    /// Builds one against a given client, looking back from `now`.
    #[must_use]
    pub fn new(client: Client, now: &str) -> Self {
        Self {
            client,
            since: since(now),
        }
    }

    /// The lower bound in force, for a caller that wants to report it.
    #[must_use]
    pub fn window_starts(&self) -> &str {
        &self.since
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

/// How far back a launch-block query looks, in hours.
///
/// # Why this replaced a fixed date, and what that cost
///
/// This was `SINCE: &str = "2026-08-01 00:00:00"` — a **constant calendar
/// date**. Every launch-block query scanned `token_transfers` from that date to
/// the present, to find rows in a single slot.
///
/// On the day it was written that scanned nothing. A month later it scanned a
/// month, and it grew by another day every day. `token_transfers` prunes on
/// `block_timestamp` and nothing else (ADR 0002), so the width of that bound
/// *is* the cost of the query, and the cost was rising without limit.
///
/// Measured on the live instance, the share of candidates whose launch block
/// could not be read inside the server's timeout:
///
/// | period | unreadable |
/// |---|---|
/// | early runs | 2–25% |
/// | a week later | 28–40% |
/// | the day this was found | **56–68%** |
///
/// Read as a vendor problem that is a degrading dependency. It is not. It is
/// this query getting monotonically more expensive, and it had no ceiling — the
/// endpoint was being asked to scan an ever-larger haystack for a needle whose
/// location was known all along.
///
/// # Why thirty-six hours
///
/// A launch block is only ever asked about for a *candidate*, and `radar
/// consider`'s window is 216,000 slots — about twenty-four hours. So the block
/// in question is always within a day, and a bound wider than that buys nothing
/// and costs scan time on every call.
///
/// Thirty-six leaves a half-day of margin for slots that ran slower than 400ms
/// and for a run that starts late. Narrower would be faster and would eventually
/// miss a block; the failure of missing one is a launch silently unscreened,
/// which is the direction that must not be traded for speed.
pub const LOOKBACK_HOURS: i64 = 36;

/// The lower bound for a launch-block query, given the present moment.
///
/// Relative to now rather than to a fixed date, so the scan is a constant width
/// forever instead of one that grows every day.
#[must_use]
pub fn since(now: &str) -> String {
    crate::prices::shift_hours(now, -LOOKBACK_HOURS).unwrap_or_else(|| now.to_owned())
}

impl LaunchBlockSource for CryptoHouseBlocks {
    type Error = QueryError;

    fn authorities_at(&self, mint: &Address, slot: Slot) -> Result<Vec<String>, Self::Error> {
        let rows: Vec<AuthorityOnly> =
            self.client
                .query(&query_for_authorities(mint, slot, &self.since))?;
        Ok(rows.into_iter().map(|r| r.authority).collect())
    }

    fn prevalence_table(&self) -> Result<radar_graph::prevalence::Table, Self::Error> {
        let rows: Vec<AuthorityRow> = self.client.query(&query_for_prevalence())?;
        Ok(radar_graph::prevalence::Table::new(
            rows.into_iter().filter_map(|row| {
                // A count that will not parse is dropped rather than read as
                // zero. Zero would classify a wallet the query selected
                // specifically for being *above* the floor as `Ordinary` — the
                // least alarming answer available, taken from a row that could
                // not be read at all. Rule 9.
                row.launch_blocks
                    .parse::<u64>()
                    .ok()
                    .map(|count| (row.authority, count))
            }),
        ))
    }

    fn shape_at(&self, mint: &Address, slot: Slot) -> Result<LaunchBlockShape, Self::Error> {
        let rows: Vec<ShapeRow> = self
            .client
            .query(&query_for_shape(mint, slot, &self.since))?;

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
    use radar_store::to_epoch;

    use super::*;

    #[test]
    fn the_prevalence_query_stays_inside_the_row_cap_by_construction() {
        // The `HAVING` is the only thing standing between this and a truncated
        // table. Without it the query returns every authority -- 8,707 in the
        // measured window -- against a fixed thousand-row cap, and the rows the
        // cut removed would each read as `Ordinary`, which is the least
        // alarming answer. Rule 9.
        let query = query_for_prevalence();
        assert!(
            query.contains(&format!(
                "HAVING uniqExact(t.mint) >= {}",
                radar_graph::prevalence::REPEAT_FLOOR
            )),
            "{query}"
        );
    }

    #[test]
    fn the_prevalence_window_is_the_one_the_bands_were_measured_over() {
        // A count over a different window is a different quantity wearing the
        // same unit. Changing the window means re-measuring the bands in 0012,
        // not adjusting a number here.
        assert_eq!(
            PREVALENCE_WINDOW_MINUTES,
            radar_graph::prevalence::WINDOW_MINUTES
        );
        let query = query_for_prevalence();
        assert_eq!(
            query
                .matches(&format!("INTERVAL {PREVALENCE_WINDOW_MINUTES} MINUTE"))
                .count(),
            2,
            "both halves of the join read the same window: {query}"
        );
    }

    #[test]
    fn the_authority_query_reads_one_block_and_prunes_on_a_timestamp() {
        // `token_transfers` prunes on `block_timestamp` and nothing else. A
        // slot filter alone reads naturally, prunes nothing, and fails at the
        // ten-billion-row ceiling even for a single mint (ADR 0002).
        let mint = Address::new([7u8; 32]);
        let query = query_for_authorities(&mint, Slot(442_771_316), A_BOUND);

        assert!(query.contains("block_slot = 442771316"), "{query}");
        assert!(
            query.contains(&format!("block_timestamp >= '{A_BOUND}'")),
            "{query}"
        );
        assert!(query.contains(&format!("mint = '{mint}'")), "{query}");
        // An empty authority is not a wallet; counting it would put every block
        // that has one into the same bucket.
        assert!(query.contains("authority != ''"), "{query}");
        // And it is a single-block read: no window, no join.
        assert!(!query.contains("INTERVAL"), "{query}");
        assert!(!query.contains("JOIN"), "{query}");
    }

    /// A fixed lower bound, for the query-shape assertions below.
    ///
    /// A constant is right *here* and was wrong in production: these tests care
    /// that the bound reaches the query, not what it is.
    const A_BOUND: &str = "2026-08-30 00:00:00";

    #[test]
    fn the_scan_window_is_a_fixed_width_and_does_not_grow_with_the_calendar() {
        // The bug this replaced, stated as a property.
        //
        // The bound was a constant date, so the window between it and the
        // present widened every single day -- and `token_transfers` prunes on
        // `block_timestamp` and nothing else, so that width IS the cost of the
        // query. It went from scanning nothing to scanning a month, and the
        // share of launch blocks that could not be read inside the server's
        // timeout went 2% -> 30% -> 68% with it.
        //
        // Two moments a year apart must produce the same width.
        for now in ["2026-08-01 12:00:00", "2027-08-01 12:00:00"] {
            let start = since(now);
            let hours =
                (to_epoch(now).expect("parses") - to_epoch(&start).expect("parses")) / 3_600;
            assert_eq!(
                hours, LOOKBACK_HOURS,
                "the window from {now} was {hours}h, not {LOOKBACK_HOURS}h"
            );
        }
    }

    #[test]
    fn the_window_covers_every_candidate_the_lane_can_ask_about_with_margin() {
        // `radar consider`'s window is 216,000 slots. At Solana's nominal 400ms
        // that is twenty-four hours, so a launch block is never older than a day
        // when it is asked about.
        //
        // The margin is the whole point of the assertion. Slots run slower than
        // nominal under load, and a bound that only just covers the nominal case
        // starts dropping the oldest candidates the first time the chain is
        // congested -- which presents as a launch silently unscreened, not as an
        // error. So require real headroom rather than a bare cover.
        const CANDIDATE_WINDOW_SLOTS: i64 = 216_000;
        const NOMINAL_SLOT_MS: i64 = 400;
        let nominal_hours = CANDIDATE_WINDOW_SLOTS * NOMINAL_SLOT_MS / 1000 / 3_600;
        assert_eq!(nominal_hours, 24, "the candidate window is a day");

        // Half again, so slots would have to run 50% slow before anything is
        // missed. Narrower is cheaper to scan, and cheap is not what this
        // constant is for -- being too narrow loses launches, being too wide only
        // costs scan time, and those two failures are not worth trading evenly.
        assert!(
            LOOKBACK_HOURS >= nominal_hours * 3 / 2,
            "a {LOOKBACK_HOURS}h window leaves too little margin over {nominal_hours}h"
        );
    }

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
        let sql = query_for_shape(&mint(), Slot(441_251_921), A_BOUND);
        assert!(sql.contains("block_timestamp >="), "must prune");
        assert!(sql.contains("block_slot = 441251921"), "must be one slot");
        assert!(sql.contains(&mint().to_string()), "must name the mint");
    }

    #[test]
    fn the_query_asks_about_exactly_one_slot() {
        // The whole signal is that a bundle is visible in the launch block
        // specifically. A range would dissolve it into ordinary early trading.
        let sql = query_for_shape(&mint(), Slot(441_251_921), A_BOUND);
        assert!(!sql.contains(">= 441251921"), "no lower-bounded slot range");
        assert!(!sql.contains("BETWEEN"), "no slot range");
    }

    #[test]
    fn quote_assets_are_excluded() {
        // Counting recipients of wrapped SOL would measure the market rather
        // than the token.
        let sql = query_for_shape(&mint(), Slot(1), A_BOUND);
        assert!(sql.contains("So11111111111111111111111111111111111111112"));
    }
}
