// SPDX-License-Identifier: Apache-2.0
//! The operator command line.
//!
//! Everything here reads live state and computes nothing it does not have. The
//! point is that `radar-cli` can never be stale: it opens the store and reports
//! what is actually in it, so a claim about what Radar has recorded is checkable
//! rather than remembered.

use std::collections::{BTreeMap, BTreeSet};
use std::process::ExitCode;

use radar_asof::AsOf;
use radar_instruments::{Context, CreatorHistory, CreatorTrackRecord, Registry, SimulateExit};
mod basis;
mod brief;
mod consider;
mod graduations;
mod replay;
mod selection;
mod study;

use radar_sim::{JupiterQuoter, RpcClient};
use radar_store::{Event, Reader, Table};
use radar_types::Slot;

fn usage() -> &'static str {
    "radar-cli <command>

commands:
  brief --store <dir>            is the system healthy right now; exits
                                 non-zero when something is out of bounds
  inspect --store <dir>          what the store holds
  launches --store <dir> [-n N]  the most recent launches
  creators --store <dir> [-n N]  creators by launch count
  tools                          the instrument catalogue, with prices
  call <name> --store <dir> --args '<json>'
                                 run an instrument and print its record
  exit <mint> [--size N]         can this token actually be sold, and at what size
  graduations --store <dir> [-n N]
                                 every graduation recorded, and the population
                                 rate the creator signal is measured against
  consider --store <dir> [--window N] [--cap N] [--record [dir]]
                                 run the whole decision lane over recorded
                                 tokens; commits nothing. --record appends what
                                 was decided to the store's decisions table,
                                 which is what a later join against prices needs
  replay --store <dir> --record <file> [--window N] [--cohort N]
                                 record what the strategy decides now
  replay --store <dir> --check <file>
                                 re-derive those decisions and report what moved
  study --store <dir> [--pivot N]
                                 does a creator's record predict their next
                                 launch; splits the store and compares
  selection --store <dir> [--cost-bps N]
                                 did the selection beat the population it
                                 selected from; the question the project exists
                                 to answer
  basis --store <dir>            how much of that return is the gap between a
                                 sell quote and a realised fill, rather than a
                                 gain; a correction `selection` owes
"
}

