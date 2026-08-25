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
    checks.push(serving(serve_url.map(|url| (url, probe_serving(url)))));
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
    Check::new(
        Status::Ok,
        "trading",
        format!(
            "autonomy {:?}, max position {} — no proposal can become an authorization",
            radar_risk::Policy::CLOSED.autonomy,
            radar_risk::Policy::CLOSED.max_position
        ),
    )
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
}
