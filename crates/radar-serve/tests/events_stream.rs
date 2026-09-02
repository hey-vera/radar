// SPDX-License-Identifier: Apache-2.0
//! The event stream, exercised over HTTP against a real store.
//!
//! What matters here is not that bytes come out — it is *when*. A stream that
//! delivers nothing until it closes is indistinguishable from a dead server,
//! and that is the failure mode the proxy configuration exists to prevent.

use std::sync::Arc;

use radar_instruments::Registry;
use radar_serve::{AppState, app};
use radar_store::{Reader, Writer};
use radar_types::{Address, Signature, Slot};
use tower::ServiceExt;

fn launch(slot: u64) -> radar_store::Event {
    radar_store::Event::Launch(Box::new(radar_store::Launch {
        envelope: radar_store::Envelope {
            slot: Slot(slot),
            signature: Signature::new([(slot % 251) as u8; 64]),
            tx_index: 0,
            instruction_index: 0,
            parent_index: None,
            succeeded: true,
        },
        origin: radar_store::Origin::known(Address::new([3u8; 32]), "create_v2"),
        mint: Address::new([1u8; 32]),
        creator: Address::new([2u8; 32]),
        name: "t".to_owned(),
        symbol: "T".to_owned(),
        uri: String::new(),
        dev_buy_lamports: None,
    }))
}

fn state_with_a_store() -> (Arc<AppState>, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut writer = Writer::open(dir.path(), 64).expect("open");
    writer.append(launch(500)).expect("append");
    writer.flush().expect("flush");
    (
        Arc::new(AppState {
            admission: radar_serve::admission::Admission::Open,
            shares: radar_serve::share::Shares::new(radar_serve::share::Allowance::per_day(100)),
            customer_salt: vec![7u8; 32],
            registry: Registry::new(),
            store: Reader::open(dir.path()),
            x402: None,
            chat: None,
            access: radar_serve::access::Mode::Off,
            keys: radar_serve::access::KeyCache::new(),
            customer: radar_serve::customer::Mode::Off,
            customer_keys: radar_serve::customer::KeyCache::new(),
            privy: None,
            linker: radar_serve::link::Linker::new(),
            challenges: None,
        }),
        dir,
    )
}

#[tokio::test]
async fn the_stream_declares_itself_as_events_and_forbids_caching() {
    // Both headers are load-bearing through a proxy. Without the content type
    // nothing downstream knows to stop buffering; without `no-cache` an
    // intermediary is entitled to serve a stale copy of a stream, which is a
    // page frozen at whatever the first viewer saw.
    let (state, _dir) = state_with_a_store();
    let response = app(state)
        .oneshot(
            axum::http::Request::builder()
                .uri("/v1/events")
                .body(axum::body::Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let headers = response.headers();
    assert_eq!(
        headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default(),
        "text/event-stream"
    );
    assert!(
        headers
            .get("cache-control")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .contains("no-cache"),
        "a cached event stream is a page frozen at whatever the first viewer saw"
    );
}

#[tokio::test]
async fn a_fresh_connection_receives_the_current_state_without_waiting() {
    // The first frame must not wait for the poll interval. A page opened during
    // a quiet minute would otherwise render blank for ten seconds and look
    // broken -- and on a store that changes hourly, most minutes are quiet.
    let (state, _dir) = state_with_a_store();
    let response = app(state)
        .oneshot(
            axum::http::Request::builder()
                .uri("/v1/events")
                .body(axum::body::Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    // One frame, read with a timeout well under the poll interval: if this only
    // arrives on the timer, the wait fails rather than passing slowly.
    let mut body = response.into_body().into_data_stream();
    let frame = tokio::time::timeout(std::time::Duration::from_secs(2), {
        use futures_util::StreamExt as _;
        body.next()
    })
    .await
    .expect("the first event must not wait for the poll interval")
    .expect("a frame")
    .expect("readable");

    let text = String::from_utf8_lossy(&frame);
    assert!(text.contains("event: store"), "unexpected frame: {text}");
    assert!(
        text.contains("\"as_of\":500"),
        "the frame carries the store's watermark: {text}"
    );
    assert!(
        text.contains("\"launches\":1"),
        "and the row counts: {text}"
    );
}

#[tokio::test]
async fn an_empty_store_does_not_stall_the_stream_open() {
    // A store that cannot name a watermark has nothing to report. The stream
    // must still open and stay open rather than erroring, because a fresh
    // instance is a normal state and the page has to render something.
    let dir = tempfile::tempdir().expect("tempdir");
    let state = Arc::new(AppState {
        admission: radar_serve::admission::Admission::Open,
        shares: radar_serve::share::Shares::new(radar_serve::share::Allowance::per_day(100)),
        customer_salt: vec![7u8; 32],
        registry: Registry::new(),
        store: Reader::open(dir.path()),
        x402: None,
        chat: None,
        access: radar_serve::access::Mode::Off,
        keys: radar_serve::access::KeyCache::new(),
        customer: radar_serve::customer::Mode::Off,
        customer_keys: radar_serve::customer::KeyCache::new(),
        privy: None,
        linker: radar_serve::link::Linker::new(),
        challenges: None,
    });

    let response = app(state)
        .oneshot(
            axum::http::Request::builder()
                .uri("/v1/events")
                .body(axum::body::Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(
        response.status(),
        axum::http::StatusCode::OK,
        "an empty store is not an error"
    );
}

#[tokio::test]
async fn the_interface_answers_reads_and_refuses_everything_else() {
    // The fallback serves the interface for GET and HEAD and returns 404 for
    // anything else, so an unrouted POST is answered like an unrouted path
    // rather than as a method error. A 405 would say the path exists and only
    // the verb is wrong, which for the unconfigured paid routes is a leak.
    //
    // Inverting the condition that decides this makes EVERY request a 404 --
    // the whole interface gone -- and nothing noticed until mutation testing
    // deleted it.
    let (state, _dir) = state_with_a_store();
    let ask = |method: &'static str, path: &'static str| {
        let app = app(state.clone());
        async move {
            app.oneshot(
                axum::http::Request::builder()
                    .method(method)
                    .uri(path)
                    .body(axum::body::Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response")
            .status()
        }
    };

    assert_eq!(
        ask("GET", "/").await,
        axum::http::StatusCode::OK,
        "a read of the interface must be served"
    );
    assert_eq!(
        ask("GET", "/tokens/anything").await,
        axum::http::StatusCode::OK,
        "and so must a route the interface owns"
    );
    assert_eq!(
        ask("POST", "/tokens/anything").await,
        axum::http::StatusCode::NOT_FOUND,
        "a write to an unrouted path is not found, never a method error"
    );
    assert_eq!(
        ask("DELETE", "/anything").await,
        axum::http::StatusCode::NOT_FOUND
    );
}
