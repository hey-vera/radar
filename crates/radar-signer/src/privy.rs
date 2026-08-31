// SPDX-License-Identifier: Apache-2.0
//! Authorising Privy to sign, without ever signing arbitrary bytes.
//!
//! [ADR 0007](https://github.com/hey-vera/radar/blob/main/docs/adr/0007-the-privy-authorization-key-lives-in-the-signer-process.md)
//! puts the Privy authorization key here rather than in `radar-serve`. The
//! reason is invariant 1: a signature made with this key causes a customer's
//! wallet to move funds, which makes it the same category of object as the local
//! wallet key this process exists to isolate.
//!
//! `radar-serve` is the process with a listener, a model provider, an HTTP
//! client, an embedded frontend and a paywall. It is the largest attack surface
//! in the system and it is the one process that should never hold a key.
//!
//! # The shape of the method, and why it is not a signing function
//!
//! There is deliberately **no** `sign(bytes) -> Signature` here.
//!
//! [`authorise`] takes a typed Privy request and a kernel [`Authorization`]. It
//! pulls the transaction out of the request body **itself**, decodes it, and puts
//! it through the same [`verify::check`] the local signing path uses — so the
//! bytes checked are provably the bytes the signature will cause to be signed. A
//! caller that could hand over one transaction for checking and another for
//! signing would make the check decorative, and that is precisely what a
//! byte-signing method would allow.
//!
//! This is the same reasoning that makes the signer refuse address lookup
//! tables: the guarantee is *every account it authorises is one it read in the
//! bytes it signed*, and any method that signs what it is handed destroys it.

use radar_risk::Authorization;
use radar_types::{Address, Slot};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::canonical::{NotCanonical, canonicalise};
use crate::verify::{Allowlist, Rejection};

/// The payload version Privy specifies.
///
/// Part of the signed bytes, so a change here changes every signature. Privy
/// pins it at 1.
const PAYLOAD_VERSION: u64 = 1;

/// The prefix Privy puts on an exported authorization key.
///
/// Stripped rather than rejected, because it is what the dashboard hands an
/// operator and requiring them to edit it out is a step that gets done wrong at
/// three in the morning.
const KEY_PREFIX: &str = "wallet-auth:";

/// Why an authorization signature could not be produced.
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum NotAuthorised {
    /// The kernel's bounds refuse this transaction.
    ///
    /// A refusal is a normal outcome, not an error — the signer refusing is the
    /// system working.
    #[error("the transaction is outside the authorisation: {}", .0.join("; "))]
    Refused(Vec<String>),
    /// The request body carries no transaction to check.
    ///
    /// **This is a refusal, not a pass.** A request whose transaction cannot be
    /// found is a request whose contents were not read, and signing one would
    /// authorise bytes nothing inspected.
    #[error(
        "the Privy request body has no `params.transaction` to check, so there \
         is nothing to authorise. A request whose contents cannot be read is \
         never signed."
    )]
    NoTransaction,
    /// The transaction inside the request is not valid base64.
    #[error("the transaction in the request body is not base64: {0}")]
    NotBase64(String),
    /// The payload could not be canonicalised.
    #[error(transparent)]
    NotCanonical(#[from] NotCanonical),
    /// The key could not sign.
    #[error("the authorization key could not sign: {0}")]
    KeyFailed(String),
}

/// A Privy JSON-RPC request, as it will be sent.
///
/// Held as the parsed body rather than as a string so the transaction can be
/// read out of it and the canonical payload built from the same value. A string
/// would have to be re-parsed, and a re-parse is where the checked bytes and the
/// sent bytes get to differ.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct PrivyRequest {
    /// The HTTP method, upper case.
    pub method: String,
    /// The full URL, without a trailing slash.
    pub url: String,
    /// The JSON body.
    pub body: Value,
    /// The `privy-app-id` header, and any idempotency or expiry header.
    ///
    /// Privy signs only its own prefixed headers, so anything else here would
    /// change the bytes without changing what Privy reconstructs.
    pub headers: serde_json::Map<String, Value>,
}

