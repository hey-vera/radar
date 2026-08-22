// SPDX-License-Identifier: Apache-2.0
#![expect(
    clippy::cast_precision_loss,
    reason = "sample counts are in the thousands; f64 is exact well past that and these               are share-of-total ratios for a threshold check, not accounting"
)]
//! Asserts the argument layouts hold across many real payloads.
//!
//! The layouts were derived by reading one sample of each instruction, which is
//! enough to form a hypothesis and nowhere near enough to trust it. A field that
//! happens to be zero, or a length that happens to be typical, agrees with almost
//! any wrong guess.
//!
//! These tests run the decoder over 1,757 payloads captured from mainnet and
//! assert properties that a wrong layout could not satisfy. The important one is
//! [`swapping_the_layout_would_produce_absurd_amounts`]: it is not enough to show
//! the current reading is plausible, because a test that only ever confirms the
//! implementation is not a test. It also has to show the alternative is not.
//!
//! Regenerate with `python scripts/probe/capture_payloads.py`.

use std::collections::BTreeMap;

use radar_decode::pumpfun::Instruction;
use radar_decode::{Layout, Side, decode_pumpfun_launch, decode_pumpfun_trade};

const PAYLOADS: &str = include_str!("fixtures/pumpfun_payloads.json");

const LAMPORTS_PER_SOL: u64 = 1_000_000_000;
/// The widest bracket a real pump.fun trade falls in.
///
/// The floor is deliberately low. An earlier version of this test used 0.0001 SOL
/// and failed on a genuine 32,113-lamport trade -- about half a cent, and exactly
/// the size fee-farming flow uses. Dust is real activity, not corrupt data.
const MIN_PLAUSIBLE_LAMPORTS: u64 = 1_000;
const MAX_PLAUSIBLE_LAMPORTS: u64 = 5_000 * LAMPORTS_PER_SOL;

/// What fraction of a sample's values land in the plausible SOL bracket.
fn plausible_share(values: &[u64]) -> f64 {
    let hits = values
        .iter()
        .filter(|v| (MIN_PLAUSIBLE_LAMPORTS..=MAX_PLAUSIBLE_LAMPORTS).contains(v))
        .count();
    hits as f64 / values.len() as f64
}

#[derive(serde::Deserialize)]
struct Sample {
    data_b58: String,
    signature: String,
}

#[derive(serde::Deserialize)]
struct InstructionPayloads {
    samples: Vec<Sample>,
}

#[derive(serde::Deserialize)]
struct Payloads {
    instructions: BTreeMap<String, InstructionPayloads>,
}

fn load() -> Payloads {
    serde_json::from_str(PAYLOADS).expect("payload fixtures parse")
}

fn bytes(s: &Sample) -> Vec<u8> {
    bs58::decode(&s.data_b58)
        .into_vec()
        .expect("sample is valid base58")
}

fn instruction_named(name: &str) -> Instruction {
    let Some((ix, _, _)) = radar_decode::pumpfun::KNOWN
        .iter()
        .find(|(_, _, n)| *n == name)
    else {
        panic!("fixture names an instruction the table lacks: {name}")
    };
    *ix
}

#[test]
fn every_captured_payload_decodes() {
    let p = load();
    let mut trades = 0;
    let mut launches = 0;

    for (name, blob) in &p.instructions {
        let ix = instruction_named(name);
        for s in &blob.samples {
            let data = bytes(s);
            if ix.is_trade() {
                let t = decode_pumpfun_trade(&data)
                    .unwrap_or_else(|| panic!("{name}: not recognised as a trade"))
                    .unwrap_or_else(|e| panic!("{name} sig {}: {e}", s.signature));
                assert_eq!(t.side, if ix.is_buy() { Side::Buy } else { Side::Sell });
                trades += 1;
            } else if ix.is_launch() {
                decode_pumpfun_launch(&data)
                    .unwrap_or_else(|| panic!("{name}: not recognised as a launch"))
                    .unwrap_or_else(|e| panic!("{name} sig {}: {e}", s.signature));
                launches += 1;
            }
        }
    }
    assert!(trades > 1_000, "only {trades} trades exercised");
    assert!(launches > 200, "only {launches} launches exercised");
}

