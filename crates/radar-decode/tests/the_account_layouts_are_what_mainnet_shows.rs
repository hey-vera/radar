// SPDX-License-Identifier: Apache-2.0
//! The account layouts [ADR 0009](../../../docs/adr/0009-radar-builds-its-own-pump-fun-swaps.md)
//! has to build, captured from mainnet.
//!
//! # Why this exists before the builder does
//!
//! ADR 0009's first precondition: *the account list comes from mainnet, not from
//! a reference*, held to the standard `pumpfun.rs` already sets for
//! discriminators — public references describe a program with roughly three
//! instructions and the live one has twenty-one.
//!
//! An instruction built with the wrong accounts does not fail cleanly. It fails
//! at simulation if you are lucky, and succeeds against the wrong account if you
//! are not, which is the failure mode that costs money rather than time. So the
//! layout is captured first, asserted here, and only then built against.
//!
//! # What the capture already established
//!
//! Three things that would each have been a plausible wrong assumption:
//!
//! - **Every one of these transactions is legacy.** The curve *is* reachable in
//!   a legacy transaction — people trade it that way every block. Research 0021
//!   is about the aggregator refusing to route one, not about the venue.
//! - **`buy` and `sell` do not share an account order.** Buy carries the token
//!   program at index 8 and the creator vault at 9; sell has them the other way
//!   round, and one account fewer. Assuming symmetry here builds a sell that
//!   passes the wrong account as the token program.
//! - **The token program varies by mint.** One capture uses SPL Token and two use
//!   Token-2022. It is an input, never a constant.

use std::collections::BTreeSet;

/// The captured transactions.
const FIXTURE: &str = include_str!("fixtures/pumpfun_accounts.json");

/// The pump.fun program, which also appears *inside* its own account lists —
/// Anchor's event-CPI convention passes the program to itself.
const PROGRAM: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";

const SYSTEM_PROGRAM: &str = "11111111111111111111111111111111";
const SPL_TOKEN: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const TOKEN_2022: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";

fn fixture() -> serde_json::Value {
    serde_json::from_str(FIXTURE).expect("the fixture is valid JSON")
}

fn instructions() -> Vec<serde_json::Value> {
    fixture()["instructions"]
        .as_array()
        .expect("an instructions array")
        .clone()
}

fn named(name: &str) -> serde_json::Value {
    instructions()
        .into_iter()
        .find(|i| i["name"] == name)
        .unwrap_or_else(|| panic!("no {name} in the fixture"))
}

fn keys(ix: &serde_json::Value) -> Vec<String> {
    ix["accounts"]
        .as_array()
        .expect("accounts")
        .iter()
        .map(|a| a["pubkey"].as_str().expect("a pubkey").to_owned())
        .collect()
}

#[test]
fn the_fixture_holds_the_three_instructions_a_trader_needs() {
    // Both buy variants and the sell. Radar's thesis is exit-first, so a capture
    // without a sell would be the half that does not matter.
    let names: BTreeSet<String> = instructions()
        .iter()
        .map(|i| i["name"].as_str().expect("a name").to_owned())
        .collect();
    assert!(names.contains("buy"), "{names:?}");
    assert!(names.contains("buy_exact_sol_in"), "{names:?}");
    assert!(names.contains("sell"), "{names:?}");
}

#[test]
fn every_captured_discriminator_is_one_the_decoder_already_knows() {
    // The single-table rule from ADR 0009's precondition 2. A builder that
    // learned its own discriminators could disagree with the decoder about the
    // same program, and the disagreement would be invisible until a transaction
    // was rejected.
    for ix in instructions() {
        let hex = ix["discriminator"].as_str().expect("a discriminator");
        let bytes: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("hex"))
            .collect();
        let known = radar_decode::pumpfun::KNOWN
            .iter()
            .find(|(_, d, _)| d.as_slice() == bytes.as_slice());
        let (_, _, name) =
            known.unwrap_or_else(|| panic!("{hex} is not in radar_decode::pumpfun::KNOWN"));
        assert_eq!(
            *name,
            ix["name"].as_str().expect("a name"),
            "the fixture and the decoder disagree about {hex}"
        );
    }
}

