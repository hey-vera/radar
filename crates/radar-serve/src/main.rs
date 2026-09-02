// SPDX-License-Identifier: Apache-2.0
//! The Radar server.

use std::net::SocketAddr;
use std::process::ExitCode;
use std::sync::Arc;

use radar_instruments::{CreatorHistory, CreatorTrackRecord, Registry, SimulateExit};
use radar_serve::chat::Chat;
use radar_serve::{AppState, access, app, chat, customer, x402};
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

    // Rule 8, and this one is not decoration. A meter that cannot record what it
    // spent cannot enforce a ceiling across a restart, so an agent with no
    // durable ledger is an unmetered spender wearing a meter's clothes -- which
    // is the state this ran in until the ledger was wired, because
    // `Agent::restore` had one caller and it was a unit test.
    let ledger = match radar_serve::ledger::Store::open(&|k| std::env::var(k).ok()) {
        Ok(ledger) => ledger,
        Err(why) => return (None, format!("off ({why})")),
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
            let today = chat::today_utc();
            let config = radar_agent::Config { budget, allowlist };
            // Restored rather than reset. A ledger from an earlier day is not
            // carried forward by `Meter::restore` -- the budget is daily -- so
            // this is safe to do unconditionally and does the right thing on the
            // first start of a new day.
            let agent = ledger
                .read::<radar_agent::Ledger>(chat::LEDGER_RECORD)
                .map_or_else(
                    || radar_agent::Agent::new(config.clone(), today),
                    |saved| radar_agent::Agent::restore(config.clone(), &saved, today),
                );
            (
                Some(Chat {
                    agent: std::sync::Mutex::new(agent),
                    ledger,
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

/// The three settings that govern the customer lane.
///
/// Extracted from `main` because it grew past what one function should hold, and
/// because these three belong together: they are the whole of what decides
/// whether a stranger can reach this instance and what they can spend on it.
///
/// Every one of them fails closed and none of them defaults quietly. A malformed
/// value stops the server rather than resolving to "closed", because the two are
/// indistinguishable once collapsed and an operator would spend the outage
/// looking at the vendor.
///
/// # Errors
///
/// Returns the message to print when any of them cannot be read.
fn customer_lane() -> Result<
    (
        radar_serve::admission::Admission,
        radar_serve::share::Shares,
        Vec<u8>,
    ),
    String,
> {
    let env = |k: &str| std::env::var(k).ok();
    let admission = radar_serve::admission::Admission::from_vars(&env)?;
    let allowance = radar_serve::share::Allowance::from_vars(&env)?;
    // The same state directory the model ledger uses, and mandatory for the same
    // reason: a meter that cannot record what it spent cannot enforce a ceiling
    // across a restart, and deploys are routine.
    let shares_store = radar_serve::ledger::Store::open(&env)
        .map_err(|why| format!("the chat share meter needs a state directory: {why}"))?;
    // The salt customer identifiers are hashed with before anything long lived
    // records them. Empty when unset, which `Subject::derive` refuses -- so an
    // unsalted instance cannot meter a customer and therefore will not spend on
    // one. Rule 8, and it is why this is not fatal: the operator surface does
    // not need it.
    let salt = env("RADAR_CUSTOMER_SALT")
        .map(String::into_bytes)
        .unwrap_or_default();
    Ok((
        admission,
        radar_serve::share::Shares::restored(
            allowance,
            shares_store,
            radar_serve::chat::today_utc(),
        ),
        salt,
    ))
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

    // Same reasoning, one step weaker. An absent Privy app id is not a
    // contradiction the way an absent Access configuration is -- it means there
    // is no customer lane, and customer routes then require operator identity.
    // A *malformed* one still stops the server, because the failure it produces
    // is indistinguishable from a vendor outage.
    let customer = match customer::Mode::from_vars(&|k| std::env::var(k).ok()) {
        Ok(mode) => mode,
        Err(why) => {
            eprintln!("radar-serve: {why}");
            return ExitCode::FAILURE;
        }
    };

    // Optional, and its absence is reported rather than fatal. An instance
    // without a Privy credential cannot look wallets up, which is not the same
    // as its customers having no wallets -- and it must not stop the operator
    // surface, which is what this process is mostly for today.
    let privy = radar_serve::privy::Credentials::from_vars(&|k| std::env::var(k).ok()).ok();
    let privy_note = privy.as_ref().map_or_else(
        || "off (no RADAR_PRIVY_APP_SECRET; wallets cannot be read)".to_owned(),
        |c| format!("on for application {}", c.app_id()),
    );
    let privy = privy.map(radar_serve::privy::Client::new);

    let x402 = x402::Config::from_env();
    let (agent, agent_note) = configure_agent();

    let (admission, shares, customer_salt) = match customer_lane() {
        Ok(lane) => lane,
        Err(why) => {
            eprintln!("radar-serve: {why}");
            return ExitCode::FAILURE;
        }
    };
    let admission_note = admission.describe();
    let share_note = shares.describe();

    let state = Arc::new(AppState {
        admission,
        shares,
        customer_salt,
        registry: registry(),
        store: Reader::open(&store_dir),
        x402,
        chat: agent,
        access: access.clone(),
        keys: access::KeyCache::new(),
        customer: customer.clone(),
        customer_keys: customer::KeyCache::new(),
        privy,
        linker: radar_serve::link::Linker::new(),
        // The domain a sign-in is bound to. Unset means no customer sign-in,
        // rather than a guess: a wrong domain here would have wallets sign a
        // message naming a site this is not, and the signature would then be
        // valid somewhere Radar does not control.
        challenges: std::env::var("RADAR_CUSTOMER_DOMAIN")
            .ok()
            .filter(|d| !d.trim().is_empty())
            .map(radar_serve::challenges::Challenges::new),
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
    println!("  admission  : {admission_note}");
    println!("  chat share : {share_note}");
    println!(
        "  customers  : {}",
        match &customer {
            customer::Mode::Enforce(config) =>
                format!("verifying Privy tokens for app {}", config.app_id),
            // Not a warning. No customer lane means customer routes require
            // operator identity, which is stricter than they will be -- but it
            // is said every start so nobody has to guess which state this is.
            customer::Mode::Off => "off — customer routes require operator identity".to_owned(),
        }
    );
    // Separate from the line above, because the two can disagree and the
    // disagreement is the interesting state: an instance that verifies customer
    // tokens but cannot read their wallets will sign people in and then fail
    // every wallet lookup, and an operator should see that at start rather than
    // from a support message.
    println!("  wallets    : {privy_note}");
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
