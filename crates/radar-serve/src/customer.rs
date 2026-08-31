// SPDX-License-Identifier: Apache-2.0
//! Verifying a customer's Privy access token.
//!
//! # Why this is a second verifier and not a widened one
//!
//! [`access`](crate::access) already verifies JWTs, and reusing it was the
//! obvious move. It is the wrong one, and
//! [ADR 0005](../../../docs/adr/0005-customers-keep-custody-and-grant-radar-a-bounded-signer.md)
//! says why: **each authenticator pins exactly one algorithm.**
//!
//! Cloudflare Access signs with RS256. Privy signs with **ES256** — ECDSA over
//! P-256 — and its published keys are EC points rather than RSA moduli, so the
//! key parsing differs too. Widening `access::verify` to accept either would
//! mean a verifier that accepts a *set* of algorithms, and accepting a set is
//! precisely the confusion attack the refusal in that file exists to prevent.
//!
//! So the two live apart. Each is independently auditable, and neither can be
//! loosened by a change made for the other's benefit.
//!
//! # What a token here proves, and what it does not
//!
//! It proves that Privy issued this token, for this application, and that it has
//! not expired. That is **authentication and nothing else**.
//!
//! It is not authority. A verified customer may not soften a refusal, widen a
//! threshold, or authorise anything — `AGENTS.md` rule 1 applies to a customer's
//! capital exactly as it applies to Radar's, and the risk kernel remains the only
//! thing that turns a proposal into an `Authorization`.

use serde::Deserialize;
use serde_json::Value;

use crate::access::Denied;

/// Which Privy application this is.
///
/// The app id is not a secret — it ships in the client bundle — but it is the
/// audience every token is checked against, and getting it wrong means accepting
/// tokens issued for somebody else's application.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Config {
    /// The Privy application id.
    pub app_id: String,
}

/// The issuer every Privy access token names.
///
/// A bare string rather than a URL, which is unlike Cloudflare and worth
/// stating: comparing it against `https://privy.io` would refuse every valid
/// token, and building it by formatting would invite exactly that.
pub const ISSUER: &str = "privy.io";

impl Config {
    /// Where the signing keys are published.
    #[must_use]
    pub fn jwks_url(&self) -> String {
        format!(
            "https://auth.privy.io/api/v1/apps/{}/jwks.json",
            self.app_id
        )
    }
}

/// One published EC signing key.
#[derive(Clone, PartialEq, Eq, Debug, Deserialize)]
pub struct Jwk {
    /// The key id a token names in its header.
    pub kid: String,
    /// The curve. Only `P-256` is accepted.
    pub crv: String,
    /// The point's x coordinate, base64url.
    pub x: String,
    /// The point's y coordinate, base64url.
    pub y: String,
}

impl Jwk {
    /// The public key as an uncompressed SEC1 point: `0x04 || x || y`.
    ///
    /// `None` when the curve is not P-256 or either coordinate is not 32 bytes.
    /// Both are refusals rather than best-effort parses: a point assembled from
    /// the wrong number of bytes still verifies *something*, and what it
    /// verifies is not the token in front of it.
    #[must_use]
    pub fn point(&self) -> Option<Vec<u8>> {
        if self.crv != "P-256" {
            return None;
        }
        let x = radar_types::b64::decode_url(&self.x)?;
        let y = radar_types::b64::decode_url(&self.y)?;
        if x.len() != 32 || y.len() != 32 {
            return None;
        }
        let mut point = Vec::with_capacity(65);
        point.push(0x04);
        point.extend_from_slice(&x);
        point.extend_from_slice(&y);
        Some(point)
    }
}

/// The published key set.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Keys(pub Vec<Jwk>);

impl Keys {
    /// Parses a JWKS document.
    ///
    /// # Errors
    ///
    /// Returns [`Denied::NoKeys`] when the document is not a key set or holds no
    /// usable EC key. An empty set is an error rather than an empty success,
    /// because an empty set refuses every token with `UnknownKey` and sends an
    /// operator looking at the token instead of at the fetch.
    pub fn parse(document: &str) -> Result<Self, Denied> {
        let value: Value =
            serde_json::from_str(document).map_err(|e| Denied::NoKeys(format!("not JSON: {e}")))?;
        let keys = value
            .get("keys")
            .and_then(Value::as_array)
            .ok_or_else(|| Denied::NoKeys("no `keys` array".to_owned()))?;

        // Keys of other types are skipped rather than refused -- a set may
        // legitimately carry more than one kind -- but one with nothing usable
        // in it is an error.
        let parsed: Vec<Jwk> = keys
            .iter()
            .filter(|k| k.get("kty").and_then(Value::as_str) == Some("EC"))
            .filter_map(|k| serde_json::from_value(k.clone()).ok())
            .collect();

        if parsed.is_empty() {
            return Err(Denied::NoKeys("no usable EC keys".to_owned()));
        }
        Ok(Self(parsed))
    }

