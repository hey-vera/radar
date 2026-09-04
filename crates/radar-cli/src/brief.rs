// SPDX-License-Identifier: Apache-2.0
//! `radar brief` — what the system is doing right now, and whether that is fine.
//!
//! Built as a **detector, not a dashboard**. It prints for a person, and it
//! *exits non-zero* when something is out of bounds, so a cron line can turn it
//! into an alarm without parsing anything. A dashboard needs someone to look at
//! it, and on 2026-08-24 the reason a thirteen-hour outage lasted thirteen hours
//! was that nobody did.
//!
//! # It checks progress, not liveness
//!
//! That day the recorder failed twice in different ways: it exited and stayed
//! dead, and then — once that was fixed — it stalled on a window it could not
//! fetch. A process check would have caught the first and passed the second,
//! while the store was equally not growing either way.
//!
//! So the headline number is **cursor age**: how long since the recorder last
//! finished a window. It catches a dead process, a stuck process, a wedged
//! upstream and a full disk, because all four look the same from the store —
//! which is the only vantage point that matters.
//!
//! # Absent is not healthy
//!
//! Every check reports [`Status::Unknown`] rather than passing when it cannot
//! measure its subject, and unknown is not a pass. A monitor that reports success
//! when it cannot see is worse than no monitor, because it is believed
//! (`AGENTS.md` rule 9, and `LEARNINGS` entry 5 one layer up).
//!
//! # It checks the serving surface too, because it did not
//!
//! Ingestion and serving are separate failures and this command only ever looked
//! at one of them. On 2026-08-25 the public server was running as a hand-started
//! process in an SSH login-session scope -- no unit, no `Restart=`, none of the
//! sandboxing `deploy/radar-serve.service` specifies -- and `brief` printed
//! *"Nothing is out of bounds"* and exited zero, because it never asked.
//!
//! An unconfigured endpoint is [`Status::Unknown`], not a pass. That is rule 8
//! applied to monitoring: a check with no target must say it cannot see rather
//! than report the silence as health. Set `RADAR_SERVE_URL`, or pass
//! `--serve-url`, to give it something to look at.

use std::path::Path;

use radar_asof::AsOf;
use radar_store::{Reader, Table, from_epoch, now_epoch, read_cursor};

/// Cursor age beyond which ingestion is considered behind.
///
/// The recorder stays 300s behind the chain by design and works in five-minute
/// windows, so a healthy cursor is routinely ~8 minutes old and occasionally
/// more. Twenty minutes is comfortably outside that and still well inside the
/// thirteen hours nobody noticed.
const LAG_WARN_SECONDS: i64 = 20 * 60;

/// Cursor age beyond which ingestion is considered stopped.
const LAG_FAIL_SECONDS: i64 = 60 * 60;

/// How a single check came out.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Status {
    Ok,
    /// Worth a look, not worth waking anyone.
    Warn,
    /// Something is broken.
    Fail,
    /// Could not be measured. Never treated as a pass.
    Unknown,
}

impl Status {
    const fn label(self) -> &'static str {
        match self {
            Self::Ok => "ok  ",
            Self::Warn => "WARN",
            Self::Fail => "FAIL",
            Self::Unknown => "????",
        }
    }

    /// Whether this status should make the command exit non-zero.
    const fn is_alarm(self) -> bool {
        matches!(self, Self::Fail | Self::Unknown)
    }
}

/// One line of the brief.
struct Check {
    status: Status,
    name: &'static str,
    detail: String,
}

impl Check {
    fn new(status: Status, name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            status,
            name,
            detail: detail.into(),
        }
    }
}

/// Prints the brief. Returns whether everything is within bounds.
///
/// Infallible on purpose. A check that cannot read its subject reports
/// [`Status::Unknown`] and alarms, rather than propagating an error — because
/// "the monitor broke" and "the thing being monitored broke" both mean someone
/// has to look, and a monitor that fails to run is the more alarming of the two.
pub fn run(store: &Path, serve_url: Option<&str>) -> bool {
    let reader = Reader::open(store);
    let now = now_epoch();

    let mut checks = vec![ingestion(store, now)];
    checks.push(watermark(&reader));
    checks.extend(tables(&reader));
    checks.push(outcomes(&reader));
    checks.push(decisions(&reader));
    checks.push(coordination(&reader));
    checks.push(screening(&reader));
    // Probed once and read twice. The agent's state is inside the health body
    // the serving check already fetched, so a second request would be a second
    // thing that can fail -- and it would fail whenever the server is down,
    // reporting the agent as broken because the server is.
    let probe = serve_url.map(|url| (url, probe_serving(url)));
    checks.push(agent(probe.as_ref().map(|(_, p)| p)));
    checks.push(serving(probe));
    checks.push(analyst(&analyst_dir()));
    checks.push(trading_lane());

    println!("radar brief — {}\n", from_epoch(now));
    for check in &checks {
        println!(
            "  [{}] {:<18} {}",
            check.status.label(),
            check.name,
            check.detail
        );
    }

    let alarms: Vec<&Check> = checks.iter().filter(|c| c.status.is_alarm()).collect();
    if alarms.is_empty() {
        println!("\nNothing is out of bounds.");
        return true;
    }

    println!("\n{} check(s) need attention:", alarms.len());
    for a in &alarms {
        println!("  {} — {}", a.name, a.detail);
    }
    false
}

/// How long since the recorder last finished a window.
///
/// The headline check, and the one that would have caught both of the day's
/// failures. Deliberately measured from the cursor rather than from a process
/// list: a recorder that is running and not advancing is an outage, and only the
/// cursor can tell.
fn ingestion(store: &Path, now: i64) -> Check {
    let Some(cursor) = read_cursor(store) else {
        return Check::new(
            Status::Unknown,
            "ingestion",
            "no follow cursor — the recorder has never run against this store, \
             or cannot write to it",
        );
    };

    let age = now - cursor;
    let detail = format!(
        "cursor {} — {} behind",
        from_epoch(cursor),
        humanise(age.max(0))
    );

    // A cursor in the future is not "very fresh", it is a clock or a corrupted
    // file, and reporting it as healthy would hide whichever it is.
    if age < -60 {
        return Check::new(
            Status::Unknown,
            "ingestion",
            format!(
                "cursor is {} in the future — check the clock",
                humanise(-age)
            ),
        );
    }
    let status = if age >= LAG_FAIL_SECONDS {
        Status::Fail
    } else if age >= LAG_WARN_SECONDS {
        Status::Warn
    } else {
        Status::Ok
    };
    Check::new(status, "ingestion", detail)
}

/// Where the analyst keeps its files.
///
/// The same default the daemon uses, and the same environment variable, so an
/// operator who moved the directory does not have to tell the brief twice.
fn analyst_dir() -> String {
    std::env::var("RADAR_ANALYST_DIR").unwrap_or_else(|_| "data/analyst".to_owned())
}

