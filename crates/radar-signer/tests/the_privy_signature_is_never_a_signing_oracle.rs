// SPDX-License-Identifier: Apache-2.0
//! The Privy authorization key signs a request the kernel authorised, or it
//! signs nothing.
//!
//! [ADR 0007](../../../docs/adr/0007-the-privy-authorization-key-lives-in-the-signer-process.md)
//! puts this key in the signer rather than in `radar-serve`, on the grounds that
//! a signature made with it causes a customer's wallet to move funds. That is
//! only worth doing if the signer actually checks what it is signing for — a key
//! in a small process that signs whatever it is handed is a signing oracle in a
//! small process.
//!
//! So these tests are about the *order* and the *provenance*: the bounds are
//! checked before anything is signed, and they are checked against the bytes
//! inside the request that will be sent, not against a copy passed alongside it.

use radar_risk::{Action, Address, Authorization, Autonomy, MicroUsd, Policy, Slot};
use radar_signer::privy::{AuthorizationKey, NotAuthorised, PrivyRequest, authorise};
use radar_signer::verify::Allowlist;
use serde_json::{Value, json};

const SYSTEM_PROGRAM: [u8; 32] = [0u8; 32];
const DEX: [u8; 32] = [0x11; 32];
const MINT: [u8; 32] = [0x22; 32];
const WALLET: [u8; 32] = [0x33; 32];
const NOW: Slot = Slot(1_000);

/// A policy wide enough not to be what any of these tests are about.
///
/// ADR 0008's clamp is exercised in `verify.rs`. Here it must be out of the way,
/// or a refusal could come from the policy rather than from the property being
/// tested -- which is how a test passes for the wrong reason.
fn policy() -> Policy {
    Policy {
        autonomy: Autonomy::Capped,
        max_position: MicroUsd(1_000_000_000),
        max_canary: MicroUsd(1_000_000_000),
        max_input_staleness: radar_risk::SlotDelta(100_000),
        ..Policy::CLOSED
    }
}

fn allowlist() -> Allowlist {
    Allowlist {
        programs: vec![DEX, SYSTEM_PROGRAM],
    }
}

fn authorization() -> Authorization {
    Authorization {
        nonce: "test".to_owned(),
        mint: Address::new(MINT),
        action: Action::Buy,
        max_notional: MicroUsd(50_000_000),
        expires_after: Slot(1_150),
        needs_operator_signature: false,
    }
}

/// A legacy transaction over the given accounts, fee payer first.
fn build(accounts: &[[u8; 32]]) -> Vec<u8> {
    let mut out = vec![0u8, 1, 0, 0];
    out.push(u8::try_from(accounts.len()).expect("small"));
    for a in accounts {
        out.extend_from_slice(a);
    }
    out.extend_from_slice(&[0xAA; 32]);
    out.push(1);
    out.extend_from_slice(&[2, 2, 0, 1, 2, 0xAB, 0xCD]);
    out
}

/// The transaction the authorisation covers.
fn honest() -> Vec<u8> {
    build(&[WALLET, MINT, DEX, SYSTEM_PROGRAM])
}

/// One for a different token, which the same authorisation must not cover.
fn substituted_mint() -> Vec<u8> {
    build(&[WALLET, [0x99; 32], DEX, SYSTEM_PROGRAM])
}

fn key() -> AuthorizationKey {
    let pkcs8 = ring::signature::EcdsaKeyPair::generate_pkcs8(
        &ring::signature::ECDSA_P256_SHA256_ASN1_SIGNING,
        &ring::rand::SystemRandom::new(),
    )
    .expect("a key pair");
    AuthorizationKey::parse(&radar_types::b64::encode(pkcs8.as_ref())).expect("parses")
}

fn request_carrying(transaction: &[u8]) -> PrivyRequest {
    let mut headers = serde_json::Map::new();
    headers.insert(
        "privy-app-id".to_owned(),
        Value::String("cmthhkznr0a3u0cl86prxlb7x".to_owned()),
    );
    PrivyRequest {
        method: "POST".to_owned(),
        url: "https://api.privy.io/v1/wallets/abc/rpc".to_owned(),
        body: json!({
            "method": "signTransaction",
            "params": {
                "transaction": radar_types::b64::encode(transaction),
                "encoding": "base64",
            },
        }),
        headers,
    }
}

#[test]
fn a_request_the_kernel_authorised_is_signed() {
    // The permitting half, and it has to hold. A gate that refuses everything is
    // not a gate, and a suite that only ever asserts refusals would pass against
    // a function wired to nothing.
    let signature = authorise(
        &key(),
        &request_carrying(&honest()),
        &authorization(),
        &Address::new(WALLET),
        &allowlist(),
        &policy(),
        NOW,
    )
    .expect("an authorised request is signed");

    assert!(
        radar_types::b64::decode(&signature).is_some_and(|d| d.len() > 8),
        "the header value must be base64 of a DER signature: {signature}"
    );
}