    /// The key with this id, if the set has one.
    #[must_use]
    pub fn by_id(&self, kid: &str) -> Option<&Jwk> {
        self.0.iter().find(|k| k.kid == kid)
    }
}

/// A verified customer.
///
/// Deliberately thin. Everything Radar does with a customer is keyed on the
/// identifier Privy issued; carrying more would be carrying claims nobody
/// checked.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Customer {
    /// Privy's identifier for the user, from `sub`.
    pub did: String,
    /// The session the token was issued for, from `sid`.
    pub session: String,
}

#[derive(Deserialize)]
struct Header {
    alg: String,
    kid: String,
}

#[derive(Deserialize)]
struct Claims {
    iss: String,
    /// A single string in Privy's tokens, though the JWT specification allows an
    /// array. Both are accepted because the specification does, and refusing the
    /// array form would break on a change nobody would think to look for.
    aud: Audience,
    sub: String,
    #[serde(default)]
    sid: String,
    exp: u64,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Audience {
    One(String),
    Many(Vec<String>),
}

impl Audience {
    fn contains(&self, expected: &str) -> bool {
        match self {
            Self::One(a) => a == expected,
            Self::Many(all) => all.iter().any(|a| a == expected),
        }
    }
}

/// Seconds of clock skew tolerated on expiry.
///
/// The same allowance the operator verifier makes, and for the same reason: two
/// machines disagreeing by a second should not log a customer out.
const SKEW_SECONDS: u64 = 60;

/// Verifies a Privy access token.
///
/// Pinned to ES256. A token naming any other algorithm is refused before its
/// signature is looked at — `none` has no signature to check, and naming a
/// symmetric algorithm invites checking one with the wrong primitive.
///
/// # Errors
///
/// Returns [`Denied`] describing the first check that failed.
pub fn verify(token: &str, keys: &Keys, config: &Config, now: u64) -> Result<Customer, Denied> {
    let mut parts = token.split('.');
    let (Some(header_b64), Some(payload_b64), Some(signature_b64), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(Denied::Malformed);
    };

    let header: Header =
        serde_json::from_slice(&radar_types::b64::decode_url(header_b64).ok_or(Denied::Malformed)?)
            .map_err(|_| Denied::Malformed)?;

    // Before the signature, and before anything else. See the module header.
    if header.alg != "ES256" {
        return Err(Denied::UnsupportedAlgorithm(header.alg));
    }

    let point = keys
        .by_id(&header.kid)
        .and_then(Jwk::point)
        .ok_or(Denied::UnknownKey)?;
    let signature = radar_types::b64::decode_url(signature_b64).ok_or(Denied::Malformed)?;

    // Over the bytes as they arrived. Re-serialising the parsed claims and
    // checking that would verify a document the sender never sent.
    //
    // `FIXED` rather than `ASN1`: a JWT's ES256 signature is the raw r||s pair,
    // sixty-four bytes, not a DER structure. Verifying with the wrong encoding
    // refuses every valid token, which looks exactly like a wrong key.
    let signed = format!("{header_b64}.{payload_b64}");
    ring::signature::UnparsedPublicKey::new(&ring::signature::ECDSA_P256_SHA256_FIXED, &point)
        .verify(signed.as_bytes(), &signature)
        .map_err(|_| Denied::BadSignature)?;

    let claims: Claims = serde_json::from_slice(
        &radar_types::b64::decode_url(payload_b64).ok_or(Denied::Malformed)?,
    )
    .map_err(|_| Denied::Malformed)?;

    // The audience check is the one most often skipped, and skipping it means a
    // token issued for any other Privy application opens Radar.
    if !claims.aud.contains(&config.app_id) {
        return Err(Denied::WrongAudience);
    }
    if claims.iss != ISSUER {
        return Err(Denied::WrongIssuer);
    }
    if claims.exp.saturating_add(SKEW_SECONDS) < now {
        return Err(Denied::Expired);
    }

    Ok(Customer {
        did: claims.sub,
        session: claims.sid,
    })
}

#[cfg(test)]
mod tests {
    use ring::rand::SystemRandom;
    use ring::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair};

