// SPDX-License-Identifier: Apache-2.0
//! Measuring what became of tokens Radar has already recorded.
//!
//! Outcomes are what every signal has to be validated against, and they are the
//! difference between a recorder and something that can say whether anything it
//! noticed predicts anything.
//!
//! They are also the one thing the thousand-row cap does *not* obstruct.
//! Aggregates return one row per mint however many billions they scan, so a
//! batch of several hundred mints is a single query — which is why outcomes are
//! extracted this way and raw trades are not (ADR 0002).

use radar_store::Outcome;
use radar_types::{Address, Slot};
use serde::Deserialize;

/// Mints per query.
///
/// Well under the thousand-row cap, because the cap counts result rows and a
/// mint with no transfers at all simply does not come back — so the margin is
/// against a batch where every mint answers, not against the batch size itself.
pub const MINTS_PER_BATCH: usize = 400;

/// One aggregate row.
#[derive(Debug, Clone, Deserialize)]
pub struct AggregateRow {
    /// The mint.
    pub mint: String,
    /// Transfers observed.
    pub transfers: String,
    /// First transfer slot.
    pub first_slot: String,
    /// Last transfer slot.
    pub last_slot: String,
    /// Distinct sending token accounts.
    pub uniq_src: String,
    /// Distinct receiving token accounts.
    pub uniq_dst: String,
}

/// Builds the aggregate query for a batch of mints.
///
/// Bounded below by `since`, a **timestamp**, because `block_timestamp` is what
/// `token_transfers` is partitioned by. Bounding on `block_slot` instead reads
/// naturally and does nothing: the server still scans every partition and gives
/// up at its ten-billion-row ceiling, which it does even for a single mint.
///
/// The upper end stays open. An outcome is *what has happened so far*, and the
/// whole point is to see tokens that kept trading for days beside ones that died
/// in four slots.
#[must_use]
pub fn query_for_mints(mints: &[String], since: &str) -> String {
    let list = mints
        .iter()
        .map(|m| format!("'{m}'"))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "SELECT mint, toString(count()) AS transfers, \
                toString(min(block_slot)) AS first_slot, \
                toString(max(block_slot)) AS last_slot, \
                toString(uniqExact(source)) AS uniq_src, \
                toString(uniqExact(destination)) AS uniq_dst \
         FROM solana.token_transfers \
         WHERE block_timestamp >= '{since}' AND mint IN ({list}) GROUP BY mint"
    )
}

/// The wall-clock time of the first block at or after a slot.
///
/// Needed because the store records launches by slot while the transfer table
/// prunes by timestamp. One cheap lookup against `solana.blocks` converts
/// between them; deriving it arithmetically from a slot rate would drift, and
/// the drift would silently narrow the scan and lose transfers.
#[must_use]
pub fn query_for_slot_time(slot: Slot) -> String {
    let slot = slot.get();
    format!(
        "SELECT toString(min(block_timestamp)) AS at FROM solana.blocks \
         WHERE slot >= {slot} AND slot < {}",
        slot + 100_000
    )
}

/// A single timestamp answer.
#[derive(Debug, Deserialize)]
pub struct TimeRow {
    /// The timestamp, as `YYYY-MM-DD HH:MM:SS`.
    pub at: String,
}

/// The query for the chain's current head, used as the measurement slot.
#[must_use]
pub fn query_for_head() -> String {
    "SELECT toString(max(slot)) AS head FROM solana.blocks \
     WHERE block_timestamp >= now() - INTERVAL 30 MINUTE"
        .to_owned()
}

/// The head slot row.
#[derive(Debug, Deserialize)]
pub struct HeadRow {
    /// The highest recent slot.
    pub head: String,
}