impl PrivyRequest {
    /// The base64 transaction this request asks Privy to sign.
    ///
    /// Read from the body that will actually be sent, which is the whole point.
    #[must_use]
    pub fn transaction(&self) -> Option<&str> {
        self.body.get("params")?.get("transaction")?.as_str()
    }

    /// The canonical payload Privy will rebuild and verify against.
    ///
    /// # Errors
    ///
    /// [`NotCanonical`] when the body contains something outside the supported
    /// subset — a float, in practice.
    pub fn payload(&self) -> Result<String, NotCanonical> {
        canonicalise(&json!({
            "version": PAYLOAD_VERSION,
            "method": self.method,
            "url": self.url,
            "body": self.body,
            "headers": Value::Object(self.headers.clone()),
        }))
    }
}

/// The P-256 key whose public half is registered as a signer on customer
/// wallets.
///
/// No `Debug`, no `Display`, no accessor for the bytes. The only thing that can
/// be done with it is [`authorise`], which will not act without a kernel
/// authorisation.
pub struct AuthorizationKey {
    pair: ring::signature::EcdsaKeyPair,
}

impl AuthorizationKey {
    /// Loads a key from Privy's exported form.
    ///
    /// Accepts the `wallet-auth:` prefix the dashboard adds, and either a PEM
    /// block or bare base64 PKCS#8.
    ///
    /// # Errors
    ///
    /// [`NotAuthorised::KeyFailed`] when the material is not a P-256 private key.
    /// Deliberately not more specific: a message distinguishing "wrong curve"
    /// from "wrong length" tells an attacker with a log more than it tells an
    /// operator.
    pub fn parse(material: &str) -> Result<Self, NotAuthorised> {
        let trimmed = material.trim().trim_start_matches(KEY_PREFIX).trim();
        let body: String = trimmed
            .lines()
            .filter(|line| !line.starts_with("-----"))
            .collect::<Vec<_>>()
            .join("");
        let der = radar_types::b64::decode(body.trim()).ok_or_else(|| {
            NotAuthorised::KeyFailed("the key material is not valid base64".to_owned())
        })?;

        // ASN.1/DER signatures, because that is what Privy verifies. `FIXED`
        // would produce a signature of the right length that never validates.
        ring::signature::EcdsaKeyPair::from_pkcs8(
            &ring::signature::ECDSA_P256_SHA256_ASN1_SIGNING,
            &der,
            &ring::rand::SystemRandom::new(),
        )
        .map(|pair| Self { pair })
        .map_err(|e| NotAuthorised::KeyFailed(e.to_string()))
    }
}

/// Produces a `privy-authorization-signature` for a request the kernel
/// authorised.
///
/// The order is the point: **check, then sign**, on bytes taken from the request
/// itself.
///
/// # Errors
///
/// [`NotAuthorised`] whenever the transaction is outside the authorisation,
/// cannot be found, cannot be decoded, or the payload cannot be canonicalised.
/// Every one is a refusal; none produces a signature.
pub fn authorise(
    key: &AuthorizationKey,
    request: &PrivyRequest,
    authorization: &Authorization,
    signing_wallet: &Address,
    allowlist: &Allowlist,
    now: Slot,
) -> Result<String, NotAuthorised> {
    // Read out of the body that will be sent. Not passed in alongside it: a
    // caller able to supply one transaction for checking and another for
    // signing would make this whole function decorative.
    let encoded = request.transaction().ok_or(NotAuthorised::NoTransaction)?;
    let bytes = radar_types::b64::decode(encoded).ok_or_else(|| {
        NotAuthorised::NotBase64("the transaction is not valid base64".to_owned())
    })?;

    crate::verify::check(authorization, &bytes, signing_wallet, allowlist, now).map_err(
        |rejections| NotAuthorised::Refused(rejections.iter().map(Rejection::to_string).collect()),
    )?;

    let payload = request.payload()?;
    let signature = key
        .pair
        .sign(&ring::rand::SystemRandom::new(), payload.as_bytes())
        .map_err(|e| NotAuthorised::KeyFailed(e.to_string()))?;
    Ok(radar_types::b64::encode(signature.as_ref()))
}
