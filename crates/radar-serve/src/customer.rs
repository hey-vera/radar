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

/// Whether this instance has a customer lane at all.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Mode {
    /// Verify customer tokens against this Privy application.
    Enforce(Config),
    /// No customer lane. Customer routes fall back to the operator check.
    ///
    /// This is the shipped state, and it is **not** a degradation: with no
    /// customer authenticator, a customer route requires operator identity —
    /// strictly more restrictive than it will be, never less. Rule 8's direction.
    Off,
}

impl Mode {
    /// Reads the mode from the environment.
    ///
    /// Unlike [`access::Mode::from_vars`](crate::access::Mode::from_vars), an
    /// absent configuration is **not** an error. Cloudflare Access has two wrong
    /// defaults and so must be chosen explicitly; here the absent case has one
    /// meaning and it is the safe one — no customer lane, operator only.
    ///
    /// # Errors
    ///
    /// Returns a message when the app id is set to something that is plainly an
    /// unsubstituted placeholder. That check exists because it already happened
    /// once with `RADAR_ACCESS_AUD`: the server started, enforced, and refused
    /// every real token, presenting as "nobody can log in" and sending an
    /// operator to the vendor's dashboard rather than to their own env file.
    pub fn from_vars(get: &impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let Some(app_id) = get("RADAR_PRIVY_APP_ID").filter(|v| !v.trim().is_empty()) else {
            return Ok(Self::Off);
        };
        if let Some(why) = crate::access::looks_unsubstituted(&app_id) {
            return Err(format!(
                "RADAR_PRIVY_APP_ID looks like an unsubstituted placeholder ({why}); \
                 the application id is on the Privy dashboard"
            ));
        }
        Ok(Self::Enforce(Config { app_id }))
    }

    /// The configuration, when there is a customer lane.
    #[must_use]
    pub const fn config(&self) -> Option<&Config> {
        match self {
            Self::Enforce(c) => Some(c),
            Self::Off => None,
        }
    }
}

/// The header a customer's token arrives in.
///
/// `Authorization: Bearer <token>`, which is what Privy's client sends and what
/// every HTTP client already knows how to set. Deliberately not the cookie the
/// operator lane also accepts: a cookie is sent by the browser automatically,
/// and a customer token that travels automatically is one that travels to places
/// nobody intended.
pub const BEARER_HEADER: &str = "authorization";

/// The customer's token from a request, if it carries one.
///
/// Neither trusted nor inspected here — it is handed to [`verify`], which is the
/// only thing that decides whether it means anything.
#[must_use]
pub fn token_from(headers: &axum::http::HeaderMap) -> Option<String> {
    let value = headers.get(BEARER_HEADER)?.to_str().ok()?;
    let token = value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))?;
    let token = token.trim();
    (!token.is_empty()).then(|| token.to_owned())
}

/// Privy's published keys, fetched on demand and cached.
///
/// Separate from the operator's cache rather than shared. They are different key
/// sets from different issuers on different rotation schedules, and one cache
/// holding both would let a fetch failure for one refuse tokens for the other.
#[derive(Debug, Default)]
pub struct KeyCache {
    inner: std::sync::RwLock<Option<(std::time::Instant, Keys)>>,
}

/// How long a fetched key set is reused.
///
/// The same hour the operator cache uses. Privy rotates, and a set held past a
/// rotation refuses every token signed with the new key — which looks like an
/// outage and is a stale cache.
const KEY_LIFETIME: std::time::Duration = std::time::Duration::from_secs(3_600);

/// Whether a key set fetched this long ago may still be used.
///
/// A pure function so the boundary can be swept, which it cannot be inside
/// `get` — the same shape [`access::is_fresh`](crate::access::is_fresh) uses and
/// for the same reason.
///
/// Strict, so a set exactly at its lifetime is refetched. The boundary arrives
/// once an hour on a busy instance and should fall towards freshness: a stale set
/// held one request too long refuses every token signed with a rotated key, which
/// looks like an outage.
#[must_use]
pub const fn is_fresh(age: std::time::Duration) -> bool {
    age.as_nanos() < KEY_LIFETIME.as_nanos()
}

