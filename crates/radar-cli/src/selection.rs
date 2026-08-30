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
use radar_store::{Decision, Outcome, Reader};

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

    print_screening_split(&decisions, &outcomes);
    print_refusal_breakdown(&decisions, &outcomes, cost_bps);

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
            control_median_bps,
            selection_beat_cost_bps,
            control_beat_cost_bps,
        } => {
            println!(
                "
{:<30} {:>12} {:>12}",
                "", "proposed", "refused"
            );
            println!(
                "{:<30} {:>12} {:>12}",
                "median return (net, bps)",
                selection_median_bps,
                render(control_median_bps)
            );
            println!(
                "{:<30} {:>12} {:>12}",
                "cleared costs (bps of set)",
                selection_beat_cost_bps,
                render(control_beat_cost_bps.and_then(|v| i64::try_from(v).ok()))
            );
            println!(
                "
The control is the refusals, priced the same way in the same passes:
                 entry at the watermark each decision was taken, exit at the last
                 observation after it. That is what makes a difference between them
                 attributable to the selection rather than to the measurement.
                 
                 For context only, research 0009 measured the unselected population at
                 {} bps median with {} bps of it clearing costs — but that enters at
                 each token's FIRST FILL, so it is not this quantity and the two must
                 not be subtracted.
                 
                 A median above the control's, over one regime, is not an edge. It is a
                 reason to keep measuring.",
                radar_research::selection::POPULATION_MEDIAN_BPS_0009,
                radar_research::selection::POPULATION_BEAT_COST_BPS_0009,
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

/// The control, split by why each token was refused.
///
/// A function rather than a block because the flat comparison it sits under is
/// the headline and this is the caveat, and the two should be readable apart.
///
/// The flat comparison cannot distinguish "the selection is wrong" from "the
/// control is flattered by returns nobody could have realised", and those want
/// opposite responses. A token refused because its exit capacity was below the
/// floor was refused *precisely because nobody could have sold it*.
fn print_refusal_breakdown(decisions: &[Decision], outcomes: &[Outcome], cost_bps: u64) {
    let groups = selection::by_reason(decisions, outcomes);
    if groups.is_empty() {
        return;
    }

    println!(
        "
the control, split by why it was refused:"
    );
    println!(
        "{:<26} {:>10} {:>8} {:>10} {:>10}",
        "reason", "decisions", "scored", "median", "cleared"
    );
    for (reason, cohort) in &groups {
        // Integer arithmetic on a per-ten-thousand rate. A float here would be
        // a lossy cast on a number that is already exact.
        let cleared = cohort.beat_cost_bps(cost_bps).map_or_else(
            || "     -".to_owned(),
            |bps| format!("{}.{:02}%", bps / 100, bps % 100),
        );
        println!(
            "{reason:<26} {:>10} {:>8} {:>10} {cleared:>10}",
            cohort.decisions,
            cohort.scored,
            render(cohort.median()),
        );
    }

    println!(
        "
Groups overlap: a decision counts under every reason it carries, so these
         do not sum to the control. The question is what tokens refused for X did,
         not what tokens refused ONLY for X did -- a much smaller, stranger population."
    );
    println!(
        "
A high median under an exit-related reason is not an edge Radar passed up.
         It is a paper return on a token nobody could sell, which is what that
         refusal is for. LEARNINGS 11 is the same mistake made with MFE."
    );
}

/// Splits the proposed cohort by whether the coordination gate actually ran.
///
/// Printed beside the headline rather than in a separate command, because a
/// blend of two populations reads exactly like one population and the reader has
/// no way to know which they are looking at.
fn print_screening_split(decisions: &[Decision], outcomes: &[Outcome]) {
    let (screened, unscreened) = radar_research::selection::by_screening(decisions, outcomes);
    if unscreened.decisions == 0 {
        return;
    }

    println!(
        "
Proposals, split by whether the coordination gate ran:
"
    );
    println!(
        "{:<12} {:>10} {:>8} {:>10} {:>10} {:>10}",
        "cohort", "decisions", "scored", "median", "p25", "p75"
    );
    for (name, cohort) in [("screened", &screened), ("unscreened", &unscreened)] {
        println!(
            "{name:<12} {:>10} {:>8} {:>10} {:>10} {:>10}",
            cohort.decisions,
            cohort.scored,
            render(cohort.median()),
            render(cohort.percentile(0.25)),
            render(cohort.percentile(0.75)),
        );
    }
    println!(
        "
`unscreened` is a candidate whose launch block CryptoHouse could not serve,
         so it was proposed without the screen research 0008 measures at 11.7x on
         instant graduation. `creator_edge` is right not to let a missing reading
         refuse -- that would refuse the population whenever the vendor hiccups -- but
         it means the headline above blends a population Radar selected with one it
         merely failed to reject. `radar brief`'s screening check watches the rate."
    );
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
