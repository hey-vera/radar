// SPDX-License-Identifier: Apache-2.0
//! `radar edge` — does any stratum clear the bar, out of sample?
//!
//! The question research 0017 answered with **0 bps** and research 0022 put a
//! bar of **456** under. Plan 0007 item 2; the protocol and the two readings it
//! settles are in [`radar_research::edge`].
//!
//! It will say "nothing found" for some time, and that is the correct output
//! rather than a placeholder — design 0010 §6.3 is what happens next when it
//! does, and none of it is a code change.
//!
//! Reads the feature table from the file `radar features` wrote, so the two
//! commands compose and the file a research note cites is the file that was
//! measured.

use radar_research::edge::{self, Candidate, Enumeration, Horizon, Options, Reading, Report};
use radar_roast::BaseRates;

use crate::flag;

/// Runs the walk-forward protocol and prints what it found.
///
/// # Errors
///
/// A message when the table cannot be read, the snapshot cannot be loaded, or
/// the protocol refuses to run.
pub fn run(args: &[String]) -> Result<(), String> {
    // Arguments first, then the large read. A typo in `--horizon` should not
    // cost the minutes it takes to load a production feature table.
    let horizon = match flag(args, "--horizon").as_deref() {
        None | Some("24h") => Horizon::TwentyFourHours,
        Some("6h") => Horizon::SixHours,
        Some(other) => return Err(format!("--horizon {other} is not 6h or 24h")),
    };
    let path = flag(args, "--features")
        .ok_or("--features <file> is required; `radar features` writes one")?;

    let rates_path =
        flag(args, "--rates").unwrap_or_else(|| radar_roast::baserates::DEFAULT_PATH.to_owned());
    let rates =
        BaseRates::load(&rates_path).map_err(|e| format!("cannot read {rates_path}: {e}"))?;

    let table = radar_research::features::read(std::path::Path::new(&path))
        .map_err(|e| format!("cannot read {path}: {e}"))?;
    let options = Options {
        horizon,
        cost_band: flag(args, "--cost-band").unwrap_or_else(|| edge::DEFAULT_COST_BAND.to_owned()),
        budget: flag(args, "--budget")
            .and_then(|v| v.parse().ok())
            .unwrap_or(edge::DEFAULT_BUDGET),
        noise_seed: flag(args, "--noise-seed").and_then(|v| v.parse().ok()),
    };

    let report = edge::run(&table, &rates, &options).map_err(|e| e.to_string())?;
    present(&report);
    Ok(())
}

/// Prints the report.
fn present(report: &Report) {
    println!("watermark    : slot {}", report.watermark);
    println!("horizon      : {}", report.horizon.label());
    println!("labelled rows: {}", report.labelled_rows);
    println!(
        "cost band    : {} at {:.0} bps round trip (snapshot of {})",
        report.cost_band, report.round_trip_bps, report.rates_measured_on
    );
    println!(
        "bar          : {:.0} bps gross, and {:.0} before a position above about $59",
        report.bar_bps, report.bar_beside_bps
    );
    println!("\nfolds (the first {} are fitted as one):", edge::FIT_FOLDS);
    for (index, fold) in report.folds.iter().enumerate() {
        println!(
            "  {index}  slots {:>12} to {:>12}  {:>7} rows",
            fold.from.get(),
            fold.to.get(),
            fold.rows
        );
    }

    // Beside every verdict, because a winner chosen from this many candidates
    // is expected to regress and the reader needs the number to discount it.
    println!("\nstrata tried : {}", report.strata_tried);
    match report.enumeration {
        Enumeration::Exhaustive => println!("               the whole grammar"),
        Enumeration::StoppedAtBudget => println!(
            "               the budget stopped the search, so \"nothing found\"\n\
             \x20              here means nothing in the part searched"
        ),
    }

    println!("\nfitted:");
    match &report.fitted {
        Some(candidate) => print_candidate(candidate, report.bar_bps),
        None => println!(
            "  nothing in the grammar held enough of the fitting period to be\n  \
             testable, which is a fact about the table rather than a result"
        ),
    }

    println!("\nfixed, not fitted:");
    for candidate in &report.fixed {
        print_candidate(candidate, report.bar_bps);
    }

    println!();
    if report.found {
        println!(
            "FOUND. Something cleared the bar on both test folds. That is an ADR for\n\
             Josh about opening the lane at Canary with capital, not a code change --\n\
             and ADR 0012's threshold becomes urgent in the same change."
        );
    } else {
        println!(
            "Nothing cleared the bar on both test folds. That is the expected result\n\
             and it is a result: the shipped policy still refuses every proposal,\n\
             nothing unfreezes, and the refusal readings above are the number the\n\
             product actually sells."
        );
    }
}

/// Prints one stratum's fit and test readings.
fn print_candidate(candidate: &Candidate, bar: f64) {
    println!("  {}", candidate.stratum.name);
    if candidate.stratum.name != candidate.stratum.describe() {
        println!("    {}", candidate.stratum.describe());
    }
    if let Some(fit) = &candidate.fit {
        println!("    fit   {}", reading(fit, bar));
    }
    for (index, test) in candidate.tests.iter().enumerate() {
        match test {
            Some(r) => println!("    test{index} {}", reading(r, bar)),
            // Rule 9: too few rows is not a bad result, it is no result, and a
            // dash must not be read as a zero return.
            None => println!("    test{index} -- no rows"),
        }
    }
    println!(
        "    {}",
        if candidate.found {
            "cleared both test folds"
        } else {
            "did not clear both test folds"
        }
    );
}

/// One reading, on one line.
fn reading(r: &Reading, bar: f64) -> String {
    format!(
        "n={:>6}  gross={:>9.1}  net={:>9.1}  paid={:>5}/{:<6} wilson>={:.3}  {}",
        r.n,
        r.median_gross,
        r.median_net,
        r.positive,
        r.n,
        r.wilson_lower,
        if r.clears(bar) { "clears" } else { "short" }
    )
}
