// SPDX-License-Identifier: Apache-2.0
//! The registry's promise: one declaration, four consistent surfaces.

use radar_asof::AsOf;
use radar_instruments::{
    Context, CreatorHistory, DEFAULT_MARGIN_PERCENT, Instrument, InstrumentError, Registry,
};
use radar_store::{Envelope, Event, Launch, Origin, Reader, Writer};
use radar_types::{Address, Signature, Slot};
use serde_json::json;

fn creator(n: u8) -> Address {
    Address::new([n; 32])
}

fn launch(slot: u64, creator_id: u8, name: &str, symbol: &str, ok: bool) -> Event {
    Event::Launch(Box::new(Launch {
        envelope: Envelope {
            slot: Slot(slot),
            signature: Signature::new([(slot % 251) as u8; 64]),
            tx_index: 1,
            instruction_index: 0,
            parent_index: None,
            succeeded: ok,
        },
        origin: Origin::known(Address::SYSTEM_PROGRAM, "create_v2"),
        mint: Address::new([(slot % 251) as u8; 32]),
        creator: creator(creator_id),
        name: name.to_owned(),
        symbol: symbol.to_owned(),
        uri: format!("https://example.invalid/{name}.json"),
        dev_buy_lamports: None,
    }))
}

/// A store holding a prolific creator (10 launches) and a quiet one (1).
fn populated() -> (tempfile::TempDir, Reader) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut w = Writer::open(dir.path(), 1_000).expect("open");
    for i in 0..10u64 {
        // The first two are the same token relaunched -- identical name, symbol
        // and URI, in the same slot. The rest are distinct.
        let (name, slot) = if i < 2 {
            ("Repeat".to_owned(), 1_000)
        } else {
            (format!("Token{i}"), 1_000 + i * 1_000)
        };
        w.append(launch(slot, 1, &name, &name, i != 9))
            .expect("append");
    }
    w.append(launch(2_500, 2, "Quiet", "QT", true))
        .expect("append");
    w.flush().expect("flush");
    let reader = Reader::open(dir.path());
    (dir, reader)
}

fn registry() -> Registry {
    let mut r = Registry::new();
    r.register(CreatorHistory);
    r
}

#[test]
fn an_instrument_answers_over_the_real_store() {
    let (_dir, store) = populated();
    let ctx = Context {
        as_of: AsOf::at(Slot(1_000_000)),
        store: &store,
    };

    let record = registry()
        .invoke(
            "creator_history",
            json!({ "creator": creator(1).to_string() }),
            &ctx,
        )
        .expect("instrument exists");

    let out = record.output.expect("succeeded");
    assert_eq!(out["launches"], 10);
    // "Repeat" twice plus eight distinct.
    assert_eq!(out["distinct_symbols"], 9);
    // Only the second "Repeat" repeats an earlier launch exactly.
    assert_eq!(out["duplicate_metadata_launches"], 1);
    assert_eq!(out["max_launches_in_one_slot"], 2);
    assert_eq!(out["failed_launches"], 1);
    assert!(
        out["evidence"].as_array().is_some_and(|e| !e.is_empty()),
        "evidence is required"
    );
}

#[test]
fn the_watermark_reaches_into_the_instrument() {
    // The point-in-time guarantee has to hold through the registry, or an
    // instrument becomes a way around it.
    let (_dir, store) = populated();
    let registry = registry();
    let args = json!({ "creator": creator(1).to_string() });

    let early = Context {
        as_of: AsOf::at(Slot(1_000)),
        store: &store,
    };
    let late = Context {
        as_of: AsOf::at(Slot(1_000_000)),
        store: &store,
    };

    let early_count = registry
        .invoke("creator_history", args.clone(), &early)
        .expect("ok")
        .output;
    let late_count = registry
        .invoke("creator_history", args, &late)
        .expect("ok")
        .output;

    assert_eq!(
        early_count.expect("output")["launches"],
        2,
        "only the slot-1000 pair is visible"
    );
    assert_eq!(late_count.expect("output")["launches"], 10);
}

