// SPDX-License-Identifier: Apache-2.0
//! Stamping a Turnkey request, after checking what it would do.
//!
//! [ADR 0011](https://github.com/hey-vera/radar/blob/main/docs/adr/0011-one-wallet-system-two-authority-levels-on-turnkey.md)
//! moves the customer wallet lane from Privy to Turnkey. This is the same
//! object [`crate::privy`] is, with the envelope changed: a P-256 key whose
//! public half is registered against an organisation, used to authenticate a
//! request that causes customer funds to move.
//!
//! # Deferred, and deliberately inert
//!
//! **Nothing calls this.** [ADR 0011's amendment](https://github.com/hey-vera/radar/blob/main/docs/adr/0011-one-wallet-system-two-authority-levels-on-turnkey.md)
//! sequences bring-your-own-wallet first, so the embedded-wallet vendor choice —
//! Turnkey or Privy — is deferred until an edge exists and a customer wants
//! autonomy. `Policy::CLOSED` ships and the measured edge is 0 bps, so a scoped
//! delegation would be built for a capability nothing justifies using.
//!
//! It is kept rather than deleted because the analysis behind it is the thing
//! worth keeping, and the stamping is verified offline against Turnkey's
//! documented format. It has **never been run against a live organisation**.
//!
//! Its only exerciser is its own tests. If that changes, ADR 0011's
//! preconditions have to be met first — starting with the policy engine being
//! shown to *refuse*.
//!
//! # Why it lives here and not in `radar-serve`
//!
//! [ADR 0007](https://github.com/hey-vera/radar/blob/main/docs/adr/0007-the-privy-authorization-key-lives-in-the-signer-process.md)'s
//! argument, reused with the noun changed. A credential that makes customer
//! funds move is the same category of object as a wallet key, so it stays out of
//! the process with a listener, a model provider, an HTTP client, an embedded
//! frontend and a paywall.
//!
//! # Check, then stamp
//!
//! The ordering is the whole point, and it is the same one `privy::authorise`
//! holds. The transaction is read **out of the body that will be sent**, run
//! through [`crate::verify::check`], and only then stamped. A caller able to
//! hand over one transaction for checking and another for stamping would make
//! the check decorative — which is why there is deliberately **no**
//! `stamp_bytes(&[u8])` on this type's public surface for arbitrary callers to
//! reach for.
//!
//! # What this is not
//!
//! It is not the Turnkey policy engine, and it does not replace it. Turnkey
//! evaluates its own policies inside an enclave before producing a signature,
//! which is the reason ADR 0011 chose it — two independent enforcements of the
//! same rule. This side is Radar's half, and it refuses first.

use radar_risk::{Authorization, Policy};
use radar_types::{Address, Slot};

use crate::verify::{Allowlist, Rejection};

/// Turnkey's stamp scheme name, as its API expects it verbatim.
const SCHEME: &str = "SIGNATURE_SCHEME_TK_API_P256";

/// The header a stamped request carries.
pub const STAMP_HEADER: &str = "X-Stamp";

/// Why a request was not stamped.
///
/// Every variant is a refusal, and none of them produces a stamp.
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum NotStamped {
    /// The key material is not a P-256 private key.
    ///
    /// Deliberately unspecific, for the reason [`crate::privy`] gives: a message
    /// distinguishing "wrong curve" from "wrong length" tells an attacker with a
    /// log more than it tells an operator.
    #[error("the API key could not be loaded")]
    KeyFailed(String),
    /// The body carries no transaction to check.
    #[error("the request body names no transaction")]
    NoTransaction,
    /// The transaction is not valid base64.
    #[error("the transaction is not base64: {0}")]
    NotBase64(String),
    /// The risk kernel's bounds do not cover this transaction.
    #[error("refused: {}", .0.join("; "))]
    Refused(Vec<String>),
    /// The signature itself could not be produced.
    #[error("signing failed")]
    SigningFailed(String),
}

/// The P-256 key registered as an API user on the Turnkey organisation.
///
/// No `Debug`, no `Display`, no accessor for the private bytes. The only thing
/// that can be done with it is [`stamp`], which will not act without a kernel
/// authorisation.
pub struct ApiKey {
    pair: ring::signature::EcdsaKeyPair,
    public_key_hex: String,
}

