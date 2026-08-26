// SPDX-License-Identifier: Apache-2.0
//! The store's guarantees, exercised against real files on disk.

use radar_asof::{AsOf, PointInTime};
use radar_store::{
    Envelope, Event, Graduation, Launch, Origin, Outcome, Reader, SLOTS_PER_PARTITION, Side, Table,
    Trade, Writer,
};
use radar_types::{Address, Signature, Slot};

fn envelope(slot: u64, tx_index: u32) -> Envelope {
    Envelope {
        slot: Slot(slot),
        signature: Signature::new([(slot % 251) as u8; 64]),
        tx_index,
        instruction_index: 1,
        parent_index: None,
        succeeded: true,
    }
}

fn pumpfun() -> Address {
    "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P"
        .parse()
        .expect("program id")
}

fn mint(n: u8) -> Address {
    Address::new([n; 32])
}

fn trade(slot: u64, tx_index: u32, realised: Option<u64>) -> Event {
    Event::Trade(Box::new(Trade {
        envelope: envelope(slot, tx_index),
        origin: Origin::known(pumpfun(), "buy"),
        mint: mint(9),
        trader: mint(8),
        side: Side::Buy,
        realised_lamports: realised,
        realised_tokens: Some(1_234_567),
        requested_amount: 150_000_000,
        requested_is_lamports: true,
        limit_amount: 0,
        accepted_any_price: true,
    }))
}

fn launch(slot: u64) -> Event {
    Event::Launch(Box::new(Launch {
        envelope: envelope(slot, 0),
        origin: Origin::known(pumpfun(), "create_v2"),
        mint: mint(1),
        creator: mint(2),
        // Deliberately awkward: creator-supplied text is arbitrary bytes and the
        // store must carry it back unchanged.
        name: "p down 🚀 \"quoted\", comma".to_owned(),
        symbol: "P\u{200b}D".to_owned(),
        uri: "https://ipfs.io/ipfs/QmZGgrcDjuuWCmtDGqA87Zv2zYKtkSnr1BH7CTM6HRiyno".to_owned(),
        dev_buy_lamports: Some(500_000_000),
    }))
}

#[test]
fn events_round_trip_through_parquet_unchanged() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut w = Writer::open(dir.path(), 1_000).expect("open");

    let written = vec![
        launch(100),
        trade(101, 5, Some(149_000_000)),
        trade(102, 9, None),
    ];
    for e in &written {
        w.append(e.clone()).expect("append");
    }
    w.flush().expect("flush");

    let r = Reader::open(dir.path());
    let mut read = r
        .read(Table::Launches, AsOf::at(Slot(1_000)))
        .expect("read launches");
    read.extend(
        r.read(Table::Trades, AsOf::at(Slot(1_000)))
            .expect("read trades"),
    );

    assert_eq!(read.len(), written.len());
    for e in &written {
        assert!(
            read.contains(e),
            "event did not survive the round trip: {e:?}"
        );
    }
}

#[test]
fn an_unrecoverable_amount_reads_back_as_none_not_zero() {
    // The single most important null in the schema. Zero would report every
    // trade whose deltas could not be resolved as free, flattering execution
    // cost exactly where it is least known.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut w = Writer::open(dir.path(), 1).expect("open");
    w.append(trade(500, 1, None)).expect("append");
    w.flush().expect("flush");

    let events = Reader::open(dir.path())
        .read(Table::Trades, AsOf::at(Slot(999)))
        .expect("read");
    let Event::Trade(t) = &events[0] else {
        panic!("expected a trade")
    };
    assert_eq!(t.realised_lamports, None);
}

#[test]
fn creator_supplied_text_survives_verbatim() {
    // Emoji, a zero-width space, quotes and a comma. Any of these silently
    // mangled would corrupt the one field an operator uses to recognise a token.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut w = Writer::open(dir.path(), 1).expect("open");
    let original = launch(700);
    w.append(original.clone()).expect("append");
    w.flush().expect("flush");

    let events = Reader::open(dir.path())
        .read(Table::Launches, AsOf::at(Slot(999)))
        .expect("read");
    assert_eq!(events[0], original);
}

