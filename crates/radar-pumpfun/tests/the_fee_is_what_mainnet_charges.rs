// SPDX-License-Identifier: Apache-2.0
//! The fee a pump.fun trade pays, asserted against the accounts that set it.
//!
//! # Why this is a test and not a constant
//!
//! [research 0022](../../../docs/research/0022-capacity-was-a-budget-not-a-ceiling.md)
//! priced the round trip with a 250 bps fee component that it took as given. A
//! number taken as given is the thing this repository keeps finding was wrong,
//! so it is read from the chain here and checked against the bytes.
//!
//! It survives the check, and the reason it does is worth stating: 250 bps is
//! two sides of a 125 bps fee, and 125 is what the fee program's schedule
//! charges. So 0022's arithmetic stands on a number it did not verify and this
//! now does.
//!
//! # The two accounts disagree
//!
//! The global account says 100 bps. The fee config says 125. Both are live, and
//! the trades captured from mainnet pass **both** the fee config and the fee
//! program, so the schedule is the binding one and the global field is the
//! fossil of a previous version.
//!
//! The disagreement is asserted rather than resolved. If pump.fun ever
//! reconciles the two, this fails and someone reads it, which is the only way a
//! silent change to a cost model gets noticed.

use radar_pumpfun::fees::{FeeConfig, Fees, global_fees};

const FIXTURE: &str = include_str!("fixtures/pumpfun_fees.json");

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

#[test]
fn the_fee_schedule_reads_as_one_hundred_and_twenty_five_basis_points() {
    let config = FeeConfig::parse(&account("fee_config")).expect("a fee config");
    // Every pump.fun launch starts far above a zero market cap, so the tier
    // that applies to a fresh token is the same one that applies to any of them.
    let fees = config
        .fees_at_market_cap(30_000_000_000)
        .expect("a tier covers a curve at launch reserves");
    assert_eq!(
        fees,
        Fees {
            lp_bps: 0,
            protocol_bps: 95,
            creator_bps: 30,
        }
    );
    assert_eq!(fees.total_bps(), 125, "one side of the round trip");
    assert_eq!(
        fees.round_trip_bps(),
        250,
        "and this is the figure research 0022 assumed"
    );
}

#[test]
fn there_are_no_liquidity_provider_fees_before_graduation() {
    // Not an incidental zero. There are no liquidity providers on a bonding
    // curve, and the field is non-zero on the AMM the curve graduates into --
    // so a cost model that carried the AMM's number backwards would overstate
    // the pre-graduation fee.
    let config = FeeConfig::parse(&account("fee_config")).expect("a fee config");
    for tier in &config.tiers {
        assert_eq!(tier.fees.lp_bps, 0, "no LP fee on the curve");
    }
}

#[test]
fn the_two_accounts_still_disagree_and_the_schedule_is_the_binding_one() {
    let global = global_fees(&account("global")).expect("a global account");
    let config = FeeConfig::parse(&account("fee_config")).expect("a fee config");
    let schedule = config.fees_at_market_cap(30_000_000_000).expect("a tier");

    assert_eq!(global.total_bps(), 100, "the global account's older figure");
    assert_eq!(schedule.total_bps(), 125, "what the fee program charges");
    assert_ne!(
        global.total_bps(),
        schedule.total_bps(),
        "if these are ever reconciled, read this test rather than deleting it: \
         Radar prices off the schedule, and a change to which one is authoritative \
         is a change to every cost estimate in the system"
    );
    // They agree on the protocol's share and differ only on the creator's, which
    // is what identifies the global field as the stale one rather than a
    // different fee altogether.
    assert_eq!(global.protocol_bps, schedule.protocol_bps);
    assert_ne!(global.creator_bps, schedule.creator_bps);
}

#[test]
fn a_market_cap_below_every_tier_has_no_fee_rather_than_a_free_trade() {
    // Rule 9. A schedule that does not cover a trade is a fee that could not be
    // read, and the convenient default -- zero -- is the one that makes an
    // unpriceable trade look free.
    let config = FeeConfig {
        flat: Fees::default(),
        tiers: vec![radar_pumpfun::fees::Tier {
            threshold_lamports: 1_000,
            fees: Fees {
                lp_bps: 0,
                protocol_bps: 95,
                creator_bps: 30,
            },
        }],
    };
    assert_eq!(config.fees_at_market_cap(999), None);
    assert!(config.fees_at_market_cap(1_000).is_some());
}

