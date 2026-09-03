// SPDX-License-Identifier: Apache-2.0
//! That a genuine Privy identity is not, on its own, a customer of this
//! instance.
//!
//! `customer::verify` proves Privy issued a token, for this application,
//! unexpired. Anyone can sign up to a Privy application — so a **verified
//! stranger** is a thing that exists, and before `admission` nothing asked
//! whether they were one of ours. The product was private only because no
//! customer authenticator was configured at all, which is a different property
//! that stops holding on the deploy that configures one.
//!
//! These go through the real router, because the check lives in the guard and a
//! unit test of `Admission::admits` passes whether or not anything calls it.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use radar_serve::admission::Admission;
use radar_serve::customer::{Config, Jwk, KeyCache, Keys, Mode};
use radar_serve::{AppState, app};
use radar_store::Reader;
use ring::rand::SystemRandom;
use ring::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair};
use tower::ServiceExt;

const APP: &str = "cmthhkznr0a3u0cl86prxlb7x";
const JOSH: &str = "did:privy:josh";
const STRANGER: &str = "did:privy:someone-else";

fn b64(bytes: &[u8]) -> String {
    radar_types::b64::encode(bytes)
        .replace('+', "-")
        .replace('/', "_")
        .trim_end_matches('=')
        .to_owned()
}

/// One key pair, and tokens for whichever identities the test needs.
///
/// Both identities are signed by the **same** published key, so every token here
/// verifies. That is the point: the difference between them is admission, not
/// authentication, and a harness where the stranger's token simply failed to
/// verify would prove nothing.
struct Issuer {
    pair: EcdsaKeyPair,
    keys: Keys,
}

impl Issuer {
    fn new() -> Self {
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng)
            .expect("a key pair");
        let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng)
            .expect("parses");
        let point = pair.public_key().as_ref().to_vec();
        let keys = Keys(vec![Jwk {
            kid: "test".to_owned(),
            crv: "P-256".to_owned(),
            x: b64(&point[1..33]),
            y: b64(&point[33..65]),
        }]);
        Self { pair, keys }
    }

    fn token(&self, did: &str) -> String {
        let exp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("after the epoch")
            .as_secs()
            + 3_600;
        let header = br#"{"alg":"ES256","kid":"test"}"#;
        let claims =
            format!(r#"{{"iss":"privy.io","aud":"{APP}","sub":"{did}","sid":"s","exp":{exp}}}"#);
        let input = format!("{}.{}", b64(header), b64(claims.as_bytes()));
        let signature = self
            .pair
            .sign(&SystemRandom::new(), input.as_bytes())
            .expect("signs");
        format!("{input}.{}", b64(signature.as_ref()))
    }
}

fn router(admission: Admission, keys: Keys) -> axum::Router {
    app(Arc::new(AppState {
        admission,
        shares: radar_serve::share::Shares::new(radar_serve::share::Allowance::per_day(100)),
        customer_salt: vec![7u8; 32],
        registry: radar_instruments::Registry::new(),
        store: Reader::open(std::env::temp_dir().join("radar-admission-test")),
        x402: None,
        chat: None,
        // Off, so nothing else can be the reason a request is refused. With
        // Access enforcing, a 403 here could be either check and the test would
        // not know which it had exercised.
        access: radar_serve::access::Mode::Off,
        keys: radar_serve::access::KeyCache::new(),
        customer: Mode::Enforce(Config {
            app_id: APP.to_owned(),
        }),
        customer_keys: KeyCache::preloaded(keys),
        privy: None,
        linker: radar_serve::link::Linker::new(),
        scoreboard: radar_serve::cache::Cache::new(),
        token: radar_serve::cache::Cache::new(),
        challenges: None,
    }))
}

async fn get(router: &axum::Router, token: &str) -> (StatusCode, String) {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/funnel")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .expect("a request"),
        )
        .await
        .expect("a response");
    let status = response.status();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("a body")
        .to_bytes();
    (status, String::from_utf8_lossy(&body).into_owned())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_verified_identity_nobody_allowlisted_is_refused() {
    let issuer = Issuer::new();
    let router = router(
        Admission::from_vars(&|k| {
            (k == radar_serve::admission::VAR).then(|| format!("allowlist:{JOSH}"))
        })
        .expect("parses"),
        issuer.keys.clone(),
    );

    // Past the guard. 503 rather than 200 because the scratch store is empty and
    // the funnel says so -- which is the handler answering, and the handler only
    // runs for a request admission let through. Asserting 200 would need a
    // populated store to say anything this test is about.
    let (status, _) = get(&router, &issuer.token(JOSH)).await;
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "the allowlisted identity reaches the handler"
    );

    // Same key, same application, same signature check. The only difference is
    // who the token says they are.
    let (status, body) = get(&router, &issuer.token(STRANGER)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(body.contains("private"), "{body}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_refusal_names_the_identity_so_an_operator_can_act_on_it() {
    // Nobody can look their own DID up from a login screen, and an operator
    // cannot add an identity they were never told. The response reveals nothing
    // the caller did not already send.
    let issuer = Issuer::new();
    let router = router(Admission::Closed, issuer.keys.clone());

    let (status, body) = get(&router, &issuer.token(STRANGER)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(
        body.contains(STRANGER),
        "the identity must be named: {body}"
    );
    assert!(
        body.contains(radar_serve::admission::VAR),
        "the remedy must name the variable: {body}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_closed_instance_admits_nobody_even_with_the_operator_check_off() {
    // The hole this would otherwise leave. With Cloudflare Access off, an
    // un-admitted customer previously fell through to "no operator check
    // configured, let them in" — which would have opened the product to every
    // Privy identity in the world through the door admission had just shut.
    let issuer = Issuer::new();
    let router = router(Admission::Closed, issuer.keys.clone());

    let (status, _) = get(&router, &issuer.token(JOSH)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_open_instance_admits_any_verified_identity() {
    // The public product. Entitlement is a separate question and this type does
    // not answer it: reaching the product and being allowed to use it are
    // different, and conflating them is how a paywall ends up in a router.
    let issuer = Issuer::new();
    let router = router(Admission::Open, issuer.keys.clone());

    let (status, _) = get(&router, &issuer.token(STRANGER)).await;
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "an empty store, from the handler -- not a refusal from the guard"
    );
    assert_ne!(status, StatusCode::FORBIDDEN);
}