impl KeyCache {
    /// An empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: std::sync::RwLock::new(None),
        }
    }

    /// A cache already holding a key set, so nothing is fetched.
    ///
    /// **This exists so the guard can be tested, and the test it exists for is
    /// worth the API.** The property that matters — that a valid customer token
    /// cannot reach an operator route — can only be observed by presenting a
    /// token that really verifies, and a verifier needs keys. Without this, the
    /// only reachable assertion is that an unauthenticated request is refused,
    /// which is the half that was never in doubt.
    ///
    /// Production uses [`Self::new`] and fetches.
    #[must_use]
    pub fn preloaded(keys: Keys) -> Self {
        Self {
            inner: std::sync::RwLock::new(Some((std::time::Instant::now(), keys))),
        }
    }

    /// The published keys, from cache or freshly fetched.
    ///
    /// # Errors
    ///
    /// Returns [`Denied::NoKeys`] when the set cannot be fetched or parsed. A
    /// refusal rather than a pass: a verifier that admits everyone when it cannot
    /// check is worse than no verifier, because it looks like one.
    pub fn get(&self, config: &Config) -> Result<Keys, Denied> {
        if let Ok(guard) = self.inner.read()
            && let Some((fetched, keys)) = guard.as_ref()
            && is_fresh(fetched.elapsed())
        {
            return Ok(keys.clone());
        }

        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(10)))
            .build()
            .into();
        let body = agent
            .get(&config.jwks_url())
            .call()
            .map_err(|e| Denied::NoKeys(e.to_string()))?
            .body_mut()
            .read_to_string()
            .map_err(|e| Denied::NoKeys(e.to_string()))?;

        let keys = Keys::parse(&body)?;
        if let Ok(mut guard) = self.inner.write() {
            *guard = Some((std::time::Instant::now(), keys.clone()));
        }
        Ok(keys)
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
    /// The tags, for a refusal that says what was actually presented.
    fn describe(&self) -> String {
        match self {
            Self::One(a) => a.clone(),
            Self::Many(all) => all.join(","),
        }
    }

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
        return Err(Denied::WrongAudience {
            presented: claims.aud.describe(),
            expected: config.app_id.clone(),
        });
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
        let refused = verify(&t, &keys, &config(), NOW).expect_err("refused");
        assert!(matches!(refused, Denied::WrongAudience { .. }));
        assert!(
            refused.to_string().contains("someone-elses-app"),
            "the refusal names the tag the token actually carried: {refused}"
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
        let refused = verify(&t, &keys, &config(), NOW).expect_err("refused");
        assert!(matches!(refused, Denied::WrongAudience { .. }));
        assert!(
            refused.to_string().contains("other,another"),
            "an array is reported whole, not just its first element: {refused}"
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
    fn an_absent_application_id_means_no_customer_lane_rather_than_an_error() {
        // Unlike Cloudflare Access, the absent case here has one meaning and it
        // is the safe one: no customer lane, so customer routes require operator
        // identity. Strictly more restrictive than it will be, never less.
        let none = |_: &str| None;
        assert_eq!(Mode::from_vars(&none), Ok(Mode::Off));

        let blank = |k: &str| (k == "RADAR_PRIVY_APP_ID").then(|| "   ".to_owned());
        assert_eq!(Mode::from_vars(&blank), Ok(Mode::Off));
    }

    #[test]
    fn an_application_id_switches_the_lane_on() {
        let set = |k: &str| (k == "RADAR_PRIVY_APP_ID").then(|| APP.to_owned());
        assert_eq!(
            Mode::from_vars(&set),
            Ok(Mode::Enforce(Config {
                app_id: APP.to_owned()
            }))
        );
    }

    #[test]
    fn a_pasted_placeholder_is_refused_rather_than_enforced() {
        // This exact failure already happened once, with `RADAR_ACCESS_AUD` on
        // 2026-08-30: the server started, enforced, and refused every real token,
        // presenting as "nobody can log in" and sending an operator to the
        // vendor's dashboard rather than to their own env file.
        for placeholder in [
            "<your privy app id>",
            "<app id from the dashboard>",
            "your app id here",
        ] {
            let get = |k: &str| (k == "RADAR_PRIVY_APP_ID").then(|| placeholder.to_owned());
            assert!(
                Mode::from_vars(&get).is_err(),
                "{placeholder} must be refused"
            );
        }
    }

    #[test]
    fn a_bearer_token_is_read_from_the_authorization_header_and_nowhere_else() {
        use axum::http::HeaderMap;

        let mut headers = HeaderMap::new();
        headers.insert(BEARER_HEADER, "Bearer abc.def.ghi".parse().unwrap());
        assert_eq!(token_from(&headers), Some("abc.def.ghi".to_owned()));

        // Lower case, because HTTP is case-insensitive about the scheme and a
        // client that sends it that way is not wrong.
        let mut headers = HeaderMap::new();
        headers.insert(BEARER_HEADER, "bearer abc.def.ghi".parse().unwrap());
        assert_eq!(token_from(&headers), Some("abc.def.ghi".to_owned()));

        // Not a cookie. A cookie travels automatically, and a customer token
        // that travels automatically travels somewhere nobody intended.
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::COOKIE,
            "privy-token=abc.def.ghi".parse().unwrap(),
        );
        assert_eq!(token_from(&headers), None);
    }

    #[test]
    fn a_header_without_the_bearer_scheme_carries_no_token() {
        use axum::http::HeaderMap;

        for value in ["abc.def.ghi", "Basic abc", "Bearer", "Bearer    "] {
            let mut headers = HeaderMap::new();
            headers.insert(BEARER_HEADER, value.parse().unwrap());
            assert_eq!(token_from(&headers), None, "{value}");
        }
    }

    #[test]
    fn a_key_set_is_used_until_its_lifetime_and_not_past_it() {
        use std::time::Duration;

        assert!(is_fresh(Duration::ZERO));
        assert!(is_fresh(
            KEY_LIFETIME.saturating_sub(Duration::from_nanos(1))
        ));
        // Strict at the boundary: exactly at its lifetime, a set is refetched.
        // Falling the other way holds a stale set one request too long, which
        // refuses every token signed with a rotated key and looks like an outage.
        assert!(!is_fresh(KEY_LIFETIME));
        assert!(!is_fresh(KEY_LIFETIME + Duration::from_nanos(1)));
        assert!(!is_fresh(Duration::from_secs(86_400)));
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
