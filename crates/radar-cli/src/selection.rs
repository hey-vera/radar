// SPDX-License-Identifier: Apache-2.0
//! `radar selection` — did the selection beat the population it selected from?
//!
//! The question the project exists to answer. It reads recorded decisions and
//! the outcome measurements that followed them, and reports what Radar's
//! choices actually returned beside the population base rate from research
//! 0009.
//!
//! It will say "not enough data" for some time, and that is the correct output
//! rather than a placeholder. Decisions began accumulating on 2026-08-26 and a
//! pass records a few dozen.

use radar_asof::AsOf;
use radar_research::selection::{self, Verdict};
use radar_store::Reader;

/// Runs the comparison.
///
/// # Errors
///
/// Returns a message if the store cannot be read.
pub fn run(reader: &Reader, cost_bps: u64) -> Result<(), String> {
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

    println!("watermark    : slot {watermark}");
    println!("decisions    : {} recorded", decisions.len());
    println!("outcomes     : {} measurements", outcomes.len());
    println!("assumed cost : {cost_bps} bps round trip\n");

    if decisions.is_empty() {
        println!(
            "No decisions recorded. `radar consider --record` writes them, and the\n\
             hourly cron does so in production. Nothing can be said about whether\n\
             the selection beats the base rate until it has made some."
        );
        return Ok(());
    }

    let report = selection::evaluate(&decisions, &outcomes, cost_bps);

    println!(
        "{:<12} {:>10} {:>8} {:>10} {:>10} {:>10}",
        "cohort", "decisions", "scored", "median", "p25", "p75"
    );
    for (name, cohort) in [("proposed", &report.proposed), ("refused", &report.refused)] {
        println!(
            "{name:<12} {:>10} {:>8} {:>10} {:>10} {:>10}",
            cohort.decisions,
            cohort.scored,
            render(cohort.median()),
            render(cohort.percentile(0.25)),
            render(cohort.percentile(0.75)),
        );
    }
    println!("\nReturns are gross, in basis points, from the price the decision saw to");
    println!("the last price observed after it. Costs are applied in the verdict below.");

    match report.verdict() {
        Verdict::NotEnoughData { scored, needed } => {
            println!(
                "\nNOT ENOUGH DATA. {scored} proposal(s) scored; {needed} more before a\n\
                 percentile means anything. A median over a handful of tokens has the\n\
                 shape of a finding and the content of noise, and this repository has\n\
                 been caught by exactly that three times -- LEARNINGS 7, 10 and 11."
            );
        }
        Verdict::Measured {
            selection_median_bps,
            population_median_bps,
            selection_beat_cost_bps,
            population_beat_cost_bps,
        } => {
            println!("\n{:<28} {:>12} {:>12}", "", "selection", "population");
            println!(
                "{:<28} {:>12} {:>12}",
                "median return (net, bps)", selection_median_bps, population_median_bps
            );
            println!(
                "{:<28} {:>12} {:>12}",
                "cleared costs (bps of set)", selection_beat_cost_bps, population_beat_cost_bps
            );
            println!(
                "\nThe population figures are research 0009's, measured over 4,199 tokens.\n\
                 A selection median above the population's, over one regime, is not an\n\
                 edge -- it is a reason to keep measuring. Report it as such."
            );
        }
    }
    Ok(())
}

/// A basis-point figure, or a dash where there is nothing to report.
///
/// A dash rather than a zero: no cohort and a cohort that broke even are
/// different findings, and zero is far better than this population's median.
fn render(value: Option<i64>) -> String {
    value.map_or_else(|| "—".to_owned(), |v| v.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_measured_renders_as_absent_not_as_zero() {
        // Zero is a return. Absent is not, and zero would be a good one here.
        assert_eq!(render(None), "—");
        assert_eq!(render(Some(0)), "0");
        assert_eq!(render(Some(-1_340)), "-1340");
    }
}
