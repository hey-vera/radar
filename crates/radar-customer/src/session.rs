// SPDX-License-Identifier: Apache-2.0
//! The token a customer holds after proving they own their wallet.
//!
//! [`crate::siws`] proves ownership once. Asking a wallet to sign on every
//! request would be unusable, so that one proof is exchanged for a bearer token
//! carrying the address it authenticated and an expiry.
//!
//! # Authenticated, not encrypted, and the difference matters
//!
//! The payload is **readable by anyone holding the token** — it is a wallet
//! address and a timestamp, both of which the holder already knows. What the tag
//! provides is that the payload cannot be *changed*: swapping in another address
//! invalidates it.
//!
//! So this is not a place to put anything secret. It is a place to put a claim
//! the server is willing to stand behind, and nothing else has been put here.
//!
//! # Why not a JWT
//!
//! Radar already verifies two JWT families it does not issue, and both arrive
//! from an identity provider. This one is issued and consumed by the same
//! process, so the format is a private matter — and the parts of JWT that cause
//! trouble are exactly the parts a private format does not need. There is no
//! algorithm field, so there is no algorithm confusion, and no `none`.
//!
//! # Pure
//!
//! No clock and no ambient state: `now` is an argument, like everywhere else in
//! this crate. A session decision replays from a recording.

use radar_types::Address;

/// How long a session lasts, in seconds.
///
/// Twelve hours. Long enough not to be a nuisance, short enough that a token
/// taken from a laptop is not indefinite. It is not refreshed on use — a sliding
/// window means a stolen token never expires as long as it is being used, which
/// is precisely the case where expiry mattered.
pub const LIFETIME_SECONDS: u64 = 43_200;

/// The shortest acceptable signing secret, in bytes.
///
/// Same floor, and the same reasoning, as the meter's salt: a short secret is
/// brute-forceable, and a token forged against it authenticates as any wallet.
pub const MIN_SECRET_BYTES: usize = 32;

/// Why a session was refused.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Invalid {
    /// Not two base64url parts separated by a dot.
    Malformed,
    /// The tag does not match the payload.
    ///
    /// Either the token was altered or it was issued under a different secret.
    /// Deliberately one variant rather than two: distinguishing them tells a
    /// forger which half to keep working on.
    BadTag,
    /// The session has expired.
    Expired {
        /// How many seconds ago.
        by_seconds: u64,
    },
    /// The secret is shorter than [`MIN_SECRET_BYTES`].
    ///
    /// A refusal to *issue*, not a refusal of the customer. Rule 8: a component
    /// configured with something unusable says so rather than proceeding.
    SecretTooShort {
        /// How many bytes were supplied.
        got: usize,
    },
}

impl core::fmt::Display for Invalid {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Malformed => write!(f, "the session token is malformed"),
            Self::BadTag => write!(f, "the session token does not verify"),
            Self::Expired { by_seconds } => {
                write!(f, "the session expired {by_seconds}s ago")
            }
            Self::SecretTooShort { got } => write!(
                f,
                "the session secret is {got} bytes; {MIN_SECRET_BYTES} is the minimum"
            ),
        }
    }
}

impl core::error::Error for Invalid {}

/// A verified session.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Session {
    /// The wallet that proved ownership.
    pub address: Address,
    /// When the session stops being valid, in seconds since the epoch.
    pub expires_at: u64,
}

/// The payload, as bytes: 32 address bytes then 8 big-endian expiry bytes.
///
/// Fixed width rather than JSON, and that is a security property rather than a
/// preference. A variable-length encoding lets two different payloads produce
/// the same tagged bytes if a field boundary can be moved — the classic
/// concatenation ambiguity. Forty-eight fixed bytes have no boundary to move.
fn encode(session: &Session) -> [u8; 40] {
    let mut out = [0u8; 40];
    out[..32].copy_from_slice(session.address.as_bytes());
    out[32..].copy_from_slice(&session.expires_at.to_be_bytes());
    out
}

fn decode(bytes: &[u8]) -> Option<Session> {
    let address: [u8; 32] = bytes.get(..32)?.try_into().ok()?;
    let expires: [u8; 8] = bytes.get(32..40)?.try_into().ok()?;
    // Exactly forty bytes. Trailing bytes would be covered by the tag but not by
    // the reader, which is a difference between what was authenticated and what
    // was understood.
    if bytes.len() != 40 {
        return None;
    }
    Some(Session {
        address: Address::new(address),
        expires_at: u64::from_be_bytes(expires),
    })
}

fn tag(secret: &[u8], payload: &[u8]) -> [u8; 32] {
    // The secret is hashed to a key rather than truncated or padded into one, so
    // a secret of any length above the floor contributes all of its entropy.
    let key = *blake3::hash(secret).as_bytes();
    *blake3::keyed_hash(&key, payload).as_bytes()
}

