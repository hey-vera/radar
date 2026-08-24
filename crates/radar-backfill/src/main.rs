// SPDX-License-Identifier: Apache-2.0
//! The backfill runner.
//!
//! Walks a time range in windows, converts each window's rows to events, and
//! appends them to the store. A window that times out server-side is halved and
//! retried rather than skipped — skipping would leave a gap in the record that
//! looks exactly like a quiet market.

use std::collections::BTreeMap;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use radar_asof::AsOf;
use radar_backfill::checkpoints;
use radar_backfill::extract::{Row, Skipped, Stats};
use radar_backfill::outcomes::{self, AggregateRow, HeadRow, TimeRow};
use radar_backfill::{Client, QueryError, Scope, events_from_rows, query_for_window};
use radar_store::{Event, Reader, Table, Writer};

/// Narrower than the server's sixty-second cap allows, because a window that
/// fits at three in the morning may not fit at peak.
const DEFAULT_WINDOW_MINUTES: i64 = 2;
/// Below this a window is small enough that a timeout means something else is
/// wrong, and halving further would only hammer the endpoint.
const MIN_WINDOW_SECONDS: i64 = 4;
/// Deliberate pacing. Radar is a guest on a free public endpoint (ADR 0002).
const PAUSE_BETWEEN_WINDOWS: Duration = Duration::from_millis(400);

/// How far behind wall-clock time follow mode stays.
///
/// Measured: CryptoHouse carries pump.fun instructions within a minute of the
/// chain. Five minutes is not latency Radar needs -- it explicitly does not
/// compete on speed -- it is margin against the trailing edge of ingestion being
/// partial, which would otherwise record a busy minute as a quiet one and never
/// revisit it.
const FOLLOW_LAG_SECONDS: i64 = 300;

/// How long follow mode waits when it has caught up.
const FOLLOW_IDLE: Duration = Duration::from_secs(60);

/// The smallest window follow mode will ask for.
///
/// Without this, a caught-up follower walks the horizon in three-second slices,
/// querying a free public endpoint every few hundred milliseconds for almost
/// nothing. Radar is a guest there (ADR 0002); waiting until a minute has
/// accumulated costs nothing it needs and is an order of magnitude fewer
/// queries.
const FOLLOW_MIN_WINDOW_SECONDS: i64 = 60;

/// Where the follow cursor lives inside the store.
const CURSOR_FILE: &str = ".follow-cursor";

struct Args {
    from: String,
    to: String,
    store: String,
    window_minutes: i64,
    scope: Scope,
    follow: bool,
    outcomes: bool,
}

fn usage() -> &'static str {
    "radar-backfill --from 'YYYY-MM-DD HH:MM:SS' --to 'YYYY-MM-DD HH:MM:SS' \
     --store <dir> [--window-minutes N] [--scope lifecycle|trades]
   radar-backfill --follow --store <dir> [--window-minutes N]

   radar-backfill --outcomes --store <dir>

--outcomes measures what became of every token already in the store: how long it
kept trading, how many transfers, how many distinct accounts. Those are the
labels every signal has to be validated against, and they are the one extraction
the thousand-row cap does not obstruct, because an aggregate returns one row per
mint however much it scans.

--follow keeps recording from where the store left off, staying five minutes
behind the chain and sleeping when caught up. It uses the same extraction path
as a one-off backfill, so history and live data are one code path."
}

fn parse_args() -> Result<Args, String> {
    let mut from = None;
    let mut to = None;
    let mut store = None;
    let mut window_minutes = DEFAULT_WINDOW_MINUTES;
    let mut scope = Scope::default();
    let mut follow = false;
    let mut measure_outcomes = false;

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
            "--follow" => follow = true,
            "--outcomes" => measure_outcomes = true,
            "-h" | "--help" => return Err(usage().to_owned()),
            other => return Err(format!("unknown flag {other}\n{}", usage())),
        }
    }

    let store = store.ok_or_else(|| {
        format!(
            "--store is required
{}",
            usage()
        )
    })?;

    // Follow mode has no explicit range: it starts from the store's own cursor
    // and runs until stopped, so requiring --from and --to would be asking for
    // values it is going to ignore.
    if follow || measure_outcomes {
        return Ok(Args {
            from: String::new(),
            to: String::new(),
            store,
            window_minutes: window_minutes.max(1),
            scope,
            follow,
            outcomes: measure_outcomes,
        });
    }

    Ok(Args {
        from: from.ok_or_else(|| {
            format!(
                "--from is required
{}",
                usage()
            )
        })?,
        to: to.ok_or_else(|| {
            format!(
                "--to is required
{}",
                usage()
            )
        })?,
        store,
        window_minutes: window_minutes.max(1),
        scope,
        follow,
        outcomes: measure_outcomes,
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

/// Reads the follow cursor, or `None` on a store that has never been followed.
fn read_cursor(store: &str) -> Option<i64> {
    let raw = std::fs::read_to_string(std::path::Path::new(store).join(CURSOR_FILE)).ok()?;
    to_epoch(raw.trim()).ok()
}

/// Writes the follow cursor.
///
/// Written only *after* the window's events are flushed to disk. The other order
/// would advance past a window whose events were never stored, and the gap would
/// be silent and permanent -- follow mode never looks backwards.
fn write_cursor(store: &str, at: i64) -> Result<(), String> {
    std::fs::create_dir_all(store).map_err(|e| e.to_string())?;
    std::fs::write(
        std::path::Path::new(store).join(CURSOR_FILE),
        from_epoch(at),
    )
    .map_err(|e| e.to_string())
}

fn now_epoch() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    )
    .unwrap_or(i64::MAX)
}