/// Whether the public analyst is answering, and when it last did.
///
/// # Why absence is Unknown rather than Ok
///
/// An analyst that has never run and an analyst that stopped look identical from
/// a directory listing, and both are reported as `Unknown`, which alarms. The
/// alternative -- treating "no log" as "nothing to report" -- is the failure
/// LEARNINGS 5 records: a check that reports absence the same way it reports
/// success. This account's whole product is answering in public, so a silent one
/// is the outage.
///
/// It reads the reply log rather than a process list, for the reason the
/// ingestion check reads the cursor: a daemon that is running and answering
/// nobody is the state worth catching, and only the log can tell.
fn analyst(dir: &str) -> Check {
    let log = format!("{dir}/replies.jsonl");
    let entries = match radar_analyst::log::read(&log) {
        Ok(entries) => entries,
        Err(e) => {
            return Check::new(
                Status::Unknown,
                "analyst",
                format!("no reply log at {log} — it has never run, or cannot write ({e})"),
            );
        }
    };

    let Some(last) = entries.iter().map(|e| e.at).max() else {
        return Check::new(
            Status::Unknown,
            "analyst",
            format!("{log} is empty — it has started and answered nothing"),
        );
    };

    // Counted over the folded view, because `publish` writes twice per reply --
    // once before it says anything and once after. Counting raw lines would
    // report double.
    let answered = radar_analyst::log::latest(&log).map_or(entries.len(), |v| v.len());
    let published = radar_analyst::log::latest(&log)
        .map_or(0, |v| v.iter().filter(|e| e.reply_id.is_some()).count());

    let age = now_epoch() - i64::try_from(last).unwrap_or(i64::MAX);
    let detail = format!(
        "{answered} answered, {published} published; last at {}",
        from_epoch(i64::try_from(last).unwrap_or(0))
    );

    // No threshold on the age. A quiet account is a quiet day, not an outage --
    // this thing answers when it is asked, and nobody asking is a fact about the
    // world rather than a fault. The number is printed so an operator can judge
    // it; alarming on it would be a check that fires on ordinary weather, which
    // AGENTS.md section 5 says is worse than no check.
    let _ = age;
    Check::new(Status::Ok, "analyst", detail)
}

/// The highest slot the store holds.
fn watermark(reader: &Reader) -> Check {
    match reader.watermark() {
        Ok(Some(slot)) => Check::new(Status::Ok, "watermark", format!("slot {slot}")),
        Ok(None) => Check::new(
            Status::Fail,
            "watermark",
            "the store holds no events at all",
        ),
        Err(e) => Check::new(Status::Unknown, "watermark", format!("cannot read: {e}")),
    }
}

/// Event counts per table, and how many of them failed on chain.
///
/// Failed transactions are counted separately because they are not a fault in
/// Radar and are real information about the market — but they are also how a
/// spam burst inflates a raw event count, so a brief that reported only the
/// total would read as a busy market during an attack.
fn tables(reader: &Reader) -> Vec<Check> {
    let Ok(Some(top)) = reader.watermark() else {
        return vec![Check::new(
            Status::Unknown,
            "tables",
            "cannot read the watermark, so counts would be meaningless",
        )];
    };
    let as_of = AsOf::at(top);
    let mut out = Vec::new();

    for table in Table::EVENT_TABLES {
        match reader.read(*table, as_of) {
            Ok(events) if events.is_empty() => {}
            Ok(events) => {
                let failed = events.iter().filter(|e| !e.envelope().succeeded).count();
                let detail = if failed == 0 {
                    format!("{} recorded", events.len())
                } else {
                    format!(
                        "{} recorded, {failed} of them failed on chain",
                        events.len()
                    )
                };
                out.push(Check::new(Status::Ok, table.dir(), detail));
            }
            Err(e) => out.push(Check::new(Status::Unknown, table.dir(), format!("{e}"))),
        }
    }
    out
}

/// When outcomes were last measured.
///
/// Reported in slots rather than wall time because that is what the measurement
/// carries. Converting it to hours would need a slot rate, and the measured rate
/// varies by 14% across days — a number with a made-up denominator reads more
/// precise than it is.
fn outcomes(reader: &Reader) -> Check {
    let Ok(Some(top)) = reader.watermark() else {
        return Check::new(Status::Unknown, "outcomes", "cannot read the watermark");
    };
    match reader.read_outcomes(AsOf::at(top)) {
        Ok(rows) if rows.is_empty() => Check::new(
            Status::Warn,
            "outcomes",
            "none measured yet — every signal is unvalidated until this runs",
        ),
        Ok(rows) => {
            let latest = rows.iter().map(|o| o.measured_at.get()).max().unwrap_or(0);
            let graduated = rows.iter().filter(|o| o.graduated()).count();
            Check::new(
                Status::Ok,
                "outcomes",
                format!(
                    "{} measurements, latest at slot {latest}, {graduated} graduations",
                    rows.len()
                ),
            )
        }
        Err(e) => Check::new(Status::Unknown, "outcomes", format!("cannot read: {e}")),
    }
}

/// How far behind the watermark the newest decision may fall before the
/// decision pass is considered stopped, in slots.
///
/// The pass runs hourly and a token is only eligible for ~40 minutes after
/// launch, so a gap wider than a few hours means passes are being missed rather
/// than that the funnel found nothing. Roughly three hours at 2.5 slots a
/// second, which tolerates two skipped runs before it complains.
const DECISIONS_STALE_AFTER: u64 = 27_000;

/// Whether the decision pass is still recording.
///
/// It exists because nothing else would notice it stopping. The outcomes cron
/// has the same shape and the same absence of supervision, and a recorder that
/// dies quietly is [LEARNINGS] entry 8 — the failure that left hours of chain
/// unrecorded and was found by regrounding rather than by an alarm.
///
/// Pure over the rows so both directions are testable without a store.
///
/// [LEARNINGS]: https://github.com/hey-vera/radar/blob/main/LEARNINGS.md
fn decisions(reader: &Reader) -> Check {
    let Ok(Some(top)) = reader.watermark() else {
        return Check::new(Status::Unknown, "decisions", "cannot read the watermark");
    };
    match reader.read_decisions(AsOf::at(top)) {
        Ok(rows) => decisions_health(&rows, top),
        Err(e) => Check::new(Status::Unknown, "decisions", format!("cannot read: {e}")),
    }
}

/// The rule, separated from the read.
fn decisions_health(rows: &[radar_store::Decision], top: radar_types::Slot) -> Check {
    let Some(newest) = rows.iter().map(|d| d.decided_at.get()).max() else {
        return Check::new(
            Status::Warn,
            "decisions",
            "none recorded — nothing can be joined to prices until the pass runs",
        );
    };

    let proposed = rows.iter().filter(|d| d.proposed()).count();
    // Counted apart from "recorded". A decision with no entry price cannot be
    // scored, so a store full of them is a pass that runs and produces nothing
    // usable -- which reads identically to a healthy one without this number.
    let scoreable = rows.iter().filter(|d| d.entry_price.is_some()).count();
    let behind = top.get().saturating_sub(newest);
    let detail = format!(
        "{} recorded, {proposed} proposed, {scoreable} with an entry price; \
         newest at slot {newest}",
        rows.len()
    );

    if behind > DECISIONS_STALE_AFTER {
        return Check::new(
            Status::Fail,
            "decisions",
            format!("{detail} — {behind} slots behind the watermark; the pass has stopped"),
        );
    }
    if scoreable == 0 {
        return Check::new(
            Status::Warn,
            "decisions",
            format!("{detail} — none carry an entry price, so none can be scored"),
        );
    }
    Check::new(Status::Ok, "decisions", detail)
}

/// Whether the coordination detector is still calibrated, over every decision
/// ever recorded.
///
/// A single `consider` pass reads a few dozen launch blocks, far short of what
/// a rate can be judged from, so a per-pass check could never fire. Recorded
/// decisions carry the verdict, so this accumulates across every pass.
///
/// It reads what was *observed*, never what was assumed: a decision whose launch
/// block could not be read carries no verdict and is excluded from both halves
/// of the rate, because counting an unread block as clean is precisely how a
/// gate goes quiet.
fn coordination(reader: &Reader) -> Check {
    let Ok(Some(top)) = reader.watermark() else {
        return Check::new(Status::Unknown, "coordination", "cannot read the watermark");
    };
    let Ok(decisions) = reader.read_decisions(AsOf::at(top)) else {
        return Check::new(Status::Unknown, "coordination", "cannot read decisions");
    };
    coordination_health(&decisions)
}

