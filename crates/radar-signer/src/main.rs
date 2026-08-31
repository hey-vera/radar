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

use radar_signer::privy::{AuthorizationKey, authorise};
use radar_signer::protocol::{Envelope, PrivyAuthorization, Response, place_signature, slot_of};
use radar_signer::{Allowlist, Key, check};
use radar_types::{Address, Slot};

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
                // The signature itself is a header value for a request that has
                // not been sent yet, and logging it would put a usable
                // authorisation in a file with looser permissions than the key
                // that made it. The nonce is what a later question needs.
                Response::Authorised { .. } => "authorised a Privy request".to_owned(),
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
    /// The Privy authorization key, when this instance serves customers.
    ///
    /// `None` is a refusal for the customer lane and leaves the local lane
    /// untouched. An instance with no customers needs no customer key, and
    /// refusing to start without one would take down the lane that works.
    privy: Option<AuthorizationKey>,
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

        // Optional, and its absence is a refusal rather than a failure to
        // start. An instance with no customers needs no customer key, and
        // refusing to run without one would take the local lane down too.
        let privy = match std::env::var("RADAR_PRIVY_AUTHORIZATION_KEY") {
            Ok(material) if !material.trim().is_empty() => {
                Some(AuthorizationKey::parse(&material).map_err(|e| e.to_string())?)
            }
            _ => None,
        };

        Ok(Self {
            key,
            allowlist: Allowlist { programs },
            privy,
        })
    }
}

/// Handles a request for a Privy authorization signature.
///
/// The wallet is the customer's, not this process's, so `config.key` is not
/// involved at all. What is involved is the same `verify::check` -- through
/// `privy::authorise`, which reads the transaction out of the request body
/// rather than being handed one.
fn handle_privy(privy: &PrivyAuthorization, config: &Config) -> Response {
    let Some(key) = config.privy.as_ref() else {
        // Rule 8. No key means no customer signing, not an unsigned request --
        // an unsigned request would simply be rejected by Privy, but reporting
        // it as anything other than "not configured" would send an operator to
        // the wrong place.
        return Response::refused("no Privy authorization key is configured");
    };
    let Ok(wallet) = privy.wallet.parse::<Address>() else {
        return Response::refused("the wallet is not a base58 address");
    };

    match authorise(
        key,
        &privy.request,
        &privy.authorization,
        &wallet,
        &config.allowlist,
        Slot(privy.now_slot),
    ) {
        Ok(signature) => Response::Authorised { signature },
        Err(why) => Response::refused(why.to_string()),
    }
}

/// Handles one request.
fn handle(line: &str, config: &Config) -> Response {
    // Unparseable is refused, and that includes an untagged request from an
    // older caller. A deployment that updates one side and not the other stops
    // signing rather than guessing which kind of signature was wanted.
    let Ok(envelope) = serde_json::from_str::<Envelope>(line) else {
        return Response::refused("unreadable request");
    };
    let request = match envelope {
        Envelope::Local(request) => request,
        Envelope::Privy(privy) => return handle_privy(&privy, config),
    };
    let Some(bytes) = radar_types::b64::decode(&request.transaction) else {
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
        transaction: radar_types::b64::encode(&signed),
    }
}