/// Reads every event the store holds.
fn read_all(reader: &Reader) -> Result<Vec<Event>, String> {
    let watermark = Reader::watermark(reader).map_err(|e| e.to_string())?;
    let Some(top) = watermark else {
        return Ok(Vec::new());
    };
    let as_of = AsOf::at(top);
    let mut all = Vec::new();
    for table in Table::EVENT_TABLES {
        all.extend(reader.read(*table, as_of).map_err(|e| e.to_string())?);
    }
    Ok(all)
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn store_of(args: &[String]) -> Result<Reader, String> {
    let dir = flag(args, "--store").ok_or_else(|| format!("--store is required\n\n{}", usage()))?;
    Ok(Reader::open(dir))
}

fn limit_of(args: &[String]) -> usize {
    flag(args, "-n")
        .or_else(|| flag(args, "--limit"))
        .and_then(|v| v.parse().ok())
        .unwrap_or(20)
}

fn inspect(args: &[String]) -> Result<(), String> {
    let reader = store_of(args)?;
    let events = read_all(&reader)?;

    if events.is_empty() {
        println!("store is empty");
        return Ok(());
    }

    let slots: Vec<Slot> = events.iter().map(Event::slot).collect();
    let (lo, hi) = (
        slots.iter().min().copied().unwrap_or_default(),
        slots.iter().max().copied().unwrap_or_default(),
    );
    let span = hi.saturating_since(lo);

    println!("events        : {}", events.len());
    println!(
        "slot range    : {lo} .. {hi}  ({span}, ~{:.1} h of chain)",
        span.approx_duration().as_secs_f64() / 3600.0
    );
    println!(
        "distinct mints: {}",
        events
            .iter()
            .map(Event::mint)
            .collect::<BTreeSet<_>>()
            .len()
    );

    let mut by_table: BTreeMap<&str, usize> = BTreeMap::new();
    let mut by_instruction: BTreeMap<String, usize> = BTreeMap::new();
    let mut unknown = 0usize;
    let mut failed = 0usize;
    for e in &events {
        *by_table
            .entry(match e {
                Event::Launch(_) => "launches",
                Event::Trade(_) => "trades",
                Event::Graduation(_) => "graduations",
            })
            .or_default() += 1;
        let origin = match e {
            Event::Launch(l) => &l.origin,
            Event::Trade(t) => &t.origin,
            Event::Graduation(g) => &g.origin,
        };
        *by_instruction
            .entry(origin.instruction.clone())
            .or_default() += 1;
        if !origin.known {
            unknown += 1;
        }
        if !e.envelope().succeeded {
            failed += 1;
        }
    }

    println!("\nby table:");
    for (t, n) in &by_table {
        println!("  {t:<14} {n}");
    }
    println!("\nby instruction:");
    for (i, n) in &by_instruction {
        println!("  {i:<26} {n}");
    }
    println!("\nfailed transactions: {failed}");
    // The program-upgrade alarm. A decoder that has stopped understanding a
    // program looks exactly like a program that has gone quiet, so this number
    // climbing is the signal to go and look.
    println!("unknown instructions: {unknown}");
    if unknown > 0 {
        println!("  ^ a rising count means a program upgrade; add the discriminator");
    }

    for table in Table::ALL {
        let files = reader.files(*table).map_err(|e| e.to_string())?;
        if !files.is_empty() {
            println!("\n{} partition files: {}", table.dir(), files.len());
        }
    }
    Ok(())
}

fn launches(args: &[String]) -> Result<(), String> {
    let reader = store_of(args)?;
    let mut events: Vec<Event> = read_all(&reader)?
        .into_iter()
        .filter(|e| matches!(e, Event::Launch(_)))
        .collect();
    events.sort_by_key(|b| std::cmp::Reverse(b.slot()));

    println!("{:>12}  {:<44}  {:<12}  NAME", "SLOT", "MINT", "SYMBOL");
    for e in events.iter().take(limit_of(args)) {
        let Event::Launch(l) = e else { continue };
        // Creator-supplied text is arbitrary and may contain control characters,
        // zero-width spaces or right-to-left overrides. Rendering it raw into a
        // terminal is how a token name rewrites the line above it.
        println!(
            "{:>12}  {:<44}  {:<12}  {}",
            l.envelope.slot,
            l.mint,
            sanitise(&l.symbol, 12),
            sanitise(&l.name, 40)
        );
    }
    Ok(())
}

fn creators(args: &[String]) -> Result<(), String> {
    let reader = store_of(args)?;
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for e in read_all(&reader)? {
        if let Event::Launch(l) = e {
            *counts.entry(l.creator.to_string()).or_default() += 1;
        }
    }
    let mut ranked: Vec<(String, usize)> = counts.into_iter().collect();
    ranked.sort_by_key(|(addr, n)| (std::cmp::Reverse(*n), addr.clone()));

    let repeat = ranked.iter().filter(|(_, n)| *n > 1).count();
    println!(
        "distinct creators: {}  ({repeat} launched more than once)",
        ranked.len()
    );
    println!(
        "
{:>7}  CREATOR",
        "LAUNCHES"
    );
    for (addr, n) in ranked.iter().take(limit_of(args)) {
        println!("{n:>7}  {addr}");
    }
    Ok(())
}

/// Renders untrusted text safely for a terminal.
///
/// Token names and symbols are arbitrary creator-controlled bytes. Printing them
/// raw lets a launch move the cursor, rewrite earlier output, or hide characters
/// behind a right-to-left override — which for an operator staring at a list of
/// candidates is a way to make one token look like another.
fn sanitise(s: &str, width: usize) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}') {
                '·'
            } else {
                c
            }
        })
        .take(width)
        .collect();
    cleaned
}