#[test]
fn the_reader_refuses_events_past_the_watermark() {
    // The point-in-time guarantee reaching onto disk. A reader that returns rows
    // past the watermark and trusts callers to filter will eventually leak the
    // future into a replay.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut w = Writer::open(dir.path(), 100).expect("open");
    for slot in [10u64, 20, 30, 40] {
        w.append(trade(slot, 1, Some(1))).expect("append");
    }
    w.flush().expect("flush");

    let r = Reader::open(dir.path());
    let visible = r.read(Table::Trades, AsOf::at(Slot(25))).expect("read");
    assert_eq!(
        visible.len(),
        2,
        "only slots 10 and 20 are admissible as of 25"
    );
    assert!(visible.iter().all(|e| e.slot() <= Slot(25)));
}

#[test]
fn events_read_back_in_chain_order() {
    // Coordination analysis depends on same-slot ordering, so the reader has to
    // restore it regardless of the order rows were written or files were listed.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut w = Writer::open(dir.path(), 100).expect("open");
    for (slot, idx) in [(20u64, 3u32), (10, 9), (20, 1), (10, 2)] {
        w.append(trade(slot, idx, Some(1))).expect("append");
    }
    w.flush().expect("flush");

    let events = Reader::open(dir.path())
        .read(Table::Trades, AsOf::at(Slot(99)))
        .expect("read");
    let order: Vec<(u64, u32)> = events
        .iter()
        .map(|e| (e.slot().get(), e.envelope().tx_index))
        .collect();
    assert_eq!(order, vec![(10, 2), (10, 9), (20, 1), (20, 3)]);
}

#[test]
fn a_second_flush_adds_a_generation_rather_than_overwriting() {
    // Append-only by construction. A store that can overwrite is one bug away
    // from losing a day of recording that cannot be bought back.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut w = Writer::open(dir.path(), 1_000).expect("open");

    w.append(trade(5, 1, Some(1))).expect("append");
    w.flush().expect("flush");
    w.append(trade(6, 1, Some(2))).expect("append");
    w.flush().expect("flush");

    let r = Reader::open(dir.path());
    let files = r.files(Table::Trades).expect("files");
    assert_eq!(files.len(), 2, "same partition, two generations");
    assert_eq!(
        r.read(Table::Trades, AsOf::at(Slot(99)))
            .expect("read")
            .len(),
        2
    );
}

#[test]
fn slots_land_in_the_partition_named_after_them() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut w = Writer::open(dir.path(), 1_000).expect("open");
    w.append(trade(1, 1, Some(1))).expect("append");
    w.append(trade(SLOTS_PER_PARTITION + 1, 1, Some(1)))
        .expect("append");
    w.flush().expect("flush");

    let files = Reader::open(dir.path())
        .files(Table::Trades)
        .expect("files");
    assert_eq!(files.len(), 2, "two partitions");
    let names: Vec<String> = files
        .iter()
        .map(|p| p.file_name().expect("name").to_string_lossy().into())
        .collect();
    assert!(names[0].starts_with("slot_000000000000"), "{names:?}");
    assert!(
        names[1].contains(&format!("{SLOTS_PER_PARTITION:012}")),
        "{names:?}"
    );
}

#[test]
fn the_watermark_reports_the_highest_slot_held() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut w = Writer::open(dir.path(), 1_000).expect("open");
    for slot in [10u64, 5_000, 77] {
        w.append(trade(slot, 1, Some(1))).expect("append");
    }
    w.append(launch(9_999)).expect("append");
    w.flush().expect("flush");

    let r = Reader::open(dir.path());
    assert_eq!(Reader::watermark(&r).expect("watermark"), Some(Slot(9_999)));
    assert!(r.can_answer(AsOf::at(Slot(9_999))).expect("can answer"));
    assert!(!r.can_answer(AsOf::at(Slot(10_000))).expect("can answer"));
}