#[test]
fn the_highest_tier_reached_is_the_one_charged() {
    let config = FeeConfig {
        flat: Fees::default(),
        tiers: vec![
            radar_pumpfun::fees::Tier {
                threshold_lamports: 0,
                fees: Fees {
                    lp_bps: 0,
                    protocol_bps: 95,
                    creator_bps: 30,
                },
            },
            radar_pumpfun::fees::Tier {
                threshold_lamports: 1_000_000,
                fees: Fees {
                    lp_bps: 0,
                    protocol_bps: 20,
                    creator_bps: 5,
                },
            },
        ],
    };
    // Taking the first match rather than the highest would charge 125 bps here.
    assert_eq!(
        config
            .fees_at_market_cap(2_000_000)
            .expect("covered")
            .total_bps(),
        25
    );
    assert_eq!(
        config
            .fees_at_market_cap(999_999)
            .expect("covered")
            .total_bps(),
        125
    );
}

#[test]
fn the_fee_charged_rounds_up() {
    // The same direction as every rounding decision on the curve: against the
    // trader. A fee rounded down is a cost estimate rounded down, and 0019 names
    // that as the direction that launders a trade past the risk kernel.
    let fees = Fees {
        lp_bps: 0,
        protocol_bps: 95,
        creator_bps: 30,
    };
    // 125 bps of 1 lamport is 0.0125, which must not round to nothing.
    assert_eq!(fees.charge(1), 1);
    // 125 bps of 10_000 is exactly 125, so rounding up must not add one.
    assert_eq!(fees.charge(10_000), 125);
    assert_eq!(fees.charge(10_001), 126);
    assert_eq!(fees.charge(0), 0);
}

#[test]
fn a_fee_larger_than_the_trade_takes_the_trade_and_not_more() {
    // Not reachable from any schedule mainnet has published. It is asserted
    // because the alternative -- subtracting a fee larger than the proceeds --
    // is an underflow, and an underflow here is a wildly profitable exit.
    let absurd = Fees {
        lp_bps: 0,
        protocol_bps: 20_000,
        creator_bps: 0,
    };
    assert_eq!(absurd.charge(500), 500);
}

#[test]
fn an_account_of_the_wrong_kind_is_refused_rather_than_parsed() {
    // The two accounts here are 1045 and 4073 bytes of a program's state. Read
    // at the wrong offset, either produces a fee that looks entirely reasonable,
    // so the discriminator check is what stands between a plausible number and a
    // correct one.
    let global = account("global");
    assert!(FeeConfig::parse(&global).is_err(), "not a fee config");
    let config = account("fee_config");
    assert!(global_fees(&config).is_err(), "not a global account");
    assert!(FeeConfig::parse(&[]).is_err());
    assert!(global_fees(&[]).is_err());
}

#[test]
fn a_truncated_schedule_is_refused_rather_than_read_as_far_as_it_goes() {
    // A schedule missing its top rows prices a large trade at a small trade's
    // fee, which is the optimistic direction and the one that matters.
    let full = account("fee_config");
    assert!(FeeConfig::parse(&full).is_ok());
    // Cut inside the tier vector, not at an arbitrary fraction. The account is
    // 4,073 bytes and the schedule occupies about 110 of them, the rest being
    // reserved padding -- so halving the account leaves the schedule entirely
    // intact. The first version of this test cut at the halfway mark, asserted a
    // refusal, and was wrong about the code rather than finding a bug in it.
    //
    // 41 bytes of header, 24 of flat fees, 4 of vector length, then 8 bytes into
    // a row that needs 40.
    let truncated = &full[..41 + 24 + 4 + 8];
    assert!(
        FeeConfig::parse(truncated).is_err(),
        "a row that is cut in half is not a row"
    );
}

/// Builds a fee-config account with `tiers` rows, so the parser's *stride*
/// through the vector is exercised.
///
/// The mainnet capture has exactly one tier, which means the loop body runs
/// once and its final offset advance never affects the result. Mutation testing
/// found that: every offset arithmetic in `FeeConfig::parse` could be corrupted
/// without a single test noticing.
fn synthetic(tiers: &[(u128, u64, u64, u64)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&radar_pumpfun::fees::FEE_CONFIG_DISCRIMINATOR);
    out.push(255); // bump
    out.extend_from_slice(&[0u8; 32]); // admin
    for value in [7u64, 8, 9] {
        out.extend_from_slice(&value.to_le_bytes()); // flat fees
    }
    out.extend_from_slice(&u32::try_from(tiers.len()).expect("small").to_le_bytes());
    for (threshold, lp, protocol, creator) in tiers {
        out.extend_from_slice(&threshold.to_le_bytes());
        out.extend_from_slice(&lp.to_le_bytes());
        out.extend_from_slice(&protocol.to_le_bytes());
        out.extend_from_slice(&creator.to_le_bytes());
    }
    out
}

