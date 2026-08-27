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

pub mod api;
pub mod chat;
mod embed;
pub mod facilitator;
pub mod mcp;
mod ops;
pub mod x402;

use core::convert::Infallible;
use core::time::Duration;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::Stream;
use radar_asof::AsOf;
use radar_instruments::{Context, Registry};
use radar_store::Reader;
use serde::Serialize;
use serde_json::{Value, json};

/// Everything the server needs to answer.
pub struct AppState {
    /// The instruments on offer.
    pub registry: Registry,
    /// The recorded event log.
    pub store: Reader,
    /// x402 configuration, or `None` when the paid surface is disabled.
    pub x402: Option<x402::Config>,
    /// The agent, or `None` when no model provider is configured.
    ///
    /// `None` is the shipped state and the route is not mounted in it. Rule 8:
    /// a component with no configuration refuses rather than degrades, and the
    /// degradation available here -- answering from a cheaper model, or from
    /// cache -- would be reporting confidence Radar does not have.
    pub chat: Option<chat::Chat>,
}

/// Builds the router.
///
/// The paid routes are only mounted when x402 is configured, so an unconfigured
/// deployment returns 404 for them rather than serving intelligence for free.
pub fn app(state: Arc<AppState>) -> Router {
    let mut router = Router::new()
        // The interface, embedded in this binary. The server-rendered ops page
        // stays at /ops as the no-JavaScript fallback: it is what answers when
        // somebody is debugging with curl, and it needs no build to exist.
        .route("/", get(interface))
        .route("/ops", get(ops_page))
        .route("/health", get(health))
        .route("/v1/funnel", get(funnel))
        .route("/v1/tokens/{mint}", get(token))
        .route("/v1/store", get(store_counts))
        .route("/v1/events", get(events))
        .route("/v1/instruments", get(list_instruments))
        .route("/v1/instruments/{name}", post(call_instrument))
        .route("/mcp", post(mcp_endpoint))
        // Anything else is either a built asset or a route the interface owns.
        // Placed last so every named route above wins.
        //
        // Registered for any method rather than `get`, because a method-scoped
        // fallback answers a POST to an unknown path with 405 — which says the
        // path exists and only the verb is wrong. For the unconfigured paid
        // routes that is a leak: they are supposed not to exist at all, and a
        // paywall that admits its own shape is halfway to one that fails open.
        .fallback(interface);

    if state.chat.is_some() {
        router = router.route("/v1/chat", post(chat::ask));
    }

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

/// The compiled interface, or a built asset.
///
/// Only reads. Anything else is not found, so an unrouted `POST` is answered
/// the same way an unrouted path is rather than as a method error — see the
/// note on the fallback registration.
async fn interface(method: axum::http::Method, uri: axum::http::Uri) -> Response {
    if method != axum::http::Method::GET && method != axum::http::Method::HEAD {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "no such route" })),
        )
            .into_response();
    }
    embed::serve(uri.path())
}

async fn ops_page(State(state): State<Arc<AppState>>) -> Html<String> {
    Html(ops::render(&state))
}

/// Why the store could not name a watermark.
///
/// A small type rather than a ready-made `Response`, because a `Response` is
/// large enough that returning one in an `Err` is a lint — and because the two
/// cases want different status codes. "The store is empty" and "the store is
/// broken" are different answers, and a route that rendered both the same way
/// would send someone to the wrong place.
enum NoWatermark {
    /// Nothing has been recorded yet. A fresh instance, not a fault.
    Empty,
    /// The store could not be read.
    Unreadable(String),
}

impl IntoResponse for NoWatermark {
    fn into_response(self) -> Response {
        match self {
            Self::Empty => (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": "the store has recorded nothing yet" })),
            )
                .into_response(),
            Self::Unreadable(detail) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": detail })),
            )
                .into_response(),
        }
    }
}

/// The watermark every store-reading route answers as of.
fn watermark_of(state: &AppState) -> Result<radar_types::Slot, NoWatermark> {
    match Reader::watermark(&state.store) {
        Ok(Some(slot)) => Ok(slot),
        Ok(None) => Err(NoWatermark::Empty),
        Err(e) => Err(NoWatermark::Unreadable(e.to_string())),
    }
}

/// What Radar has decided, and where it stopped.
async fn funnel(State(state): State<Arc<AppState>>) -> Response {
    let watermark = match watermark_of(&state) {
        Ok(w) => w,
        Err(e) => return e.into_response(),
    };
    let as_of = AsOf::at(watermark);
    // The launch figure is a count, not a read: the funnel needs how many, and
    // decoding every event to learn that took 5.5 seconds against the live
    // store.
    let (Ok(decisions), Ok(launches)) = (
        state.store.read_decisions(as_of),
        state.store.count(radar_store::Table::Launches, as_of),
    ) else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "cannot read the store" })),
        )
            .into_response();
    };
    // The shipped policy, asked rather than assumed.
    let closed = radar_risk::Policy::CLOSED.is_closed();
    Json(api::funnel(&decisions, launches, watermark.get(), closed)).into_response()
}

/// How often the store is asked whether anything moved.
///
/// The recorder stays about five minutes behind the chain and the outcome and
/// decision passes run hourly, so nothing here changes faster than this. Ten
/// seconds is already far more often than the data does; it is short enough
/// that a page feels live and long enough that a dozen open tabs cost nothing.
///
/// Affordable only because reading a watermark stopped costing 3.4 seconds. At
/// the old price this endpoint would have pinned a core on a box shared with
/// two other production services.
const POLL: Duration = Duration::from_secs(10);