#[test]
fn an_empty_store_cannot_answer_rather_than_answering_nothing() {
    // Distinct outcomes: "I have no data for that slot" and "nothing happened in
    // that slot" look identical downstream and mean opposite things.
    let dir = tempfile::tempdir().expect("tempdir");
    Writer::open(dir.path(), 10).expect("open");
    let r = Reader::open(dir.path());
    assert_eq!(Reader::watermark(&r).expect("watermark"), None);
    assert!(PointInTime::watermark(&r).is_err());
}

#[test]
fn dropping_the_writer_flushes_rather_than_losing_the_buffer() {
    // Losing buffered events on shutdown leaves a gap indistinguishable from a
    // quiet market.
    let dir = tempfile::tempdir().expect("tempdir");
    {
        let mut w = Writer::open(dir.path(), 1_000_000).expect("open");
        w.append(trade(42, 1, Some(1))).expect("append");
        assert_eq!(w.buffered(), 1);
    }
    let events = Reader::open(dir.path())
        .read(Table::Trades, AsOf::at(Slot(99)))
        .expect("read");
    assert_eq!(events.len(), 1);
}

#[test]
fn graduations_round_trip() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut w = Writer::open(dir.path(), 1).expect("open");
    let g = Event::Graduation(Box::new(Graduation {
        envelope: envelope(300, 4),
        origin: Origin::known(pumpfun(), "migrate_v2"),
        mint: mint(3),
    }));
    w.append(g.clone()).expect("append");
    w.flush().expect("flush");

    let events = Reader::open(dir.path())
        .read(Table::Graduations, AsOf::at(Slot(999)))
        .expect("read");
    assert_eq!(events, vec![g]);
}

#[test]
fn unknown_instructions_are_stored_and_stay_queryable() {
    // The unknown rate is the program-upgrade alarm, so it has to survive a
    // round trip through disk.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut w = Writer::open(dir.path(), 1).expect("open");
    let mut e = trade(88, 1, None);
    if let Event::Trade(t) = &mut e {
        t.origin = Origin::unknown(pumpfun(), "577c34bf3426d6e8");
    }
    w.append(e).expect("append");
    w.flush().expect("flush");

    let events = Reader::open(dir.path())
        .read(Table::Trades, AsOf::at(Slot(999)))
        .expect("read");
    let Event::Trade(t) = &events[0] else {
        panic!()
    };
    assert!(!t.origin.known);
    assert_eq!(t.origin.instruction, "577c34bf3426d6e8");
}

// --- outcomes ----------------------------------------------------------------

/// Writes an outcomes file in the shape the store used *before* prices existed,
/// bypassing the writer so the old schema is reproduced exactly.
///
/// The point is to hold a guarantee that cannot be checked any other way: the
/// live store held 29 outcome files, 142,826 measurements, written without these
/// columns. A reader that demanded them would have made every one unreadable.
fn write_pre_price_outcomes(dir: &std::path::Path) -> std::path::PathBuf {
    use arrow::array::{StringBuilder, UInt64Builder};
    use arrow::datatypes::{DataType, Field, Schema};
    use std::sync::Arc;

    let schema = Arc::new(Schema::new(vec![
        Field::new("mint", DataType::Utf8, false),
        Field::new("measured_at", DataType::UInt64, false),
        Field::new("launch_slot", DataType::UInt64, false),
        Field::new("first_transfer_slot", DataType::UInt64, true),
        Field::new("last_transfer_slot", DataType::UInt64, true),
        Field::new("transfers", DataType::UInt64, false),
        Field::new("unique_senders", DataType::UInt64, false),
        Field::new("unique_receivers", DataType::UInt64, false),
        Field::new("graduated_at", DataType::UInt64, true),
    ]));

    let mut mint_b = StringBuilder::new();
    let mut cols: Vec<UInt64Builder> = (0..8).map(|_| UInt64Builder::new()).collect();
    mint_b.append_value(mint(1).to_string());
    cols[0].append_value(500_000); // measured_at
    cols[1].append_value(1_000); // launch_slot
    cols[2].append_value(1_001); // first_transfer_slot
    cols[3].append_value(1_500); // last_transfer_slot
    cols[4].append_value(42); // transfers
    cols[5].append_value(7); // unique_senders
    cols[6].append_value(5); // unique_receivers
    cols[7].append_null(); // graduated_at

    let mut arrays: Vec<arrow::array::ArrayRef> = vec![Arc::new(mint_b.finish())];
    for c in &mut cols {
        arrays.push(Arc::new(c.finish()));
    }
    let batch = arrow::record_batch::RecordBatch::try_new(schema.clone(), arrays).expect("batch");

    let outcomes = dir.join("outcomes");
    std::fs::create_dir_all(&outcomes).expect("mkdir");
    let path = outcomes.join("000000000000000000-legacy.parquet");
    let file = std::fs::File::create(&path).expect("create");
    let mut w = parquet::arrow::ArrowWriter::try_new(file, schema, None).expect("writer");
    w.write(&batch).expect("write");
    w.close().expect("close");
    path
}

