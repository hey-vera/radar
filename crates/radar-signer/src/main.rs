// SPDX-License-Identifier: Apache-2.0
//! The signer process.
//!
//! Runs as its own systemd unit under its own user, with the key file readable
//! by nobody else. It has no network, no listener and no RPC: requests arrive on
//! stdin as newline-delimited JSON and answers leave on stdout.
//!
//! Everything that decides anything lives in the library, so this file is short
//! enough to read in full before trusting it. That is the point of it being
//! short.
//!
//! # Configuration
//!
//! - `RADAR_SIGNER_KEY` — path to a Solana keypair JSON file. Absent means every
//!   request is refused, which is the correct behaviour for a signer that does
//!   not know what it is signing for.
//! - `RADAR_SIGNER_PROGRAMS` — comma-separated base58 program ids that may
//!   appear in a signed transaction. Absent means every request is refused; an
//!   empty allowlist is a signer that will sign anything, and that is the one
//!   configuration mistake with no upper bound on its cost.

use std::io::{BufRead as _, Write as _};
use std::path::PathBuf;

use radar_signer::protocol::{Request, Response, place_signature, slot_of};
use radar_signer::{Allowlist, Key, b64, check};
use radar_types::Address;

fn main() -> std::process::ExitCode {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    // Loaded once, at start. A signer that re-reads its allowlist per request
    // is a signer whose rules can be changed by whoever can write that file
    // while it runs.
    let config = match Config::from_env() {
        Ok(c) => Some(c),
        Err(why) => {
            // Still serve, still refuse. Exiting would make a misconfiguration
            // look like a crash, and the executor would retry it forever.
            eprintln!("radar-signer: refusing everything: {why}");
            None
        }
    };

    if let Some(c) = &config {
        eprintln!(
            "radar-signer: ready as {} with {} allowed programs",
            c.key.public(),
            c.allowlist.programs.len()
        );
    }

    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            break;
        };
        if line.trim().is_empty() {
            continue;
        }

        let response = config.as_ref().map_or_else(
            || Response::refused("signer is not configured"),
            |c| handle(&line, c),
        );

        // Every decision is logged before it is returned. A signature that
        // reached the chain with no line here would mean this process was not
        // the one that made it.
        eprintln!(
            "radar-signer: {}",
            match &response {
                Response::Signed { signature, .. } => format!("signed {signature}"),
                Response::Refused { reasons } => format!("refused: {}", reasons.join("; ")),
            }
        );

        let Ok(json) = serde_json::to_string(&response) else {
            continue;
        };
        if writeln!(stdout, "{json}").is_err() || stdout.flush().is_err() {
            break;
        }
    }
    std::process::ExitCode::SUCCESS
}

/// What the process was told at start.
struct Config {
    key: Key,
    allowlist: Allowlist,
}

impl Config {
    fn from_env() -> Result<Self, String> {
        let path = std::env::var("RADAR_SIGNER_KEY")
            .map_err(|_| "RADAR_SIGNER_KEY is not set".to_owned())?;
        let key = Key::load(&PathBuf::from(path)).map_err(|e| e.to_string())?;

        let listed = std::env::var("RADAR_SIGNER_PROGRAMS")
            .map_err(|_| "RADAR_SIGNER_PROGRAMS is not set".to_owned())?;
        let mut programs = Vec::new();
        for entry in listed.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let address: Address = entry
                .parse()
                .map_err(|_| format!("`{entry}` is not a base58 address"))?;
            programs.push(*address.as_bytes());
        }
        if programs.is_empty() {
            // An empty allowlist would sign anything. Of every misconfiguration
            // available here, it is the one with no upper bound on its cost.
            return Err("RADAR_SIGNER_PROGRAMS is empty".to_owned());
        }

        Ok(Self {
            key,
            allowlist: Allowlist { programs },
        })
    }
}

/// Handles one request.
fn handle(line: &str, config: &Config) -> Response {
    let Ok(request) = serde_json::from_str::<Request>(line) else {
        return Response::refused("unreadable request");
    };
    let Some(bytes) = b64::decode(&request.transaction) else {
        return Response::refused("transaction is not base64");
    };

    let checked = match check(
        &request.authorization,
        &bytes,
        &config.key.public(),
        &config.allowlist,
        slot_of(&request),
    ) {
        Ok(c) => c,
        Err(rejections) => {
            return Response::Refused {
                reasons: rejections.iter().map(ToString::to_string).collect(),
            };
        }
    };

    let signature = config.key.sign(&checked);
    let Some(signed) = place_signature(
        checked.bytes(),
        checked.message().message_offset,
        0,
        signature.as_bytes(),
    ) else {
        // The transaction verified but has no room for a signature. That is the
        // executor's bug, and signing something else to work around it is not
        // this process's decision to make.
        return Response::refused("no signature slot in the transaction");
    };

    Response::Signed {
        signature: signature.to_string(),
        wallet: config.key.public().to_string(),
        transaction: b64::encode(&signed),
    }
}
