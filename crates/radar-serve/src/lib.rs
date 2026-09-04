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

pub mod access;
pub mod admission;
pub mod api;
pub mod cache;
pub mod challenges;
pub mod chat;
pub mod customer;
mod embed;
pub mod evidence;
pub mod facilitator;
pub mod ledger;
pub mod link;
pub mod mcp;
mod ops;
pub mod privy;
pub mod share;
pub mod siws;
pub mod x402;

use core::convert::Infallible;
use core::time::Duration;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
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
    /// Whether requests are checked against Cloudflare Access.
    ///
    /// There is no `Option` here and no default: `access::Mode::from_vars`
    /// refuses to start unless the operator either configured Access or wrote
    /// `RADAR_ACCESS=off` in as many words. Both possible defaults are wrong in
    /// a way that is invisible.
    pub access: access::Mode,
    /// Cloudflare's published signing keys, fetched on demand.
    pub keys: access::KeyCache,
    /// Whether this instance has a customer lane, and for which Privy app.
    ///
    /// `Off` is the shipped state and is not a degradation: with no customer
    /// authenticator a customer route requires operator identity, which is
    /// strictly more restrictive than it will be.
    pub customer: customer::Mode,
    /// How customers' wallets are read, when this instance has a credential.
    ///
    /// `None` is the shipped state and means wallet lookup is unavailable --
    /// **not** that customers have no wallets. Rule 8: those two are
    /// indistinguishable to a caller that collapses them, and only one is safe
    /// to act on.
    pub privy: Option<privy::Client>,
    /// Privy's published signing keys, fetched on demand.
    ///
    /// Separate from the operator's cache: different issuers on different
    /// rotation schedules, and one cache holding both would let a fetch failure
    /// for one refuse tokens for the other.
    pub customer_keys: customer::KeyCache,
    /// Which verified customers this instance lets in.
    ///
    /// Separate from `customer`, which decides whether a token is *genuine*.
    /// This decides whether the identity in a genuine token is one of ours, and
    /// nothing asked that before: the product was private only because no
    /// customer authenticator was configured at all.
    pub admission: admission::Admission,
    /// How much of the day's model budget any one customer may take.
    ///
    /// Separate from the agent's global budget rather than replacing it. The
    /// global one bounds what Radar spends; this bounds what any single customer
    /// can take of it, which is a different question and was unasked.
    pub shares: share::Shares,
    /// The per-instance salt customer identifiers are hashed with.
    ///
    /// Empty when unconfigured, which is a refusal rather than a fallback: an
    /// unsalted hash of a DID is a stable identifier anyone holding a DID can
    /// recompute, and `Subject::derive` refuses it. Rule 8.
    pub customer_salt: Vec<u8>,
    /// The one credential-linking flow that may be in progress.
    pub linker: link::Linker,
    /// The scoreboard, computed once per watermark.
    ///
    /// It scans the whole store -- 6,185 decisions against 1,147,649 outcomes
    /// when measured -- for an answer identical for every caller at a given
    /// watermark. Keyed on that watermark, so a replay at an older `AsOf` is
    /// never handed today's answer (rule 3).
    pub scoreboard: cache::Cache<radar_research::selection::Report>,
    /// One mint's evidence, computed once per watermark.
    ///
    /// Keyed on the watermark **and** the mint, holding the most recently asked
    /// one only. A map keyed on the mint would be unbounded and reachable by
    /// anyone who can name one, and the page this serves is read one token at a
    /// time.
    pub token: cache::Cache<api::TokenEvidence, String>,
    /// Outstanding sign-in challenges, or `None` when no customer domain is set.
    ///
    /// `None` is rule 8's shape: an instance that does not know its own domain
    /// cannot bind a signature to itself, so it refuses to issue challenges
    /// rather than issuing ones that would authenticate against any site.
    pub challenges: Option<challenges::Challenges>,
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
        .route("/v1/decisions", get(decisions))
        .route("/v1/evidence/capacity", get(capacity))
        .route("/v1/evidence/returns", get(returns))
        .route("/v1/evidence/activity", get(activity))
        .route("/v1/tokens/{mint}", get(token))
        .route("/v1/store", get(store_counts))
        .route("/v1/analyst/replies", get(analyst_replies))
        .route("/v1/scoreboard", get(scoreboard))
        .route("/v1/customer/config", get(customer_config))
        .route("/v1/customer/siws/challenge", post(siws::challenge))
        .route("/v1/customer/siws/verify", post(siws::verify))
        .route("/v1/customer/wallet", get(customer_wallet))
        .route("/v1/events", get(events))
        .route("/v1/customer/events", get(customer_events))
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
        router = router
            .route("/v1/chat", post(chat::ask))
            .route("/v1/link", post(link::begin))
            .route("/v1/link", get(link::status));
    }

    if state.x402.is_some() {
        router = router
            .route("/x402/v1/instruments", get(list_instruments))
            .route("/x402/v1/instruments/{name}", post(paid_instrument));
    }

    router
        .layer(axum::middleware::from_fn_with_state(
            Arc::clone(&state),
            guard,
        ))
        .with_state(state)
}

