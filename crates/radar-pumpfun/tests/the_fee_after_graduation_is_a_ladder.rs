// SPDX-License-Identifier: Apache-2.0
//! The fee a trade pays after graduation, asserted against the account that
//! sets it.
//!
//! [research 0023](../../../docs/research/0023-the-fee-is-a-schedule-and-the-published-interface-is-incomplete.md)
//! read the bonding curve's schedule and found one tier. Design 0009 §1 then
//! said, correctly, that nothing had measured the fee **after** graduation, and
//! that the venue's help centre described a ladder the chain had not been asked
//! about. [research 0028](../../../docs/research/0028-the-fee-after-graduation-is-a-ladder.md)
//! asked. The fee program keeps one schedule per program, and PumpSwap's --
//! `fee_config` seeded with the PumpSwap program id -- has twenty-five rows keyed
//! on market capitalisation in lamports. The same parser reads it; nothing in
//! `fees.rs` changed.
//!
//! # What this pins
//!
//! The rows the prize arithmetic depends on: 30 bps to the creator below 420
//! SOL of market cap, 95 from there to 1,470, and 5 at the top. Three live swaps
//! in the note paid exactly the row their pool's market cap selects, and two
//! paid a row further down the ladder than their cap selects -- both in the
//! direction of a cap once reached and since lost -- so the tier *lookup* is
//! asserted here only where the chain agreed with it. The disagreement is the
//! note's open question, not this file's.
//!
//! # Three accounts, three answers
//!
//! PumpSwap's global config says the creator gets 5 bps. The schedule's flat
//! entry says 0. The schedule's rows say 5 to 95. Live swaps pass the fee
//! program and the schedule, and pay the rows; the other two are fossils, and
//! the disagreement is asserted so that the day it closes something fails.

use radar_pumpfun::fees::{FeeConfig, Fees};

const FIXTURE: &str = include_str!("fixtures/pumpswap_fees.json");

fn fixture() -> serde_json::Value {
    serde_json::from_str(FIXTURE).expect("the fixture is valid JSON")
}

/// The account bytes, as captured.
fn account(name: &str) -> Vec<u8> {
    let value = fixture();
    let hex = value["accounts"][name]["data_hex"]
        .as_str()
        .expect("the fixture carries this account")
        .to_owned();
    assert!(hex.len().is_multiple_of(2), "hex is whole bytes");
    (0..hex.len() / 2)
        .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).expect("the fixture is valid hex"))
        .collect()
}

fn schedule() -> FeeConfig {
    FeeConfig::parse(&account("fee_config"))
        .expect("PumpSwap's fee config parses with the curve's parser")
}

const SOL: u128 = 1_000_000_000;

#[test]
fn the_schedule_after_graduation_has_twenty_five_rows_and_the_curve_has_one() {
    let after = schedule();
    assert_eq!(after.tiers.len(), 25);
    // The curve's config, captured for 0023, is the other file. One row there.
    let curve: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/pumpfun_fees.json")).expect("valid JSON");
    let hex = curve["accounts"]["fee_config"]["data_hex"]
        .as_str()
        .expect("hex");
    let bytes: Vec<u8> = (0..hex.len() / 2)
        .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).expect("hex"))
        .collect();
    assert_eq!(FeeConfig::parse(&bytes).expect("parses").tiers.len(), 1);
}

#[test]
fn the_creator_fee_is_thirty_below_420_sol_ninety_five_above_it_and_five_at_the_top() {
    let after = schedule();
    let creator = |cap_sol: u128| {
        after
            .fees_at_market_cap(cap_sol * SOL)
            .expect("a row covers this cap")
            .creator_bps
    };
    // The three rows the prize arithmetic turns on. On the venue's published
    // initial virtual reserves -- 30 SOL against 1,073,000,000 tokens, with
    // 793,100,000 for sale, a claim this repository has not captured -- a curve
    // completes at about 411 SOL of market cap, which is the first row; the
    // first step up is nine SOL of market cap away.
    assert_eq!(creator(0), 30);
    assert_eq!(creator(411), 30);
    assert_eq!(creator(419), 30);
    assert_eq!(creator(420), 95);
    assert_eq!(creator(1_469), 95);
    assert_eq!(creator(1_470), 90);
    assert_eq!(creator(98_239), 8);
    assert_eq!(creator(98_240), 5);
    assert_eq!(creator(1_500_000), 5);
}

#[test]
fn the_ladder_only_goes_down_after_the_first_step() {
    // 30, then 95, then monotonically down to 5. A row out of order would be a
    // schedule nobody checked, and the parser reads rows in the order written.
    let rows: Vec<(u128, u64)> = schedule()
        .tiers
        .iter()
        .map(|t| (t.threshold_lamports, t.fees.creator_bps))
        .collect();
    assert_eq!(rows[0], (0, 30));
    assert_eq!(rows[1], (420 * SOL, 95));
    for pair in rows[1..].windows(2) {
        assert!(pair[0].0 < pair[1].0, "thresholds ascend: {pair:?}");
        assert!(pair[0].1 > pair[1].1, "creator fee descends: {pair:?}");
    }
    assert_eq!(rows[24], (98_240 * SOL, 5));
}

#[test]
fn the_liquidity_providers_are_paid_after_graduation_and_the_protocol_takes_five() {
    // Zero on the curve (0023). Twenty on the AMM at every row but the first,
    // where the split is 2 / 93 / 30 and the total is the curve's 125.
    let after = schedule();
    assert_eq!(
        after.fees_at_market_cap(0).expect("row"),
        Fees {
            lp_bps: 2,
            protocol_bps: 93,
            creator_bps: 30,
        }
    );
    for tier in &after.tiers[1..] {
        assert_eq!(tier.fees.lp_bps, 20, "{tier:?}");
        assert_eq!(tier.fees.protocol_bps, 5, "{tier:?}");
    }
}

#[test]
fn three_live_swaps_paid_the_row_their_market_cap_selects() {
    // Research 0028's table. Market cap is the pool's pre-trade quote reserve
    // times the mint's supply over its base reserve, in SOL; the fee is what the
    // swap event said the coin creator was paid, in basis points.
    let after = schedule();
    for (cap_sol, paid) in [(119, 30), (972, 95), (96_632, 8), (1_427_780, 5)] {
        assert_eq!(
            after
                .fees_at_market_cap(cap_sol * SOL)
                .expect("row")
                .creator_bps,
            paid,
            "a pool at {cap_sol} SOL paid {paid}"
        );
    }
}

#[test]
fn the_flat_entry_and_the_global_config_still_disagree_with_the_rows() {
    // Flat: lp 25, protocol 5, creator 0. Global config (PumpSwap's own
    // account, offsets 40 and 48 for lp and protocol, 313 for the creator
    // after eight recipient keys): 20, 5, 5. Neither is what a swap pays.
    let after = schedule();
    assert_eq!(
        after.flat,
        Fees {
            lp_bps: 25,
            protocol_bps: 5,
            creator_bps: 0,
        }
    );
    let global = account("global_config");
    let u64_at = |at: usize| u64::from_le_bytes(global[at..at + 8].try_into().expect("8 bytes"));
    assert_eq!((u64_at(40), u64_at(48), u64_at(313)), (20, 5, 5));
    assert_ne!(after.flat.creator_bps, after.tiers[0].fees.creator_bps);
    assert_ne!(u64_at(313), after.tiers[0].fees.creator_bps);
}
