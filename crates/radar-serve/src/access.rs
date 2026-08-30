// SPDX-License-Identifier: Apache-2.0
//! Who is allowed to look.
//!
//! Radar's interface shows what it decided about which tokens, what it refused,
//! and what it spent. That is operational detail about a private system, and
//! until the public phase exists it should be readable by one person.
//!
//! # The header is not the check
//!
//! Cloudflare Access puts an identity in `Cf-Access-Authenticated-User-Email`,
//! and the tempting implementation reads it. That header is a *claim by whoever
//! sent the request*: anything that reaches the origin can set it, and "the
//! origin is behind a tunnel and hard to reach" is a network topology, not an
//! authentication model. Tunnels are misconfigured, origins get a second
//! ingress for a debugging session that is never removed, and the failure is
//! silent in the direction of admitting everyone.
//!
//! So Radar verifies the **signature** on the `Cf-Access-Jwt-Assertion` token
//! against Cloudflare's published keys, and takes the identity from the verified
//! payload. The header is ignored entirely.
//!
//! # What is checked, and why each one
//!
//! - **Algorithm.** Only `RS256`. `alg: none` and an `HS256` token whose "secret"
//!   is the RSA public key are the two classic forgeries, and both are refused
//!   by reading the header before touching the signature.
//! - **Key.** By `kid`, against the fetched JWKS. An unknown `kid` is a refusal,
//!   never a fallback to "try them all with the first key".
//! - **Signature**, over the exact `header.payload` bytes as received rather
//!   than over a re-serialisation of the parsed claims.
//! - **Audience.** Must contain this application's AUD tag. Without it any token
//!   from any application in the same Cloudflare team unlocks Radar, which is
//!   the check people most often skip.
//! - **Issuer.** Must be this team's domain.
//! - **Expiry**, with a small skew allowance in the direction of refusing.
//!
//! # Rule 8, arranged so it cannot be tripped over
//!
//! An unset configuration does **not** mean "serve to everyone", and it does not
//! silently mean "refuse everyone" either — a server that refuses everything
//! after an install looks identical to a broken deploy. Both readings are bad,
//! so neither is a default: [`Mode::from_vars`] *fails* unless the operator has
//! either configured Access or written `RADAR_ACCESS=off` in as many words. The
//! binary then refuses to start, saying which two things it will accept.
//!
//! That is deliberately louder than the alternatives. A security control whose
//! absence is silent is the one that is absent.

use core::time::Duration;
use std::sync::RwLock;

use serde::Deserialize;
use serde_json::Value;

/// How long a fetched key set is trusted before it is fetched again.
///
/// Cloudflare rotates these; a set cached forever is a set that stops verifying
/// real tokens after a rotation, which presents as "nobody can log in" some days
/// after the change that caused it.
pub const KEY_LIFETIME: Duration = Duration::from_secs(3_600);

/// How much clock skew is forgiven on `exp`.
///
/// Small, and one-sided: a token accepted a minute after it expired is a minute
/// of exposure, and a token refused a minute early is a page reload.
pub const SKEW_SECONDS: u64 = 60;

/// The header Cloudflare puts the signed assertion in.
pub const ASSERTION_HEADER: &str = "cf-access-jwt-assertion";

/// Where the identity is taken from.
///
/// Named as a constant so the one thing this module must *not* do is greppable:
/// nothing reads it, and a future reader looking for why will find the module
/// comment above.
pub const UNTRUSTED_EMAIL_HEADER: &str = "cf-access-authenticated-user-email";

/// Which Cloudflare Access application this is.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Config {
    /// The team domain, e.g. `heyvera.cloudflareaccess.com`.
    pub team_domain: String,
    /// The application's AUD tag.
    pub aud: String,
}

impl Config {
    /// The issuer a valid token must name.
    #[must_use]
    pub fn issuer(&self) -> String {
        format!("https://{}", self.team_domain)
    }

    /// Where the signing keys are published.
    #[must_use]
    pub fn jwks_url(&self) -> String {
        format!("https://{}/cdn-cgi/access/certs", self.team_domain)
    }
}

/// Whether this instance checks identity.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Mode {
    /// Verify every request against Cloudflare Access.
    Enforce(Config),
    /// Serve without checking, because the operator said so explicitly.
    Off,
}

