// SPDX-License-Identifier: Apache-2.0
//! The Radar server.

use std::net::SocketAddr;
use std::process::ExitCode;
use std::sync::Arc;

use radar_instruments::{CreatorHistory, CreatorTrackRecord, Registry};
use radar_serve::{AppState, app, x402};
use radar_store::Reader;

/// Every instrument Radar exposes. The CLI builds the same list.
fn registry() -> Registry {
    let mut r = Registry::new();
    r.register(CreatorHistory);
    r.register(CreatorTrackRecord);
    r
}

#[tokio::main]
async fn main() -> ExitCode {
    let store_dir = std::env::var("RADAR_STORE").unwrap_or_else(|_| "./data/store".to_owned());
    let bind: SocketAddr = std::env::var("RADAR_BIND")
        .unwrap_or_else(|_| "127.0.0.1:8080".to_owned())
        .parse()
        .unwrap_or_else(|_| SocketAddr::from(([127, 0, 0, 1], 8080)));

    let x402 = x402::Config::from_env();
    let state = Arc::new(AppState {
        registry: registry(),
        store: Reader::open(&store_dir),
        x402,
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
