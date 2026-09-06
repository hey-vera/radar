// SPDX-License-Identifier: Apache-2.0
//! The payout process.
//!
//! Runs as its own systemd unit under its own user, with the key file readable
//! by nobody else, on a timer. Everything that decides anything is in the
//! library or in the small functions below, which a test can call; `main`
//! reads three variables and one flag and prints what happened.
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

use radar_contest::{Record, Week};
use radar_payout::{Outcome, PayError, Rpc, load_key, pay};

/// A variable's value, with blank counting as unset.
fn present(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.trim().is_empty())
}

fn env(key: &str) -> Option<String> {
    present(std::env::var(key).ok())
}

/// The value following `name`, if it is there.
fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// Whether the run plans without signing.
fn dry_run(args: &[String]) -> bool {
    args.iter().any(|a| a == "--dry-run")
}

/// Which weeks a run pays: every claimed, unpaid record under `--due`, or the
/// one named by `--week`.
///
/// # Errors
///
/// A message when neither flag is usable.
fn weeks_to_pay(args: &[String], records: &[Record]) -> Result<Vec<Week>, String> {
    if args.iter().any(|a| a == "--due") {
        return Ok(records
            .iter()
            .filter(|r| r.claim.is_some() && r.payout.is_none())
            .map(|r| r.week)
            .collect());
    }
    flag(args, "--week")
        .and_then(|w| w.parse::<u64>().ok())
        .map(|n| vec![Week(n)])
        .ok_or_else(|| "--week <n> or --due is required".to_owned())
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
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

    let records = radar_contest::records_in(std::path::Path::new(&contest_dir));
    let weeks = match weeks_to_pay(&args, &records) {
        Ok(weeks) => weeks,
        Err(why) => {
            eprintln!("radar-payout: {why}");
            return ExitCode::FAILURE;
        }
    };
    if weeks.is_empty() {
        println!("radar-payout: nothing is claimed and unpaid.");
        return ExitCode::SUCCESS;
    }

    let mut failed = false;
    for week in weeks {
        match pay(&chain, &contest_dir, week, &key, now(), dry_run(&args)) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use radar_contest::{Claim, Payout, Ranking};

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    fn record(week: u64, claimed: bool, paid: bool) -> Record {
        let mut r = Record::close(
            Week(week),
            Ranking::default(),
            &radar_contest::Rules::published(["op"]),
        );
        if claimed {
            r.claim = Some(Claim {
                address: "A".to_owned(),
                reply_id: "c".to_owned(),
                at: 1,
            });
        }
        if paid {
            r.payout = Some(Payout {
                recipient: "A".to_owned(),
                lamports: 1,
                signature: "S".to_owned(),
                at: 2,
            });
        }
        r
    }

    #[test]
    fn a_blank_variable_is_unset_and_a_flag_takes_the_value_after_it() {
        // CI's mutants dropped the `!` in the blank filter and moved `flag`'s
        // arithmetic; each is one assertion here.
        assert_eq!(present(Some("  ".to_owned())), None);
        assert_eq!(present(Some("x".to_owned())), Some("x".to_owned()));
        assert_eq!(present(None), None);
        let a = args(&["--week", "7", "--dry-run"]);
        assert_eq!(flag(&a, "--week"), Some("7".to_owned()));
        assert_eq!(
            flag(&a, "--dry-run"),
            None,
            "a flag at the end has no value"
        );
        assert_eq!(flag(&a, "--nope"), None);
        assert!(dry_run(&a));
        assert!(!dry_run(&args(&["--week", "7"])));
    }

    #[test]
    fn due_pays_the_claimed_and_unpaid_and_week_names_one() {
        // Re-applied `&&` as `||` in the filter: the paid week and the
        // unclaimed week both come back and the first assertion fails.
        let records = [
            record(1, false, false),
            record(2, true, false),
            record(3, true, true),
            record(4, true, false),
        ];
        assert_eq!(
            weeks_to_pay(&args(&["--due"]), &records),
            Ok(vec![Week(2), Week(4)])
        );
        assert_eq!(
            weeks_to_pay(&args(&["--week", "9"]), &records),
            Ok(vec![Week(9)])
        );
        assert!(weeks_to_pay(&args(&["--week", "nine"]), &records).is_err());
        assert!(weeks_to_pay(&args(&[]), &records).is_err());
    }
}