/// Which Privy application the interface should authenticate against.
///
/// **Public, and it has to be**: this is what the frontend reads *before* a
/// customer has any token at all, so gating it behind a customer token would
/// make logging in require being logged in.
///
/// The app id is a public client identifier — it ships inside the JavaScript
/// bundle of every Privy application there is, and holding one authorises
/// nothing. What is emphatically not here is the app *secret*, which is the
/// credential, lives only in the server's environment, and is what
/// `state.privy` holds.
///
/// Nothing else is returned. A bootstrap endpoint that also reported store
/// counts or instance health would be an operator surface wearing a public
/// one's clothes, which is exactly the mistake `audience_of` exists to prevent.
async fn customer_config(State(state): State<Arc<AppState>>) -> Response {
    let Some(config) = state.customer.config() else {
        // Rule 8. An instance with no customer authentication configured says
        // so, rather than serving a page that will fail at the login button
        // with something less legible.
        return chat::refuse(
            StatusCode::SERVICE_UNAVAILABLE,
            "this instance has no customer authentication configured",
        );
    };
    Json(serde_json::json!({ "privy_app_id": config.app_id })).into_response()
}

/// The signed-in customer's Solana wallet.
///
/// # Why this reads Privy on every request
///
/// [ADR 0006](https://github.com/hey-vera/radar/blob/main/docs/adr/0006-radar-records-only-what-it-cannot-recover.md).
/// Privy is authoritative for the address, and a stale cached one is an address
/// Radar might show a customer as their deposit destination after they no longer
/// control it.
///
/// # What it deliberately does not do
///
/// It does not create a wallet, and it does not ask for a signer grant. Both are
/// the customer's actions, taken in their own session against Privy — a server
/// that could grant itself a signer is not a bounded signer.
async fn customer_wallet(
    State(state): State<Arc<AppState>>,
    request: axum::extract::Request,
) -> Response {
    // The identity the guard verified, and the only source of a DID here. A DID
    // taken from a path or a query would let any caller read any customer's
    // wallet, which is the whole of the authorisation on this route.
    let Some(customer) = request.extensions().get::<customer::Customer>().cloned() else {
        return chat::refuse(
            StatusCode::FORBIDDEN,
            "no verified customer on this request",
        );
    };

    let Some(client) = state.privy.as_ref() else {
        // Rule 8, and the distinction matters to whoever reads it: this instance
        // cannot look wallets up, which is not the same as this customer having
        // no wallet.
        return chat::refuse(
            StatusCode::SERVICE_UNAVAILABLE,
            "this instance has no Privy application credential configured",
        );
    };

    match tokio::task::block_in_place(|| client.wallet_for(&customer.did)) {
        Ok(wallet) => Json(serde_json::json!({
            "address": wallet.address,
            // Reported rather than acted on. The frontend needs it to know
            // whether to show the customer a grant prompt, and nothing here
            // treats it as permission -- that check belongs where money moves.
            "delegated": wallet.delegated,
        }))
        .into_response(),
        Err(privy::Unavailable::NoWallet) => Json(serde_json::json!({
            // A real answer: a customer who just signed up has no wallet yet,
            // and the frontend needs to tell that apart from a failure.
            "address": serde_json::Value::Null,
            "delegated": false,
        }))
        .into_response(),
        Err(why) => chat::refuse(StatusCode::BAD_GATEWAY, &why.to_string()),
    }
}