/// Keeps recording from where the store left off.
fn follow(args: &Args) -> Result<(), String> {
    let client = Client::default();
    let step = args.window_minutes * 60;

    // A fresh store starts one window back rather than at the epoch: the
    // alternative is silently attempting to backfill fifty-six years.
    let mut cursor =
        read_cursor(&args.store).unwrap_or_else(|| now_epoch() - FOLLOW_LAG_SECONDS - step);

    println!(
        "following from {} in {}-minute windows",
        from_epoch(cursor),
        args.window_minutes
    );
    println!("staying {FOLLOW_LAG_SECONDS}s behind the chain; ctrl-c to stop");

    let mut totals = Stats::default();
    loop {
        let horizon = now_epoch() - FOLLOW_LAG_SECONDS;
        if horizon - cursor < FOLLOW_MIN_WINDOW_SECONDS {
            std::thread::sleep(FOLLOW_IDLE);
            continue;
        }
        let window_end = (cursor + step).min(horizon);

        let rows =
            fetch_window(&client, cursor, window_end, 0, args.scope).map_err(|e| e.to_string())?;
        let (events, stats) = events_from_rows(&rows);
        let emitted = events.len();

        // Open, write and flush per window. Holding a writer across the whole
        // loop would buffer events indefinitely on a quiet market, and a process
        // killed mid-buffer would lose them.
        {
            let mut writer = Writer::open(&args.store, 20_000).map_err(|e| e.to_string())?;
            for e in events {
                writer.append(e).map_err(|e| e.to_string())?;
            }
            writer.flush().map_err(|e| e.to_string())?;
        }
        write_cursor(&args.store, window_end)?;
        merge(&mut totals, &stats);

        println!(
            "  {} .. {}  rows {:>5}  events {:>5}  skipped {:>4}  (total {})",
            from_epoch(cursor),
            from_epoch(window_end),
            rows.len(),
            emitted,
            stats.total_skipped(),
            totals.emitted
        );

        cursor = window_end;
        std::thread::sleep(PAUSE_BETWEEN_WINDOWS);
    }
}

/// Every token the store knows about, and when each graduation happened.
///
/// The graduation **slot** rather than a set of mints, because the outcome label
/// needs to distinguish a curve bought out in the launch block from one that
/// filled over days, and a membership test cannot.
type Universe = (
    Vec<(radar_types::Address, radar_types::Slot)>,
    BTreeMap<radar_types::Address, radar_types::Slot>,
);

/// Reads the token universe from the store, deduplicated by mint.
fn universe(reader: &Reader, as_of: AsOf) -> Result<Universe, String> {
    let mut launches: Vec<(radar_types::Address, radar_types::Slot)> = reader
        .read(Table::Launches, as_of)
        .map_err(|e| e.to_string())?
        .iter()
        .filter_map(|e| match e {
            Event::Launch(l) => Some((l.mint, l.envelope.slot)),
            _ => None,
        })
        .collect();
    // A mint can appear twice if a partition was written more than once. The
    // launch slot is the same either way, so the first wins.
    launches.sort_unstable();
    launches.dedup_by_key(|(mint, _)| *mint);

    // Earliest wins: a token graduates once, and a partition written twice must
    // not turn one event into a later-looking second one.
    let mut graduated: BTreeMap<radar_types::Address, radar_types::Slot> = BTreeMap::new();
    for event in reader
        .read(Table::Graduations, as_of)
        .map_err(|e| e.to_string())?
    {
        // A failed `migrate` moved nothing. It is worth recording — a migration
        // that was attempted and reverted is real information — but it is not a
        // graduation, and counting it as one inflated the rarest and most
        // load-bearing label in the store by about a third.
        if !event.envelope().succeeded {
            continue;
        }
        let slot = event.envelope().slot;
        graduated
            .entry(event.mint())
            .and_modify(|at| *at = (*at).min(slot))
            .or_insert(slot);
    }

    Ok((launches, graduated))
}

