// SPDX-License-Identifier: Apache-2.0
//! `radar model-prices` — what to paste into `analyst.env`, from the catalog.
//!
//! # Why a command and not a lookup in the daemon
//!
//! The daemon never reads the catalog. [`radar_model::catalog`]'s module note
//! carries the argument in full; the short version is that a third party's
//! number inside the budget's own accounting would under-count silently when it
//! went stale, which is exactly what the no-default-price rule exists to
//! prevent. So the catalog produces lines an operator pastes, and the number
//! that governs spending is the one in their file: pinned, dated, and theirs.
//!
//! What this removes is not the decision. It is the arithmetic — dollars per
//! million into whole micro-dollars per million — and the transcription, which
//! is the step where a zero goes missing and a bill is under-counted tenfold.
//!
//! # Three pure functions and a thin shell
//!
//! [`asked`], [`lines_for`] and [`drift`] hold every decision this command
//! makes and none of the I/O. [`run`] fetches, reads the environment and
//! prints. That split is not tidiness: CI's mutation check replaced eleven
//! operators inside the argument parsing and the comparison with nothing
//! failing, because both were reachable only through a network call.

use radar_model::catalog::{self, Listed};
use radar_types::MicroUsd;

/// Seconds to wait for the catalog.
const TIMEOUT_SECONDS: u64 = 20;

/// The usage line.
const USAGE: &str = "usage: radar model-prices <model>            the lines to paste
       radar model-prices --list              every model, cheapest first
       radar model-prices [<model>] --check   does the environment still match

--check with no model named reads RADAR_MODEL_NAME.";

/// What the arguments ask for.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Asked {
    /// Every text model, cheapest first.
    List,
    /// Print the lines to paste for this model.
    Lines(String),
    /// Compare the environment against the catalog for this model.
    Check(String),
    /// Nothing usable was named.
    Usage,
}

/// Reads the arguments.
///
/// `env_model` is `RADAR_MODEL_NAME`, supplied rather than read so this can be
/// driven without setting a process-wide variable that parallel tests would
/// fight over — the shape `Prices::from_vars` uses, and for the same reason.
///
/// `--check` with no model named falls back to the environment, because the
/// question it answers is *does what I have configured still match*, and the
/// model is part of what is configured.
#[must_use]
pub fn asked(args: &[String], env_model: Option<&str>) -> Asked {
    // `--list` first: it takes no model, and reading one from the environment
    // would make its output depend on a variable nobody mentioned.
    if args.iter().any(|a| a == "--list") {
        return Asked::List;
    }
    let checking = args.iter().any(|a| a == "--check");
    // The first bare word after the command name. A flag is not a model name,
    // and reading one as such would send `--check` to the catalog and report it
    // missing -- sending an operator to check a spelling they never typed.
    let named = args
        .iter()
        .skip(1)
        .find(|a| !a.starts_with('-'))
        .map(String::as_str)
        .or(if checking { env_model } else { None });

    match (named, checking) {
        (Some(m), true) => Asked::Check(m.to_owned()),
        (Some(m), false) => Asked::Lines(m.to_owned()),
        (None, _) => Asked::Usage,
    }
}