/// Turns aggregate rows into outcomes.
///
/// A mint that was asked about and did not come back has never been transferred
/// — not even its own mint-to landed in this table — so it is recorded with zero
/// transfers rather than omitted. Omitting it would make "no data" and "no
/// activity" indistinguishable, and for outcome labels those are opposite
/// answers.
#[must_use]
pub fn outcomes_from_rows(
    rows: &[AggregateRow],
    launches: &[(Address, Slot)],
    measured_at: Slot,
    graduated: &[Address],
) -> Vec<Outcome> {
    let mut out = Vec::with_capacity(launches.len());

    for (mint, launch_slot) in launches {
        let key = mint.to_string();
        let row = rows.iter().find(|r| r.mint == key);
        let parse = |s: &str| s.parse::<u64>().ok();

        out.push(Outcome {
            mint: *mint,
            measured_at,
            launch_slot: *launch_slot,
            first_transfer_slot: row.and_then(|r| parse(&r.first_slot)).map(Slot),
            last_transfer_slot: row.and_then(|r| parse(&r.last_slot)).map(Slot),
            transfers: row.and_then(|r| parse(&r.transfers)).unwrap_or(0),
            unique_senders: row.and_then(|r| parse(&r.uniq_src)).unwrap_or(0),
            unique_receivers: row.and_then(|r| parse(&r.uniq_dst)).unwrap_or(0),
            graduated: graduated.contains(mint),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mint(n: u8) -> Address {
        Address::new([n; 32])
    }

    fn row(mint: &Address, transfers: &str, first: &str, last: &str) -> AggregateRow {
        AggregateRow {
            mint: mint.to_string(),
            transfers: transfers.to_owned(),
            first_slot: first.to_owned(),
            last_slot: last.to_owned(),
            uniq_src: "9".to_owned(),
            uniq_dst: "7".to_owned(),
        }
    }

    #[test]
    fn a_mint_with_no_rows_is_recorded_as_zero_rather_than_omitted() {
        // "No data" and "no activity" are opposite answers for an outcome label.
        // Dropping the silent ones would quietly remove every instant death from
        // the dataset -- which is the population the labels exist to identify.
        let launches = vec![(mint(1), Slot(1_000)), (mint(2), Slot(1_000))];
        let rows = vec![row(&mint(1), "50", "1001", "5000")];

        let out = outcomes_from_rows(&rows, &launches, Slot(9_999), &[]);
        assert_eq!(out.len(), 2, "both mints must appear");

        let silent = out
            .iter()
            .find(|o| o.mint == mint(2))
            .expect("the silent one");
        assert_eq!(silent.transfers, 0);
        assert_eq!(silent.last_transfer_slot, None, "absent, not zero");
        assert_eq!(silent.survived_slots(), 0);
    }

    #[test]
    fn the_real_pair_from_mainnet_separates() {
        // Both measured: one traded for four slots with three transfers, the
        // other for 374,582 slots with 1,535.
        let launches = vec![(mint(1), Slot(440_624_864)), (mint(2), Slot(440_623_612))];
        let rows = vec![
            row(&mint(1), "3", "440624864", "440624868"),
            row(&mint(2), "1535", "440623612", "440998194"),
        ];
        let out = outcomes_from_rows(&rows, &launches, Slot(441_000_000), &[]);

        let dead = out.iter().find(|o| o.mint == mint(1)).expect("dead");
        let alive = out.iter().find(|o| o.mint == mint(2)).expect("alive");
        assert_eq!(dead.survived_slots(), 4);
        assert!(dead.appears_stillborn());
        assert_eq!(alive.survived_slots(), 374_582);
        assert!(!alive.appears_stillborn());
    }

    #[test]
    fn graduation_is_carried_through() {
        let launches = vec![(mint(1), Slot(1_000))];
        let rows = vec![row(&mint(1), "900", "1001", "90000")];
        let out = outcomes_from_rows(&rows, &launches, Slot(99_999), &[mint(1)]);
        assert!(out[0].graduated);
    }

    #[test]
    fn every_outcome_carries_the_slot_it_was_measured_at() {
        // Without it the row cannot be admitted through a watermark, so it could
        // never be used in a replay.
        let launches = vec![(mint(1), Slot(1_000))];
        let out = outcomes_from_rows(&[], &launches, Slot(555_555), &[]);
        assert_eq!(out[0].measured_at, Slot(555_555));
    }

    #[test]
    fn a_malformed_count_reads_as_absent_rather_than_zero_activity() {
        // A parse failure is missing data, not a quiet token, and the two must
        // not collapse into the same label.
        let launches = vec![(mint(1), Slot(1_000))];
        let rows = vec![row(&mint(1), "not a number", "also not", "nope")];
        let out = outcomes_from_rows(&rows, &launches, Slot(9_999), &[]);
        assert_eq!(out[0].first_transfer_slot, None);
        assert_eq!(out[0].last_transfer_slot, None);
    }

    #[test]
    fn the_head_query_uses_the_column_that_table_actually_has() {
        // solana.blocks calls it `slot`; every other table calls it `block_slot`.
        let sql = query_for_head();
        assert!(sql.contains("max(slot)"), "{sql}");
        assert!(!sql.contains("block_slot"), "{sql}");
    }

    #[test]
    fn the_query_names_every_mint_and_prunes_on_the_partition_key() {
        let mints: Vec<String> = (1..=3).map(|n| mint(n).to_string()).collect();
        let sql = query_for_mints(&mints, "2026-08-21 00:00:00");
        for m in &mints {
            assert!(sql.contains(m.as_str()), "query is missing {m}");
        }
        assert!(
            sql.contains("GROUP BY mint"),
            "must aggregate, or it hits the thousand-row cap"
        );
        // token_transfers prunes by block_timestamp. Bounding on block_slot
        // reads naturally and prunes nothing -- the server scans every partition
        // and refuses at ten billion rows, even for one mint.
        assert!(
            sql.contains("block_timestamp >= '2026-08-21 00:00:00'"),
            "{sql}"
        );
        assert!(
            !sql.contains("block_slot >="),
            "a slot bound does not prune this table"
        );
        // Open above: an outcome is what has happened so far.
        assert!(!sql.contains("block_timestamp <"), "{sql}");
    }

    #[test]
    fn the_slot_to_time_lookup_is_bounded_so_it_stays_cheap() {
        let sql = query_for_slot_time(Slot(440_620_000));
        assert!(sql.contains("slot >= 440620000"), "{sql}");
        assert!(
            sql.contains("slot < 440720000"),
            "an unbounded scan is not cheap: {sql}"
        );
    }
}
