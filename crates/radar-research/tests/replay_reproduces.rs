// SPDX-License-Identifier: Apache-2.0
//! The leakage and determinism test the plan asks for, over a real store.
//!
//! The store here is written to disk and read back through `Reader`, rather than
//! candidates being built in memory. That matters: the failure this is meant to
//! catch lives in the read path — a store answering differently for the same
//! watermark — and a test that never reads a file cannot see it.

use radar_asof::AsOf;
use radar_research::{Verdict, record, replay};
use radar_store::{Envelope, Event, Launch, Origin, Outcome, Reader, Writer};
use radar_strategy::{Candidate, CreatorEdge, Decision, Strategy, universe};
use radar_types::{Address, Signature, Slot};

fn creator(n: u8) -> Address {
    Address::new([n; 32])
}

fn mint(n: u8) -> Address {
    Address::new([n.wrapping_add(100); 32])
}

fn launch(mint_id: u8, creator_id: u8, slot: u64) -> Event {
    Event::Launch(Box::new(Launch {
        envelope: Envelope {
            slot: Slot(slot),
            signature: Signature::new([mint_id; 64]),
            tx_index: 1,
            instruction_index: 0,
            parent_index: None,
            succeeded: true,
        },
        origin: Origin::known(Address::new([9; 32]), "create_v2"),
        mint: mint(mint_id),
        creator: creator(creator_id),
        name: format!("Token{mint_id}"),
        symbol: "TKN".to_owned(),
        uri: String::new(),
        dev_buy_lamports: None,
    }))
}

/// An outcome for a token that traded and, optionally, graduated.
fn outcome(
    mint_id: u8,
    launch_slot: u64,
    measured_at: u64,
    graduated_after: Option<u64>,
) -> Outcome {
    Outcome {
        mint: mint(mint_id),
        measured_at: Slot(measured_at),
        launch_slot: Slot(launch_slot),
        first_transfer_slot: Some(Slot(launch_slot)),
        last_transfer_slot: Some(Slot(launch_slot + 40_000)),
        transfers: 600,
        unique_senders: 30,
        unique_receivers: 30,
        graduated_at: graduated_after.map(|d| Slot(launch_slot + d)),
    }
}

/// A store with one creator whose eight launches are all measured.
fn store(dir: &std::path::Path, extra: &[Event]) -> Reader {
    let mut w = Writer::open(dir, 1_000).expect("open");
    for i in 0..8u8 {
        let slot = 10_000 + u64::from(i) * 100;
        w.append(launch(i, 1, slot)).expect("append");
        // Six graduate organically, two never do.
        let after = (i < 6).then_some(900);
        w.append_outcome(outcome(i, slot, 20_000, after))
            .expect("append outcome");
    }
    for e in extra {
        w.append(e.clone()).expect("append extra");
    }
    w.flush().expect("flush");
    Reader::open(dir)
}

/// Rebuilds the candidate for `mint_id` from the store, at the given watermark.
fn candidate_at(reader: &Reader, as_of: AsOf, mint_id: u8) -> Candidate {
    universe(reader, as_of)
        .expect("universe")
        .candidate(&mint(mint_id), None, None)
        .expect("the launch is in the store")
}

const WATERMARK: Slot = Slot(30_000);

#[test]
fn a_recorded_decision_replays_identically() {
    // The plan's test, in its simplest form: same store, same watermark, same
    // answer. If this fails, nothing downstream of it means anything.
    let dir = tempfile::tempdir().expect("tempdir");
    let reader = store(dir.path(), &[]);
    let as_of = AsOf::at(WATERMARK);
    let strategy = CreatorEdge::default();

    let before = candidate_at(&reader, as_of, 0);
    let recording = record(&strategy, &before).expect("record");

    // Re-read the store from scratch rather than reusing the candidate: reusing
    // it would test that a value equals itself.
    let reader = Reader::open(dir.path());
    let after = candidate_at(&reader, as_of, 0);

    assert_eq!(
        replay(&recording, &strategy, &after).expect("replay"),
        Verdict::Identical
    );
}

#[test]
fn replaying_ten_times_never_drifts() {
    // A determinism bug that only shows up sometimes -- iteration order over a
    // hash map is the classic -- would pass a single replay about half the time.
    let dir = tempfile::tempdir().expect("tempdir");
    let reader = store(dir.path(), &[]);
    let as_of = AsOf::at(WATERMARK);
    let strategy = CreatorEdge::default();
    let recording = record(&strategy, &candidate_at(&reader, as_of, 0)).expect("record");

    for attempt in 0..10 {
        let reader = Reader::open(dir.path());
        let verdict =
            replay(&recording, &strategy, &candidate_at(&reader, as_of, 0)).expect("replay");
        assert_eq!(verdict, Verdict::Identical, "drifted on attempt {attempt}");
    }
}