impl Mode {
    /// Reads the mode, refusing to guess.
    ///
    /// # Errors
    ///
    /// Returns a message naming both acceptable configurations when neither is
    /// present. This is the whole design of the function: there is no default,
    /// because both possible defaults are wrong in a way that is invisible.
    pub fn from_vars(get: &impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let explicit_off =
            get("RADAR_ACCESS").is_some_and(|v| v.trim().eq_ignore_ascii_case("off"));
        let team = get("RADAR_ACCESS_TEAM").filter(|v| !v.trim().is_empty());
        let aud = get("RADAR_ACCESS_AUD").filter(|v| !v.trim().is_empty());

        // Before anything else: a value that is obviously an unsubstituted
        // placeholder. Set to one, the server starts, enforces, fetches
        // Cloudflare's keys and then refuses every real token -- because no
        // token's audience contains that text. The site locks out completely.
        //
        // Fail-closed, which is the right direction, and useless as a
        // diagnosis: it presents as "nobody can log in", which sends an
        // operator to Cloudflare's dashboard rather than to their own env file.
        // This happened on 2026-08-30, from a runbook whose command carried the
        // placeholder inside a heredoc.
        for (name, value) in [("RADAR_ACCESS_TEAM", &team), ("RADAR_ACCESS_AUD", &aud)] {
            if let Some(value) = value.as_ref()
                && let Some(why) = looks_unsubstituted(value)
            {
                return Err(format!(
                    "{name} looks like an unsubstituted placeholder ({why});                      the audience tag is on the Access application's Overview page"
                ));
            }
        }

        match (explicit_off, team, aud) {
            // Configured *and* switched off is a contradiction, and resolving it
            // either way silently is how an instance ends up open with an Access
            // application sitting in front of it that nobody checks.
            (true, Some(_), _) | (true, _, Some(_)) => {
                Err("RADAR_ACCESS=off is set alongside RADAR_ACCESS_TEAM/AUD; pick one".to_owned())
            }
            (true, None, None) => Ok(Self::Off),
            (false, Some(team_domain), Some(aud)) => Ok(Self::Enforce(Config {
                team_domain: team_domain.trim().to_owned(),
                aud: aud.trim().to_owned(),
            })),
            (false, team, aud) => {
                let missing = match (team.is_none(), aud.is_none()) {
                    (true, true) => "neither is set",
                    (true, false) => "RADAR_ACCESS_TEAM is missing",
                    (false, true) => "RADAR_ACCESS_AUD is missing",
                    (false, false) => unreachable!("both present is handled above"),
                };
                Err(format!(
                    "refusing to start without deciding who may look: set \
                     RADAR_ACCESS_TEAM and RADAR_ACCESS_AUD to verify Cloudflare Access \
                     tokens, or RADAR_ACCESS=off to serve without checking ({missing})"
                ))
            }
        }
    }
}

/// Whether a configured value is obviously a placeholder rather than a value.
///
/// Deliberately narrow. It looks for the two things that appear in prose and
/// never in a credential — whitespace and angle brackets — and **does not pin
/// the format** of a Cloudflare audience tag, which is a vendor's to change. A
/// regex for "64 hex characters" would reject a valid tag the day that changes,
/// and would do it at startup on a server that had been working.
///
/// The failure it is for is a heredoc pasted with its own placeholder still in
/// it, which is a mistake a careful person makes at three in the morning and
/// which no amount of care in the runbook prevents.
fn looks_unsubstituted(value: &str) -> Option<&'static str> {
    let trimmed = value.trim();
    if trimmed.contains(char::is_whitespace) {
        return Some("it contains spaces");
    }
    if trimmed.contains('<') || trimmed.contains('>') {
        return Some("it contains angle brackets");
    }
    None
}

/// Why a request was not admitted.
///
/// Several variants because they mean different things to an operator, and one
/// of them — [`Denied::UnknownKey`] — is the signal that a key rotation happened
/// and the cache is stale.
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum Denied {
    /// No token was presented.
    #[error("no Cloudflare Access assertion")]
    Missing,
    /// The token is not three base64url segments.
    #[error("the assertion is malformed")]
    Malformed,
    /// The header names an algorithm this verifier will not accept.
    ///
    /// The two forgeries worth naming: `none`, and `HS256` signed with the RSA
    /// public key as the shared secret.
    #[error("unsupported algorithm `{0}`")]
    UnsupportedAlgorithm(String),
    /// No published key matches the token's `kid`.
    #[error("no published key with that id")]
    UnknownKey,
    /// The signature does not verify.
    #[error("the signature does not verify")]
    BadSignature,
    /// The token is out of date.
    #[error("the assertion expired")]
    Expired,
    /// The token was issued for a different application.
    #[error("the assertion is for a different application")]
    WrongAudience,
    /// The token was issued by a different team.
    #[error("the assertion is from a different issuer")]
    WrongIssuer,
    /// The keys could not be fetched, so nothing can be verified.
    ///
    /// A refusal rather than a pass. A verifier that admits everyone when it
    /// cannot check is worse than no verifier, because it looks like one.
    #[error("the signing keys could not be fetched: {0}")]
    NoKeys(String),
}

/// Who the verified token says is asking.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Identity {
    /// The address Cloudflare authenticated.
    pub email: String,
    /// Cloudflare's stable identifier, which survives an address change.
    pub subject: String,
}

/// One published signing key.
#[derive(Clone, PartialEq, Eq, Debug, Deserialize)]
pub struct Jwk {
    /// The key id a token names in its header.
    pub kid: String,
    /// RSA modulus, base64url.
    pub n: String,
    /// RSA exponent, base64url.
    pub e: String,
}

/// The published key set.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Keys(pub Vec<Jwk>);

impl Keys {
    /// Parses a JWKS document.
    ///
    /// # Errors
    ///
    /// Returns [`Denied::NoKeys`] when the document is not a key set. Keys of
    /// other types are skipped rather than refused — Cloudflare has published
    /// mixed sets before — but a set with no usable RSA key is an error, not an
    /// empty set that would refuse every token with `UnknownKey` and send an
    /// operator looking at the wrong thing.
    pub fn parse(document: &str) -> Result<Self, Denied> {
        let value: Value =
            serde_json::from_str(document).map_err(|e| Denied::NoKeys(format!("not JSON: {e}")))?;
        let keys = value
            .get("keys")
            .and_then(Value::as_array)
            .ok_or_else(|| Denied::NoKeys("no `keys` array".to_owned()))?;

        let parsed: Vec<Jwk> = keys
            .iter()
            .filter(|k| k.get("kty").and_then(Value::as_str) == Some("RSA"))
            .filter_map(|k| serde_json::from_value(k.clone()).ok())
            .collect();

        if parsed.is_empty() {
            return Err(Denied::NoKeys("no usable RSA keys".to_owned()));
        }
        Ok(Self(parsed))
    }

