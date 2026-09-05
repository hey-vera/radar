// SPDX-License-Identifier: Apache-2.0
//! The payout process.
//!
//! Runs as its own systemd unit under its own user, with the key file readable
//! by nobody else, on a timer. Everything that decides anything is in the
//! library; this file reads three variables and one flag and prints what
//! happened.
//!
//! # Configuration
//!
//! - `RADAR_PAYOUT_KEY` -- path to the creator wallet's keypair file. Absent
//!   means nothing is paid, and the process says so.
//! - `RADAR_RPC_URL` -- the direct RPC endpoint (rule 7). Absent means nothing
//!   is paid: a default endpoint would be a spending path nobody chose.
//! - `RADAR_CONTEST_DIR` -- where the week records live; `data/contest`.
//!
//! `--week N` names the week. `--dry-run` plans, prints the unsigned
//! transaction, and signs nothing. `--due` pays every claimed, unpaid week in
//! the directory, which is what the timer runs.

use std::process::ExitCode;

use radar_contest::Week;
use radar_payout::{Outcome, PayError, Rpc, load_key, pay};

fn env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dry_run = args.iter().any(|a| a == "--dry-run");
    let contest_dir = env("RADAR_CONTEST_DIR").unwrap_or_else(|| "data/contest".to_owned());

    let Some(key_path) = env("RADAR_PAYOUT_KEY") else {
        eprintln!("radar-payout: RADAR_PAYOUT_KEY is not set, so nothing is paid.");
        return ExitCode::FAILURE;
    };
    let Some(rpc_url) = env("RADAR_RPC_URL") else {
        eprintln!("radar-payout: RADAR_RPC_URL is not set, so nothing is paid.");
        return ExitCode::FAILURE;
    };
    let key = match load_key(std::path::Path::new(&key_path)) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("radar-payout: {e}");
            return ExitCode::FAILURE;
        }
    };
    let chain = Rpc::new(rpc_url);

    let weeks: Vec<Week> = if args.iter().any(|a| a == "--due") {
        radar_contest::records_in(std::path::Path::new(&contest_dir))
            .into_iter()
            .filter(|r| r.claim.is_some() && r.payout.is_none())
            .map(|r| r.week)
            .collect()
    } else {
        let Some(n) = flag(&args, "--week").and_then(|w| w.parse::<u64>().ok()) else {
            eprintln!("radar-payout: --week <n> or --due is required");
            return ExitCode::FAILURE;
        };
        vec![Week(n)]
    };
    if weeks.is_empty() {
        println!("radar-payout: nothing is claimed and unpaid.");
        return ExitCode::SUCCESS;
    }

    let mut failed = false;
    for week in weeks {
        match pay(&chain, &contest_dir, week, &key, now(), dry_run) {
            Ok(Outcome::Planned(plan)) => {
                println!(
                    "week {}: would pay {} lamports from {} to {}; unsigned transaction, base64:\n{}",
                    week.0,
                    plan.lamports,
                    plan.creator,
                    plan.recipient,
                    plan.unsigned_base64()
                );
            }
            Ok(Outcome::Paid(payout)) => {
                println!(
                    "week {}: paid {} lamports to {}; signature {}",
                    week.0, payout.lamports, payout.recipient, payout.signature
                );
            }
            Err(PayError::Refused(why)) => {
                // A refusal is the answer, not an error: the timer will run
                // again and nothing was wrong with the process.
                println!("week {}: refused: {why:?}", week.0);
            }
            Err(PayError::NothingCollected { vault, reserve }) => {
                println!(
                    "week {}: the vault holds {vault} lamports against a reserve of {reserve}; nothing to pay",
                    week.0
                );
            }
            Err(e) => {
                eprintln!("radar-payout: week {}: {e}", week.0);
                failed = true;
            }
        }
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
