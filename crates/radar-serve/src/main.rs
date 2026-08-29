// SPDX-License-Identifier: Apache-2.0
//! The Radar server.

use std::net::SocketAddr;
use std::process::ExitCode;
use std::sync::Arc;

use radar_instruments::{CreatorHistory, CreatorTrackRecord, Registry, SimulateExit};
use radar_serve::chat::Chat;
use radar_serve::{AppState, access, app, chat, x402};
use radar_store::Reader;

/// Every instrument Radar exposes. The CLI builds the same list.
fn registry() -> Registry {
    let mut r = Registry::new();
    r.register(CreatorHistory);
    r.register(CreatorTrackRecord);
    r.register(SimulateExit::default());
    r
}

/// Builds the agent from the environment, and says what happened either way.
///
/// Rule 8, and the reporting half is the point. An unconfigured agent and a
/// *misconfigured* one both end up as `None`, and they need different responses
/// from whoever is reading the startup log: one is the shipped state, the other
/// is somebody who set four variables out of five and will otherwise conclude
/// the feature does not work.
fn configure_agent() -> (Option<Chat>, String) {
    let Some(budget) = radar_model::budget_from_vars(&|k| std::env::var(k).ok()) else {
        return (
            None,
            "off (no RADAR_MODEL_DAILY_USD; a model with no budget spends without a ceiling)"
                .to_owned(),
        );
    };

    // Built separately from the boxed provider so the route knows whether there
    // is a credential to link *before* somebody presses the button, rather than
    // discovering it from a failure.
    let linkable = radar_model::codex_from_vars(&|k| std::env::var(k).ok());

    match radar_model::from_vars(&|k| std::env::var(k).ok()) {
        Ok(provider) => {
            let name = provider.name();
            let mut allowlist = radar_agent::Allowlist::new();
            // The read-only instrument registry, and nothing else. Every one of
            // these receives a `&Reader` and structurally cannot write.
            for instrument in registry().iter() {
                allowlist.allow(instrument.spec().name);
            }
            let tools = allowlist.len();
            let agent = radar_agent::Agent::new(
                radar_agent::Config { budget, allowlist },
                chat::today_utc(),
            );
            (
                Some(Chat {
                    agent: std::sync::Mutex::new(agent),
                    provider,
                    linkable,
                    last: std::sync::Mutex::new(radar_serve::chat::LastCall::Never),
                }),
                // Integer arithmetic, because a startup line reporting the
                // ceiling as `$2.00` when it is `$2.004` is a line an operator
                // would reasonably quote back later.
                format!(
                    "on via {name}, {tools} read-only tool(s), ${}.{:06}/day",
                    budget.daily_max.get() / 1_000_000,
                    budget.daily_max.get() % 1_000_000
                ),
            )
        }
        // Printed rather than swallowed. A misconfiguration that produces
        // silence is one an operator debugs by reading source.
        Err(why) => (None, format!("off — {why}")),
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let store_dir = std::env::var("RADAR_STORE").unwrap_or_else(|_| "./data/store".to_owned());
    let bind: SocketAddr = std::env::var("RADAR_BIND")
        .unwrap_or_else(|_| "127.0.0.1:8080".to_owned())
        .parse()
        .unwrap_or_else(|_| SocketAddr::from(([127, 0, 0, 1], 8080)));

    // Before anything binds a socket. A server that starts and then discovers
    // it does not know who may look has already answered a request by then.
    let access = match access::Mode::from_vars(&|k| std::env::var(k).ok()) {
        Ok(mode) => mode,
        Err(why) => {
            eprintln!("radar-serve: {why}");
            return ExitCode::FAILURE;
        }
    };

    let x402 = x402::Config::from_env();
    let (agent, agent_note) = configure_agent();
    let state = Arc::new(AppState {
        registry: registry(),
        store: Reader::open(&store_dir),
        x402,
        chat: agent,
        access: access.clone(),
        keys: access::KeyCache::new(),
        linker: radar_serve::link::Linker::new(),
    });

    println!("radar-serve v{}", env!("CARGO_PKG_VERSION"));
    println!("  store      : {store_dir}");
    println!("  instruments: {}", state.registry.len());
    println!(
        "  paid surface: {}",
        if state.x402.is_some() {
            "on"
        } else {
            "off (set RADAR_X402_PAY_TO and RADAR_X402_FACILITATOR to enable)"
        }
    );
    println!(
        "  access     : {}",
        match &access {
            access::Mode::Enforce(config) => format!("verifying {} tokens", config.team_domain),
            // Said plainly, every start. An instance serving operational detail
            // to anyone who can reach it should say so in its own logs.
            access::Mode::Off => "OFF — anyone who can reach this can read it".to_owned(),
        }
    );
    println!("  agent      : {agent_note}");
    println!("  listening  : http://{bind}");

    let listener = match tokio::net::TcpListener::bind(bind).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("cannot bind {bind}: {e}");
            return ExitCode::FAILURE;
        }
    };

    if let Err(e) = axum::serve(listener, app(state)).await {
        eprintln!("server stopped: {e}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