/// How many recent verdicts the calibration is judged over.
///
/// **A trailing window, not the lifetime.** A rate over all history is
/// insensitive to exactly what this monitor exists to catch: a detector that
/// worked for months and broke yesterday keeps a healthy lifetime average for
/// weeks while refusing nothing. The window has to be short enough to notice
/// and long enough to judge.
///
/// Three times [`MIN_SAMPLE`](radar_graph::MIN_SAMPLE), which at the hourly
/// pass's rate is a couple of days of decisions.
const CALIBRATION_WINDOW: usize = radar_graph::MIN_SAMPLE * 3;

/// The rule, separated from the read.
fn coordination_health(decisions: &[radar_store::Decision]) -> Check {
    // Newest last, as `read_decisions` returns them, so the tail is the window.
    // Only decisions carrying a verdict count: one whose launch block could not
    // be read is excluded from both halves, because counting an unread block as
    // clean is how a gate goes quiet.
    let recent: Vec<&radar_store::Decision> = decisions
        .iter()
        .filter(|d| d.coordination.is_some())
        .rev()
        .take(CALIBRATION_WINDOW)
        .collect();
    let observed = recent.len();
    let likely = recent
        .iter()
        .filter(|d| d.coordination.as_deref() == Some("Likely"))
        .count();

    match radar_graph::calibration_of(likely, observed) {
        radar_graph::Calibration::NotEnoughData { observed, needed } => Check::new(
            Status::Warn,
            "coordination",
            format!(
                "{observed} launch block(s) read, {needed} more before the detector's \
                 calibration can be judged"
            ),
        ),
        radar_graph::Calibration::Consistent { centre_rate_bps } => Check::new(
            Status::Ok,
            "coordination",
            format!("{likely} of {observed} at the centre ({centre_rate_bps} bps), consistent"),
        ),
        radar_graph::Calibration::Silent {
            centre_rate_bps,
            expected_bps,
            observed,
        } => Check::new(
            Status::Fail,
            "coordination",
            format!(
                "{centre_rate_bps} bps at the centre over {observed} block(s) against \
                 {expected_bps} measured — the band has gone quiet, and that is the \
                 direction that fails permissive"
            ),
        ),
        radar_graph::Calibration::Elevated {
            centre_rate_bps,
            expected_bps,
            observed,
        } => Check::new(
            Status::Warn,
            "coordination",
            format!(
                "{centre_rate_bps} bps at the centre over {observed} block(s) against \
                 {expected_bps} measured — either the market moved or this sample is not \
                 what it is believed to be"
            ),
        ),
    }
}

/// What a probe of the serving surface came back with.
///
/// An enum rather than a `Result<String, _>` so that "answered, but wrongly" is
/// a distinct case from "did not answer". A server returning 500 and a server
/// that is not there are different failures with different fixes, and a check
/// that rendered both as "down" would send someone to the wrong place.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ServingProbe {
    /// The endpoint answered, with this status code and body.
    Answered {
        /// HTTP status.
        status: u16,
        /// Response body, truncated by the caller if need be.
        body: String,
    },
    /// The endpoint could not be reached at all.
    Unreachable(String),
}

/// Whether the public serving surface is up.
///
/// Pure, and separated from the fetch for the same reason `radar-sim` puts the
/// curve logic behind [`Quoter`]: the rule that decides *healthy* has to be
/// exercisable in both directions without a network. A check only ever verified
/// in its passing direction is the bug this function was added for.
///
/// [`Quoter`]: https://github.com/hey-vera/radar
/// How often the coordination gate actually ran.
///
/// # Why this is separate from [`coordination`]
///
/// That check judges the detector's *calibration*, and to do so it deliberately
/// excludes decisions whose launch block could not be read — "counting an unread
/// block as clean is how a gate goes quiet". Correct for calibration, and it
/// leaves the detector blind to the one failure that matters most here: if every
/// launch-block read started failing, `observed` would fall to zero and that
/// check would report `NotEnoughData`, which is a **`Warn`**, which is **not an
/// alarm**. The strongest refusal signal in the system could stop running
/// entirely with `radar brief` still exiting zero.
///
/// So coverage is measured on its own. It asks a different question — not "is
/// the detector calibrated" but "did it get to look at all".
///
/// # What a miss costs
///
/// `consider.rs` records `coordination = None` when CryptoHouse cannot serve the
/// launch block, and `creator_edge` correctly refuses to let `None` refuse —
/// inventing evidence would refuse the whole population whenever the vendor
/// hiccups. The consequence is that a candidate whose block went unread proceeds
/// **without** the screen that [`0008`] measures at 11.7× on instant
/// graduation, and nothing downstream says so.
///
/// Measured on 2026-08-30 over 2,706 paid-tier candidates: 528 had an unreadable
/// launch block, and they were proposed at 55.0% against 51.8% overall.
///
/// [`0008`]: ../../docs/research/0008-the-launch-block-gives-the-bundle-away.md
fn screening(reader: &Reader) -> Check {
    let Ok(Some(top)) = reader.watermark() else {
        return Check::new(Status::Unknown, "screening", "cannot read the watermark");
    };
    let Ok(decisions) = reader.read_decisions(AsOf::at(top)) else {
        return Check::new(Status::Unknown, "screening", "cannot read decisions");
    };
    screening_health(&decisions)
}

/// Coverage below which the gate is warned about, per ten thousand.
///
/// **Set from the spread, not from a round number.** Measured per hourly run on
/// 2026-08-30 the unreadable share moved between 5.1% and 39.5% — so a threshold
/// on one run would fire on noise, and this is judged over the same trailing
/// window the calibration uses. Ninety per cent is above the whole of that
/// observed band, so anything inside normal operation reads as degraded, which
/// it is.
const SCREENED_WARN_BPS: u64 = 9_000;

/// Coverage below which it is an alarm, per ten thousand.
///
/// Half. Below this the gate is more off than on, and describing that as
/// degradation rather than failure would be a euphemism.
///
/// Deliberately far below the *current* rate. A check that fails from the day it
/// ships teaches its reader to ignore it, and the state this catches — the gate
/// mostly gone — has not happened. The warn threshold is what speaks to today.
const SCREENED_FAIL_BPS: u64 = 5_000;

/// How many recent decisions the fast window judges on.
///
/// **Three hourly runs at `--cap 40`.** The trailing window this check used to
/// judge on alone is 600 decisions — about fifteen hours — and that is long
/// enough to hide the exact failure it exists to catch.
///
/// It did. On 2026-08-31 the launch-block read collapsed to roughly 38% coverage
/// over three consecutive runs, well past the fail threshold, and this check
/// reported **73%** and a warning: the fifteen-hour average was still carrying
/// twelve hours of healthy runs. A monitor that takes most of a day to describe a
/// collapse is reporting history.
///
/// Three runs rather than one, because a single run genuinely is noisy — the
/// per-run unreadable share moved between 5.1% and 39.5% on 2026-08-30, so a
/// one-run trigger would fire on weather. Three consecutive bad runs is not
/// weather.
const FAST_WINDOW: usize = 120;

