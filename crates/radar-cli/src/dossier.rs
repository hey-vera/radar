// SPDX-License-Identifier: Apache-2.0
//! `radar dossier <mint>` — everything Radar can say about one token, from the
//! chain.
//!
//! The caller `radar-onchain` was built for. It exists in the same change as the
//! crate, so nothing here is a library waiting for a user: this is how the
//! whole on-demand path is exercised, and none of it needs the X adapter, an
//! account, or a key.
//!
//! # Rendering is where untrusted bytes become dangerous
//!
//! A token's name and symbol are arbitrary creator-controlled bytes
//! (AGENTS.md rule 4). Printed raw to a terminal they can carry escape
//! sequences, right-to-left overrides that reverse the text after them, and
//! homoglyphs. `radar-cli` already learned this once — `main.rs`'s existing
//! renderer says *"token names and symbols are arbitrary creator-controlled
//! bytes"* — so metadata goes through [`safe`] here rather than to `println!`.

use std::fmt::Write as _;
use std::time::Duration;

use radar_onchain::budget::Count;
use radar_onchain::{Budget, Dossier, RpcClient};
use radar_types::Address;

use crate::flag;

/// Renders lamports as SOL.
///
/// Integer division for the whole part and a remainder for the fraction, rather
/// than a float. `radar-types` keeps money integral on purpose, and a `u64` of
/// lamports is past f64's exact range -- a printed figure that has silently
/// rounded is exactly the kind of number this account must not publish.
fn sol(lamports: u64) -> String {
    format!(
        "{}.{:04}",
        lamports / 1_000_000_000,
        (lamports % 1_000_000_000) / 100_000
    )
}

/// Renders creator-controlled text safely.
///
/// Keeps printable ASCII and ordinary spaces; every other byte becomes an
/// escape. That is deliberately aggressive — it mangles legitimate non-Latin
/// names — and the trade is the right way round for a security boundary: a
/// mangled name is a cosmetic problem, and a terminal that has been handed a
/// direction override is a reader who has been shown something other than what
/// the chain says.
#[must_use]
pub fn safe(raw: &str, limit: usize) -> String {
    let mut out = String::with_capacity(raw.len().min(limit));
    for c in raw.chars().take(limit) {
        if c.is_ascii_graphic() || c == ' ' {
            out.push(c);
        } else {
            let _ = write!(out, "\\u{{{:x}}}", u32::from(c));
        }
    }
    if raw.chars().count() > limit {
        out.push('…');
    }
    out
}

/// The mint, positionally or behind `--mint`.
///
/// Extracted so the `!` in the filter is testable — deleting it survived
/// mutation, and the consequence is that `radar dossier --mint X` reads the
/// **flag name** `--mint` as the address instead of `X`. `radar-cli` has been
/// bitten by this exact class before: `flag`'s doc records a `+ 1` that could
/// be turned into a `- 1` with every test still passing, giving a flag the
/// previous argument's value.
#[must_use]
pub fn mint_arg_of(args: &[String]) -> Option<String> {
    args.get(1)
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .or_else(|| flag(args, "--mint"))
}

/// Runs the command.
///
/// # Errors
///
/// A message when the mint is missing or unparseable, or when the token's
/// history cannot be read at all. A dossier that is merely *partial* is a
/// success — it prints what it has and names what it could not read.
pub fn run(args: &[String]) -> Result<(), String> {
    let mint_arg = mint_arg_of(args)
        .ok_or_else(|| "usage: radar dossier <mint> [--rpc URL] [--seconds N]".to_owned())?;

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
    print!("{}", render(&dossier));
    Ok(())
}

/// The curve half of the report.
///
/// Split out because `render` was over the line limit, and because these are the
/// two figures most likely to be quoted out of context -- capacity as though it
/// were the venue's, and the venue fee as though it were the round trip.
fn render_curve(out: &mut String, curve: &radar_onchain::CurveFacts) {
    out.push_str(
        "
curve
",
    );
    let _ = writeln!(
        out,
        "  graduated   : {}",
        if curve.complete { "yes" } else { "no" }
    );
    let _ = writeln!(out, "  reserves    : {} SOL", sol(curve.real_sol_reserves));
    match curve.capacity_lamports {
        Some(l) => {
            let _ = writeln!(
                out,
                "  capacity    : {} SOL at {} bps impact",
                sol(l),
                radar_onchain::dossier::CAPACITY_IMPACT_BPS
            );
            // 0022 in one line, next to the number it corrects. STATE.md and
            // GOAL.md both called this figure a venue ceiling for weeks.
            out.push_str(
                "                (Radar's impact budget, NOT a venue ceiling -- research 0022)
",
            );
        }
        // "cannot exit", never "no limit found".
        None => out.push_str(
            "  capacity    : none -- cannot size into this
",
        ),
    }
    match &curve.fees {
        Some(fees) => {
            let _ = writeln!(
                out,
                "  venue fee   : {} bps a side, {} bps round trip (read from the chain)",
                fees.total_bps(),
                fees.round_trip_bps()
            );
            // The fee is one of three round-trip numbers in circulation, and
            // quoting it as the cost of trading understates the measured
            // figure by about 3.4x. STATE.md reconciles them.
            out.push_str(
                "                (the venue fee only -- the measured all-in round trip is 850 bps)
",
            );
        }
        None => out.push_str(
            "  venue fee   : could not be read (not assumed)
",
        ),
    }
}

