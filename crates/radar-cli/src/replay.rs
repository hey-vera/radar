// SPDX-License-Identifier: Apache-2.0
//! `radar replay` — record decisions, then prove they reproduce.
//!
//! Two halves of one loop. `--record` writes what the strategy decided about
//! every candidate in a cohort, at the watermark it decided there. `--check`
//! re-derives all of it from the store and reports what moved.
//!
//! The value is entirely in the second run being separated from the first by
//! time and by a process boundary. A determinism check that runs inside the same
//! process that produced the answer mostly proves the machine did not catch
//! fire; one that reads the store fresh, hours later, is checking the thing that
//! actually matters — that a decision made then can be re-derived now, which is
//! the whole premise of replaying history as a backtest.

use std::collections::BTreeMap;
use std::path::Path;

use radar_asof::AsOf;
use radar_research::{Recording, Summary, Verdict, record, replay};
use radar_store::Reader;
use radar_strategy::{CreatorEdge, universe};
use radar_types::Slot;

/// How many candidates a recording run covers by default.
///
/// A cap rather than everything: the point is a representative cohort that can
/// be re-derived quickly, not a second copy of the store.
pub const DEFAULT_COHORT: usize = 200;

/// Records the current cohort's decisions to a file.
///
/// # Errors
///
/// Returns a message if the store cannot be read or the file cannot be written.
pub fn record_to(reader: &Reader, path: &Path, window: u64, cohort: usize) -> Result<(), String> {
    let watermark = reader
        .watermark()
        .map_err(|e| format!("cannot read the store: {e}"))?
        .ok_or("the store has recorded nothing yet")?;
    let as_of = AsOf::at(watermark);
    let strategy = CreatorEdge::default();

    let universe = universe(reader, as_of).map_err(|e| format!("cannot assemble: {e}"))?;
    let recent = recent_mints(&universe, watermark, window, cohort);

    let mut recordings = Vec::with_capacity(recent.len());
    for mint in &recent {
        let Some(candidate) = universe.candidate(mint, None, None) else {
            continue;
        };
        recordings.push(record(&strategy, &candidate).map_err(|e| e.to_string())?);
    }

    let json = serde_json::to_string_pretty(&recordings).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| format!("cannot write {}: {e}", path.display()))?;

    println!(
        "recorded {} decisions at slot {watermark} to {}",
        recordings.len(),
        path.display()
    );
    println!(
        "\nReplay them later with:\n  radar replay --store <dir> --check {}",
        path.display()
    );
    Ok(())
}

/// Replays a recordings file against the store and reports what moved.
///
/// # Errors
///
/// Returns a message if the file cannot be read, or if any recording proved
/// non-deterministic — that is a code bug and the exit status has to say so.
pub fn check(reader: &Reader, path: &Path) -> Result<(), String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let recordings: Vec<Recording> =
        serde_json::from_str(&raw).map_err(|e| format!("cannot parse {}: {e}", path.display()))?;

    if recordings.is_empty() {
        return Err(format!("{} holds no recordings", path.display()));
    }

    let strategy = CreatorEdge::default();
    let mut summary = Summary::default();
    let mut moved: Vec<(&Recording, Verdict)> = Vec::new();

    // Grouped by watermark so the universe is assembled once per distinct slot
    // rather than once per recording. Assembling it per recording would be the
    // same answer at a hundred times the cost.
    let mut by_watermark: BTreeMap<Slot, Vec<&Recording>> = BTreeMap::new();
    for r in &recordings {
        by_watermark.entry(r.as_of).or_default().push(r);
    }

    for (slot, group) in &by_watermark {
        let as_of = AsOf::at(*slot);
        let universe = universe(reader, as_of).map_err(|e| format!("cannot assemble: {e}"))?;

        for recording in group {
            let Some(candidate) = universe.candidate(&recording.mint, None, None) else {
                // An append-only store that has lost a launch it already had is
                // not a provenance nuance, it is data loss.
                summary.count_gone();
                println!(
                    "  GONE          {}  no longer in the store as of slot {slot}",
                    recording.mint
                );
                continue;
            };
            let verdict = replay(recording, &strategy, &candidate).map_err(|e| e.to_string())?;
            summary.count(&verdict);
            if verdict.needs_review() {
                moved.push((recording, verdict));
            }
        }
    }

    report(&summary, &moved, by_watermark.len());

    if summary.is_failure() {
        return Err(
            "replay failed: a decision moved while its inputs did not, or a candidate \
             vanished from an append-only store"
                .to_owned(),
        );
    }
    Ok(())
}

/// Prints the outcome.
fn report(summary: &Summary, moved: &[(&Recording, Verdict)], watermarks: usize) {
    println!(
        "replayed {} recordings across {watermarks} watermark(s)",
        summary.total()
    );
    println!("  identical         : {}", summary.identical);
    println!("  inputs changed    : {}", summary.inputs_changed);
    println!("  not deterministic : {}", summary.not_deterministic);
    if summary.candidate_gone > 0 {
        println!("  candidate gone    : {}", summary.candidate_gone);
    }

    for (recording, verdict) in moved.iter().take(20) {
        match verdict {
            Verdict::InputsChanged {
                was,
                now,
                decision_moved,
            } => {
                println!(
                    "  INPUTS CHANGED  {}  {} -> {}  (decision {})",
                    recording.mint,
                    was.short(),
                    now.short(),
                    if *decision_moved { "moved" } else { "held" }
                );
            }
            Verdict::NotDeterministic { was, now } => {
                println!("  NOT DETERMINISTIC  {}", recording.mint);
                println!("      was: {was}");
                println!("      now: {now}");
            }
            Verdict::Identical => {}
        }
    }

    if summary.not_deterministic > 0 {
        println!(
            "\nA decision moved while its inputs did not. That is not a data\n\
             question — the strategy is not a pure function of its inputs, and\n\
             every backtest over it is measuring the strategy plus whatever else\n\
             it is reading."
        );
    } else if summary.inputs_changed > 0 {
        println!(
            "\nInputs moved but the code held. Expected while a backfill is still\n\
             filling in history, or after a repair that added events at slots the\n\
             watermark already covered. Not expected once a slot range is settled —\n\
             if this range was supposed to be closed, the store gained data it\n\
             should not have."
        );
    }
}

/// The most recently launched mints, newest first, capped.
fn recent_mints(
    universe: &radar_strategy::Universe,
    watermark: Slot,
    window: u64,
    cohort: usize,
) -> Vec<radar_types::Address> {
    let floor = watermark.get().saturating_sub(window);
    let mut recent: Vec<(Slot, radar_types::Address)> = universe
        .launches
        .iter()
        .filter(|(_, facts)| facts.slot.get() >= floor)
        .map(|(mint, facts)| (facts.slot, *mint))
        .collect();
    // Newest first, and by mint where slots tie, so the cohort a run picks is a
    // function of the store rather than of map iteration order. A cohort that
    // varies between runs would make every replay report changed inputs.
    recent.sort_unstable_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    recent.truncate(cohort);
    recent.into_iter().map(|(_, mint)| mint).collect()
}