impl ApiKey {
    /// Loads a key from PKCS#8, PEM or bare base64.
    ///
    /// `public_key_hex` is the compressed P-256 public key Turnkey shows for the
    /// API user. It is carried rather than derived because the stamp must name
    /// the key Turnkey has on file, and a locally derived value that disagreed
    /// with the dashboard would fail at the API with a message about
    /// authentication rather than about configuration.
    ///
    /// # Errors
    ///
    /// [`NotStamped::KeyFailed`] when the material is not a P-256 private key.
    pub fn parse(material: &str, public_key_hex: &str) -> Result<Self, NotStamped> {
        let trimmed = material.trim();
        let body: String = trimmed
            .lines()
            .filter(|line| !line.starts_with("-----"))
            .collect::<Vec<_>>()
            .join("");
        let der = radar_types::b64::decode(body.trim()).ok_or_else(|| {
            NotStamped::KeyFailed("the key material is not valid base64".to_owned())
        })?;
        // ASN.1/DER signatures, because that is what Turnkey verifies. `FIXED`
        // would produce a signature of the right length that never validates --
        // the same trap `privy::AuthorizationKey::parse` documents.
        ring::signature::EcdsaKeyPair::from_pkcs8(
            &ring::signature::ECDSA_P256_SHA256_ASN1_SIGNING,
            &der,
            &ring::rand::SystemRandom::new(),
        )
        .map(|pair| Self {
            pair,
            public_key_hex: public_key_hex.trim().to_owned(),
        })
        .map_err(|e| NotStamped::KeyFailed(e.to_string()))
    }

    /// The public half, as Turnkey names it. Safe to log.
    #[must_use]
    pub fn public_key_hex(&self) -> &str {
        &self.public_key_hex
    }
}

/// Hex, lower case, as Turnkey encodes a stamp's signature.
fn hex(bytes: &[u8]) -> String {
    use core::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

/// Everything the check needs, grouped because they travel together and are
/// meaningless apart.
///
/// A struct rather than five parameters: the two that matter most --
/// `authorization` and `policy` -- are both "bounds", and a call site that
/// transposed them would still compile if they were bare arguments of similar
/// shape.
pub struct Bounds<'a> {
    /// What the risk kernel permitted.
    pub authorization: &'a Authorization,
    /// The wallet expected to sign.
    pub signing_wallet: &'a Address,
    /// Programs a signed transaction may invoke.
    pub allowlist: &'a Allowlist,
    /// **The signer's own** policy, not the caller's (ADR 0008).
    pub policy: &'a Policy,
    /// The caller's view of the chain head.
    pub now: Slot,
}

/// Produces the `X-Stamp` value for `body`, after checking the transaction it
/// carries against the kernel's authorisation.
///
/// `transaction_base64` is read from the body by the caller that built it, and
/// **must be the same string the body contains** — see the module docs for why
/// this function does not accept bytes on their own.
///
/// # Errors
///
/// [`NotStamped`] whenever the transaction is absent, undecodable, or outside
/// the authorisation. Every one is a refusal; none produces a stamp.
pub fn stamp(
    key: &ApiKey,
    body: &str,
    transaction_base64: Option<&str>,
    bounds: &Bounds<'_>,
) -> Result<String, NotStamped> {
    let encoded = transaction_base64.ok_or(NotStamped::NoTransaction)?;
    // The transaction must actually be in the body being stamped. Without this,
    // a caller could present a benign transaction for checking and stamp a body
    // containing a different one, which is precisely the substitution the
    // check-then-sign ordering exists to prevent.
    if !body.contains(encoded) {
        return Err(NotStamped::NoTransaction);
    }
    let bytes = radar_types::b64::decode(encoded)
        .ok_or_else(|| NotStamped::NotBase64("the transaction is not valid base64".to_owned()))?;

    crate::verify::check(
        bounds.authorization,
        &bytes,
        bounds.signing_wallet,
        bounds.allowlist,
        bounds.policy,
        bounds.now,
    )
    .map_err(|rejections| {
        NotStamped::Refused(rejections.iter().map(Rejection::to_string).collect())
    })?;

    let signature = key
        .pair
        .sign(&ring::rand::SystemRandom::new(), body.as_bytes())
        .map_err(|e| NotStamped::SigningFailed(e.to_string()))?;

    // Turnkey's stamp is base64url of this JSON object, unpadded.
    Ok(envelope(key, signature.as_ref()))
}

