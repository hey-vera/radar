// SPDX-License-Identifier: Apache-2.0
//! Program-derived addresses, and the pump.fun accounts derived from them.
//!
//! # Why this is here rather than pulled in
//!
//! Solana's own crate would bring an enormous dependency tree into a workspace
//! whose stated bar is "the alternative is writing it ourselves and getting it
//! wrong". This is a SHA-256 and a curve check, both of which are already in the
//! tree, and the result is *verifiable against mainnet* — which is the part that
//! matters. Every derivation below is asserted against an address observed in a
//! real transaction, so "getting it wrong" is a thing the tests would catch
//! rather than a thing to worry about.
//!
//! # What a program-derived address is
//!
//! `sha256(seeds || bump || program || "ProgramDerivedAddress")`, taking the
//! first bump from 255 downward whose hash is **not** a point on the ed25519
//! curve. Being off the curve is what makes it unsignable: there is no private
//! key, so only the program can authorise it.
//!
//! The bump search direction is not a detail. Solana's canonical address is the
//! *first* one found counting down, and a derivation that counted up would
//! produce a different, equally valid-looking address that the program would not
//! recognise.

use sha2::{Digest, Sha256};

use radar_types::Address;

/// The pump.fun bonding-curve program.
///
/// Re-exported from the decoder rather than declared again — one address for one
/// program, per ADR 0009's single-table rule.
pub use radar_decode::pumpfun::PROGRAM_ID;

/// The marker Solana appends before hashing, which is what stops a PDA from
/// colliding with an ordinary hash of the same bytes.
const MARKER: &[u8] = b"ProgramDerivedAddress";

/// The associated-token-account program.
///
/// Every one of these constants is *checked* by a test that derives an address
/// with it and compares against mainnet, so a transcription error fails here
/// rather than on chain.
pub const ATA_PROGRAM: Address = Address::new([
    0x8c, 0x97, 0x25, 0x8f, 0x4e, 0x24, 0x89, 0xf1, 0xbb, 0x3d, 0x10, 0x29, 0x14, 0x8e, 0x0d, 0x83,
    0x0b, 0x5a, 0x13, 0x99, 0xda, 0xff, 0x10, 0x84, 0x04, 0x8e, 0x7b, 0xd8, 0xdb, 0xe9, 0xf8, 0x59,
]);

/// The fee program pump.fun's instructions carry.
pub const FEE_PROGRAM: Address = Address::new([
    0x0c, 0x35, 0xff, 0xa9, 0x05, 0x5a, 0x8e, 0x56, 0x8d, 0xa8, 0xf7, 0xbc, 0x07, 0x56, 0x15, 0x27,
    0x4c, 0xf1, 0xc9, 0x2c, 0xa4, 0x1f, 0x40, 0x00, 0x9c, 0x51, 0x6a, 0xa4, 0x14, 0xc2, 0x7c, 0x70,
]);

/// Whether a 32-byte value is a point on the ed25519 curve.
///
/// A PDA is precisely an address that is **not**, because that is what
/// guarantees no private key exists for it.
fn on_curve(bytes: &[u8; 32]) -> bool {
    ed25519_dalek::VerifyingKey::from_bytes(bytes).is_ok()
}

/// Derives a program address from seeds, with its bump.
///
/// `None` when every bump from 255 to 0 lands on the curve, which is
/// astronomically unlikely and is returned rather than panicked so that a caller
/// building a transaction gets a refusal instead of a crash.
#[must_use]
pub fn find(seeds: &[&[u8]], program: &Address) -> Option<(Address, u8)> {
    // Counting **down** from 255. Solana's canonical address is the first found
    // in this direction; counting up yields a different address that is equally
    // off-curve and that the program will not accept.
    for bump in (0..=u8::MAX).rev() {
        let mut hasher = Sha256::new();
        for seed in seeds {
            hasher.update(seed);
        }
        hasher.update([bump]);
        hasher.update(program.as_bytes());
        hasher.update(MARKER);
        let digest: [u8; 32] = hasher.finalize().into();
        if !on_curve(&digest) {
            return Some((Address::new(digest), bump));
        }
    }
    None
}

/// The bonding curve holding a mint's reserves.
///
/// Seed verified against mainnet: see the tests.
#[must_use]
pub fn bonding_curve(mint: &Address) -> Option<Address> {
    find(&[b"bonding-curve", mint.as_bytes()], &PROGRAM_ID).map(|(a, _)| a)
}