#[test]
fn outcomes_written_before_prices_existed_are_still_readable() {
    // Verified against the live store when the columns were added: 142,826
    // measurements across 29 files, none of which carry a price column. A
    // reader that required them would have failed on every one, and the failure
    // would have looked like a corrupted store rather than a schema change.
    let dir = tempfile::tempdir().expect("tempdir");
    write_pre_price_outcomes(dir.path());

    let rows = Reader::open(dir.path())
        .read_outcomes(AsOf::at(Slot(1_000_000)))
        .expect("an older file must still read");

    assert_eq!(rows.len(), 1, "the row survived the schema change");
    let o = &rows[0];
    assert_eq!(o.transfers, 42, "the old columns still arrive intact");
    assert_eq!(o.unique_senders, 7);

    // Absent, not zero. A file that never carried a price is not a token that
    // traded at nothing, and every derived figure must decline to answer.
    assert_eq!(o.first_price, None);
    assert_eq!(o.peak_price, None);
    assert_eq!(o.vwap, None);
    assert_eq!(o.fills, 0);
    assert_eq!(
        o.mfe_bps(),
        None,
        "no price means no excursion, not a flat one"
    );
    assert_eq!(o.mae_bps(), None);
    assert_eq!(o.held_to_end_gain_bps(), None);
}

#[test]
fn a_price_path_round_trips() {
    // The other direction: what the writer emits, the reader returns.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut o = outcome(2, 500_000, 1_000, Some(1_500), 42);
    o.first_price = Some(21_000_000_000_000);
    o.last_price = Some(30_296_236_543);
    o.peak_price = Some(68_758_447_614_858);
    o.trough_price = Some(1_886_773_264_633);
    o.vwap = Some(21_002_820_020_797);
    o.fills = 45;

    let mut w = Writer::open(dir.path(), 1).expect("writer");
    w.append_outcome(o.clone()).expect("append");
    w.flush().expect("flush");

    let back = Reader::open(dir.path())
        .read_outcomes(AsOf::at(Slot(1_000_000)))
        .expect("read");
    assert_eq!(back.len(), 1);
    assert_eq!(back[0], o, "every price field survives the round trip");
}

fn outcome(mint_id: u8, measured: u64, launch: u64, last: Option<u64>, transfers: u64) -> Outcome {
    Outcome {
        mint: mint(mint_id),
        measured_at: Slot(measured),
        launch_slot: Slot(launch),
        first_transfer_slot: last.map(|_| Slot(launch)),
        last_transfer_slot: last.map(Slot),
        transfers,
        unique_senders: 7,
        unique_receivers: 5,
        graduated_at: None,
        first_price: None,
        last_price: None,
        peak_price: None,
        trough_price: None,
        vwap: None,
        fills: 0,
    }
}

