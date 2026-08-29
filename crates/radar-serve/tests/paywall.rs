// SPDX-License-Identifier: Apache-2.0
//! The paywall, exercised through the real router.
//!
//! The module tests prove the rules; this proves the route enforces them. The
//! failure worth guarding against is a handler that computes the right refusal
//! and returns the response anyway — and the module's own tests are green either
//! way.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt as _;
use radar_instruments::{CreatorHistory, Registry};
use radar_serve::x402::{Config, Network};
use radar_serve::{AppState, app};
use radar_store::Reader;
use tower::ServiceExt as _;

/// A router whose paid surface points at a facilitator that is not there.
fn configured() -> axum::Router {
    let mut registry = Registry::new();
    registry.register(CreatorHistory);
    app(Arc::new(AppState {
        registry,
        store: Reader::open(std::env::temp_dir().join("radar-paywall-test")),
        x402: Some(Config {
            pay_to: "RadarTreasury1111111111111111111111111111111".to_owned(),
            facilitator: "http://127.0.0.1:1".to_owned(),
            network: Network::solana_usdc(),
            margin_percent: 50,
        }),
        chat: None,
        access: radar_serve::access::Mode::Off,
        keys: radar_serve::access::KeyCache::new(),
        linker: radar_serve::link::Linker::new(),
    }))
}

/// A router with the paid surface switched off.
fn unconfigured() -> axum::Router {
    let mut registry = Registry::new();
    registry.register(CreatorHistory);
    app(Arc::new(AppState {
        registry,
        store: Reader::open(std::env::temp_dir().join("radar-paywall-test")),
        x402: None,
        chat: None,
        access: radar_serve::access::Mode::Off,
        keys: radar_serve::access::KeyCache::new(),
        linker: radar_serve::link::Linker::new(),
    }))
}

async fn post(router: axum::Router, path: &str, payment: Option<&str>) -> (StatusCode, String) {
    let mut builder = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json");
    if let Some(p) = payment {
        builder = builder.header("payment-signature", p);
    }
    let response = router
        .oneshot(
            builder
                .body(Body::from(r#"{"creator":"x"}"#))
                .expect("request"),
        )
        .await
        .expect("router answers");
    let status = response.status();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    (status, String::from_utf8_lossy(&body).into_owned())
}

#[tokio::test]
async fn a_call_with_no_payment_is_challenged() {
    let (status, body) = post(configured(), "/x402/v1/instruments/creator_history", None).await;
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED);
    assert!(body.contains("\"scheme\":\"exact\""), "{body}");
    assert!(body.contains("maxAmountRequired"), "{body}");
}

#[tokio::test]
async fn a_payment_that_cannot_be_verified_is_never_served() {
    // The one that matters. The facilitator is unreachable, so nothing can
    // establish that this caller paid — and an unverifiable payment must be
    // refused rather than trusted.
    let payment = radar_types::b64::encode(br#"{"scheme":"exact"}"#);
    let (status, body) = post(
        configured(),
        "/x402/v1/instruments/creator_history",
        Some(&payment),
    )
    .await;
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED, "{body}");
    assert!(body.contains("facilitator unreachable"), "{body}");
    assert!(body.contains("\"retryable\":true"), "{body}");
}

#[tokio::test]
async fn a_malformed_payment_header_is_refused() {
    let (status, body) = post(
        configured(),
        "/x402/v1/instruments/creator_history",
        Some("!!!not a payment!!!"),
    )
    .await;
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED);
    assert!(body.contains("malformed"), "{body}");
    assert!(body.contains("\"retryable\":false"), "{body}");
}

#[tokio::test]
async fn an_unconfigured_paid_surface_does_not_exist() {
    // Not "serves free", not "serves on trust" — absent. A paywall that falls
    // back to open is a paywall in name only.
    let (status, _) = post(unconfigured(), "/x402/v1/instruments/creator_history", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_unknown_instrument_is_not_found_rather_than_charged_for() {
    let (status, _) = post(configured(), "/x402/v1/instruments/nonesuch", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_paid_route_never_answers_200_without_a_settled_payment() {
    // Stated as its own test because it is the property, and because a future
    // refactor that adds an early return is exactly how it would be lost.
    let payment = radar_types::b64::encode(br#"{"scheme":"exact"}"#);
    for header in [None, Some(payment.as_str()), Some("garbage")] {
        let (status, body) =
            post(configured(), "/x402/v1/instruments/creator_history", header).await;
        assert_ne!(status, StatusCode::OK, "served without payment: {body}");
    }
}
