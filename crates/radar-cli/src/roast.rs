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
    let mint_arg = mint_arg_from(args).ok_or_else(|| {
        "usage: radar roast <mint> [--rpc URL] [--rates PATH] [--sheet] [--seconds N]".to_owned()
    })?;

    let mint: Address = mint_arg
        .parse()
        .map_err(|_| format!("not a valid address: {}", safe(&mint_arg, 64)))?;

    let client = flag(args, "--rpc").map_or_else(
        || RpcClient::from_vars(&|k| std::env::var(k).ok()),
        RpcClient::new,
    );
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

    // The creator's record, and its absence is worth saying out loud: without
    // it every reply about a fresh launch says the same thing, because the cost
    // line is a constant and most launches sit in the same recipient band.
    let creators = radar_roast::CreatorIndex::read(radar_roast::creator::DEFAULT_PATH).ok();
    if creators.is_none() {
        eprintln!(
            "no creator index at {}; the reply will say nothing about who launched this.              Build one with `radar creator-index --store <dir>`.",
            radar_roast::creator::DEFAULT_PATH
        );
    }

    // ADR 0013 constraint 5: the analyst's own token never has its price or
    // market cap stated. Read the way the daemon reads it and refused the same
    // way -- a mint that will not parse is the rule silently switched off for
    // the real token, so the command stops rather than printing a reply that
    // looks right.
    let self_mint = radar_analyst::daemon::self_mint_from(&|k| std::env::var(k).ok())?;

    let (sheet, reply) = radar_roast::roast(
        &dossier,
        rates.as_ref(),
        creators.as_ref(),
        provider.as_deref(),
        self_mint.as_ref(),
    );

    if wants_sheet(args) {
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
    if needs_newline(&reply.text) {
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

/// The mint a `radar roast` invocation names, if any.
///
/// Positional first, then `--mint`. The positional slot is guarded against
/// swallowing a flag: without that guard `radar roast --rpc URL` takes `--rpc`
/// as the mint and reports it as an invalid address, which is a confusing way
/// of saying "you named no mint at all".
fn mint_arg_from(args: &[String]) -> Option<String> {
    args.get(1)
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .or_else(|| flag(args, "--mint"))
}

/// Whether the fact sheet was asked for.
///
/// Its own function so the comparison can be tested. Inline, `==` could become
/// `!=` and every run would print the sheet except the ones that asked for it.
fn wants_sheet(args: &[String]) -> bool {
    args.iter().any(|a| a == "--sheet")
}

/// Whether a reply needs a newline before the shell prompt returns.
///
/// One line, and extracted anyway, for the reason above: a mutation of the `!`
/// is invisible in a `print!` and visible here.
fn needs_newline(text: &str) -> bool {
    !text.ends_with('\n')
}

/// Today, as `YYYY-MM-DD`.
///
/// Derived from the system clock, which is fine for "are these base rates a
/// fortnight old" and is deliberately nowhere near the decision path -- the
/// risk kernel is pure and has no clock, and nothing here feeds it.
///
/// The clock is the only thing this does. All the arithmetic is in
/// [`from_days`], which is pure and therefore checkable at a fixed day.
fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    from_days(i64::try_from(secs / 86_400).unwrap_or(0))
}

/// A `YYYY-MM-DD` date from a count of days since the 1970 epoch.
///
/// Civil-from-days, Howard Hinnant's algorithm.
///
/// **This was inside `today()` and copied into the test module.** The tests
/// then exercised the copy, so every mutation of the real arithmetic survived --
/// twenty-four of them, reported by CI on 2026-09-03. It is LEARNINGS 18's shape
/// applied to a test: two instruments compared as if they were one, except here
/// only one of them was ever run. The function is now production code with one
/// definition, and the tests call it.
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
    fn the_mint_argument_is_read_positionally_and_never_swallows_a_flag() {
        let args = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();

        // Positional.
        assert_eq!(
            mint_arg_from(&args(&["roast", "SoMeMiNt"])),
            Some("SoMeMiNt".to_owned())
        );
        // Named.
        assert_eq!(
            mint_arg_from(&args(&["roast", "--mint", "SoMeMiNt"])),
            Some("SoMeMiNt".to_owned())
        );
        // A flag in the positional slot is not a mint. Without the guard this
        // returns Some("--rpc"), and the command then reports the flag as an
        // invalid address rather than saying no mint was given.
        assert_eq!(mint_arg_from(&args(&["roast", "--rpc", "http://x"])), None);
        // ...and the named form still wins from behind a flag.
        assert_eq!(
            mint_arg_from(&args(&["roast", "--sheet", "--mint", "SoMeMiNt"])),
            Some("SoMeMiNt".to_owned())
        );
        // Nothing at all.
        assert_eq!(mint_arg_from(&args(&["roast"])), None);
    }

    #[test]
    fn the_sheet_is_printed_only_when_it_is_asked_for() {
        let args = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert!(wants_sheet(&args(&["roast", "M", "--sheet"])));
        assert!(!wants_sheet(&args(&["roast", "M"])));
        // A different flag is not this flag.
        assert!(!wants_sheet(&args(&["roast", "M", "--sheets"])));
    }

    #[test]
    fn a_reply_gets_a_newline_only_when_it_lacks_one() {
        assert!(needs_newline("no trailing newline"));
        assert!(!needs_newline("has one\n"));
        // Empty output still wants the prompt on its own line.
        assert!(needs_newline(""));
    }

    #[test]
    fn run_refuses_before_it_reaches_the_network() {
        // Both refusals happen during argument handling, so this needs no RPC
        // and no fixture -- and it is what stops the whole body being
        // replaceable with `Ok(())`. A `radar roast` that printed nothing and
        // exited zero would look exactly like success, which is LEARNINGS 5.
        let args = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();

        let usage = run(&args(&["roast"])).expect_err("no mint is a usage error");
        assert!(usage.starts_with("usage: radar roast"), "{usage}");

        let bad = run(&args(&["roast", "not-an-address"])).expect_err("a bad mint is an error");
        assert!(bad.starts_with("not a valid address:"), "{bad}");
    }

    #[test]
    fn the_calendar_is_right_at_every_boundary_the_algorithm_has() {
        // Two known values used to be the whole of this, against a copy of the
        // algorithm that lived in this module -- so every arithmetic mutation of
        // the real one survived. These are chosen so that each constant and each
        // branch changes an answer if it moves.

        // The epoch, and the era offset that centres the algorithm on March.
        assert_eq!(from_days(0), "1970-01-01");
        assert_eq!(from_days(1), "1970-01-02");
        assert_eq!(from_days(31), "1970-02-01");

        // The March pivot itself: `mp < 10` and the `y + 1` correction either
        // side of it. 1970-02-28 and 1970-03-01 are consecutive days that take
        // opposite branches.
        assert_eq!(from_days(58), "1970-02-28");
        assert_eq!(from_days(59), "1970-03-01");

        // Leap day in an ordinary leap year, and the day either side of it.
        assert_eq!(from_days(19_781), "2024-02-28");
        assert_eq!(from_days(19_782), "2024-02-29");
        assert_eq!(from_days(19_783), "2024-03-01");

        // 2000 is a leap year because it is divisible by 400 -- the case the
        // `doe / 36_524` and `doe / 146_096` terms exist for, and the one a
        // naive rule gets wrong.
        assert_eq!(from_days(11_015), "2000-02-28");
        assert_eq!(from_days(11_016), "2000-02-29");
        assert_eq!(from_days(11_017), "2000-03-01");

        // 2100 is *not* a leap year, because it is divisible by 100 and not by
        // 400. Without this the century rule is unchecked in the direction that
        // matters.
        assert_eq!(from_days(47_540), "2100-02-28");
        assert_eq!(from_days(47_541), "2100-03-01");

        // Year and month ends, where `doy`, `mp` and the `+ 1` on the day meet.
        assert_eq!(from_days(19_721), "2023-12-30");
        assert_eq!(from_days(19_722), "2023-12-31");
        assert_eq!(from_days(19_723), "2024-01-01");

        // A date from before the epoch, so `div_euclid` and `rem_euclid` are
        // exercised on a negative `z` -- the reason they are there rather than
        // `/` and `%`.
        assert_eq!(from_days(-1), "1969-12-31");
        assert_eq!(from_days(-365), "1969-01-01");

        // The day this was written, cross-checked against a calendar.
        assert_eq!(from_days(20_699), "2026-09-03");
    }

    #[test]
    fn today_uses_the_same_arithmetic_the_tests_check() {
        // The defect this file had was a second copy of the algorithm. This is
        // the assertion that there is only one: whatever day the clock is on,
        // `today()` must equal `from_days` of that day.
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let days = i64::try_from(secs / 86_400).expect("days since the epoch");
        assert_eq!(today(), from_days(days));
    }
}