#[test]
fn outcomes_round_trip_through_parquet() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut w = Writer::open(dir.path(), 1_000).expect("open");
    let written = vec![
        outcome(1, 500_000, 440_623_612, Some(440_998_194), 1_535),
        outcome(2, 500_000, 440_624_864, Some(440_624_868), 3),
        // Never traded: both transfer slots absent, which must not read as zero.
        outcome(3, 500_000, 440_624_900, None, 0),
        // Graduated in its own launch block, and graduated five minutes later.
        // Both must survive as slots: collapsed to a boolean they are the same
        // row, and that collapse is what the split exists to undo.
        Outcome {
            graduated_at: Some(Slot(440_624_864)),
            ..outcome(4, 500_000, 440_624_864, Some(440_624_870), 210)
        },
        Outcome {
            graduated_at: Some(Slot(440_625_700)),
            ..outcome(5, 500_000, 440_624_864, Some(440_700_000), 980)
        },
    ];
    for o in &written {
        w.append_outcome(o.clone()).expect("append");
    }
    w.flush().expect("flush");

    let read = Reader::open(dir.path())
        .read_outcomes(AsOf::at(Slot(999_999)))
        .expect("read");
    assert_eq!(read.len(), 5);
    for o in &written {
        assert!(
            read.contains(o),
            "outcome did not survive the round trip: {o:?}"
        );
    }
    let never_traded = read
        .iter()
        .find(|o| o.transfers == 0)
        .expect("the untraded one");
    assert_eq!(never_traded.last_transfer_slot, None, "absent, not zero");
}

#[test]
fn a_later_measurement_is_a_new_row_rather_than_an_update() {
    // An outcome is an observation, not a fact. Overwriting would destroy the
    // ability to ask what a token looked like at the moment a decision was made,
    // which is the only question a backtest is allowed to ask.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut w = Writer::open(dir.path(), 1_000).expect("open");
    w.append_outcome(outcome(1, 100_000, 90_000, Some(95_000), 10))
        .expect("append");
    w.append_outcome(outcome(1, 900_000, 90_000, Some(880_000), 4_000))
        .expect("append");
    w.flush().expect("flush");

    let r = Reader::open(dir.path());
    assert_eq!(
        r.read_outcomes(AsOf::at(Slot(999_999)))
            .expect("read")
            .len(),
        2
    );

    // As of the earlier slot, only the earlier measurement exists -- the token
    // had ten transfers, and the four thousand it eventually saw are the future.
    let early = r.read_outcomes(AsOf::at(Slot(100_000))).expect("read");
    assert_eq!(early.len(), 1);
    assert_eq!(early[0].transfers, 10);
}

#[test]
fn outcomes_count_toward_the_store_watermark() {
    // They are stamped with `measured_at` rather than `slot`, so a watermark
    // that only looked at the event tables would under-report what the store
    // can answer.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut w = Writer::open(dir.path(), 1_000).expect("open");
    w.append_outcome(outcome(1, 777_777, 700_000, Some(770_000), 42))
        .expect("append");
    w.flush().expect("flush");
    assert_eq!(
        Reader::watermark(&Reader::open(dir.path())).expect("watermark"),
        Some(Slot(777_777))
    );
}

#[test]
fn reading_outcomes_as_events_fails_with_the_reason() {
    // Iterating Table::ALL and calling read() on each compiles and breaks at
    // runtime. It broke the CLI once, and the error was "stored file has no
    // `slot` column" -- true, and useless for working out what to do instead.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut w = Writer::open(dir.path(), 1).expect("open");
    w.append_outcome(outcome(1, 500_000, 400_000, Some(450_000), 12))
        .expect("append");
    w.flush().expect("flush");

    let err = Reader::open(dir.path())
        .read(Table::Outcomes, AsOf::at(Slot(999_999)))
        .expect_err("must refuse");
    let message = err.to_string();
    assert!(
        message.contains("read_outcomes"),
        "the error must name the way out: {message}"
    );
    assert!(
        !message.contains("slot` column"),
        "and must not blame a column: {message}"
    );
}

#[test]
fn every_event_table_can_actually_be_read_as_events() {
    // The other half: EVENT_TABLES must not drift into listing something that
    // read() then refuses.
    let dir = tempfile::tempdir().expect("tempdir");
    Writer::open(dir.path(), 1).expect("open");
    let r = Reader::open(dir.path());
    for table in Table::EVENT_TABLES {
        assert!(
            r.read(*table, AsOf::at(Slot(1))).is_ok(),
            "{table:?} should be readable"
        );
    }
}