/// The rule, separated from the read.
///
/// Judged on **two** windows, and the verdict is the worse of them. The slow
/// window catches a gradual drift that no single stretch would show; the fast one
/// catches a collapse while it is still happening. Taking the worse rather than
/// averaging them is deliberate — rule 8's direction, where the reading that
/// costs money if ignored wins.
fn screening_health(decisions: &[radar_store::Decision]) -> Check {
    // Newest last, as `read_decisions` returns them, so the tail is the window.
    let recent: Vec<&radar_store::Decision> =
        decisions.iter().rev().take(CALIBRATION_WINDOW).collect();
    if recent.is_empty() {
        return Check::new(
            Status::Warn,
            "screening",
            "no decisions recorded yet, so the gate's coverage cannot be judged",
        );
    }

    let (slow_bps, total, screened) = coverage(&recent);
    let missed = total - screened;

    // Only when there is a fast window's worth. Below that the fast reading is
    // the slow one with fewer samples, and reporting it as a second opinion
    // would be reporting the same number twice.
    let fast = (recent.len() >= FAST_WINDOW).then(|| coverage(&recent[..FAST_WINDOW]).0);

    let trailing = fast.map_or_else(String::new, |fast_bps| {
        format!("; the last {FAST_WINDOW} read {fast_bps} bps")
    });
    let detail = format!(
        "{screened} of {total} candidates had their launch block read ({slow_bps} bps); {missed} skipped the coordination screen{trailing}"
    );

    // The worse of the two. A fast window well below the slow one is a collapse
    // in progress, and it is the reading that matters.
    let judged = fast.map_or(slow_bps, |fast_bps| fast_bps.min(slow_bps));
    let status = if judged < SCREENED_FAIL_BPS {
        Status::Fail
    } else if judged < SCREENED_WARN_BPS {
        Status::Warn
    } else {
        Status::Ok
    };
    Check::new(status, "screening", detail)
}

/// Coverage over a slice, per ten thousand, with the counts it came from.
///
/// Integer throughout, so the same counts always give the same verdict.
fn coverage(window: &[&radar_store::Decision]) -> (u64, usize, usize) {
    let total = window.len();
    let screened = window.iter().filter(|d| d.coordination.is_some()).count();
    let bps = u64::try_from(screened).unwrap_or(0).saturating_mul(10_000)
        / u64::try_from(total).unwrap_or(1).max(1);
    (bps, total, screened)
}

fn serving(probed: Option<(&str, ServingProbe)>) -> Check {
    // Rule 8. A monitor with no target must say it cannot see, rather than
    // report the silence as health -- which is exactly how a server that was
    // never installed as a service went unnoticed while this command printed
    // "Nothing is out of bounds".
    let Some((url, probe)) = probed else {
        return Check::new(
            Status::Unknown,
            "serving",
            "no endpoint configured — set RADAR_SERVE_URL or pass --serve-url; an unwatched surface is not a healthy one",
        );
    };

    match probe {
        ServingProbe::Unreachable(why) => Check::new(
            Status::Fail,
            "serving",
            format!("{url} is not answering: {why}"),
        ),
        ServingProbe::Answered { status, .. } if status != 200 => {
            Check::new(Status::Fail, "serving", format!("{url} answered {status}"))
        }
        ServingProbe::Answered { body, .. } => {
            // The endpoint reports its own status field. Trusting the 200 alone
            // would pass a server that is up and broken.
            if body.contains("\"status\":\"ok\"") || body.contains("\"status\": \"ok\"") {
                let paid = if body.contains("\"paidSurface\":true")
                    || body.contains("\"paidSurface\": true")
                {
                    "paid surface on"
                } else {
                    "paid surface off"
                };
                Check::new(Status::Ok, "serving", format!("{url} ok, {paid}"))
            } else {
                // 200 with a body we do not recognise is not a pass. It is most
                // likely something else listening on the port.
                Check::new(
                    Status::Fail,
                    "serving",
                    format!("{url} answered 200 but not as radar-serve"),
                )
            }
        }
    }
}

/// Asks the serving surface how it is.
///
/// Short timeouts: this runs on a timer and a hanging health check is its own
/// outage. Never panics and never propagates -- an unreachable endpoint is a
/// finding, not an error.
fn probe_serving(url: &str) -> ServingProbe {
    let health = format!("{}/health", url.trim_end_matches('/'));
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(5)))
        .build()
        .into();

    match agent.get(&health).call() {
        Ok(mut response) => {
            let status = response.status().as_u16();
            let body = response
                .body_mut()
                .read_to_string()
                .unwrap_or_else(|e| format!("<unreadable body: {e}>"));
            ServingProbe::Answered { status, body }
        }
        Err(e) => ServingProbe::Unreachable(e.to_string()),
    }
}

/// What the trading lane is doing, which is nothing.
///
/// Stated every run rather than only when it changes. An operator reading a
/// brief should never have to remember whether capital is armed, and a line that
/// appears only on change is a line whose absence means two different things.
fn trading_lane() -> Check {
    // Derived, not asserted. This read `Policy::CLOSED` and hardcoded both
    // `Status::Ok` and the words "no proposal can become an authorization" — so
    // it answered "nothing can trade" whatever the decider had been changed to.
    // `brief` is the thing that wakes somebody up, which makes it the worst
    // place in the system for a line that cannot stop reassuring.
    //
    // `Policy::SHIPPED` is the constant `consider` decides with, so this line
    // moves when that does.
    let policy = radar_risk::Policy::SHIPPED;
    if policy.is_closed() {
        return Check::new(
            Status::Ok,
            "trading",
            format!(
                "autonomy {:?}, max position {} — no proposal can become an authorization",
                policy.autonomy, policy.max_position
            ),
        );
    }
    // Not `Fail`: an armed lane may be exactly what the operator intended, and
    // an alarm that fires on an intended state is one that gets silenced. It is
    // still the largest state change this command can report and must never be
    // printed as routine.
    Check::new(
        Status::Warn,
        "trading",
        format!(
            "CAPITAL IS ARMED — autonomy {:?}, max position {}. The signer holds its              own policy (ADR 0008) and it is not readable from here",
            policy.autonomy, policy.max_position
        ),
    )
}

