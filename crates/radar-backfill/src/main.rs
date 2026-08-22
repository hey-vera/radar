// SPDX-License-Identifier: Apache-2.0
//! The backfill runner.
//!
//! Walks a time range in windows, converts each window's rows to events, and
//! appends them to the store. A window that times out server-side is halved and
//! retried rather than skipped — skipping would leave a gap in the record that
//! looks exactly like a quiet market.

use std::process::ExitCode;
use std::time::{Duration, Instant};

use radar_backfill::extract::{Row, Skipped, Stats};
use radar_backfill::{Client, QueryError, Scope, events_from_rows, query_for_window};
use radar_store::Writer;

/// Narrower than the server's sixty-second cap allows, because a window that
/// fits at three in the morning may not fit at peak.
const DEFAULT_WINDOW_MINUTES: i64 = 2;
/// Below this a window is small enough that a timeout means something else is
/// wrong, and halving further would only hammer the endpoint.
const MIN_WINDOW_SECONDS: i64 = 4;
/// Deliberate pacing. Radar is a guest on a free public endpoint (ADR 0002).
const PAUSE_BETWEEN_WINDOWS: Duration = Duration::from_millis(400);

struct Args {
    from: String,
    to: String,
    store: String,
    window_minutes: i64,
    scope: Scope,
}

fn usage() -> &'static str {
    "radar-backfill --from 'YYYY-MM-DD HH:MM:SS' --to 'YYYY-MM-DD HH:MM:SS' \
     --store <dir> [--window-minutes N] [--scope lifecycle|trades]"
}

fn parse_args() -> Result<Args, String> {
    let mut from = None;
    let mut to = None;
    let mut store = None;
    let mut window_minutes = DEFAULT_WINDOW_MINUTES;
    let mut scope = Scope::default();

    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = || it.next().ok_or_else(|| format!("{flag} needs a value"));
        match flag.as_str() {
            "--from" => from = Some(value()?),
            "--to" => to = Some(value()?),
            "--store" => store = Some(value()?),
            "--window-minutes" => {
                window_minutes = value()?
                    .parse()
                    .map_err(|_| "window-minutes must be a number".to_owned())?;
            }
            "--scope" => {
                scope = match value()?.as_str() {
                    "lifecycle" => Scope::Lifecycle,
                    "trades" => Scope::Trades,
                    other => {
                        return Err(format!(
                            "unknown scope {other}; expected lifecycle or trades"
                        ));
                    }
                };
            }
            "-h" | "--help" => return Err(usage().to_owned()),
            other => return Err(format!("unknown flag {other}\n{}", usage())),
        }
    }

    Ok(Args {
        from: from.ok_or_else(|| format!("--from is required\n{}", usage()))?,
        to: to.ok_or_else(|| format!("--to is required\n{}", usage()))?,
        store: store.ok_or_else(|| format!("--store is required\n{}", usage()))?,
        window_minutes: window_minutes.max(1),
        scope,
    })
}

/// Seconds since the epoch for a `YYYY-MM-DD HH:MM:SS` timestamp, treated as UTC.
///
/// A hand-rolled conversion rather than a date crate: this is the only date
/// arithmetic in the workspace, and it is not worth a dependency that would
/// then be in the tree of every process that links the store.
fn to_epoch(stamp: &str) -> Result<i64, String> {
    let bad = || format!("expected 'YYYY-MM-DD HH:MM:SS', got '{stamp}'");
    let (date, clock) = stamp.split_once(' ').ok_or_else(bad)?;
    let ymd: Vec<i64> = date.split('-').map(|p| p.parse().unwrap_or(-1)).collect();
    let hms: Vec<i64> = clock.split(':').map(|p| p.parse().unwrap_or(-1)).collect();
    if ymd.len() != 3 || hms.len() != 3 || ymd.iter().chain(&hms).any(|v| *v < 0) {
        return Err(bad());
    }
    let (year, month, day) = (ymd[0], ymd[1], ymd[2]);
    // Days from civil, per Howard Hinnant's algorithm: March-based years, so
    // the leap day lands at the end and needs no special case.
    let shifted_year = year - i64::from(month <= 2);
    let era = if shifted_year >= 0 {
        shifted_year
    } else {
        shifted_year - 399
    } / 400;
    let year_of_era = shifted_year - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    Ok(days * 86_400 + hms[0] * 3_600 + hms[1] * 60 + hms[2])
}

fn from_epoch(mut secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    secs = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}",
        secs / 3_600,
        (secs / 60) % 60,
        secs % 60
    )
}

fn merge(into: &mut Stats, from: &Stats) {
    into.emitted += from.emitted;
    for (k, v) in &from.skipped {
        *into.skipped.entry(*k).or_default() += v;
    }
}

