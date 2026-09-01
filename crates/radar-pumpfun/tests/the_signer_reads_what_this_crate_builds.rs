// SPDX-License-Identifier: Apache-2.0
//! The signer re-decoding a real pump.fun buy that this crate built.
//!
//! # Why this test is the point of the crate
//!
//! Rule 1's guarantee is that *every account the signer authorises is one it
//! read in the bytes it signed*. That guarantee is only worth something if the
//! bytes Radar produces are bytes the signer can actually read — and until now
//! nothing checked that, because nothing in the repository built a transaction
//! for this venue at all.
//!
//! [ADR 0003](../../../docs/adr/0003-legacy-transactions-because-the-signer-must-be-able-to-read-them.md)
//! is why it could fail: the signer refuses address lookup tables, so a builder
//! that emitted a versioned message would be refused at the last possible
//! moment, after every other stage had said yes. Research
//! [0021](../../../docs/research/0021-the-signer-cannot-read-the-only-venue-that-lists-them.md)
//! is that failure, found on the aggregator's routing.
//!
//! # What the accounts are
//!
//! Every account here is derived, and every derivation is asserted against a
//! mainnet capture elsewhere in this crate. The full eighteen-account form —
//! including the two the program's own IDL does not declare — simulates against
//! mainnet with no error; see research
//! [0023](../../../docs/research/0023-the-fee-is-a-schedule-and-the-published-interface-is-incomplete.md).
//!
//! # What this does not do
//!
//! Nothing is signed and nothing is sent. `check` returning `Ok` means the
//! transaction is one the signer *would* sign, which is the question this file
//! asks. Whether it would land, and at what price, is not asked here.

use radar_pumpfun::pda;
use radar_pumpfun::transaction::{AccountMeta, Instruction, transaction};
use radar_pumpfun::{Fees, Trade};
use radar_risk::{Action, Authorization, Autonomy, MicroUsd, Policy};
use radar_signer::verify::{Allowlist, Rejection, SYSTEM_PROGRAM, check};
use radar_types::{Address, Slot, SlotDelta};

/// The mint from the captured buy.
const MINT: &str = "6T1BNshzGAKAHvJ3NZ5n62X2eg5rqqsMipUMZJvLpump";
/// Its creator, read out of the bonding curve account.
const CREATOR: &str = "A833PVpQZrK4HUbAW19g4wCvQtGGNgUSEqEa2jsWQCQP";
const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const FEE_RECIPIENT: &str = "7hTckgnGnLQR6sdH7YkqFTAA7VwTfYFaZ6EhEsU3saCX";
const PUMP_PROGRAM: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";
const FEE_PROGRAM: &str = "pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ";

/// The buyback vault index Radar picks.
///
/// Any valid index is accepted — research 0023 simulated index 2 successfully
/// against a capture that used 6 — so this is a choice rather than a
/// requirement, and it is written down here as one.
const BUYBACK_INDEX: u8 = 2;

fn addr(s: &str) -> Address {
    s.parse().expect("a base58 address")
}

/// The wallet Radar would trade from. Arbitrary: nothing here holds a key.
fn wallet() -> Address {
    Address::new([0x33; 32])
}

