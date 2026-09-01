// SPDX-License-Identifier: Apache-2.0
//! `radar route` — build a swap transaction and describe it, without signing.
//!
//! # Why this exists
//!
//! Everything from the decision back is exercised hourly. Everything from the
//! transaction forward has **never run**: Jupiter's `/swap` endpoint, the
//! address-lookup-table avoidance, the shape check the signer's decoder depends
//! on. `radar-exec` had no production caller at all until 2026-09-01, and its
//! pipeline traits were satisfied only by test stubs.
//!
//! That is the shape [LEARNINGS](../../../LEARNINGS.md) 10 records. A live run
//! over 41,254 candidates raised zero proposals because a hardcoded exit-probe
//! size made a proposal arithmetically impossible — every stage had passed its
//! tests against fixtures no real candidate resembled. The lesson was not "test
//! more", it was "run the thing against reality before trusting it".
//!
//! So this command runs exactly the untested part and stops.
//!
//! # What it cannot do
//!
//! **It cannot sign and it cannot send.** There is no signer here, no key, no
//! RPC endpoint, and no `--submit` flag to discover later. A diagnostic that
//! could be talked into moving money is not a diagnostic.
//!
//! It also issues no `Authorization` and consults no policy, because it produces
//! nothing anybody could act on. The transaction it prints is unsigned bytes.

use radar_exec::pipeline::Routing;
use radar_exec::route::{QUOTE_API, Router, SWAP_API, verify_shape};
use radar_types::Address;

// The endpoints come from `radar-exec`, not from constants of this command's
// own. That is not tidiness -- the first version of this file hardcoded
// `quote-api.jup.ag/v6`, which no longer resolves, and the command failed with a
// DNS error while the executor's own constants had been pointing at the current
// host all along.
//
// A diagnostic whose endpoint can drift from the thing it is diagnosing reports
// on a system nobody runs.

/// Runs the command.
///
/// # Errors
///
/// A message for the operator. Every failure here is informative rather than
/// fatal to anything: nothing has been signed or sent.
pub fn run(args: &[String]) -> Result<(), String> {
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };

    let mint: Address = flag("--mint")
        .ok_or("--mint is required")?
        .parse()
        .map_err(|_| "--mint is not a base58 address".to_owned())?;
    let wallet: Address = flag("--wallet")
        .ok_or("--wallet is required")?
        .parse()
        .map_err(|_| "--wallet is not a base58 address".to_owned())?;
    let lamports: u64 = flag("--lamports")
        .ok_or("--lamports is required")?
        .parse()
        .map_err(|_| "--lamports is not a number".to_owned())?;

    let router = Router::new(
        flag("--quote-endpoint").unwrap_or_else(|| QUOTE_API.to_owned()),
        flag("--swap-endpoint").unwrap_or_else(|| SWAP_API.to_owned()),
    );

    println!("routing {lamports} lamports into {mint}");
    println!("  fee payer  : {wallet}");

    // Through the trait rather than the inherent method, deliberately: this is
    // the path the executor takes, and a diagnostic that exercised a different
    // one would be reporting on code nothing else runs.
    let route = Routing::build_buy(&router, &mint, &wallet, lamports)
        .map_err(|e| format!("no route: {e}"))?;

    println!("  venues     : {}", route.venues.join(" -> "));
    println!("  expected   : {} base units out", route.expected_out);
    println!("  impact     : {} bps", route.impact_bps);
    println!("  transaction: {} bytes base64", route.transaction.len());

    // The check the signer's decoder depends on. A versioned transaction with
    // address lookup tables names accounts the signer cannot see in the bytes it
    // signs, so it refuses them (ADR 0003) -- and finding that out here, with
    // nothing at stake, is the entire point of this command.
    match verify_shape(&route.transaction) {
        Ok(()) => println!("  shape      : legacy, and the signer can read every account"),
        Err(why) => {
            println!("  shape      : REFUSED by the signer's rules — {why}");
            return Err(
                "the router returned a transaction the signer would refuse. That is a \
                 routing configuration problem, not a market condition: see ADR 0003."
                    .to_owned(),
            );
        }
    }

    println!();
    println!("Nothing was signed and nothing was sent. This command has no key,");
    println!("no RPC endpoint, and no flag that would give it either.");
    Ok(())
}
