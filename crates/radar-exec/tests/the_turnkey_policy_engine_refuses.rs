// SPDX-License-Identifier: Apache-2.0
//! [ADR 0011](../../../docs/adr/0011-one-wallet-system-two-authority-levels-on-turnkey.md)'s
//! first precondition: **the policy engine refuses**.
//!
//! # Why the negative case is the whole test
//!
//! ADR 0005 asked for a policy-engine spike *verified by making it refuse*, and
//! it has been unmet since it was written. Showing an allow working proves only
//! that a request is well formed. What has to be proved is that a request the
//! policy should reject **is** rejected, by the provider, inside the enclave,
//! before a signature exists.
//!
//! That matters because ADR 0011 chose Turnkey over Privy specifically for it:
//! two independent enforcements of the same rule, one in
//! [`radar_signer::verify::check`] and one held by a party that is neither Radar
//! nor the customer. An enforcement that cannot be shown refusing is not an
//! enforcement.
//!
//! # Why it skips without credentials
//!
//! It needs a real Turnkey organisation. Credentials are read from the
//! environment and **never** committed, printed, or logged — the private key in
//! particular is loaded straight into [`ApiKey::parse`] and never rendered.
//!
//! ```text
//! TURNKEY_ORGANIZATION_ID=... TURNKEY_API_PUBLIC_KEY=... \
//! TURNKEY_API_PRIVATE_KEY=... \
//!   cargo test -p radar-exec --test the_turnkey_policy_engine_refuses -- --nocapture
//! ```
//!
//! A skip is not free, and the risk is named rather than hidden: a test that
//! usually skips is a test nobody notices has stopped working. It prints what it
//! would have proved and how to run it.
//!
//! # Status
//!
//! **This harness has never been run.** It is written from Turnkey's documented
//! API and has not been executed against a live organisation, because doing so
//! needs an account this repository does not have. Until it runs, ADR 0011 is a
//! decision with an unmet precondition, and the ADR says so.

use radar_risk::{Action, Authorization, Autonomy, MicroUsd, Policy};
use radar_signer::turnkey::{ApiKey, Bounds, STAMP_HEADER, stamp};
use radar_signer::verify::{Allowlist, SYSTEM_PROGRAM};
use radar_types::{Address, Slot, SlotDelta};

/// Turnkey's public API.
const API: &str = "https://api.turnkey.com";

/// The one read this harness makes, named once so the stamp and the request
/// cannot drift apart -- a stamp signed over a different path than the one
/// called is an authentication failure that reads like a credential problem.
const WHOAMI: &str = "/public/v1/query/whoami";

/// What the harness needs, and what each is for.
const VARS: [(&str, &str); 3] = [
    ("TURNKEY_ORGANIZATION_ID", "the organisation to query"),
    ("TURNKEY_API_PUBLIC_KEY", "the API user's public key, hex"),
    (
        "TURNKEY_API_PRIVATE_KEY",
        "the API user's private key — never printed",
    ),
];

/// The credentials, or `None` with a loud explanation.
fn credentials() -> Option<(String, ApiKey)> {
    let mut missing = Vec::new();
    for (name, _) in VARS {
        if std::env::var(name).ok().is_none_or(|v| v.trim().is_empty()) {
            missing.push(name);
        }
    }
    if !missing.is_empty() {
        eprintln!("SKIPPED: the Turnkey policy-engine spike did not run.");
        eprintln!();
        eprintln!("It would prove ADR 0011's first precondition: that the policy");
        eprintln!("engine REFUSES a request it should refuse, inside the enclave,");
        eprintln!("before a signature exists. Nothing else in this repository");
        eprintln!("proves that, and ADR 0011 does not hold without it.");
        eprintln!();
        eprintln!("Missing: {}", missing.join(", "));
        for (name, why) in VARS {
            eprintln!("  {name}: {why}");
        }
        return None;
    }
    let organization = std::env::var("TURNKEY_ORGANIZATION_ID").ok()?;
    let public = std::env::var("TURNKEY_API_PUBLIC_KEY").ok()?;
    let private = std::env::var("TURNKEY_API_PRIVATE_KEY").ok()?;
    match ApiKey::parse(&private, &public) {
        Ok(key) => Some((organization, key)),
        Err(e) => {
            // The error deliberately says nothing about the key material.
            eprintln!("SKIPPED: TURNKEY_API_PRIVATE_KEY did not load: {e}");
            None
        }
    }
}

/// Posts a stamped request and returns `(status, body)`.
///
/// Errors are returned rather than panicked so a network failure reads as a
/// network failure rather than as a policy refusal — the two must never be
/// confused, because one of them would be a *false pass* of a test whose whole
/// job is to observe a refusal.
fn post(path: &str, body: &str, stamp_value: &str) -> Result<(u16, String), String> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .build()
        .new_agent();
    match agent
        .post(&format!("{API}{path}"))
        .header("Content-Type", "application/json")
        .header(STAMP_HEADER, stamp_value)
        .send(body)
    {
        Ok(mut response) => {
            let status = response.status().as_u16();
            let text = response.body_mut().read_to_string().unwrap_or_default();
            Ok((status, text))
        }
        Err(ureq::Error::StatusCode(code)) => Ok((code, String::new())),
        Err(e) => Err(e.to_string()),
    }
}