#[test]
fn a_transaction_for_another_token_is_refused_rather_than_signed() {
    // The attack this process exists for, in its Privy costume: a caller holding
    // a valid authorisation for one token builds a request for another.
    //
    // Nothing about the request is malformed. The signature would be perfectly
    // valid and Privy would accept it -- which is exactly why the check has to
    // happen here, before the key is used, rather than anywhere downstream.
    let refusal = authorise(
        &key(),
        &request_carrying(&substituted_mint()),
        &authorization(),
        &Address::new(WALLET),
        &allowlist(),
        &policy(),
        NOW,
    )
    .expect_err("a substituted mint must not be signed");

    assert!(
        matches!(refusal, NotAuthorised::Refused(_)),
        "expected a refusal, got {refusal:?}"
    );
}

#[test]
fn the_bytes_checked_are_the_bytes_the_request_carries() {
    // The provenance property, and the reason `authorise` reads the transaction
    // out of the body rather than taking it as an argument.
    //
    // If it took one, a caller could pass the honest transaction for checking
    // and send the substituted one -- and every other test here would still
    // pass. This asserts the two cannot be separated: the same authorisation
    // that signs the honest request refuses the substituted one, and the *only*
    // difference between them is the body.
    let honest_request = request_carrying(&honest());
    let substituted = request_carrying(&substituted_mint());
    assert_eq!(
        honest_request.method, substituted.method,
        "the requests differ only in their body"
    );
    assert_eq!(honest_request.url, substituted.url);

    let signed = authorise(
        &key(),
        &honest_request,
        &authorization(),
        &Address::new(WALLET),
        &allowlist(),
        &policy(),
        NOW,
    );
    let refused = authorise(
        &key(),
        &substituted,
        &authorization(),
        &Address::new(WALLET),
        &allowlist(),
        &policy(),
        NOW,
    );
    assert!(signed.is_ok(), "the honest body signs");
    assert!(refused.is_err(), "the substituted body does not");
}

#[test]
fn an_expired_authorisation_signs_nothing() {
    // `expires_after` is what makes the grant temporary. A signer that ignored
    // it would turn every authorisation into a standing one.
    let refusal = authorise(
        &key(),
        &request_carrying(&honest()),
        &authorization(),
        &Address::new(WALLET),
        &allowlist(),
        &policy(),
        Slot(9_999),
    )
    .expect_err("an expired authorisation must not sign");
    assert!(matches!(refusal, NotAuthorised::Refused(_)));
}

#[test]
fn a_request_with_no_transaction_in_it_is_refused_rather_than_passed() {
    // Rule 9's shape: a transaction that cannot be found is not an absent
    // constraint, it is an unread request. Signing one would authorise bytes
    // nothing inspected -- and this is the single most tempting place to write
    // "no transaction, nothing to check, carry on".
    for body in [
        json!({"method": "signTransaction"}),
        json!({"method": "signTransaction", "params": {}}),
        json!({"method": "signTransaction", "params": {"transaction": 7}}),
    ] {
        let mut request = request_carrying(&honest());
        request.body = body.clone();
        assert!(
            matches!(
                authorise(
                    &key(),
                    &request,
                    &authorization(),
                    &Address::new(WALLET),
                    &allowlist(),
                    &policy(),
                    NOW,
                ),
                Err(NotAuthorised::NoTransaction)
            ),
            "a body with no readable transaction must refuse: {body}"
        );
    }
}

#[test]
fn the_signed_payload_is_the_one_privy_will_rebuild() {
    // The bytes Privy reconstructs on its side. A different key order, a stray
    // space, or a missing field means every signature fails authentication --
    // and the failure would look like a credential problem rather than an
    // encoding one, which is a long afternoon.
    let request = request_carrying(&honest());
    let payload = request.payload().expect("canonicalises");

    assert!(
        payload.starts_with(r#"{"body":{"method":"signTransaction","#),
        "keys sort body < headers < method < url < version: {payload}"
    );
    assert!(
        payload.ends_with(r#""version":1}"#),
        "the version is part of the signed bytes: {payload}"
    );
    assert!(
        payload.contains(r#""headers":{"privy-app-id":"#),
        "the app id header is signed: {payload}"
    );
    assert!(
        !payload.contains(", ") && !payload.contains(": "),
        "canonical JSON has no whitespace separators: {payload}"
    );
}

#[test]
fn the_dashboards_key_prefix_is_accepted() {
    // Privy's dashboard hands out keys prefixed `wallet-auth:`. Requiring an
    // operator to strip it is a step that gets done wrong at three in the
    // morning, and the failure is an unstartable signer.
    let pkcs8 = ring::signature::EcdsaKeyPair::generate_pkcs8(
        &ring::signature::ECDSA_P256_SHA256_ASN1_SIGNING,
        &ring::rand::SystemRandom::new(),
    )
    .expect("a key pair");
    let encoded = radar_types::b64::encode(pkcs8.as_ref());

    assert!(AuthorizationKey::parse(&encoded).is_ok(), "bare base64");
    assert!(
        AuthorizationKey::parse(&format!("wallet-auth:{encoded}")).is_ok(),
        "the prefix the dashboard adds"
    );
    assert!(
        AuthorizationKey::parse(&format!(
            "-----BEGIN PRIVATE KEY-----\n{encoded}\n-----END PRIVATE KEY-----\n"
        ))
        .is_ok(),
        "a PEM block"
    );
}

#[test]
fn material_that_is_not_a_p256_key_is_refused() {
    for material in ["", "not base64 at all !!!", "aGVsbG8gd29ybGQ="] {
        assert!(
            AuthorizationKey::parse(material).is_err(),
            "{material:?} must not load as a key"
        );
    }
}
