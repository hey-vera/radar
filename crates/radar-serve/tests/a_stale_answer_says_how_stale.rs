// SPDX-License-Identifier: Apache-2.0
//! Serving a slightly older answer, and never serving one silently.
//!
//! Keyed exactly on the watermark, the endpoint cache missed about half the time
//! in production: the recorder advances the watermark roughly every fifty
//! seconds, and a miss measured 4.4 to 5.2 seconds against a 500ms budget. So a
//! recent-enough answer is reused.
//!
//! The whole argument for doing that is the label. An older answer rendered as
//! the current one is rule 9's shape — unknown presented as fresh — and the
//! reader has no way to tell. So every response carries the watermark it was
//! actually computed at, and this file is what holds that up.

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

fn state_at(dir: &std::path::Path) -> Arc<AppState> {
    Arc::new(AppState {
        admission: radar_serve::admission::Admission::Open,
        shares: radar_serve::share::Shares::new(radar_serve::share::Allowance::per_day(100)),
        customer_salt: vec![7u8; 32],
        registry: Registry::new(),
        store: Reader::open(dir),
        x402: None,
        chat: None,
        access: radar_serve::access::Mode::Off,
        keys: radar_serve::access::KeyCache::new(),
        customer: radar_serve::customer::Mode::Off,
        customer_keys: radar_serve::customer::KeyCache::new(),
        privy: None,
        linker: radar_serve::link::Linker::new(),
        scoreboard: radar_serve::cache::Cache::new(),
        token: radar_serve::cache::Cache::new(),
        challenges: None,
    })
}

/// The `as_of` a request to `/v1/scoreboard` comes back with.
async fn scoreboard_as_of(state: &Arc<AppState>) -> u64 {
    let response = app(Arc::clone(state))
        .oneshot(
            axum::http::Request::builder()
                .uri("/v1/scoreboard")
                .body(axum::body::Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 4_000_000)
        .await
        .expect("body");
    let value: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    value
        .get("as_of")
        .and_then(serde_json::Value::as_u64)
        .expect("every response says which watermark it was computed at")
}

#[tokio::test]
async fn a_reused_answer_reports_the_watermark_it_was_computed_at() {
    // The three cases in one run, because the interesting thing is the
    // transition between them and a test of each alone would not see it.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut writer = Writer::open(dir.path(), 64).expect("open");
    writer.append(launch(500)).expect("append");
    writer.flush().expect("flush");

    let state = state_at(dir.path());

    // 1. Computed here, so it is its own watermark.
    assert_eq!(scoreboard_as_of(&state).await, 500);

    // 2. The store moves 100 slots, inside the allowance. The cached answer is
    //    served -- and it says 500, not 600. Reporting 600 here would be the bug
    //    this file exists for: the reader would believe the answer accounts for
    //    everything up to 600, and nothing would say otherwise.
    writer.append(launch(600)).expect("append");
    writer.flush().expect("flush");
    assert_eq!(
        scoreboard_as_of(&state).await,
        500,
        "a reused answer keeps its own watermark"
    );

    // 3. Past the allowance, so it is recomputed and is current again.
    writer.append(launch(800)).expect("append");
    writer.flush().expect("flush");
    assert_eq!(
        scoreboard_as_of(&state).await,
        800,
        "past the allowance the answer is recomputed, not stretched"
    );
}

#[tokio::test]
async fn one_token_is_never_answered_with_another_ones_evidence() {
    // The allowance must not weaken the key. It nearly did once already: an
    // earlier shape kept the mint inside the cached value and left the handler
    // to compare it, and the handler fell through to a lookup keyed on the
    // watermark alone. LEARNINGS 27.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut writer = Writer::open(dir.path(), 64).expect("open");
    writer.append(launch(500)).expect("append");
    writer.flush().expect("flush");
    let state = state_at(dir.path());

    let ask = |mint: &'static str| {
        let state = Arc::clone(&state);
        async move {
            let response = app(state)
                .oneshot(
                    axum::http::Request::builder()
                        .uri(format!("/v1/tokens/{mint}"))
                        .body(axum::body::Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");
            let bytes = axum::body::to_bytes(response.into_body(), 4_000_000)
                .await
                .expect("body");
            serde_json::from_slice::<serde_json::Value>(&bytes).expect("json")
        }
    };

    let a = "11111111111111111111111111111111";
    let b = "22222222222222222222222222222222";
    assert_eq!(ask(a).await.get("mint").and_then(|m| m.as_str()), Some(a));
    // Same watermark, well inside the allowance, different mint.
    assert_eq!(
        ask(b).await.get("mint").and_then(|m| m.as_str()),
        Some(b),
        "a recent answer for another mint is not this mint's answer"
    );
}