/// The only path prefix a read may be stamped for.
///
/// Turnkey separates reads (`/query/`) from state changes (`/submit/`), and
/// that separation is what makes [`stamp_query`] safe to exist.
pub const QUERY_PREFIX: &str = "/public/v1/query/";

/// Stamps a **read**, which carries no transaction to check.
///
/// This is the one entry point that signs bytes without running
/// [`crate::verify::check`], and it is narrow on purpose. A general
/// "stamp these bytes" function is exactly what this module refuses to offer:
/// it would let a caller stamp a signing request that nothing had checked,
/// which is the whole failure [`stamp`] is shaped to prevent.
///
/// So the safety here is **structural, not advisory**. The path is an argument
/// and anything outside [`QUERY_PREFIX`] is refused, so this function cannot be
/// pointed at `/submit/` however it is called. Turnkey's own API is what makes
/// that a real boundary rather than a naming convention: reads and state
/// changes live under different prefixes.
///
/// # Errors
///
/// [`NotStamped::NoTransaction`] when the path is not a query — reusing that
/// variant because the condition is the same one it always means: this request
/// was not checked, so it is not stamped.
pub fn stamp_query(key: &ApiKey, path: &str, body: &str) -> Result<String, NotStamped> {
    if !path.starts_with(QUERY_PREFIX) {
        return Err(NotStamped::NoTransaction);
    }
    let signature = key
        .pair
        .sign(&ring::rand::SystemRandom::new(), body.as_bytes())
        .map_err(|e| NotStamped::SigningFailed(e.to_string()))?;
    Ok(envelope(key, signature.as_ref()))
}

