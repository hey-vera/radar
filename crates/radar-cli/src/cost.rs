// SPDX-License-Identifier: Apache-2.0
//! `radar cost` — what a round trip costs, and whether that depends on its size.
//!
//! `assumed_round_trip_bps` is 850, and it is the figure that turns research
//! 0014's gross median of +21 bps into −829. The measurement behind it is
//! described in detail in `creator_edge`'s documentation and **the query that
//! produced it is not in the repository** — LEARNINGS entry 1's shape, on the
//! most load-bearing constant in the system.
//!
//! This re-derives it, and asks the question the constant cannot answer: the
//! method it came from captures rent and a second hop, and rent is fixed. A cost
//! with a fixed component applied in basis points to a $6.21 position is not a
//! cost model. So the output is cost **by notional bucket**, in basis points and
//! in lamports — a proportional cost is flat in bps across the buckets, and a
//! fixed one falls as the notional grows.

use radar_backfill::{cost, cryptohouse};

/// Runs the measurement over one hour.
///
/// # Errors
///
/// Returns a message if the endpoint refuses the query.
pub fn run(from: &str, to: &str) -> Result<(), String> {
    let program = radar_decode::pumpfun::PROGRAM_ID.to_string();
    let discs = radar_backfill::extract::Scope::Trades.discriminators();
    let sql = cost::query_for_window(from, to, &program, &discs);

    let rows: Vec<cost::CostRow> = cryptohouse::Client::default()
        .query(&sql)
        .map_err(|e| format!("cryptohouse: {e}"))?;

    if rows.is_empty() {
        println!(
            "No fills in {from} .. {to}. That is a statement about the window, not\n\
             about the venue -- widen it before concluding anything."
        );
        return Ok(());
    }

    println!("window       : {from} .. {to}");
    println!("program      : {program}\n");
    println!(
        "{:>16} {:>10} {:>12} {:>16}",
        "notional >=", "fills", "median bps", "median lamports"
    );

    for row in &rows {
        println!(
            "{:>16} {:>10} {:>12} {:>16}",
            row.bucket_lamports, row.fills, row.median_bps, row.median_lamports
        );
    }

    println!("\n{} fill(s) measured, one leg each.", total_fills(&rows));
    println!(
        "\nCost is the gap between the largest outflow and the largest inflow of the\n\
         transaction -- protocol fee, priority fee, rent and slippage together.\n\
         Approximate in the way the original measurement was: it attributes the\n\
         largest inflow/outflow pair to the trade, so it also captures rent and any\n\
         second hop.\n\
         \n\
         Read the two right-hand columns against each other. A purely proportional\n\
         cost is FLAT in basis points and grows in lamports. A fixed cost is flat in\n\
         LAMPORTS and falls in basis points as the notional rises. Radar's median\n\
         proposed notional is $6.21, around 31,000,000 lamports, so the bucket at\n\
         10,000,000 is the one its own economics live in."
    );

    Ok(())
}

/// Fills across every bucket.
///
/// Extracted from the printer so it can be tested without a network round trip.
/// It is the only number in the output that is not straight from the server, and
/// it is the one a reader uses to judge whether any bucket's median is worth
/// reading — 90 legs and 79,061 legs are different evidence.
///
/// A row whose count does not parse contributes nothing rather than aborting the
/// report: the per-bucket figures beside it are still the measurement, and
/// refusing to print them because a total cannot be formed would lose more than
/// it protects.
fn total_fills(rows: &[cost::CostRow]) -> u64 {
    rows.iter()
        .filter_map(|r| r.fills.parse::<u64>().ok())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(fills: &str) -> cost::CostRow {
        cost::CostRow {
            bucket_lamports: "1000000".to_owned(),
            fills: fills.to_owned(),
            median_bps: "0".to_owned(),
            median_lamports: "0".to_owned(),
        }
    }

    #[test]
    fn the_total_sums_every_bucket() {
        // Summed, not differenced or multiplied. The distinction is not academic:
        // this number is what tells a reader whether a bucket's median rests on
        // ninety legs or seventy-nine thousand.
        assert_eq!(
            total_fills(&[row("29996"), row("79061"), row("90")]),
            109_147
        );
        assert_eq!(total_fills(&[]), 0);
        assert_eq!(total_fills(&[row("1")]), 1);
    }

    #[test]
    fn an_unparseable_count_contributes_nothing_rather_than_aborting() {
        // The per-bucket figures beside it are still the measurement.
        assert_eq!(
            total_fills(&[row("12"), row("not a number"), row("30")]),
            42
        );
    }
}