/// Formats a dossier.
///
/// Split from [`run`] so the output can be asserted without a network. Every
/// number it prints is one the public analyst may later publish, so the shape
/// of this text is the shape of the claim.
#[must_use]
pub fn render(d: &Dossier) -> String {
    let mut out = String::new();

    let _ = writeln!(out, "mint          : {}", d.mint);
    // The slot is printed before any figure, because a number without the slot
    // it was read at cannot be checked against an explorer -- and being
    // checkable is the whole of this account's claim.
    match d.read_at {
        Some(slot) => {
            let _ = writeln!(out, "read at slot  : {}", slot.0);
        }
        None => out.push_str(
            "read at slot  : unknown
",
        ),
    }
    let _ = writeln!(
        out,
        "cost          : {} rpc calls, {} ms",
        d.calls, d.elapsed_ms
    );

    if let Some(launch) = &d.launch {
        out.push_str(
            "
launch block
",
        );
        let _ = writeln!(out, "  slot        : {}", launch.slot.0);
        let _ = writeln!(out, "  creator     : {}", launch.creator);
        let _ = writeln!(
            out,
            "  recipients  : {}   (token accounts, not owners -- research 0012)",
            launch.recipients
        );
        let _ = writeln!(out, "  transactions: {}", launch.transactions);
        match launch.dev_buy_lamports {
            Some(l) => {
                let _ = writeln!(out, "  dev buy     : {} SOL", sol(l));
            }
            // Not "0 SOL". Rule 9, and this one is a statement about a person.
            None => out.push_str(
                "  dev buy     : not found (absent, not zero)
",
            ),
        }
        let _ = writeln!(out, "  name        : {}", safe(&launch.metadata.name, 48));
        let _ = writeln!(out, "  symbol      : {}", safe(&launch.metadata.symbol, 16));
        out.push_str(
            "  (name, symbol and uri are creator-controlled and untrusted)
",
        );
    }

    if let Some(curve) = &d.curve {
        render_curve(&mut out, curve);
    }

    if let Some(count) = d.creator_transactions {
        out.push_str(
            "
creator
",
        );
        let _ = writeln!(
            out,
            "  transactions: {count}   (transactions, not launches)"
        );
        if matches!(count, Count::AtLeast(_)) {
            out.push_str(
                "  a launch count needs the store's creator index -- Phase 2
",
            );
        }
    }

    if !d.unavailable.is_empty() {
        out.push_str(
            "
not available
",
        );
        for miss in &d.unavailable {
            let _ = writeln!(out, "  {:<12}: {}", miss.fact, miss.why);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creator_controlled_text_cannot_reach_the_terminal_raw() {
        // The attack this is for: a right-to-left override reverses everything
        // printed after it, so a token can be named such that the *address* on
        // the next line reads backwards.
        assert_eq!(safe("\u{202e}drowssap", 64), "\\u{202e}drowssap");
        // An escape sequence that would otherwise move the cursor or set a
        // colour.
        assert_eq!(safe("\u{1b}[31mRED", 64), "\\u{1b}[31mRED");
        // A newline cannot forge a field of its own in this output.
        assert_eq!(
            safe("ok\ncreator: attacker", 64),
            "ok\\u{a}creator: attacker"
        );
        // Ordinary text survives unchanged, or the rendering is useless.
        assert_eq!(safe("Doge Killer 2", 64), "Doge Killer 2");
    }

    #[test]
    fn run_refuses_without_a_mint_rather_than_succeeding_silently() {
        // `run -> Ok(())` survived, and this kills it without a network: the
        // argument check happens before any client is built, so a command
        // invoked wrongly must fail rather than exit zero having done nothing.
        //
        // That matters beyond the mutant. `radar dossier` is meant to be
        // scriptable, and a command that returns success when it was given no
        // token is one whose exit code cannot be trusted by whatever runs it.
        let err = run(&["dossier".to_owned()]).expect_err("no mint is an error");
        assert!(err.contains("usage"), "{err}");

        // An unparseable mint is also an error, and also decided before any
        // network call -- so this stays offline too.
        let err = run(&["dossier".to_owned(), "not-an-address".to_owned()])
            .expect_err("a bad mint is an error");
        assert!(err.contains("not a valid address"), "{err}");
    }

    #[test]
    fn sol_renders_lamports_by_integer_arithmetic() {
        // Replacing `sol` with an empty string survived. It is how every SOL
        // figure in the dossier reaches a reader, and the reason it does not go
        // through a float is that a u64 of lamports is past f64's exact range:
        // a figure that has silently rounded is the one thing this account must
        // not publish.
        assert_eq!(sol(1_000_000_000), "1.0000");
        assert_eq!(sol(303_000_000), "0.3030");
        assert_eq!(sol(6_186_150_833), "6.1861");
        assert_eq!(sol(0), "0.0000");
        // Sub-precision dust rounds down rather than up: a capacity reported
        // larger than it is would be the expensive direction.
        assert_eq!(sol(1), "0.0000");
    }

    #[test]
    fn the_curve_section_renders_its_numbers_and_their_caveats() {
        // Replacing `render_curve` with `()` survived -- the whole section
        // could vanish and nothing noticed. Both caveats are asserted, not just
        // the numbers: they are what keep the figures honest, and dropping one
        // turns a Radar setting into a claim about the venue.
        let mut out = String::new();
        render_curve(
            &mut out,
            &radar_onchain::CurveFacts {
                complete: false,
                real_sol_reserves: 6_186_150_833,
                capacity_lamports: Some(303_000_000),
                fees: None,
            },
        );
        assert!(out.contains("6.1861"), "{out}");
        assert!(out.contains("0.3030"), "{out}");
        assert!(out.contains("graduated   : no"), "{out}");
        assert!(out.contains("NOT a venue ceiling"), "{out}");
        assert!(out.contains("could not be read (not assumed)"), "{out}");
    }

    #[test]
    fn a_graduated_curve_reports_no_capacity_rather_than_zero() {
        // Rule 9 at the rendering layer: "cannot size into this" and "capacity
        // is zero" read very differently to somebody deciding what to do.
        let mut out = String::new();
        render_curve(
            &mut out,
            &radar_onchain::CurveFacts {
                complete: true,
                real_sol_reserves: 0,
                capacity_lamports: None,
                fees: None,
            },
        );
        assert!(out.contains("graduated   : yes"), "{out}");
        assert!(out.contains("cannot size into this"), "{out}");
        assert!(!out.contains("0.0000 SOL at"), "{out}");
    }

    #[test]
    fn text_of_exactly_the_limit_is_not_marked_truncated() {
        // `>` survived being turned into `>=`. The consequence is an ellipsis
        // on text that was shown in full -- a reply claiming it withheld
        // something it did not, which is a small lie in a product whose whole
        // claim is that it does not tell them.
        assert_eq!(safe("abcde", 5), "abcde");
        assert_eq!(safe("abcd", 5), "abcd");
        assert_eq!(safe("abcdef", 5), "abcde…");
    }

    #[test]
    fn the_mint_argument_is_read_positionally_or_from_the_flag() {
        // Deleting the `!` survived, and the consequence is that
        // `radar dossier --mint X` reads the flag NAME as the address. This is
        // the same class `flag`'s own doc records: a `+ 1` turned into a `- 1`
        // with every test still passing.
        let args = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            mint_arg_of(&args(&["dossier", "MintOne"])),
            Some("MintOne".to_owned())
        );
        assert_eq!(
            mint_arg_of(&args(&["dossier", "--mint", "MintOne"])),
            Some("MintOne".to_owned()),
            "the flag's value, never the flag's name"
        );
        assert_eq!(
            mint_arg_of(&args(&[
                "dossier", "--rpc", "http://x", "--mint", "MintOne"
            ])),
            Some("MintOne".to_owned())
        );
        assert_eq!(mint_arg_of(&args(&["dossier"])), None);
        assert_eq!(mint_arg_of(&args(&["dossier", "--rpc", "http://x"])), None);
    }

    #[test]
    fn the_unavailable_section_appears_only_when_something_is_unavailable() {
        // Deleting the `!` survived. The consequence is every complete dossier
        // printing an empty "not available" heading -- which reads as "we could
        // not check" on an answer where everything was checked.
        let mut d = Dossier {
            mint: Address::new([1u8; 32]),
            read_at: None,
            launch: None,
            curve: None,
            creator_transactions: None,
            unavailable: Vec::new(),
            calls: 0,
            elapsed_ms: 0,
        };
        assert!(!render(&d).contains("not available"));

        d.unavailable.push(radar_onchain::dossier::Unavailable {
            fact: "curve",
            why: "no bonding-curve account".to_owned(),
        });
        let text = render(&d);
        assert!(text.contains("not available"));
        assert!(text.contains("no bonding-curve account"));
    }

    #[test]
    fn creator_controlled_text_is_truncated_rather_than_printed_whole() {
        // A megabyte name is a denial of service against whoever reads the
        // reply, and against the reply's own length budget.
        let long = "a".repeat(1000);
        let rendered = safe(&long, 10);
        assert_eq!(
            rendered.chars().count(),
            11,
            "ten characters and an ellipsis"
        );
        assert!(rendered.ends_with('…'));
    }

    #[test]
    fn an_absent_dev_buy_never_renders_as_zero() {
        // Rule 9, and the reason it is tested at the *rendering* layer too: the
        // type keeps the distinction all the way here, and one `unwrap_or(0)`
        // in a format string would throw it away at the last step.
        let d = Dossier {
            mint: Address::new([1u8; 32]),
            read_at: None,
            launch: None,
            curve: None,
            creator_transactions: None,
            unavailable: Vec::new(),
            calls: 0,
            elapsed_ms: 0,
        };
        let text = render(&d);
        assert!(text.contains("read at slot  : unknown"));
        assert!(!text.contains("dev buy"));
    }
}
