// SPDX-License-Identifier: Apache-2.0
//! The JSON surface, exercised against real files on disk.
//!
//! The shaping functions are unit-tested over hand-built rows in the module
//! itself. These check the half that needs a store: that a read for one mint
//! returns that mint and nothing else.

use radar_asof::AsOf;
use radar_serve::api;
use radar_store::{Conclusion, Decision, Outcome, Reader, Writer};
use radar_types::{Address, Slot};

fn decision(mint: u8, decided_at: u64) -> Decision {
    Decision {
        mint: Address::new([mint; 32]),
        creator: Address::new([200u8; 32]),
        decided_at: Slot(decided_at),
        launch_slot: Slot(decided_at.saturating_sub(6_000)),
        strategy: "creator_edge".to_owned(),
        strategy_version: "0.1.0".to_owned(),
        conclusion: Conclusion::Proposed,
        reasons: Vec::new(),
        notional_micro_usd: Some(6_300_000),
        exit_capacity_micro_usd: Some(31_520_000),
        assumed_round_trip_bps: 850,
        coordination: Some("unremarkable".to_owned()),
        kernel_outcome: Some(radar_store::KernelOutcome::Refused),
        kernel_reasons: vec!["NoAutonomy".to_owned()],
        entry_price: Some(27_583_000_000),
        inputs_digest: "abc".to_owned(),
    }
}

fn outcome(mint: u8, measured_at: u64) -> Outcome {
    Outcome {
        mint: Address::new([mint; 32]),
        measured_at: Slot(measured_at),
        launch_slot: Slot(4_000),
        first_transfer_slot: None,
        last_transfer_slot: None,
        transfers: 12,
        unique_senders: 3,
        unique_receivers: 4,
        graduated_at: None,
        first_price: Some(1_000),
        last_price: Some(870),
        peak_price: Some(1_400),
        trough_price: Some(800),
        vwap: Some(1_050),
        fills: 9,
    }
}

/// A store holding two mints, so a filter that returns everything is visible.
fn store_with_two_mints() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut writer = Writer::open(dir.path(), 64).expect("open");
    for mint in [1u8, 2u8] {
        writer
            .append_decision(decision(mint, 10_000))
            .expect("append decision");
        writer
            .append_outcome(outcome(mint, 11_000))
            .expect("append outcome");
    }
    writer.flush().expect("flush");
    dir
}

#[test]
fn asking_for_one_token_returns_that_token_and_no_other() {
    // Inverting either filter returns every mint EXCEPT the one asked for, and
    // the page would still render — with somebody else's evidence under this
    // token's name. Two mutants did exactly that and survived a suite that only
    // ever asked a store holding one mint.
    let dir = store_with_two_mints();
    let reader = Reader::open(dir.path());
    let wanted = Address::new([1u8; 32]).to_string();

    let evidence = api::token_evidence(&reader, &wanted, AsOf::at(Slot(50_000))).expect("reads");

    assert_eq!(evidence.mint, wanted);
    assert_eq!(evidence.decisions.len(), 1, "one decision, for this mint");
    assert_eq!(
        evidence.measurements.len(),
        1,
        "one measurement, for this mint"
    );
    assert_eq!(
        evidence.decisions[0].mint.to_string(),
        wanted,
        "the decision must belong to the mint that was asked for"
    );

    // And the other mint's rows are genuinely present in the store, so the
    // assertions above are not passing because there was nothing to exclude.
    let other = Address::new([2u8; 32]).to_string();
    let others = api::token_evidence(&reader, &other, AsOf::at(Slot(50_000))).expect("reads");
    assert_eq!(others.decisions.len(), 1);
    assert_ne!(others.decisions[0].mint.to_string(), wanted);
}

#[test]
fn a_mint_the_store_has_never_seen_returns_empty_rather_than_failing() {
    // A page for an unknown token is a normal request -- somebody pasted an
    // address. Empty is the answer; an error would suggest the store is broken.
    let dir = store_with_two_mints();
    let unknown = Address::new([99u8; 32]).to_string();

    let evidence = api::token_evidence(&Reader::open(dir.path()), &unknown, AsOf::at(Slot(50_000)))
        .expect("an unknown mint is not an error");
    assert!(evidence.decisions.is_empty());
    assert!(evidence.measurements.is_empty());
    assert_eq!(evidence.mint, unknown, "the request is echoed back");
}

#[test]
fn evidence_taken_after_the_watermark_is_not_returned() {
    // The point-in-time guarantee reaches the API surface too. A page that could
    // show what Radar decided after the watermark it claims to answer as of is
    // not answering as of anything.
    let dir = store_with_two_mints();
    let reader = Reader::open(dir.path());
    let wanted = Address::new([1u8; 32]).to_string();

    let before = api::token_evidence(&reader, &wanted, AsOf::at(Slot(9_999))).expect("reads");
    assert!(
        before.decisions.is_empty(),
        "a decision at slot 10,000 is not visible as of 9,999"
    );
    assert!(before.measurements.is_empty());

    let after = api::token_evidence(&reader, &wanted, AsOf::at(Slot(11_000))).expect("reads");
    assert_eq!(after.decisions.len(), 1);
    assert_eq!(after.measurements.len(), 1);
}

#[test]
fn the_store_counts_match_what_was_written() {
    // The health screen reads these. A count that drifted from the store would
    // be the screen most likely to be trusted without checking.
    let dir = store_with_two_mints();
    let counts =
        api::store_counts(&Reader::open(dir.path()), AsOf::at(Slot(50_000))).expect("counts");

    assert_eq!(counts.decisions, 2);
    assert_eq!(counts.outcomes, 2);
    assert_eq!(counts.launches, 0, "none were written");
    assert_eq!(counts.graduations, 0);
}
