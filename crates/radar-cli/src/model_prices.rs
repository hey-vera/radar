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

use radar_model::catalog::{self, Listed};
use radar_types::MicroUsd;

/// Seconds to wait for the catalog.
const TIMEOUT_SECONDS: u64 = 20;

/// Runs the command.
///
/// # Errors
///
/// The catalog could not be fetched or read, the model is not in it, or no
/// model was named.
pub fn run(args: &[String]) -> Result<(), String> {
    let document = catalog::fetch(TIMEOUT_SECONDS)?;

    if args.iter().any(|a| a == "--list") {
        return list(&document);
    }

    // The first bare word after the command name. A flag-shaped argument is
    // not a model name, and treating one as such would send `--check` to the
    // catalog and report it missing.
    let named = args.iter().skip(1).find(|a| !a.starts_with('-'));
    let model = match named {
        Some(m) => m.clone(),
        None if args.iter().any(|a| a == "--check") => {
            std::env::var("RADAR_MODEL_NAME").map_err(|_| {
                "--check with no model named needs RADAR_MODEL_NAME in the environment".to_owned()
            })?
        }
        None => {
            return Err(
                "usage: radar model-prices <model> | radar model-prices --list | \
                 radar model-prices [<model>] --check"
                    .to_owned(),
            );
        }
    };

    let listed = catalog::find(&document, &model)?;
    if args.iter().any(|a| a == "--check") {
        check(&listed)
    } else {
        print!("{}", lines_for(&listed));
        Ok(())
    }
}

/// Every model, cheapest first.
fn list(document: &str) -> Result<(), String> {
    println!("{:>10}  {:>10}  {:<7}  model", "in $/M", "out $/M", "kind");
    for m in catalog::list(document)? {
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

/// The block to paste, and the sentence that explains it.
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

/// Compares the environment against the catalog.
fn check(m: &Listed) -> Result<(), String> {
    let read = |name: &str| -> Option<u64> { std::env::var(name).ok()?.trim().parse().ok() };
    let mut drift = Vec::new();
    for (name, listed) in [
        ("RADAR_MODEL_PRICE_IN", m.input),
        ("RADAR_MODEL_PRICE_OUT", m.output),
    ] {
        match read(name) {
            Some(set) if set == listed.get() => {}
            Some(set) => drift.push(format!(
                "{name} is {set}, the catalog says {}",
                listed.get()
            )),
            None => drift.push(format!(
                "{name} is unset; the catalog says {}",
                listed.get()
            )),
        }
    }
    let effort = std::env::var("RADAR_MODEL_REASONING_EFFORT").unwrap_or_default();
    if m.reasoning && effort.trim().is_empty() {
        drift.push(
            "RADAR_MODEL_REASONING_EFFORT is unset and this model reasons by default, \
             so it is billing output tokens that never reach a reply"
                .to_owned(),
        );
    }
    if !m.reasoning && !effort.trim().is_empty() {
        drift.push(format!(
            "RADAR_MODEL_REASONING_EFFORT is {effort:?} and this model does not reason, \
             which the provider answers with a 400"
        ));
    }

    if drift.is_empty() {
        println!("{}/{} matches the catalog.", m.provider, m.id);
        return Ok(());
    }
    // An error, so a caller in a script or a health check sees a non-zero exit
    // rather than having to read the words.
    Err(format!(
        "{}/{} has drifted from {}:\n  {}",
        m.provider,
        m.id,
        catalog::CATALOG,
        drift.join("\n  ")
    ))
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
