// SPDX-License-Identifier: Apache-2.0
//! Radar's HTTP surface: an ops page, a JSON API, a stateless MCP endpoint, and
//! an x402-priced public surface.
//!
//! All four are derived from one instrument registry. A second catalogue, price
//! list or schema maintained anywhere else would drift, and the one that drifted
//! would be the paid one.
//!
//! The paid surface is off unless x402 is configured. It is never served free as
//! a fallback: a paywall that fails open is not a paywall.

#![forbid(unsafe_code)]

pub mod mcp;
mod ops;
pub mod x402;

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use radar_asof::AsOf;
use radar_instruments::{Context, Registry};
use radar_store::Reader;
use serde_json::{Value, json};

/// Everything the server needs to answer.
pub struct AppState {
    /// The instruments on offer.
    pub registry: Registry,
    /// The recorded event log.
    pub store: Reader,
    /// x402 configuration, or `None` when the paid surface is disabled.
    pub x402: Option<x402::Config>,
}

/// Builds the router.
///
/// The paid routes are only mounted when x402 is configured, so an unconfigured
/// deployment returns 404 for them rather than serving intelligence for free.
pub fn app(state: Arc<AppState>) -> Router {
    let mut router = Router::new()
        .route("/", get(ops_page))
        .route("/health", get(health))
        .route("/v1/instruments", get(list_instruments))
        .route("/v1/instruments/{name}", post(call_instrument))
        .route("/mcp", post(mcp_endpoint));

    if state.x402.is_some() {
        router = router
            .route("/x402/v1/instruments", get(list_instruments))
            .route("/x402/v1/instruments/{name}", post(paid_instrument));
    }

    router.with_state(state)
}

async fn health(State(state): State<Arc<AppState>>) -> Json<Value> {
    let watermark = Reader::watermark(&state.store).ok().flatten();
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "instruments": state.registry.len(),
        // Null rather than zero for an empty store: "nothing recorded" and
        // "recorded up to genesis" are different states.
        "watermarkSlot": watermark.map(radar_types::Slot::get),
        "paidSurface": state.x402.is_some(),
    }))
}

async fn ops_page(State(state): State<Arc<AppState>>) -> Html<String> {
    Html(ops::render(&state))
}

async fn list_instruments(State(state): State<Arc<AppState>>) -> Json<Value> {
    let margin = state
        .x402
        .as_ref()
        .map_or(radar_instruments::DEFAULT_MARGIN_PERCENT, |c| {
            c.margin_percent
        });

    Json(json!({
        "instruments": state.registry.iter().map(|i| {
            let spec = i.spec();
            json!({
                "name": spec.name,
                "version": spec.version.to_string(),
                "summary": spec.summary,
                "latency": spec.latency,
                "determinism": spec.determinism,
                "priceMicroUsd": spec.public_price(margin).get(),
                "inputSchema": i.input_schema(),
                "outputSchema": i.output_schema(),
            })
        }).collect::<Vec<_>>(),
    }))
}

/// Runs an instrument at the store's own watermark.
fn invoke(state: &AppState, name: &str, args: Value) -> Response {
    let watermark = match Reader::watermark(&state.store) {
        Ok(Some(slot)) => slot,
        Ok(None) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": "the store is empty and cannot answer as of any slot" })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    // The caller does not choose the watermark. Letting them would let them ask
    // what Radar knew before it knew it.
    let ctx = Context {
        as_of: AsOf::at(watermark),
        store: &state.store,
    };
    match state.registry.invoke(name, args, &ctx) {
        Ok(record) => {
            let status = if record.error.is_some() {
                StatusCode::UNPROCESSABLE_ENTITY
            } else {
                StatusCode::OK
            };
            (status, Json(json!(record))).into_response()
        }
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn call_instrument(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    body: Option<Json<Value>>,
) -> Response {
    let args = body.map_or_else(|| json!({}), |Json(v)| v);
    invoke(&state, &name, args)
}

/// The paid surface.
///
/// Challenges with `402` when no payment is presented. A payment that *is*
/// presented is not honoured yet — verification needs a facilitator round trip,
/// and until that is wired an unverified payment must be refused rather than
/// trusted. Serving the response on the strength of a header nobody checked
/// would be worse than not offering the route.
async fn paid_instrument(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
    // Deliberately unread. The arguments are only worth parsing once a payment
    // has been verified, and verification is not wired yet.
    _body: Option<Json<Value>>,
) -> Response {
    let Some(config) = state.x402.as_ref() else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "paid surface not configured" })),
        )
            .into_response();
    };
    let Some(instrument) = state.registry.get(&name) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!("no instrument `{name}`") })),
        )
            .into_response();
    };

    let spec = instrument.spec();
    let price = spec.public_price(config.margin_percent);
    let resource = format!("/x402/v1/instruments/{name}");

    let Some(_payment) = x402::payment_header(&headers) else {
        return (
            StatusCode::PAYMENT_REQUIRED,
            Json(config.challenge(&resource, spec.summary, price)),
        )
            .into_response();
    };

    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({
            "error": "payment verification is not wired to a facilitator yet",
            "detail": "Radar will not serve a paid response on the strength of an \
                       unverified header. Configure a facilitator that can verify and \
                       settle, or use the internal surface.",
        })),
    )
        .into_response()
}

async fn mcp_endpoint(State(state): State<Arc<AppState>>, Json(body): Json<Value>) -> Response {
    let server = mcp::Server {
        registry: &state.registry,
        store: &state.store,
    };

    // A batch is an array; a single call is an object. Both are valid JSON-RPC.
    let response = if let Some(batch) = body.as_array() {
        let replies: Vec<Value> = batch.iter().filter_map(|r| server.handle(r)).collect();
        if replies.is_empty() {
            // Every element was a notification, so there is nothing to say.
            return StatusCode::ACCEPTED.into_response();
        }
        Value::Array(replies)
    } else {
        match server.handle(&body) {
            Some(reply) => reply,
            None => return StatusCode::ACCEPTED.into_response(),
        }
    };

    (StatusCode::OK, Json(response)).into_response()
}