/// Issues a token for `address`.
///
/// # Errors
///
/// [`Invalid::SecretTooShort`] when the secret is below [`MIN_SECRET_BYTES`].
pub fn issue(address: &Address, secret: &[u8], now: u64) -> Result<String, Invalid> {
    if secret.len() < MIN_SECRET_BYTES {
        return Err(Invalid::SecretTooShort { got: secret.len() });
    }
    let session = Session {
        address: *address,
        expires_at: now.saturating_add(LIFETIME_SECONDS),
    };
    let payload = encode(&session);
    let tag = tag(secret, &payload);
    Ok(format!(
        "{}.{}",
        radar_types::b64::encode_url(&payload),
        radar_types::b64::encode_url(&tag)
    ))
}

/// Verifies a token and returns the session it carries.
///
/// # Errors
///
/// [`Invalid`] when the token is malformed, does not verify, or has expired.
/// The tag is checked **before** the expiry, so an attacker cannot learn whether
/// a forged payload would have been in date.
pub fn verify(token: &str, secret: &[u8], now: u64) -> Result<Session, Invalid> {
    if secret.len() < MIN_SECRET_BYTES {
        return Err(Invalid::SecretTooShort { got: secret.len() });
    }
    let (payload_b64, tag_b64) = token.split_once('.').ok_or(Invalid::Malformed)?;
    let payload = radar_types::b64::decode_url(payload_b64).ok_or(Invalid::Malformed)?;
    let supplied = radar_types::b64::decode_url(tag_b64).ok_or(Invalid::Malformed)?;

    let expected = tag(secret, &payload);
    // Constant time. A byte-at-a-time comparison leaks how much of a forged tag
    // was right, which turns forgery from guessing 2^256 into guessing 32 bytes
    // one at a time.
    if !constant_time_eq(&expected, &supplied) {
        return Err(Invalid::BadTag);
    }

    // Only now is the payload trusted enough to read.
    let session = decode(&payload).ok_or(Invalid::Malformed)?;
    if now >= session.expires_at {
        return Err(Invalid::Expired {
            by_seconds: now - session.expires_at,
        });
    }
    Ok(session)
}

