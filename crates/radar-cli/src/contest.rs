// SPDX-License-Identifier: Apache-2.0
//! `radar contest`: the payout's manual fallback, through the same check.
//!
//! Design 0007 C5. Two subcommands, and neither holds a key:
//!
//! - `pay --week N --creator <address> --rpc <url> [--contest-dir <dir>]`
//!   plans the week's payment exactly as `radar-payout` would and prints the
//!   unsigned transaction, base64, for the operator to sign and send with
//!   tooling of their choice. Nothing is signed here.
//! - `record-payout --week N --creator <address> --rpc <url> --signature <sig>`
//!   reads that transaction back off the chain through the same `verify` step
//!   the automated path uses -- one transfer, from the creator, to the claimed
//!   address -- and only then writes the payout into the week's record.
//!
//! The fallback is therefore exercised by the automated path's own tests,
//! which is the condition design 0007 set for having one at all.

use radar_contest::Week;
use radar_payout::{Chain, Rpc, plan, record_payout};
use radar_pumpfun::pda;
use radar_types::Address;

/// Runs the command.
///
/// # Errors
///
/// A message when a flag is missing, the chain cannot be read, the record is
/// missing, or the policy refuses.
pub fn run(args: &[String]) -> Result<(), String> {
    let sub = args.get(1).map(String::as_str);
    let week = crate::flag(args, "--week")
        .and_then(|w| w.parse::<u64>().ok())
        .map(Week)
        .ok_or("--week <n> is required")?;
    let creator: Address = crate::flag(args, "--creator")
        .ok_or("--creator <address> is required: the wallet that launched the token")?
        .parse()
        .map_err(|e| format!("--creator: {e}"))?;
    let rpc = crate::flag(args, "--rpc").ok_or("--rpc <url> is required (direct RPC, rule 7)")?;
    let contest_dir =
        crate::flag(args, "--contest-dir").unwrap_or_else(|| "data/contest".to_owned());
    let chain = Rpc::new(rpc);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());

    match sub {
        Some("pay") => {
            if !args.iter().any(|a| a == "--dry-run") {
                return Err("`radar contest pay` only plans; pass --dry-run to say so, and sign elsewhere".to_owned());
            }
            let record = radar_payout::read_record(&contest_dir, week).map_err(|e| e.to_string())?;
            let vault = pda::creator_vault(&creator).ok_or("the creator vault could not be derived")?;
            let lamports = chain.balance(&vault)?;
            let blockhash = chain.latest_blockhash()?;
            let planned = plan(&record, &creator, lamports, &blockhash).map_err(|e| e.to_string())?;
            println!(
                "week {}: pay {} lamports from {} to {}\nvault {} holds {} lamports\nunsigned transaction, base64 (sign with the creator key and send; then `radar contest record-payout --signature <sig>`):\n{}",
                week.0,
                planned.lamports,
                planned.creator,
                planned.recipient,
                vault,
                lamports,
                planned.unsigned_base64()
            );
            Ok(())
        }
        Some("record-payout") => {
            let signature = crate::flag(args, "--signature").ok_or("--signature <sig> is required")?;
            let payout = record_payout(&chain, &contest_dir, week, &creator, &signature, now)
                .map_err(|e| e.to_string())?;
            println!(
                "week {}: recorded {} lamports to {} under {}",
                week.0, payout.lamports, payout.recipient, payout.signature
            );
            Ok(())
        }
        _ => Err(
            "radar contest <pay --dry-run | record-payout --signature <sig>> --week <n> --creator <address> --rpc <url>"
                .to_owned(),
        ),
    }
}