#[test]
fn an_unknown_creator_answers_zero_rather_than_failing() {
    // "This address has launched nothing" is a real and useful answer, and it is
    // different from "I cannot tell you".
    let (_dir, store) = populated();
    let ctx = Context {
        as_of: AsOf::at(Slot(1_000_000)),
        store: &store,
    };
    let record = registry()
        .invoke(
            "creator_history",
            json!({ "creator": creator(99).to_string() }),
            &ctx,
        )
        .expect("ok");
    let out = record.output.expect("succeeded");
    assert_eq!(out["launches"], 0);
    assert_eq!(out["first_launch_slot"], serde_json::Value::Null);
    assert_eq!(out["launches_per_hour"], serde_json::Value::Null);
}

#[test]
fn every_invocation_is_recorded_whether_or_not_it_worked() {
    // The recording is the research dataset. A failure is data too: an
    // instrument that fails on a class of token is telling you about that class.
    let (_dir, store) = populated();
    let ctx = Context {
        as_of: AsOf::at(Slot(1_000)),
        store: &store,
    };

    let bad = registry()
        .invoke("creator_history", json!({ "wrong_field": 1 }), &ctx)
        .expect("the instrument exists even though the arguments are wrong");
    assert!(bad.output.is_none());
    assert!(
        bad.error.is_some(),
        "a failure must be recorded, not swallowed"
    );
    assert_eq!(bad.instrument, "creator_history");
    assert_eq!(
        bad.as_of, 1_000,
        "the watermark is recorded even on failure"
    );
    assert_eq!(
        bad.arguments,
        json!({ "wrong_field": 1 }),
        "arguments recorded verbatim"
    );
}

#[test]
fn an_unregistered_name_is_an_error_rather_than_an_empty_record() {
    let (_dir, store) = populated();
    let ctx = Context {
        as_of: AsOf::at(Slot(1)),
        store: &store,
    };
    assert!(matches!(
        registry().invoke("no_such_instrument", json!({}), &ctx),
        Err(InstrumentError::NotFound(_))
    ));
}

#[test]
fn the_mcp_catalogue_carries_the_schema_and_the_price_from_one_declaration() {
    // Three surfaces each holding their own copy of the price would drift, and
    // the one that drifted would be the paid one.
    let registry = registry();
    let tools = registry.mcp_tools();
    let tool = &tools.as_array().expect("array")[0];

    assert_eq!(tool["name"], "creator_history");
    assert!(tool["description"].as_str().is_some_and(|d| !d.is_empty()));
    assert!(
        tool["inputSchema"]["properties"]["creator"].is_object(),
        "input schema is derived"
    );
    assert!(
        tool["outputSchema"]["properties"]["launches"].is_object(),
        "output schema is derived"
    );

    let spec = Instrument::spec(&CreatorHistory);
    let advertised = tool["_meta"]["org.heyvera.radar/priceMicroUsd"]
        .as_u64()
        .expect("price");
    assert_eq!(advertised, spec.public_price(DEFAULT_MARGIN_PERCENT).get());
    assert_eq!(tool["_meta"]["org.heyvera.radar/version"], "1.0");
}

#[test]
fn registering_two_instruments_under_one_name_is_refused() {
    // Ambiguity here would make the HTTP route, the price and the MCP tool
    // depend on registration order.
    let result = std::panic::catch_unwind(|| {
        let mut r = Registry::new();
        r.register(CreatorHistory);
        r.register(CreatorHistory);
    });
    assert!(
        result.is_err(),
        "duplicate registration must not be silently accepted"
    );
}