/// Every instrument Radar exposes.
///
/// One registry backs the CLI, the HTTP routes, the x402 price list and the MCP
/// catalogue. A second list maintained anywhere else would drift.
fn registry() -> Registry {
    let mut r = Registry::new();
    r.register(CreatorHistory);
    r.register(CreatorTrackRecord);
    r.register(SimulateExit::default());
    r
}

fn tools() {
    let registry = registry();
    println!(
        "{:<20} {:>5}  {:>6}  {:>9}  SUMMARY",
        "NAME", "VER", "LATENCY", "PRICE"
    );
    for instrument in registry.iter() {
        let spec = instrument.spec();
        let price = spec.public_price(radar_instruments::DEFAULT_MARGIN_PERCENT);
        println!(
            "{:<20} {:>5}  {:>6?}  {:>9}  {}",
            spec.name,
            spec.version.to_string(),
            spec.latency,
            price.to_string(),
            spec.summary
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        );
    }
}

fn call(args: &[String]) -> Result<(), String> {
    let name = args
        .get(1)
        .filter(|a| !a.starts_with('-'))
        .ok_or("call needs an instrument name")?;
    let reader = store_of(args)?;
    let raw = flag(args, "--args").unwrap_or_else(|| "{}".to_owned());
    let parsed: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| format!("--args is not valid JSON: {e}"))?;

    let watermark = Reader::watermark(&reader)
        .map_err(|e| e.to_string())?
        .ok_or("store is empty, so nothing can be answered as of any slot")?;
    let as_of = flag(args, "--as-of")
        .and_then(|v| v.parse().ok())
        .map_or(watermark, radar_types::Slot);

    let ctx = Context {
        as_of: AsOf::at(as_of),
        store: &reader,
    };
    let record = registry()
        .invoke(name, parsed, &ctx)
        .map_err(|e| e.to_string())?;

    println!(
        "{}",
        serde_json::to_string_pretty(&record).map_err(|e| e.to_string())?
    );
    if record.error.is_some() {
        return Err("instrument failed; the record above says why".to_owned());
    }
    Ok(())
}

/// Formats lamports as SOL without going through a float.
///
/// A lamport count exceeds f64's exact integer range, and this number is read by
/// someone deciding how much to risk.
fn sol(lamports: u64) -> String {
    format!(
        "{}.{:09}",
        lamports / 1_000_000_000,
        lamports % 1_000_000_000
    )
}

/// Answers whether a token can actually be sold, and at what size.
///
/// The question Radar asks before any other, and the one the risk kernel refuses
/// a position without.
fn exit_analysis(args: &[String]) -> Result<(), String> {
    let mint_arg = args
        .get(1)
        .filter(|a| !a.starts_with('-'))
        .ok_or("exit needs a mint address")?;
    let mint: radar_types::Address = mint_arg
        .parse()
        .map_err(|_| format!("`{mint_arg}` is not a base58 address"))?;
    let size: u64 = flag(args, "--size")
        .and_then(|v| v.parse().ok())
        // A million base units is one token at six decimals, which is the
        // scale pump.fun mints use.
        .unwrap_or(1_000_000_000);

    println!("{mint}");
    let structure = report_structure(&mint);
    let report = radar_sim::probe(&JupiterQuoter::default(), &mint, structure, size);
    report_exit(&report, size);
    Ok(())
}

/// Reads the mint account and prints what it says.
///
/// Structure first: it is free, and it can rule a token out before any quote is
/// worth asking for.
fn report_structure(mint: &radar_types::Address) -> Option<radar_sim::MintStructure> {
    match RpcClient::default().mint_structure(mint) {
        Ok(s) => {
            println!(
                "
  decimals          : {}",
                s.decimals
            );
            println!("  supply            : {}", s.supply);
            println!(
                "  mint authority    : {}",
                s.mint_authority
                    .map_or_else(|| "revoked".to_owned(), |a| a.to_string())
            );
            println!(
                "  freeze authority  : {}",
                s.freeze_authority
                    .map_or_else(|| "revoked".to_owned(), |a| a.to_string())
            );
            println!(
                "  program           : {}",
                if s.token_2022 { "Token-2022" } else { "Token" }
            );
            if s.extensions.is_empty() {
                println!("  extensions        : none");
            } else {
                println!("  extensions        : {:?}", s.extensions);
            }
            Some(s)
        }
        Err(e) => {
            println!(
                "
  structure         : UNREADABLE ({e})"
            );
            None
        }
    }
}