/// Refuses anything that cannot prove who it is.
///
/// Placed as a layer rather than per-route on purpose: a per-route check is one
/// somebody forgets on the route they add next year, and the route they add next
/// year is the one that matters.
///
/// The audience comes from [`access::audience_of`], which is total and falls
/// back to [`access::Audience::Operator`] — so a route nobody classified is
/// treated as the most sensitive thing it could be. [`access::Audience::Customer`]
/// currently requires the same operator check, because no customer
/// authenticator exists and an audience with nothing behind it must fall to the
/// strictest available rather than the loosest (rule 8).
async fn guard(
    State(state): State<Arc<AppState>>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let audience = access::audience_of(request.uri().path());
    if audience.is_open() {
        return next.run(request).await;
    }

    // Set when a token verified but names an identity this instance does not
    // admit. Carried so the eventual refusal can name the actual problem: a
    // reader told only "denied" cannot tell a private instance from a broken
    // login, and the two want opposite responses.
    let mut not_admitted: Option<String> = None;

    // A customer token is tried first, and only where the route is product.
    // `accepts_customer` is false for every operator route, so a valid customer
    // token cannot reach `/v1/store` or `/mcp` however well formed it is.
    if audience.accepts_customer()
        && let Some(config) = state.customer.config()
        && let Some(token) = customer::token_from(request.headers())
    {
        // A wallet session first. Under ADR 0011's amendment this is the lane
        // that ships, and a bearer token is one or the other -- the two formats
        // cannot be confused, because a session token is two base64url parts
        // separated by a dot with no header, and a JWT has three.
        if let Ok(session) =
            radar_customer::session::verify(&token, &state.customer_salt, now_unix())
        {
            // The address is the identity. It goes in the same `Customer` the
            // Privy path produces, so admission, metering and every handler
            // downstream work unchanged and there is one notion of "who is
            // calling" rather than two that can disagree.
            //
            // The two namespaces cannot collide: a Privy subject is
            // `did:privy:...` and this is base58.
            let customer = customer::Customer {
                did: session.address.to_string(),
                session: session.expires_at.to_string(),
            };
            if state.admission.admits(&customer.did) {
                let mut request = request;
                request.extensions_mut().insert(customer);
                return next.run(request).await;
            }
            not_admitted = Some(customer.did);
        }

        let verified = tokio::task::block_in_place(|| {
            let keys = state.customer_keys.get(config)?;
            customer::verify(&token, &keys, config, now_unix())
        });
        if let Ok(customer) = verified {
            // Genuine, and now: is this one of ours? `verify` proves Privy
            // issued the token for this application, which is authentication.
            // Anyone can sign up to a Privy application, so a verified stranger
            // is a thing that exists and must not be a customer of this
            // instance while it is private.
            if state.admission.admits(&customer.did) {
                // The verified identity travels in the request, so a handler
                // never re-parses a token. Two parses of one token are two
                // chances to disagree about who is calling, and the handler's
                // copy would be the one nothing checked a signature on.
                //
                // Inserted only after a successful verification, so an extension
                // being present *is* the proof rather than something to
                // re-check.
                let mut request = request;
                request.extensions_mut().insert(customer);
                return next.run(request).await;
            }
            // Not admitted. Fall through to the operator check for the same
            // reason an unverified token does -- an operator debugging with
            // their own session must not be locked out by whichever bearer token
            // their browser happened to send -- but remember who it was, so the
            // refusal below can say something an operator can act on rather
            // than "denied".
            not_admitted = Some(customer.did);
        }
        // A customer token that does not verify falls through to the operator
        // check rather than refusing here. That is deliberate: an operator
        // debugging with their own session should not be locked out by a stale
        // bearer token their browser happened to send.
    }

    let access::Mode::Enforce(config) = &state.access else {
        // No operator check either. A verified customer who is not admitted must
        // still be refused here, or an instance with Access off would admit
        // every Privy identity in the world through the front door it just
        // closed.
        if let Some(did) = not_admitted {
            return private(&did);
        }
        return next.run(request).await;
    };

    let Some(token) = access::token_from(request.headers()) else {
        if let Some(did) = not_admitted {
            return private(&did);
        }
        return denied(&access::Denied::Missing);
    };

    // Blocking: fetching the key set is one HTTPS call an hour, and the
    // signature check is microseconds. `block_in_place` rather than pretending
    // either is async.
    let verified = tokio::task::block_in_place(|| {
        let keys = state.keys.get(config)?;
        access::verify(&token, &keys, config, now_unix())
    });

    match verified {
        Ok(_) => next.run(request).await,
        Err(why) => match not_admitted {
            // The admission problem is the more useful diagnosis: the operator
            // check failing is expected for a customer, and reporting that
            // instead would send them to Cloudflare.
            Some(did) => private(&did),
            None => denied(&why),
        },
    }
}

