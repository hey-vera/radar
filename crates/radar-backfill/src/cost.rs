// SPDX-License-Identifier: Apache-2.0
//! What a round trip actually costs, and whether that depends on its size.
//!
//! # Why this exists
//!
//! `creator_edge::Thresholds::assumed_round_trip_bps` is **850**, and the doc
//! comment beside it describes the measurement in detail — 26,691 fills, a
//! median of 423 bps a leg, a mean of 845, a 90th percentile of 2,280, and a
//! breakdown by transaction shape.
//!
//! **The query that produced those numbers is not in this repository.**
//! `docs/research/queries/` holds one file and it is not this one, and there is
//! no research note. The constant is the single most load-bearing figure in the
//! system — [`0014`](https://github.com/hey-vera/radar) turns a gross median of
//! +21 bps into −829 with it — and it rests on a measurement nobody can re-run.
//! That is [`LEARNINGS`](https://github.com/hey-vera/radar/blob/main/LEARNINGS.md)
//! entry 1's shape: a design documented as canonical with its source lost.
//!
//! The guard beside it does not help. `the_assumed_cost_is_not_below_what_a
//! _round_trip_was_measured_to_cost` compares 850 against
//! `MEASURED_MEDIAN_ROUND_TRIP_BPS`, and both are hand-entered constants in the
//! same file. It guards a transcription, not a measurement.
//!
//! # The question the constant cannot answer
//!
//! 850 bps is applied as a pure proportion, and the method it came from
//! explicitly captures "rent and any second hop" — and rent is **fixed**. A cost
//! with a fixed component, measured on other people's trade sizes and then
//! applied in basis points to a $6 position, is not a cost model.
//!
//! Radar's median proposed notional is **$6.21**. Whether a round trip at that
//! size costs 850 bps or 3,000 decides whether any selection could pay for
//! itself, and no figure in the repository distinguishes them. So this measures
//! cost **as a function of notional** rather than as one number.
//!
//! # Method
//!
//! For each transaction, `balance_changes` gives every account's lamport delta.
//! The largest outflow is what the trader gave up and the largest inflow is what
//! somebody received; the gap between them is the protocol fee, the priority
//! fee, account rent and slippage together — which is what a trader actually
//! loses. That is the same construction the original measurement describes, and
//! it is approximate in the same way: it attributes the largest inflow/outflow
//! pair to the trade, so it also captures rent and any second hop.
//!
//! Bucketed by the size of the outflow, so the fixed and proportional halves
//! separate: a purely proportional cost is flat in basis points across the
//! buckets, and a fixed one falls as the notional grows.

use serde::Deserialize;

/// One notional bucket's cost, as the server returns it.
///
/// Every field is a string because CryptoHouse returns `UInt64` as JSON numbers
/// that exceed `f64`'s exact range, and the rest of this crate reads them the
/// same way for the same reason.
#[derive(Debug, Clone, Deserialize)]
pub struct CostRow {
    /// Lower edge of the bucket, in lamports.
    pub bucket_lamports: String,
    /// Fills in the bucket.
    pub fills: String,
    /// Median cost of a leg, in basis points of the outflow.
    pub median_bps: String,
    /// Median cost of a leg, in lamports.
    ///
    /// The half that says whether the cost is fixed. A proportional cost grows
    /// with the bucket; a fixed one does not.
    pub median_lamports: String,
}

/// Lower edges of the notional buckets, in lamports.
///
/// Roughly $0.20, $2, $20, $200 and $2,000 at 1e9 lamports to the SOL and a SOL
/// near $200. Chosen to bracket Radar's own median proposed notional of $6.21,
/// which sits in the second bucket — the point of the measurement is what a
/// round trip costs *there*, and the buckets either side are what say whether
/// that figure is size-dependent.
pub const BUCKET_EDGES: [u64; 5] = [
    1_000_000,
    10_000_000,
    100_000_000,
    1_000_000_000,
    10_000_000_000,
];