/// Prints the sell curve and what it implies.
fn report_exit(report: &radar_sim::ExitReport, size: u64) {
    println!(
        "
  sell curve (from {size} base units):"
    );
    if report.curve.is_empty() {
        println!("    nothing quotable at any size probed");
    }
    for point in &report.curve {
        let impact = if point.impact_bps == u32::MAX {
            "unknown".to_owned()
        } else {
            format!("{:.2}%", f64::from(point.impact_bps) / 100.0)
        };
        println!(
            "    {:>16} units -> {:>12} lamports  ({} SOL, impact {impact})",
            point.size_tokens,
            point.out_lamports,
            sol(point.out_lamports)
        );
    }
    for size in &report.no_route_at {
        println!("    {size:>16} units -> no route");
    }

    println!(
        "
  capacity within an impact budget:"
    );
    for (bps, capacity) in radar_sim::capacity_table(report) {
        match capacity {
            Some(lamports) => {
                println!(
                    "    <= {:.2}%  : {} SOL",
                    f64::from(bps) / 100.0,
                    sol(lamports)
                );
            }
            None => println!("    <= {:.2}%  : nothing fits", f64::from(bps) / 100.0),
        }
    }

    println!(
        "
  confidence        : {:?}",
        report.confidence
    );
    if !report.structural_threats.is_empty() {
        println!("  structural threats: {:?}", report.structural_threats);
    }
    println!("  can be stopped    : {}", report.can_be_stopped);
    println!(
        "  EXITABLE          : {}",
        if report.is_exitable() { "yes" } else { "NO" }
    );
    if !report.is_exitable() {
        println!(
            "
  The risk kernel refuses a position without a measured exit, so this"
        );
        println!("  token cannot be sized at all until that changes.");
    }
}

/// Reports every graduation the store holds.
fn graduation_report(args: &[String]) -> Result<(), String> {
    graduations::run(&store_of(args)?, limit_of(args))
}

/// Runs the whole decision lane over recorded tokens.
fn decision_lane(args: &[String]) -> Result<(), String> {
    let reader = store_of(args)?;
    let window = flag(args, "--window")
        .and_then(|v| v.parse().ok())
        // ~24 hours at 2.5 slots a second. A token older than this has either
        // been considered already or is no longer a launch.
        .unwrap_or(216_000);
    let cap = flag(args, "--cap")
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(consider::default_cap);
    let record_to = record_target(args);
    consider::run(&reader, window, cap, record_to.as_deref())
}

/// Where `consider` should record its decisions, if anywhere.
///
/// `None` unless `--record` is present, because a read-only command that
/// quietly starts writing to production is not a flag anyone should have to
/// notice. `--record` alone writes to `--store`; `--record <dir>` writes
/// elsewhere, which is what a dry run wants.
fn record_target(args: &[String]) -> Option<String> {
    if !args.iter().any(|a| a == "--record") {
        return None;
    }
    // A following value that is itself a flag is the next flag, not a path.
    flag(args, "--record")
        .filter(|v| !v.starts_with("--"))
        .or_else(|| flag(args, "--store"))
}

/// Reports whether the selection beat the population.
fn selection_report(args: &[String]) -> Result<(), String> {
    let reader = store_of(args)?;
    let cost_bps = flag(args, "--cost-bps")
        .and_then(|v| v.parse().ok())
        // The measured round trip, not a placeholder. See `Thresholds`.
        .unwrap_or(radar_strategy::creator_edge::Thresholds::DEFAULT.assumed_round_trip_bps);
    selection::run(&reader, cost_bps)
}

/// Reports the gap between the quoted and realised price instruments.
fn basis_report(args: &[String]) -> Result<(), String> {
    let reader = store_of(args)?;
    basis::run(&reader)
}

