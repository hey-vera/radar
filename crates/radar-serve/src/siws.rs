// SPDX-License-Identifier: Apache-2.0
//! Sign-In With Solana: the challenge, and the verification of what came back.
//!
//! Lifted out of `lib.rs` on 2026-09-03. The line count was not the reason --
//! design 0004 §3.5 found that most of this repository's large files are half
//! inline test module, and that the one real case was this crate, where 1,366
//! lines of code sat against 79 of test in the only process on the internet.
//!
//! This is the part of it that most deserved its own file. It is the act of
//! authenticating, so both handlers are unauthenticated by necessity, and every
//! guarantee they carry is a property of text a stranger sent. Sitting in a
//! module of its own, its tests are visibly its tests.
//!
//! A connected wallet is **authentication, not authority** (`AGENTS.md` rule 1).
//! Nothing here softens a refusal or reaches a signer; it establishes who is
//! asking, and no more than that.

use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse as _, Response},
};

use crate::{AppState, chat, now_unix};

/// What a wallet asks for.
#[derive(serde::Deserialize)]
pub struct ChallengeBody {
    /// The wallet about to sign. Needed because the message names it, and the
    /// message is rendered here rather than by the client.
    pub address: String,
}

/// Issues a challenge for a wallet to sign.
///
/// **Public, and unauthenticated by necessity** -- it runs before anyone has
/// proved anything. That is why `Challenges` has a ceiling: the only caller is
/// the whole internet.
///
/// The domain comes from the server, never from the request. A caller who could
/// choose the domain could have a wallet sign a message naming somebody else's
/// site, which is the replay this binds against.
pub async fn challenge(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ChallengeBody>,
) -> Response {
    let Some(challenges) = state.challenges.as_ref() else {
        // Rule 8: an instance with no customer domain configured cannot bind a
        // signature to itself, so it does not pretend to.
        return chat::refuse(
            StatusCode::SERVICE_UNAVAILABLE,
            "this instance has no customer sign-in configured",
        );
    };
    let Ok(address) = body.address.parse::<radar_types::Address>() else {
        return chat::refuse(StatusCode::BAD_REQUEST, "that is not a Solana address");
    };
    let nonce = radar_types::b64::encode_url(&random_nonce());
    match challenges.issue(nonce, now_unix()) {
        Ok(challenge) => Json(serde_json::json!({
            // The exact text to sign, rendered **here**. The client must not
            // build its own: two renderings of one message can drift, and the
            // drift would surface as a signature that will not verify, which
            // reads like a wallet bug rather than a mismatch.
            //
            // The address is in it because `siws::verify` requires the signed
            // text to name the key that signed -- see that module for the
            // attack it stops.
            "message": challenge.message(&address),
            "expires_in_seconds": radar_customer::siws::MAX_AGE_SECONDS,
        }))
        .into_response(),
        Err(busy) => chat::refuse(StatusCode::SERVICE_UNAVAILABLE, &busy.to_string()),
    }
}

/// What a wallet sends back.
#[derive(serde::Deserialize)]
pub struct SignInBody {
    /// The wallet that signed.
    pub address: String,
    /// The full signed text, so the nonce spent is the nonce signed.
    pub message: String,
    /// The signature, base64.
    pub signature: String,
}

/// Verifies a signed challenge and issues a session.
///
/// Public for the same reason as the challenge: this *is* the act of
/// authenticating, so requiring authentication would be circular.
pub async fn verify(State(state): State<Arc<AppState>>, Json(body): Json<SignInBody>) -> Response {
    let Some(challenges) = state.challenges.as_ref() else {
        return chat::refuse(
            StatusCode::SERVICE_UNAVAILABLE,
            "this instance has no customer sign-in configured",
        );
    };
    let Ok(address) = body.address.parse::<radar_types::Address>() else {
        return chat::refuse(StatusCode::BAD_REQUEST, "that is not a Solana address");
    };
    let Some(signature) = radar_types::b64::decode(&body.signature) else {
        return chat::refuse(StatusCode::BAD_REQUEST, "the signature is not base64");
    };

    // The nonce is read out of the message the wallet signed, not from a
    // separate field. A caller that could name one nonce and sign another would
    // spend a challenge it never proved anything about.
    let Some(nonce) = nonce_in(&body.message) else {
        return chat::refuse(StatusCode::BAD_REQUEST, "the message carries no nonce");
    };
    let now = now_unix();
    // Spent here, before verification, and deliberately: a signature that fails
    // to verify still consumes the challenge it was offered against. Otherwise
    // one issued nonce can be attacked repeatedly.
    let Some(challenge) = challenges.spend(&nonce, now) else {
        return chat::refuse(
            StatusCode::UNAUTHORIZED,
            "that challenge is unknown, spent, or expired",
        );
    };

    let signin = radar_customer::siws::SignIn {
        address,
        message: body.message,
        signature,
    };
    match radar_customer::siws::verify(&signin, &challenge, now) {
        Ok(verified) => {
            match radar_customer::session::issue(&verified, &state.customer_salt, now) {
                Ok(token) => Json(serde_json::json!({
                    "token": token,
                    "address": verified.to_string(),
                    "expires_in_seconds": radar_customer::session::LIFETIME_SECONDS,
                }))
                .into_response(),
                Err(e) => chat::refuse(StatusCode::SERVICE_UNAVAILABLE, &e.to_string()),
            }
        }
        Err(refused) => chat::refuse(StatusCode::UNAUTHORIZED, &refused.to_string()),
    }
}