    use super::*;

    const APP: &str = "cmthhkznr0a3u0cl86prxlb7x";
    const NOW: u64 = 1_800_000_000;

    /// Privy's real published key set, fetched on 2026-08-31.
    ///
    /// A production document rather than one written to suit the parser, for the
    /// reason LEARNINGS 12 gives: every test that used a hand-written row was
    /// blind to a row shape nobody who had not seen it would think to write.
    const REAL_JWKS: &str = r#"{"keys":[{"kty":"EC","x":"Qdk5ozZ223tuwpXJABpd-5Obnsdr-rpSd_01ebjRjP8","y":"RbH6PeD-Sg1CuGvInJkmadq7aT6qV816S5aIa-cdEIU","crv":"P-256","kid":"n1HfWGnXEjjndsHa1PwDXlVYF8oI52m4BRtOFUWSgTY","use":"sig","alg":"ES256"},{"kty":"EC","x":"LNfPYt5hAgxb9T5l_AVCp3KhwmzKd-WvNpXnwy1QLIE","y":"Nu05Uw7DtErtj8YshSziwJ68DcnSYo6ulTI-tt_mjpo","crv":"P-256","kid":"rLYOBN8dugt14QYmY9MxGTCx6sS3FDwwGZn4eW6Qiow","use":"sig","alg":"ES256"}]}"#;

    /// base64url, no padding — the encoding a JWT uses.
    ///
    /// Local to the tests rather than added beside `decode_url` in
    /// `radar-types`: nothing in production builds a token, and production API
    /// added for a test'''s convenience is API nobody audits.
    fn b64(bytes: &[u8]) -> String {
        radar_types::b64::encode(bytes)
            .replace('+', "-")
            .replace('/', "_")
            .trim_end_matches('=')
            .to_owned()
    }

