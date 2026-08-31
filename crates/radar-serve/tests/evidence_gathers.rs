// SPDX-License-Identifier: Apache-2.0
//! Evidence gathering, against real files on disk.
//!
//! `plan` is pure and unit-tested in the module. This is the half that needs a
//! store: that the plan is executed against the registry it was given, that the
//! citation names the invocation, and that an empty store yields no evidence
//! rather than an error.

use radar_instruments::{CreatorHistory, CreatorTrackRecord, Registry, SimulateExit};
use radar_serve::evidence;
use radar_store::{Reader, Writer};
use radar_types::{Address, Slot};

/// The creator every fixture launch belongs to, base58, as an operator would
/// paste it into the question box.
fn creator() -> String {
    Address::new([200u8; 32]).to_string()
}

fn registry() -> Registry {
    let mut r = Registry::new();
    r.register(CreatorHistory);
    r.register(CreatorTrackRecord);
    r.register(SimulateExit::default());
    r
}

/// A store with something in it, so the watermark is not `None`.
fn populated() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut writer = Writer::open(dir.path(), 64).expect("open");
    writer
        .append_outcome(radar_store::Outcome {
            mint: Address::new([1u8; 32]),
            measured_at: Slot(10_000),
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
            window_peak_price: None,
            window_trough_price: None,
            vwap: Some(1_050),
            fills: 9,
        })
        .expect("append");
    writer.flush().expect("flush");
    dir
}

#[test]
fn a_question_naming_a_creator_is_answered_by_the_creator_instruments() {
    // The plan says which instruments to run; this checks the plan is executed
    // against the registry it was handed. A name match inverted here would
    // silently run the *wrong* instrument and cite the right one.
    let dir = populated();
    let store = Reader::open(dir.path());
    let question = format!("what has {} launched?", creator());

    let blocks = evidence::gather(&registry(), &store, &question);
    assert!(
        !blocks.is_empty(),
        "an address in the question is looked up"
    );

    let sources: Vec<&str> = blocks.iter().map(|b| b.source.as_str()).collect();
    for source in &sources {
        assert!(
            source.starts_with("creator_history(") || source.starts_with("creator_track_record("),
            "unexpected source {source}"
        );
        assert!(
            source.contains(&creator()),
            "the citation names the argument, so it can be re-run: {source}"
        );
        assert!(
            !source.starts_with("simulate_exit"),
            "it needs a size nobody supplied: {source}"
        );
    }
}

#[test]
fn the_citation_names_the_instrument_that_actually_answered() {
    // The sharp case. `gather` looks an instrument up by name; if that match
    // were inverted it would run whichever instrument is *not* the one planned
    // and label the result with the planned name — a citation that is wrong in
    // the one way a reader cannot detect, because it looks right.
    let dir = populated();
    let store = Reader::open(dir.path());
    let blocks = evidence::gather(&registry(), &store, &format!("tell me about {}", creator()));

    // Both creator instruments accept the same argument and both echo it, so
    // "the content mentions the creator" does NOT distinguish them -- the first
    // version of this test asserted exactly that and an inverted match sailed
    // through it. Each is identified by a field only it returns.
    let fingerprint = |name: &str| match name {
        "creator_history" => "duplicate_metadata_launches",
        "creator_track_record" => "stillborn",
        other => panic!("unplanned instrument {other}"),
    };

    assert_eq!(blocks.len(), 2, "both creator instruments ran");
    for block in &blocks {
        let named = block
            .source
            .split('(')
            .next()
            .expect("the source names an instrument");
        assert!(
            block.content.contains(fingerprint(named)),
            "cited as {named}, but the output is not {named}'s: {}",
            block.content
        );
        assert!(
            block.content.contains(&creator()),
            "{named} answered about a different address than it was cited for"
        );
    }
}

#[test]
fn a_registry_without_the_planned_instruments_yields_nothing_rather_than_failing() {
    // A build that registered something else. Skipping is right — the plan is a
    // list of what would be useful, and an instrument this build does not have
    // is not an error in the question — but it must not panic or invent a
    // citation for a call that never happened.
    let dir = populated();
    let store = Reader::open(dir.path());
    let mut sparse = Registry::new();
    sparse.register(SimulateExit::default());

    let blocks = evidence::gather(&sparse, &store, &format!("about {}", creator()));
    assert!(blocks.is_empty(), "no call, so no citation: {blocks:?}");
}

#[test]
fn an_empty_store_yields_no_evidence_rather_than_an_error() {
    // A fresh instance. The reply is then marked uncited, which is honest: the
    // model answered from its own recollection because there was nothing to
    // show it.
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Reader::open(dir.path());
    assert!(evidence::gather(&registry(), &store, &format!("about {}", creator())).is_empty());
}

#[test]
fn a_question_naming_no_address_costs_no_instrument_calls() {
    // The common case, and the one where the cost matters: a question about the
    // funnel should not invoke anything at all.
    let dir = populated();
    let store = Reader::open(dir.path());
    assert!(evidence::gather(&registry(), &store, "why do we refuse so much?").is_empty());
}