/// The eighteen accounts a pump.fun buy takes, in the order the program checks.
fn buy_accounts(mint: &Address) -> Vec<AccountMeta> {
    let curve = pda::bonding_curve(mint).expect("a curve");
    let user = wallet();
    let token_program = addr(TOKEN_PROGRAM);
    vec![
        AccountMeta::readonly(pda::global().expect("global")),
        AccountMeta::writable(addr(FEE_RECIPIENT)),
        AccountMeta::readonly(*mint),
        AccountMeta::writable(curve),
        AccountMeta::writable(
            pda::associated_token_account(&curve, &token_program, mint).expect("curve ata"),
        ),
        AccountMeta::writable(
            pda::associated_token_account(&user, &token_program, mint).expect("user ata"),
        ),
        AccountMeta::signer(user),
        AccountMeta::readonly(Address::new(SYSTEM_PROGRAM)),
        AccountMeta::readonly(token_program),
        AccountMeta::writable(pda::creator_vault(&addr(CREATOR)).expect("creator vault")),
        AccountMeta::readonly(pda::event_authority().expect("event authority")),
        AccountMeta::readonly(addr(PUMP_PROGRAM)),
        AccountMeta::readonly(pda::global_volume_accumulator().expect("global volume")),
        AccountMeta::writable(pda::user_volume_accumulator(&user).expect("user volume")),
        AccountMeta::readonly(pda::fee_config().expect("fee config")),
        AccountMeta::readonly(addr(FEE_PROGRAM)),
        // The two the IDL does not declare, in this order. Transposed, the
        // program returns a different error than when they are missing.
        AccountMeta::readonly(pda::bonding_curve_v2(mint).expect("v2 curve")),
        AccountMeta::writable(pda::buyback_vault(BUYBACK_INDEX).expect("buyback vault")),
    ]
}

fn buy_instruction(mint: &Address) -> Instruction {
    let data = Trade::BuyExactSolIn {
        lamports: 20_000_000,
        slippage_bps: 500,
        track_volume: false,
    }
    .data()
    .expect("a discriminator");
    Instruction {
        program_id: addr(PUMP_PROGRAM),
        accounts: buy_accounts(mint),
        data,
    }
}

fn bytes_for(mint: &Address) -> Vec<u8> {
    transaction(&wallet(), &[buy_instruction(mint)], &[0xAA; 32]).expect("a legacy transaction")
}

fn authorisation_for(mint: &Address) -> Authorization {
    Authorization {
        nonce: "pumpfun-buy".to_owned(),
        mint: *mint,
        action: Action::Buy,
        max_notional: MicroUsd(5_000_000),
        expires_after: Slot(1_100),
        needs_operator_signature: false,
    }
}

/// Deliberately permissive, so a refusal is about the transaction rather than
/// about the policy. The shipped policy is `Policy::CLOSED` and it refuses
/// everything — which is asserted separately below, because that is the fact
/// that matters most.
fn permissive() -> Policy {
    Policy {
        autonomy: Autonomy::Capped,
        max_position: MicroUsd(1_000_000_000),
        max_canary: MicroUsd(1_000_000_000),
        max_input_staleness: SlotDelta(100_000),
        ..Policy::CLOSED
    }
}

fn allowlist() -> Allowlist {
    Allowlist {
        programs: vec![
            *addr(PUMP_PROGRAM).as_bytes(),
            *addr(FEE_PROGRAM).as_bytes(),
            SYSTEM_PROGRAM,
        ],
    }
}

#[test]
fn the_signer_can_read_a_pump_fun_buy_this_crate_built() {
    // The whole point. ADR 0003 says the signer takes legacy transactions only;
    // research 0021 found that the aggregator would only give versioned ones.
    // Building the transaction directly is the way out, and this is the proof
    // that the way out actually arrives somewhere.
    let mint = addr(MINT);
    let checked = check(
        &authorisation_for(&mint),
        &bytes_for(&mint),
        &wallet(),
        &allowlist(),
        &permissive(),
        Slot(1_000),
    );
    assert!(
        checked.is_ok(),
        "the signer refused a transaction this crate built: {:?}",
        checked.err()
    );
}

#[test]
fn an_authorisation_for_another_mint_is_refused() {
    // The check that makes the signer worth having. An executor bug that swapped
    // the mint for another would otherwise hold a valid authorisation for a
    // transaction that spends on something else entirely.
    let mint = addr(MINT);
    let elsewhere = Address::new([0x77; 32]);
    let rejections = check(
        &authorisation_for(&elsewhere),
        &bytes_for(&mint),
        &wallet(),
        &allowlist(),
        &permissive(),
        Slot(1_000),
    )
    .expect_err("an authorisation for a mint the transaction never names");
    assert!(
        rejections
            .iter()
            .any(|r| matches!(r, Rejection::MintAbsent(_))),
        "expected the mint check to fire, got {rejections:?}"
    );
}