/// The base64url stamp envelope Turnkey expects.
fn envelope(key: &ApiKey, signature: &[u8]) -> String {
    let stamp = serde_json::json!({
        "publicKey": key.public_key_hex,
        "scheme": SCHEME,
        "signature": hex(signature),
    });
    radar_types::b64::encode_url(stamp.to_string().as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verify::SYSTEM_PROGRAM;
    use radar_risk::{Action, Autonomy, MicroUsd};
    use radar_types::SlotDelta;
    use ring::signature::KeyPair as _;

    const DEX: [u8; 32] = [0x11; 32];
    const MINT: [u8; 32] = [0x22; 32];
    const WALLET: [u8; 32] = [0x33; 32];

    /// A freshly generated P-256 key, so no key material is committed.
    fn key() -> ApiKey {
        let rng = ring::rand::SystemRandom::new();
        let der = ring::signature::EcdsaKeyPair::generate_pkcs8(
            &ring::signature::ECDSA_P256_SHA256_ASN1_SIGNING,
            &rng,
        )
        .expect("a key");
        ApiKey::parse(&radar_types::b64::encode(der.as_ref()), "02deadbeef").expect("parses")
    }

    /// A transaction that names the mint and calls only allowed programs.
    fn transaction() -> Vec<u8> {
        let mut bytes = vec![0u8, 1, 0, 0, 4];
        for a in [WALLET, MINT, DEX, SYSTEM_PROGRAM] {
            bytes.extend_from_slice(&a);
        }
        bytes.extend_from_slice(&[0xAA; 32]);
        bytes.extend_from_slice(&[1, 2, 2, 0, 1, 2, 0xAB, 0xCD]);
        bytes
    }

    fn body_with(encoded: &str) -> String {
        format!(r#"{{"type":"ACTIVITY_TYPE_SIGN_TRANSACTION","unsignedTransaction":"{encoded}"}}"#)
    }

    fn authorization() -> Authorization {
        Authorization {
            nonce: "turnkey".to_owned(),
            mint: Address::new(MINT),
            action: Action::Buy,
            max_notional: MicroUsd(50_000_000),
            expires_after: Slot(1_150),
            needs_operator_signature: false,
        }
    }

    fn allowlist() -> Allowlist {
        Allowlist {
            programs: vec![DEX, SYSTEM_PROGRAM],
        }
    }

    fn permissive() -> Policy {
        Policy {
            autonomy: Autonomy::Capped,
            max_position: MicroUsd(1_000_000_000),
            max_canary: MicroUsd(1_000_000_000),
            max_input_staleness: SlotDelta(100_000),
            ..Policy::CLOSED
        }
    }

    fn stamp_ok() -> String {
        let encoded = radar_types::b64::encode(&transaction());
        let body = body_with(&encoded);
        stamp(
            &key(),
            &body,
            Some(&encoded),
            &Bounds {
                authorization: &authorization(),
                signing_wallet: &Address::new(WALLET),
                allowlist: &allowlist(),
                policy: &permissive(),
                now: Slot(1_000),
            },
        )
        .expect("a stamp")
    }

    #[test]
    fn a_stamp_is_unpadded_base64url_of_the_documented_json() {
        let decoded = radar_types::b64::decode_url(&stamp_ok()).expect("url-safe base64");
        let value: serde_json::Value = serde_json::from_slice(&decoded).expect("the stamp is JSON");
        assert_eq!(value["scheme"], SCHEME);
        assert_eq!(value["publicKey"], "02deadbeef");
        let signature = value["signature"].as_str().expect("a signature");
        assert!(
            signature.chars().all(|c| c.is_ascii_hexdigit()),
            "lower-case hex, not base64: {signature}"
        );
        // DER-encoded ECDSA, so it starts with a SEQUENCE tag. A `FIXED`
        // signature would be 64 bytes of raw r||s and never validate -- the
        // failure mode this crate has already hit once with Privy.
        assert!(signature.starts_with("30"), "ASN.1 DER: {signature}");
    }

    #[test]
    fn the_signature_covers_the_body_that_will_be_sent() {
        // The property the whole file exists for. If the signature covered
        // anything other than the exact bytes transmitted, Turnkey would be
        // authenticating a request nobody checked.
        let api = key();
        let encoded = radar_types::b64::encode(&transaction());
        let body = body_with(&encoded);
        let stamped = stamp(
            &api,
            &body,
            Some(&encoded),
            &Bounds {
                authorization: &authorization(),
                signing_wallet: &Address::new(WALLET),
                allowlist: &allowlist(),
                policy: &permissive(),
                now: Slot(1_000),
            },
        )
        .expect("a stamp");

        let decoded = radar_types::b64::decode_url(&stamped).expect("base64url");
        let value: serde_json::Value = serde_json::from_slice(&decoded).expect("JSON");
        let signature_hex = value["signature"].as_str().expect("a signature");
        let signature: Vec<u8> = (0..signature_hex.len() / 2)
            .map(|i| u8::from_str_radix(&signature_hex[i * 2..i * 2 + 2], 16).expect("hex"))
            .collect();

        let public = api.pair.public_key().as_ref().to_vec();
        let verifier = ring::signature::UnparsedPublicKey::new(
            &ring::signature::ECDSA_P256_SHA256_ASN1,
            &public,
        );
        assert!(
            verifier.verify(body.as_bytes(), &signature).is_ok(),
            "the stamp must verify against the body"
        );
        // And must not verify against a different body.
        assert!(
            verifier.verify(b"{}", &signature).is_err(),
            "a signature that verifies against anything is not a signature"
        );
    }

    #[test]
    fn a_transaction_outside_the_authorisation_is_refused_rather_than_stamped() {
        // The check that makes this worth having. An executor bug that swapped
        // the mint would otherwise present a valid authorisation for a request
        // spending on something else.
        let encoded = radar_types::b64::encode(&transaction());
        let body = body_with(&encoded);
        let mut elsewhere = authorization();
        elsewhere.mint = Address::new([0x77; 32]);
        let refused = stamp(
            &key(),
            &body,
            Some(&encoded),
            &Bounds {
                authorization: &elsewhere,
                signing_wallet: &Address::new(WALLET),
                allowlist: &allowlist(),
                policy: &permissive(),
                now: Slot(1_000),
            },
        );
        assert!(matches!(refused, Err(NotStamped::Refused(_))));
    }

    #[test]
    fn the_shipped_policy_refuses_to_stamp_anything() {
        // `Policy::SHIPPED` is `CLOSED`. Closed at the credential, not merely in
        // the process that decides.
        let encoded = radar_types::b64::encode(&transaction());
        let body = body_with(&encoded);
        let refused = stamp(
            &key(),
            &body,
            Some(&encoded),
            &Bounds {
                authorization: &authorization(),
                signing_wallet: &Address::new(WALLET),
                allowlist: &allowlist(),
                policy: &Policy::SHIPPED,
                now: Slot(1_000),
            },
        );
        assert!(matches!(refused, Err(NotStamped::Refused(_))));
    }

    #[test]
    fn a_transaction_that_is_not_in_the_body_is_refused() {
        // The substitution this ordering exists to prevent: present a benign
        // transaction for checking, stamp a body containing a different one.
        // Without the containment check the stamp would be produced happily.
        let checked = radar_types::b64::encode(&transaction());
        let body = body_with("c29tZXRoaW5nIGVsc2U");
        let refused = stamp(
            &key(),
            &body,
            Some(&checked),
            &Bounds {
                authorization: &authorization(),
                signing_wallet: &Address::new(WALLET),
                allowlist: &allowlist(),
                policy: &permissive(),
                now: Slot(1_000),
            },
        );
        assert!(
            matches!(refused, Err(NotStamped::NoTransaction)),
            "a transaction absent from the body must not be stamped: {refused:?}"
        );
    }

    #[test]
    fn a_body_with_no_transaction_is_refused() {
        let refused = stamp(
            &key(),
            "{}",
            None,
            &Bounds {
                authorization: &authorization(),
                signing_wallet: &Address::new(WALLET),
                allowlist: &allowlist(),
                policy: &permissive(),
                now: Slot(1_000),
            },
        );
        assert!(matches!(refused, Err(NotStamped::NoTransaction)));
    }

    #[test]
    fn key_material_that_is_not_a_p256_key_is_refused() {
        assert!(matches!(
            ApiKey::parse("not base64 at all !!", "02"),
            Err(NotStamped::KeyFailed(_))
        ));
        assert!(matches!(
            ApiKey::parse(&radar_types::b64::encode(b"too short"), "02"),
            Err(NotStamped::KeyFailed(_))
        ));
    }

    #[test]
    fn a_read_may_be_stamped_but_only_on_a_query_path() {
        // The structural half of the guarantee. `stamp_query` is the only
        // function here that signs without checking a transaction, so the thing
        // that matters is that it cannot be aimed at a submission.
        let api = key();
        let body = r#"{"organizationId":"org"}"#;
        assert!(stamp_query(&api, "/public/v1/query/whoami", body).is_ok());

        for path in [
            "/public/v1/submit/sign_transaction",
            "/public/v1/submit/create_policy",
            "/",
            "",
            // A query prefix that is not at the start must not pass either.
            "/evil?next=/public/v1/query/whoami",
        ] {
            assert!(
                matches!(
                    stamp_query(&api, path, body),
                    Err(NotStamped::NoTransaction)
                ),
                "a read stamp must be refused for {path}"
            );
        }
    }

    #[test]
    fn a_read_stamp_and_a_checked_stamp_share_one_envelope() {
        // Two producers of the same header would be two things that can drift,
        // and a stamp Turnkey rejects fails as an authentication error rather
        // than as the encoding bug it is.
        let api = key();
        let read = stamp_query(&api, "/public/v1/query/whoami", "{}").expect("a stamp");
        let decoded = radar_types::b64::decode_url(&read).expect("base64url");
        let value: serde_json::Value = serde_json::from_slice(&decoded).expect("JSON");
        assert_eq!(value["scheme"], SCHEME);
        assert_eq!(value["publicKey"], "02deadbeef");
        assert!(
            value["signature"].as_str().expect("hex").starts_with("30"),
            "ASN.1 DER, the same as a checked stamp"
        );
    }
}
