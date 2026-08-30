// SPDX-License-Identifier: Apache-2.0
//! Positions, written and read back off disk.
//!
//! `position.rs` unit-tests the fold over hand-built rows. This is the half that
//! needs files: that a written position comes back as itself, that the watermark
//! applies to it the way it applies to every other read, and that buffering does
//! not lose one.

use radar_asof::AsOf;
use radar_store::{Position, Reader, Writer, fold_positions};
use radar_types::{Address, Slot};

fn open(mint: u8, opened_at: u64, notional: u64) -> Position {
    Position {
        mint: Address::new([mint; 32]),
        creator: Address::new([200u8; 32]),
        opened_at: Slot(opened_at),
        notional_micro_usd: notional,
        entry_price: Some(1_000),
        closed_at: None,
        exit_price: None,
        realised_micro_usd: None,
    }
}

fn closed_at(mut position: Position, at: u64, realised: i64) -> Position {
    position.closed_at = Some(Slot(at));
    position.exit_price = Some(870);
    position.realised_micro_usd = Some(realised);
    position
}

/// Writes rows and returns the directory holding them.
fn store_with(rows: Vec<Position>) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut writer = Writer::open(dir.path(), 64).expect("open");
    for row in rows {
        writer.append_position(row).expect("append");
    }
    writer.flush().expect("flush");
    dir
}

#[test]
fn a_written_position_comes_back_as_itself() {
    // Every field, including the three that are `None` on an open position.
    // A column that silently read back as zero would turn "not recorded" into
    // "free", and a return computed against a free entry reports a total loss.
    let original = open(1, 10_000, 5_000_000);
    let dir = store_with(vec![original.clone()]);

    let read = Reader::open(dir.path())
        .read_positions(AsOf::at(Slot(20_000)))
        .expect("read");
    assert_eq!(read, vec![original]);
}

#[test]
fn a_closed_position_keeps_its_signed_loss() {
    // The realised amount is the only signed column in the store. A width or
    // sign error here would be invisible until a daily-loss limit failed to
    // bind.
    let original = closed_at(open(1, 10_000, 5_000_000), 12_000, -650_000);
    let dir = store_with(vec![original.clone()]);

    let read = Reader::open(dir.path())
        .read_positions(AsOf::at(Slot(20_000)))
        .expect("read");
    assert_eq!(read, vec![original.clone()]);
    assert_eq!(read[0].realised_micro_usd, Some(-650_000));
    assert_eq!(read[0].loss_micro_usd(), 650_000);
}

#[test]
fn the_watermark_applies_to_positions_the_way_it_applies_to_everything() {
    // AGENTS.md rule 3. A position opened after the watermark did not exist as
    // of it, and a decision that could see it would be informed by its own
    // future.
    let dir = store_with(vec![
        open(1, 10_000, 1_000_000),
        open(2, 15_000, 2_000_000),
        open(3, 30_000, 3_000_000),
    ]);
    let reader = Reader::open(dir.path());

    let early = reader.read_positions(AsOf::at(Slot(15_000))).expect("read");
    assert_eq!(
        early.len(),
        2,
        "inclusive at the watermark itself: {early:?}"
    );
    assert!(early.iter().all(|p| p.opened_at <= Slot(15_000)));

    let earlier = reader.read_positions(AsOf::at(Slot(14_999))).expect("read");
    assert_eq!(earlier.len(), 1, "one slot earlier excludes the second");

    let all = reader.read_positions(AsOf::at(Slot(40_000))).expect("read");
    assert_eq!(all.len(), 3);

    let before_everything = reader.read_positions(AsOf::at(Slot(1))).expect("read");
    assert!(before_everything.is_empty());
}

#[test]
fn buffering_does_not_lose_a_position() {
    // The writer buffers until a flush threshold and then writes. A count that
    // did not advance, or a threshold compared the wrong way, loses rows
    // silently -- and a lost position is exposure the kernel cannot see.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut writer = Writer::open(dir.path(), 4).expect("open");
    for i in 0..10u8 {
        writer
            .append_position(open(i, 10_000 + u64::from(i), 1_000_000))
            .expect("append");
    }
    writer.flush().expect("flush");

    let read = Reader::open(dir.path())
        .read_positions(AsOf::at(Slot(40_000)))
        .expect("read");
    assert_eq!(read.len(), 10, "every row survives the flush threshold");
    assert_eq!(
        writer.written_rows(),
        10,
        "and the writer's own count agrees"
    );
}

#[test]
fn a_close_written_after_an_open_supersedes_it_across_files() {
    // The append-only contract, end to end: two writes, two flushes, and the
    // fold resolves them. Across files is the case that matters -- within one
    // batch the order is obvious, and across files it is not.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut writer = Writer::open(dir.path(), 64).expect("open");

    let opened = open(1, 10_000, 5_000_000);
    writer.append_position(opened.clone()).expect("append");
    writer.flush().expect("flush");

    let shut = closed_at(opened, 12_000, -650_000);
    writer.append_position(shut.clone()).expect("append");
    writer.flush().expect("flush");

    let rows = Reader::open(dir.path())
        .read_positions(AsOf::at(Slot(20_000)))
        .expect("read");
    assert_eq!(rows.len(), 2, "both rows are on disk; the read is a read");

    let folded = fold_positions(rows);
    assert_eq!(folded, vec![shut], "and the fold resolves them");
    assert!(!folded[0].is_open());
}

#[test]
fn the_last_close_written_is_the_one_that_stands() {
    // Two closes for one position should not happen, and if they do the later
    // row is the correction. Keeping the first would make a correction
    // unwritable -- an append-only store's only way to fix a row is to append
    // a better one.
    let opened = open(1, 10_000, 5_000_000);
    let first = closed_at(opened.clone(), 12_000, -650_000);
    let corrected = closed_at(opened, 12_000, -420_000);

    let folded = fold_positions(vec![first, corrected.clone()]);
    assert_eq!(folded, vec![corrected]);
}

#[test]
fn an_empty_store_holds_no_positions_rather_than_failing() {
    // A fresh instance. Nothing writes a position yet, so this is the state
    // every deployment is in today, and it must read as "none" rather than as
    // an error -- the caller treats an error as none, and the two agreeing is
    // what makes that safe.
    let dir = tempfile::tempdir().expect("tempdir");
    let read = Reader::open(dir.path())
        .read_positions(AsOf::at(Slot(10_000)))
        .expect("an empty store is readable");
    assert!(read.is_empty());
}