/// Records or re-checks decisions, proving they reproduce.
fn replay_lane(args: &[String]) -> Result<(), String> {
    let reader = store_of(args)?;
    if let Some(path) = flag(args, "--check") {
        return replay::check(&reader, std::path::Path::new(&path));
    }
    let Some(path) = flag(args, "--record") else {
        return Err("replay needs --record <file> or --check <file>".to_owned());
    };
    let window = flag(args, "--window")
        .and_then(|v| v.parse().ok())
        .unwrap_or(216_000);
    let cohort = flag(args, "--cohort")
        .and_then(|v| v.parse().ok())
        .unwrap_or(replay::DEFAULT_COHORT);
    replay::record_to(&reader, std::path::Path::new(&path), window, cohort)
}

/// Prints the operational brief. Exits non-zero when something is wrong, so a
/// cron line can alarm on it without parsing the output.
/// Which serving endpoint `brief` should probe, if any.
///
/// Flag first, then environment: the deploy sets the environment so the timer
/// never runs blind, and a flag lets an operator point at another instance.
///
/// **An empty value counts as unset.** `RADAR_SERVE_URL=` in a unit file, and the
/// empty default of the `just brief` recipe, are both how "no endpoint" gets
/// spelled in practice. Treating that as a URL turns a *missing config* report
/// into a `bad uri` *failure*, and those are the two things rule 8 exists to keep
/// apart — one means nobody configured the check, the other means the server is
/// broken, and they send an operator to different places.
fn resolve_serve_url(from_flag: Option<String>, from_env: Option<String>) -> Option<String> {
    from_flag.or(from_env).filter(|url| !url.trim().is_empty())
}

fn brief_report(args: &[String]) -> Result<(), String> {
    let store = flag(args, "--store").ok_or("brief needs --store <dir>")?;
    // Flag first, then environment. The deploy sets the environment so the timer
    // never runs blind; a flag lets an operator point at another instance.
    let serve_url = resolve_serve_url(
        flag(args, "--serve-url"),
        std::env::var("RADAR_SERVE_URL").ok(),
    );
    if brief::run(std::path::Path::new(&store), serve_url.as_deref()) {
        Ok(())
    } else {
        // A distinct message from an error: the brief worked, the system is not
        // well. Conflating them would make a broken monitor and a broken
        // recorder look identical to whatever is watching.
        Err("unhealthy".to_owned())
    }
}

