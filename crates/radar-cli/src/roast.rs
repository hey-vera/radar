// SPDX-License-Identifier: Apache-2.0
//! `radar roast <mint>` — the reply the public analyst would post.
//!
//! The caller `radar-roast` was built for, and the reason it could be built
//! before anything touching X exists. Everything hard about the reply pipeline
//! — the fact sheet, the verdict, the voice pass, the two checks — is proved
//! here, offline, against real mints, with no account and no credential.
//!
//! **It prints. It does not post.** Nothing in this path can publish, and the
//! X adapter is a separate binary precisely so that "write a reply" and "send a
//! reply" are different programs.
//!
//! # Reading a hundred of these is the point
//!
//! Before a public account says anything, somebody should read a large number
//! of its replies and disagree with some. `--sheet` prints the fact sheet
//! beside the reply so a disagreement can be traced to a measurement rather
//! than argued about.

use std::time::Duration;

use radar_onchain::{Budget, RpcClient};
use radar_roast::{BaseRates, Fellback};
use radar_types::Address;

use crate::dossier::safe;
use crate::flag;

/// Runs the command.
///
/// # Errors
///
/// A message when the mint is missing or unparseable, or when the token's
/// history cannot be read at all.
pub fn run(args: &[String]) -> Result<(), String> {
    let mint_arg = args
        .get(1)
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .or_else(|| flag(args, "--mint"))
        .ok_or_else(|| {
            "usage: radar roast <mint> [--rpc URL] [--rates PATH] [--sheet] [--seconds N]"
                .to_owned()
        })?;

    let mint: Address = mint_arg
        .parse()
        .map_err(|_| format!("not a valid address: {}", safe(&mint_arg, 64)))?;

    let client = flag(args, "--rpc").map_or_else(RpcClient::from_env, RpcClient::new);
    let seconds = flag(args, "--seconds")
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);
    let mut budget = Budget::new(
        radar_onchain::budget::DEFAULT_MAX_CALLS,
        radar_onchain::budget::DEFAULT_MAX_PAGES,
        Duration::from_secs(seconds),
    );

    let dossier = radar_onchain::build(&client, &mut budget, &mint).map_err(|e| e.to_string())?;

    // Rule 8 twice over. A snapshot that will not load means the reply carries
    // no population context -- it never means falling back on remembered
    // numbers, which is how 0008's superseded 68% would outlive its own
    // correction. The failure is printed rather than swallowed, because an
    // analyst quietly saying less is indistinguishable from one with nothing
    // to say.
    let rates_path =
        flag(args, "--rates").unwrap_or_else(|| radar_roast::baserates::DEFAULT_PATH.to_owned());
    let rates = match BaseRates::load(&rates_path) {
        Ok(r) => Some(r),
        Err(e) => {
            eprintln!("no base rates ({e}); the reply will carry no population context");
            None
        }
    };
    if let Some(r) = &rates
        && r.is_stale_at(&today())
    {
        eprintln!(
            "base rates were measured on {} and are stale; re-run research 0024 before \
             trusting the population figures",
            r.measured_on
        );
    }

    // No provider is the ordinary case on a machine with no credential, and it
    // is not an error: the deterministic template ships. `RADAR_MODEL_CODEX` is
    // marked private-use-only, so a public analyst must go through the metered
    // API-key path -- but that choice belongs to `radar-model`, which reads the
    // environment, not to this file.
    let provider = radar_model::from_vars(&|k| std::env::var(k).ok()).ok();

    let (sheet, reply) = radar_roast::roast(&dossier, rates.as_ref(), provider.as_deref());

    if args.iter().any(|a| a == "--sheet") {
        println!("--- fact sheet ---");
        print!("{}", sheet.render());
        for (label, value) in &sheet.untrusted {
            // Escaped on the way to a terminal for the same reason the dossier
            // escapes: these are arbitrary creator-controlled bytes.
            println!("{label} (untrusted): {}", safe(value, 64));
        }
        println!("--- reply ---");
    }

    print!("{}", reply.text);
    if !reply.text.ends_with('\n') {
        println!();
    }

    // The fallback reason is the single most useful line this command emits.
    // A reply that fell back because the model fabricated a figure is the only
    // early warning that the voice pass is drifting, and a silent fallback
    // would hide exactly that.
    match &reply.fellback {
        None => eprintln!("(model reply, both checks passed)"),
        Some(Fellback::NoProvider) => {
            eprintln!("(deterministic template: no model provider configured)");
        }
        Some(Fellback::Unreachable(why)) => {
            eprintln!("(deterministic template: provider unreachable -- {why})");
        }
        Some(Fellback::Empty) => eprintln!("(deterministic template: provider said nothing)"),
        Some(Fellback::Forbidden(v)) => {
            eprintln!("(deterministic template: the model wrote a claim it may not publish)");
            for violation in v {
                eprintln!("    {:?} -- {}", violation.phrase, violation.because);
            }
        }
        Some(Fellback::Fabricated(f)) => {
            eprintln!("(deterministic template: the model wrote a number nothing measured)");
            for fab in f {
                eprintln!("    {} is not on the fact sheet", fab.literal);
            }
        }
    }
    Ok(())
}

/// Today, as `YYYY-MM-DD`.
///
/// Derived from the system clock, which is fine for "are these base rates a
/// fortnight old" and is deliberately nowhere near the decision path -- the
/// risk kernel is pure and has no clock, and nothing here feeds it.
fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let days = secs / 86_400;
    // Civil-from-days, Howard Hinnant's algorithm, for the 1970 epoch.
    let z = i64::try_from(days).unwrap_or(0) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn today_is_a_date_the_staleness_check_can_read() {
        // The staleness rule treats an unparseable date as stale, so a broken
        // clock here would make every snapshot look old rather than making an
        // old one look fresh. Worth asserting the shape anyway: the silent
        // failure would be a warning that fires on every run and gets ignored.
        let t = today();
        assert_eq!(t.len(), 10, "{t}");
        assert_eq!(t.as_bytes()[4], b'-');
        assert_eq!(t.as_bytes()[7], b'-');
        let year: i32 = t[..4].parse().expect("a year");
        assert!((2024..2100).contains(&year), "{t}");
        let month: u32 = t[5..7].parse().expect("a month");
        assert!((1..=12).contains(&month), "{t}");
        let day: u32 = t[8..].parse().expect("a day");
        assert!((1..=31).contains(&day), "{t}");
    }

    #[test]
    fn the_epoch_renders_correctly() {
        // One known value, because an off-by-one in a civil-date conversion is
        // invisible in the shape check above.
        assert_eq!(from_days(0), "1970-01-01");
        assert_eq!(from_days(19_000), "2022-01-08");
    }

    /// The date arithmetic, separated so it can be checked at a fixed day.
    fn from_days(days: i64) -> String {
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = if m <= 2 { y + 1 } else { y };
        format!("{y:04}-{m:02}-{d:02}")
    }
}
