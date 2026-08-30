// SPDX-License-Identifier: Apache-2.0
//! `radar control` — the selection against a control it could have traded.
//!
//! [`radar selection`](crate::selection) compares Radar's proposals against its
//! own refusals, and research 0014 found that comparison unusable: every
//! scoreable refusal was `CapacityBelowFloor`, so the control was tokens Radar
//! had measured and found it could not sell.
//!
//! This compares against tokens Radar **never decided on**, priced the same way
//! on both sides — outcome to outcome, so the instrument is identical — and
//! matched on token age at entry and holding period, the two confounders
//! research 0011 shows move the population median on their own.

use radar_asof::AsOf;
use radar_research::control::{self, Stratum, Verdict};
use radar_store::Reader;

/// Runs the comparison.
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

    let report = control::evaluate(&decisions, &outcomes);

    println!("watermark    : slot {watermark}");
    println!("selected     : {} proposals priced", report.selected);
    println!(
        "control      : {} untouched tokens priced\n",
        report.control
    );

    println!(
        "{:<6} {:<6} {:>8} {:>8} {:>9} {:>9} {:>8} {:>9} {:>9}",
        "age", "hold", "sel n", "ctl n", "sel p25", "sel med", "sel p75", "ctl med", "edge"
    );
    for s in &report.strata {
        let mark = if s.is_comparable() { "" } else { "  (thin)" };
        println!(
            "{:<6} {:<6} {:>8} {:>8} {:>9} {:>9} {:>8} {:>9} {:>9}{mark}",
            s.age,
            s.hold,
            s.selected_bps.len(),
            s.control_bps.len(),
            render(Stratum::percentile(&s.selected_bps, 0.25)),
            render(s.selected_median()),
            render(Stratum::percentile(&s.selected_bps, 0.75)),
            render(s.control_median()),
            render(s.edge_bps()),
        );
    }

    println!(
        "
Share of each cohort that returned exactly zero:"
    );
    for s in &report.strata {
        if !s.is_comparable() {
            continue;
        }
        println!(
            "  {:<6} {:<6} selected {:>5} bps   control {:>5} bps",
            s.age,
            s.hold,
            render_u(Stratum::zero_share_bps(&s.selected_bps)),
            render_u(Stratum::zero_share_bps(&s.control_bps)),
        );
    }

    println!(
        "\nBoth sides are priced outcome to outcome, so the instrument is the same\n\
         on each and on both ends -- which is what research 0016 found `radar\n\
         selection` does not do. Only strata where BOTH cohorts clear {} rows are\n\
         compared; the rest are marked thin and contribute nothing.",
        control::MIN_STRATUM
    );

    match report.verdict() {
        Verdict::NoComparableStratum { populated } => {
            println!(
                "\nNO COMPARABLE STRATUM. {populated} stratum/strata held returns, and none\n\
                 had both cohorts above the floor. That is the honest output: a\n\
                 stratum only the selection reaches would compare it against itself."
            );
        }
        Verdict::Measured {
            strata,
            median_edge_bps,
            strata_favouring_selection,
        } => {
            println!(
                "\nOver {strata} comparable stratum/strata, the median edge is\n\
                 {median_edge_bps} bps, and {strata_favouring_selection} of {strata} favour the selection."
            );
            println!(
                "\nPositive means Radar's proposals beat matched tokens it never looked\n\
                 at. This is gross, and the measured round trip is 850 bps."
            );
        }
    }

    Ok(())
}

/// A figure, or a dash where nothing was measured.
///
/// A dash rather than a zero: an empty cohort has no median, and zero would read
/// as "these performed identically" (rule 9).
fn render(v: Option<i64>) -> String {
    v.map_or_else(|| "—".to_owned(), |v| v.to_string())
}

/// An unsigned figure, or a dash where nothing was measured.
fn render_u(v: Option<u64>) -> String {
    v.map_or_else(|| "—".to_owned(), |v| v.to_string())
}