    /// The key with this id.
    #[must_use]
    pub fn by_id(&self, kid: &str) -> Option<&Jwk> {
        self.0.iter().find(|k| k.kid == kid)
    }
}

/// What a token's header says.
#[derive(Clone, PartialEq, Eq, Debug, Deserialize)]
struct Header {
    alg: String,
    #[serde(default)]
    kid: String,
}

/// What a token's payload says.
#[derive(Clone, PartialEq, Eq, Debug, Deserialize)]
struct Claims {
    #[serde(default)]
    aud: Vec<String>,
    #[serde(default)]
    iss: String,
    #[serde(default)]
    exp: u64,
    #[serde(default)]
    email: String,
    #[serde(default)]
    sub: String,
}

/// Verifies a token and returns who it is for.
///
/// Pure: the keys and the current time are arguments. That is what makes every
/// refusal here reproducible from a fixture rather than from a live Cloudflare
/// account, and it is why the forgery cases below can be real signed tokens
/// instead of hand-waving.
///
/// # Errors
///
/// Returns [`Denied`] naming which check failed.
pub fn verify(token: &str, keys: &Keys, config: &Config, now: u64) -> Result<Identity, Denied> {
    let mut parts = token.split('.');
    let (Some(header_b64), Some(payload_b64), Some(signature_b64), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(Denied::Malformed);
    };

    let header: Header =
        serde_json::from_slice(&radar_types::b64::decode_url(header_b64).ok_or(Denied::Malformed)?)
            .map_err(|_| Denied::Malformed)?;

    // Before the signature, and that ordering is the point. `alg: none` has no
    // signature to check and `HS256` invites checking one with the wrong
    // primitive; both are settled by refusing to proceed at all.
    if header.alg != "RS256" {
        return Err(Denied::UnsupportedAlgorithm(header.alg));
    }

    let key = keys.by_id(&header.kid).ok_or(Denied::UnknownKey)?;
    let modulus = radar_types::b64::decode_url(&key.n).ok_or(Denied::UnknownKey)?;
    let exponent = radar_types::b64::decode_url(&key.e).ok_or(Denied::UnknownKey)?;
    let signature = radar_types::b64::decode_url(signature_b64).ok_or(Denied::Malformed)?;

    // Over the bytes as they arrived. Re-serialising the parsed claims and
    // signing that would verify a document the sender never sent, which is how
    // a verifier ends up checking one thing and trusting another.
    let signed = format!("{header_b64}.{payload_b64}");
    ring::signature::RsaPublicKeyComponents {
        n: &modulus,
        e: &exponent,
    }
    .verify(
        &ring::signature::RSA_PKCS1_2048_8192_SHA256,
        signed.as_bytes(),
        &signature,
    )
    .map_err(|_| Denied::BadSignature)?;

    let claims: Claims = serde_json::from_slice(
        &radar_types::b64::decode_url(payload_b64).ok_or(Denied::Malformed)?,
    )
    .map_err(|_| Denied::Malformed)?;

    // The audience check is the one most often skipped, and skipping it means
    // any token from any application in the same Cloudflare team opens Radar.
    if !claims.aud.iter().any(|a| a == &config.aud) {
        return Err(Denied::WrongAudience);
    }
    if claims.iss != config.issuer() {
        return Err(Denied::WrongIssuer);
    }
    if claims.exp.saturating_add(SKEW_SECONDS) < now {
        return Err(Denied::Expired);
    }

    Ok(Identity {
        email: claims.email,
        subject: claims.sub,
    })
}

/// The published keys, fetched and cached.
///
/// The impure half, kept apart from [`verify`] so that everything deciding
/// anything is testable from a fixture.
#[derive(Debug)]
pub struct KeyCache {
    inner: RwLock<Option<(std::time::Instant, Keys)>>,
}

impl Default for KeyCache {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyCache {
    /// An empty cache.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            inner: RwLock::new(None),
        }
    }

    /// The current keys, fetching them if the cache is cold or stale.
    ///
    /// # Errors
    ///
    /// Returns [`Denied::NoKeys`] when the fetch fails and there is nothing
    /// cached. A stale-but-present set is **not** used past its lifetime here:
    /// serving on keys that may have been revoked is exactly the tradeoff this
    /// module refuses everywhere else.
    pub fn get(&self, config: &Config) -> Result<Keys, Denied> {
        if let Ok(guard) = self.inner.read()
            && let Some((fetched, keys)) = guard.as_ref()
            && is_fresh(fetched.elapsed())
        {
            return Ok(keys.clone());
        }

        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(10)))
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