/// The mint's second-generation bonding curve.
///
/// **Required on every trade, and absent from the program's published IDL.**
/// Mainnet passes it as a remaining account, and the program validates its
/// address -- a wrong one is refused with `InvalidBondingCurveV2`, which is how
/// its name was learned. See
/// [research 0023](https://github.com/hey-vera/radar/blob/main/docs/research/0023-the-fee-is-a-schedule-and-the-published-interface-is-incomplete.md).
///
/// The account **does not exist** for the tokens Radar selects, and passing it
/// anyway is not optional: dropping it fails the instruction. An absent account
/// at a validated address is the program saying "this token has no v2 curve",
/// which is a fact rather than a gap.
#[must_use]
pub fn bonding_curve_v2(mint: &Address) -> Option<Address> {
    find(&[b"bonding-curve-v2", mint.as_bytes()], &PROGRAM_ID).map(|(a, _)| a)
}

/// One of the fee program's buyback vaults.
///
/// Also a remaining account, and also required. `index` selects among several
/// vaults; mainnet captures rotate through them, but **any valid index is
/// accepted** -- simulation confirms a trade built with index 2 succeeds where
/// the capture used 6. So the rotation spreads load rather than constraining a
/// caller, and Radar may pick one.
///
/// It is derived under the **fee** program, not pump.fun.
#[must_use]
pub fn buyback_vault(index: u8) -> Option<Address> {
    find(&[b"buyback-vault", &[index]], &FEE_PROGRAM).map(|(a, _)| a)
}

/// The program's global configuration.
#[must_use]
pub fn global() -> Option<Address> {
    find(&[b"global"], &PROGRAM_ID).map(|(a, _)| a)
}

/// The authority the program self-CPIs to when emitting an event.
///
/// Anchor's convention, and it appears in every instruction's account list
/// alongside the program itself.
#[must_use]
pub fn event_authority() -> Option<Address> {
    find(&[b"__event_authority"], &PROGRAM_ID).map(|(a, _)| a)
}

/// Where a creator's fees accrue.
#[must_use]
pub fn creator_vault(creator: &Address) -> Option<Address> {
    find(&[b"creator-vault", creator.as_bytes()], &PROGRAM_ID).map(|(a, _)| a)
}

/// The program-wide volume accumulator.
#[must_use]
pub fn global_volume_accumulator() -> Option<Address> {
    find(&[b"global_volume_accumulator"], &PROGRAM_ID).map(|(a, _)| a)
}

/// One trader's volume accumulator.
#[must_use]
pub fn user_volume_accumulator(user: &Address) -> Option<Address> {
    find(&[b"user_volume_accumulator", user.as_bytes()], &PROGRAM_ID).map(|(a, _)| a)
}

/// A holder's associated token account for a mint.
///
/// `["owner", token_program, mint]` under the ATA program — and the token
/// program is a **seed**, not a constant. Captures show both SPL Token and
/// Token-2022 on this venue, so deriving with the wrong one yields an address
/// the program will not accept.
#[must_use]
pub fn associated_token_account(
    owner: &Address,
    mint: &Address,
    token_program: &Address,
) -> Option<Address> {
    find(
        &[owner.as_bytes(), token_program.as_bytes(), mint.as_bytes()],
        &ATA_PROGRAM,
    )
    .map(|(a, _)| a)
}