/// A decision with every optional field populated, so a round trip that drops
/// one is visible.
fn decision(mint: u8, decided: u64) -> radar_store::Decision {
    radar_store::Decision {
        mint: Address::new([mint; 32]),
        creator: Address::new([200u8; 32]),
        decided_at: Slot(decided),
        launch_slot: Slot(decided.saturating_sub(6_000)),
        strategy: "creator_edge".to_owned(),
        strategy_version: "0.1.0".to_owned(),
        conclusion: radar_store::Conclusion::Proposed,
        reasons: Vec::new(),
        notional_micro_usd: Some(6_300_000),
        exit_capacity_micro_usd: Some(31_520_000),
        assumed_round_trip_bps: 850,
        coordination: Some("unremarkable".to_owned()),
        kernel_outcome: Some(radar_store::KernelOutcome::Refused),
        kernel_reasons: vec!["NoAutonomy".to_owned(), "InputsTooStale".to_owned()],
        entry_price: Some(27_583_000_000),
        inputs_digest: "9f2c4a1b".to_owned(),
    }
}

#[test]
fn a_decision_survives_a_round_trip_through_parquet() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut writer = Writer::open(dir.path(), 64).expect("open");

    let refused = {
        let mut d = decision(1, 500_000);
        d.conclusion = radar_store::Conclusion::Passed;
        d.reasons = vec![
            "ExitCanBeStopped".to_owned(),
            "CreatorNeverGraduated".to_owned(),
        ];
        d.notional_micro_usd = None;
        d.exit_capacity_micro_usd = None;
        d.kernel_outcome = None;
        d.kernel_reasons = Vec::new();
        d.coordination = None;
        d
    };
    let proposed = decision(2, 500_001);

    writer.append_decision(refused.clone()).expect("append");
    writer.append_decision(proposed.clone()).expect("append");
    writer.flush().expect("flush");

    let reader = Reader::open(dir.path());
    let back = reader
        .read_decisions(AsOf::at(Slot(1_000_000)))
        .expect("read");

    assert_eq!(back.len(), 2);
    assert_eq!(back[0], refused, "a refusal must survive exactly");
    assert_eq!(back[1], proposed, "and so must a proposal");
}

#[test]
fn a_decision_taken_after_the_watermark_is_not_returned() {
    // The point-in-time guarantee applies to decisions too. A backtest that
    // could see what Radar decided tomorrow is not a backtest.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut writer = Writer::open(dir.path(), 64).expect("open");
    writer
        .append_decision(decision(3, 400_000))
        .expect("append");
    writer
        .append_decision(decision(4, 900_000))
        .expect("append");
    writer.flush().expect("flush");

    let reader = Reader::open(dir.path());
    let visible = reader
        .read_decisions(AsOf::at(Slot(500_000)))
        .expect("read");
    assert_eq!(visible.len(), 1, "only the earlier decision is admitted");
    assert_eq!(visible[0].decided_at, Slot(400_000));

    // And the boundary is inclusive, matching every other read in the store.
    let at_boundary = reader
        .read_decisions(AsOf::at(Slot(400_000)))
        .expect("read");
    assert_eq!(at_boundary.len(), 1);
}

#[test]
fn empty_reason_lists_and_populated_ones_both_survive() {
    // A list column is the one place a round trip can silently flatten two
    // different rows into the same shape: zero reasons and one empty reason
    // must not become each other.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut writer = Writer::open(dir.path(), 64).expect("open");

    let mut none = decision(5, 500_000);
    none.reasons = Vec::new();
    none.kernel_reasons = Vec::new();
    let mut many = decision(6, 500_001);
    many.reasons = vec!["A".to_owned(), "B".to_owned(), "C".to_owned()];
    many.kernel_reasons = vec!["D".to_owned()];

    writer.append_decision(none.clone()).expect("append");
    writer.append_decision(many.clone()).expect("append");
    writer.flush().expect("flush");

    let back = Reader::open(dir.path())
        .read_decisions(AsOf::at(Slot(1_000_000)))
        .expect("read");
    assert!(back[0].reasons.is_empty());
    assert!(back[0].kernel_reasons.is_empty());
    assert_eq!(back[1].reasons, vec!["A", "B", "C"]);
    assert_eq!(back[1].kernel_reasons, vec!["D"]);
}

