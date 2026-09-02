// SPDX-License-Identifier: Apache-2.0
//! Proving a customer owns the wallet they claim.
//!
//! [ADR 0011's amendment](https://github.com/hey-vera/radar/blob/main/docs/adr/0011-one-wallet-system-two-authority-levels-on-turnkey.md)
//! sequences bring-your-own-wallet first: the customer connects a wallet they
//! already have and signs each trade themselves. That removes the embedded
//! wallet, and with it the vendor that was also supplying **identity**.
//!
//! So identity becomes the wallet itself. A customer is a Solana address, and
//! the proof is a signature over a message this server issued — Sign-In With
//! Solana, the same shape as Sign-In With Ethereum.
//!
//! # Pure, and that is the point
//!
//! No clock and no network. `now` arrives as an argument, the challenge is
//! supplied rather than stored here, and everything is a function of its inputs
//! — so a refusal can be reproduced from a recording, like every other decision
//! in this crate.
//!
//! # The four things a signature alone does not prove
//!
//! A valid ed25519 signature proves someone holds a key. On its own it proves
//! nothing about *this* login, and each gap below is a real attack rather than
//! a formality:
//!
//! - **Which site.** Without domain binding, a signature a user made on any
//!   other Solana site can be replayed here. Wallets sign what they are asked to
//!   sign, and users approve message prompts routinely.
//! - **Which attempt.** Without a nonce, one captured signature logs an attacker
//!   in forever.
//! - **When.** Without an issue time and a bound on it, a nonce that leaks a
//!   year later is still good.
//! - **Who.** This is the subtle one. Verifying a signature against the
//!   *claimed* address proves the claimant holds that key — but if the message
//!   text names a different address, the server may bind the session to the name
//!   in the text rather than to the key that signed. The address is therefore
//!   checked to appear **in the signed text**, so the thing signed and the thing
//!   authenticated cannot come apart.
//!
//! Rule 8's shape throughout: every one of these is a refusal, and there is no
//! path that returns an identity without all four holding.

use radar_types::Address;

/// How long a challenge stays good, in seconds.
///
/// Five minutes: long enough to open a wallet and read a prompt, short enough
/// that a signature captured from a screen recording is stale before it is
/// useful. Not configurable, because the safe value does not vary by deployment
/// and a knob here is a knob to get wrong.
pub const MAX_AGE_SECONDS: u64 = 300;

/// Why a sign-in was refused.
///
/// Every variant is a refusal, and none of them yields an identity.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Refused {
    /// The signature does not verify against the claimed address.
    BadSignature,
    /// The signature was not 64 bytes, or the address is not a valid key.
    Malformed,
    /// The message was signed for a different site.
    ///
    /// The load-bearing one. A signature made on another Solana application is
    /// a perfectly valid signature; what makes it not a login *here* is that it
    /// does not name this domain.
    WrongDomain,
    /// The message does not carry the nonce this server issued.
    WrongNonce,
    /// The message does not name the address that signed it.
    ///
    /// See the module docs: this prevents the signed text and the authenticated
    /// identity coming apart.
    AddressNotInMessage,
    /// The challenge is older than [`MAX_AGE_SECONDS`], or issued in the future.
    Expired {
        /// How many seconds old the challenge was.
        age_seconds: i64,
    },
}

impl core::fmt::Display for Refused {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadSignature => write!(f, "the signature does not verify"),
            Self::Malformed => write!(f, "the signature or address is malformed"),
            Self::WrongDomain => write!(f, "the message was signed for another site"),
            Self::WrongNonce => write!(f, "the message does not carry this challenge"),
            Self::AddressNotInMessage => {
                write!(f, "the message does not name the address that signed it")
            }
            Self::Expired { age_seconds } => {
                write!(f, "the challenge is {age_seconds}s old")
            }
        }
    }
}

impl core::error::Error for Refused {}

/// What the server asked the customer to sign.
///
/// Held by the caller between issuing and verifying. This crate does not store
/// it: a nonce store is state with a clock, and both are things this crate does
/// not have.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Challenge {
    /// The site the customer is signing in to.
    pub domain: String,
    /// A single-use random value.
    ///
    /// The caller generates it and must not accept the same one twice — that
    /// half cannot live here, because remembering is state.
    pub nonce: String,
    /// When it was issued, in seconds since the epoch.
    pub issued_at: u64,
}

impl Challenge {
    /// The exact text the wallet is asked to sign.
    ///
    /// Rendered here rather than by the caller so the text that is *checked* and
    /// the text that is *shown* come from one function. Two renderings of the
    /// same message is one more thing that can drift, and the drift would look
    /// like a wallet bug.
    ///
    /// Plain language on purpose: a customer approving a wallet prompt should be
    /// able to read what they are agreeing to, and "authentication, not
    /// authority" is the thing worth saying.
    #[must_use]
    pub fn message(&self, address: &Address) -> String {
        format!(
            "{domain} wants you to sign in with your Solana account:\n\
             {address}\n\n\
             Signing proves you own this wallet. It authorises no transaction \
             and moves no funds.\n\n\
             Nonce: {nonce}\n\
             Issued At: {issued_at}",
            domain = self.domain,
            nonce = self.nonce,
            issued_at = self.issued_at,
        )
    }
}

