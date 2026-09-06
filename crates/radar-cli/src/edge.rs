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

use radar_research::edge::{self, Candidate, Cost, Enumeration, Horizon, Options, Reading, Report};
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
        // Unset charges the fresh-launch cohort's measured round trip, which
        // is the population these rows belong to. A band is a sensitivity run.
        cost: flag(args, "--cost-band").map_or(Cost::FreshLaunch, Cost::Band),
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
        "charged      : {:.0} bps round trip -- {} (snapshot of {})",
        report.round_trip_bps, report.cost_source, report.rates_measured_on
    );
    // Printed, and held to by nothing. It is the same measurement as the line
    // above read in the $20-$200 band, so requiring it beside the charge would
    // charge one number twice -- plan 0007 Q2, answered in `edge::Cost`.
    println!(
        "for context  : {:.0} bps is that measurement in the $20-$200 band, which
                        circulates as \"the bar\". It is not a second hurdle here.",
        report.band_bar_bps
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
        Some(candidate) => print_candidate(candidate),
        None => println!(
            "  nothing in the grammar held enough of the fitting period to be\n  \
             testable, which is a fact about the table rather than a result"
        ),
    }

    println!("\nfixed, not fitted:");
    for candidate in &report.fixed {
        print_candidate(candidate);
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
fn print_candidate(candidate: &Candidate) {
    for line in candidate_lines(candidate) {
        println!("{line}");
    }
}

/// The lines one candidate prints as.
///
/// Built rather than printed so the two decisions in here can be tested: a
/// fitted stratum's name already *is* its description and must not be printed
/// twice, and a fold with too few rows says so rather than showing a figure.
fn candidate_lines(candidate: &Candidate) -> Vec<String> {
    let mut lines = vec![format!("  {}", candidate.stratum.name)];
    let described = candidate.stratum.describe();
    if candidate.stratum.name != described {
        lines.push(format!("    {described}"));
    }
    if let Some(fit) = &candidate.fit {
        lines.push(format!("    fit   {}", reading(fit)));
    }
    for (index, test) in candidate.tests.iter().enumerate() {
        lines.push(match test {
            Some(r) => format!("    test{index} {}", reading(r)),
            // Rule 9: too few rows is not a bad result, it is no result, and a
            // dash must not be read as a zero return.
            None => format!("    test{index} -- no rows"),
        });
    }
    lines.push(format!(
        "    {}",
        if candidate.found {
            "cleared both test folds"
        } else {
            "did not clear both test folds"
        }
    ));
    lines
}

/// One reading, on one line.
fn reading(r: &Reading) -> String {
    format!(
        "n={:>6}  gross={:>9.1}  net={:>9.1}  paid={:>5}/{:<6} wilson>={:.3}  {}",
        r.n,
        r.median_gross,
        r.median_net,
        r.positive,
        r.n,
        r.wilson_lower,
        if r.clears() { "clears" } else { "short" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use radar_research::edge::{Stratum, Term};

    fn stratum(name: &str) -> Stratum {
        Stratum::named(
            name,
            vec![Term {
                feature: 0,
                at_least: true,
                threshold: 3.0,
            }],
        )
    }

    fn reading_of(n: usize) -> radar_research::edge::Reading {
        radar_research::edge::Reading {
            n,
            median_gross: 900.0,
            median_net: 650.0,
            positive: n,
            wilson_lower: 0.9,
            se_median: 10.0,
        }
    }

    #[test]
    fn a_fitted_stratum_is_not_described_twice() {
        // A fitted stratum's name *is* its description, and printing both
        // would show the same conjunction on two lines -- which reads as two
        // strata.
        let described = stratum("x").describe();
        let candidate = Candidate {
            stratum: stratum(&described),
            fit: Some(reading_of(400)),
            tests: vec![Some(reading_of(200))],
            found: true,
        };
        let lines = candidate_lines(&candidate);
        assert_eq!(
            lines.iter().filter(|l| l.contains(&described)).count(),
            1,
            "{lines:?}"
        );
    }

    #[test]
    fn a_named_stratum_carries_its_terms_beneath_its_name() {
        // The fixed strata are named for what they mean -- "refused: launching
        // too fast" -- and a reader has to be able to see the thresholds that
        // name stands for.
        let candidate = Candidate {
            stratum: stratum("refused: launching too fast"),
            fit: None,
            tests: vec![None],
            found: false,
        };
        let lines = candidate_lines(&candidate);
        assert!(lines[0].contains("refused: launching too fast"));
        assert!(lines[1].contains("launch_traders >= 3"), "{lines:?}");
        assert!(
            lines.iter().any(|l| l.contains("no rows")),
            "a fold with too few rows says so rather than showing a figure: {lines:?}"
        );
        assert!(lines.last().expect("a verdict").contains("did not clear"));
    }
}
