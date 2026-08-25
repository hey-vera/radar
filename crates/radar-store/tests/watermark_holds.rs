// SPDX-License-Identifier: Apache-2.0
//! The watermark guarantee, tested adversarially rather than asserted.
//!
//! `AGENTS.md` rule 3 says nothing reads past its watermark. That is the
//! property every research result rests on, and until these tests existed it was
//! held up by reading the code and agreeing with it.
//!
//! The interesting case is **a file that straddles the watermark**. `Reader` skips
//! whole files whose slot range starts after `as_of`, which is a real and worthwhile
//! optimisation — and it means a file starting *before* the watermark is opened and
//! read in full, so every row in it past the watermark has to be caught one at a
//! time. A test that only puts future events in future files would pass against a
//! reader with no per-row filter at all.

use radar_asof::AsOf;
use radar_store::{
    Envelope, Event, Launch, Origin, Outcome, Reader, SLOTS_PER_PARTITION, Table, Writer,
};
use radar_types::{Address, Signature, Slot};

fn launch_at(slot: u64) -> Event {
    Event::Launch(Box::new(Launch {
        envelope: Envelope {
            slot: Slot(slot),
            signature: Signature::new([u8::try_from(slot % 251).unwrap_or(1); 64]),
            tx_index: 1,
            instruction_index: 0,
            parent_index: None,
            succeeded: true,
        },
        origin: Origin::known(Address::new([9; 32]), "create_v2"),
        mint: Address::new([u8::try_from(slot % 251).unwrap_or(1); 32]),
        creator: Address::new([7; 32]),
        name: format!("T{slot}"),
        symbol: "TKN".to_owned(),
        uri: String::new(),
        dev_buy_lamports: None,
    }))
}

fn outcome_at(measured_at: u64) -> Outcome {
    Outcome {
        mint: Address::new([u8::try_from(measured_at % 251).unwrap_or(1); 32]),
        measured_at: Slot(measured_at),
        launch_slot: Slot(1_000),
        first_transfer_slot: Some(Slot(1_001)),
        last_transfer_slot: Some(Slot(1_500)),
        transfers: 20,
        unique_senders: 5,
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

/// Slots that all fall inside one partition, so they land in one file.
///
/// That is the arrangement the per-row filter has to survive: the file starts
/// before the watermark, so it is opened, and half its contents are from after it.
fn slots_in_one_partition() -> [u64; 4] {
    let base = SLOTS_PER_PARTITION;
    [base, base + 2_000, base + 7_000, base + 12_000]
}

#[test]
fn a_file_that_straddles_the_watermark_yields_only_the_admissible_half() {
    let dir = tempfile::tempdir().expect("tempdir");
    let slots = slots_in_one_partition();

    let mut w = Writer::open(dir.path(), 10_000).expect("open");
    for slot in slots {
        w.append(launch_at(slot)).expect("append");
    }
    w.flush().expect("flush");

    // One file, so the whole-file skip cannot be what does the filtering.
    let files: Vec<_> = std::fs::read_dir(dir.path().join(Table::Launches.dir()))
        .expect("dir")
        .filter_map(Result::ok)
        .collect();
    assert_eq!(
        files.len(),
        1,
        "the fixture must be a single straddling file"
    );

    // Between the second and third slot.
    let watermark = Slot(slots[1] + 1);
    let read = Reader::open(dir.path())
        .read(Table::Launches, AsOf::at(watermark))
        .expect("read");

    assert_eq!(read.len(), 2, "expected only the two admissible events");
    for event in &read {
        assert!(
            event.envelope().slot <= watermark,
            "leaked slot {} past watermark {watermark}",
            event.envelope().slot
        );
    }
}

#[test]
fn no_read_at_any_watermark_ever_returns_a_later_event() {
    // Swept rather than spot-checked: an off-by-one at a partition or file
    // boundary would pass a single well-chosen watermark.
    let dir = tempfile::tempdir().expect("tempdir");
    let slots = slots_in_one_partition();
    let mut w = Writer::open(dir.path(), 10_000).expect("open");
    for slot in slots {
        w.append(launch_at(slot)).expect("append");
    }
    w.flush().expect("flush");
    let reader = Reader::open(dir.path());

    for probe in slots {
        for watermark in [probe - 1, probe, probe + 1] {
            let as_of = AsOf::at(Slot(watermark));
            let read = reader.read(Table::Launches, as_of).expect("read");
            for event in &read {
                assert!(
                    event.envelope().slot.get() <= watermark,
                    "at watermark {watermark}, leaked slot {}",
                    event.envelope().slot
                );
            }
            let expected = slots.iter().filter(|s| **s <= watermark).count();
            assert_eq!(read.len(), expected, "at watermark {watermark}");
        }
    }
}

#[test]
fn outcomes_are_gated_on_when_they_were_measured() {
    // An outcome describes a token's past but is *known* only from the slot it
    // was measured at. Gating it on the launch slot instead would hand a decision
    // a measurement taken after it — the exact leak this crate exists to stop.
    //
    // Measured inside one partition on purpose. Spread across partitions, the
    // whole-file skip does all the work and the per-row filter is never reached —
    // which is exactly what an earlier version of this test did, and it passed
    // with the per-row filter deleted.
    let dir = tempfile::tempdir().expect("tempdir");
    let measured = slots_in_one_partition();
    let watermark = Slot(measured[1] + 1);

    let mut w = Writer::open(dir.path(), 10_000).expect("open");
    for measured_at in measured {
        w.append_outcome(outcome_at(measured_at)).expect("append");
    }
    w.flush().expect("flush");

    let read = Reader::open(dir.path())
        .read_outcomes(AsOf::at(watermark))
        .expect("read");

    assert_eq!(
        read.len(),
        2,
        "expected only the two admissible measurements"
    );
    for outcome in &read {
        assert!(
            outcome.measured_at <= watermark,
            "leaked a measurement from slot {}",
            outcome.measured_at
        );
        // Every one of these has launch_slot 1_000, well before the watermark, so
        // a reader gating on the launch slot would have returned all three.
        assert!(outcome.launch_slot < outcome.measured_at);
    }
}

#[test]
fn a_watermark_before_everything_returns_nothing_rather_than_everything() {
    // The failure direction that matters: an inverted comparison returns the
    // whole store, and every count downstream still looks plausible.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut w = Writer::open(dir.path(), 10_000).expect("open");
    for slot in slots_in_one_partition() {
        w.append(launch_at(slot)).expect("append");
    }
    w.append_outcome(outcome_at(500_000)).expect("append");
    w.flush().expect("flush");

    let reader = Reader::open(dir.path());
    let ancient = AsOf::at(Slot(1));
    assert!(
        reader
            .read(Table::Launches, ancient)
            .expect("read")
            .is_empty()
    );
    assert!(reader.read_outcomes(ancient).expect("read").is_empty());
}