#[test]
fn the_layout_is_distinguishable_from_its_opposite() {
    // The discriminating test, arrived at in three attempts, each of which taught
    // something:
    //
    //  1. "our SOL field is plausible" -- passed trivially. A test that only
    //     confirms the implementation is not a test.
    //  2. "the other field would be absurd above 5,000 SOL" -- failed at 45%.
    //     Token base units and lamports overlap in magnitude more than expected.
    //  3. relative plausibility -- worked for five of six instructions, and left
    //     `sell` at 74% against 53%, too close to call.
    //
    // What settles `sell` is the sentinel. Of 250 real sells, 146 set their
    // minimum SOL output to *exactly zero* -- panic exits and bots indifferent to
    // slippage. A token amount is never exactly zero, because a trade for nothing
    // is not a trade. So a field that is frequently zero is a bound, and a field
    // that is never zero is an amount. That is structural rather than statistical,
    // and it does not depend on any threshold being right.
    let p = load();
    let mut checked = 0;

    for (name, blob) in &p.instructions {
        let ix = instruction_named(name);
        if !ix.is_trade() {
            continue;
        }

        let mut exacts = Vec::new();
        let mut bounds = Vec::new();
        for s in &blob.samples {
            let t = decode_pumpfun_trade(&bytes(s))
                .expect("trade")
                .expect("decodes");
            exacts.push(t.exact.raw());
            bounds.push(t.limit.raw());
        }

        // An exact amount is never a sentinel. This alone would fail loudly if
        // the two fields were swapped for any instruction whose bounds are often
        // zero.
        for (v, s) in exacts.iter().zip(&blob.samples) {
            assert!(
                *v != 0 && *v != u64::MAX,
                "{name} sig {}: the field read as an exact amount is a sentinel                  ({v}), which means the layout is swapped",
                s.signature
            );
        }

        let sentinel_bounds = bounds
            .iter()
            .filter(|v| **v == 0 || **v == u64::MAX)
            .count() as f64
            / bounds.len() as f64;

        let sol_side: Vec<u64> = match ix.layout() {
            Some(Layout::SolThenTokenBound) => exacts.clone(),
            _ => bounds
                .iter()
                .copied()
                .filter(|v| *v != 0 && *v != u64::MAX)
                .collect(),
        };
        let token_side: Vec<u64> = match ix.layout() {
            Some(Layout::SolThenTokenBound) => bounds.clone(),
            _ => exacts.clone(),
        };

        let gap = if sol_side.len() >= 20 {
            plausible_share(&sol_side) - plausible_share(&token_side)
        } else {
            0.0
        };

        assert!(
            gap > 0.25 || sentinel_bounds > 0.05,
            "{name}: neither test distinguishes the layout from its opposite --              plausibility gap {:.0}%, sentinel bounds {:.0}%. Re-derive the layout              rather than trusting it.",
            gap * 100.0,
            sentinel_bounds * 100.0
        );
        checked += 1;
    }
    assert!(checked >= 6, "only {checked} trade instructions exercised");
}

#[test]
fn sol_exact_instructions_pin_the_sol_side() {
    let p = load();
    for name in ["buy_exact_sol_in", "buy_exact_quote_in_v2"] {
        let Some(blob) = p.instructions.get(name) else {
            continue;
        };
        assert_eq!(
            instruction_named(name).layout(),
            Some(Layout::SolThenTokenBound)
        );

        let values: Vec<u64> = blob
            .samples
            .iter()
            .map(|s| {
                decode_pumpfun_trade(&bytes(s))
                    .expect("trade")
                    .expect("decodes")
                    .exact_lamports()
                    .expect("SOL-exact instruction pins lamports")
            })
            .collect();
        let share = plausible_share(&values);
        assert!(
            share > 0.95,
            "{name}: only {:.0}% of SOL amounts plausible",
            share * 100.0
        );
    }
}

#[test]
fn token_exact_instructions_pin_a_nonzero_token_amount() {
    // A zero token amount would mean the offset is wrong: nobody submits a trade
    // for nothing, and the program would reject it if they did.
    let p = load();
    for name in ["buy", "buy_v2", "sell", "sell_v2"] {
        let Some(blob) = p.instructions.get(name) else {
            continue;
        };
        assert_eq!(
            instruction_named(name).layout(),
            Some(Layout::TokensThenSolBound)
        );

        let mut unbounded = 0;
        for s in &blob.samples {
            let t = decode_pumpfun_trade(&bytes(s))
                .expect("trade")
                .expect("decodes");
            assert!(
                t.exact_tokens().is_some_and(|v| v > 0),
                "{name} sig {}: zero token amount",
                s.signature
            );
            assert!(
                t.limit.lamports().is_some(),
                "{name}: bound is not lamports"
            );
            if t.accepted_any_price() {
                unbounded += 1;
            }
        }
        // Sells especially set a zero floor constantly -- panic exits and bots
        // that do not care about slippage. Recorded because it is a behavioural
        // signal, not noise.
        if name.starts_with("sell") {
            assert!(unbounded > 0, "{name}: expected some unbounded sells");
        }
    }
}

#[test]
fn launch_metadata_decodes_as_creator_supplied_text() {
    // Names and symbols are arbitrary creator-controlled bytes -- emoji, scripts,
    // padding, deliberate lookalikes. The decoder must read them without
    // mangling and the creator must never come out as the zero address.
    let p = load();
    let mut with_uri = 0;
    let mut total = 0;

    for name in ["create_v2", "create"] {
        let Some(blob) = p.instructions.get(name) else {
            continue;
        };
        for s in &blob.samples {
            let data = bytes(s);
            let l = decode_pumpfun_launch(&data)
                .expect("launch")
                .expect("decodes");
            assert_ne!(
                l.creator,
                radar_types::Address::new([0u8; 32]),
                "{name} sig {}: creator decoded as the system program, which means the \
                 field offset is wrong",
                s.signature
            );
            assert!(
                !l.symbol.is_empty(),
                "{name} sig {}: empty symbol",
                s.signature
            );
            if l.uri.starts_with("https://") || l.uri.starts_with("ipfs://") {
                with_uri += 1;
            }
            total += 1;
        }
    }

    assert!(total > 200, "only {total} launches exercised");
    // Not all URIs are well-formed and that is the creator's business, but if
    // almost none parse as URLs the field offset is wrong rather than the data.
    let share = f64::from(with_uri) / f64::from(total);
    assert!(
        share > 0.8,
        "only {:.0}% of launch URIs look like URLs",
        share * 100.0
    );
}