/// Refuses a verified identity this instance does not admit.
///
/// Names the identity, which is the whole point: the operator has to be able to
/// add it, and nobody can look up their own DID from a login screen. It reveals
/// nothing to the caller that the caller did not already send.
fn private(did: &str) -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(json!({
            "error": "this instance is private",
            "identity": did,
            "remedy": format!(
                "add this identity to {} to grant access",
                admission::VAR
            ),
        })),
    )
        .into_response()
}

/// Seconds since the epoch, for the expiry check.
///
/// The clock the whole expiry check rests on, which is why it is asserted
/// against a range rather than merely called. A version of this returning zero
/// makes every token unexpired forever, and the interface would look entirely
/// healthy while accepting an assertion issued to somebody who left the company
/// last year.
#[must_use]
pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

/// A refusal that says enough to debug and no more.
///
/// 403 rather than 401: there is no `WWW-Authenticate` scheme Radar could name,
/// and Cloudflare Access is what issues credentials. A 401 would invite a
/// browser password prompt for an account that does not exist.
fn denied(why: &access::Denied) -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(json!({ "error": why.to_string() })),
    )
        .into_response()
}

/// What Radar's selection returned, against its own refusals.
///
/// The cost is a query parameter with a documented default rather than a
/// hardcoded constant, because it is an assumption and the reader should be able
/// to move it and watch the answer move.
async fn scoreboard(State(state): State<Arc<AppState>>) -> Response {
    let Ok(Some(watermark)) = state.store.watermark() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "the store has recorded nothing yet" })),
        )
            .into_response();
    };
    // A recent-enough answer is served rather than recomputed, labelled with the
    // watermark it was computed at. See `MAX_STALE_SLOTS`.
    if let Some((as_of, report)) = state.scoreboard.recent(watermark, &(), MAX_STALE_SLOTS) {
        return Json(WithWatermark {
            as_of: as_of.get(),
            body: &*report,
        })
        .into_response();
    }
    let computed = state.scoreboard.get_or_compute(watermark, (), || {
        api::scoreboard(&state.store, AsOf::at(watermark), api::ASSUMED_COST_BPS)
    });
    match computed {
        Ok(report) => Json(WithWatermark {
            as_of: watermark.get(),
            body: &*report,
        })
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
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
        // The deciding policy, so the interface reports it rather than asserting
        // it. The health screen printed "policy closed" as literal text, which
        // is the same shape as the four backend instances this replaced: a
        // claim that could not stop being made.
        //
        // Scope, on the field because the name invites more than it means: this
        // is `Policy::SHIPPED`, what `radar consider` judges against. The signer
        // holds its own policy (ADR 0008) and can refuse what this one permits.
        "policyClosed": radar_risk::Policy::SHIPPED.is_closed(),
        // The agent's own account of itself, so `radar brief` can alarm on it
        // from the probe it already makes rather than by opening a second
        // connection to a component that might be the thing that is down.
        "agent": chat::status(state.chat.as_ref()),
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

/// How far behind the current watermark a cached answer may be and still be
/// served.
///
/// 150 slots is about a minute. The recorder advances the watermark roughly
/// every fifty seconds, so keyed exactly, the cache missed about half the time
/// and each miss cost 4.4 to 5.2 seconds against a 500ms budget. An answer
/// about thousands of historical decisions does not change in a minute --
/// outcomes are measured hourly -- so this trades a minute of staleness, stated
/// on every response, for the budget.
///
/// It is deliberately not a duration. The store counts in slots, and converting
/// would need a clock the read path does not have and a slot time that is a
/// property of the network rather than of Radar.
const MAX_STALE_SLOTS: u64 = 150;

/// A response, with the watermark it was actually computed at.
///
/// `flatten` so the body keeps its shape and only gains a field: existing
/// readers are unaffected, and every reader can now tell how old the answer is.
///
/// This type exists so that serving a stale answer without saying so is not
/// something a handler can do by forgetting. An older answer rendered as the
/// current one is rule 9's shape -- unknown presented as fresh -- and the whole
/// argument for serving one at all is that it is labelled.
#[derive(Serialize)]
struct WithWatermark<'a, T> {
    /// The watermark the body was computed at, which may be behind the store's.
    as_of: u64,
    #[serde(flatten)]
    body: &'a T,
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
    // The policy this build decides with.
    //
    // This read `Policy::CLOSED.is_closed()` under a comment claiming it was
    // "asked rather than assumed". It asked a constant named `CLOSED` whether
    // it was closed, so it answered `true` unconditionally — including on the
    // day somebody opened the real policy in `radar consider`, which held its
    // own separate copy. The interface's strongest safety claim could not move
    // with the thing it described, and it failed towards reassurance.
    //
    // `Policy::SHIPPED` is the constant the decider uses. One constant cannot
    // diverge from itself. It still says nothing about the signer's own policy
    // (ADR 0008), which lives in another process and is documented as out of
    // scope on the field itself.
    let closed = radar_risk::Policy::SHIPPED.is_closed();
    Json(api::funnel(&decisions, launches, watermark.get(), closed)).into_response()
}