/// Runs the command.
///
/// # Errors
///
/// No model was named, the catalog could not be fetched or read, the model is
/// not in it, or the environment has drifted from it.
pub fn run(args: &[String]) -> Result<(), String> {
    // Read before the fetch. A command that downloads four megabytes and then
    // prints its usage has wasted somebody's time and somebody's bandwidth.
    let asked = asked(args, std::env::var("RADAR_MODEL_NAME").ok().as_deref());
    if asked == Asked::Usage {
        return Err(USAGE.to_owned());
    }

    let document = catalog::fetch(TIMEOUT_SECONDS)?;
    match asked {
        Asked::List => {
            println!("{:>10}  {:>10}  {:<7}  model", "in $/M", "out $/M", "kind");
            for m in catalog::list(&document)? {
                println!(
                    "{:>10}  {:>10}  {:<7}  {}/{}",
                    dollars(m.input),
                    dollars(m.output),
                    if m.reasoning { "reasons" } else { "chat" },
                    m.provider,
                    m.id
                );
            }
            println!("\nsource: {}", catalog::CATALOG);
            Ok(())
        }
        Asked::Lines(model) => {
            print!("{}", lines_for(&catalog::find(&document, &model)?));
            Ok(())
        }
        Asked::Check(model) => {
            let listed = catalog::find(&document, &model)?;
            let read =
                |name: &str| -> Option<u64> { std::env::var(name).ok()?.trim().parse().ok() };
            let found = drift(
                &listed,
                read("RADAR_MODEL_PRICE_IN"),
                read("RADAR_MODEL_PRICE_OUT"),
                &std::env::var("RADAR_MODEL_REASONING_EFFORT").unwrap_or_default(),
            );
            if found.is_empty() {
                println!("{}/{} matches the catalog.", listed.provider, listed.id);
                return Ok(());
            }
            // An error, so a health check or a script sees a non-zero exit
            // rather than having to read the words.
            Err(format!(
                "{}/{} has drifted from {}:\n  {}",
                listed.provider,
                listed.id,
                catalog::CATALOG,
                found.join("\n  ")
            ))
        }
        Asked::Usage => unreachable!("returned above"),
    }
}

/// The block to paste.
///
/// Separated from printing so the exact bytes an operator will paste are what a
/// test asserts on. A command whose output is instructions is a document, and
/// this repository's rule about documents that lag the code applies to it.
#[must_use]
pub fn lines_for(m: &Listed) -> String {
    let mut out = format!(
        "# {}/{} -- ${} per million in, ${} per million out\n\
         # from {}\n\
         #\n\
         # Paste into /etc/radar/analyst.env, replacing any existing\n\
         # RADAR_MODEL_ lines. Restart the unit afterwards.\n\n\
         RADAR_MODEL_NAME={}\n\
         RADAR_MODEL_PRICE_IN={}\n\
         RADAR_MODEL_PRICE_OUT={}\n",
        m.provider,
        m.id,
        dollars(m.input),
        dollars(m.output),
        catalog::CATALOG,
        m.id,
        m.input.get(),
        m.output.get(),
    );
    if m.reasoning {
        // The second variable this lookup decides. Reasoning tokens bill at the
        // output rate and never reach the reply, so a reasoning model asked for
        // three sentences pays for thinking nobody reads -- and can spend the
        // whole ceiling on it, return nothing, and ship the template.
        out.push_str("RADAR_MODEL_REASONING_EFFORT=none\n");
        out.push_str(
            "\n# That last line is not optional for this model. It reasons by default,\n\
             # reasoning tokens bill at the OUTPUT rate, and none of them reach the reply.\n",
        );
    } else {
        out.push_str(
            "\n# Do NOT set RADAR_MODEL_REASONING_EFFORT for this model: it does not\n\
             # reason, and the field is a 400.\n",
        );
    }
    out
}

/// Every way a configuration and the catalog disagree.
///
/// Empty means they match. Takes the three values rather than reading them, so
/// each disagreement can be exercised without a process-wide variable.
#[must_use]
pub fn drift(
    m: &Listed,
    price_in: Option<u64>,
    price_out: Option<u64>,
    reasoning_effort: &str,
) -> Vec<String> {
    let mut found = Vec::new();
    for (name, set, listed) in [
        ("RADAR_MODEL_PRICE_IN", price_in, m.input),
        ("RADAR_MODEL_PRICE_OUT", price_out, m.output),
    ] {
        match set {
            Some(set) if set == listed.get() => {}
            // Both numbers, because "the price is wrong" tells an operator
            // nothing they can act on.
            Some(set) => found.push(format!(
                "{name} is {set}, the catalog says {}",
                listed.get()
            )),
            None => found.push(format!(
                "{name} is unset; the catalog says {}",
                listed.get()
            )),
        }
    }

    // The pairing an operator gets wrong, and it is wrong in both directions.
    let effort = reasoning_effort.trim();
    if m.reasoning && effort.is_empty() {
        found.push(
            "RADAR_MODEL_REASONING_EFFORT is unset and this model reasons by default, \
             so it is billing output tokens that never reach a reply"
                .to_owned(),
        );
    }
    if !m.reasoning && !effort.is_empty() {
        found.push(format!(
            "RADAR_MODEL_REASONING_EFFORT is {effort:?} and this model does not reason, \
             which the provider answers with a 400"
        ));
    }
    found
}