#[test]
fn a_pure_instrument_replays_identically() {
    // The leakage test rests on this: same arguments, same watermark, same
    // answer. A divergence means a leak or a non-determinism bug.
    let (_dir, store) = populated();
    let ctx = Context {
        as_of: AsOf::at(Slot(5_000)),
        store: &store,
    };
    let registry = registry();
    let args = json!({ "creator": creator(1).to_string() });

    let first = registry
        .invoke("creator_history", args.clone(), &ctx)
        .expect("ok")
        .output;
    let second = registry
        .invoke("creator_history", args, &ctx)
        .expect("ok")
        .output;
    assert_eq!(first, second);
}

/// A creator whose tokens all reached an AMM, six of them inside their own
/// launch block and one over an hour.
///
/// The shape that motivated the split: on the undifferentiated count this
/// creator is flawless, and flawless is exactly what a bundler looks like.
fn store_with_one_bundler() -> (tempfile::TempDir, Reader) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut w = Writer::open(dir.path(), 1_000).expect("open");

    let mints: Vec<radar_types::Address> = (0..7u8).map(|i| Address::new([i + 40; 32])).collect();
    for (i, mint) in mints.iter().enumerate() {
        let n = u8::try_from(i).expect("seven tokens");
        let slot = 10_000 + u64::from(n) * 100;
        w.append(Event::Launch(Box::new(Launch {
            envelope: Envelope {
                slot: Slot(slot),
                signature: Signature::new([n + 40; 64]),
                tx_index: 1,
                instruction_index: 0,
                parent_index: None,
                succeeded: true,
            },
            origin: Origin::known(Address::new([9; 32]), "create_v2"),
            mint: *mint,
            creator: creator(7),
            name: format!("Bundle{i}"),
            symbol: "BND".to_owned(),
            uri: String::new(),
            dev_buy_lamports: None,
        })))
        .expect("append");

        // Six graduate in the same slot they launched; the seventh takes an hour.
        let after = if n == 6 { 9_000 } else { 0 };
        w.append_outcome(radar_store::Outcome {
            mint: *mint,
            measured_at: Slot(500_000),
            launch_slot: Slot(slot),
            first_transfer_slot: Some(Slot(slot)),
            last_transfer_slot: Some(Slot(slot + after + 500)),
            transfers: 900,
            unique_senders: 40,
            unique_receivers: 40,
            graduated_at: Some(Slot(slot + after)),
            first_price: None,
            last_price: None,
            peak_price: None,
            trough_price: None,
            window_peak_price: None,
            window_trough_price: None,
            vwap: None,
            fills: 0,
        })
        .expect("append outcome");
    }
    w.flush().expect("flush");
    let reader = Reader::open(dir.path());
    (dir, reader)
}

#[test]
fn the_instrument_separates_a_bundler_from_a_builder() {
    // A 100% graduation rate that is 86% instant is the exact record the old
    // undifferentiated count would have ranked top. The instrument has to expose
    // both numbers, because the JSON and MCP surfaces are what other agents rank
    // on and they cannot recover the split from a single total.
    let (_dir, reader) = store_with_one_bundler();
    let ctx = Context {
        as_of: AsOf::at(Slot(500_000)),
        store: &reader,
    };
    // A registry of its own: the shared one holds only `creator_history`, and
    // widening it would move counts other tests assert on.
    let mut r = Registry::new();
    r.register(radar_instruments::CreatorTrackRecord);
    let record = r
        .invoke(
            "creator_track_record",
            json!({ "creator": creator(7).to_string() }),
            &ctx,
        )
        .expect("instrument exists");
    let o = record.output.expect("succeeded");

    assert_eq!(o["measured"], 7);
    assert_eq!(o["graduated"], 7, "flawless, on the old metric");
    assert_eq!(o["graduated_instant"], 6);
    assert_eq!(o["graduated_organic"], 1);

    // Above the five-sample floor, so both rates are stated rather than withheld.
    assert_eq!(o["graduation_rate"], 1.0);
    let organic = o["organic_graduation_rate"].as_f64().expect("a rate");
    assert!(
        (organic - 1.0 / 7.0).abs() < 1e-9,
        "organic rate must reflect the one real graduation, got {organic}"
    );
}