/// Compares two byte strings without a length-dependent early exit.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    // Folded rather than short-circuited: `all` and `==` on slices are both free
    // to return at the first difference.
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = &[7u8; 32];
    const NOW: u64 = 1_788_000_000;

    fn address() -> Address {
        Address::new([3u8; 32])
    }

    #[test]
    fn a_token_round_trips_to_the_address_it_was_issued_for() {
        let token = issue(&address(), SECRET, NOW).expect("issued");
        let session = verify(&token, SECRET, NOW + 60).expect("verifies");
        assert_eq!(session.address, address());
        assert_eq!(session.expires_at, NOW + LIFETIME_SECONDS);
    }

    #[test]
    fn a_token_issued_under_another_secret_does_not_verify() {
        // The property the whole file exists for. Without it anyone can mint a
        // session for any wallet.
        let token = issue(&address(), SECRET, NOW).expect("issued");
        assert_eq!(verify(&token, &[9u8; 32], NOW + 60), Err(Invalid::BadTag));
    }

    #[test]
    fn swapping_the_address_invalidates_the_token() {
        // The concrete attack: take your own valid token, change the wallet it
        // names, and become someone else. The payload is readable -- it is meant
        // to be -- so the tag is the only thing standing in the way.
        let token = issue(&address(), SECRET, NOW).expect("issued");
        let (_, tag_part) = token.split_once('.').expect("two parts");
        let forged_payload = encode(&Session {
            address: Address::new([0xEE; 32]),
            expires_at: NOW + LIFETIME_SECONDS,
        });
        let forged = format!(
            "{}.{tag_part}",
            radar_types::b64::encode_url(&forged_payload)
        );
        assert_eq!(verify(&forged, SECRET, NOW + 60), Err(Invalid::BadTag));
    }

    #[test]
    fn extending_the_expiry_invalidates_the_token() {
        // The other field, and the other obvious edit. A token whose expiry can
        // be pushed out is a token that never expires.
        let token = issue(&address(), SECRET, NOW).expect("issued");
        let (_, tag_part) = token.split_once('.').expect("two parts");
        let forged_payload = encode(&Session {
            address: address(),
            expires_at: NOW + 10_000_000,
        });
        let forged = format!(
            "{}.{tag_part}",
            radar_types::b64::encode_url(&forged_payload)
        );
        assert_eq!(verify(&forged, SECRET, NOW + 60), Err(Invalid::BadTag));
    }

    #[test]
    fn a_session_expires_and_the_boundary_is_swept() {
        let token = issue(&address(), SECRET, NOW).expect("issued");
        assert!(
            verify(&token, SECRET, NOW + LIFETIME_SECONDS - 1).is_ok(),
            "the last valid second"
        );
        assert_eq!(
            verify(&token, SECRET, NOW + LIFETIME_SECONDS),
            Err(Invalid::Expired { by_seconds: 0 }),
            "expiry is exclusive"
        );
        assert_eq!(
            verify(&token, SECRET, NOW + LIFETIME_SECONDS + 5),
            Err(Invalid::Expired { by_seconds: 5 })
        );
    }

    #[test]
    fn the_tag_is_checked_before_the_expiry() {
        // Ordering, and it is a real leak rather than a nicety: if expiry were
        // checked first, a forger would learn from the error message whether
        // their forged payload was in date -- an oracle over the payload they
        // control, answered without a valid tag.
        let expired_payload = encode(&Session {
            address: address(),
            expires_at: NOW - 1,
        });
        let forged = format!(
            "{}.{}",
            radar_types::b64::encode_url(&expired_payload),
            radar_types::b64::encode_url(&[0u8; 32])
        );
        assert_eq!(
            verify(&forged, SECRET, NOW),
            Err(Invalid::BadTag),
            "a bad tag must not be reported as an expiry"
        );
    }

    #[test]
    fn a_malformed_token_is_refused_rather_than_panicking() {
        for bad in [
            "",
            "no-dot",
            ".",
            "a.b",
            "!!!.???",
            // A valid tag over a payload of the wrong length.
            "AAAA.AAAA",
        ] {
            assert!(verify(bad, SECRET, NOW).is_err(), "{bad:?} must be refused");
        }
    }

    #[test]
    fn a_payload_with_trailing_bytes_is_refused_even_when_tagged_correctly() {
        // Everything under the tag is authentic, but forty-one bytes is not a
        // session -- and reading the first forty would authenticate something
        // different from what was signed. That gap is where padding attacks
        // live.
        let mut payload = encode(&Session {
            address: address(),
            expires_at: NOW + 1_000,
        })
        .to_vec();
        payload.push(0xAB);
        let token = format!(
            "{}.{}",
            radar_types::b64::encode_url(&payload),
            radar_types::b64::encode_url(&tag(SECRET, &payload))
        );
        assert_eq!(verify(&token, SECRET, NOW), Err(Invalid::Malformed));
    }

    #[test]
    fn a_short_secret_refuses_to_issue_or_verify() {
        // Rule 8. A component configured with something unusable says so.
        for length in [0usize, 1, MIN_SECRET_BYTES - 1] {
            let secret = vec![1u8; length];
            assert_eq!(
                issue(&address(), &secret, NOW),
                Err(Invalid::SecretTooShort { got: length })
            );
            assert_eq!(
                verify("a.b", &secret, NOW),
                Err(Invalid::SecretTooShort { got: length })
            );
        }
        assert!(issue(&address(), &[1u8; MIN_SECRET_BYTES], NOW).is_ok());
    }

    #[test]
    fn the_comparison_does_not_exit_early() {
        // Not a timing measurement -- that would be flaky. It asserts the
        // property that makes early exit impossible: every byte is folded in, so
        // a difference in the last position is caught exactly like one in the
        // first.
        let a = [0u8; 32];
        let mut first = a;
        first[0] = 1;
        let mut last = a;
        last[31] = 1;
        assert!(!constant_time_eq(&a, &first));
        assert!(!constant_time_eq(&a, &last));
        assert!(constant_time_eq(&a, &a));
        assert!(!constant_time_eq(&a, &[0u8; 31]));

        // Differences must **accumulate**, not cancel. Folding with `^` instead
        // of `|` reads identically and is a forgery: any two bytes that differ
        // by the same amount annihilate, so `00 00` would compare equal to
        // `01 01`. Mutation testing found this exact substitution, and the
        // single-byte cases above did not catch it.
        let mut two_differences = a;
        two_differences[0] = 1;
        two_differences[1] = 1;
        assert!(
            !constant_time_eq(&a, &two_differences),
            "two equal differences must not cancel"
        );
        let mut paired = [0u8; 4];
        paired[0] = 0xAA;
        paired[3] = 0xAA;
        assert!(!constant_time_eq(&[0u8; 4], &paired));
    }

    #[test]
    fn every_refusal_says_which_check_failed() {
        let cases = [
            (Invalid::Malformed, "malformed"),
            (Invalid::BadTag, "does not verify"),
            (Invalid::Expired { by_seconds: 9 }, "expired 9s ago"),
            (Invalid::SecretTooShort { got: 4 }, "4 bytes"),
        ];
        let mut seen = std::collections::BTreeSet::new();
        for (invalid, expected) in cases {
            let rendered = invalid.to_string();
            assert!(rendered.contains(expected), "{invalid:?} -> {rendered:?}");
            assert!(seen.insert(rendered), "two refusals render identically");
        }
    }
}