/// Whether a cached key set of this age may still be used.
///
/// A pure function rather than an inline comparison, because the comparison is
/// the whole cache and inlined it is untestable without waiting an hour. The
/// two ways to get it wrong point in opposite directions and only one is
/// visible: refetching every time is slow, and *never* refetching means Radar
/// keeps verifying against keys Cloudflare has revoked.
///
/// Strict, so a set exactly at its lifetime is refetched. The boundary case
/// arrives once an hour on a busy instance and should fall towards freshness.
#[must_use]
pub const fn is_fresh(age: Duration) -> bool {
    age.as_nanos() < KEY_LIFETIME.as_nanos()
}

/// Paths served without an identity check even when Access is enforced.
///
/// Two, and both are deliberate. `/health` is what an uptime monitor reads, and
/// putting it behind a login turns every check into a false alarm. `/x402/` is
/// the paid public surface, which has its own paywall and is *meant* to be
/// reachable by strangers.
///
/// Prefix matching, and the trailing slash on `/x402/` is load-bearing: without
/// it a path like `/x402-internal` would match.
#[must_use]
pub fn is_public(path: &str) -> bool {
    path == "/health" || path.starts_with("/x402/")
}

/// The token on a request, from the header or the cookie Cloudflare sets.
///
/// The header is the documented one; the cookie is what a browser navigating
/// directly carries. Neither is trusted — both are fed to [`verify`].
#[must_use]
pub fn token_from(headers: &axum::http::HeaderMap) -> Option<String> {
    if let Some(assertion) = headers
        .get(ASSERTION_HEADER)
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.trim().is_empty())
    {
        return Some(assertion.trim().to_owned());
    }

    // `CF_Authorization` among possibly several cookies. Parsed by hand because
    // the alternative is a cookie-parsing dependency for one lookup, and because
    // the value is about to be signature-checked either way.
    headers
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())?
        .split(';')
        .filter_map(|pair| pair.split_once('='))
        .find(|(name, _)| name.trim() == "CF_Authorization")
        .map(|(_, value)| value.trim().to_owned())
        .filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// The modulus of the key that signed the fixtures below, base64url.
    ///
    /// Generated for this test and used nowhere else. The private half was
    /// discarded; these tokens are the only thing it ever signed.
    const TEST_N: &str = "sOasvv-ulJl6m-qA-hMcVSzI3HnXrU7NvQKgJimrEzKZYXWTW00gScybMoTE9IUCx4n3axXlaAPlMaj9DhC9N05i-qY0hleT_-AgUzWIp27YS5ynqUSscqp-vqCEw13ILT8qmjhBJsxmt8R2nHNlLV7N76UtQcGBFoC7V_bqCnDECR-kUO85LUceoNPbsXPBcdzcCVNwZjOY33aMsDx8tnwwBkRRlVzZ2IXifUTThuNJ_CeB8SGADmIMdUgBbGhcujcBGyxhSxV0sCrG6-eZx7afdQ9Rlov1HN5UX3_NMr25Si_YFH0iphtRVN2USZSiAckgDn3V7VoXAHaLzvxGYw";

    /// A real RS256 token for the configured application, expiring in 2099.
    const GOOD: &str = "eyJhbGciOiJSUzI1NiIsImtpZCI6InRlc3Qta2V5LTEiLCJ0eXAiOiJKV1QifQ.eyJhdWQiOlsicmFkYXItYXVkLXRhZyJdLCJpc3MiOiJodHRwczovL2hleXZlcmEuY2xvdWRmbGFyZWFjY2Vzcy5jb20iLCJleHAiOjQxMDI0NDQ4MDAsImlhdCI6MTc1NjAwMDAwMCwiZW1haWwiOiJqb3NoZmFpcjJAZ21haWwuY29tIiwic3ViIjoidXNlci0xIn0.U-RIQ227qDUpb756_3D3a5FYCJ_gzgGNDU5gwB4IUegcY3h_qi1xG7QVa077IUjWNtMaoteEGWhhhRtAbskhnSXAoGaeFBwYSx4QgFJXH5gElTG82StiIZNu3RAFsbOUhKbaGJ_ibJt_huPIgAeKKfq7v8_f9kemIbBeSICdluXddFiQ_1JwtKPzxLR7qoVZwXjNB9foix9X5qCE5U3IGpvgP4OLtw9CBmQ9D_Pu8d5A2iKJofH1YJnQCs07KLKjrHvmeLryO8UoSm1njRf9ZGtipCSLdxPSj77zt2hl_b3k-3BdjQF0H9gKG-SJOlvr4WHFVjROLT2I1Uzxdsc-Cw";

    /// Correctly signed, expired in 2025.
    const EXPIRED: &str = "eyJhbGciOiJSUzI1NiIsImtpZCI6InRlc3Qta2V5LTEiLCJ0eXAiOiJKV1QifQ.eyJhdWQiOlsicmFkYXItYXVkLXRhZyJdLCJpc3MiOiJodHRwczovL2hleXZlcmEuY2xvdWRmbGFyZWFjY2Vzcy5jb20iLCJleHAiOjE3NTYwMDAwMDAsImlhdCI6MTc1NTAwMDAwMCwiZW1haWwiOiJqb3NoZmFpcjJAZ21haWwuY29tIn0.VkHaVGEwL7lwidc4gzOfXUeHrOFTsxFGn9v4qZPJTFMFmqeipgujmHg53kA3PMHtv57KT32JMtapkhoK-I7muhECZehYet9td9fvyWMyM9zNmKjhRGKnmjxeCSd6URsv4cY0PtOP2-h0D7UkCpcaZ5lLUYSSRPoh1Z8TJ48ROfFLsfIk177BX2oTUGN5iT5evqKLSqHfY_qDA2NhLsZYDEemmgABRxYiKuYu9lzmxDAb67FB7U7cUod7luKDDoYZALVhA2e5iJT22FVThBSV0KKP38--d7ISdoXieMCQNf9FIuPtYQDxVtVFX0cE_v6gtVoKv5xG-7tI9HhdwbDaZA";

    /// Correctly signed by the same team, for a different application.
    const WRONG_AUD: &str = "eyJhbGciOiJSUzI1NiIsImtpZCI6InRlc3Qta2V5LTEiLCJ0eXAiOiJKV1QifQ.eyJhdWQiOlsic29tZS1vdGhlci1hcHAiXSwiaXNzIjoiaHR0cHM6Ly9oZXl2ZXJhLmNsb3VkZmxhcmVhY2Nlc3MuY29tIiwiZXhwIjo0MTAyNDQ0ODAwLCJpYXQiOjE3NTYwMDAwMDAsImVtYWlsIjoiam9zaGZhaXIyQGdtYWlsLmNvbSJ9.krp9z0TPj4VQg9Vy6XV_szH6uJLlVp4sRIbD-RVMw20s4Swdz69MChoyiwck6hcUo9Xot80P-bU8UuiDvc1--XeCYdxMapZ4xCldn9Mq-TdcXv8ZS2hYKNze2dxPStFLrJDVlUy8Bv7-DCa5RgeBcB-sQjdZFpa6ovUxxSB6YWjjhpzgWeFMsQSZqSn-8DFeE_mdsv_uLwQTtX-8UZqxGx-x9M2Pmce1ZEkiLlon8pLPtHBzmIm56gLSiaxn9QH7NZjs-Xv0IYUB0uFTGtcl79Dt4LCBJOWJmt4zDT4B3Keql5FNMPb2U8EbB0SPvQLqzUx46yzGAq6yr6jtxT6Qug";

    /// Correctly signed by this key, claiming a different issuer.
    const WRONG_ISS: &str = "eyJhbGciOiJSUzI1NiIsImtpZCI6InRlc3Qta2V5LTEiLCJ0eXAiOiJKV1QifQ.eyJhdWQiOlsicmFkYXItYXVkLXRhZyJdLCJpc3MiOiJodHRwczovL2V2aWwuY2xvdWRmbGFyZWFjY2Vzcy5jb20iLCJleHAiOjQxMDI0NDQ4MDAsImlhdCI6MTc1NjAwMDAwMCwiZW1haWwiOiJlQGV4YW1wbGUuY29tIn0.M2jC4cH5ykdsux4g6yTpCbkKAKnEOi7b5bvj-4V7terDQ2B9Xm3UoSahM3xIxoG2jrNEL0fuZ143vVXcGF0JTiSDZ1RmN-kSjLVXXJNt9ydvwcHBwOPa6AlxEqID3uEc5x-ijgKot2Lc1THPFP3_7oC3MkvS5GPXLWE12vAHmBygI9Ni67VaTZAkrdSNt3gIybYOdG1ISzybnLysK0DLYe5TwH0tXxx_CIoKcwLQ346yXhM1ROWRWeAqvdcoFdffpK3_z16F01MyRzZbdB5pGSEfvq1RCGMu_kJ_kLezKj85y972ktC2jxuxaSam8RiJzmbCerfGPggpaBaO0oGYwQ";

    /// Well-formed, correct claims, signed by an unrelated key.
    const OTHER_KEY: &str = "eyJhbGciOiJSUzI1NiIsImtpZCI6InRlc3Qta2V5LTEiLCJ0eXAiOiJKV1QifQ.eyJhdWQiOlsicmFkYXItYXVkLXRhZyJdLCJpc3MiOiJodHRwczovL2hleXZlcmEuY2xvdWRmbGFyZWFjY2Vzcy5jb20iLCJleHAiOjQxMDI0NDQ4MDAsImlhdCI6MTc1NjAwMDAwMCwiZW1haWwiOiJhdHRhY2tlckBleGFtcGxlLmNvbSJ9.oSwokGHd4Ytcu7bSBj3J4cTBkA-d-hGYBdNRpLkrSpKcR_uBqu8vsZwwo7hwejFfDPEKqiYiUJst8AQdjhdMmfX6ne34s67KvRtvi12v_iioEy_ew0rGRUv5mD5hkeQ0dx8Gk_OduRQGsCwnV20cM5CjWm_woI669_e-c7pcim5RdCXlyJl7BMa9A0n8hDBW6LNhY1FuxJ5PDCeMRMxYNceM_TJdbkZSvery0E0uzBJzRrq8m6GOyCiGzqgUVWwOMaeA9qu3KsdWcOhNOuQStYJDYXUKu6d--1yvCZRIIpnC4NM7Rey5BSZNqGRoSoBwDGRfc81locscabEvl61Hpw";

    /// The oldest forgery: correct claims, no signature, `alg: none`.
    const ALG_NONE: &str = "eyJhbGciOiJub25lIiwia2lkIjoidGVzdC1rZXktMSIsInR5cCI6IkpXVCJ9.eyJhdWQiOlsicmFkYXItYXVkLXRhZyJdLCJpc3MiOiJodHRwczovL2hleXZlcmEuY2xvdWRmbGFyZWFjY2Vzcy5jb20iLCJleHAiOjQxMDI0NDQ4MDAsImlhdCI6MTc1NjAwMDAwMCwiZW1haWwiOiJhdHRhY2tlckBleGFtcGxlLmNvbSJ9.";

    /// Algorithm confusion: HMAC-SHA256 with the RSA *public* key as the secret.
    const HS256: &str = "eyJhbGciOiJIUzI1NiIsImtpZCI6InRlc3Qta2V5LTEiLCJ0eXAiOiJKV1QifQ.eyJhdWQiOlsicmFkYXItYXVkLXRhZyJdLCJpc3MiOiJodHRwczovL2hleXZlcmEuY2xvdWRmbGFyZWFjY2Vzcy5jb20iLCJleHAiOjQxMDI0NDQ4MDAsImlhdCI6MTc1NjAwMDAwMCwiZW1haWwiOiJhdHRhY2tlckBleGFtcGxlLmNvbSJ9.Hmn4NGDt3l9PuwFCx5x48msuHDEZFLfvheKBxlScn0A";

    /// 2026-08-27, comfortably inside the good token's life.
    const NOW: u64 = 1_772_000_000;

    fn config() -> Config {
        Config {
            team_domain: "heyvera.cloudflareaccess.com".to_owned(),
            aud: "radar-aud-tag".to_owned(),
        }
    }

    fn keys() -> Keys {
        Keys(vec![Jwk {
            kid: "test-key-1".to_owned(),
            n: TEST_N.to_owned(),
            e: "AQAB".to_owned(),
        }])
    }

    fn vars(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: BTreeMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |k: &str| map.get(k).cloned()
    }

    #[test]
    fn a_genuine_token_is_admitted_and_carries_its_identity() {
        // The positive case matters as much as the refusals: a verifier that
        // refuses everything is not a verifier, it is an outage, and it would
        // pass every test below.
        let identity = verify(GOOD, &keys(), &config(), NOW).expect("a real RS256 token");
        assert_eq!(identity.email, "joshfair2@gmail.com");
        assert_eq!(identity.subject, "user-1");
    }

    #[test]
    fn a_token_signed_by_someone_else_is_refused() {
        // Claims identical to the good one, signature from an unrelated 2048-bit
        // key. This is what an attacker who has read the source produces.
        assert_eq!(
            verify(OTHER_KEY, &keys(), &config(), NOW),
            Err(Denied::BadSignature)
        );
    }

    #[test]
    fn alg_none_is_refused_before_the_signature_is_looked_at() {
        // The oldest JWT forgery, and it works against any verifier that
        // dispatches on the header instead of pinning the algorithm.
        assert_eq!(
            verify(ALG_NONE, &keys(), &config(), NOW),
            Err(Denied::UnsupportedAlgorithm("none".to_owned()))
        );
    }

    #[test]
    fn an_hs256_token_signed_with_the_public_key_is_refused() {
        // Algorithm confusion: the RSA public key is not secret, so a verifier
        // willing to treat it as an HMAC secret can be handed a token anyone
        // can mint. Pinning RS256 settles it without any of that reasoning
        // having to be right at the call site.
        assert_eq!(
            verify(HS256, &keys(), &config(), NOW),
            Err(Denied::UnsupportedAlgorithm("HS256".to_owned()))
        );
    }

    #[test]
    fn a_token_for_another_application_in_the_same_team_is_refused() {
        // The check people skip. Cloudflare signs every application in a team
        // with the same keys, so without this any colleague's access to any
        // other application is access to Radar -- with a signature that
        // verifies perfectly.
        assert_eq!(
            verify(WRONG_AUD, &keys(), &config(), NOW),
            Err(Denied::WrongAudience)
        );
    }

    #[test]
    fn a_token_naming_another_issuer_is_refused() {
        assert_eq!(
            verify(WRONG_ISS, &keys(), &config(), NOW),
            Err(Denied::WrongIssuer)
        );
    }

    #[test]
    fn expiry_is_enforced_and_the_skew_is_one_sided() {
        // A correctly signed token that has expired. The skew allowance is
        // asserted at both edges, because a skew applied in the wrong direction
        // is a token that stops working a minute early rather than one that
        // works a minute late -- and only one of those gets reported.
        assert_eq!(
            verify(EXPIRED, &keys(), &config(), NOW),
            Err(Denied::Expired)
        );

        // exp of the good token is 4102444800.
        let exp = 4_102_444_800;
        assert!(verify(GOOD, &keys(), &config(), exp).is_ok(), "at exp");
        assert!(
            verify(GOOD, &keys(), &config(), exp + SKEW_SECONDS).is_ok(),
            "inside the skew"
        );
        assert_eq!(
            verify(GOOD, &keys(), &config(), exp + SKEW_SECONDS + 1),
            Err(Denied::Expired),
            "one second past the skew"
        );
    }

    #[test]
    fn an_unknown_key_id_is_refused_rather_than_tried_against_the_others() {
        // The tempting fallback -- "no matching kid, try them all" -- turns a
        // key set into a list of keys any of which will do, which is not what a
        // key id is for.
        let other_kid = Keys(vec![Jwk {
            kid: "some-other-key".to_owned(),
            n: TEST_N.to_owned(),
            e: "AQAB".to_owned(),
        }]);
        assert_eq!(
            verify(GOOD, &other_kid, &config(), NOW),
            Err(Denied::UnknownKey)
        );
    }

    #[test]
    fn a_tampered_payload_does_not_verify() {
        // The point of a signature. Swap one character of the payload -- which
        // is plainly readable base64 -- and the token must die.
        let mut parts: Vec<&str> = GOOD.split('.').collect();
        let payload = parts[1].to_owned();
        let tampered = format!("{}X{}", &payload[..payload.len() - 1], "");
        parts[1] = &tampered;
        assert_eq!(
            verify(&parts.join("."), &keys(), &config(), NOW),
            Err(Denied::BadSignature)
        );
    }

    #[test]
    fn anything_that_is_not_three_segments_is_malformed() {
        for bad in [
            "",
            "one",
            "one.two",
            "one.two.three.four",
            "....",
            "not base64!.at.all",
        ] {
            assert!(
                matches!(
                    verify(bad, &keys(), &config(), NOW),
                    Err(Denied::Malformed | Denied::UnsupportedAlgorithm(_))
                ),
                "{bad:?} should not verify"
            );
        }
    }

    #[test]
    fn the_mode_refuses_to_guess_when_nothing_says_who_may_look() {
        // The design of the whole function. Both possible defaults are wrong in
        // a way that is invisible: serving to everyone looks like it works, and
        // refusing everyone looks like a broken deploy.
        let why = Mode::from_vars(&vars(&[])).expect_err("no default");
        assert!(why.contains("RADAR_ACCESS_TEAM"), "{why}");
        assert!(why.contains("RADAR_ACCESS=off"), "{why}");

        // Half-configured is the same refusal, naming the half that is missing.
        let half = Mode::from_vars(&vars(&[(
            "RADAR_ACCESS_TEAM",
            "heyvera.cloudflareaccess.com",
        )]))
        .expect_err("a team without an audience verifies nothing useful");
        assert!(half.contains("RADAR_ACCESS_AUD"), "{half}");
    }

    #[test]
    fn switching_it_off_has_to_be_written_down_in_as_many_words() {
        assert_eq!(
            Mode::from_vars(&vars(&[("RADAR_ACCESS", "off")])),
            Ok(Mode::Off)
        );
        assert_eq!(
            Mode::from_vars(&vars(&[("RADAR_ACCESS", "OFF")])),
            Ok(Mode::Off),
            "case is not the operator's problem"
        );
        // And anything else is not "off". A typo must not disable the check.
        for typo in ["of", "false", "no", "0", "disabled", ""] {
            assert!(
                Mode::from_vars(&vars(&[("RADAR_ACCESS", typo)])).is_err(),
                "{typo:?} must not read as off"
            );
        }
    }

    #[test]
    fn a_placeholder_pasted_verbatim_is_refused_rather_than_enforced() {
        // The real incident, on 2026-08-30. A runbook command carried its own
        // placeholder inside a heredoc and the whole line went into the env
        // file. Accepted, the server enforces against an audience no token can
        // match and the site locks out completely -- fail-closed, and useless as
        // a diagnosis, because it presents as "nobody can log in".
        let pasted = vars(&[
            ("RADAR_ACCESS_TEAM", "heyvera.cloudflareaccess.com"),
            (
                "RADAR_ACCESS_AUD",
                "<Application Audience tag from the Access app's Overview page>",
            ),
        ]);
        let why = Mode::from_vars(&pasted).expect_err("that is not an audience tag");
        assert!(why.contains("RADAR_ACCESS_AUD"), "{why}");
        assert!(why.contains("placeholder"), "{why}");
        assert!(
            why.contains("Overview page"),
            "and says where to find it: {why}"
        );
    }

    #[test]
    fn the_placeholder_check_looks_for_prose_not_for_a_format() {
        // Narrow on purpose. Pinning "64 hex characters" would reject a valid
        // tag the day the vendor changes it, at startup, on a server that had
        // been working -- which is a worse failure than the one being caught.
        assert!(looks_unsubstituted("<TODO>").is_some());
        // One bracket, not two. `<TODO>` alone passes a check that requires
        // BOTH, and a half-pasted placeholder -- a heredoc cut at a line
        // boundary, a value copied without its closing bracket -- is exactly
        // the shape that would then slip through.
        assert!(
            looks_unsubstituted("<TODO").is_some(),
            "an opening bracket alone"
        );
        assert!(
            looks_unsubstituted("TODO>").is_some(),
            "a closing bracket alone"
        );
        assert!(looks_unsubstituted("your tag here").is_some());
        assert!(looks_unsubstituted("  spaced value  ").is_some());

        // A real tag is 64 hex characters today, and anything without spaces or
        // brackets passes -- including shapes this code has never seen.
        assert!(looks_unsubstituted(&"a3f9".repeat(16)).is_none());
        assert!(looks_unsubstituted("heyvera.cloudflareaccess.com").is_none());
        assert!(looks_unsubstituted("some-future-format_v2").is_none());
    }

    #[test]
    fn configured_and_switched_off_at_once_is_a_contradiction() {
        // Resolving this silently either way is how an instance ends up open
        // with an Access application sitting in front of it that nobody checks.
        assert!(
            Mode::from_vars(&vars(&[
                ("RADAR_ACCESS", "off"),
                ("RADAR_ACCESS_TEAM", "heyvera.cloudflareaccess.com"),
                ("RADAR_ACCESS_AUD", "radar-aud-tag"),
            ]))
            .is_err()
        );
    }

    #[test]
    fn a_full_configuration_derives_the_issuer_and_the_key_url() {
        let Ok(Mode::Enforce(config)) = Mode::from_vars(&vars(&[
            ("RADAR_ACCESS_TEAM", " heyvera.cloudflareaccess.com "),
            ("RADAR_ACCESS_AUD", " radar-aud-tag "),
        ])) else {
            panic!("fully configured");
        };
        assert_eq!(config.issuer(), "https://heyvera.cloudflareaccess.com");
        assert_eq!(
            config.jwks_url(),
            "https://heyvera.cloudflareaccess.com/cdn-cgi/access/certs"
        );
    }

    #[test]
    fn a_key_set_with_nothing_usable_in_it_is_an_error_not_an_empty_set() {
        // An empty set refuses every token with `UnknownKey`, which sends an
        // operator looking for a rotation that did not happen.
        assert!(matches!(Keys::parse("{}"), Err(Denied::NoKeys(_))));
        assert!(matches!(Keys::parse("not json"), Err(Denied::NoKeys(_))));
        assert!(matches!(
            Keys::parse(r#"{"keys":[{"kty":"EC","kid":"x"}]}"#),
            Err(Denied::NoKeys(_))
        ));

        let mixed = Keys::parse(&format!(
            r#"{{"keys":[{{"kty":"EC","kid":"ec"}},{{"kty":"RSA","kid":"test-key-1","n":"{TEST_N}","e":"AQAB"}}]}}"#
        ))
        .expect("one usable key");
        assert_eq!(mixed.0.len(), 1, "the EC key is skipped, not fatal");
        assert!(mixed.by_id("test-key-1").is_some());
        assert!(mixed.by_id("ec").is_none());
    }

    #[test]
    fn a_cached_key_set_is_used_until_its_lifetime_and_not_past_it() {
        // The two failures point in opposite directions and only one is
        // visible. Refetching every request is slow and somebody notices;
        // never refetching means Radar keeps verifying against keys Cloudflare
        // has revoked, and nobody notices at all.
        assert!(is_fresh(Duration::ZERO));
        assert!(is_fresh(
            KEY_LIFETIME
                .checked_sub(Duration::from_nanos(1))
                .expect("the lifetime is not zero")
        ));
        assert!(!is_fresh(KEY_LIFETIME), "exactly at its lifetime, refetch");
        assert!(!is_fresh(KEY_LIFETIME + Duration::from_secs(1)));
        assert!(!is_fresh(Duration::from_secs(86_400)));
    }

    #[test]
    fn only_the_monitor_path_and_the_paid_surface_are_public() {
        assert!(is_public("/health"));
        assert!(is_public("/x402/v1/instruments"));

        for private in [
            "/",
            "/ops",
            "/v1/funnel",
            "/v1/tokens/abc",
            "/v1/events",
            "/v1/chat",
            "/mcp",
            "/assets/index-abc123.js",
            // The trailing slash in the prefix is what stops these.
            "/x402",
            "/x402-internal/secrets",
            "/health/../v1/funnel",
        ] {
            assert!(!is_public(private), "{private} must not be public");
        }
    }

    #[test]
    fn the_token_is_taken_from_the_header_or_the_cookie() {
        use axum::http::HeaderMap;

        let mut headers = HeaderMap::new();
        assert_eq!(token_from(&headers), None, "nothing presented");

        headers.insert(
            COOKIE_FOR_TEST,
            "other=1; CF_Authorization=abc; x=2".parse().unwrap(),
        );
        assert_eq!(token_from(&headers).as_deref(), Some("abc"));

        // The header wins when both are present: it is the documented one, and
        // a stale cookie beside a fresh header should not be what is checked.
        headers.insert(ASSERTION_HEADER, "from-header".parse().unwrap());
        assert_eq!(token_from(&headers).as_deref(), Some("from-header"));

        // An empty header is absence, not a token -- otherwise it is fed to the
        // verifier as `Malformed` rather than reported as `Missing`, and the
        // two send an operator to different places.
        let mut blank = HeaderMap::new();
        blank.insert(ASSERTION_HEADER, "   ".parse().unwrap());
        assert_eq!(token_from(&blank), None);

        let mut no_such_cookie = HeaderMap::new();
        no_such_cookie.insert(COOKIE_FOR_TEST, "session=1; other=2".parse().unwrap());
        assert_eq!(token_from(&no_such_cookie), None);
    }

    const COOKIE_FOR_TEST: axum::http::HeaderName = axum::http::header::COOKIE;

    #[test]
    fn the_email_header_is_named_only_so_that_nothing_reads_it() {
        // Rendered here rather than in a comment, so a search for the header
        // finds this test and its explanation before it finds a use of it.
        assert_eq!(UNTRUSTED_EMAIL_HEADER, "cf-access-authenticated-user-email");
        assert_eq!(ASSERTION_HEADER, "cf-access-jwt-assertion");
    }
}
