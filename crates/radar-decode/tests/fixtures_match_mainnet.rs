// SPDX-License-Identifier: Apache-2.0
//! Asserts the discriminator table still matches reality.
//!
//! Two independent checks, because either alone can pass while the table is
//! wrong:
//!
//! 1. **Against the chain.** Every fixture is raw instruction data captured from
//!    mainnet, paired with the name the program itself logged. Decoding the
//!    bytes must yield that instruction. This catches a table entry that was
//!    typed wrong.
//! 2. **Against the convention.** Recomputing `sha256("global:" + anchor_name)`
//!    must reproduce the same eight bytes. This catches a fixture captured from
//!    a mislabelled transaction, and documents where the constants come from.
//!
//! Regenerate fixtures with `python scripts/probe/capture_fixtures.py`.

use std::collections::BTreeMap;

use base64::Engine as _;
use radar_decode::pumpfun::{self, Instruction};
use radar_decode::{Decoded, Discriminator, decode_pumpfun};
use sha2::{Digest, Sha256};

const FIXTURES: &str = include_str!("fixtures/pumpfun_instructions.json");

#[derive(serde::Deserialize)]
struct Fixture {
    logged_name: String,
    snake_case: String,
    discriminator: Vec<u8>,
    instruction_data_b64: String,
    data_len: usize,
    signature: String,
}

#[derive(serde::Deserialize)]
struct Fixtures {
    program: String,
    instructions: BTreeMap<String, Fixture>,
}

fn load() -> Fixtures {
    serde_json::from_str(FIXTURES).expect("fixtures parse")
}

/// Maps the name pump.fun logs to the variant it should decode to.
fn variant_for(logged: &str) -> Option<Instruction> {
    Some(match logged {
        "CreateV2" => Instruction::CreateV2,
        "Buy" => Instruction::Buy,
        "BuyV2" => Instruction::BuyV2,
        "BuyExactSolIn" => Instruction::BuyExactSolIn,
        "BuyExactQuoteInV2" => Instruction::BuyExactQuoteInV2,
        "Sell" => Instruction::Sell,
        "SellV2" => Instruction::SellV2,
        "ClaimCashback" => Instruction::ClaimCashback,
        "ClaimCashbackV2" => Instruction::ClaimCashbackV2,
        "CollectCreatorFee" => Instruction::CollectCreatorFee,
        "CollectCreatorFeeV2" => Instruction::CollectCreatorFeeV2,
        "DistributeCreatorFees" => Instruction::DistributeCreatorFees,
        "InitUserVolumeAccumulator" => Instruction::InitUserVolumeAccumulator,
        "CloseUserVolumeAccumulator" => Instruction::CloseUserVolumeAccumulator,
        _ => return None,
    })
}

#[test]
fn the_program_id_matches_the_fixtures() {
    assert_eq!(pumpfun::PROGRAM_ID.to_string(), load().program);
}

#[test]
fn real_mainnet_bytes_decode_to_the_instruction_the_program_logged() {
    let f = load();
    assert!(!f.instructions.is_empty(), "no fixtures captured");

    for (key, fx) in &f.instructions {
        let data = base64::engine::general_purpose::STANDARD
            .decode(&fx.instruction_data_b64)
            .unwrap_or_else(|e| panic!("{key}: bad base64: {e}"));
        assert_eq!(
            data.len(),
            fx.data_len,
            "{key}: fixture length disagrees with itself"
        );

        let Some(expected) = variant_for(&fx.logged_name) else {
            panic!(
                "{key}: mainnet is running an instruction the table does not know \
                 (sig {}). Add it rather than deleting the fixture.",
                fx.signature
            );
        };

        match decode_pumpfun(&data) {
            Decoded::Known(got) => assert_eq!(
                got, expected,
                "{key}: decoded to {got:?} but the program logged {}",
                fx.logged_name
            ),
            other => panic!(
                "{key}: expected {expected:?}, got {other:?} (sig {})",
                fx.signature
            ),
        }
    }
}

#[test]
fn table_discriminators_match_the_anchor_naming_convention() {
    // Anchor derives these as sha256("global:" + snake_case_name)[..8]. Every
    // one of the fourteen captured from mainnet matched, so the convention holds
    // for this program and the constants are not arbitrary.
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
fn fixture_discriminators_match_the_table() {
    let f = load();
    for (key, fx) in &f.instructions {
        let Some(expected) = variant_for(&fx.logged_name) else {
            continue;
        };
        let captured: [u8; 8] = fx.discriminator[..].try_into().expect("8 bytes");
        assert_eq!(
            expected.discriminator().as_bytes(),
            &captured,
            "{key}: table and captured bytes disagree"
        );
    }
}

#[test]
fn the_snake_case_names_agree_between_fixture_and_table() {
    // The capture script derives snake_case from the logged name and the table
    // carries it independently. If those ever disagree, the discriminator in the
    // table was computed from a different name than the one on chain, and the
    // convention test would be checking the table against itself.
    let f = load();
    for (key, fx) in &f.instructions {
        let Some(ix) = variant_for(&fx.logged_name) else {
            continue;
        };
        assert_eq!(
            ix.anchor_name(),
            fx.snake_case,
            "{key}: table name and captured name disagree"
        );
    }
}

#[test]
fn every_table_entry_has_a_fixture() {
    // A constant with nothing behind it is a claim. If an instruction is in the
    // table but was never seen on chain, either it is dead or the capture window
    // was too small -- and either way it should not be silently trusted.
    let f = load();
    let captured: Vec<Instruction> = f
        .instructions
        .values()
        .filter_map(|fx| variant_for(&fx.logged_name))
        .collect();

    let missing: Vec<_> = pumpfun::KNOWN
        .iter()
        .map(|(ix, _, _)| *ix)
        .filter(|ix| !captured.contains(ix))
        .collect();

    assert!(
        missing.is_empty(),
        "table entries never observed on mainnet: {missing:?} — \
         re-run scripts/probe/capture_fixtures.py over a wider window, or drop them"
    );
}
