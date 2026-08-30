// SPDX-License-Identifier: Apache-2.0
//! `radar basis` — how much of the selection's return is the instrument?
//!
//! [`radar selection`](crate::selection) prices a decision from a **sell quote**
//! and its outcome from **realised fills of both sides**. A bid measured against
//! a mid is positive before the market moves, so some part of every return that
//! command reports is the gap between two instruments rather than a gain.
//!
//! This measures that gap, and it is a precondition for reading `selection` at
//! all: on 2026-08-30 the gross median there was +21 bps, which is smaller than
//! a single leg of pump.fun's fee.
//!
//! The output is deliberately a table by time gap rather than one number. A
//! basis that is flat across the buckets is an artefact of the instruments; one
//! that grows with the gap is the market moving. The reader should be able to
//! tell which they are looking at.

use radar_asof::AsOf;
use radar_research::basis::{self, Verdict};
use radar_store::Reader;

/// Runs the measurement.
///
/// # Errors
///
/// Returns a message if the store cannot be read.
pub fn run(reader: &Reader) -> Result<(), String> {
    let watermark = reader
        .watermark()
        .map_err(|e| format!("cannot read the store: {e}"))?
        .ok_or("the store has recorded nothing yet")?;
    let as_of = AsOf::at(watermark);

    let decisions = reader
        .read_decisions(as_of)
        .map_err(|e| format!("cannot read decisions: {e}"))?;
    let outcomes = reader
        .read_outcomes(as_of)
        .map_err(|e| format!("cannot read outcomes: {e}"))?;

    let report = basis::measure(&decisions, &outcomes);

    println!("watermark    : slot {watermark}");
    println!("decisions    : {} recorded", decisions.len());
    println!(
        "with entry   : {} carry a quote to compare",
        report.with_entry
    );
    println!("paired       : {} found a realised price\n", report.paired);

    println!(
        "{:<8} {:>8} {:>10} {:>10} {:>10}",
        "gap", "pairs", "p25", "median", "p75"
    );
    for bucket in &report.buckets {
        println!(
            "{:<8} {:>8} {:>10} {:>10} {:>10}",
            bucket.label,
            bucket.n(),
            render(bucket.percentile(0.25)),
            render(bucket.median()),
            render(bucket.percentile(0.75)),
        );
    }

    println!(
        "\nBasis is the realised price against the quoted one, in basis points.\n\
         Positive means the realised price sat above the quote -- the direction\n\
         that flatters every return `radar selection` reports."
    );

    match report.verdict() {
        Verdict::NotEnoughData { paired, needed } => {
            println!(
                "
NOT ENOUGH DATA. {paired} pair(s) found, and no single gap bucket holds
                 enough to report a median -- the largest needs {needed} more. A median
                 over a handful of rows has the shape of a finding and the content of
                 noise."
            );
        }
        Verdict::Measured {
            tightest_median_bps,
            tightest_label,
            tightest_n,
            widest_median_bps,
            drifts_down,
        } => {
            println!(
                "
At the tightest measurable gap ({tightest_label}), over {tightest_n} pairs,
                 the basis is {tightest_median_bps} bps."
            );

            if let Some(widest) = widest_median_bps {
                println!("The widest populated bucket reads {widest} bps.");
            }

            if drifts_down {
                println!(
                    "
The basis FALLS as the gap widens, so it is not a market effect --
                     a market effect would grow with time, not shrink. What the buckets
                     share is the instrument; what separates them is this market's
                     downward drift, which SUBTRACTS from the basis.
                     
                     So {tightest_median_bps} bps is a FLOOR, not an estimate. The gap between a sell
                     quote and a realised fill is at least this wide, and `radar selection`
                     owes at least this much as a correction -- subtracted, because a quote
                     below a mid makes a flat position look like a gain."
                );
            } else {
                println!(
                    "
The basis does not fall as the gap widens, so the drift in this
                     sample is not negative and {tightest_median_bps} bps cannot be read as a floor.
                     It is the whole gap between the instruments at this distance, market
                     movement included."
                );
            }
        }
    }

    Ok(())
}

/// A basis point figure, or a dash where nothing was measured.
///
/// A dash rather than a zero: an empty bucket has no basis, and printing zero
/// would read as "these two instruments agree" (rule 9).
fn render(v: Option<i64>) -> String {
    v.map_or_else(|| "-".to_owned(), |v| v.to_string())
}