#[test]
fn decisions_are_written_where_reading_events_would_refuse_them() {
    // LEARNINGS 6: adding a member to `Table::ALL` broke every caller that
    // iterated it and called `read`. Decisions are the second table that has to
    // stay out of that loop, and the guard is that `read` refuses by name
    // rather than failing on a missing column.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut writer = Writer::open(dir.path(), 64).expect("open");
    writer
        .append_decision(decision(7, 500_000))
        .expect("append");
    writer.flush().expect("flush");

    let reader = Reader::open(dir.path());
    let err = reader
        .read(Table::Decisions, AsOf::at(Slot(1_000_000)))
        .expect_err("reading decisions as events must refuse");
    assert!(
        err.to_string().contains("decisions"),
        "the refusal must name the table: {err}"
    );
    assert!(
        reader.read_decisions(AsOf::at(Slot(1_000_000))).is_ok(),
        "and the right reader must work"
    );
}

#[test]
fn an_authorised_verdict_reads_back_as_authorised() {
    // The reader maps "authorised" to Authorised and everything else to
    // Refused, which is the safe direction -- but a round trip that only ever
    // stored Refused would pass with the mapping deleted, and every recorded
    // authorisation would silently become a refusal.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut writer = Writer::open(dir.path(), 64).expect("open");

    let mut authorised = decision(8, 500_000);
    authorised.kernel_outcome = Some(radar_store::KernelOutcome::Authorised);
    authorised.kernel_reasons = Vec::new();
    let refused = decision(9, 500_001);

    writer.append_decision(authorised.clone()).expect("append");
    writer.append_decision(refused.clone()).expect("append");
    writer.flush().expect("flush");

    let back = Reader::open(dir.path())
        .read_decisions(AsOf::at(Slot(1_000_000)))
        .expect("read");
    assert_eq!(
        back[0].kernel_outcome,
        Some(radar_store::KernelOutcome::Authorised)
    );
    assert_eq!(
        back[1].kernel_outcome,
        Some(radar_store::KernelOutcome::Refused)
    );
}

#[test]
fn decisions_in_a_later_partition_are_skipped_by_filename_not_by_row() {
    // The partition skip is an optimisation that has to be exactly right: too
    // eager and it drops decisions that are within the watermark, and the row
    // filter downstream would hide the bug by producing a correct-looking
    // shorter answer. Spanning two partitions is what makes the comparison
    // load-bearing rather than trivially true.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut writer = Writer::open(dir.path(), 64).expect("open");
    let early = 10;
    let late = SLOTS_PER_PARTITION * 3 + 10;
    writer.append_decision(decision(10, early)).expect("append");
    writer.append_decision(decision(11, late)).expect("append");
    writer.flush().expect("flush");

    let reader = Reader::open(dir.path());
    assert_eq!(
        reader.files(Table::Decisions).expect("files").len(),
        2,
        "the two decisions must land in different partition files"
    );

    // A watermark inside the first partition sees only the first.
    let early_only = reader
        .read_decisions(AsOf::at(Slot(early + 1)))
        .expect("read");
    assert_eq!(early_only.len(), 1);
    assert_eq!(early_only[0].decided_at, Slot(early));

    // A watermark exactly at the later decision still sees both: the skip
    // compares the partition's *start* slot, so `>=` there would discard a
    // partition whose first row is admissible.
    let both = reader.read_decisions(AsOf::at(Slot(late))).expect("read");
    assert_eq!(both.len(), 2, "an inclusive watermark admits the later row");
}

#[test]
fn a_full_buffer_flushes_without_being_asked() {
    // `append_decision` flushes when the buffer fills. If that counter stops
    // advancing, nothing reaches disk until an explicit flush -- and a run that
    // crashed mid-pass would lose every decision it had made, which is exactly
    // what recording exists to prevent.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut writer = Writer::open(dir.path(), 2).expect("open");
    writer
        .append_decision(decision(12, 500_000))
        .expect("append");
    writer
        .append_decision(decision(13, 500_001))
        .expect("append");

    // No explicit flush.
    let back = Reader::open(dir.path())
        .read_decisions(AsOf::at(Slot(1_000_000)))
        .expect("read");
    assert_eq!(back.len(), 2, "the buffer must have flushed on its own");
    assert_eq!(writer.buffered(), 0, "and the counter must have reset");
}