/// The query.
///
/// Bounded to one hour, and to transactions touching the pump.fun program, for
/// the reason [ADR 0002](https://github.com/hey-vera/radar) gives: Radar is a
/// guest on a shared free endpoint, and a six-hour window over
/// `balance_changes` reads 57 GB.
///
/// `arrayMin` over the deltas is the largest outflow as a negative number, and
/// `arrayMax` is the largest inflow. The gap between them is the cost. Fills
/// where the outflow is not positive are dropped: a transaction nobody paid for
/// is not a trade, and dividing by it would produce a basis-point figure with no
/// content.
#[must_use]
pub fn query_for_window(from: &str, to: &str, program: &str) -> String {
    let edges = BUCKET_EDGES
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "SELECT toString(bucket) AS bucket_lamports, \
                toString(count()) AS fills, \
                toString(toUInt64(quantileExact(0.5)(cost * 10000 / out))) AS median_bps, \
                toString(toUInt64(quantileExact(0.5)(cost))) AS median_lamports \
         FROM ( \
           SELECT -arrayMin(arrayMap(x -> toInt128(x.3 - x.2), balance_changes)) AS out, \
                  arrayMax(arrayMap(x -> toInt128(x.3 - x.2), balance_changes)) AS inn, \
                  out - inn AS cost, \
                  roundDown(out, [{edges}]) AS bucket \
           FROM solana.transactions \
           WHERE block_timestamp >= '{from}' AND block_timestamp < '{to}' \
             AND has(account_keys, '{program}') AND succeeded \
             AND out > 0 AND cost >= 0 \
         ) GROUP BY bucket ORDER BY bucket"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const PUMP: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";

    #[test]
    fn the_query_bounds_both_ends_of_the_window() {
        // An unbounded upper end reads to the head of the chain, which on
        // `balance_changes` is the difference between a three-second query and a
        // timeout. `prices.rs` leaves its upper end open deliberately and says
        // why; this one must not, because it is a measurement of a fixed past
        // window rather than of what has happened so far.
        let q = query_for_window("2026-08-25 04:00:00", "2026-08-25 05:00:00", PUMP);
        assert!(q.contains("block_timestamp >= '2026-08-25 04:00:00'"));
        assert!(q.contains("block_timestamp < '2026-08-25 05:00:00'"));
    }

    #[test]
    fn the_query_counts_only_successful_transactions() {
        // A failed transaction still burns a fee, so its "cost" is total and its
        // notional is nothing -- which would drag every bucket's median toward
        // infinity. LEARNINGS 7 recorded 35 of 97 migrations in a sampled hour
        // being failures; this filter is that lesson applied here.
        assert!(query_for_window("a", "b", PUMP).contains("succeeded"));
    }

    #[test]
    fn the_query_refuses_a_trade_nobody_paid_for() {
        // `out > 0` is the denominator guard. Without it a transaction with no
        // outflow divides by zero and returns a figure with the shape of a cost.
        assert!(query_for_window("a", "b", PUMP).contains("out > 0"));
    }

    #[test]
    fn every_bucket_edge_reaches_the_query() {
        // A dropped edge silently merges two buckets, and merging is exactly
        // what destroys the fixed-versus-proportional split this exists to make.
        let q = query_for_window("a", "b", PUMP);
        for edge in BUCKET_EDGES {
            assert!(q.contains(&edge.to_string()), "edge {edge} missing");
        }
    }

    #[test]
    fn the_buckets_ascend_and_bracket_radars_own_notional() {
        // Radar's median proposed notional is $6.21, which at a SOL near $200 is
        // about 31,000,000 lamports -- inside the second bucket. The measurement
        // is worthless if its buckets do not contain the size actually traded.
        assert!(BUCKET_EDGES.windows(2).all(|w| w[0] < w[1]));
        let radar_notional = 31_000_000u64;
        assert!(
            BUCKET_EDGES[1] <= radar_notional && radar_notional < BUCKET_EDGES[2],
            "Radar's own notional must land in a measured bucket"
        );
    }

    #[test]
    fn the_program_is_a_parameter_and_reaches_the_filter() {
        // Hardcoding it here would make this unusable for the venue question the
        // measurement exists to inform.
        assert!(query_for_window("a", "b", PUMP).contains(PUMP));
    }
}
