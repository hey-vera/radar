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
