// SPDX-License-Identifier: Apache-2.0
//! A customer token opens the product and nothing else.
//!
//! From outside the crate, through the real router, because the property is
//! about the **guard** rather than about any type it consults.
//!
//! `access.rs`'s unit tests prove `Audience::accepts_customer` answers correctly
//! for each path. That is not the same as the guard asking it. Widening the
//! guard's condition from `audience.accepts_customer()` to `!audience.is_open()`
//! compiles, passes every unit test, and hands `/v1/store` and `/mcp` to anyone
//! holding a valid Privy token — which is the whole customer base.
//!
//! So these tests present a token that **really verifies**. An invalid one is
//! refused everywhere and proves nothing about the routing; the only way to see
//! the boundary is to have something that would open the door if the door were
//! wrong.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use radar_instruments::{CreatorHistory, Registry};
use radar_serve::customer::{Config, Jwk, KeyCache, Keys, Mode};
use radar_serve::{AppState, app};
use radar_store::Reader;
use ring::rand::SystemRandom;
use ring::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair};
use tower::ServiceExt;

const APP: &str = "cmthhkznr0a3u0cl86prxlb7x";

fn b64(bytes: &[u8]) -> String {
    radar_types::b64::encode(bytes)
        .replace('+', "-")
        .replace('/', "_")
        .trim_end_matches('=')
        .to_owned()
}

/// A signed token, and the key set that would publish its key.
fn valid_token() -> (String, Keys) {
    let rng = SystemRandom::new();
    let pkcs8 =
        EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).expect("a key pair");
    let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng)
        .expect("parses");
    let point = pair.public_key().as_ref().to_vec();
    let keys = Keys(vec![Jwk {
        kid: "test".to_owned(),
        crv: "P-256".to_owned(),
        x: b64(&point[1..33]),
        y: b64(&point[33..65]),
    }]);

    // Far enough ahead that this file does not start failing on a Tuesday.
    let exp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("a clock after 1970")
        .as_secs()
        + 3_600;
    let claims =
        format!(r#"{{"iss":"privy.io","aud":"{APP}","sub":"did:privy:x","sid":"s","exp":{exp}}}"#);
    let signing_input = format!(
        "{}.{}",
        b64(br#"{"alg":"ES256","kid":"test"}"#),
        b64(claims.as_bytes())
    );
    let signature = pair
        .sign(&SystemRandom::new(), signing_input.as_bytes())
        .expect("signs");
    (format!("{signing_input}.{}", b64(signature.as_ref())), keys)
}

/// A router with a customer lane and an operator lane that nothing can satisfy.
///
/// The operator domain is unreachable **on purpose**, and `Mode::Off` would be
/// wrong here: `Off` skips the operator check entirely and serves every route to
/// everyone, so a 200 would prove nothing about the customer token. Enforcing
/// against a domain no token can match means an operator route refuses unless
/// something else opened it — which is exactly the failure being looked for.
fn router(keys: Keys) -> axum::Router {
    let mut registry = Registry::new();
    registry.register(CreatorHistory);
    app(Arc::new(AppState {
        admission: radar_serve::admission::Admission::Open,
        shares: radar_serve::share::Shares::new(radar_serve::share::Allowance::per_day(100)),
        customer_salt: vec![7u8; 32],
        registry,
        store: Reader::open(std::env::temp_dir().join("radar-customer-guard-test")),
        x402: None,
        chat: None,
        access: radar_serve::access::Mode::Enforce(radar_serve::access::Config {
            team_domain: "radar-test.invalid".to_owned(),
            aud: "radar-aud-tag".to_owned(),
        }),
        keys: radar_serve::access::KeyCache::new(),
        customer: Mode::Enforce(Config {
            app_id: APP.to_owned(),
        }),
        customer_keys: KeyCache::preloaded(keys),
        linker: radar_serve::link::Linker::new(),
        challenges: None,
        privy: None,
    }))
}

async fn status_with(router: axum::Router, path: &str, token: &str) -> StatusCode {
    router
        .oneshot(
            Request::builder()
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .expect("a well-formed request"),
        )
        .await
        .expect("the router answers")
        .status()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_valid_customer_token_never_opens_an_operator_route() {
    // The test this file exists for.
    //
    // The operator lane enforces against a domain no token can match, so these
    // refuse unless the customer token opened them. Every one is the operator's
    // debugging surface: store counts, a raw event stream, the machine
    // interface, and the instrument registry.
    let (token, keys) = valid_token();
    for path in ["/ops", "/v1/store", "/v1/events", "/mcp", "/v1/instruments"] {
        let status = status_with(router(keys.clone()), path, &token).await;
        assert_ne!(
            status,
            StatusCode::OK,
            "{path} answered a customer token — it is the operator's surface"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_valid_customer_token_opens_the_product() {
    // The other half, and it has to hold too. A boundary that refuses everything
    // is not a boundary, and a test suite that only ever asserts refusals would
    // pass with the customer lane wired to nothing at all.
    let (token, keys) = valid_token();
    for path in ["/v1/funnel", "/v1/scoreboard"] {
        let status = status_with(router(keys.clone()), path, &token).await;
        assert_ne!(
            status,
            StatusCode::FORBIDDEN,
            "{path} refused a valid customer token — it is the product"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_forged_customer_token_opens_nothing() {
    // A token signed by a key the set does not publish. The signature is real
    // and the claims are perfect; it is simply not Privy's.
    let (_, published) = valid_token();
    let (forged, _) = valid_token();
    for path in ["/v1/funnel", "/v1/store"] {
        let status = status_with(router(published.clone()), path, &forged).await;
        assert_ne!(
            status,
            StatusCode::OK,
            "{path} accepted a token signed by an unpublished key"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_wallet_route_refuses_a_request_carrying_no_customer_identity() {
    // An operator can reach `/v1/customer/wallet` -- `Audience::Customer`
    // accepts operator identity, because debugging a customer's problem requires
    // it. But an operator is not a customer, and there is no DID on that
    // request.
    //
    // The handler must refuse rather than guess. The two ways it could guess are
    // both catastrophic: reading a DID from the query string would let anyone
    // read anyone's wallet, and defaulting to some "current" customer would be
    // worse. So the only source is the extension the guard inserts after a
    // successful verification, and its absence is a refusal.
    //
    // `Mode::Off` here opens the operator lane deliberately: it is the state in
    // which the request definitely reaches the handler, which is the state this
    // test is about.
    let (_, keys) = valid_token();
    let mut registry = Registry::new();
    registry.register(CreatorHistory);
    let router = app(Arc::new(AppState {
        admission: radar_serve::admission::Admission::Open,
        shares: radar_serve::share::Shares::new(radar_serve::share::Allowance::per_day(100)),
        customer_salt: vec![7u8; 32],
        registry,
        store: Reader::open(std::env::temp_dir().join("radar-wallet-route-test")),
        x402: None,
        chat: None,
        access: radar_serve::access::Mode::Off,
        keys: radar_serve::access::KeyCache::new(),
        customer: Mode::Enforce(Config {
            app_id: APP.to_owned(),
        }),
        customer_keys: KeyCache::preloaded(keys),
        linker: radar_serve::link::Linker::new(),
        challenges: None,
        privy: None,
    }));

    let status = router
        .oneshot(
            Request::builder()
                .uri("/v1/customer/wallet")
                .body(Body::empty())
                .expect("a well-formed request"),
        )
        .await
        .expect("the router answers")
        .status();

    assert_ne!(
        status,
        StatusCode::OK,
        "a request with no verified customer must not receive a wallet"
    );
}
