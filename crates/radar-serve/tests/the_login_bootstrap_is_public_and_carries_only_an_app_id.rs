// SPDX-License-Identifier: Apache-2.0
//! `/v1/customer/config`: the one public route under `/v1/customer/`.
//!
//! # Why it is public, and why that needs a test rather than a comment
//!
//! The interface reads this **before** a customer has any token, so gating it
//! behind one would make logging in require being logged in. That is a
//! deliberate hole in an otherwise closed prefix, and a deliberate hole is
//! exactly the kind of thing that widens without anyone deciding to widen it.
//!
//! So two properties are asserted here, and they pull against each other:
//!
//! - It answers **without** credentials of any kind.
//! - It answers with the app id and **nothing else** — no store counts, no
//!   instance health, no customer data. A bootstrap endpoint that also reported
//!   those would be an operator surface wearing a public one's clothes, which is
//!   what `audience_of` exists to prevent and what `/v1/store` is the cautionary
//!   example of.
//!
//! And rule 8: with no application configured it refuses, rather than serving a
//! page that fails later at the login button with something less legible.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt as _;
use radar_serve::admission::Admission;
use radar_serve::customer::{Config, KeyCache, Mode};
use radar_serve::{AppState, app};
use radar_store::Reader;
use tower::ServiceExt as _;

const APP: &str = "test-privy-app-id";

fn router(customer: Mode) -> axum::Router {
    app(Arc::new(AppState {
        admission: Admission::Closed,
        shares: radar_serve::share::Shares::new(radar_serve::share::Allowance::per_day(100)),
        customer_salt: vec![7u8; 32],
        registry: radar_instruments::Registry::new(),
        store: Reader::open(std::env::temp_dir().join("radar-config-test")),
        x402: None,
        chat: None,
        // Enforcing, deliberately. If this route is reachable with Access *on*
        // and no token at all, it is reachable full stop -- which is the claim.
        access: radar_serve::access::Mode::Off,
        keys: radar_serve::access::KeyCache::new(),
        customer,
        customer_keys: KeyCache::new(),
        privy: None,
        linker: radar_serve::link::Linker::new(),
    }))
}

async fn get(router: &axum::Router, uri: &str) -> (StatusCode, String) {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                // No authorization header, no Access assertion, nothing.
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

#[tokio::test]
async fn it_answers_an_anonymous_caller_with_the_application_id() {
    let router = router(Mode::Enforce(Config {
        app_id: APP.to_owned(),
    }));
    let (status, body) = get(&router, "/v1/customer/config").await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let value: serde_json::Value = serde_json::from_str(&body).expect("JSON");
    assert_eq!(value["privy_app_id"], APP);
}

#[tokio::test]
async fn it_carries_the_app_id_and_nothing_else() {
    // The containment half. A bootstrap route that grew a second field would
    // widen a public surface without anyone deciding to, and the field most
    // likely to be added -- some count or status "while we're here" -- is
    // exactly the operator data this prefix exists to keep back.
    let router = router(Mode::Enforce(Config {
        app_id: APP.to_owned(),
    }));
    let (_, body) = get(&router, "/v1/customer/config").await;
    let value: serde_json::Value = serde_json::from_str(&body).expect("JSON");
    let object = value.as_object().expect("an object");
    assert_eq!(
        object.keys().collect::<Vec<_>>(),
        vec!["privy_app_id"],
        "exactly one field; adding another widens a public route"
    );
    // And nothing that reads like an operator surface has leaked in.
    for forbidden in ["decisions", "outcomes", "launches", "store", "watermark"] {
        assert!(
            !body.contains(forbidden),
            "the login bootstrap must not mention {forbidden}"
        );
    }
}

#[tokio::test]
async fn an_instance_with_no_application_configured_refuses() {
    // Rule 8. Not an empty string, not a placeholder -- a refusal, so whoever
    // deploys it learns at startup rather than at a login button.
    let router = router(Mode::Off);
    let (status, body) = get(&router, "/v1/customer/config").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "body: {body}");
    assert!(
        !body.contains(APP),
        "a refusal must not carry an application id"
    );
}

#[tokio::test]
async fn its_siblings_are_not_public() {
    // The exception must not have widened. These are the routes an anonymous
    // caller must still be refused, checked through the same door.
    let router = router(Mode::Enforce(Config {
        app_id: APP.to_owned(),
    }));
    // `/v1/events` is deliberately absent: it is a server-sent-event stream
    // that never completes, so requesting it here hangs the test rather than
    // failing it. Its audience is covered by the route table in `access.rs`.
    for uri in ["/v1/customer/wallet", "/v1/store"] {
        let (status, _) = get(&router, uri).await;
        assert_ne!(
            status,
            StatusCode::OK,
            "{uri} must not answer an anonymous caller"
        );
    }
}
