// SPDX-License-Identifier: Apache-2.0
//! `radar exits` — would an exit rule have helped?
//!
//! Research 0009 concluded "the exit rule is not where the edge is" on the
//! strength of a take-profit table, and its own diagnosis of why those failed —
//! that the losers "do not miss by a little" — is the case for a **stop**, which
//! it never tested. This tests both, and reports each rule as a pair of bounds
//! rather than a number, because the order of two crossings inside one checkpoint
//! interval is not recoverable from this data.

use radar_asof::AsOf;
use radar_research::exits::{self, Rule};
use radar_store::Reader;

/// Runs the comparison.
///
/// # Errors
///
/// Returns a message if the store cannot be read.
pub fn run(reader: &Reader, cost_bps: i64) -> Result<(), String> {
    let watermark = reader
        .watermark()
        .map_err(|e| format!("cannot read the store: {e}"))?
        .ok_or("the store has recorded nothing yet")?;

    let outcomes = reader
        .read_outcomes(AsOf::at(watermark))
        .map_err(|e| format!("cannot read outcomes: {e}"))?;

    let report = exits::evaluate(&outcomes);

    println!("watermark    : slot {watermark}");
    println!(
        "paths        : {} mints with a usable price path",
        report.paths
    );
    println!("assumed cost : {cost_bps} bps round trip\n");

    println!(
        "{:<14} {:>8} {:>10} {:>10} {:>10} {:>8} {:>8} {:>8}",
        "target/stop", "n", "pess p25", "pess med", "opt med", "target", "stop", "held"
    );
    for rule in &report.rules {
        let thin = if rule.is_reportable() { "" } else { "  (thin)" };
        println!(
            "{:<14} {:>8} {:>10} {:>10} {:>10} {:>8} {:>8} {:>8}{thin}",
            rule.label(),
            rule.pessimistic.n(),
            render(rule.pessimistic.percentile(0.25)),
            render(rule.pessimistic.median()),
            render(rule.optimistic.median()),
            rule.pessimistic.target,
            rule.pessimistic.stopped,
            rule.pessimistic.held,
        );
    }

    println!(
        "\nTwo medians because the order of two crossings inside one checkpoint\n\
         interval is not recoverable. `pess` assumes the stop was hit first\n\
         wherever both moved; `opt` assumes the target was. A rule is only worth\n\
         having if its PESSIMISTIC bound is."
    );

    match report.baseline().and_then(|b| b.pessimistic.median()) {
        None => println!("\nNo baseline could be measured, so nothing can be compared."),
        Some(base) => {
            println!("\nBaseline — no rule, held to the last observation: {base} bps.");
            let winners = report.beats_baseline();
            if winners.is_empty() {
                println!(
                    "\nNO RULE BEATS IT on the pessimistic bound. That is 0009's conclusion\n\
                     reached with the stop it never tested, rather than assumed from a\n\
                     take-profit table alone."
                );
            } else {
                println!("\nRules beating the baseline on the pessimistic bound:\n");
                for rule in &winners {
                    print_winner(rule, base, cost_bps);
                }
                println!(
                    "\nGross. Every rule that exits early pays the round trip more often than\n\
                     the baseline, and none of that is charged above."
                );
            }
        }
    }

    Ok(())
}

/// One rule that beat the baseline, with the margin stated.
fn print_winner(rule: &Rule, baseline_bps: i64, cost_bps: i64) {
    let median = rule.pessimistic.median().unwrap_or(0);
    let cleared = rule
        .pessimistic
        .beat_cost_bps(cost_bps)
        .map_or_else(|| "—".to_owned(), |b| format!("{b} bps"));
    println!(
        "  {:<14} {median} bps ({:+} vs baseline), {cleared} of them cleared costs",
        rule.label(),
        median - baseline_bps,
    );
}

/// A figure, or a dash where nothing was measured.
///
/// A dash rather than a zero: an unmeasured cohort has no median, and zero is a
/// perfectly good return (rule 9).
fn render(v: Option<i64>) -> String {
    v.map_or_else(|| "—".to_owned(), |v| v.to_string())
}