/// The nonce line of a signed message.
///
/// Read from the signed text rather than taken as a separate field, so the
/// challenge that is spent is the one that was actually signed.
fn nonce_in(message: &str) -> Option<String> {
    message
        .lines()
        .find_map(|line| line.strip_prefix("Nonce: "))
        .map(|n| n.trim().to_owned())
        .filter(|n| !n.is_empty())
}

/// The sign-in domain an instance is configured with, if any.
///
/// Extracted from `main` so it can be tested. Inline, the emptiness guard could
/// be deleted and nothing noticed -- and what it guards is not cosmetic: a blank
/// domain would have wallets sign a message naming no site at all, and the
/// binding that stops a signature being replayed from another application is
/// exactly that name.
#[must_use]
pub fn domain_from(raw: Option<String>) -> Option<String> {
    raw.filter(|d| !d.trim().is_empty())
}

/// Thirty-two random bytes.
fn random_nonce() -> [u8; 32] {
    use ring::rand::SecureRandom as _;
    let mut bytes = [0u8; 32];
    // A failure here means the system random source is unavailable, which is not
    // a condition to paper over with a weaker source.
    ring::rand::SystemRandom::new()
        .fill(&mut bytes)
        .expect("the system random source");
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_nonce_line_that_is_blank_is_not_a_nonce() {
        // The guard exists because an empty nonce would be looked up in the
        // challenge store, miss, and refuse -- but only by luck. A message
        // carrying `Nonce: ` and nothing else is malformed, and saying so here
        // keeps the store from being asked about the empty string at all.
        assert_eq!(nonce_in("Nonce: abc123"), Some("abc123".to_owned()));
        assert_eq!(nonce_in("Nonce:    spaced   "), Some("spaced".to_owned()));
        assert_eq!(nonce_in("Nonce: "), None);
        assert_eq!(nonce_in("Nonce:    "), None);
        assert_eq!(nonce_in("no nonce here"), None);
        assert_eq!(nonce_in(""), None);
    }

    #[test]
    fn the_nonce_is_read_from_the_line_that_names_it() {
        // Multi-line, because the message a wallet signs is multi-line and the
        // nonce is not the first field.
        let message = "site wants you to sign in:\naddress\n\nNonce: n-1\nIssued At: 5";
        assert_eq!(nonce_in(message), Some("n-1".to_owned()));
    }

    #[test]
    fn a_nonce_is_actually_random() {
        // The mutation that prompted this replaced the whole function with a
        // constant, and nothing failed. A fixed nonce defeats the entire
        // anti-replay mechanism: every challenge would carry the same value, so
        // one captured signature would authenticate forever.
        let a = random_nonce();
        let b = random_nonce();
        assert_ne!(a, b, "two nonces must not be equal");
        assert_ne!(a, [0u8; 32], "and must not be a fixed constant");
        assert_ne!(a, [1u8; 32]);
        // Not a randomness test -- that is the system source's job. This asserts
        // the bytes were written at all, which is what the mutation removed.
        assert!(a.iter().any(|byte| *byte != a[0]), "not all one value");
    }

    #[test]
    fn a_blank_customer_domain_is_no_domain() {
        // Rule 8. An instance that does not know its own site cannot bind a
        // signature to it, and a blank string is not a site -- it is a missing
        // setting that happens to be present.
        assert_eq!(
            domain_from(Some("radar.heyvera.org".to_owned())),
            Some("radar.heyvera.org".to_owned())
        );
        assert_eq!(domain_from(None), None);
        assert_eq!(domain_from(Some(String::new())), None);
        assert_eq!(domain_from(Some("   ".to_owned())), None);
        assert_eq!(domain_from(Some("\t\n".to_owned())), None);
    }
}