/// The capacity wall.
///
/// The reason the product does not work yet, in one picture: 80% of proposals
/// sit in a ±13% band around $31, because every pre-graduation pump.fun token
/// rides the same bonding curve. It is an aggregate rather than rows because
/// ~938k measurements must never reach a browser.
async fn capacity(State(state): State<Arc<AppState>>) -> Response {
    let watermark = match watermark_of(&state) {
        Ok(w) => w,
        Err(e) => return e.into_response(),
    };
    let Ok(decisions) = state.store.read_decisions(AsOf::at(watermark)) else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "cannot read the store" })),
        )
            .into_response();
    };
    Json(api::capacity(
        &decisions,
        watermark.get(),
        api::ASSUMED_COST_BPS,
    ))
    .into_response()
}

/// What the selection returned, as a distribution rather than a median.
///
/// The median is already on `/v1/scoreboard`. This exists because a median over
/// this population is a report about a point mass: 24–43% of it returns exactly
/// zero, which is research 0017's central caveat and the thing a single figure
/// cannot show.
async fn returns(State(state): State<Arc<AppState>>) -> Response {
    let watermark = match watermark_of(&state) {
        Ok(w) => w,
        Err(e) => return e.into_response(),
    };
    let as_of = AsOf::at(watermark);
    let (Ok(decisions), Ok(outcomes)) = (
        state.store.read_decisions(as_of),
        state.store.read_outcomes(as_of),
    ) else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "cannot read the store" })),
        )
            .into_response();
    };
    Json(api::returns(
        &decisions,
        &outcomes,
        watermark.get(),
        api::ASSUMED_COST_BPS,
    ))
    .into_response()
}