/// Runs the event study over recorded data.
fn event_study(args: &[String]) -> Result<(), String> {
    let reader = store_of(args)?;
    let pivot = flag(args, "--pivot").and_then(|v| v.parse().ok());
    study::run(&reader, pivot)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = args.first() else {
        eprint!("{}", usage());
        return ExitCode::FAILURE;
    };

    let result = match command.as_str() {
        "brief" => brief_report(&args),
        "inspect" => inspect(&args),
        "launches" => launches(&args),
        "creators" => creators(&args),
        "exit" => exit_analysis(&args),
        "graduations" => graduation_report(&args),
        "consider" => decision_lane(&args),
        "replay" => replay_lane(&args),
        "study" => event_study(&args),
        "selection" => selection_report(&args),
        "basis" => basis_report(&args),
        "tools" => {
            tools();
            Ok(())
        }
        "call" => call(&args),
        "-h" | "--help" | "help" => {
            print!("{}", usage());
            return ExitCode::SUCCESS;
        }
        other => Err(format!("unknown command {other}\n\n{}", usage())),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("{msg}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|p| (*p).to_owned()).collect()
    }

    #[test]
    fn recording_is_off_unless_asked_for() {
        // The direction that matters. A `consider` run that silently appended to
        // the production store would be a read-only command with a side effect,
        // and nobody would find out until the store had rows nobody meant.
        assert_eq!(record_target(&argv(&["consider", "--store", "s"])), None);
    }

    #[test]
    fn record_without_a_path_writes_to_the_store_being_read() {
        assert_eq!(
            record_target(&argv(&["consider", "--store", "s", "--record"])),
            Some("s".to_owned())
        );
        // And the next flag is a flag, not a directory named "--cap".
        assert_eq!(
            record_target(&argv(&[
                "consider", "--store", "s", "--record", "--cap", "8"
            ])),
            Some("s".to_owned()),
            "a following flag must not be mistaken for a path"
        );
    }

    #[test]
    fn record_with_a_path_writes_there_instead() {
        // What a dry run wants: exercise the whole lane without touching the
        // store it read from.
        assert_eq!(
            record_target(&argv(&[
                "consider", "--store", "prod", "--record", "scratch"
            ])),
            Some("scratch".to_owned())
        );
    }

    #[test]
    fn the_usage_text_names_every_command_the_dispatcher_accepts() {
        // `--record` was added and very nearly shipped undocumented; mutation
        // testing then found that nothing constrained this string at all, so
        // blanking it entirely left the suite green.
        let u = usage();
        for command in [
            "brief",
            "inspect",
            "launches",
            "creators",
            "tools",
            "call",
            "exit",
            "graduations",
            "consider",
            "replay",
            "study",
        ] {
            assert!(
                u.contains(command),
                "usage() does not mention {command}:\n{u}"
            );
        }
        assert!(u.contains("--record"), "the recording flag is undocumented");
    }

    use super::*;

    #[test]
    fn control_characters_in_a_token_name_cannot_rewrite_the_terminal() {
        // A name carrying a carriage return would otherwise overwrite the line
        // above it, which is a cheap way to make one candidate look like another.
        assert_eq!(sanitise("evil\r\nname", 40), "evil··name");
        assert_eq!(sanitise("tab\there", 40), "tab·here");
    }

    #[test]
    fn zero_width_and_bidi_overrides_are_made_visible() {
        // Two tokens whose names differ only by an invisible character must not
        // render identically.
        assert_eq!(sanitise("A\u{200b}B", 40), "A·B");
        assert_eq!(sanitise("safe\u{202e}drawkcab", 40), "safe·drawkcab");
    }

    #[test]
    fn ordinary_text_including_emoji_survives() {
        assert_eq!(sanitise("p down 🚀", 40), "p down 🚀");
    }

    #[test]
    fn output_is_truncated_to_the_column_width() {
        assert_eq!(sanitise("0123456789abcdef", 8), "01234567");
    }

    #[test]
    fn an_empty_serving_endpoint_is_the_same_as_no_endpoint() {
        // Found on the deployed instance: `RADAR_SERVE_URL=` reported
        // "serving  is not answering: bad uri: /health is missing scheme" —
        // a FAIL that reads as a broken server when the truth is that nobody
        // configured the check. Both alarm, but they are not the same finding.
        assert_eq!(resolve_serve_url(None, Some(String::new())), None);
        assert_eq!(resolve_serve_url(None, Some("   ".to_owned())), None);
        assert_eq!(resolve_serve_url(Some(String::new()), None), None);
        assert_eq!(resolve_serve_url(None, None), None);
    }

    #[test]
    fn a_real_endpoint_survives_and_the_flag_wins() {
        // The other direction, so the rule above is not satisfied by discarding
        // everything.
        assert_eq!(
            resolve_serve_url(None, Some("http://127.0.0.1:8402".to_owned())),
            Some("http://127.0.0.1:8402".to_owned())
        );
        assert_eq!(
            resolve_serve_url(
                Some("http://elsewhere:1".to_owned()),
                Some("http://127.0.0.1:8402".to_owned())
            ),
            Some("http://elsewhere:1".to_owned()),
            "an explicit flag beats the deploy's environment"
        );
    }

    #[test]
    fn flags_are_read_positionally() {
        let args: Vec<String> = ["launches", "--store", "/tmp/s", "-n", "5"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        assert_eq!(flag(&args, "--store"), Some("/tmp/s".to_owned()));
        assert_eq!(limit_of(&args), 5);
        assert_eq!(limit_of(&args[..1]), 20, "defaults when absent");
    }
}
