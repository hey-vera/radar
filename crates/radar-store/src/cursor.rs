// SPDX-License-Identifier: Apache-2.0
//! The follow cursor: how far the recorder has got.
//!
//! It lives here rather than in the recorder because it is **part of the store's
//! on-disk layout**, and two processes need it for opposite reasons. The recorder
//! writes it to know where to resume. Everything else reads it to answer the only
//! question that matters about a recorder — *is it still recording?*
//!
//! Keeping the path and the format in one place is not tidiness. A second copy
//! of "the cursor is a `YYYY-MM-DD HH:MM:SS` string in `.follow-cursor`" is a
//! second thing that can drift, and the failure it would produce is a monitor
//! that cannot read the cursor reporting that ingestion is fine.
//!
//! # Why progress, not liveness
//!
//! On 2026-08-24 the recorder failed twice in one day: first it exited and
//! stayed dead, then — after that was fixed — it stalled for twenty minutes on a
//! window it could not fetch. A liveness check would have caught the first and
//! passed the second, while the store was equally not growing either way.
//!
//! Cursor age catches both, because it measures the thing that is actually
//! wanted. A process that is running and not advancing is an outage.

use std::path::Path;

/// Where the follow cursor lives inside the store.
pub const CURSOR_FILE: &str = ".follow-cursor";

/// Reads the follow cursor, as seconds since the epoch.
///
/// `None` when the file is absent or unreadable — which is the state of a store
/// that has never been followed, and is deliberately not reported as a cursor of
/// zero. A cursor at the epoch would render as fifty-six years of ingestion lag
/// and read as a catastrophe rather than as an empty store.
#[must_use]
pub fn read_cursor(store: &Path) -> Option<i64> {
    let raw = std::fs::read_to_string(store.join(CURSOR_FILE)).ok()?;
    to_epoch(raw.trim()).ok()
}

/// Writes the follow cursor.
///
/// # Errors
///
/// Returns the underlying message if the store directory or the file cannot be
/// written.
pub fn write_cursor(store: &Path, at: i64) -> Result<(), String> {
    std::fs::create_dir_all(store).map_err(|e| e.to_string())?;
    std::fs::write(store.join(CURSOR_FILE), from_epoch(at)).map_err(|e| e.to_string())
}

/// Seconds since the epoch for a `YYYY-MM-DD HH:MM:SS` timestamp, treated as UTC.
///
/// A hand-rolled conversion rather than a date crate: this is the only date
/// arithmetic in the workspace, and it is not worth a dependency that would then
/// be in the tree of every process that links the store.
///
/// # Errors
///
/// Returns a message naming what was expected and what arrived.
pub fn to_epoch(stamp: &str) -> Result<i64, String> {
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

/// Renders seconds since the epoch as `YYYY-MM-DD HH:MM:SS`, in UTC.
#[must_use]
pub fn from_epoch(mut secs: i64) -> String {
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

/// Wall-clock now, as seconds since the epoch.
///
/// The one clock read in the reading path, and it is confined to operational
/// reporting. Nothing that produces a decision may call this — a decision's sense
/// of time is its watermark, which is why [`AsOf`](radar_asof::AsOf) exists.
#[must_use]
pub fn now_epoch() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    )
    .unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_round_trip() {
        for s in [
            "2026-08-21 06:00:00",
            "2026-01-01 00:00:00",
            "2024-02-29 23:59:59",
            "1970-01-01 00:00:00",
        ] {
            assert_eq!(from_epoch(to_epoch(s).expect(s)), s, "{s}");
        }
    }

    #[test]
    fn a_malformed_stamp_says_what_it_wanted() {
        // The cursor is read by a monitor whose whole job is to be believed. A
        // parse failure that reported "0" would render as fifty-six years of lag.
        assert!(to_epoch("not a timestamp").is_err());
        assert!(to_epoch("2026-08-21").is_err());
        assert!(to_epoch("2026-08-21 06:00").is_err());
    }

    #[test]
    fn a_store_that_was_never_followed_has_no_cursor_rather_than_a_zero_one() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(read_cursor(dir.path()), None);
    }

    #[test]
    fn a_written_cursor_reads_back_identically() {
        let dir = tempfile::tempdir().expect("tempdir");
        let at = to_epoch("2026-08-24 20:11:52").expect("parse");
        write_cursor(dir.path(), at).expect("write");
        assert_eq!(read_cursor(dir.path()), Some(at));
    }
}