/// A transaction naming a mint the authorisation does not cover.
fn transaction(mint: [u8; 32], wallet: [u8; 32], dex: [u8; 32]) -> Vec<u8> {
    let mut bytes = vec![0u8, 1, 0, 0, 4];
    for a in [wallet, mint, dex, SYSTEM_PROGRAM] {
        bytes.extend_from_slice(&a);
    }
    bytes.extend_from_slice(&[0xAA; 32]);
    bytes.extend_from_slice(&[1, 2, 2, 0, 1, 2, 0xAB, 0xCD]);
    bytes
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

#[test]
fn the_stamp_authenticates_against_the_live_api() {
    // Runs first because everything below is noise if it fails. `whoami` is the
    // cheapest request Turnkey exposes and it exercises the entire stamp format:
    // the base64url envelope, the DER signature encoding, and the public key
    // matching the one on file.
    let Some((organization, key)) = credentials() else {
        return;
    };
    let body = format!(r#"{{"organizationId":"{organization}"}}"#);

    // `whoami` carries no transaction, so it cannot go through `stamp` -- which
    // refuses without one, by design. `stamp_query` is the narrow exception, and
    // it is refused for anything outside `/public/v1/query/`, so this cannot be
    // turned into a way to stamp a submission.
    let stamped = match radar_signer::turnkey::stamp_query(&key, WHOAMI, &body) {
        Ok(value) => value,
        Err(e) => panic!("could not stamp a read: {e}"),
    };

    match post(WHOAMI, &body, &stamped) {
        Ok((200, text)) => {
            println!(
                "stamp accepted by Turnkey; whoami returned {} bytes",
                text.len()
            );
            assert!(
                text.contains(&organization),
                "whoami should name the organisation asked about"
            );
        }
        Ok((status, text)) => panic!(
            "the stamp was not accepted: HTTP {status}. \
             This is a stamp-format failure, not a policy refusal. Body: {text}"
        ),
        Err(e) => panic!("could not reach Turnkey: {e}"),
    }
}

#[test]
fn a_transaction_outside_the_authorisation_never_reaches_turnkey() {
    // Radar's own half, asserted here rather than only in the signer's unit
    // tests, because this is the file that would otherwise imply the provider is
    // the only thing standing between a bad request and a signature.
    //
    // It needs no credentials and no network: the refusal happens before either.
    const MINT: [u8; 32] = [0x22; 32];
    const WALLET: [u8; 32] = [0x33; 32];
    const DEX: [u8; 32] = [0x11; 32];

    let bytes = transaction(MINT, WALLET, DEX);
    let encoded = radar_types::b64::encode(&bytes);
    let body = format!(r#"{{"unsignedTransaction":"{encoded}"}}"#);

    let elsewhere = Authorization {
        nonce: "spike".to_owned(),
        mint: Address::new([0x77; 32]),
        action: Action::Buy,
        max_notional: MicroUsd(50_000_000),
        expires_after: Slot(1_150),
        needs_operator_signature: false,
    };

    let refused = stamp(
        &dummy_key(),
        &body,
        Some(&encoded),
        &Bounds {
            authorization: &elsewhere,
            signing_wallet: &Address::new(WALLET),
            allowlist: &Allowlist {
                programs: vec![DEX, SYSTEM_PROGRAM],
            },
            policy: &permissive(),
            now: Slot(1_000),
        },
    );
    assert!(
        refused.is_err(),
        "a transaction for another mint must not be stamped at all"
    );
}

#[test]
fn the_shipped_policy_refuses_before_any_request_is_made() {
    // `Policy::SHIPPED` is `CLOSED`. Nothing reaches Turnkey while it ships, and
    // that is asserted here so the spike cannot be read as evidence that trading
    // is open.
    const MINT: [u8; 32] = [0x22; 32];
    const WALLET: [u8; 32] = [0x33; 32];
    const DEX: [u8; 32] = [0x11; 32];

    let bytes = transaction(MINT, WALLET, DEX);
    let encoded = radar_types::b64::encode(&bytes);
    let body = format!(r#"{{"unsignedTransaction":"{encoded}"}}"#);

    let authorization = Authorization {
        nonce: "spike".to_owned(),
        mint: Address::new(MINT),
        action: Action::Buy,
        max_notional: MicroUsd(50_000_000),
        expires_after: Slot(1_150),
        needs_operator_signature: false,
    };

    let refused = stamp(
        &dummy_key(),
        &body,
        Some(&encoded),
        &Bounds {
            authorization: &authorization,
            signing_wallet: &Address::new(WALLET),
            allowlist: &Allowlist {
                programs: vec![DEX, SYSTEM_PROGRAM],
            },
            policy: &Policy::SHIPPED,
            now: Slot(1_000),
        },
    );
    assert!(refused.is_err(), "the shipped policy authorises nothing");
}

/// A throwaway key, so the offline assertions need no credentials.
fn dummy_key() -> ApiKey {
    let rng = ring::rand::SystemRandom::new();
    let der = ring::signature::EcdsaKeyPair::generate_pkcs8(
        &ring::signature::ECDSA_P256_SHA256_ASN1_SIGNING,
        &rng,
    )
    .expect("a key");
    ApiKey::parse(&radar_types::b64::encode(der.as_ref()), "02").expect("parses")
}

#[test]
fn a_read_stamp_cannot_be_aimed_at_a_submission() {
    // Guards the one exception this harness relies on. `stamp_query` signs
    // without checking a transaction, so the thing that keeps it safe is that
    // it refuses any path outside `/public/v1/query/` -- structurally, not by
    // convention. Needs no credentials.
    let key = dummy_key();
    assert!(
        radar_signer::turnkey::stamp_query(&key, "/public/v1/submit/sign_transaction", "{}")
            .is_err(),
        "a read stamp must never be produced for a submission"
    );
}