/// What an event carries.
///
/// The watermark and the row counts, which is enough for a client to know
/// *that* something changed and cheap enough to compute on a timer. What
/// changed is a separate fetch, because sending the funnel every ten seconds
/// would be sending the same bytes to a page that mostly did not need them.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
struct Tick {
    /// The store's watermark.
    as_of: u64,
    /// Row counts by table.
    counts: api::StoreCounts,
}

impl Tick {
    /// Reads the current state, or `None` if the store cannot answer.
    fn read(state: &AppState) -> Option<Self> {
        let watermark = Reader::watermark(&state.store).ok().flatten()?;
        let counts = api::store_counts(&state.store, AsOf::at(watermark)).ok()?;
        Some(Self {
            as_of: watermark.get(),
            counts,
        })
    }
}

/// A stream of changes to the store.
///
/// **Emits only when something actually moved.** A client that reconnects gets
/// one immediate event so a fresh page is never blank, and after that silence
/// means nothing has happened — which is information, and is why the keep-alive
/// comment is separate from the data.
///
/// The keep-alive matters more than it looks. An idle event stream through a
/// proxy is indistinguishable from a dead one, and both Caddy and the tunnel
/// will close a connection that says nothing for long enough.
async fn events(
    State(state): State<Arc<AppState>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    // `None` as the seed rather than the current state, so the first tick is
    // always sent. A page that connected during a quiet minute would otherwise
    // show nothing at all until the store moved.
    let stream = futures_util::stream::unfold((state, None::<Tick>), |(state, last)| async move {
        loop {
            if let Some(tick) = Tick::read(&state)
                && last.as_ref() != Some(&tick)
            {
                let event = Event::default()
                    .event("store")
                    .json_data(&tick)
                    .unwrap_or_else(|_| Event::default().comment("unserialisable"));
                return Some((Ok(event), (state, Some(tick))));
            }
            tokio::time::sleep(POLL).await;
        }
    });

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("radar"),
    )
}

/// Everything recorded about one token.
async fn token(State(state): State<Arc<AppState>>, Path(mint): Path<String>) -> Response {
    let watermark = match watermark_of(&state) {
        Ok(w) => w,
        Err(e) => return e.into_response(),
    };
    match api::token_evidence(&state.store, &mint, AsOf::at(watermark)) {
        Ok(evidence) => Json(evidence).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// How many rows each table holds.
async fn store_counts(State(state): State<Arc<AppState>>) -> Response {
    let watermark = match watermark_of(&state) {
        Ok(w) => w,
        Err(e) => return e.into_response(),
    };
    match api::store_counts(&state.store, AsOf::at(watermark)) {
        Ok(counts) => Json(counts).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
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
/// Challenges with `402` when no payment is presented, then verifies and settles
/// through the configured facilitator before the answer leaves the process.
///
/// The order is deliberate. The instrument runs *before* settlement, so a caller
/// is never charged for a call that was going to fail — and the response is held
/// until settlement succeeds, so a caller who disconnects has not been given the
/// answer for free. Every failure in between refuses; see [`facilitator`].
async fn paid_instrument(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
    body: Option<Json<Value>>,
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

    let Some(payment) = x402::payment_header(&headers) else {
        return (
            StatusCode::PAYMENT_REQUIRED,
            Json(config.challenge(&resource, spec.summary, price)),
        )
            .into_response();
    };

    // Verification first, before any work. Otherwise an unpaid caller could use
    // the paid route as a free health check: the status alone would say whether
    // the instrument was going to answer.
    //
    // Blocking, because the facilitator client is synchronous and holding an
    // executor thread through a network round trip would stall every other
    // request this process is serving.
    let owned = config.clone();
    let summary = spec.summary.to_owned();
    let for_verify = resource.clone();
    let verified = match tokio::task::spawn_blocking(move || {
        facilitator::verify(&owned, &payment, &for_verify, &summary, price.get())
    })
    .await
    {
        Ok(Ok(v)) => v,
        Ok(Err(rejected)) => return refused(&rejected),
        // A panic in the blocking task. The payment state is unknown, and
        // serving on an unknown is serving free.
        Err(e) => return refused(&facilitator::Rejected::Unreachable(e.to_string())),
    };

    // Only now is the work done, and only a successful answer is charged for.
    let args = body.map_or_else(|| json!({}), |Json(v)| v);
    let answer = invoke(&state, &name, args);
    if !answer.status().is_success() {
        return answer;
    }

    let owned = config.clone();
    let settled = tokio::task::spawn_blocking(move || facilitator::settle(&owned, &verified)).await;

    match settled {
        Ok(Ok(receipt)) => {
            let mut response = answer;
            // The receipt names the transaction that paid for this, so a dispute
            // is settled by looking at the chain rather than by trusting either
            // party's log.
            if let Ok(value) = HeaderValue::from_str(&receipt.transaction) {
                response.headers_mut().insert("x-payment-response", value);
            }
            if let Ok(value) = HeaderValue::from_str(&receipt.payer) {
                response.headers_mut().insert("x-payment-payer", value);
            }
            response
        }
        Ok(Err(rejected)) => refused(&rejected),
        Err(e) => refused(&facilitator::Rejected::Unreachable(e.to_string())),
    }
}

/// The body for a payment that was not accepted.
fn refused(rejected: &facilitator::Rejected) -> Response {
    (
        StatusCode::PAYMENT_REQUIRED,
        Json(json!({
            "error": rejected.reason(),
            "retryable": rejected.is_retryable(),
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