#[test]
fn every_capture_is_a_legacy_transaction() {
    // The finding that makes ADR 0009 possible at all. The curve is reachable in
    // a legacy transaction -- the signer's requirement is satisfiable on this
    // venue, and 0021 is about the aggregator rather than the chain.
    for ix in instructions() {
        assert_eq!(
            ix["version"], "legacy",
            "{} was captured from a versioned transaction",
            ix["name"]
        );
    }
}

#[test]
fn buy_and_sell_do_not_share_an_account_order() {
    // The assumption most likely to be made and most expensive to get wrong. A
    // sell built on the buy layout passes the creator vault where the token
    // program belongs.
    let buy = keys(&named("buy"));
    let sell = keys(&named("sell"));

    assert_eq!(buy.len(), 18, "buy layout changed");
    assert_eq!(sell.len(), 17, "sell layout changed");

    assert!(
        buy[8] == SPL_TOKEN || buy[8] == TOKEN_2022,
        "buy[8] is the token program, got {}",
        buy[8]
    );
    assert!(
        sell[9] == SPL_TOKEN || sell[9] == TOKEN_2022,
        "sell[9] is the token program, got {}",
        sell[9]
    );
    assert!(
        sell[8] != SPL_TOKEN && sell[8] != TOKEN_2022,
        "sell[8] is the creator vault, not the token program"
    );
}

#[test]
fn the_token_program_is_an_input_and_never_a_constant() {
    // One capture is SPL Token and the others are Token-2022. A builder that
    // hardcodes either produces a transaction that fails for half of all mints.
    let programs: BTreeSet<String> = instructions()
        .iter()
        .flat_map(keys)
        .filter(|k| k == SPL_TOKEN || k == TOKEN_2022)
        .collect();
    assert_eq!(
        programs.len(),
        2,
        "both token programs must appear, or this fixture cannot show the \
         difference: {programs:?}"
    );
}

#[test]
fn the_invariant_accounts_really_are_invariant() {
    // Some accounts are the same in every capture -- the global config, the
    // event authority, the program itself, the system program. Those are the
    // ones a builder may hold as constants, and this is what says which.
    //
    // Everything else varies per trade and must be derived or passed in.
    let all: Vec<Vec<String>> = instructions().iter().map(keys).collect();
    let shared: BTreeSet<String> = all.iter().skip(1).fold(
        all[0].iter().cloned().collect(),
        |acc: BTreeSet<String>, ks| {
            acc.intersection(&ks.iter().cloned().collect())
                .cloned()
                .collect()
        },
    );

    for expected in [PROGRAM, SYSTEM_PROGRAM] {
        assert!(
            shared.contains(expected),
            "{expected} should be in every capture: {shared:?}"
        );
    }
    // The global config and the event authority are shared too, and they are
    // *discovered* rather than named here: hardcoding an address this test then
    // asserts would make it a copy of the builder rather than a check on it.
    assert!(
        shared.len() >= 4,
        "expected at least the program, system program, global config and event \
         authority to be invariant, found {shared:?}"
    );
}

#[test]
fn exactly_one_account_signs_and_it_is_writable() {
    // The trader. A layout with two signers would need a second key at signing
    // time, which `radar-signer` has no way to provide -- so this is a property
    // the builder depends on rather than a curiosity.
    for ix in instructions() {
        let signers: Vec<&serde_json::Value> = ix["accounts"]
            .as_array()
            .expect("accounts")
            .iter()
            .filter(|a| a["signer"] == true)
            .collect();
        assert_eq!(
            signers.len(),
            1,
            "{} has {} signers",
            ix["name"],
            signers.len()
        );
        assert_eq!(
            signers[0]["writable"], true,
            "{}'s signer pays fees and must be writable",
            ix["name"]
        );
    }
}
