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
