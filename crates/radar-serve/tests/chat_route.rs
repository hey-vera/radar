// SPDX-License-Identifier: Apache-2.0
//! The chat route exists only when a provider is configured.
//!
//! Asserted from outside the crate, through the real router, because the thing
//! being checked is a property of `app()` rather than of a handler. A unit test
//! calling the handler directly would pass with the route mounted
//! unconditionally, which is precisely the bug.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use radar_instruments::{CreatorHistory, Registry};
use radar_serve::{AppState, app};
use radar_store::Reader;
use tower::ServiceExt;

/// A server with no model provider — the shipped state.
fn unconfigured() -> axum::Router {
    let mut registry = Registry::new();
    registry.register(CreatorHistory);
    app(Arc::new(AppState {
        admission: radar_serve::admission::Admission::Open,
        shares: radar_serve::share::Shares::new(radar_serve::share::Allowance::per_day(100)),
        customer_salt: vec![7u8; 32],
        registry,
        store: Reader::open(std::env::temp_dir().join("radar-chat-route-test")),
        x402: None,
        chat: None,
        access: radar_serve::access::Mode::Off,
        keys: radar_serve::access::KeyCache::new(),
        customer: radar_serve::customer::Mode::Off,
        customer_keys: radar_serve::customer::KeyCache::new(),
        privy: None,
        linker: radar_serve::link::Linker::new(),
        challenges: None,
    }))
}

async fn post(router: axum::Router, path: &str, body: &str) -> (StatusCode, String) {
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_owned()))
                .expect("a well-formed request"),
        )
        .await
        .expect("the router answers");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("a readable body")
        .to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test]
async fn an_unconfigured_agent_has_no_route_rather_than_a_broken_one() {
    // Rule 8, and the sharp part is *which* refusal. A 503 would say the
    // feature exists and is down, which invites a retry loop against something
    // that will never come up. A 404 says there is nothing here.
    let (status, body) = post(unconfigured(), "/v1/chat", r#"{"question":"hello"}"#).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

#[tokio::test]
async fn the_unconfigured_route_does_not_leak_its_own_shape() {
    // The failure this catches has happened once already in this crate: a
    // method-scoped fallback answered POST-to-unknown with 405, which says the
    // path exists and only the verb is wrong. For a surface that is supposed
    // not to exist, that is the whole leak.
    //
    // Sending deliberate nonsense: a body the handler could not parse must
    // still produce "no such route" rather than "bad request", because a
    // parse error is an admission that something tried to parse it.
    let (status, body) = post(unconfigured(), "/v1/chat", "not json at all").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert!(
        !body.contains("question"),
        "the request shape must not be echoed back: {body}"
    );
}

#[tokio::test]
async fn a_get_to_the_chat_path_is_not_a_method_error() {
    // Same leak from the other side. `GET /v1/chat` on an unconfigured server
    // must be indistinguishable from `GET /v1/anything-else`, which the
    // interface fallback answers.
    let router = unconfigured();
    let chat = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/chat")
                .body(Body::empty())
                .expect("well-formed"),
        )
        .await
        .expect("answered");
    let other = router
        .oneshot(
            Request::builder()
                .uri("/v1/there-is-no-such-thing")
                .body(Body::empty())
                .expect("well-formed"),
        )
        .await
        .expect("answered");
    assert_eq!(
        chat.status(),
        other.status(),
        "an unmounted route must look like any other unknown path"
    );
    assert_ne!(chat.status(), StatusCode::METHOD_NOT_ALLOWED);
}