/// Micro-dollars per million, back to the dollars a rate card prints.
///
/// For the human-readable half only. The pasted lines carry the integer, which
/// is what the meter reads and what this conversion cannot round.
fn dollars(m: MicroUsd) -> String {
    // Display only, and every catalog price is far below the point where a
    // `u64` stops being exact in an `f64`.
    #[expect(clippy::cast_precision_loss, reason = "display only; see above")]
    let d = m.get() as f64 / 1_000_000.0;
    format!("{d:.4}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luna() -> Listed {
        Listed {
            provider: "openai".to_owned(),
            id: "gpt-5.6-luna".to_owned(),
            input: MicroUsd(200_000),
            output: MicroUsd(1_200_000),
            reasoning: true,
        }
    }

    fn mini() -> Listed {
        Listed {
            provider: "openai".to_owned(),
            id: "gpt-4o-mini".to_owned(),
            input: MicroUsd(150_000),
            output: MicroUsd(600_000),
            reasoning: false,
        }
    }

    fn args(words: &[&str]) -> Vec<String> {
        std::iter::once("model-prices")
            .chain(words.iter().copied())
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn the_arguments_decide_one_of_four_things() {
        assert_eq!(
            asked(&args(&["gpt-4o-mini"]), None),
            Asked::Lines("gpt-4o-mini".to_owned())
        );
        assert_eq!(asked(&args(&["--list"]), None), Asked::List);
        assert_eq!(
            asked(&args(&["gpt-4o-mini", "--check"]), None),
            Asked::Check("gpt-4o-mini".to_owned())
        );
        assert_eq!(asked(&args(&[]), None), Asked::Usage);
    }

    #[test]
    fn a_flag_is_never_read_as_a_model_name() {
        // The failure it prevents: `--check` alone going to the catalog as a
        // model called "--check" and coming back "no such model", which sends
        // an operator to check a spelling they never typed.
        assert_eq!(
            asked(&args(&["--check"]), Some("gpt-5.6-luna")),
            Asked::Check("gpt-5.6-luna".to_owned()),
            "--check alone falls back to the environment"
        );
        // And with nothing configured there is nothing to check.
        assert_eq!(asked(&args(&["--check"]), None), Asked::Usage);
        // The environment is a fallback, never an override: a model named on
        // the command line is the one the operator meant.
        assert_eq!(
            asked(&args(&["gpt-4o-mini", "--check"]), Some("gpt-5.6-luna")),
            Asked::Check("gpt-4o-mini".to_owned())
        );
    }

    #[test]
    fn listing_takes_no_model_and_ignores_the_environment() {
        assert_eq!(asked(&args(&["--list"]), Some("gpt-5.6-luna")), Asked::List);
        assert_eq!(asked(&args(&["--list", "--check"]), Some("x")), Asked::List);
    }

    #[test]
    fn the_pasted_lines_are_the_whole_configuration_and_nothing_else() {
        // What this command exists to remove is the arithmetic and the
        // transcription. So the assertion is on the exact bytes: a missing zero
        // here under-counts a bill tenfold and nothing downstream would notice.
        let out = lines_for(&luna());
        assert!(out.contains("\nRADAR_MODEL_NAME=gpt-5.6-luna\n"), "{out}");
        assert!(out.contains("\nRADAR_MODEL_PRICE_IN=200000\n"), "{out}");
        assert!(out.contains("\nRADAR_MODEL_PRICE_OUT=1200000\n"), "{out}");
        // Every non-comment line is a variable assignment, so the block can be
        // pasted whole without reading it.
        for line in out.lines().filter(|l| !l.starts_with('#') && !l.is_empty()) {
            assert!(line.contains('='), "not pasteable: {line:?}");
            assert!(line.starts_with("RADAR_MODEL_"), "stray line: {line:?}");
        }
    }

    #[test]
    fn a_reasoning_model_carries_the_line_that_turns_reasoning_off() {
        // And a non-reasoning one must NOT, because the field is a 400 there.
        // Re-apply by emitting it unconditionally: the second assertion fails,
        // and on the box every reply would become the template.
        assert!(lines_for(&luna()).contains("RADAR_MODEL_REASONING_EFFORT=none"));
        assert!(!lines_for(&mini()).contains("RADAR_MODEL_REASONING_EFFORT=none"));
        assert!(
            lines_for(&mini()).contains("Do NOT set"),
            "and it says so rather than staying quiet"
        );
    }

    #[test]
    fn a_configuration_matching_the_catalog_reports_nothing() {
        assert!(drift(&luna(), Some(200_000), Some(1_200_000), "none").is_empty());
        assert!(drift(&mini(), Some(150_000), Some(600_000), "").is_empty());
    }

    #[test]
    fn each_price_drifts_on_its_own_and_says_both_numbers() {
        // Re-apply by comparing with `!=`: a correct configuration reports
        // drift and a wrong one reports none.
        let one = drift(&luna(), Some(20_000), Some(1_200_000), "none");
        assert_eq!(one.len(), 1, "{one:?}");
        assert!(one[0].contains("RADAR_MODEL_PRICE_IN"), "{one:?}");
        assert!(one[0].contains("20000"), "what is set: {one:?}");
        assert!(one[0].contains("200000"), "what it should be: {one:?}");

        let other = drift(&luna(), Some(200_000), Some(999), "none");
        assert_eq!(other.len(), 1, "{other:?}");
        assert!(other[0].contains("RADAR_MODEL_PRICE_OUT"), "{other:?}");

        // Unset is its own message. "is 0" would be a lie about what is there.
        let missing = drift(&luna(), None, None, "none");
        assert_eq!(missing.len(), 2, "{missing:?}");
        assert!(missing.iter().all(|d| d.contains("unset")), "{missing:?}");
    }

    #[test]
    fn the_reasoning_pairing_is_wrong_in_both_directions() {
        // Left unset on a reasoning model it bills output tokens nobody reads;
        // set on a model that cannot reason it is a 400 on every call, and
        // every reply becomes the template.
        //
        // Re-apply by turning either `&&` into `||`, or by deleting either
        // `!`: a correct configuration reports drift.
        let unset = drift(&luna(), Some(200_000), Some(1_200_000), "");
        assert_eq!(unset.len(), 1, "{unset:?}");
        assert!(unset[0].contains("reasons by default"), "{unset:?}");

        let pointless = drift(&mini(), Some(150_000), Some(600_000), "none");
        assert_eq!(pointless.len(), 1, "{pointless:?}");
        assert!(pointless[0].contains("400"), "{pointless:?}");

        // Whitespace is absence, the way it is everywhere else here.
        assert_eq!(
            drift(&luna(), Some(200_000), Some(1_200_000), "   ").len(),
            1
        );
        assert!(drift(&mini(), Some(150_000), Some(600_000), "  ").is_empty());
    }

    #[test]
    fn the_human_readable_price_is_the_rate_card_figure() {
        // The integer is what the meter reads; this is the number an operator
        // checks against the vendor's page, so it has to look like that page.
        assert_eq!(dollars(MicroUsd(200_000)), "0.2");
        assert_eq!(dollars(MicroUsd(1_200_000)), "1.2");
        assert_eq!(dollars(MicroUsd(150_000)), "0.15");
        assert_eq!(dollars(MicroUsd(10_000_000)), "10");
        assert_eq!(dollars(MicroUsd::ZERO), "0");
    }
}