/// How many decisions were taken per day, most recent fortnight.
///
/// Its obvious job is answered by the watermark on the operator's page. This is
/// the same question asked in a form that shows a **gap**: a watermark tells you
/// where the recorder got to, and a row of buckets tells you it stopped on
/// Tuesday.
async fn activity(State(state): State<Arc<AppState>>) -> Response {
    /// A fortnight. Long enough for a weekly rhythm to be visible and short
    /// enough that a gap is obvious rather than a thin line among many.
    const DAYS: u64 = 14;

    let watermark = match watermark_of(&state) {
        Ok(w) => w,
        Err(e) => return e.into_response(),
    };
    let Ok(decisions) = state.store.read_decisions(AsOf::at(watermark)) else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "cannot read the store" })),
        )
            .into_response();
    };
    Json(api::activity(&decisions, watermark.get(), DAYS)).into_response()
}

/// The decision record, newest first, one page at a time.
///
/// # Why this exists beside `/v1/funnel`
///
/// The funnel is an aggregate, and an aggregate barely moves: roughly 960
/// decisions a day fall into the same handful of reason buckets, so a reader
/// coming back tomorrow learns nothing from it they did not know today. What
/// changes is *which* tokens were refused and *why*, and that is this route.
///
/// It is also the one screen no competitor can build, because no competitor
/// records a reason list per decision.
///
/// # The cursor
///
/// `after=<slot>:<mint>`, and the mint half is load-bearing rather than
/// decorative. See [`api::Cursor`].
async fn decisions(
    State(state): State<Arc<AppState>>,
    Query(params): Query<DecisionParams>,
) -> Response {
    let watermark = match watermark_of(&state) {
        Ok(w) => w,
        Err(e) => return e.into_response(),
    };
    let as_of = AsOf::at(watermark);

    let Ok(all) = state.store.read_decisions(as_of) else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "cannot read the store" })),
        )
            .into_response();
    };

    // A cursor that does not parse is refused rather than ignored. Ignoring it
    // silently restarts the reader at the newest page, which looks like the
    // record looping rather than like a bad request.
    let after = match params.after.as_deref() {
        None => None,
        Some(raw) => match api::Cursor::parse(raw) {
            Some(cursor) => Some(cursor),
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": "after must be <slot>:<mint>" })),
                )
                    .into_response();
            }
        },
    };

    let conclusion = match params.conclusion.as_deref() {
        None => None,
        Some("proposed") => Some(radar_store::Conclusion::Proposed),
        Some("passed") => Some(radar_store::Conclusion::Passed),
        Some(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "conclusion must be proposed or passed" })),
            )
                .into_response();
        }
    };

    let query = api::Query {
        after,
        reason: params.reason,
        conclusion,
        // Trimmed and dropped when empty. `?prefix=` is a stray parameter, not a
        // request for decisions whose mint starts with nothing -- which every
        // decision does, so it would look like the filter had silently failed.
        prefix: api::normalise_prefix(params.prefix),
        limit: params.limit.unwrap_or(api::DEFAULT_LIMIT),
    };

    Json(api::page(all, &query, watermark.get())).into_response()
}

/// The query string `/v1/decisions` accepts.
#[derive(serde::Deserialize)]
struct DecisionParams {
    after: Option<String>,
    prefix: Option<String>,
    reason: Option<String>,
    conclusion: Option<String>,
    limit: Option<usize>,
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
    changes(state, Tick::read)
}

/// The watermark, and nothing else.
///
/// What a customer's page needs in order to know the store moved. Deliberately
/// **not** [`Tick`], which carries row counts: those are the operator's view of
/// the instance and there is no product reason for a customer to have them.
#[derive(PartialEq, Eq, Debug, serde::Serialize)]
struct Watermark {
    as_of: u64,
}

impl Watermark {
    fn read(state: &AppState) -> Option<Self> {
        Reader::watermark(&state.store)
            .ok()
            .flatten()
            .map(|slot| Self { as_of: slot.get() })
    }
}

