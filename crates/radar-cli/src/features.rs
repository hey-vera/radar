// SPDX-License-Identifier: Apache-2.0
//! `radar features` — write the table the walk-forward protocol runs over.
//!
//! One row per succeeded launch, every value observed at or before T, written
//! as parquet named by the store's watermark. Plan 0007 item 1; the guarantee
//! it rests on and the reason it is a type rather than a convention are in
//! [`radar_research::features`].
//!
//! Deterministic on purpose: run twice over an unchanged store it produces the
//! same bytes, so a research note can cite a sha256 and mean it.
//!
//! On a production store the trades table is the large read, so `--from` and
//! `--to` are not a convenience — they are how one fold is built at a time.

use radar_asof::AsOf;
use radar_research::features;
use radar_store::Reader;
use radar_types::Slot;

use crate::flag;

/// Builds and writes the feature table.
///
/// # Errors
///
/// A message when the store cannot be read, a feature carried a slot from
/// after its own T, or the file cannot be written.
pub fn run(reader: &Reader, args: &[String]) -> Result<(), String> {
    let watermark = reader
        .watermark()
        .map_err(|e| format!("cannot read the store: {e}"))?
        .ok_or("the store holds no events, so there is nothing to build")?;

    let (from, to) = window(args, watermark)?;

    let out = flag(args, "--out").unwrap_or_else(|| features::file_name(watermark));

    let table = features::build(reader, AsOf::at(watermark), from, to)
        .map_err(|e| format!("cannot build the table: {e}"))?;

    let path = std::path::Path::new(&out);
    features::write(&table, path).map_err(|e| format!("cannot write {out}: {e}"))?;

    let with_6h = table
        .rows
        .iter()
        .filter(|r| r.gross_6h_bps.is_some())
        .count();
    let with_24h = table
        .rows
        .iter()
        .filter(|r| r.gross_24h_bps.is_some())
        .count();

    println!("watermark    : slot {watermark}");
    println!("launch window: {from} to {to}");
    println!("T            : launch + {} slots", table.entry_offset);
    println!("rows         : {}", table.rows.len());
    println!("features     : {}", features::FEATURES.len());
    // Both counts, because the gap between them is what the protocol has to
    // work with. A table of a million rows and four hundred labels is four
    // hundred rows for every purpose that matters.
    println!("labelled 6h  : {with_6h}");
    println!("labelled 24h : {with_24h}");

    // A store that records no trades produces no trade features, and says so
    // rather than filling twelve columns with zeros. Printed because the
    // difference between "nobody traded" and "nothing recorded trades" is
    // invisible in the file and decides what the protocol can be asked.
    let traders = features::feature_index("launch_traders").expect("a known feature");
    let with_trades = table
        .rows
        .iter()
        .filter(|r| r.value(traders).is_some())
        .count();
    println!("trade features: {with_trades} of {} rows", table.rows.len());
    if with_trades == 0 && !table.rows.is_empty() {
        println!(
            "               the trades table covers none of these windows, so the
                            twelve trade-derived features are absent rather than zero"
        );
    }
    println!("written to   : {out}");

    if table.rows.is_empty() {
        println!(
            "\nNo succeeded launches in that window. That is a fact about the window,\n\
             not an error: the file carries the watermark and no rows."
        );
    }
    Ok(())
}

/// The launch window to build, defaulting to the whole store.
///
/// A reversed window selects nothing, and a pass that writes an empty file
/// because two flags were the wrong way round looks exactly like a pass that
/// found nothing -- so it is a refusal, not an empty result.
fn window(args: &[String], watermark: Slot) -> Result<(Slot, Slot), String> {
    let from = slot_flag(args, "--from")?.unwrap_or(Slot(0));
    let to = slot_flag(args, "--to")?.unwrap_or(watermark);
    if from > to {
        return Err(format!(
            "--from {from} is after --to {to}, which selects no launches"
        ));
    }
    Ok((from, to))
}

/// Reads a slot from a flag, refusing a value that is not one.
///
/// A slot that does not parse must not fall back to a default: a typo in
/// `--from` would silently widen the window to all of history, and the pass
/// would look like it worked.
fn slot_flag(args: &[String], name: &str) -> Result<Option<Slot>, String> {
    let Some(raw) = flag(args, name) else {
        return Ok(None);
    };
    raw.parse::<u64>()
        .map(|n| Some(Slot(n)))
        .map_err(|_| format!("{name} {raw} is not a slot"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn a_window_defaults_to_the_whole_store_and_refuses_a_reversed_one() {
        assert_eq!(
            window(&args(["features"].as_slice()), Slot(500)),
            Ok((Slot(0), Slot(500))),
            "no flags is the whole store, up to the watermark"
        );
        assert_eq!(
            window(
                &args(["features", "--from", "10", "--to", "10"].as_slice()),
                Slot(500)
            ),
            Ok((Slot(10), Slot(10))),
            "a single-slot window is a window"
        );
        assert!(
            window(
                &args(["features", "--from", "11", "--to", "10"].as_slice()),
                Slot(500)
            )
            .is_err(),
            "reversed selects nothing, and an empty file is not the way to say so"
        );
    }

    #[test]
    fn a_slot_flag_is_read_or_refused_and_never_defaulted() {
        assert_eq!(
            slot_flag(&args(["features", "--from", "42"].as_slice()), "--from"),
            Ok(Some(Slot(42)))
        );
        assert_eq!(
            slot_flag(&args(["features"].as_slice()), "--from"),
            Ok(None)
        );
        assert!(
            slot_flag(&args(["features", "--from", "soon"].as_slice()), "--from").is_err(),
            "a value that is not a slot must be refused, not silently ignored"
        );
    }
}
