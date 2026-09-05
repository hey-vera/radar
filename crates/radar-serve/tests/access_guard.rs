// SPDX-License-Identifier: Apache-2.0
//! Nothing is served without an identity when Access is enforced.
//!
//! From outside the crate, through the real router, because the property is
//! about the *layer* rather than any handler: a per-route check is one somebody
//! forgets on the route they add next year, and this is what notices.
//!
//! The unit tests in `access.rs` prove the verifier refuses forgeries. These
//! prove the verifier is actually in the path — which is the half that has
//! historically been missing when an authentication bug is written up.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use radar_instruments::{CreatorHistory, Registry};
use radar_serve::access::{Config, KeyCache, Mode};
use radar_serve::{AppState, app};
use radar_store::Reader;
use tower::ServiceExt;

fn router(access: Mode) -> axum::Router {
    let mut registry = Registry::new();
    registry.register(CreatorHistory);
    app(Arc::new(AppState {
        admission: radar_serve::admission::Admission::Open,
        shares: radar_serve::share::Shares::new(radar_serve::share::Allowance::per_day(100)),
        customer_salt: vec![7u8; 32],
        registry,
        store: Reader::open(std::env::temp_dir().join("radar-access-guard-test")),
        x402: None,
        chat: None,
        access,
        keys: KeyCache::new(),
        customer: radar_serve::customer::Mode::Off,
        customer_keys: radar_serve::customer::KeyCache::new(),
        privy: None,
        linker: radar_serve::link::Linker::new(),
        scoreboard: radar_serve::cache::Cache::new(),
        token: radar_serve::cache::Cache::new(),
        challenges: None,
    }))
}

fn enforcing() -> axum::Router {
    router(Mode::Enforce(Config {
        // Unreachable on purpose. A test that resolved a real Cloudflare domain
        // would be a test that fails when the network does, and the property
        // being checked is that an unauthenticated request never gets far
        // enough to need a key at all.
        team_domain: "radar-test.invalid".to_owned(),
        aud: "radar-aud-tag".to_owned(),
    }))
}

async fn status(router: axum::Router, path: &str) -> StatusCode {
    router
        .oneshot(
            Request::builder()
                .uri(path)
                .body(Body::empty())
                .expect("a well-formed request"),
        )
        .await
        .expect("the router answers")
        .status()
}

#[tokio::test(flavor = "multi_thread")]
async fn every_private_path_is_refused_without_an_assertion() {
    // The list is deliberately long and includes the asset path. An interface
    // whose HTML is behind a login and whose JavaScript bundle is not still
    // leaks the shape of the product, and the bundle is where the API paths are
    // written down.
    for path in [
        "/",
        "/ops",
        "/v1/funnel",
        "/v1/store",
        "/v1/tokens/So11111111111111111111111111111111111111112",
        "/v1/instruments",
        "/v1/events",
        "/assets/index-abc123.js",
        "/there-is-no-such-path",
    ] {
        assert_eq!(
            status(enforcing(), path).await,
            StatusCode::FORBIDDEN,
            "{path} was served without an identity"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_forged_assertion_does_not_get_through_the_layer() {
    // `access.rs` proves the verifier refuses this. This proves the verifier is
    // in the path -- which is the half that is missing when an authentication
    // bug gets written up.
    let response = enforcing()
        .oneshot(
            Request::builder()
                .uri("/v1/funnel")
                .header("cf-access-jwt-assertion", "eyJhbGciOiJub25lIn0.e30.")
                .body(Body::empty())
                .expect("well-formed"),
        )
        .await
        .expect("answered");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_authenticated_user_email_header_is_not_an_identity() {
    // The tempting implementation reads this header. Anything that reaches the
    // origin can set it, and "the origin is behind a tunnel" is a network
    // topology rather than an authentication model -- tunnels get a second
    // ingress for a debugging session that is never removed.
    let response = enforcing()
        .oneshot(
            Request::builder()
                .uri("/v1/funnel")
                .header("cf-access-authenticated-user-email", "joshfair2@gmail.com")
                .body(Body::empty())
                .expect("well-formed"),
        )
        .await
        .expect("answered");
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "a header anyone can set is not an identity"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_monitor_path_stays_reachable() {
    // A health check behind a login turns every uptime probe into a false
    // alarm, and then the alarm gets switched off.
    assert_eq!(status(enforcing(), "/health").await, StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_public_sites_three_documents_are_served_without_an_identity() {
    // Design 0008 phase 1. The site lives on another origin and has no token;
    // a document behind the login would make every visitor see the committed
    // fixture for ever and never notice. Each is public by exact path, and
    // nothing else under `/v1/public/` is.
    //
    // The leaderboard and the pool answer with their honest empty documents
    // when nothing is on disk. The stats document refuses -- 404, not zeroes
    // -- because a figure that is not on disk cannot be stated; what matters
    // here is that the refusal is the handler's and not the guard's.
    //
    // The bodies are read, not just the statuses. CI's mutants run on
    // 2026-09-05 replaced each handler with an empty 200 and this test still
    // passed: a guard test that only checks "not forbidden" cannot tell a
    // document from a blank page. Nothing is configured here, so the handlers
    // read their default paths relative to the crate and find nothing.
    let (code, kind, origin, body) = document(enforcing(), "/v1/public/leaderboard").await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(kind.as_deref(), Some("application/json"));
    assert!(origin.is_none(), "no origin configured, so no CORS header");
    assert_eq!(body["week"], serde_json::Value::Null);
    assert_eq!(body["entries"], serde_json::json!([]));

    let (code, kind, _, body) = document(enforcing(), "/v1/public/pool").await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(kind.as_deref(), Some("application/json"));
    assert_eq!(body["vault"], serde_json::Value::Null);
    assert_eq!(body["winners"], serde_json::json!([]));

    let (code, kind, _, body) = document(enforcing(), "/v1/public/stats").await;
    assert_eq!(code, StatusCode::NOT_FOUND);
    assert_eq!(kind.as_deref(), Some("application/json"));
    assert_eq!(body["error"], "not measured yet on this instance");

    assert_eq!(
        status(enforcing(), "/v1/public/anything-else").await,
        StatusCode::FORBIDDEN,
        "the directory is not public, only the three documents"
    );
}

/// One public document: status, content type, the CORS origin header if any,
/// and the body as JSON.
async fn document(
    router: axum::Router,
    path: &str,
) -> (
    StatusCode,
    Option<String>,
    Option<String>,
    serde_json::Value,
) {
    let response = router
        .oneshot(
            Request::builder()
                .uri(path)
                .body(Body::empty())
                .expect("a well-formed request"),
        )
        .await
        .expect("the router answers");
    let status = response.status();
    let header = |name: &str| {
        response
            .headers()
            .get(name)
            .map(|v| v.to_str().expect("ascii").to_owned())
    };
    let kind = header("content-type");
    let origin = header("access-control-allow-origin");
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("a body");
    let body = serde_json::from_slice(&bytes).expect("the document is JSON");
    (status, kind, origin, body)
}

#[tokio::test(flavor = "multi_thread")]
async fn switching_access_off_serves_normally() {
    // The other direction. A guard verified only in the refusing direction is
    // indistinguishable from a server that is simply broken, which is finding
    // 1.6 of the previous plan and the reason `radar brief` grew both-way
    // checks.
    assert_eq!(status(router(Mode::Off), "/health").await, StatusCode::OK);
    assert_ne!(
        status(router(Mode::Off), "/v1/funnel").await,
        StatusCode::FORBIDDEN,
        "with access off, the funnel answers on its own merits"
    );
}