#[test]
fn every_tier_in_a_multi_row_schedule_is_read_at_the_right_offset() {
    // Three rows with distinct values, so a wrong stride produces a wrong fee
    // rather than an error. This is the shape the mainnet capture cannot test,
    // because a one-row vector's stride is unobservable.
    let bytes = synthetic(&[
        (0, 1, 2, 3),
        (1_000_000, 10, 20, 30),
        (5_000_000, 100, 200, 300),
    ]);
    let config = FeeConfig::parse(&bytes).expect("a fee config");
    assert_eq!(config.tiers.len(), 3);
    assert_eq!(config.flat.total_bps(), 24, "7 + 8 + 9");
    assert_eq!(config.tiers[0].threshold_lamports, 0);
    assert_eq!(config.tiers[0].fees.total_bps(), 6);
    assert_eq!(config.tiers[1].threshold_lamports, 1_000_000);
    assert_eq!(config.tiers[1].fees.total_bps(), 60);
    assert_eq!(config.tiers[2].threshold_lamports, 5_000_000);
    assert_eq!(config.tiers[2].fees.total_bps(), 600);
    // And the schedule selects across them.
    assert_eq!(
        config
            .fees_at_market_cap(4_999_999)
            .expect("covered")
            .total_bps(),
        60
    );
    assert_eq!(
        config
            .fees_at_market_cap(5_000_000)
            .expect("covered")
            .total_bps(),
        600
    );
}

#[test]
fn a_truncation_reports_how_many_bytes_the_layout_needed() {
    // The `needed` figure is the parser's own offset arithmetic, reported. If it
    // is never asserted, every `+` in that arithmetic can be corrupted silently
    // -- which is precisely what mutation testing found.
    use radar_pumpfun::curve::Malformed;
    let full = synthetic(&[(0, 1, 2, 3), (1_000_000, 10, 20, 30)]);
    // Cut one byte into the second tier's threshold. Header is 41, flat fees 24,
    // the length prefix 4, and the first row 40, so the second row starts at 109
    // and needs 16 bytes for its threshold.
    let cut = 109 + 1;
    match FeeConfig::parse(&full[..cut]) {
        Err(Malformed::TooShort { len, needed }) => {
            assert_eq!(len, cut);
            assert_eq!(needed, 109 + 16, "the threshold this row could not read");
        }
        other => panic!("expected a length refusal, got {other:?}"),
    }

    // And again, cut inside that row's *fees* rather than its threshold, so the
    // other offset in the loop body is reported too. The threshold occupies
    // 109..125; the fees need 24 more.
    let cut = 125 + 8;
    match FeeConfig::parse(&full[..cut]) {
        Err(Malformed::TooShort { len, needed }) => {
            assert_eq!(len, cut);
            assert_eq!(needed, 125 + 24, "the fees this row could not read");
        }
        other => panic!("expected a length refusal, got {other:?}"),
    }
}

#[test]
fn a_schedule_claiming_more_rows_than_it_carries_is_refused() {
    // The row count is four bytes read out of an account. A parser that trusted
    // it would allocate on it and read past the end; one that read as far as it
    // could would return a schedule missing its top rows, which prices a large
    // trade at a small trade's fee.
    let mut bytes = synthetic(&[(0, 1, 2, 3)]);
    bytes[41 + 24..41 + 24 + 4].copy_from_slice(&99u32.to_le_bytes());
    assert!(FeeConfig::parse(&bytes).is_err());
}

#[test]
fn a_cut_before_the_tier_vector_reports_its_own_offsets_too() {
    // The offsets *before* the loop: the flat fees and the row count. The
    // earlier truncation test only ever reached offsets inside the loop body,
    // so these two could be corrupted silently -- which is what a second round
    // of mutation testing found.
    use radar_pumpfun::curve::Malformed;
    let full = synthetic(&[(0, 1, 2, 3)]);

    // Header is 41 bytes; the flat fees need 24 more, so they end at 65.
    match FeeConfig::parse(&full[..41 + 8]) {
        Err(Malformed::TooShort { len, needed }) => {
            assert_eq!(len, 41 + 8);
            assert_eq!(needed, 65, "the flat fees the header could not reach");
        }
        other => panic!("expected a length refusal, got {other:?}"),
    }

    // Past the flat fees but inside the four-byte row count, which ends at 69.
    match FeeConfig::parse(&full[..67]) {
        Err(Malformed::TooShort { len, needed }) => {
            assert_eq!(len, 67);
            assert_eq!(needed, 69, "the row count that was cut in half");
        }
        other => panic!("expected a length refusal, got {other:?}"),
    }
}