/// Fetches one window, halving it on a server timeout.
fn fetch_window(
    client: &Client,
    from: i64,
    to: i64,
    depth: u32,
    scope: Scope,
) -> Result<Vec<Row>, QueryError> {
    let sql = query_for_window(&from_epoch(from), &from_epoch(to), scope);
    match client.query::<Row>(&sql) {
        Ok(rows) => Ok(rows),
        Err(e) if e.should_narrow() && (to - from) > MIN_WINDOW_SECONDS && depth < 10 => {
            let mid = from + (to - from) / 2;
            eprintln!(
                "    window too wide ({}), halving: {} .. {}",
                if e.to_string().contains("TOO_MANY_ROWS") {
                    "row cap"
                } else {
                    "timeout"
                },
                from_epoch(from),
                from_epoch(to)
            );
            let mut rows = fetch_window(client, from, mid, depth + 1, scope)?;
            std::thread::sleep(PAUSE_BETWEEN_WINDOWS);
            rows.extend(fetch_window(client, mid, to, depth + 1, scope)?);
            Ok(rows)
        }
        Err(e) => Err(e),
    }
}

fn run(args: &Args) -> Result<(), String> {
    let start = to_epoch(&args.from)?;
    let end = to_epoch(&args.to)?;
    if end <= start {
        return Err("--to must be after --from".to_owned());
    }

    let client = Client::default();
    let mut writer = Writer::open(&args.store, 20_000).map_err(|e| e.to_string())?;
    let mut totals = Stats::default();
    let mut rows_seen = 0u64;
    let step = args.window_minutes * 60;
    let began = Instant::now();

    println!(
        "backfilling {:?} {} .. {} in {}-minute windows into {}",
        args.scope, args.from, args.to, args.window_minutes, args.store
    );

    let mut cursor = start;
    while cursor < end {
        let window_end = (cursor + step).min(end);
        let rows =
            fetch_window(&client, cursor, window_end, 0, args.scope).map_err(|e| e.to_string())?;
        rows_seen += rows.len() as u64;

        let (events, stats) = events_from_rows(&rows);
        let emitted = events.len();
        for e in events {
            writer.append(e).map_err(|e| e.to_string())?;
        }
        merge(&mut totals, &stats);

        println!(
            "  {} .. {}  rows {:>6}  events {:>6}  skipped {:>5}",
            from_epoch(cursor),
            from_epoch(window_end),
            rows.len(),
            emitted,
            stats.total_skipped()
        );

        cursor = window_end;
        std::thread::sleep(PAUSE_BETWEEN_WINDOWS);
    }

    writer.flush().map_err(|e| e.to_string())?;

    println!(
        "\n--- backfill complete in {:.1}s ---",
        began.elapsed().as_secs_f64()
    );
    println!("  rows fetched  : {rows_seen}");
    println!("  events written: {}", totals.emitted);
    println!("  files written : {}", writer.written_files());
    if let Some(rate) = totals.yield_rate() {
        println!("  yield         : {:.1}%", rate * 100.0);
    }
    if totals.total_skipped() > 0 {
        println!("  skipped:");
        for (why, n) in &totals.skipped {
            let note = match why {
                Skipped::UnknownInstruction => "  <-- a program upgrade; add the discriminator",
                Skipped::AmbiguousMint => "  <-- refused rather than guessed",
                _ => "",
            };
            println!("    {why:?}: {n}{note}");
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::FAILURE;
        }
    };
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("backfill failed: {msg}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_round_trip_through_epoch_seconds() {
        for s in [
            "2026-08-21 06:00:00",
            "2026-01-01 00:00:00",
            "2024-02-29 23:59:59",
            "2000-03-01 12:34:56",
            "1970-01-01 00:00:00",
        ] {
            let e = to_epoch(s).unwrap_or_else(|err| panic!("{s}: {err}"));
            assert_eq!(from_epoch(e), s, "round trip failed for {s}");
        }
    }

    #[test]
    fn a_known_epoch_is_correct() {
        // Cross-checked against Python's calendar.timegm, not against memory: an
        // earlier version of this assertion carried an invented constant and the
        // test failed on the assertion rather than the code.
        assert_eq!(
            to_epoch("2026-08-21 06:00:00").expect("parses"),
            1_787_292_000
        );
        assert_eq!(to_epoch("1970-01-01 00:00:00").expect("parses"), 0);
        // One day apart, which needs no external reference to check.
        assert_eq!(
            to_epoch("2026-08-22 06:00:00").expect("parses")
                - to_epoch("2026-08-21 06:00:00").expect("parses"),
            86_400
        );
    }

    #[test]
    fn leap_days_are_handled() {
        let a = to_epoch("2024-02-28 00:00:00").expect("parses");
        let b = to_epoch("2024-03-01 00:00:00").expect("parses");
        assert_eq!(b - a, 2 * 86_400, "2024 is a leap year");
        let c = to_epoch("2023-02-28 00:00:00").expect("parses");
        let d = to_epoch("2023-03-01 00:00:00").expect("parses");
        assert_eq!(d - c, 86_400, "2023 is not");
    }

    #[test]
    fn a_malformed_timestamp_is_rejected_rather_than_silently_zero() {
        // Falling back to the epoch would backfill fifty-six years of nothing and
        // report success.
        for bad in ["2026-08-21", "not a date", "2026/08/21 06:00:00", ""] {
            assert!(to_epoch(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn stats_merge_by_reason() {
        let mut a = Stats {
            emitted: 2,
            ..Stats::default()
        };
        let mut b = Stats {
            emitted: 3,
            ..Stats::default()
        };
        b.skipped.insert(Skipped::NoMint, 4);
        merge(&mut a, &b);
        assert_eq!(a.emitted, 5);
        assert_eq!(a.skipped.get(&Skipped::NoMint), Some(&4));
    }
}
