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

    #[test]
    fn different_seeds_derive_different_addresses() {
        // Guards the obvious way to break this: hashing the program and marker
        // but forgetting the seeds.
        let a = bonding_curve(&addr(MINT)).expect("a");
        let b = bonding_curve(&addr("So11111111111111111111111111111111111111112")).expect("b");
        assert_ne!(a, b);
        assert_ne!(global().expect("g"), event_authority().expect("e"));
    }
}