    /// A key pair, and the key set that would publish it.
    fn keypair() -> (EcdsaKeyPair, Keys) {
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng)
            .expect("a key pair");
        let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng)
            .expect("parses");
        // The public key is `0x04 || x || y`; the JWKS carries the halves.
        let point = pair.public_key().as_ref().to_vec();
        let keys = Keys(vec![Jwk {
            kid: "test".to_owned(),
            crv: "P-256".to_owned(),
            x: b64(&point[1..33]),
            y: b64(&point[33..65]),
        }]);
        (pair, keys)
    }

    /// Signs a token with the given header and claims.
    fn token(pair: &EcdsaKeyPair, header: &str, claims: &str) -> String {
        let signing_input = format!("{}.{}", b64(header.as_bytes()), b64(claims.as_bytes()));
        let signature = pair
            .sign(&SystemRandom::new(), signing_input.as_bytes())
            .expect("signs");
        format!("{signing_input}.{}", b64(signature.as_ref()))
    }

    fn good_claims() -> String {
        format!(
            r#"{{"iss":"privy.io","aud":"{APP}","sub":"did:privy:abc","sid":"sess1","exp":{}}}"#,
            NOW + 3600
        )
    }

    fn config() -> Config {
        Config {
            app_id: APP.to_owned(),
        }
    }

    #[test]
    fn a_token_privy_signed_for_this_app_verifies() {
        let (pair, keys) = keypair();
        let t = token(&pair, r#"{"alg":"ES256","kid":"test"}"#, &good_claims());
        assert_eq!(
            verify(&t, &keys, &config(), NOW),
            Ok(Customer {
                did: "did:privy:abc".to_owned(),
                session: "sess1".to_owned(),
            })
        );
    }

    #[test]
    fn a_tampered_payload_does_not_verify() {
        // The point of a signature. The payload is plainly readable base64, so
        // changing it must kill the token.
        let (pair, keys) = keypair();
        let t = token(&pair, r#"{"alg":"ES256","kid":"test"}"#, &good_claims());
        let mut parts: Vec<&str> = t.split('.').collect();
        let forged = b64(
            format!(
                r#"{{"iss":"privy.io","aud":"{APP}","sub":"did:privy:SOMEONE_ELSE","sid":"s","exp":{}}}"#,
                NOW + 3600
            )
            .as_bytes(),
        );
        parts[1] = &forged;
        assert_eq!(
            verify(&parts.join("."), &keys, &config(), NOW),
            Err(Denied::BadSignature)
        );
    }

    #[test]
    fn rs256_is_refused_here_exactly_as_es256_is_refused_next_door() {
        // The symmetry ADR 0005 requires. This verifier must not accept the
        // operator's algorithm any more than the operator's accepts this one --
        // a verifier that accepts a set is the confusion attack both refusals
        // exist to prevent.
        let (pair, keys) = keypair();
        let t = token(&pair, r#"{"alg":"RS256","kid":"test"}"#, &good_claims());
        assert_eq!(
            verify(&t, &keys, &config(), NOW),
            Err(Denied::UnsupportedAlgorithm("RS256".to_owned()))
        );
    }

    #[test]
    fn alg_none_is_refused_before_the_signature_is_looked_at() {
        let (pair, keys) = keypair();
        let t = token(&pair, r#"{"alg":"none","kid":"test"}"#, &good_claims());
        assert_eq!(
            verify(&t, &keys, &config(), NOW),
            Err(Denied::UnsupportedAlgorithm("none".to_owned()))
        );
    }

    #[test]
    fn a_token_for_another_privy_application_is_refused() {
        // The check most often skipped. Without it, any token from any Privy
        // application in the world opens Radar -- with a signature that verifies
        // perfectly, because Privy really did sign it.
        let (pair, keys) = keypair();
        let claims = format!(
            r#"{{"iss":"privy.io","aud":"someone-elses-app","sub":"did:privy:x","sid":"s","exp":{}}}"#,
            NOW + 3600
        );
        let t = token(&pair, r#"{"alg":"ES256","kid":"test"}"#, &claims);
        assert_eq!(
            verify(&t, &keys, &config(), NOW),
            Err(Denied::WrongAudience)
        );
    }

    #[test]
    fn an_audience_array_is_accepted_because_the_specification_allows_one() {
        let (pair, keys) = keypair();
        let claims = format!(
            r#"{{"iss":"privy.io","aud":["other","{APP}"],"sub":"did:privy:x","sid":"s","exp":{}}}"#,
            NOW + 3600
        );
        let t = token(&pair, r#"{"alg":"ES256","kid":"test"}"#, &claims);
        assert!(verify(&t, &keys, &config(), NOW).is_ok());
    }

    #[test]
    fn an_audience_array_without_this_app_in_it_is_refused() {
        // The array branch needs a negative as well as a positive. With only the
        // positive, `any(|a| a == expected)` can become `!=` and still pass --
        // any array with one non-matching entry satisfies it, which is every
        // array an attacker would send.
        let (pair, keys) = keypair();
        let claims = format!(
            r#"{{"iss":"privy.io","aud":["other","another"],"sub":"did:privy:x","sid":"s","exp":{}}}"#,
            NOW + 3600
        );
        let t = token(&pair, r#"{"alg":"ES256","kid":"test"}"#, &claims);
        assert_eq!(
            verify(&t, &keys, &config(), NOW),
            Err(Denied::WrongAudience)
        );
    }

    #[test]
    fn the_skew_allowance_is_swept_at_its_exact_boundary() {
        // A token expiring exactly one skew-window ago is still accepted; one
        // second older is not. Testing either side without the boundary lets `<`
        // become `<=`, which shortens every session by a second -- invisible
        // until it is a customer reporting random logouts.
        let (pair, keys) = keypair();
        let at = |exp: u64| {
            let claims = format!(
                r#"{{"iss":"privy.io","aud":"{APP}","sub":"did:privy:x","sid":"s","exp":{exp}}}"#
            );
            token(&pair, r#"{"alg":"ES256","kid":"test"}"#, &claims)
        };

        // exp + SKEW == now: the last instant that is still inside.
        assert!(
            verify(&at(NOW - SKEW_SECONDS), &keys, &config(), NOW).is_ok(),
            "the boundary itself is inside the allowance"
        );
        assert_eq!(
            verify(&at(NOW - SKEW_SECONDS - 1), &keys, &config(), NOW),
            Err(Denied::Expired),
            "one second past it is not"
        );
    }

    #[test]
    fn a_different_issuer_is_refused_and_the_issuer_is_a_bare_string() {
        // `privy.io`, not `https://privy.io`. Comparing against the URL form
        // would refuse every valid token, and it is the kind of mistake that
        // looks like a key problem.
        assert_eq!(ISSUER, "privy.io");
        let (pair, keys) = keypair();
        let claims = format!(
            r#"{{"iss":"https://privy.io","aud":"{APP}","sub":"did:privy:x","sid":"s","exp":{}}}"#,
            NOW + 3600
        );
        let t = token(&pair, r#"{"alg":"ES256","kid":"test"}"#, &claims);
        assert_eq!(verify(&t, &keys, &config(), NOW), Err(Denied::WrongIssuer));
    }

    #[test]
    fn an_expired_token_is_refused_but_a_minute_of_skew_is_not() {
        let (pair, keys) = keypair();
        let expired = format!(
            r#"{{"iss":"privy.io","aud":"{APP}","sub":"did:privy:x","sid":"s","exp":{}}}"#,
            NOW - 3600
        );
        let t = token(&pair, r#"{"alg":"ES256","kid":"test"}"#, &expired);
        assert_eq!(verify(&t, &keys, &config(), NOW), Err(Denied::Expired));

        // Thirty seconds past expiry is inside the allowance.
        let just_lapsed = format!(
            r#"{{"iss":"privy.io","aud":"{APP}","sub":"did:privy:x","sid":"s","exp":{}}}"#,
            NOW - 30
        );
        let t = token(&pair, r#"{"alg":"ES256","kid":"test"}"#, &just_lapsed);
        assert!(verify(&t, &keys, &config(), NOW).is_ok());
    }

    #[test]
    fn a_key_this_set_does_not_publish_is_refused_rather_than_tried_against_the_others() {
        let (pair, keys) = keypair();
        let t = token(
            &pair,
            r#"{"alg":"ES256","kid":"some-other-key"}"#,
            &good_claims(),
        );
        assert_eq!(verify(&t, &keys, &config(), NOW), Err(Denied::UnknownKey));
    }

    #[test]
    fn the_real_published_key_set_parses() {
        // Production data, not a fixture written to suit the parser. Both of
        // Privy's keys must survive it, because a set that silently loses one
        // breaks every token signed with the other the next time they rotate.
        let keys = Keys::parse(REAL_JWKS).expect("the real set parses");
        assert_eq!(keys.0.len(), 2);
        assert!(
            keys.by_id("n1HfWGnXEjjndsHa1PwDXlVYF8oI52m4BRtOFUWSgTY")
                .is_some()
        );
        assert!(
            keys.by_id("rLYOBN8dugt14QYmY9MxGTCx6sS3FDwwGZn4eW6Qiow")
                .is_some()
        );
        for key in &keys.0 {
            assert_eq!(key.point().expect("a valid point").len(), 65);
            assert_eq!(key.point().expect("a valid point")[0], 0x04);
        }
    }

    #[test]
    fn a_key_set_with_nothing_usable_is_an_error_not_an_empty_set() {
        // An empty set refuses every token with `UnknownKey`, which sends an
        // operator to look at the token rather than at the fetch.
        assert!(matches!(
            Keys::parse(r#"{"keys":[{"kty":"RSA","kid":"k","n":"x","e":"AQAB"}]}"#),
            Err(Denied::NoKeys(_))
        ));
        assert!(matches!(Keys::parse("not json"), Err(Denied::NoKeys(_))));
        assert!(matches!(Keys::parse("{}"), Err(Denied::NoKeys(_))));
    }

    #[test]
    fn a_point_on_the_wrong_curve_or_the_wrong_length_is_refused() {
        // A point assembled from the wrong number of bytes still verifies
        // something, and what it verifies is not the token in front of it.
        let wrong_curve = Jwk {
            kid: "k".to_owned(),
            crv: "P-384".to_owned(),
            x: b64(&[1u8; 32]),
            y: b64(&[2u8; 32]),
        };
        assert!(wrong_curve.point().is_none());

        let short = Jwk {
            kid: "k".to_owned(),
            crv: "P-256".to_owned(),
            x: b64(&[1u8; 31]),
            y: b64(&[2u8; 32]),
        };
        assert!(short.point().is_none());
    }

    #[test]
    fn the_key_url_is_built_from_the_application_id() {
        assert_eq!(
            config().jwks_url(),
            format!("https://auth.privy.io/api/v1/apps/{APP}/jwks.json")
        );
    }

    #[test]
    fn anything_that_is_not_three_segments_is_malformed() {
        let (_, keys) = keypair();
        for bad in ["", "one", "one.two", "one.two.three.four", "...."] {
            assert_eq!(
                verify(bad, &keys, &config(), NOW),
                Err(Denied::Malformed),
                "{bad}"
            );
        }
    }
}