/// The fee configuration, which lives under the **fee** program rather than
/// pump.fun's.
///
/// The seed is pump.fun's program id, which is the part that is easy to get
/// wrong: it is a PDA of one program keyed by another.
#[must_use]
pub fn fee_config() -> Option<Address> {
    find(&[b"fee_config", PROGRAM_ID.as_bytes()], &FEE_PROGRAM).map(|(a, _)| a)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Addresses observed in real mainnet transactions on 2026-09-01, captured
    /// alongside the account layouts in
    /// `radar-decode/tests/fixtures/pumpfun_accounts.json`.
    ///
    /// These are the whole point of this module's tests. A derivation checked
    /// only against itself proves the code is self-consistent; checked against a
    /// transaction the network accepted, it proves the seeds are right.
    const MINT: &str = "6T1BNshzGAKAHvJ3NZ5n62X2eg5rqqsMipUMZJvLpump";
    const OBSERVED_BONDING_CURVE: &str = "BfVg4yLn5WcffzTyXvWgqaupqmAeX4en2Pawq32TdZpb";
    const OBSERVED_GLOBAL: &str = "4wTV1YmiEkRvAtNtsSGPtUrqRYQMe5SKy2uB4Jjaxnjf";
    const OBSERVED_EVENT_AUTHORITY: &str = "Ce6TQqeHC9p8KetsN6JsjHK7UTZk7nasjjnr7XxXp9F1";

    /// The two accounts mainnet passes beyond the sixteen the IDL declares.
    /// Both are required; see `research/0023`.
    const OBSERVED_BONDING_CURVE_V2: &str = "BmvnaP7PdqZAigF9kPUggzWDCwZtiVvMT2BqQR2ajLm8";
    const OBSERVED_BUYBACK_VAULT_6: &str = "5eHhjP8JaYkz83CWwvGU2uMUXefd3AazWGx4gpcuEEYD";
    /// A second mint, so the v2 derivation is checked against more than one
    /// case -- a single match can be a coincidence of one address.
    const MINT_TWO: &str = "BVQg1SM8P8DwUyHrJGfY7oS5y4kRDLL9iGB6At5yAd3m";
    const OBSERVED_BONDING_CURVE_V2_TWO: &str = "6eHxRLJsrxP5GPxQttCbLnQJcAvy6MHNDxQoDKJyZXvJ";

    fn addr(s: &str) -> Address {
        s.parse().expect("a base58 address")
    }

    #[test]
    fn the_bonding_curve_seed_is_the_one_mainnet_uses() {
        // If this fails, every instruction this crate builds names the wrong
        // account for the reserves -- the account the trade actually moves.
        let derived = bonding_curve(&addr(MINT)).expect("a derivation");
        assert_eq!(derived, addr(OBSERVED_BONDING_CURVE));
    }

    #[test]
    fn the_global_and_event_authority_match_mainnet() {
        // Both appear in every captured instruction, so a wrong seed here breaks
        // every instruction rather than one.
        assert_eq!(global().expect("global"), addr(OBSERVED_GLOBAL));
        assert_eq!(
            event_authority().expect("event authority"),
            addr(OBSERVED_EVENT_AUTHORITY)
        );
    }

    #[test]
    fn a_derived_address_is_never_on_the_curve() {
        // The property that makes a PDA unsignable. If a derivation returned an
        // on-curve address, someone could hold its key.
        for mint in [MINT, "So11111111111111111111111111111111111111112"] {
            let derived = bonding_curve(&addr(mint)).expect("a derivation");
            assert!(
                !on_curve(derived.as_bytes()),
                "{mint} derived an on-curve address"
            );
        }
    }

    #[test]
    fn the_bump_counts_down_from_255() {
        // Solana's canonical address is the first found counting down. Counting
        // up would yield a different address that is equally off-curve and that
        // the program would reject -- a bug with no symptom until a transaction
        // fails on chain.
        let (_, bump) = find(&[b"global"], &PROGRAM_ID).expect("a derivation");
        assert_eq!(bump, 255, "global's canonical bump is the first tried");

        // And one that is not 255, so the test above is not passing because the
        // search stops immediately every time.
        let (_, mint_bump) =
            find(&[b"bonding-curve", addr(MINT).as_bytes()], &PROGRAM_ID).expect("a derivation");
        assert!(mint_bump < 255, "got {mint_bump}");
    }

    /// The rest of the buy layout, observed in the same transaction.
    const OBSERVED_ASSOCIATED_BONDING_CURVE: &str = "9TJmRk1oLNYzSWfLJZmUf7ZuUfHwdATs45xjA91kQioX";
    const OBSERVED_USER: &str = "5Kuix3HiXh7adsdcybmr5N5coLBi2mv7exGMLvoPSKjM";
    const OBSERVED_USER_ATA: &str = "4To2g3f93LVuZmDMVu6ow1nWKJ58wZxXsEfhyDXrBF3X";
    const OBSERVED_CREATOR: &str = "A833PVpQZrK4HUbAW19g4wCvQtGGNgUSEqEa2jsWQCQP";
    const OBSERVED_CREATOR_VAULT: &str = "CnJeYMf13M2AKZSkjMtKFsAB7zRPtYCbUdZvgzdbsDGe";
    const OBSERVED_GLOBAL_VOLUME: &str = "Hq2wp8uJ9jCPsYgNHex8RtqdvMPfVGoYwjvF1ATiwn2Y";
    const OBSERVED_USER_VOLUME: &str = "3315RuJqJ3uza7LYUdkcKQNzV6GfiTNot9jQg2YESQmj";
    const OBSERVED_FEE_CONFIG: &str = "8Wf5TiAheLUqBrKXeYg2JtAFFMWtKdG2BSFgqUcPVwTt";
    const SPL_TOKEN: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";

    #[test]
    fn both_token_accounts_derive_as_associated_token_accounts() {
        // The curve's own token account and the trader's. Both are ordinary
        // ATAs, which is worth pinning: they look like pump.fun PDAs in the
        // account list and are not.
        let spl = addr(SPL_TOKEN);
        assert_eq!(
            associated_token_account(&addr(OBSERVED_BONDING_CURVE), &addr(MINT), &spl)
                .expect("a derivation"),
            addr(OBSERVED_ASSOCIATED_BONDING_CURVE)
        );
        assert_eq!(
            associated_token_account(&addr(OBSERVED_USER), &addr(MINT), &spl)
                .expect("a derivation"),
            addr(OBSERVED_USER_ATA)
        );
    }

    #[test]
    fn the_token_program_is_a_seed_and_changes_the_address() {
        // Captures show both SPL Token and Token-2022 on this venue. Deriving
        // with the wrong one yields a valid-looking address the program will
        // not accept -- and the mistake is invisible until a transaction fails.
        let t22 = addr("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");
        let with_t22 = associated_token_account(&addr(OBSERVED_USER), &addr(MINT), &t22)
            .expect("a derivation");
        assert_ne!(with_t22, addr(OBSERVED_USER_ATA));
    }

    #[test]
    fn the_creator_vault_derives_from_the_creator_in_the_curve_account() {
        // The creator is not in the instruction's account list -- it is read out
        // of the bonding curve account. So building a buy needs the curve
        // fetched first, which is a real ordering constraint on the executor
        // rather than an implementation detail.
        assert_eq!(
            creator_vault(&addr(OBSERVED_CREATOR)).expect("a derivation"),
            addr(OBSERVED_CREATOR_VAULT)
        );
    }

    #[test]
    fn both_volume_accumulators_match_mainnet() {
        assert_eq!(
            global_volume_accumulator().expect("a derivation"),
            addr(OBSERVED_GLOBAL_VOLUME)
        );
        assert_eq!(
            user_volume_accumulator(&addr(OBSERVED_USER)).expect("a derivation"),
            addr(OBSERVED_USER_VOLUME)
        );
    }

    #[test]
    fn the_fee_config_is_a_pda_of_one_program_keyed_by_another() {
        // Under the *fee* program, seeded with pump.fun's id. Deriving it under
        // pump.fun -- the obvious guess -- gives a different address.
        assert_eq!(
            fee_config().expect("a derivation"),
            addr(OBSERVED_FEE_CONFIG)
        );

        let wrong = find(&[b"fee_config", PROGRAM_ID.as_bytes()], &PROGRAM_ID)
            .expect("a derivation")
            .0;
        assert_ne!(wrong, addr(OBSERVED_FEE_CONFIG));
    }

    #[test]
    fn the_program_constants_are_the_ones_mainnet_uses() {
        // Transcribed byte arrays, checked rather than trusted.
        assert_eq!(
            ATA_PROGRAM,
            addr("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL")
        );
        assert_eq!(
            FEE_PROGRAM,
            addr("pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ")
        );
    }

    #[test]
    fn different_seeds_derive_different_addresses() {
        // Guards the obvious way to break this: hashing the program and marker
        // but forgetting the seeds.
        let a = bonding_curve(&addr(MINT)).expect("a");
        let b = bonding_curve(&addr("So11111111111111111111111111111111111111112")).expect("b");
        assert_ne!(a, b);
        assert_ne!(global().expect("g"), event_authority().expect("e"));
    }

    #[test]
    fn the_second_generation_curve_derives_for_every_captured_mint() {
        // Learned from the runtime rather than from a reference: about six
        // hundred seed derivations failed to find this, and the program named it
        // in an error message the moment a wrong address was simulated.
        assert_eq!(
            bonding_curve_v2(&addr(MINT)).expect("a derivation"),
            addr(OBSERVED_BONDING_CURVE_V2)
        );
        assert_eq!(
            bonding_curve_v2(&addr(MINT_TWO)).expect("a derivation"),
            addr(OBSERVED_BONDING_CURVE_V2_TWO)
        );
    }

    #[test]
    fn the_second_generation_curve_is_not_the_first() {
        // Two accounts, one mint, both required on the same instruction and
        // neither substitutable for the other. Passing one where the other
        // belongs is refused on chain, so the difference is asserted here where
        // it costs nothing to find.
        let mint = addr(MINT);
        assert_ne!(
            bonding_curve(&mint).expect("v1"),
            bonding_curve_v2(&mint).expect("v2")
        );
    }

    #[test]
    fn the_buyback_vault_derives_under_the_fee_program() {
        // Under the *fee* program, not pump.fun. Deriving it under the program
        // that consumes it is the obvious mistake and produces a valid-looking
        // address that the instruction is then refused for.
        assert_eq!(
            buyback_vault(6).expect("a derivation"),
            addr(OBSERVED_BUYBACK_VAULT_6)
        );
        assert_ne!(
            buyback_vault(6).expect("six"),
            buyback_vault(2).expect("two"),
            "the index has to actually select"
        );
    }
}