/// A stream of changes, for a customer.
///
/// # Why this exists rather than opening `/v1/events`
///
/// `/v1/events` is `Audience::Operator` because its payload is the operator's
/// store counts. Reclassifying it would hand those to every customer to save
/// writing forty lines, which is the trade that puts an operator surface in
/// front of a paying stranger.
///
/// The interface needs one bit from it — *did the store move* — so that is what
/// this sends. Without it the funnel would keep rendering its first fetch
/// forever on the day the customer lane switches on, and a page that cannot see
/// changes must not look like a page where nothing has changed.
async fn customer_events(
    State(state): State<Arc<AppState>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    changes(state, Watermark::read)
}

/// The machinery both change streams share.
///
/// Extracted rather than copied, because the two properties worth having here
/// are easy to lose in a second copy: the seed is `None` so a page that connects
/// during a quiet minute is not blank, and the keep-alive is a comment rather
/// than data so silence still means "nothing happened" instead of "this
/// connection died".
fn changes<T, F>(
    state: Arc<AppState>,
    read: F,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>>
where
    T: PartialEq + serde::Serialize + Send + 'static,
    F: Fn(&AppState) -> Option<T> + Send + 'static,
{
    // `None` as the seed rather than the current state, so the first tick is
    // always sent. A page that connected during a quiet minute would otherwise
    // show nothing at all until the store moved.
    let stream =
        futures_util::stream::unfold((state, None::<T>, read), |(state, last, read)| async move {
            loop {
                if let Some(tick) = read(&state)
                    && last.as_ref() != Some(&tick)
                {
                    let event = Event::default()
                        .event("store")
                        .json_data(&tick)
                        .unwrap_or_else(|_| Event::default().comment("unserialisable"));
                    return Some((Ok(event), (state, Some(tick), read)));
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
    // Keyed on the mint as well as the watermark, so a hit can never be another
    // token's evidence.
    if let Some((as_of, evidence)) = state.token.recent(watermark, &mint, MAX_STALE_SLOTS) {
        return Json(WithWatermark {
            as_of: as_of.get(),
            body: &*evidence,
        })
        .into_response();
    }
    let computed = state.token.get_or_compute(watermark, mint.clone(), || {
        api::token_evidence(&state.store, &mint, AsOf::at(watermark))
    });
    match computed {
        Ok(evidence) => Json(WithWatermark {
            as_of: watermark.get(),
            body: &*evidence,
        })
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// What the public analyst has said, and what it only decided to say.
///
/// # Operator, by falling through rather than by being listed
///
/// `audience_of` classifies the customer routes explicitly and everything else
/// falls to `Audience::Operator`. This route is not in that list, so it is
/// operator-only without a line being added — which is the fallback doing its
/// job. It stays that way deliberately: the reply log carries the fact sheet
/// behind every answer, and that is an operator's working material rather than
/// a public artefact.
///
/// # It reads the log, never the store
///
/// The analyst does not write to the store and this does not read from it. A
/// reply is a live observation about a mint at a slot, not a recorded fact about
/// the chain, and putting one where a replay reads would be rule 3 broken for
/// the sake of a convenient join.
///
/// Folded, so one reply is one row. `publish` appends twice — once before it
/// says anything and once after — and a page counting raw lines would report
/// every answer twice.
async fn analyst_replies() -> Response {
    let dir = std::env::var("RADAR_ANALYST_DIR").unwrap_or_else(|_| "data/analyst".to_owned());
    analyst_replies_in(&dir)
}

/// The handler, over a directory it is given.
///
/// Split from the route so it can be tested without setting a process-wide
/// environment variable that parallel tests would fight over -- the same shape
/// the daemon's config readers use.
fn analyst_replies_in(dir: &str) -> Response {
    let log = format!("{dir}/replies.jsonl");

    let Ok(entries) = radar_analyst::log::latest(&log) else {
        // Not an error. An instance with no analyst running has no log, and
        // reporting that as a failure would make an ordinary configuration look
        // like a broken one. The count says which.
        return Json(json!({
            "log": log,
            "running": false,
            "answered": 0,
            "published": 0,
            "replies": [],
        }))
        .into_response();
    };

    let published = entries.iter().filter(|e| e.reply_id.is_some()).count();
    // Newest first: an operator opening this wants the last thing the account
    // said, not the first.
    let mut replies: Vec<_> = entries.iter().collect();
    // Descending, so `Reverse` rather than a comparator: clippy is right that a
    // key is clearer, and the key here is "how recent", inverted.
    replies.sort_by_key(|e| std::cmp::Reverse(e.at));
    replies.truncate(200);

    Json(json!({
        "log": log,
        "running": true,
        "answered": entries.len(),
        "published": published,
        "replies": replies,
    }))
    .into_response()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_the_expiry_check_rests_on_reads_a_plausible_time() {
        // A range known in advance rather than a lower bound. `> 0` passes for
        // milliseconds, for nanoseconds, and for a constant of 1 -- and the
        // failure mode of a wrong clock here is not a crash, it is expired
        // assertions being accepted while everything looks healthy.
        let now = now_unix();
        assert!(
            (1_750_000_000..2_000_000_000).contains(&now),
            "seconds since 1970 is ~1.77e9 in 2026 and ~2.0e9 in 2033; got {now}"
        );
    }

    /// Reads a handler's JSON body.
    async fn body_of(response: Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .expect("a body");
        serde_json::from_slice(&bytes).expect("json")
    }

    #[tokio::test]
    async fn an_instance_with_no_analyst_says_so_rather_than_failing() {
        // A missing log is an ordinary configuration -- most instances do not
        // run the analyst -- and reporting it as an error would make that look
        // like a fault. `running` is what tells the two apart, and a page that
        // read only `answered` could not.
        let dir = std::env::temp_dir().join(format!("radar-noanalyst-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp dir");

        let body = body_of(analyst_replies_in(dir.to_str().expect("a path"))).await;
        assert_eq!(body["running"], false);
        assert_eq!(body["answered"], 0);
        assert_eq!(body["published"], 0);
        assert!(body["replies"].as_array().expect("an array").is_empty());
    }

    #[tokio::test]
    async fn replies_are_folded_newest_first_and_counted_once() {
        // `publish` appends twice per reply, so a handler counting raw lines
        // reports every answer twice. And an operator opening this wants the
        // last thing the account said, not the first.
        let dir = std::env::temp_dir().join(format!("radar-analyst-api-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp dir");
        let log = dir.join("replies.jsonl");
        let log = log.to_str().expect("a path").to_owned();

        for (at, id, published) in [(100_u64, "m1", true), (200, "m2", false)] {
            let mut entry = radar_analyst::Entry {
                at,
                mention_id: id.to_owned(),
                summoner: "a".to_owned(),
                mint: None,
                read_at_slot: None,
                fact_sheet: "evidence".to_owned(),
                reply: "text".to_owned(),
                fellback: None,
                reply_id: None,
            };
            radar_analyst::log::append(&log, &entry).expect("intent");
            if published {
                entry.reply_id = Some(format!("r-{id}"));
            }
            radar_analyst::log::append(&log, &entry).expect("outcome");
        }

        let body = body_of(analyst_replies_in(dir.to_str().expect("a path"))).await;
        assert_eq!(body["running"], true);
        assert_eq!(body["answered"], 2, "four lines are two replies");
        assert_eq!(body["published"], 1, "one of them was posted");
        let replies = body["replies"].as_array().expect("an array");
        assert_eq!(replies.len(), 2);
        assert_eq!(replies[0]["mention_id"], "m2", "newest first");
        assert_eq!(replies[0]["fact_sheet"], "evidence");
    }

    #[test]
    fn a_refusal_is_forbidden_rather_than_unauthorized() {
        // 401 would invite a browser password prompt for an account that does
        // not exist: Cloudflare Access issues the credential, not Radar, and
        // there is no `WWW-Authenticate` scheme to name.
        let response = denied(&access::Denied::Missing);
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}