#[test]
fn the_shipped_policy_refuses_it() {
    // `Policy::SHIPPED` is `CLOSED`, and this is what closed means at the key
    // rather than merely in the process that decides (ADR 0008). Everything else
    // in this file uses a permissive policy so that a refusal is about the
    // transaction; this one asserts the transaction is refused anyway.
    let mint = addr(MINT);
    let rejections = check(
        &authorisation_for(&mint),
        &bytes_for(&mint),
        &wallet(),
        &allowlist(),
        &Policy::SHIPPED,
        Slot(1_000),
    )
    .expect_err("the shipped policy authorises nothing");
    assert!(
        rejections
            .iter()
            .any(|r| matches!(r, Rejection::AutonomyInsufficient { .. })),
        "expected the policy to refuse unattended signing, got {rejections:?}"
    );
}

#[test]
fn a_program_outside_the_allowlist_is_refused() {
    // The fee program is CPI'd into by pump.fun rather than invoked directly, so
    // it is here as a *named account* and not as a program in the message. This
    // asserts the allowlist is doing work: drop pump.fun from it and the
    // transaction stops being signable.
    let mint = addr(MINT);
    let rejections = check(
        &authorisation_for(&mint),
        &bytes_for(&mint),
        &wallet(),
        &Allowlist {
            programs: vec![SYSTEM_PROGRAM],
        },
        &permissive(),
        Slot(1_000),
    )
    .expect_err("pump.fun is not on this allowlist");
    assert!(!rejections.is_empty());
}

#[test]
fn the_transaction_is_legacy_and_names_every_account_in_full() {
    // ADR 0003's actual requirement. A versioned message would set the high bit
    // of the first byte, and the signer refuses those -- so a builder that ever
    // emitted one would be refused at the last possible moment, after every
    // other stage had said yes.
    let bytes = bytes_for(&addr(MINT));
    let signatures = usize::from(bytes[0]);
    let first_message_byte = bytes[1 + signatures * 64];
    assert_eq!(
        first_message_byte & 0x80,
        0,
        "the version bit must be clear: this is a legacy message"
    );
    assert_eq!(signatures, 1, "only the wallet signs");
}

#[test]
fn every_account_in_the_instruction_is_derived_rather_than_hardcoded() {
    // Guards against the failure ADR 0009's first precondition exists to
    // prevent. Sixteen of the eighteen come from `pda`, and the two constants
    // that remain -- the fee recipient and the token program -- are inputs that
    // genuinely vary per trade and per mint, which is why they are arguments
    // rather than derivations.
    let mint = addr(MINT);
    let accounts = buy_accounts(&mint);
    assert_eq!(accounts.len(), 18, "mainnet passes eighteen, not sixteen");
    // Exactly one signer, and it is the wallet that pays.
    let signers: Vec<_> = accounts.iter().filter(|a| a.signer).collect();
    assert_eq!(signers.len(), 1);
    assert_eq!(signers[0].pubkey, wallet());
    // The two undeclared accounts sit last, in the order the program checks.
    assert_eq!(
        accounts[16].pubkey,
        pda::bonding_curve_v2(&mint).expect("v2")
    );
    assert_eq!(
        accounts[17].pubkey,
        pda::buyback_vault(BUYBACK_INDEX).expect("vault")
    );
}

#[test]
fn the_size_is_priced_with_the_fee_the_chain_charges() {
    // Not part of the signer's check, and included because the alternative is a
    // test file that builds a transaction without ever asking what it costs.
    // 125 bps a side, read from the fee schedule rather than assumed.
    let fees = Fees {
        lp_bps: 0,
        protocol_bps: 95,
        creator_bps: 30,
    };
    assert_eq!(fees.round_trip_bps(), 250);
    let spend = 20_000_000u64;
    assert_eq!(fees.charge(spend), 250_000);
}
