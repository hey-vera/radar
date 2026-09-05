// SPDX-License-Identifier: Apache-2.0
//! Civil dates from day counts, defined once.
//!
//! Howard Hinnant's civil-from-days algorithm. It lived in `radar-cli`'s roast
//! command, where a test module once held a *second copy* and tested that
//! instead of the real one -- twenty-four arithmetic mutants survived until the
//! copy was deleted (LEARNINGS 18's shape, applied to a test). The public
//! endpoints need the same function for a contest week's Monday, so it moves
//! here and both callers share one definition. `radar-backfill` carries its
//! own pair for parsing timestamps; that is a known second copy, recorded
//! rather than folded in, because its tests pin the inverse direction too.
//!
//! No clock anywhere in this module. Every function takes the day and returns
//! the text, so it is checkable at a fixed day.

/// Seconds in a day.
const SECONDS_PER_DAY: u64 = 86_400;

/// A `YYYY-MM-DD` date from a count of days since 1970-01-01.
///
/// Negative days are before the epoch and are handled, which is why the
/// arithmetic is Euclidean rather than truncating.
#[must_use]
pub fn date_from_days(days: i64) -> String {
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

/// A `YYYY-MM-DDTHH:MM:SSZ` timestamp from seconds since the epoch.
///
/// UTC, always: every clock in this repository is UTC and a published
/// document that carried a zone would invite a reader to convert it twice.
#[must_use]
pub fn timestamp_from_seconds(secs: u64) -> String {
    let days = i64::try_from(secs / SECONDS_PER_DAY).unwrap_or(i64::MAX);
    let rest = secs % SECONDS_PER_DAY;
    let (h, m, s) = (rest / 3600, (rest % 3600) / 60, rest % 60);
    format!("{}T{h:02}:{m:02}:{s:02}Z", date_from_days(days))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_calendar_is_right_at_every_boundary_the_algorithm_has() {
        // Chosen so that each constant and each branch changes an answer if it
        // moves; the same set `radar-cli` pinned when its copy was deleted.
        assert_eq!(date_from_days(0), "1970-01-01");
        assert_eq!(date_from_days(1), "1970-01-02");
        assert_eq!(date_from_days(31), "1970-02-01");
        // The March pivot: consecutive days that take opposite branches.
        assert_eq!(date_from_days(58), "1970-02-28");
        assert_eq!(date_from_days(59), "1970-03-01");
        // A leap day, and the day either side.
        assert_eq!(date_from_days(19_781), "2024-02-28");
        assert_eq!(date_from_days(19_782), "2024-02-29");
        assert_eq!(date_from_days(19_783), "2024-03-01");
        // 2000 is a leap year (divisible by 400); 2100 is not.
        assert_eq!(date_from_days(11_016), "2000-02-29");
        assert_eq!(date_from_days(47_540), "2100-02-28");
        assert_eq!(date_from_days(47_541), "2100-03-01");
        // Year end.
        assert_eq!(date_from_days(19_722), "2023-12-31");
        assert_eq!(date_from_days(19_723), "2024-01-01");
        // Before the epoch: the reason for `div_euclid` and `rem_euclid`.
        assert_eq!(date_from_days(-1), "1969-12-31");
        assert_eq!(date_from_days(-365), "1969-01-01");
        // The two days the rest of the repository cross-checks against: a
        // Thursday, and the Monday that opens contest week 2958.
        assert_eq!(date_from_days(20_699), "2026-09-03");
        assert_eq!(date_from_days(20_703), "2026-09-07");
    }

    #[test]
    fn a_timestamp_carries_the_time_of_day_in_utc() {
        assert_eq!(timestamp_from_seconds(0), "1970-01-01T00:00:00Z");
        assert_eq!(timestamp_from_seconds(86_399), "1970-01-01T23:59:59Z");
        assert_eq!(timestamp_from_seconds(86_400), "1970-01-02T00:00:00Z");
        // 2026-09-04T23:55:00Z, the moment the site's fixture was measured:
        // day 20,700 plus 23:55.
        assert_eq!(
            timestamp_from_seconds(20_700 * 86_400 + 23 * 3600 + 55 * 60),
            "2026-09-04T23:55:00Z"
        );
    }
}