#[test]
fn the_earliest_slot_sees_decisions_too() {
    // `earliest` walks every table's own slot column. A decision recorded
    // before any event would otherwise be invisible to it, and the whole point
    // of `slot_column` is that each table names that column for itself.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut writer = Writer::open(dir.path(), 64).expect("open");
    writer.append_decision(decision(14, 7_000)).expect("append");
    writer.flush().expect("flush");

    let reader = Reader::open(dir.path());
    assert_eq!(
        reader.earliest().expect("earliest"),
        Some(Slot(7_000)),
        "earliest must read the decisions table's own slot column"
    );
    assert_eq!(
        Reader::watermark(&reader).expect("watermark"),
        Some(Slot(7_000))
    );
}

#[test]
fn a_partition_starting_exactly_at_the_watermark_is_not_skipped() {
    // The skip compares the partition's START slot against the watermark, and
    // the only case that separates `>` from `>=` or `==` is equality. A
    // partition whose first row sits exactly on the watermark holds an
    // admissible decision, and skipping it drops a row the caller is entitled
    // to see -- silently, because the shorter answer still looks like an answer.
    let boundary = SLOTS_PER_PARTITION * 3;
    let dir = tempfile::tempdir().expect("tempdir");
    let mut writer = Writer::open(dir.path(), 64).expect("open");
    writer
        .append_decision(decision(15, boundary))
        .expect("append");
    writer.flush().expect("flush");

    let back = Reader::open(dir.path())
        .read_decisions(AsOf::at(Slot(boundary)))
        .expect("read");
    assert_eq!(
        back.len(),
        1,
        "a decision exactly on the watermark, in a partition starting there, must be returned"
    );
    assert_eq!(back[0].decided_at, Slot(boundary));
}

#[test]
fn decisions_buffer_until_the_threshold_rather_than_writing_a_file_each() {
    // Buffering is why the store does not accumulate one Parquet file per row.
    // Flushing eagerly still gets the data to disk, so a test that only checks
    // the data arrives cannot tell the two apart -- and on a two-core box
    // sharing disk with two other services, a file per decision is the
    // difference that matters.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut writer = Writer::open(dir.path(), 3).expect("open");
    writer
        .append_decision(decision(16, 500_000))
        .expect("append");
    writer
        .append_decision(decision(17, 500_001))
        .expect("append");

    assert_eq!(
        writer.buffered(),
        2,
        "below the threshold, nothing is written"
    );
    assert_eq!(writer.written_files(), 0);
    assert!(
        Reader::open(dir.path())
            .read_decisions(AsOf::at(Slot(1_000_000)))
            .expect("read")
            .is_empty(),
        "nothing should have reached disk yet"
    );

    writer
        .append_decision(decision(18, 500_002))
        .expect("append");
    assert_eq!(
        writer.buffered(),
        0,
        "the third append crosses the threshold"
    );
    assert_eq!(writer.written_files(), 1, "and writes exactly one file");
}

#[test]
fn flushing_decisions_counts_the_rows_and_files_it_wrote() {
    // The backfill prints these, and a counter that stays at zero reports a
    // successful pass as having written nothing -- which is indistinguishable
    // from a pass that genuinely wrote nothing.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut writer = Writer::open(dir.path(), 64).expect("open");
    for (i, slot) in [500_000u64, 500_001, 500_002].iter().enumerate() {
        writer
            .append_decision(decision(20 + u8::try_from(i).expect("small"), *slot))
            .expect("append");
    }
    assert_eq!(writer.written_rows(), 0, "nothing written before the flush");
    writer.flush().expect("flush");

    assert_eq!(writer.written_rows(), 3, "three decisions, three rows");
    assert_eq!(writer.written_files(), 1, "one partition, one file");
}
