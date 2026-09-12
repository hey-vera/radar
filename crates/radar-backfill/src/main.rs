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
use radar_backfill::coverage;
use radar_backfill::extract::{Row, Skipped, Stats};
use radar_backfill::market::{fold as market_fold, query as market_query};
use radar_backfill::market_tape;
use radar_backfill::outcomes::{self, AggregateRow, HeadRow, TimeRow};
use radar_backfill::prices::{self, PriceRow};
use radar_backfill::{Client, QueryError, Scope, events_from_rows, query_for_window};
use radar_store::{Completion, Event, Reader, Table, Writer, from_epoch, now_epoch, to_epoch};

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

/// How long follow mode waits after a window fails, and how far that grows.
///
/// A one-off backfill should fail loudly and stop: an operator is watching it,
/// and the range can be re-run. A **daemon must not**, and this one did. On
/// 2026-08-24 a two-minute burst of 7,233 failed `migrate` spam transactions
/// pushed one window past the endpoint's row cap; the query eventually returned
/// a bare HTTP 500, the error propagated out of the loop, and the recorder
/// exited. It stayed down for hours, and the shape of that outage in the store
/// is a slot range with nothing in it — which looks exactly like a quiet market,
/// the one failure this project is most organised against.
///
/// So follow mode retries and never gives up on a window. **The cursor does not
/// advance**, deliberately: skipping would put a hole in the record that nothing
/// ever revisits, while stalling is visible in the log and costs only the time
/// it takes someone to look. A stall is recoverable and a gap is not.
const FOLLOW_RETRY_MIN: Duration = Duration::from_secs(5);
/// The ceiling on that backoff. Radar is a guest on a free public endpoint
/// (ADR 0002), so an endpoint that is already unhappy is not hammered.
const FOLLOW_RETRY_MAX: Duration = Duration::from_secs(300);

/// How wide a market-tape pass's window is, and how long it sleeps between
/// passes. The two are the same number on purpose, and it is
/// `market_tape::PASS_INTERVAL_SECONDS` rather than a second copy of it --
/// `market_tape::Budget::PER_PASS` is derived by dividing that interval into
/// the hourly quota, so a local copy that drifted would silently change the
/// budget's meaning without changing the budget.
///
/// The loop sleeps this long regardless of how long a pass itself took, so the
/// interval can only widen under load, never narrow below what the budget
/// assumes.
const MARKET_TAPE_PASS_INTERVAL: Duration =
    Duration::from_secs(market_tape::PASS_INTERVAL_SECONDS as u64);

/// How far behind wall-clock time a market-tape pass stays, so the window it
/// asks for has already had time to land in CryptoHouse.
const MARKET_TAPE_LAG_SECONDS: i64 = 120;

/// The narrowest window a market-tape pass's trades query will still try
/// before giving up rather than halving further.
const MARKET_TAPE_MIN_WINDOW_SECONDS: i64 = 4;

/// The smallest window follow mode will shrink to under repeated failure.
///
/// Shrinking matters more than waiting. A five-minute window during a migration
/// spam burst is several thousand rows, and `fetch_window` has to narrow it into
/// dozens of sub-queries that must *all* succeed together — one HTTP 500 near
/// the end discards the lot. Retrying the same five minutes just re-runs the
/// same long odds.
///
/// Halving the window instead turns all-or-nothing into incremental progress:
/// the cursor advances as far as it can get, and what is left is a smaller
/// problem than what failed. Measured on the 2026-08-24 bursts, a minute of the
/// worst of it is ~5,800 rows, which narrows into a handful of sub-queries
/// rather than dozens.
const FOLLOW_MIN_STEP_SECONDS: i64 = 15;

/// The window to try after one that size failed.
///
/// Halved, with a floor. Pulled out with its counterpart so the pair can be
/// tested: between them they decide whether a burst is crawled or stalled on.
const fn shrink_step(current: i64) -> i64 {
    let halved = current / 2;
    if halved < FOLLOW_MIN_STEP_SECONDS {
        FOLLOW_MIN_STEP_SECONDS
    } else {
        halved
    }
}

/// The window to try after one that size succeeded.
///
/// Doubled, never past `configured`. Doubling rather than jumping straight back:
/// one window that fits is not evidence that the size which just failed will.
const fn grow_step(current: i64, configured: i64) -> i64 {
    let doubled = current.saturating_mul(2);
    if doubled > configured {
        configured
    } else {
        doubled
    }
}

/// The wait after `failures` consecutive failed windows.
///
/// Doubling from [`FOLLOW_RETRY_MIN`], capped at [`FOLLOW_RETRY_MAX`]. Pulled
/// out of the loop so the schedule can be tested without running a daemon.
fn retry_backoff(failures: u32) -> Duration {
    let doubled = FOLLOW_RETRY_MIN
        .checked_mul(1u32.checked_shl(failures.min(16)).unwrap_or(u32::MAX))
        .unwrap_or(FOLLOW_RETRY_MAX);
    doubled.min(FOLLOW_RETRY_MAX)
}

#[allow(
    clippy::struct_excessive_bools,
    reason = "independent CLI flags, not a state machine -- --follow, --outcomes, --market-tape and --reprice each stand alone and folding them into enums would not make any of the modes below clearer"
)]
struct Args {
    from: String,
    to: String,
    store: String,
    window_minutes: i64,
    scope: Scope,
    follow: bool,
    outcomes: bool,
    /// Collect the venue-agnostic market tape `radar-serve`'s market routes
    /// read from, rather than querying CryptoHouse live. See
    /// `radar_backfill::market_tape`.
    market_tape: bool,
    /// Replace recorded price paths instead of folding onto them.
    reprice: bool,
    /// The analyst's directory, when the account runs on this host.
    ///
    /// Its reply log is the set of mints that earn a seven-day checkpoint.
    /// `None` measures every token to a day and no further, which is what a
    /// research backfill wants.
    analyst_dir: Option<String>,
}

fn usage() -> &'static str {
    "radar-backfill --from 'YYYY-MM-DD HH:MM:SS' --to 'YYYY-MM-DD HH:MM:SS' \
     --store <dir> [--window-minutes N] [--scope lifecycle|graduations|trades]
   radar-backfill --follow --store <dir> [--window-minutes N]

   radar-backfill --outcomes --store <dir> [--analyst-dir <dir>]
   radar-backfill --outcomes --reprice --store <dir>

   radar-backfill --market-tape --store <dir>