/// What the serving surface says about its reading assistant.
///
/// Read out of the health body the serving check already fetched, rather than by
/// opening a second connection. A separate probe would be a second thing that
/// can fail, and it would fail *in the same way* whenever the component it is
/// probing is the one that is down — reporting the agent as broken because the
/// server is, which is a wrong diagnosis dressed as a specific one.
///
/// Alarms in both directions, which is the whole reason it exists. A check that
/// can only say "a provider is configured" is a working component and a dead one
/// printing the same thing — LEARNINGS 5, 7 and 10, and the failure this feature
/// is most likely to have: a credential that lapsed after a fortnight of
/// inactivity leaves the configuration untouched and every call failing.
fn agent(probed: Option<&ServingProbe>) -> Check {
    let Some(ServingProbe::Answered { body, .. }) = probed else {
        // Not a failure of the agent. The serving check already alarms on an
        // unreachable server, and two lines reporting one outage is two people
        // looking for two problems.
        return Check::new(
            Status::Unknown,
            "agent",
            "cannot see — the serving surface did not answer",
        );
    };

    let Ok(health) = serde_json::from_str::<serde_json::Value>(body) else {
        return Check::new(Status::Unknown, "agent", "the health body is not JSON");
    };

    let Some(agent) = health.get("agent") else {
        // An older binary than this check. Saying which is the useful part: the
        // alternative reading -- "no agent" -- is indistinguishable from a
        // deliberately unconfigured one, and only one of them wants action.
        return Check::new(
            Status::Unknown,
            "agent",
            "the serving surface reports no agent field — it predates this check",
        );
    };

    if agent.get("configured").and_then(serde_json::Value::as_bool) != Some(true) {
        // The shipped state. Not an alarm, or the brief is permanently red on
        // every instance that has not been given a model.
        return Check::new(
            Status::Ok,
            "agent",
            "off — no model provider configured (set RADAR_MODEL_DAILY_USD and a provider)",
        );
    }

    let provider = agent
        .get("provider")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unnamed");
    let spent = agent
        .get("spent_micro_usd")
        .and_then(serde_json::Value::as_u64);
    let tools = agent
        .get("tools")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_default();

    let today = spent.map_or_else(
        || "spend unknown".to_owned(),
        |s| format!("${}.{:06} today", s / 1_000_000, s % 1_000_000),
    );

    match agent
        .get("last")
        .and_then(|l| l.get("last_call"))
        .and_then(serde_json::Value::as_str)
    {
        Some("ok") => Check::new(
            Status::Ok,
            "agent",
            format!("{provider} answering, {tools} read-only tool(s), {today}"),
        ),
        Some("failed") => {
            let why = agent
                .get("last")
                .and_then(|l| l.get("why"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("no reason given");
            Check::new(
                Status::Fail,
                "agent",
                format!("{provider} refused the last call: {why}"),
            )
        }
        Some("never") => Check::new(
            Status::Ok,
            "agent",
            format!("{provider} configured, nothing asked since the last restart, {today}"),
        ),
        // A state this check does not know. Not a pass: the variant was added by
        // somebody who did not update this file, and guessing which way it goes
        // is how a new failure state gets reported as health.
        other => Check::new(
            Status::Unknown,
            "agent",
            format!("{provider} reports an unrecognised state: {other:?}"),
        ),
    }
}

/// Renders a duration the way an operator reads one.
fn humanise(seconds: i64) -> String {
    match seconds {
        s if s < 90 => format!("{s}s"),
        s if s < 5_400 => format!("{}m", s / 60),
        s => format!("{}h{:02}m", s / 3_600, (s % 3_600) / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_store_with_no_cursor_is_unknown_rather_than_healthy() {
        // The failure this whole command exists to catch would be invisible if
        // "cannot measure" counted as a pass.
        let dir = tempfile::tempdir().expect("tempdir");
        let check = ingestion(dir.path(), now_epoch());
        assert_eq!(check.status, Status::Unknown);
        assert!(check.status.is_alarm(), "unknown must alarm");
    }

    fn a_decision(slot: u64, proposed: bool, entry: Option<u64>) -> radar_store::Decision {
        radar_store::Decision {
            mint: radar_types::Address::new([1u8; 32]),
            creator: radar_types::Address::new([2u8; 32]),
            decided_at: radar_types::Slot(slot),
            launch_slot: radar_types::Slot(slot.saturating_sub(6_000)),
            strategy: "creator_edge".to_owned(),
            strategy_version: "0.1.0".to_owned(),
            conclusion: if proposed {
                radar_store::Conclusion::Proposed
            } else {
                radar_store::Conclusion::Passed
            },
            reasons: Vec::new(),
            notional_micro_usd: None,
            exit_capacity_micro_usd: None,
            assumed_round_trip_bps: 850,
            coordination: None,
            authority_prevalence: None,
            kernel_outcome: None,
            kernel_reasons: Vec::new(),
            entry_price: entry,
            inputs_digest: "d".to_owned(),
        }
    }

    fn with_coordination(verdict: Option<&str>) -> radar_store::Decision {
        let mut d = a_decision(999_900, false, None);
        d.coordination = verdict.map(std::borrow::ToOwned::to_owned);
        d
    }

    #[test]
    fn the_gate_going_completely_silent_is_an_alarm_where_calibration_only_warns() {
        // The exact gap `screening` exists to close, asserted as a relationship
        // rather than as two separate facts.
        //
        // `an_unread_launch_block_is_excluded_from_both_halves_of_the_rate`
        // below establishes that 300 unread blocks leave `coordination` at
        // `Warn`, and `Status::is_alarm` covers only `Fail` and `Unknown` -- so
        // on that check alone the strongest refusal signal in the system could
        // stop running entirely and `radar brief` would still exit zero.
        let decisions: Vec<radar_store::Decision> =
            (0..300).map(|_| with_coordination(None)).collect();

        assert!(
            !coordination_health(&decisions).status.is_alarm(),
            "the calibration check cannot see this, which is why screening exists"
        );
        assert_eq!(
            screening_health(&decisions).status,
            Status::Fail,
            "a gate that never ran must alarm"
        );
        assert!(screening_health(&decisions).status.is_alarm());
    }

    #[test]
    fn a_collapse_in_the_last_three_runs_alarms_despite_a_healthy_day_behind_it() {
        // The failure this window was added for, reproduced exactly.
        //
        // On 2026-08-31 the launch-block read collapsed to roughly 38% coverage
        // over three consecutive hourly runs. That is well past the fail
        // threshold. This check reported **73% and a warning**, because its
        // fifteen-hour window was still carrying twelve hours of healthy runs.
        //
        // Built newest-last, as `read_decisions` returns them, so the collapse is
        // at the tail where the fast window looks.
        let mut decisions = Vec::new();
        // Twelve healthy hours: 480 decisions at ~95% coverage.
        for i in 0..480 {
            decisions.push(with_coordination((i % 20 != 0).then_some("Unlikely")));
        }
        // Then three runs at 38%.
        for i in 0..FAST_WINDOW {
            decisions.push(with_coordination((i % 100 < 38).then_some("Unlikely")));
        }

        let check = screening_health(&decisions);
        assert_eq!(
            check.status,
            Status::Fail,
            "a collapse to 38% must alarm while it is happening: {}",
            check.detail
        );
        // And it has to *say* both, or a reader cannot tell a collapse from a
        // slow decline -- which is the difference between the two windows.
        assert!(
            check
                .detail
                .contains(&format!("the last {FAST_WINDOW} read")),
            "the detail must report the fast window: {}",
            check.detail
        );
    }

    #[test]
    fn a_single_bad_run_does_not_alarm_on_its_own() {
        // The other direction, and the reason the fast window is three runs
        // rather than one. The per-run unreadable share moved between 5.1% and
        // 39.5% on 2026-08-30 -- so one bad run is weather, and a monitor that
        // fires on weather teaches its reader to ignore it.
        let mut decisions = Vec::new();
        for i in 0..560 {
            decisions.push(with_coordination((i % 20 != 0).then_some("Unlikely")));
        }
        // One run of 40 at 38%, followed by two healthy ones, so the fast window
        // holds a single bad run among three.
        for i in 0..40 {
            decisions.push(with_coordination((i % 100 < 38).then_some("Unlikely")));
        }
        for i in 0..80 {
            decisions.push(with_coordination((i % 20 != 0).then_some("Unlikely")));
        }

        assert_ne!(
            screening_health(&decisions).status,
            Status::Fail,
            "one bad run among three is not a collapse"
        );
    }

    #[test]
    fn a_healthy_recent_stretch_does_not_rescue_a_degraded_history() {
        // The worse-of-two rule, checked in the direction that would be easy to
        // get backwards. A good fast window must not clear a slow window that is
        // failing -- averaging or preferring the fast reading would let a system
        // that has been broken all day report healthy after one good hour.
        let mut decisions = Vec::new();
        for i in 0..480 {
            decisions.push(with_coordination((i % 100 < 30).then_some("Unlikely")));
        }
        for _ in 0..FAST_WINDOW {
            decisions.push(with_coordination(Some("Unlikely")));
        }

        assert_eq!(
            screening_health(&decisions).status,
            Status::Fail,
            "one good stretch must not clear a day of failure"
        );
    }

    #[test]
    fn full_coverage_passes_and_partial_coverage_is_graded() {
        // Swept across the thresholds rather than sampled at one point, because
        // the interesting property is that each band is reachable -- a threshold
        // no input can land between is a threshold that does nothing.
        let cohort = |screened: usize, unscreened: usize| -> Vec<radar_store::Decision> {
            let mut v: Vec<radar_store::Decision> = (0..screened)
                .map(|_| with_coordination(Some("Unlikely")))
                .collect();
            v.extend((0..unscreened).map(|_| with_coordination(None)));
            v
        };

        assert_eq!(screening_health(&cohort(100, 0)).status, Status::Ok);
        // 95% -- inside the band, clear of the warn threshold.
        assert_eq!(screening_health(&cohort(95, 5)).status, Status::Ok);
        // 80% -- the rate actually measured in production on 2026-08-30.
        assert_eq!(screening_health(&cohort(80, 20)).status, Status::Warn);
        // 60% -- degraded but the gate is still mostly running.
        assert_eq!(screening_health(&cohort(60, 40)).status, Status::Warn);
        // 40% -- more off than on.
        assert_eq!(screening_health(&cohort(40, 60)).status, Status::Fail);
    }

    #[test]
    fn both_coverage_thresholds_are_inclusive_on_the_healthier_side() {
        // Swept at the exact boundary rather than either side of it. The
        // previous test steps over both thresholds without ever landing on one,
        // so `<` could become `<=` and nothing noticed -- which would move a
        // cohort sitting exactly on the line into the worse band.
        let cohort = |screened: usize, unscreened: usize| -> Vec<radar_store::Decision> {
            let mut v: Vec<radar_store::Decision> = (0..screened)
                .map(|_| with_coordination(Some("Unlikely")))
                .collect();
            v.extend((0..unscreened).map(|_| with_coordination(None)));
            v
        };

        // Exactly 9,000 bps -- the warn threshold. Clearing it is passing.
        assert_eq!(screening_health(&cohort(90, 10)).status, Status::Ok);
        assert_eq!(screening_health(&cohort(89, 11)).status, Status::Warn);

        // Exactly 5,000 bps -- the fail threshold. Half the gate running is
        // degraded, not failed; below half is failed.
        assert_eq!(screening_health(&cohort(50, 50)).status, Status::Warn);
        assert_eq!(screening_health(&cohort(49, 51)).status, Status::Fail);
    }

    #[test]
    fn the_detail_names_how_many_candidates_skipped_the_screen() {
        // The status says how bad it is; only the detail says how many. An
        // operator reading `[warn] screening` needs the count to act, and
        // nothing else asserts it -- so the subtraction that produces it was
        // free to become an addition.
        let mut decisions: Vec<radar_store::Decision> = (0..80)
            .map(|_| with_coordination(Some("Unlikely")))
            .collect();
        decisions.extend((0..20).map(|_| with_coordination(None)));

        let detail = screening_health(&decisions).detail;
        assert!(
            detail.contains("80 of 100"),
            "must name both halves: {detail}"
        );
        assert!(
            !detail.contains("  "),
            "no run of spaces in a line an operator reads: {detail}"
        );
        assert!(
            detail.contains("20 skipped"),
            "must name the miss count, not the sum: {detail}"
        );
    }

    #[test]
    fn an_empty_store_cannot_report_coverage_and_says_so() {
        // Absent is not zero (rule 9). No decisions is "cannot judge", not "the
        // gate never ran" -- reporting the latter would alarm on a fresh install.
        let check = screening_health(&[]);
        assert_eq!(check.status, Status::Warn);
        assert!(
            check.detail.contains("cannot be judged"),
            "must say why: {}",
            check.detail
        );
    }

    #[test]
    fn an_unread_launch_block_is_excluded_from_both_halves_of_the_rate() {
        // Counting an unread block as clean is precisely how a gate goes quiet:
        // the denominator grows, the rate falls, and the monitor reports a
        // detector that has stopped working as one that is finding nothing.
        let mut decisions: Vec<radar_store::Decision> =
            (0..300).map(|_| with_coordination(None)).collect();
        let check = coordination_health(&decisions);
        assert_eq!(
            check.status,
            Status::Warn,
            "three hundred unread blocks is still no sample: {}",
            check.detail
        );

        // Add a real sample at the measured rate and it becomes judgeable.
        for i in 0..300 {
            decisions.push(with_coordination(Some(if i < 17 {
                "Likely"
            } else {
                "Unremarkable"
            })));
        }
        assert_eq!(
            coordination_health(&decisions).status,
            Status::Ok,
            "17 of 300 is 566 bps, near the 580 measured"
        );
    }

    #[test]
    fn a_detector_that_has_gone_quiet_fails_the_brief() {
        // The alarm that matters, and the reason this check exists at all.
        // BUNDLE_CENTRE is a bundler tool's default; when it moves, nothing
        // errors and Radar simply stops refusing.
        let decisions: Vec<radar_store::Decision> = (0..400)
            .map(|_| with_coordination(Some("Unremarkable")))
            .collect();
        let check = coordination_health(&decisions);
        assert_eq!(check.status, Status::Fail);
        assert!(
            check.detail.contains("fails permissive"),
            "the detail must say why quiet is the dangerous direction: {}",
            check.detail
        );
    }

    #[test]
    fn a_small_sample_warns_rather_than_passing_or_failing() {
        // Neither a clean bill of health nor an alarm. An hourly pass reads a
        // few dozen blocks, so this is the state the check spends its first days
        // in, and it must not read as either.
        let decisions: Vec<radar_store::Decision> = (0..20)
            .map(|_| with_coordination(Some("Unremarkable")))
            .collect();
        assert_eq!(coordination_health(&decisions).status, Status::Warn);
    }

    #[test]
    fn a_detector_that_broke_recently_alarms_despite_a_healthy_history() {
        // The reason the window exists. A lifetime rate lets months of correct
        // behaviour drown a detector that stopped working yesterday -- it keeps
        // a healthy average for weeks while refusing nothing, which is the exact
        // failure this monitor was built to notice.
        let mut decisions = Vec::new();
        // A long, healthy past: 5,000 verdicts at roughly the measured rate.
        for i in 0..5_000 {
            decisions.push(with_coordination(Some(if i % 17 == 0 {
                "Likely"
            } else {
                "Unremarkable"
            })));
        }
        // Then it goes quiet, for longer than the window.
        for _ in 0..CALIBRATION_WINDOW {
            decisions.push(with_coordination(Some("Unremarkable")));
        }

        assert_eq!(
            coordination_health(&decisions).status,
            Status::Fail,
            "a healthy history must not mask a detector that has just stopped"
        );
    }

    #[test]
    fn a_detector_that_has_recovered_stops_alarming() {
        // The other direction, and the reason this is a window rather than a
        // marker: a fix has to be able to clear the alarm without anybody
        // editing state. A long dead stretch followed by a healthy window reads
        // as healthy.
        let mut decisions: Vec<radar_store::Decision> = (0..5_000)
            .map(|_| with_coordination(Some("Unremarkable")))
            .collect();
        for i in 0..CALIBRATION_WINDOW {
            decisions.push(with_coordination(Some(if i % 17 == 0 {
                "Likely"
            } else {
                "Unremarkable"
            })));
        }

        assert_eq!(
            coordination_health(&decisions).status,
            Status::Ok,
            "a recovered detector must clear on its own"
        );
    }

    #[test]
    fn unread_blocks_do_not_consume_the_window() {
        // A stretch where the source was unavailable must not push real verdicts
        // out of the window and leave the monitor judging nothing. Absence is
        // excluded before the window is taken, not after.
        let mut decisions: Vec<radar_store::Decision> = (0..300)
            .map(|i| {
                with_coordination(Some(if i % 17 == 0 {
                    "Likely"
                } else {
                    "Unremarkable"
                }))
            })
            .collect();
        for _ in 0..5_000 {
            decisions.push(with_coordination(None));
        }

        assert_eq!(
            coordination_health(&decisions).status,
            Status::Ok,
            "five thousand unread blocks must not evict three hundred real ones"
        );
    }

    #[test]
    fn a_decision_pass_that_has_stopped_is_reported_as_broken() {
        // The check exists because nothing else would notice. The outcomes cron
        // has the same shape and the same absence of supervision, and a recorder
        // that dies quietly is LEARNINGS 8 -- hours of chain lost, found by
        // regrounding rather than by an alarm.
        let top = radar_types::Slot(1_000_000);
        let fresh = vec![a_decision(1_000_000 - 100, true, Some(9))];
        let stalled = vec![a_decision(
            1_000_000 - DECISIONS_STALE_AFTER - 1,
            true,
            Some(9),
        )];

        assert_eq!(decisions_health(&fresh, top).status, Status::Ok);
        assert_eq!(
            decisions_health(&stalled, top).status,
            Status::Fail,
            "a pass that has not run in hours must alarm, not read as quiet"
        );
    }

    #[test]
    fn decisions_that_cannot_be_scored_are_not_reported_as_healthy() {
        // A pass that runs and records rows carrying no entry price produces
        // nothing usable, and without this it reads identically to a healthy
        // one. That is a check reporting absence the same way it reports
        // success -- LEARNINGS 5.
        let top = radar_types::Slot(1_000_000);
        let unscoreable = vec![a_decision(999_900, true, None)];
        let scoreable = vec![a_decision(999_900, true, Some(9))];

        assert_eq!(decisions_health(&unscoreable, top).status, Status::Warn);
        assert_eq!(decisions_health(&scoreable, top).status, Status::Ok);
    }

    #[test]
    fn no_decisions_at_all_warns_rather_than_passing_silently() {
        let empty: Vec<radar_store::Decision> = Vec::new();
        let check = decisions_health(&empty, radar_types::Slot(1_000_000));
        assert_eq!(check.status, Status::Warn);
        assert!(
            check.detail.contains("joined to prices"),
            "the detail should say why it matters: {}",
            check.detail
        );
    }

    #[test]
    fn the_staleness_window_tolerates_a_missed_pass_but_not_a_day() {
        // Exercised through the rule rather than asserted about the constant:
        // an `assert!` over a constant is a lint, and more to the point it
        // checks the number rather than the behaviour the number is for.
        //
        // Hourly at roughly 9,000 slots an hour. One missed pass must not cry
        // wolf; a day of silence must not pass.
        let top = radar_types::Slot(1_000_000);
        let gap = |slots: u64| {
            decisions_health(&[a_decision(1_000_000 - slots, true, Some(9))], top).status
        };

        assert_eq!(gap(9_000), Status::Ok, "one missed pass is not a failure");
        assert_eq!(gap(216_000), Status::Fail, "a day of silence is");

        // The boundary itself, because `>` and `>=` differ only there and a
        // window tested only well inside and well outside is a window whose
        // edge nobody has checked.
        assert_eq!(
            gap(DECISIONS_STALE_AFTER),
            Status::Ok,
            "exactly at the window is still within it"
        );
        assert_eq!(
            gap(DECISIONS_STALE_AFTER + 1),
            Status::Fail,
            "one slot past it is not"
        );
    }

    #[test]
    fn a_fresh_cursor_is_healthy_and_a_stale_one_is_not() {
        let dir = tempfile::tempdir().expect("tempdir");
        let now = radar_store::to_epoch("2026-08-24 20:00:00").expect("parse");

        radar_store::write_cursor(dir.path(), now - 300).expect("write");
        assert_eq!(ingestion(dir.path(), now).status, Status::Ok);

        // Twenty minutes: outside the recorder's normal rhythm.
        radar_store::write_cursor(dir.path(), now - LAG_WARN_SECONDS - 1).expect("write");
        assert_eq!(ingestion(dir.path(), now).status, Status::Warn);

        // An hour: the shape of the outage that went unnoticed for thirteen.
        radar_store::write_cursor(dir.path(), now - LAG_FAIL_SECONDS - 1).expect("write");
        let check = ingestion(dir.path(), now);
        assert_eq!(check.status, Status::Fail);
        assert!(check.status.is_alarm());
    }

    #[test]
    fn the_thirteen_hour_outage_would_have_alarmed() {
        // The actual event, as a regression test. The cursor sat at 05:30:26
        // while the clock reached 18:36, and nothing said so.
        let dir = tempfile::tempdir().expect("tempdir");
        let stuck = radar_store::to_epoch("2026-08-24 05:30:26").expect("parse");
        let found = radar_store::to_epoch("2026-08-24 18:36:09").expect("parse");
        radar_store::write_cursor(dir.path(), stuck).expect("write");

        let check = ingestion(dir.path(), found);
        assert_eq!(check.status, Status::Fail);
        assert!(check.detail.contains("13h"), "detail was {}", check.detail);
    }

    #[test]
    fn a_cursor_from_the_future_is_not_reported_as_very_fresh() {
        // A clock skew or a corrupted file would otherwise render as the
        // healthiest possible reading.
        let dir = tempfile::tempdir().expect("tempdir");
        let now = radar_store::to_epoch("2026-08-24 20:00:00").expect("parse");
        radar_store::write_cursor(dir.path(), now + 3_600).expect("write");
        assert_eq!(ingestion(dir.path(), now).status, Status::Unknown);
    }

    #[test]
    fn an_empty_store_fails_rather_than_reporting_nothing_wrong() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(watermark(&Reader::open(dir.path())).status, Status::Fail);
    }

    /// A health body with the agent block filled in as given.
    fn health_with(agent_json: &str) -> ServingProbe {
        ServingProbe::Answered {
            status: 200,
            body: format!(r#"{{"status":"ok","agent":{agent_json}}}"#),
        }
    }

    #[test]
    fn the_agent_check_alarms_when_the_last_call_failed() {
        // The direction that matters, and the failure this feature is most
        // likely to have: a credential that lapsed after a fortnight of
        // inactivity leaves every setting untouched and every call failing.
        let probe = health_with(
            r#"{"configured":true,"provider":"codex","tools":3,"spent_micro_usd":0,
                "last":{"last_call":"failed","why":"the CLI exited with exit status: 1"}}"#,
        );
        let check = agent(Some(&probe));
        assert_eq!(check.status, Status::Fail);
        assert!(check.detail.contains("exit status: 1"), "{}", check.detail);
        assert!(check.status.is_alarm(), "and it exits non-zero");
    }

    #[test]
    fn the_agent_check_passes_when_the_last_call_worked() {
        // The other direction. A check verified only where it fails is
        // indistinguishable from one that always fails, which is finding 1.6 of
        // the plan and the reason this test exists beside the one above.
        let probe = health_with(
            r#"{"configured":true,"provider":"codex","tools":3,"spent_micro_usd":1500000,
                "last":{"last_call":"ok"}}"#,
        );
        let check = agent(Some(&probe));
        assert_eq!(check.status, Status::Ok);
        assert!(check.detail.contains("codex answering"), "{}", check.detail);
        assert!(check.detail.contains("$1.500000"), "{}", check.detail);
    }

    #[test]
    fn an_unconfigured_agent_is_not_an_alarm() {
        // The shipped state. Alarming on it would leave the brief permanently
        // red on every instance that has not been given a model, and a monitor
        // that is always red is a monitor that gets ignored.
        let check = agent(Some(&health_with(r#"{"configured":false}"#)));
        assert_eq!(check.status, Status::Ok);
        assert!(check.detail.starts_with("off"), "{}", check.detail);
    }

    #[test]
    fn a_configured_agent_that_has_not_been_asked_anything_says_so() {
        // Neither a pass nor a failure dressed as one. A restart makes this the
        // normal state, so it must not alarm -- but calling an untested
        // provider "answering" would be a claim nothing supports.
        let probe = health_with(
            r#"{"configured":true,"provider":"codex","tools":3,"spent_micro_usd":0,
                "last":{"last_call":"never"}}"#,
        );
        let check = agent(Some(&probe));
        assert_eq!(check.status, Status::Ok);
        assert!(check.detail.contains("nothing asked"), "{}", check.detail);
    }

    #[test]
    fn a_state_this_check_does_not_recognise_is_unknown_rather_than_a_pass() {
        // Added by somebody who did not update this file. Guessing which way a
        // new state goes is how a new failure gets reported as health.
        let probe = health_with(
            r#"{"configured":true,"provider":"codex","last":{"last_call":"rate_limited"}}"#,
        );
        assert_eq!(agent(Some(&probe)).status, Status::Unknown);

        // As is a health body with no agent block at all: an older binary than
        // this check, which is a different thing from a deliberately
        // unconfigured one and only one of them wants action.
        let old = ServingProbe::Answered {
            status: 200,
            body: r#"{"status":"ok"}"#.to_owned(),
        };
        let check = agent(Some(&old));
        assert_eq!(check.status, Status::Unknown);
        assert!(check.detail.contains("predates"), "{}", check.detail);
    }

    #[test]
    fn an_unreachable_server_does_not_also_report_the_agent_as_broken() {
        // Two lines reporting one outage is two people looking for two
        // problems. The serving check already alarms on this; the agent check
        // says it cannot see, which is true and is not a second diagnosis.
        for probe in [
            None,
            Some(ServingProbe::Unreachable("connection refused".to_owned())),
        ] {
            let check = agent(probe.as_ref());
            assert_eq!(check.status, Status::Unknown);
            assert!(check.detail.contains("cannot see"), "{}", check.detail);
        }
    }

    #[test]
    fn an_unconfigured_serving_endpoint_is_unknown_rather_than_healthy() {
        // The exact hole this check was added for. On 2026-08-25 the public
        // server was a hand-started process with no unit, and this command
        // printed "Nothing is out of bounds" because it never looked. Not
        // looking must alarm.
        let check = serving(None);
        assert_eq!(check.status, Status::Unknown);
        assert!(check.status.is_alarm(), "an unwatched surface must alarm");
    }

    #[test]
    fn a_healthy_serving_surface_passes() {
        // The other direction. Without this, the test above is also satisfied by
        // a check that alarms unconditionally.
        let check = serving(Some((
            "http://127.0.0.1:8402",
            ServingProbe::Answered {
                status: 200,
                body: r#"{"status":"ok","instruments":6,"paidSurface":true}"#.to_owned(),
            },
        )));
        assert_eq!(check.status, Status::Ok, "detail was {}", check.detail);
        assert!(!check.status.is_alarm());
        assert!(check.detail.contains("paid surface on"));
    }

    #[test]
    fn a_stopped_server_fails() {
        let check = serving(Some((
            "http://127.0.0.1:8402",
            ServingProbe::Unreachable("connection refused".to_owned()),
        )));
        assert_eq!(check.status, Status::Fail);
        assert!(check.status.is_alarm());
        assert!(check.detail.contains("connection refused"));
    }

    #[test]
    fn a_server_that_is_up_and_broken_is_not_a_pass() {
        // A 200 is not health. The endpoint reports its own status field, and a
        // check that stopped at the status code would pass a server that is
        // listening and answering wrongly.
        let degraded = serving(Some((
            "http://127.0.0.1:8402",
            ServingProbe::Answered {
                status: 200,
                body: r#"{"status":"degraded"}"#.to_owned(),
            },
        )));
        assert_eq!(degraded.status, Status::Fail, "{}", degraded.detail);

        // And something else entirely on the port is not radar-serve.
        let impostor = serving(Some((
            "http://127.0.0.1:8402",
            ServingProbe::Answered {
                status: 200,
                body: "<html>some other service</html>".to_owned(),
            },
        )));
        assert_eq!(impostor.status, Status::Fail, "{}", impostor.detail);
    }

    #[test]
    fn a_non_200_names_the_status_it_got() {
        // "answered wrongly" and "did not answer" are different failures with
        // different fixes, so the detail has to tell them apart.
        let check = serving(Some((
            "http://127.0.0.1:8402",
            ServingProbe::Answered {
                status: 502,
                body: String::new(),
            },
        )));
        assert_eq!(check.status, Status::Fail);
        assert!(check.detail.contains("502"), "detail was {}", check.detail);
    }

    #[test]
    fn durations_read_the_way_an_operator_reads_them() {
        assert_eq!(humanise(45), "45s");
        assert_eq!(humanise(600), "10m");
        assert_eq!(humanise(47_143), "13h05m");
    }
    #[test]
    fn an_analyst_that_never_ran_is_unknown_rather_than_fine() {
        // The failure LEARNINGS 5 records: a check reporting absence the same
        // way it reports success. This account's product is answering in
        // public, so a silent one is the outage -- and `Unknown` alarms.
        let dir = std::env::temp_dir().join(format!("radar-brief-none-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temp dir");
        let check = analyst(dir.to_str().expect("a path"));
        assert_eq!(check.status, Status::Unknown, "{}", check.detail);
        assert!(check.detail.contains("never run"), "{}", check.detail);
    }

    #[test]
    fn an_empty_reply_log_is_unknown_too() {
        let dir = std::env::temp_dir().join(format!("radar-brief-empty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temp dir");
        std::fs::write(dir.join("replies.jsonl"), "").expect("written");
        let check = analyst(dir.to_str().expect("a path"));
        assert_eq!(check.status, Status::Unknown, "{}", check.detail);
    }

    #[test]
    fn the_analyst_check_counts_replies_rather_than_log_lines() {
        // `publish` writes twice per reply -- once before it says anything and
        // once after -- so counting raw lines reports double, and an operator
        // reading "6 answered" when three people asked would be reading a
        // number that means nothing.
        let dir = std::env::temp_dir().join(format!("radar-brief-count-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temp dir");
        let log = dir.join("replies.jsonl");
        let log = log.to_str().expect("a path").to_owned();
        let _ = std::fs::remove_file(&log);

        for id in ["m1", "m2", "m3"] {
            let mut entry = radar_analyst::Entry {
                at: 1_788_000_000,
                mention_id: id.to_owned(),
                summoner: "a".to_owned(),
                mint: None,
                read_at_slot: None,
                fact_sheet: String::new(),
                reply: "text".to_owned(),
                fellback: None,
                reply_id: None,
            };
            // The intent, then the outcome, exactly as `publish` writes them.
            radar_analyst::log::append(&log, &entry).expect("intent");
            entry.reply_id = Some(format!("r-{id}"));
            radar_analyst::log::append(&log, &entry).expect("outcome");
        }

        let check = analyst(dir.to_str().expect("a path"));
        assert_eq!(check.status, Status::Ok, "{}", check.detail);
        assert!(
            check.detail.starts_with("3 answered, 3 published"),
            "six lines are three replies: {}",
            check.detail
        );
    }

    #[test]
    fn a_reply_that_was_never_published_is_counted_separately() {
        // The difference between "we decided this" and "we said this" is the
        // whole reason the log records both, and an operator needs to see when
        // the gap opens -- a publisher that is down all night answers nobody
        // while the log fills up.
        let dir = std::env::temp_dir().join(format!("radar-brief-dry-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temp dir");
        let log = dir.join("replies.jsonl");
        let log = log.to_str().expect("a path").to_owned();
        let _ = std::fs::remove_file(&log);

        let entry = radar_analyst::Entry {
            at: 1_788_000_000,
            mention_id: "m1".to_owned(),
            summoner: "a".to_owned(),
            mint: None,
            read_at_slot: None,
            fact_sheet: String::new(),
            reply: "text".to_owned(),
            fellback: Some("not published: no credential".to_owned()),
            reply_id: None,
        };
        radar_analyst::log::append(&log, &entry).expect("intent");
        radar_analyst::log::append(&log, &entry).expect("outcome");

        let check = analyst(dir.to_str().expect("a path"));
        assert!(
            check.detail.starts_with("1 answered, 0 published"),
            "{}",
            check.detail
        );
    }
}