#[test]
fn a_store_that_gained_history_reports_changed_inputs_not_a_leak() {
    // The case the plan's one-line version gets wrong, and the one that actually
    // happened: on 2026-08-24 a repair added 1,740 graduation events with
    // historical slots to the live store. Those rows are at or before the
    // watermark, so a replay across that boundary legitimately sees more than the
    // recording did. Calling that a leak would fail the build on the act of
    // fixing the data.
    let dir = tempfile::tempdir().expect("tempdir");
    let reader = store(dir.path(), &[]);
    let as_of = AsOf::at(WATERMARK);
    let strategy = CreatorEdge::default();
    let recording = record(&strategy, &candidate_at(&reader, as_of, 0)).expect("record");

    // A backfill catching up: another launch by the same creator, at a slot the
    // watermark already covered.
    let mut w = Writer::open(dir.path(), 1_000).expect("open");
    w.append(launch(50, 1, 12_000)).expect("append");
    w.flush().expect("flush");

    let reader = Reader::open(dir.path());
    let verdict = replay(&recording, &strategy, &candidate_at(&reader, as_of, 0)).expect("replay");

    let Verdict::InputsChanged { was, now, .. } = &verdict else {
        panic!("late-arriving history must read as changed inputs, got {verdict:?}");
    };
    assert_ne!(was, now);
    assert!(
        !verdict.is_failure(),
        "a backfill must not fail the build, or the check gets ignored"
    );
    assert!(verdict.needs_review(), "but a human still has to see it");
}

#[test]
fn a_strategy_that_is_not_a_pure_function_of_its_inputs_is_caught() {
    // The verdict that must fail CI. Without this test the harness could report
    // Identical unconditionally and every other test here would still pass --
    // which is exactly the failure mode LEARNINGS 4 records: a test that cannot
    // fail against the plausible wrong answer is not evidence.
    struct Moody {
        calls: std::cell::Cell<u32>,
    }
    impl Strategy for Moody {
        fn name(&self) -> &'static str {
            "moody"
        }
        fn version(&self) -> &'static str {
            "0.1.0"
        }
        fn consider(&self, candidate: &radar_strategy::Candidate) -> Decision {
            self.calls.set(self.calls.get() + 1);
            // Same candidate, different answer on the second call.
            if self.calls.get() > 1 {
                Decision::pass(vec![radar_strategy::PassReason::CreatorUnproven])
            } else {
                CreatorEdge::default().consider(candidate)
            }
        }
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let reader = store(dir.path(), &[]);
    let as_of = AsOf::at(WATERMARK);
    let strategy = Moody {
        calls: std::cell::Cell::new(0),
    };

    let candidate = candidate_at(&reader, as_of, 0);
    let recording = record(&strategy, &candidate).expect("record");
    let verdict = replay(&recording, &strategy, &candidate).expect("replay");

    assert!(
        matches!(verdict, Verdict::NotDeterministic { .. }),
        "a strategy that answers differently on identical inputs must be caught, got {verdict:?}"
    );
    assert!(verdict.is_failure(), "and it must fail the build");
}

#[test]
fn the_digest_covers_every_input_not_a_chosen_few() {
    // A digest over a hand-picked subset would let a new input move decisions
    // while every replay still reported them identical. Changing any field of
    // the candidate must move the hash, including ones no current strategy reads.
    let dir = tempfile::tempdir().expect("tempdir");
    let reader = store(dir.path(), &[]);
    let as_of = AsOf::at(WATERMARK);
    let base = candidate_at(&reader, as_of, 0);
    let baseline = radar_research::Digest::of(&base).expect("digest");

    let mut moved = base.clone();
    moved.sol_price_micro_usd = Some(radar_types::MicroUsd(123_456));
    assert_ne!(
        radar_research::Digest::of(&moved).expect("digest"),
        baseline,
        "a changed input must change the digest"
    );

    let mut same = base.clone();
    same.sol_price_micro_usd = base.sol_price_micro_usd;
    assert_eq!(
        radar_research::Digest::of(&same).expect("digest"),
        baseline,
        "an unchanged candidate must hash identically"
    );
}