--analyst-dir names the account's directory, and its only effect is to give the
mints in `replies.jsonl` a fourth measurement at seven days. Every other token
settles at a day. Without it the daily post reports a day-old measurement, which
it now says out loud rather than calling it a week.

--outcomes measures what became of every token already in the store: how long it
kept trading, how many transfers, how many distinct accounts. Those are the
labels every signal has to be validated against, and they are the one extraction
the thousand-row cap does not obstruct, because an aggregate returns one row per
mint however much it scans.

--follow keeps recording from where the store left off, staying five minutes
behind the chain and sleeping when caught up. It uses the same extraction path
as a one-off backfill, so history and live data are one code path.

--market-tape collects the venue-agnostic trade tape radar-serve's market
routes read: two queries per two-minute pass (60/hour), so radar-serve never
queries CryptoHouse on a request path and the shared hourly quota is not
exhausted by traffic. Keeps its own cursor, separate from --follow's, so the
two can run against the same store without fighting over one file."
}

fn parse_args() -> Result<Args, String> {
    let mut from = None;
    let mut to = None;
    let mut store = None;
    let mut window_minutes = DEFAULT_WINDOW_MINUTES;
    let mut scope = Scope::default();
    let mut follow = false;
    let mut measure_outcomes = false;
    let mut market_tape = false;
    let mut reprice = false;
    let mut analyst_dir = None;

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
                    "graduations" => Scope::Graduations,
                    "trades" => Scope::Trades,
                    other => {
                        return Err(format!(
                            "unknown scope {other}; expected lifecycle, graduations or trades"
                        ));
                    }
                };
            }
            "--follow" => follow = true,
            "--outcomes" => measure_outcomes = true,
            "--market-tape" => market_tape = true,
            "--analyst-dir" => analyst_dir = Some(value()?),
            "--reprice" => reprice = true,
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

    // Follow mode, outcomes and the market tape have no explicit range: each
    // starts from its own cursor or window and runs (or measures) until
    // stopped, so requiring --from and --to would be asking for values none
    // of them are going to use.
    if runs_without_a_range(follow, measure_outcomes, market_tape) {
        return Ok(Args {
            from: String::new(),
            to: String::new(),
            store,
            window_minutes: window_minutes.max(1),
            scope,
            follow,
            outcomes: measure_outcomes,
            market_tape,
            reprice,
            analyst_dir,
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
        market_tape,
        reprice,
        analyst_dir,
    })
}

fn merge(into: &mut Stats, from: &Stats) {
    into.emitted += from.emitted;
    for (k, v) in &from.skipped {
        *into.skipped.entry(*k).or_default() += v;
    }
}

/// Fetches one window, halving it on a server timeout.
///
/// A thin wrapper over [`radar_backfill::fetch_windowed`], which is generic so
/// the same halving discipline is available to other callers — see its own
/// doc comment for why this used to be a private `fn` here.
fn fetch_window(
    client: &Client,
    from: i64,
    to: i64,
    depth: u32,
    scope: Scope,
) -> Result<Vec<Row>, QueryError> {
    radar_backfill::fetch_windowed(
        client,
        from,
        to,
        depth,
        MIN_WINDOW_SECONDS,
        PAUSE_BETWEEN_WINDOWS,
        &|from, to| query_for_window(from, to, scope),
    )
}

