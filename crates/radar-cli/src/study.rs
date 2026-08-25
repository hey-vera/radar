// SPDX-License-Identifier: Apache-2.0
//! `radar study` — does the creator signal predict anything?
//!
//! Prints the event study. The output is deliberately shaped so the honest
//! answer and the flattering one look different: every group rate sits beside
//! the population rate it has to beat, and a group too small to speak for prints
//! its counts with no rate at all.

use radar_asof::AsOf;
use radar_research::study::{self, Group, MIN_GROUP, MIN_PRIOR_LAUNCHES};
use radar_store::Reader;
use radar_types::Slot;

/// Where the pivot goes when the caller does not choose one.
///
/// Halfway between the **first outcome measurement** and the head — not halfway
/// through the recorded slot range, which is what this did first and which was
/// wrong in a way that produced a confident empty answer.
///
/// The store holds launches from well before its first measurement, so the
/// midpoint of the slot range landed before anything had been measured. Every
/// creator then had an empty prior, and the table read as "creator history
/// predicts nothing" when it actually said "nothing had been measured yet".
/// A pivot is only useful where knowledge existed.
fn default_pivot(first_measurement: Slot, head: Slot) -> Slot {
    Slot(first_measurement.get() + (head.get().saturating_sub(first_measurement.get())) / 2)
}

/// Runs the study and prints it.
///
/// # Errors
///
/// Returns a message if the store cannot be read or holds nothing.
pub fn run(reader: &Reader, pivot: Option<u64>) -> Result<(), String> {
    let head = reader
        .watermark()
        .map_err(|e| format!("cannot read the store: {e}"))?
        .ok_or("the store has recorded nothing yet")?;
    let earliest = reader
        .earliest()
        .map_err(|e| format!("cannot read the store: {e}"))?
        .unwrap_or(head);
    let first_measurement = reader
        .read_outcomes(AsOf::at(head))
        .map_err(|e| format!("cannot read outcomes: {e}"))?
        .iter()
        .map(|o| o.measured_at)
        .min();

    let pivot = match (pivot, first_measurement) {
        (Some(p), _) => Slot(p),
        (None, Some(first)) => default_pivot(first, head),
        (None, None) => {
            return Err("the store holds no outcome measurements, so there is nothing a                         prior could be built from"
                .to_owned());
        }
    };
    let study = study::run(reader, pivot, head).map_err(|e| format!("cannot study: {e}"))?;

    println!("store spans   : slot {earliest} .. {head}");
    println!("pivot         : slot {pivot}");
    println!(
        "creators      : {} with >= {MIN_PRIOR_LAUNCHES} launches before the pivot and at \
         least one after",
        study.creators
    );
    println!(
        "prior coverage: {} of {} pre-pivot launches had been measured by then",
        study.prior_measured, study.prior_launches
    );
    println!(
        "later launches: {} of which {} graduated organically",
        study.later_launches, study.later_organic
    );

    // Refused before the table is printed, not after. A table showing every
    // creator in "no organic graduation known" reads as a finding about
    // creators, and it is not one when nothing had been measured.
    if !study.prior_is_informative() {
        println!(
            "
Nothing had been measured about these creators by slot {pivot}, so every
             prior is empty and the grouping below would be an artefact. Choose a pivot
             inside the measured window with --pivot, or wait for the outcome pass to
             cover more of the record.

             This is the difference between 'creator history predicts nothing' and 'we
             had not looked yet', and they are not distinguishable from the table alone."
        );
        return Ok(());
    }

    let Some(base) = study.base_rate_bps() else {
        println!(
            "\nNo later launches from any creator with a prior record. The study has\n\
             nothing to compare, which is a fact about how much chain has been\n\
             recorded rather than about the signal."
        );
        return Ok(());
    };
    println!("base rate     : {}", bps(base));

    println!("\nWhat creators did next, by what was known about them at the pivot:\n");
    println!(
        "  {:<30}  {:>8}  {:>8}  {:>8}  RATE",
        "KNOWN AT PIVOT", "CREATORS", "LAUNCHES", "ORGANIC"
    );
    for group in &study.groups {
        println!(
            "  {:<30}  {:>8}  {:>8}  {:>8}  {}",
            group.label,
            group.creators,
            group.later_launches,
            group.later_organic,
            match (group.later_organic_bps(), group.later_organic_ci_bps()) {
                (Some(r), Some((lo, hi))) =>
                    format!("{} [{} – {}]{}", bps(r), bps(lo), bps(hi), lift(r, base)),
                _ => format!("(under {MIN_GROUP} creators; no rate stated)"),
            }
        );
    }

    verdict(&study.groups);
    Ok(())
}

/// Says what the table supports, and refuses to say more.
fn verdict(groups: &[Group]) {
    let Some(without) = groups
        .iter()
        .find(|g| g.label == "no organic graduation known")
    else {
        return;
    };
    let best = groups
        .iter()
        .filter(|g| g.label != "no organic graduation known")
        .filter(|g| g.later_organic_bps().is_some())
        .max_by_key(|g| g.later_organic_bps().unwrap_or(0));

    let Some(best) = best else {
        println!(
            "
No group of creators with a prior organic graduation is large enough to"
        );
        println!("state a rate for, so there is nothing to compare. Not evidence either way.");
        return;
    };

    if best.clearly_above(without) {
        println!(
            "
Creators with a prior organic graduation went on to graduate more often,"
        );
        println!("and the two intervals do not overlap — so this is a separation rather than a");
        println!("gap between two noisy midpoints. It is the direction `creator_edge` assumes.");
        println!();
        println!("What it still is not: this is one window of a few days, the better group is");
        println!(
            "{} creators, and nothing here controls for how often a creator launches — a",
            best.creators
        );
        println!("creator with four hundred launches has four hundred chances. Treat it as the");
        println!("first evidence the rule is not arbitrary, not as its validation.");
    } else {
        println!(
            "
The intervals overlap, so the difference between these groups has not been"
        );
        println!("shown. Their midpoints may differ and that is not the same thing. On this");
        println!("much data that is the expected answer, and it is not evidence against the");
        println!("rule either.");
    }
}

/// A rate in basis points, rendered as a percentage a person can read.
fn bps(v: u64) -> String {
    #[expect(clippy::cast_precision_loss, reason = "display of a small integer")]
    let pct = v as f64 / 100.0;
    format!("{pct:.2}%")
}

/// How a group compares with the base rate, when the comparison is meaningful.
fn lift(rate: u64, base: u64) -> String {
    if base == 0 {
        return String::new();
    }
    #[expect(clippy::cast_precision_loss, reason = "display of a small integer")]
    let ratio = rate as f64 / base as f64;
    format!("  ({ratio:.2}x)")
}