/// What the customer sent back.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SignIn {
    /// The wallet they claim.
    pub address: Address,
    /// The exact text they signed.
    ///
    /// Sent back rather than reconstructed, so that what is verified is what the
    /// wallet actually saw. A server that rebuilt the message would verify a
    /// signature over text nobody displayed.
    pub message: String,
    /// The 64-byte ed25519 signature.
    pub signature: Vec<u8>,
}

/// Verifies a sign-in and returns the address it authenticates.
///
/// # Errors
///
/// [`Refused`] on any of the four checks in the module docs, or an unverifiable
/// signature. There is no success path that skips one.
pub fn verify(signin: &SignIn, challenge: &Challenge, now: u64) -> Result<Address, Refused> {
    // Cheap, decisive checks first -- they cost nothing and a signature
    // verification is the expensive part. The ordering is for cost, not for
    // safety: none of these is skippable.
    if !signin.message.contains(&challenge.domain) {
        return Err(Refused::WrongDomain);
    }
    if !signin.message.contains(&challenge.nonce) {
        return Err(Refused::WrongNonce);
    }
    if !signin.message.contains(&signin.address.to_string()) {
        return Err(Refused::AddressNotInMessage);
    }

    // Signed, not unsigned: a challenge issued in the *future* is as wrong as
    // one long past, and unsigned arithmetic would wrap it to something enormous
    // and then compare it against the ceiling as though it were ancient.
    let age = i64::try_from(now).unwrap_or(i64::MAX)
        - i64::try_from(challenge.issued_at).unwrap_or(i64::MAX);
    if age < 0 || age > i64::try_from(MAX_AGE_SECONDS).unwrap_or(i64::MAX) {
        return Err(Refused::Expired { age_seconds: age });
    }

    let signature: [u8; 64] = signin
        .signature
        .as_slice()
        .try_into()
        .map_err(|_| Refused::Malformed)?;
    let key = ed25519_dalek::VerifyingKey::from_bytes(signin.address.as_bytes())
        .map_err(|_| Refused::Malformed)?;
    key.verify_strict(
        signin.message.as_bytes(),
        &ed25519_dalek::Signature::from_bytes(&signature),
    )
    .map_err(|_| Refused::BadSignature)?;

    Ok(signin.address)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer as _, SigningKey};

    const DOMAIN: &str = "radar.heyvera.org";
    const NONCE: &str = "a1b2c3d4e5f6";
    const ISSUED: u64 = 1_788_000_000;

    fn wallet() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn address_of(key: &SigningKey) -> Address {
        Address::new(key.verifying_key().to_bytes())
    }

    fn challenge() -> Challenge {
        Challenge {
            domain: DOMAIN.to_owned(),
            nonce: NONCE.to_owned(),
            issued_at: ISSUED,
        }
    }

    fn signed_by(key: &SigningKey, message: &str) -> SignIn {
        SignIn {
            address: address_of(key),
            message: message.to_owned(),
            signature: key.sign(message.as_bytes()).to_bytes().to_vec(),
        }
    }

    fn good() -> SignIn {
        let key = wallet();
        signed_by(&key, &challenge().message(&address_of(&key)))
    }

    #[test]
    fn a_wallet_that_signs_the_challenge_is_authenticated() {
        let key = wallet();
        assert_eq!(
            verify(&good(), &challenge(), ISSUED + 10),
            Ok(address_of(&key))
        );
    }

    #[test]
    fn the_message_says_plainly_that_it_authorises_nothing() {
        // A customer approving a wallet prompt should be able to read what they
        // are agreeing to. "Authentication, not authority" is the rule this
        // whole lane rests on, and it is worth saying to the person signing.
        let key = wallet();
        let message = challenge().message(&address_of(&key));
        assert!(message.contains("authorises no transaction"));
        assert!(message.contains("moves no funds"));
        assert!(message.contains(DOMAIN));
        assert!(message.contains(NONCE));
        assert!(message.contains(&address_of(&key).to_string()));
    }

    #[test]
    fn a_signature_for_another_site_is_refused() {
        // The attack this exists to stop. A signature made on any other Solana
        // application is perfectly valid; what makes it not a login here is that
        // it does not name this domain. Users approve message prompts routinely.
        let key = wallet();
        let elsewhere = Challenge {
            domain: "not-radar.example".to_owned(),
            ..challenge()
        };
        let signin = signed_by(&key, &elsewhere.message(&address_of(&key)));
        assert_eq!(
            verify(&signin, &challenge(), ISSUED + 10),
            Err(Refused::WrongDomain)
        );
    }

    #[test]
    fn a_signature_over_a_different_challenge_is_refused() {
        // Without this one captured signature logs an attacker in forever.
        let key = wallet();
        let older = Challenge {
            nonce: "999999999999".to_owned(),
            ..challenge()
        };
        let signin = signed_by(&key, &older.message(&address_of(&key)));
        assert_eq!(
            verify(&signin, &challenge(), ISSUED + 10),
            Err(Refused::WrongNonce)
        );
    }

    #[test]
    fn a_message_naming_someone_elses_address_is_refused() {
        // The subtle one. The signature verifies -- this wallet really did sign
        // this text -- but the text names a different account. A server that
        // bound the session to the name in the text rather than to the key that
        // signed would hand the attacker the victim's session.
        let attacker = SigningKey::from_bytes(&[9u8; 32]);
        let victim = address_of(&wallet());
        // The attacker signs a message naming the victim, and claims their own
        // address -- so the signature is genuine and the text is a lie.
        let message = challenge().message(&victim);
        let signin = signed_by(&attacker, &message);
        assert_eq!(
            verify(&signin, &challenge(), ISSUED + 10),
            Err(Refused::AddressNotInMessage)
        );
    }

    #[test]
    fn a_signature_from_the_wrong_key_is_refused() {
        // The message is correct in every respect; only the signature is not
        // this address's.
        let key = wallet();
        let imposter = SigningKey::from_bytes(&[3u8; 32]);
        let message = challenge().message(&address_of(&key));
        let signin = SignIn {
            address: address_of(&key),
            message: message.clone(),
            signature: imposter.sign(message.as_bytes()).to_bytes().to_vec(),
        };
        assert_eq!(
            verify(&signin, &challenge(), ISSUED + 10),
            Err(Refused::BadSignature)
        );
    }

    #[test]
    fn a_stale_challenge_is_refused_and_so_is_one_from_the_future() {
        // Both directions. A challenge issued in the future is as wrong as one
        // long past -- and computing the age unsigned would wrap it to something
        // enormous, which then compares as ancient rather than as impossible.
        let signin = good();
        assert!(matches!(
            verify(&signin, &challenge(), ISSUED + MAX_AGE_SECONDS + 1),
            Err(Refused::Expired { .. })
        ));
        assert!(matches!(
            verify(&signin, &challenge(), ISSUED - 1),
            Err(Refused::Expired { age_seconds: -1 })
        ));
    }

    #[test]
    fn the_boundary_of_the_window_is_inclusive_on_both_sides() {
        // Swept rather than sampled. `>` and `>=` on the ceiling are a
        // one-second difference that no round-number test distinguishes.
        let signin = good();
        assert!(verify(&signin, &challenge(), ISSUED).is_ok(), "issued now");
        assert!(
            verify(&signin, &challenge(), ISSUED + MAX_AGE_SECONDS).is_ok(),
            "the last good second"
        );
        assert!(
            verify(&signin, &challenge(), ISSUED + MAX_AGE_SECONDS + 1).is_err(),
            "one second past"
        );
    }

    #[test]
    fn a_malformed_signature_is_refused_rather_than_panicking() {
        let key = wallet();
        let message = challenge().message(&address_of(&key));
        for length in [0usize, 63, 65, 128] {
            let signin = SignIn {
                address: address_of(&key),
                message: message.clone(),
                signature: vec![0u8; length],
            };
            assert_eq!(
                verify(&signin, &challenge(), ISSUED + 10),
                Err(Refused::Malformed),
                "a {length}-byte signature"
            );
        }
    }

    #[test]
    fn an_address_that_is_not_a_key_is_refused() {
        // Not every 32-byte value is a point on the curve, and an address that
        // is not one cannot have signed anything.
        let key = wallet();
        let message = challenge().message(&address_of(&key));
        let signin = SignIn {
            // All-ones is not a valid compressed ed25519 point.
            address: Address::new([0xFF; 32]),
            message,
            signature: vec![0u8; 64],
        };
        // It fails the message check first, which is correct and is the cheap
        // order -- so assert it is refused rather than which check fired.
        assert!(verify(&signin, &challenge(), ISSUED + 10).is_err());
    }

    #[test]
    fn every_refusal_says_which_check_failed() {
        // The only thing that tells an operator *why* a login was refused, and
        // the six reasons need different remedies: a wrong domain is a
        // misconfigured frontend, an expired challenge is a slow user, a bad
        // signature is a broken wallet or an attack. A `Display` that returned
        // the same text for all of them -- or nothing -- would make them one
        // undiagnosable failure.
        let cases = [
            (Refused::BadSignature, "does not verify"),
            (Refused::Malformed, "malformed"),
            (Refused::WrongDomain, "another site"),
            (Refused::WrongNonce, "does not carry this challenge"),
            (Refused::AddressNotInMessage, "does not name the address"),
            (Refused::Expired { age_seconds: 42 }, "42s old"),
        ];
        let mut seen = std::collections::BTreeSet::new();
        for (refusal, expected) in cases {
            let rendered = refusal.to_string();
            assert!(
                rendered.contains(expected),
                "{refusal:?} rendered as {rendered:?}"
            );
            assert!(
                seen.insert(rendered.clone()),
                "two refusals render identically: {rendered:?}"
            );
        }
        assert_eq!(seen.len(), 6, "one distinct message per reason");
    }
}