/// Writes one coverage record per table the scope collected into.
///
/// Both paths go through here so that neither can quietly stop recording a
/// window: what a reader must be able to tell apart is *ran and found nothing*
/// from *never ran*, and that only holds if every window that ends leaves a
/// record — including the empty ones, which are the whole point.
fn write_coverage(
    writer: &mut Writer,
    scope: Scope,
    status: Completion,
    events: &[Event],
    recorded_at: radar_types::Slot,
    source: &str,
) -> Result<(), String> {
    for record in coverage::records_for_window(scope, status, events, recorded_at, source) {
        writer.append_coverage(record).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// The windows a run walks, in order, tiling `start..end` exactly.
///
/// This is one line of arithmetic pulled out of `run`, and it is pulled out
/// because of what the rest of this file now does. The backfill writes down
/// which ranges it covered. An error in this stepping would therefore not lose
/// data quietly — it would file a confident coverage record for a range nobody
/// read, which is the single failure the coverage table exists to make
/// impossible. `run` fetches over the network, so its loop cannot be driven
/// from a test. This can.
///
/// A `step` of zero or less would not terminate, so it is clamped. `Args`
/// already clamps `window_minutes` with `.max(1)`, but the clamp belongs where
/// the loop is, not only where the argument is parsed.
/// Built on `step_by` rather than a cursor that advances itself, so it
/// terminates whatever the arithmetic below it does. That is deliberate: a
/// cursor loop mutated from `+` to `-` or `*` does not return a wrong answer,
/// it spins forever, and a test that hangs is a weaker signal than one that
/// fails. This shape turns both of those into a wrong list a test can name.
fn windows(start: i64, end: i64, step: i64) -> Vec<(i64, i64)> {
    let step = step.max(1);
    // `step_by` wants a `usize`. The clamp above makes the value positive, so
    // the only way this conversion fails is a step larger than a 32-bit pointer
    // can hold — which would be a single window spanning the whole range, and
    // `usize::MAX` gives exactly that rather than a truncated stride that would
    // silently collect the range several times over.
    let stride = usize::try_from(step).unwrap_or(usize::MAX);
    (start..end)
        .step_by(stride)
        .map(|from| (from, (from + step).min(end)))
        .collect()
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

    // The highest slot anything in the run has seen, for stamping coverage
    // records with. Read once from the store rather than per window: it only
    // moves forward, and every window that finds a row moves it here.
    let mut store_high = Reader::open(&args.store)
        .watermark()
        .map_err(|e| e.to_string())?;

    for (cursor, window_end) in windows(start, end, step) {
        let rows = match fetch_window(&client, cursor, window_end, 0, args.scope) {
            Ok(rows) => rows,
            Err(e) => {
                // The range was attempted and did not finish, and that is a
                // fact about it. Recorded before the error propagates, so the
                // window a stopped run died on reads as an unfinished attempt
                // rather than as a range nobody ever asked for.
                write_coverage(
                    &mut writer,
                    args.scope,
                    Completion::Partial,
                    &[],
                    coverage::established_at(None, store_high),
                    coverage::SOURCE_BACKFILL,
                )?;
                writer.flush().map_err(|e| e.to_string())?;
                return Err(e.to_string());
            }
        };
        rows_seen += rows.len() as u64;

        let (events, stats) = events_from_rows(&rows);
        let emitted = events.len();
        let recorded_at = coverage::established_at(coverage::highest_slot(&events), store_high);
        store_high = Some(recorded_at);
        write_coverage(
            &mut writer,
            args.scope,
            Completion::Complete,
            &events,
            recorded_at,
            coverage::SOURCE_BACKFILL,
        )?;
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

/// Keeps recording from where the store left off.
fn follow(args: &Args) -> Result<(), String> {
    let client = Client::default();
    let step = args.window_minutes * 60;

    // A fresh store starts one window back rather than at the epoch: the
    // alternative is silently attempting to backfill fifty-six years.
    let mut cursor = radar_store::read_cursor(std::path::Path::new(&args.store))
        .unwrap_or_else(|| now_epoch() - FOLLOW_LAG_SECONDS - step);

    println!(
        "following from {} in {}-minute windows",
        from_epoch(cursor),
        args.window_minutes
    );
    println!("staying {FOLLOW_LAG_SECONDS}s behind the chain; ctrl-c to stop");

    let mut totals = Stats::default();
    // The highest slot the recorder has seen, for stamping coverage records
    // with. Read from the store once at startup and carried forward: a window
    // that finds no row has no slot of its own, and the store's own top is the
    // latest moment anything here can honestly point at.
    let mut store_high = Reader::open(&args.store)
        .watermark()
        .map_err(|e| e.to_string())?;
    // Consecutive failed windows, for the backoff. Reset by any success, so a
    // transient upstream wobble does not leave the recorder crawling for hours.
    let mut failures: u32 = 0;
    // The window size actually in use. Shrinks on failure and recovers on
    // success, so a burst is crawled rather than repeatedly re-attempted whole.
    let mut current_step = step;
    loop {
        let horizon = now_epoch() - FOLLOW_LAG_SECONDS;
        if horizon - cursor < FOLLOW_MIN_WINDOW_SECONDS {
            std::thread::sleep(FOLLOW_IDLE);
            continue;
        }
        let window_end = (cursor + current_step).min(horizon);

        let rows = match fetch_window(&client, cursor, window_end, 0, args.scope) {
            Ok(rows) => {
                failures = 0;
                // Recover by doubling rather than jumping straight back to the
                // configured step: one lucky window is not evidence that the
                // size which just failed will now work.
                current_step = grow_step(current_step, step);
                rows
            }
            Err(e) => {
                let wait = retry_backoff(failures);
                failures = failures.saturating_add(1);
                eprintln!(
                    "  {} .. {} failed ({failures}): {e}",
                    from_epoch(cursor),
                    from_epoch(window_end)
                );
                let shrunk = shrink_step(current_step);
                eprintln!(
                    "  window {current_step}s -> {shrunk}s, retrying in {}s. The cursor stays \
                     at {} so nothing is skipped — a stall is recoverable and a gap in the \
                     record is not.",
                    wait.as_secs(),
                    from_epoch(cursor)
                );
                current_step = shrunk;
                // The attempt happened and did not finish. Written down as
                // `Partial`, which covers nothing: without the row, an outage
                // is a stretch of store with no coverage records in it, which
                // is exactly what a stretch nobody has reached yet looks like.
                let mut writer = Writer::open(&args.store, 20_000).map_err(|e| e.to_string())?;
                write_coverage(
                    &mut writer,
                    args.scope,
                    Completion::Partial,
                    &[],
                    coverage::established_at(None, store_high),
                    coverage::SOURCE_FOLLOW,
                )?;
                writer.flush().map_err(|e| e.to_string())?;
                std::thread::sleep(wait);
                continue;
            }
        };
        let (events, stats) = events_from_rows(&rows);
        let emitted = events.len();
        let recorded_at = coverage::established_at(coverage::highest_slot(&events), store_high);
        store_high = Some(recorded_at);

        // Open, write and flush per window. Holding a writer across the whole
        // loop would buffer events indefinitely on a quiet market, and a process
        // killed mid-buffer would lose them.
        {
            let mut writer = Writer::open(&args.store, 20_000).map_err(|e| e.to_string())?;
            // Buffered here, written last: `Writer::flush` puts the coverage
            // file down after the event files, so a crash mid-flush loses the
            // record and keeps the rows rather than the other way round.
            write_coverage(
                &mut writer,
                args.scope,
                Completion::Complete,
                &events,
                recorded_at,
                coverage::SOURCE_FOLLOW,
            )?;
            for e in events {
                writer.append(e).map_err(|e| e.to_string())?;
            }
            writer.flush().map_err(|e| e.to_string())?;
        }
        radar_store::write_cursor(std::path::Path::new(&args.store), window_end)?;
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

/// Where the market tape keeps its own cursor.
///
/// A subdirectory under the store, not the store root: the follow cursor
/// (`radar_store::CURSOR_FILE`) lives at the root, and two writers of one file
/// fight over it. This collector's cursor lives beside it in its own
/// directory instead, using the same atomic read/write.
fn market_tape_scope(store: &str) -> std::path::PathBuf {
    std::path::Path::new(store).join(market_tape::SCOPE_DIR)
}

/// [`MARKET_TAPE_PASS_INTERVAL`] in seconds, as the `i64` the rest of this
/// file's time arithmetic uses. `try_from` rather than `as`: a `Duration`
/// this large would be a caller bug, not a value to wrap silently into a
/// negative interval.
fn market_tape_pass_seconds() -> i64 {
    i64::try_from(MARKET_TAPE_PASS_INTERVAL.as_secs()).unwrap_or(i64::MAX)
}

/// The end of the next market-tape window, given where the cursor sits, how
/// wide a pass is, and how far the collector may reach before wall-clock lag.
///
/// Pulled out of the loop for the same reason [`windows`] is: the decision is
/// arithmetic a fake transport cannot exercise, since it runs *before* either
/// query, but a test can drive it directly with no network at all. A pass
/// never reaches past `horizon` even if `pass_seconds` would carry it there,
/// which is what keeps the collector from asking CryptoHouse for a window
/// that has not landed yet.
/// Whether a mode runs from its own cursor rather than an explicit range.
///
/// `--follow`, `--outcomes` and `--market-tape` each start from somewhere they
/// work out for themselves and run until stopped, so requiring `--from` and
/// `--to` would demand values none of them reads. Named so the disjunction can
/// be tested: with an `&&` in place of either `||`, a run of one of these modes
/// alone falls through to the range parser and is refused for missing arguments
/// it was never going to use.
const fn runs_without_a_range(follow: bool, outcomes: bool, market_tape: bool) -> bool {
    follow || outcomes || market_tape
}

/// Whether the collector is between passes, with no window yet to ask about.
///
/// True when the horizon has not moved past the cursor, which is the ordinary
/// state while waiting -- the caller sleeps rather than asking CryptoHouse
/// about a window of zero or negative length. Inverted, the collector sleeps
/// exactly when there *is* work and queries exactly when there is none, which
/// spends quota on empty windows while the cursor never advances.
///
/// **Named for the true case on purpose.** Phrased the other way round the
/// call site needs a `!`, and a leading `!` is one character a mutation can
/// delete in a daemon loop no test reaches. Stated positively there is nothing
/// at the call site to mutate, and the comparison itself is right here where a
/// test can hold it.
const fn is_between_passes(window_end: i64, cursor: i64) -> bool {
    window_end <= cursor
}

/// How far a market-tape pass may reach: wall-clock, less the lag that lets a
/// window land in CryptoHouse first.
///
/// Named rather than inlined so the subtraction can be tested. Added as `+`
/// instead it reaches into the future, and the collector asks for a window
/// that has not happened -- which returns nothing and is recorded as a quiet
/// market.
const fn market_tape_horizon(now: i64, lag_seconds: i64) -> i64 {
    now - lag_seconds
}

/// Where a fresh store's cursor starts: one full pass behind the horizon, so
/// the first pass has a complete window to ask about rather than an empty
/// sliver.
const fn market_tape_first_cursor(now: i64, lag_seconds: i64, pass_seconds: i64) -> i64 {
    market_tape_horizon(now, lag_seconds) - pass_seconds
}

const fn market_tape_window_end(cursor: i64, pass_seconds: i64, horizon: i64) -> i64 {
    let wanted = cursor + pass_seconds;
    if wanted < horizon { wanted } else { horizon }
}

/// Collects the venue-agnostic market tape: two CryptoHouse queries per pass,
/// one pass every [`MARKET_TAPE_PASS_INTERVAL`]. See
/// `radar_backfill::market_tape`'s module doc for the arithmetic and the
/// coverage caveat.
/// Records that a market-tape pass ran and collected nothing it can attest to.
///
/// **Not the same as recording nothing.** A pass that failed and wrote no
/// coverage leaves a slot range indistinguishable from a quiet market, which is
/// the failure this project is most organised against. A `Partial` record with
/// no slots says the window was visited and did not complete, which a reader
/// can act on.
///
/// Extracted because both failure arms of `market_tape` need it and wrote it
/// out identically -- and because a third copy is exactly how one of them
/// eventually gets forgotten.
fn record_partial_pass(store: &str, store_high: Option<radar_types::Slot>) -> Result<(), String> {
    let mut writer = Writer::open(store, 20_000).map_err(|e| e.to_string())?;
    writer
        .append_coverage(market_tape::coverage_record(
            Completion::Partial,
            &[],
            coverage::established_at(None, store_high),
        ))
        .map_err(|e| e.to_string())?;
    writer.flush().map_err(|e| e.to_string())
}

fn market_tape(args: &Args) -> Result<(), String> {
    let client = Client::default();
    let scope = market_tape_scope(&args.store);
    let pass_seconds = market_tape_pass_seconds();
    let mut cursor = radar_store::read_cursor(&scope).unwrap_or_else(|| {
        market_tape_first_cursor(now_epoch(), MARKET_TAPE_LAG_SECONDS, pass_seconds)
    });

    println!(
        "market tape: {pass_seconds}s per pass, at most {} queries per pass ({}/hour ceiling),          top {} mints, from {} into {}",
        market_tape::Budget::PER_PASS,
        u64::from(market_tape::Budget::PER_PASS) * 3_600 / MARKET_TAPE_PASS_INTERVAL.as_secs(),
        market_tape::SHORTLIST,
        from_epoch(cursor),
        args.store
    );
    println!("ctrl-c to stop");

    let mut store_high = Reader::open(&args.store)
        .watermark()
        .map_err(|e| e.to_string())?;

    loop {
        let horizon = market_tape_horizon(now_epoch(), MARKET_TAPE_LAG_SECONDS);
        let window_end = market_tape_window_end(cursor, pass_seconds, horizon);
        if is_between_passes(window_end, cursor) {
            std::thread::sleep(MARKET_TAPE_PASS_INTERVAL);
            continue;
        }
        let (from_s, to_s) = (from_epoch(cursor), from_epoch(window_end));

        // One budget per pass. Every query below goes through it, including
        // the ones `narrowing_fetch` generates by halving -- which is the
        // whole point, since those are the ones nothing was counting.
        let budget = market_tape::Budget::new(market_tape::Budget::PER_PASS);

        // The window's active mints.
        let candidates: Vec<market_fold::CoinCandidateRow> = match budget.guard(
            |sql| client.query(sql),
            &market_query::coin_candidates_query(&from_s, &to_s, market_tape::SHORTLIST),
        ) {
            Ok(rows) => rows,
            Err(e) => {
                eprintln!("  {from_s} .. {to_s} candidates query failed: {e}");
                record_partial_pass(&args.store, store_high)?;
                std::thread::sleep(MARKET_TAPE_PASS_INTERVAL);
                continue;
            }
        };

        // Query 2 of 2: those mints' trades, batched into one `IN (...)`.
        // Empty candidates skip the query rather than asking for an empty
        // `IN ()`, which is invalid SQL and would waste the round trip.
        let trades = if candidates.is_empty() {
            Vec::new()
        } else {
            let mints: Vec<String> = candidates.iter().map(|c| c.mint.clone()).collect();
            match radar_backfill::narrowing_fetch(
                &|sql| budget.guard(|s| client.query(s), sql),
                cursor,
                window_end,
                0,
                MARKET_TAPE_MIN_WINDOW_SECONDS,
                PAUSE_BETWEEN_WINDOWS,
                &|f, t| market_query::trades_query(&mints, f, t, market_tape::TRADES_PER_MINT),
            ) {
                Ok(rows) => market_tape::fold_market_trades(&rows),
                Err(e) => {
                    // A spent budget is not a fault and reads differently in
                    // the log, because an operator acts on it differently:
                    // one means the endpoint is unwell, the other means this
                    // window was busier than the quota can cover.
                    eprintln!(
                        "  {from_s} .. {to_s} trades incomplete ({} queries left): {e}",
                        budget.remaining()
                    );
                    record_partial_pass(&args.store, store_high)?;
                    std::thread::sleep(MARKET_TAPE_PASS_INTERVAL);
                    continue;
                }
            }
        };

        let recorded_at = coverage::established_at(market_tape::highest_slot(&trades), store_high);
        store_high = Some(recorded_at);

        {
            let mut writer = Writer::open(&args.store, 20_000).map_err(|e| e.to_string())?;
            let trade_count = trades.len();
            // The coverage record is built from `trades` before they are
            // moved into the writer, and buffered before the rows -- but
            // `Writer::flush` still puts the coverage file down *after* the
            // event files, so a crash mid-flush loses the record and keeps
            // the rows rather than the other way round.
            let coverage = market_tape::coverage_record(Completion::Complete, &trades, recorded_at);
            for trade in trades {
                writer
                    .append_market_trade(trade)
                    .map_err(|e| e.to_string())?;
            }
            writer
                .append_coverage(coverage)
                .map_err(|e| e.to_string())?;
            writer.flush().map_err(|e| e.to_string())?;
            println!(
                "  {from_s} .. {to_s}  candidates {:>4}  trades {:>5}",
                candidates.len(),
                trade_count
            );
        }
        radar_store::write_cursor(&scope, window_end)?;

        cursor = window_end;
        std::thread::sleep(MARKET_TAPE_PASS_INTERVAL);
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

/// When each mint graduated, from graduation events the chain **accepted**.
///
/// Split out of [`universe`] so the two rules in it can be tested without a
/// store on disk. Both have cost something already:
///
/// - **A failed `migrate` moved nothing.** It is worth recording — a migration
///   attempted and reverted is real information — but it is not a graduation,
///   and counting it as one inflated the rarest and most load-bearing label in
///   the store by about a third. An **unresolved** outcome is excluded for the
///   other reason: nobody established that it happened.
/// - **Earliest wins.** A token graduates once, and a partition written twice
///   must not turn one event into a later-looking second one.
fn earliest_graduations(events: &[Event]) -> BTreeMap<radar_types::Address, radar_types::Slot> {
    let mut graduated: BTreeMap<radar_types::Address, radar_types::Slot> = BTreeMap::new();
    for event in events {
        if !event.envelope().succeeded() {
            continue;
        }
        let slot = event.envelope().slot;
        graduated
            .entry(event.mint())
            .and_modify(|at| *at = (*at).min(slot))
            .or_insert(slot);
    }
    graduated
}

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

    let graduated = earliest_graduations(
        &reader
            .read(Table::Graduations, as_of)
            .map_err(|e| e.to_string())?,
    );

    Ok((launches, graduated))
}

/// Which tokens have crossed a checkpoint their last measurement predates.
///
/// Measuring everything on every pass would re-measure a month of history a
/// million times over, almost all of it long settled. Measuring once would be
/// worse: a token seen an hour after launch and the same token seen a day later
/// are different observations, and the second is the one that says whether the
/// first meant anything.
/// The mints this account has answered about in public.
///
/// Empty when no analyst directory was given, which is the ordinary case for a
/// research backfill and is not an error: with no watched set every token
/// settles at a day, exactly as before. Rule 8's shape — the absent
/// configuration is the cheaper behaviour, not the more expensive one.
///
/// A missing or unreadable log is also empty. The seven-day checkpoint is an
/// improvement to one post; it is not worth failing an outcomes pass over.
fn answered_about(analyst_dir: Option<&str>) -> std::collections::BTreeSet<radar_types::Address> {
    let Some(dir) = analyst_dir else {
        return std::collections::BTreeSet::new();
    };
    let paths = radar_analyst::daemon::Paths::under(dir);
    radar_analyst::log::read(&paths.log)
        .unwrap_or_default()
        .iter()
        // Published only. A dry-run answer was never a public call and has
        // nothing to age, which is the same filter the post itself applies.
        .filter(|e| e.reply_id.is_some())
        .filter_map(|e| e.mint.as_ref()?.parse().ok())
        .collect()
}

/// `watched` is the set of mints the account has answered about in public.
/// Those get one more checkpoint, at a week, because the daily post is a claim
/// about a week and the store's last look at them is a day old. Everything else
/// settles at a day, for the reason `checkpoints` gives.
fn due_for_measurement(
    launches: &[(radar_types::Address, radar_types::Slot)],
    already: &[radar_store::Outcome],
    head: radar_types::Slot,
    watched: &std::collections::BTreeSet<radar_types::Address>,
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
                watched.contains(mint),
            )
        })
        .collect()
}

/// Measures what became of every token already in the store.
/// The window a price query may cover, as `(from, to)`.
///
/// Bounded at both ends, where the transfer window deliberately is not. Prices
/// come from a join against `solana.transactions`, and the cost is driven by how
/// many transactions match rather than by how long the window is -- so a full day
/// is affordable and an unbounded range is not.
///
/// `(None, None)` when the chain head's timestamp cannot be resolved. The
/// outcomes still get written; their prices simply stay absent, because an
/// unwritten outcome is a hole in the record and an unpriced one is not.
fn price_window(
    client: &Client,
    measured_at: radar_types::Slot,
    since: &str,
) -> (Option<String>, Option<String>) {
    let Ok(now) = client.query::<TimeRow>(&outcomes::query_for_slot_time(measured_at)) else {
        return (None, None);
    };
    let to = now
        .first()
        .map(|t| t.at.clone())
        .filter(|t| !t.is_empty() && !t.starts_with("1970"));
    let from = to.as_ref().map(|to| prices::window_start(to, since));
    (from, to)
}

/// Prices for one batch, or an empty map if they could not be fetched.
///
/// A pricing failure must not discard the outcome. An outcome with no price is a
/// measurement with a gap; an outcome that was never written because pricing
/// failed is a hole in the record, and the second is worse -- a missing slot
/// range in this store is indistinguishable from a quiet market, which is the
/// failure the whole project is organised against.
fn price_batch(
    client: &Client,
    mints: &[String],
    from: Option<&str>,
    to: Option<&str>,
) -> std::collections::BTreeMap<String, prices::Prices> {
    let (Some(from), Some(to)) = (from, to) else {
        return std::collections::BTreeMap::new();
    };
    match client.query::<PriceRow>(&prices::query_for_mints(mints, from, to)) {
        Ok(rows) => prices::to_prices(&rows),
        Err(e) => {
            eprintln!("  prices unavailable for this batch, recording without: {e}");
            std::collections::BTreeMap::new()
        }
    }
}

/// The recorded price path that a new measurement is folded onto.
///
/// Normally this is what each mint was last priced at, so a six-hour window can
/// extend what is known about a token older than six hours instead of replacing
/// it.
///
/// `--reprice` returns nothing, and it exists because **the fold is a ratchet**.
/// `peak` folds with `max` and `trough` with `min`, so a wrong extreme can never
/// be lowered by a later, better measurement. The price query admitted dust
/// transactions until LEARNINGS 14, and every mint priced before that fix
/// carries an extreme that no correct pass can undo. Repricing replaces the path
/// instead of extending it.
///
/// It reaches a token only while that token is still due for a checkpoint, so it
/// repairs what is still in flight rather than the whole store. Saying which is
/// the point: a repair that looked complete and was not would be worse than
/// none.
fn baseline_prices(
    already: &[radar_store::Outcome],
    reprice: bool,
) -> BTreeMap<String, prices::Prices> {
    if reprice {
        println!(
            "--reprice: replacing recorded price paths rather than folding onto them.
               Reaches only tokens still due for a checkpoint; anything already
               settled keeps whatever it was last measured with."
        );
        return BTreeMap::new();
    }
    prices::prior_prices(already)
}

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

    let (price_from, price_to) = price_window(&client, measured_at, &since);
    let prior = baseline_prices(&already, args.reprice);

    let total_known = launches.len();
    let watched = answered_about(args.analyst_dir.as_deref());
    let due = due_for_measurement(&launches, &already, measured_at, &watched);

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
    let (mut priced_batches, mut with_price) = (0u64, 0u64);

    for batch in launches.chunks(outcomes::MINTS_PER_BATCH) {
        let mints: Vec<String> = batch.iter().map(|(m, _)| m.to_string()).collect();
        let rows: Vec<AggregateRow> = client
            .query(&outcomes::query_for_mints(&mints, &since))
            .map_err(|e| e.to_string())?;

        let priced = price_batch(&client, &mints, price_from.as_deref(), price_to.as_deref());
        priced_batches += u64::from(!priced.is_empty());

        let measured = outcomes::outcomes_from_rows(&rows, batch, measured_at, &graduated);
        let measured = prices::apply(measured, &priced, &prior);
        for outcome in measured {
            if outcome.appears_stillborn() {
                stillborn += 1;
            }
            if outcome.transfers > 0 {
                with_activity += 1;
            }
            if outcome.first_price.is_some() {
                with_price += 1;
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
    println!("  with a measured price : {with_price} (from {priced_batches} priced batch(es))");
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
    } else if args.market_tape {
        market_tape(&args)
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

    /// Any one of the cursor-driven modes runs without a range.
    ///
    /// Kills both `||` mutants. With an `&&`, running `--market-tape` alone
    /// falls through to the range parser and is refused for missing `--from`
    /// and `--to` that it would never have read -- so the mode simply stops
    /// working, with an error blaming the operator.
    #[test]
    fn each_cursor_driven_mode_runs_without_a_range_on_its_own() {
        assert!(runs_without_a_range(true, false, false), "--follow alone");
        assert!(runs_without_a_range(false, true, false), "--outcomes alone");
        assert!(
            runs_without_a_range(false, false, true),
            "--market-tape alone"
        );
        assert!(
            !runs_without_a_range(false, false, false),
            "a plain backfill does need --from and --to"
        );
    }

    /// A window with no width is not asked about.
    ///
    /// Kills the mutant inverting the comparison, which would make the
    /// collector sleep exactly when there is work and query exactly when there
    /// is none -- spending quota on empty windows while the cursor never
    /// advances.
    #[test]
    fn a_window_of_no_width_is_slept_through_rather_than_asked_about() {
        assert!(
            !is_between_passes(1_100, 1_000),
            "a real window is work, not waiting"
        );
        assert!(
            is_between_passes(1_000, 1_000),
            "zero width is nothing to ask about"
        );
        assert!(
            is_between_passes(900, 1_000),
            "a horizon behind the cursor is not a window at all"
        );
    }

    /// The horizon is behind the clock, never ahead of it.
    ///
    /// Kills the mutants turning the subtraction into an addition or a
    /// division. Either reaches into the future, where the collector asks for
    /// a window that has not happened, gets nothing, and records a quiet
    /// market.
    #[test]
    fn the_horizon_is_behind_the_clock_by_exactly_the_lag() {
        assert_eq!(market_tape_horizon(1_000_000, 120), 999_880);
        assert!(
            market_tape_horizon(1_000_000, 120) < 1_000_000,
            "a horizon at or ahead of now asks about a window still filling"
        );
    }

    /// A fresh store starts one whole pass behind the horizon.
    ///
    /// Kills the same two mutants on the other subtraction. A first cursor at
    /// the horizon leaves a zero-width first window; one ahead of it makes
    /// `market_tape_window_end` return something at or behind the cursor and
    /// the collector sleeps instead of starting.
    #[test]
    fn a_fresh_store_starts_one_pass_behind_the_horizon() {
        let now = 1_000_000;
        let first = market_tape_first_cursor(now, 120, 300);
        assert_eq!(first, 999_580);
        assert_eq!(
            market_tape_horizon(now, 120) - first,
            300,
            "exactly one pass of room, so the first window is a full one"
        );
        assert!(
            market_tape_window_end(first, 300, market_tape_horizon(now, 120)) > first,
            "the first window must have width, or the loop never starts"
        );
    }

    /// A pass never reaches past the horizon, and stops exactly at it.
    ///
    /// The horizon is where CryptoHouse has actually landed; asking past it
    /// returns a window still filling, which the collector would then record
    /// as complete. Kills the mutant relaxing `wanted < horizon` to `<=`,
    /// which changes which of two equal values is returned -- harmless here,
    /// and the reason the boundary is asserted from both sides rather than
    /// only the clamped one.
    #[test]
    fn a_pass_stops_at_the_horizon_rather_than_reaching_past_it() {
        // Room to spare: the full pass width.
        assert_eq!(market_tape_window_end(1_000, 300, 10_000), 1_300);
        // Not enough room: clamped to the horizon, not to cursor + width.
        assert_eq!(market_tape_window_end(1_000, 300, 1_100), 1_100);
        // Exactly at it: the same answer either way, which is why this line
        // exists -- so a reader knows the equality case was considered.
        assert_eq!(market_tape_window_end(1_000, 300, 1_300), 1_300);
        // Already past it: the caller sees a window end at or behind the
        // cursor and sleeps rather than asking for a backwards range.
        assert!(market_tape_window_end(1_000, 300, 900) <= 1_000);
    }

    /// The pass width is the interval, as seconds, and it is positive.
    ///
    /// Kills the three mutants that replace this with -1, 0 or 1. Each is
    /// quiet and each is ruinous: a zero or negative width makes every window
    /// end at or behind its cursor, so the collector sleeps forever and the
    /// store stays empty while the service reports itself active.
    #[test]
    fn the_pass_width_is_the_configured_interval_in_seconds() {
        let seconds = market_tape_pass_seconds();
        assert_eq!(
            seconds,
            radar_backfill::market_tape::PASS_INTERVAL_SECONDS,
            "the loop's width and the budget's divisor must be one number"
        );
        assert!(
            seconds > 1,
            "a width of zero or one collects nothing, forever"
        );
        assert_eq!(
            market_tape_window_end(0, seconds, i64::MAX),
            seconds,
            "a pass from zero with room to spare is exactly one interval wide"
        );
    }
    #[test]
    fn the_usage_text_names_every_flag_the_parser_accepts() {
        // Not style. `--reprice` was added to the parser and very nearly shipped
        // undocumented, and a flag that exists but is not in `--help` is a flag
        // nobody finds. Mutation testing caught that nothing at all constrained
        // this string: blanking `usage()` entirely left the suite green.
        let u = usage();
        for flag in [
            "--from",
            "--to",
            "--store",
            "--window-minutes",
            "--scope",
            "--follow",
            "--outcomes",
            "--market-tape",
            "--reprice",
        ] {
            assert!(u.contains(flag), "usage() does not mention {flag}:\n{u}");
        }
    }

    #[test]
    fn repricing_drops_the_baseline_so_a_bad_extreme_can_be_undone() {
        // The fold is a ratchet: `peak` combines with `max`, so a contaminated
        // extreme survives every later correct measurement. That is how a dust
        // transaction admitted before LEARNINGS 14 became permanent, and why
        // fixing the query repairs nothing that is already recorded.
        let contaminated = radar_store::Outcome {
            mint: radar_types::Address::new([3u8; 32]),
            measured_at: radar_types::Slot(9_000),
            launch_slot: radar_types::Slot(0),
            first_transfer_slot: None,
            last_transfer_slot: None,
            transfers: 12,
            unique_senders: 4,
            unique_receivers: 5,
            graduated_at: None,
            first_price: Some(1_000),
            last_price: Some(900),
            // Eight million times the first price -- the shape a one-base-unit
            // transfer produces when the SOL beside it is unrelated.
            peak_price: Some(8_000_000_000),
            trough_price: Some(1),
            window_peak_price: None,
            window_trough_price: None,
            vwap: Some(950),
            fills: 40,
        };
        let already = vec![contaminated];
        let key = radar_types::Address::new([3u8; 32]).to_string();

        let folded = baseline_prices(&already, false);
        assert_eq!(
            folded.get(&key).and_then(|p| p.peak),
            Some(8_000_000_000),
            "the ordinary path carries the recorded extreme forward, which is \
             correct for a real path and fatal for a wrong one"
        );

        let replaced = baseline_prices(&already, true);
        assert!(
            replaced.is_empty(),
            "repricing must hand `apply` nothing to fold onto, or `max` keeps \
             the bad extreme however correct the new measurement is"
        );
    }

    use super::*;

    #[test]
    fn the_retry_backoff_climbs_and_then_stops_climbing() {
        // The recorder must keep trying a failed window forever, and must not
        // hammer a public endpoint while doing it. Both halves are load-bearing:
        // without the first it exits and the store grows a hole, without the
        // second it turns an upstream wobble into an outage of its own.
        assert_eq!(retry_backoff(0), FOLLOW_RETRY_MIN);
        assert_eq!(retry_backoff(1), FOLLOW_RETRY_MIN * 2);
        assert_eq!(retry_backoff(2), FOLLOW_RETRY_MIN * 4);
        assert_eq!(retry_backoff(6), FOLLOW_RETRY_MAX, "reaches the ceiling");

        // And stays there, including at values that would overflow a naive shift.
        for failures in [7u32, 20, 100, u32::MAX] {
            assert_eq!(
                retry_backoff(failures),
                FOLLOW_RETRY_MAX,
                "backoff must saturate rather than wrap at {failures} failures"
            );
        }
    }

    #[test]
    fn the_window_shrinks_under_failure_and_recovers_after_it() {
        // Shrinking is what turns a burst from a stall into a crawl. A five
        // minute window during the 2026-08-24 spam bursts was several thousand
        // rows and needed dozens of narrowed sub-queries to all succeed at once;
        // retrying that whole window just re-runs the same long odds.
        let configured = 300;
        assert_eq!(shrink_step(configured), 150);
        assert_eq!(shrink_step(150), 75);

        // Recovery is gradual, and never past what the operator asked for.
        assert_eq!(grow_step(75, configured), 150);
        assert_eq!(grow_step(150, configured), configured);
        assert_eq!(grow_step(configured, configured), configured);
    }

    #[test]
    fn the_window_never_shrinks_to_nothing_or_grows_without_bound() {
        // A window of zero seconds would spin against a public endpoint forever
        // without ever advancing the cursor, which is a worse stall than the one
        // this replaced -- it would look like progress in the log.
        let mut step = 300;
        for _ in 0..40 {
            step = shrink_step(step);
            assert!(step >= FOLLOW_MIN_STEP_SECONDS, "shrank to {step}");
        }
        assert_eq!(step, FOLLOW_MIN_STEP_SECONDS);

        // And growth is bounded by the configured size however long it runs well.
        let mut step = FOLLOW_MIN_STEP_SECONDS;
        for _ in 0..40 {
            step = grow_step(step, 300);
            assert!(step <= 300, "grew to {step}");
        }
        assert_eq!(step, 300);

        // Saturating rather than wrapping, so a preposterous --window-minutes
        // cannot turn into a negative window.
        assert_eq!(grow_step(i64::MAX, 300), 300);
    }

    #[test]
    fn the_backoff_never_reaches_zero() {
        // A zero wait would turn a persistently failing window into a hot loop
        // against a free public endpoint, which is how a guest stops being one.
        for failures in 0..64u32 {
            assert!(
                retry_backoff(failures) >= FOLLOW_RETRY_MIN,
                "backoff collapsed at {failures} failures"
            );
        }
    }

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

    /// A graduation event for `mint` at `slot`, with the outcome given.
    fn graduation(mint: u8, slot: u64, success: Option<bool>) -> Event {
        Event::Graduation(Box::new(radar_store::Graduation {
            envelope: radar_store::Envelope {
                slot: radar_types::Slot(slot),
                signature: radar_types::Signature::new([mint; 64]),
                tx_index: Some(1),
                instruction_index: 0,
                parent_index: None,
                success,
            },
            origin: radar_store::Origin::known(radar_types::Address::new([5u8; 32]), "migrate_v2"),
            mint: radar_types::Address::new([mint; 32]),
        }))
    }

    #[test]
    fn only_a_migration_the_chain_accepted_is_a_graduation() {
        // Re-apply by deleting the `!`: the reverted and the unresolved
        // migrations both become graduations, and the rarest label in the store
        // gains two mints that never left their curve. Counting reverted
        // migrations inflated it by about a third when it last happened.
        let graduated = earliest_graduations(&[
            graduation(1, 100, Some(true)),
            graduation(2, 200, Some(false)),
            graduation(3, 300, None),
        ]);

        assert_eq!(graduated.len(), 1, "one accepted migration: {graduated:?}");
        assert_eq!(
            graduated.get(&radar_types::Address::new([1u8; 32])),
            Some(&radar_types::Slot(100))
        );
        assert!(!graduated.contains_key(&radar_types::Address::new([2u8; 32])));
        assert!(
            !graduated.contains_key(&radar_types::Address::new([3u8; 32])),
            "an unresolved migration is not one that happened"
        );
    }

    #[test]
    fn a_mint_graduates_at_the_earliest_slot_recorded_for_it() {
        // A partition written twice must not turn one event into a
        // later-looking second one. Order is deliberately reversed, so a
        // mutation to `max` -- or to insert-first-wins -- changes the answer.
        let graduated = earliest_graduations(&[
            graduation(1, 900, Some(true)),
            graduation(1, 100, Some(true)),
        ]);

        assert_eq!(
            graduated.get(&radar_types::Address::new([1u8; 32])),
            Some(&radar_types::Slot(100))
        );
    }

    #[test]
    fn windows_tile_the_range_exactly() {
        let walked = windows(0, 250, 100);
        assert_eq!(walked, vec![(0, 100), (100, 200), (200, 250)]);

        // Each window begins where the last ended, and the whole walk spans the
        // range and no more. A gap here is a range nobody collected; an overlap
        // is one collected twice and recorded once. Both would be written into
        // the coverage table as fact.
        assert_eq!(walked.first().map(|w| w.0), Some(0));
        assert_eq!(walked.last().map(|w| w.1), Some(250));
        for pair in walked.windows(2) {
            assert_eq!(pair[0].1, pair[1].0, "windows must not gap or overlap");
        }
    }

    #[test]
    fn a_range_with_nothing_in_it_walks_no_windows() {
        assert!(windows(100, 100, 60).is_empty(), "empty range");
        assert!(windows(100, 50, 60).is_empty(), "backwards range");
    }

    #[test]
    fn a_step_that_would_not_terminate_is_clamped() {
        // `Args` clamps `window_minutes` with `.max(1)`, but the loop must not
        // depend on that: a zero step would append the same window forever.
        assert_eq!(windows(0, 3, 0).len(), 3);
    }

    #[test]
    fn a_market_tape_pass_reaches_exactly_the_configured_width_when_the_horizon_allows_it() {
        assert_eq!(market_tape_window_end(1_000, 120, 10_000), 1_120);
    }

    #[test]
    fn a_market_tape_pass_never_reaches_past_the_horizon() {
        // The horizon is wall-clock lag: asking for a window that has not
        // landed in CryptoHouse yet would just fail, or worse, silently
        // narrow to whatever partial data happened to exist there.
        assert_eq!(
            market_tape_window_end(9_950, 120, 10_000),
            10_000,
            "the configured width would overshoot the horizon by 70s"
        );
    }

    #[test]
    fn a_market_tape_pass_at_exactly_the_horizon_reaches_no_further() {
        // The boundary the two tests above straddle: `wanted == horizon`
        // must return the horizon itself, not overshoot by treating equality
        // as "still room to grow".
        assert_eq!(market_tape_window_end(9_880, 120, 10_000), 10_000);
    }
}
