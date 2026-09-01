// SPDX-License-Identifier: Apache-2.0
//! That one customer cannot spend the day's model budget for everybody.
//!
//! Asserted through the real router rather than against the meter, because the
//! meter was never the doubtful part. `share::Shares` has its own unit tests and
//! they pass whether or not anything calls it — and "a component that is correct
//! and unreached" is the failure [`LEARNINGS`] entry 10 records, in a repository
//! where it has happened three times.
//!
//! So the provider here **records whether it was reached**. A question that gets
//! as far as the model when it should have been refused sets a flag the test
//! reads, which is the only way this file can be about the wiring rather than
//! about the arithmetic.
//!
//! [`LEARNINGS`]: https://github.com/hey-vera/radar/blob/main/LEARNINGS.md

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use radar_model::{Answer, Provider, Unreachable};
use radar_serve::customer::{Config, Jwk, KeyCache, Keys, Mode};
use radar_serve::share::{Allowance, Shares};
use radar_serve::{AppState, app};
use radar_store::Reader;
use radar_types::MicroUsd;
use ring::rand::SystemRandom;
use ring::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair};
use tower::ServiceExt;

const APP: &str = "cmthhkznr0a3u0cl86prxlb7x";

/// A provider that records whether it was reached, and never answers.
///
/// The whole point of the file. The meter's own arithmetic is unit-tested next
/// door and passes whether or not anything calls it — so what is asserted here
/// is that a refused question **does not reach the model**, which is the only
/// thing that costs money.
///
/// It records rather than panicking: a panic inside a handler is caught by the
/// runtime and surfaces as a 500, which is indistinguishable from a dozen other
/// faults. A flag is unambiguous.
#[derive(Debug, Default)]
struct Watched(Arc<AtomicBool>);

impl Provider for Watched {
    fn name(&self) -> &'static str {
        "watched"
    }
    fn estimate(&self) -> MicroUsd {
        MicroUsd(1_000)
    }
    fn ask(&self, _request: &radar_model::Request) -> Result<Answer, Unreachable> {
        self.0.store(true, Ordering::SeqCst);
        Err(Unreachable::NoContact(
            "this provider never answers".to_owned(),
        ))
    }
}

fn b64(bytes: &[u8]) -> String {
    radar_types::b64::encode(bytes)
        .replace('+', "-")
        .replace('/', "_")
        .trim_end_matches('=')
        .to_owned()
}

/// A signed Privy-shaped token, and the key set that would publish it.
fn valid_token(did: &str) -> (String, Keys) {
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

    let exp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("after the epoch")
        .as_secs()
        + 3_600;
    let header = br#"{"alg":"ES256","kid":"test"}"#;
    let claims =
        format!(r#"{{"iss":"privy.io","aud":"{APP}","sub":"{did}","sid":"s","exp":{exp}}}"#);
    let signing_input = format!("{}.{}", b64(header), b64(claims.as_bytes()));
    let signature = pair
        .sign(&SystemRandom::new(), signing_input.as_bytes())
        .expect("signs");
    (format!("{signing_input}.{}", b64(signature.as_ref())), keys)
}

/// A router, and the flag saying whether the model was reached through it.
fn router(allowance: Allowance, keys: Keys) -> (axum::Router, Arc<AtomicBool>) {
    let dir = std::env::temp_dir().join("radar-share-test");
    let reached = Arc::new(AtomicBool::new(false));
    let router = app(Arc::new(AppState {
        admission: radar_serve::admission::Admission::Open,
        shares: Shares::new(allowance),
        customer_salt: vec![7u8; 32],
        registry: radar_instruments::Registry::new(),
        store: Reader::open(&dir),
        x402: None,
        chat: Some(radar_serve::chat::Chat {
            agent: std::sync::Mutex::new(radar_agent::Agent::new(
                radar_agent::Config {
                    budget: radar_agent::Budget {
                        per_call_max: MicroUsd(10_000),
                        daily_max: MicroUsd(1_000_000),
                    },
                    allowlist: radar_agent::Allowlist::new(),
                },
                0,
            )),
            provider: Box::new(Watched(Arc::clone(&reached))),
            linkable: None,
            ledger: radar_serve::ledger::Store::at(&dir.join("ledger"))
                .expect("a writable scratch directory"),
            last: std::sync::Mutex::new(radar_serve::chat::LastCall::Never),
        }),
        access: radar_serve::access::Mode::Off,
        keys: radar_serve::access::KeyCache::new(),
        customer: Mode::Enforce(Config {
            app_id: APP.to_owned(),
        }),
        customer_keys: KeyCache::preloaded(keys),
        privy: None,
        linker: radar_serve::link::Linker::new(),
    }));
    (router, reached)
}

async fn ask(router: &axum::Router, token: &str) -> StatusCode {
    router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(r#"{"question":"what did you refuse today"}"#))
                .expect("a request"),
        )
        .await
        .expect("a response")
        .status()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_customer_is_refused_once_their_own_share_is_spent() {
    // The leak, closed. With a global budget only, the first caller to exhaust
    // it takes everybody else's day with them -- and nothing in the system
    // reports that as anything but a spent budget.
    let (token, keys) = valid_token("did:privy:first");
    let (router, reached) = router(Allowance::per_day(1), keys);

    // The first question is inside the allowance, so it must reach the model.
    // Asserted so the refusal below is the meter acting rather than the route
    // being broken for every request -- a test whose subject never works proves
    // nothing about why it stopped.
    let first = ask(&router, &token).await;
    assert!(
        reached.load(Ordering::SeqCst),
        "the first question is inside the allowance and must reach the provider \
         (got {first})"
    );

    reached.store(false, Ordering::SeqCst);

    // The second is over this customer's own ceiling.
    assert_eq!(ask(&router, &token).await, StatusCode::TOO_MANY_REQUESTS);
    assert!(
        !reached.load(Ordering::SeqCst),
        "a refused question must not reach the model, which is the part that costs"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_refusal_belongs_to_a_customer_rather_than_to_the_instance() {
    // The property a global budget cannot express. After one customer is
    // refused, the meter must still be counting per identity -- a counter that
    // had become global would refuse the next caller too.
    let (first, keys) = valid_token("did:privy:first");
    let (router, reached) = router(Allowance::per_day(1), keys);

    ask(&router, &first).await;
    assert_eq!(ask(&router, &first).await, StatusCode::TOO_MANY_REQUESTS);

    // A token from a different key pair does not verify, so it arrives with no
    // customer identity at all -- which is the operator path, and is not metered
    // here. What it shows is that the 429 above was specific rather than sticky.
    reached.store(false, Ordering::SeqCst);
    let (other, _) = valid_token("did:privy:second");
    assert_ne!(
        ask(&router, &other).await,
        StatusCode::TOO_MANY_REQUESTS,
        "the refusal must belong to a customer, not to the instance"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unconfigured_allowance_refuses_every_customer_rather_than_sharing() {
    // Rule 8's direction, and the state this instance is actually in today. An
    // instance that has never thought about a per-customer ceiling must not hand
    // its whole model budget to whoever signs up first.
    let (token, keys) = valid_token("did:privy:first");
    let (router, reached) = router(Allowance::CLOSED, keys);

    // 503, not 429: nothing the customer does fixes this, and a rate-limit
    // status would have them waiting for a window that never opens.
    assert_eq!(ask(&router, &token).await, StatusCode::SERVICE_UNAVAILABLE);
    assert!(!reached.load(Ordering::SeqCst));
}
