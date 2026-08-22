// SPDX-License-Identifier: Apache-2.0
//! Asserts the discriminator table still matches reality.
//!
//! Fixtures are every distinct discriminator pump.fun emitted across three
//! two-hour windows, pulled from CryptoHouse (ADR 0002) together with a real
//! sample of each one's instruction data. Sourcing them that way rather than by
//! sampling RPC slots is what surfaced the eleven instructions the first capture
//! missed — including `create`, the original launch path, which is still live.
//!
//! Four independent checks, because any one of them can pass while the table is
//! wrong:
//!
//! 1. Real captured bytes decode to the expected instruction.
//! 2. `sha256("global:" + name)[..8]` reproduces the table's bytes.
//! 3. Every table entry was actually observed on chain.
//! 4. Instructions seen on chain but *not* in the table decode to `Unknown` —
//!    never silently to something else.
//!
//! Regenerate with `python scripts/probe/capture_fixtures_cryptohouse.py`.

use std::collections::BTreeMap;

use radar_decode::pumpfun::{self, Instruction};
use radar_decode::{Decoded, Discriminator, decode_pumpfun};
use sha2::{Digest, Sha256};

const FIXTURES: &str = include_str!("fixtures/pumpfun_instructions.json");

#[derive(serde::Deserialize)]
struct Fixture {
    discriminator: String,
    anchor_name: Option<String>,
    kind: String,
    observed_count: u64,
    sample_data_b58: String,
    min_data_len: usize,
    example_signature: String,
}

#[derive(serde::Deserialize)]
struct Fixtures {
    program: String,
    instructions: BTreeMap<String, Fixture>,
}

fn load() -> Fixtures {
    serde_json::from_str(FIXTURES).expect("fixtures parse")
}

fn decode_b58(s: &str) -> Vec<u8> {
    bs58::decode(s)
        .into_vec()
        .expect("fixture sample is valid base58")
}

/// The table, keyed by Anchor name.
fn by_name(name: &str) -> Option<Instruction> {
    pumpfun::KNOWN
        .iter()
        .find(|(_, _, n)| *n == name)
        .map(|(ix, _, _)| *ix)
}

#[test]
fn the_program_id_matches_the_fixtures() {
    assert_eq!(pumpfun::PROGRAM_ID.to_string(), load().program);
}

#[test]
fn captured_mainnet_bytes_decode_to_the_expected_instruction() {
    let f = load();
    assert!(!f.instructions.is_empty(), "no fixtures captured");
    let mut checked = 0;

    for (disc_hex, fx) in &f.instructions {
        let Some(name) = fx.anchor_name.as_deref() else {
            continue;
        };
        let Some(expected) = by_name(name) else {
            panic!(
                "mainnet emits `{name}` ({disc_hex}, {} times, sig {}) and the table has no \
                 entry for it. Add it rather than dropping the fixture.",
                fx.observed_count, fx.example_signature
            );
        };

        let data = decode_b58(&fx.sample_data_b58);
        assert!(
            data.len() >= fx.min_data_len,
            "{name}: sample is shorter than the observed minimum"
        );
        match decode_pumpfun(&data) {
            Decoded::Known(got) => assert_eq!(
                got, expected,
                "{name} ({disc_hex}): decoded to {got:?} (sig {})",
                fx.example_signature
            ),
            other => panic!("{name} ({disc_hex}): expected {expected:?}, got {other:?}"),
        }
        checked += 1;
    }
    assert!(
        checked >= 20,
        "only {checked} named fixtures checked; expected the full table"
    );
}

#[test]
fn table_discriminators_match_the_anchor_naming_convention() {
    // Anchor derives these as sha256("global:" + snake_case_name)[..8]. Every
    // named instruction captured from mainnet matched, so the constants are
    // derived rather than arbitrary — but they are stored as bytes, because
    // bytes are what the chain sends and names are what drift.
    for (ix, bytes, anchor_name) in pumpfun::KNOWN {
        let mut h = Sha256::new();
        h.update(format!("global:{anchor_name}").as_bytes());
        let computed: [u8; 8] = h.finalize()[..8].try_into().expect("8 bytes");
        assert_eq!(
            &computed,
            bytes,
            "{ix:?}: table says {} but sha256(\"global:{anchor_name}\")[..8] is {}",
            Discriminator::new(*bytes),
            Discriminator::new(computed)
        );
    }
}

#[test]
fn fixture_discriminators_agree_with_the_table_bytes() {
    let f = load();
    for (disc_hex, fx) in &f.instructions {
        let Some(name) = fx.anchor_name.as_deref() else {
            continue;
        };
        let Some(ix) = by_name(name) else { continue };
        assert_eq!(
            ix.discriminator().to_string(),
            *disc_hex,
            "{name}: table bytes and captured bytes disagree"
        );
    }
}

#[test]
fn every_table_entry_was_observed_on_chain() {
    // A constant with nothing behind it is a claim. If an instruction is in the
    // table but never appeared, either it is dead or the capture window was too
    // narrow — and either way it should not be silently trusted.
    let f = load();
    let observed: Vec<&str> = f
        .instructions
        .values()
        .filter_map(|fx| fx.anchor_name.as_deref())
        .collect();

    let missing: Vec<&str> = pumpfun::KNOWN
        .iter()
        .map(|(_, _, n)| *n)
        .filter(|n| !observed.contains(n))
        .collect();

    assert!(
        missing.is_empty(),
        "table entries never observed on mainnet: {missing:?} — \
         widen the windows in capture_fixtures_cryptohouse.py, or drop them"
    );
}

#[test]
fn instructions_the_table_does_not_know_decode_to_unknown() {
    // Three discriminators appear on chain whose names resisted an 8,064-candidate
    // brute force. They are real instructions and must stay visible as Unknown so
    // the unknown-rate alarm can see them — never silently mapped to something
    // that happens to be nearby.
    let f = load();
    let mut unknown_seen = 0;

    for (disc_hex, fx) in &f.instructions {
        if fx.anchor_name.is_some() || fx.kind != "instruction" {
            continue;
        }
        let data = decode_b58(&fx.sample_data_b58);
        let d = decode_pumpfun(&data);
        assert!(
            d.is_unrecognised(),
            "unnamed discriminator {disc_hex} decoded to {d:?} — a name was guessed"
        );
        unknown_seen += 1;
    }
    assert!(
        unknown_seen > 0,
        "expected at least one unnamed instruction in the fixtures"
    );
}

#[test]
fn the_anchor_event_tag_is_present_and_not_treated_as_an_instruction() {
    // It is the second highest-volume discriminator on the program. Treating it
    // as an instruction would double-count every trade that emits an event.
    let f = load();
    let event = f
        .instructions
        .values()
        .find(|fx| fx.kind == "anchor_event_cpi")
        .expect("the anchor event CPI tag should appear in any real capture");

    assert_eq!(event.discriminator, pumpfun::ANCHOR_EVENT_CPI.to_string());
    let data = decode_b58(&event.sample_data_b58);
    assert!(
        decode_pumpfun(&data).is_unrecognised(),
        "the event CPI tag must not resolve to a user instruction"
    );
}