/// Which tokens have crossed a checkpoint their last measurement predates.
///
/// Measuring everything on every pass would re-measure a month of history a
/// million times over, almost all of it long settled. Measuring once would be
/// worse: a token seen an hour after launch and the same token seen a day later
/// are different observations, and the second is the one that says whether the
/// first meant anything.
fn due_for_measurement(
    launches: &[(radar_types::Address, radar_types::Slot)],
    already: &[radar_store::Outcome],
    head: radar_types::Slot,
) -> Vec<(radar_types::Address, radar_types::Slot)> {
    let mut newest_age: std::collections::BTreeMap<radar_types::Address, radar_types::SlotDelta> =
        std::collections::BTreeMap::new();
    for outcome in already {
        let age = checkpoints::age_of(outcome.launch_slot, outcome.measured_at);
        newest_age
            .entry(outcome.mint)
            .and_modify(|held| {
                if age > *held {
                    *held = age;
                }
            })
            .or_insert(age);
    }

    launches
        .iter()
        .copied()
        .filter(|(mint, launch_slot)| {
            checkpoints::needs_measuring(
                checkpoints::age_of(*launch_slot, head),
                newest_age.get(mint).copied(),
            )
        })
        .collect()
}

/// Measures what became of every token already in the store.
fn measure(args: &Args) -> Result<(), String> {
    let client = Client::default();
    let reader = Reader::open(&args.store);

    let watermark = Reader::watermark(&reader)
        .map_err(|e| e.to_string())?
        .ok_or("store is empty; nothing to measure")?;
    let as_of = AsOf::at(watermark);

    let (launches, graduated) = universe(&reader, as_of)?;

    if launches.is_empty() {
        return Err("store holds no launches to measure".to_owned());
    }
    let already = reader.read_outcomes(as_of).map_err(|e| e.to_string())?;

    // The head is the honest measurement slot: an outcome is a statement about
    // what had happened by a moment, and that moment is when it was asked.
    let head: Vec<HeadRow> = client
        .query(&outcomes::query_for_head())
        .map_err(|e| e.to_string())?;
    let measured_at = radar_types::Slot(
        head.first()
            .and_then(|h| h.head.parse().ok())
            .ok_or("could not read the chain head")?,
    );

    // The transfer table prunes by timestamp, not slot, so the earliest launch
    // slot is converted once and the whole run is bounded by it.
    let earliest_slot = launches
        .iter()
        .map(|(_, slot)| *slot)
        .min()
        .unwrap_or(radar_types::Slot(0));
    let times: Vec<TimeRow> = client
        .query(&outcomes::query_for_slot_time(earliest_slot))
        .map_err(|e| e.to_string())?;
    let since = times
        .first()
        .map(|t| t.at.clone())
        .filter(|t| !t.is_empty() && !t.starts_with("1970"))
        .ok_or("could not resolve a timestamp for the earliest launch slot")?;

    let total_known = launches.len();
    let due = due_for_measurement(&launches, &already, measured_at);

    if due.is_empty() {
        println!(
            "{total_known} tokens known, none due for measurement --              all are either too young for the first checkpoint or already settled"
        );
        return Ok(());
    }
    let launches = due;

    println!(
        "{total_known} tokens known, {} due; measuring as of slot {measured_at},          transfers since {since}, batches of {}",
        launches.len(),
        outcomes::MINTS_PER_BATCH
    );

    let mut writer = Writer::open(&args.store, 20_000).map_err(|e| e.to_string())?;
    let (mut written, mut stillborn, mut with_activity) = (0u64, 0u64, 0u64);

    for batch in launches.chunks(outcomes::MINTS_PER_BATCH) {
        let mints: Vec<String> = batch.iter().map(|(m, _)| m.to_string()).collect();
        let rows: Vec<AggregateRow> = client
            .query(&outcomes::query_for_mints(&mints, &since))
            .map_err(|e| e.to_string())?;

        let measured = outcomes::outcomes_from_rows(&rows, batch, measured_at, &graduated);
        for outcome in measured {
            if outcome.appears_stillborn() {
                stillborn += 1;
            }
            if outcome.transfers > 0 {
                with_activity += 1;
            }
            writer.append_outcome(outcome).map_err(|e| e.to_string())?;
            written += 1;
        }
        println!("  {written}/{} measured", launches.len());
        std::thread::sleep(PAUSE_BETWEEN_WINDOWS);
    }
    writer.flush().map_err(|e| e.to_string())?;

    println!(
        "
--- measured {written} tokens as of slot {measured_at} ---"
    );
    println!("  with any transfer     : {with_activity}");
    println!("  apparently stillborn  : {stillborn}");
    if written > 0 {
        #[expect(clippy::cast_precision_loss, reason = "a display ratio")]
        let share = stillborn as f64 / written as f64 * 100.0;
        println!("  stillborn share       : {share:.1}%");
    }
    println!(
        "
These are labels, not verdicts. Whether any of them predicts anything"
    );
    println!("is a question for the research store to answer against them.");
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
    let result = if args.outcomes {
        measure(&args)
    } else if args.follow {
        follow(&args)
    } else {
        run(&args)
    };
    match result {
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
