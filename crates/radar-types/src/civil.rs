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

/// Days since 1970-01-01 for a civil date: the inverse of [`date_from_days`].
///
/// Hinnant's days-from-civil. Needed because the platform reports when an
/// account was created as a timestamp, and the contest's age rule is in days.
#[must_use]
pub fn days_from_date(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = i64::from(month);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Seconds since the epoch for a `YYYY-MM-DDTHH:MM:SS[.fff]Z` timestamp, or
/// `None` when the text is not that shape.
///
/// UTC only, `Z` required: a zoned timestamp is refused rather than read as
/// UTC, because a wrong zone is a wrong age that looks right.
#[must_use]
pub fn seconds_from_timestamp(text: &str) -> Option<u64> {
    let text = text.strip_suffix('Z')?;
    let (date, time) = text.split_once('T')?;
    let mut date = date.split('-');
    let year: i64 = date.next()?.parse().ok()?;
    let month: u32 = date.next()?.parse().ok()?;
    let day: u32 = date.next()?.parse().ok()?;
    if date.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let time = time.split_once('.').map_or(time, |(whole, _)| whole);
    let mut time = time.split(':');
    let h: u64 = time.next()?.parse().ok()?;
    let m: u64 = time.next()?.parse().ok()?;
    let s: u64 = time.next()?.parse().ok()?;
    if time.next().is_some() || h > 23 || m > 59 || s > 60 {
        return None;
    }
    let days = days_from_date(year, month, day);
    let secs = days.checked_mul(i64::try_from(SECONDS_PER_DAY).ok()?)?;
    u64::try_from(secs).ok()?.checked_add(h * 3600 + m * 60 + s)
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
    fn the_inverse_round_trips_every_boundary_and_a_timestamp_reads_back() {
        // Each day the forward direction pins, back through the inverse. A
        // constant wrong in one direction only would break the round trip.
        for days in [
            0i64, 1, 31, 58, 59, 19_781, 19_782, 19_783, -1, -365, 20_697,
        ] {
            let text = date_from_days(days);
            let mut parts = text.split('-');
            let y: i64 = parts.next().expect("y").parse().expect("y");
            let m: u32 = parts.next().expect("m").parse().expect("m");
            let d: u32 = parts.next().expect("d").parse().expect("d");
            assert_eq!(days_from_date(y, m, d), days, "{text}");
        }
        assert_eq!(days_from_date(1970, 1, 1), 0);
        assert_eq!(days_from_date(2024, 2, 29), 19_782);

        // The platform's shape, with and without milliseconds.
        assert_eq!(seconds_from_timestamp("1970-01-01T00:00:01Z"), Some(1));
        assert_eq!(
            seconds_from_timestamp("2024-02-29T12:34:56.000Z"),
            Some(19_782 * 86_400 + 12 * 3600 + 34 * 60 + 56)
        );
        assert_eq!(
            timestamp_from_seconds(seconds_from_timestamp("2026-09-05T08:00:00Z").expect("secs")),
            "2026-09-05T08:00:00Z"
        );
        // Refused, not guessed: a zone, a missing Z, a month out of range,
        // and a date before the epoch (an account cannot predate it).
        for bad in [
            "2024-02-29T12:34:56+01:00",
            "2024-02-29T12:34:56",
            "2024-13-01T00:00:00Z",
            "2024-02-29",
            "1969-12-31T23:59:59Z",
            "",
            // Each of the three time bounds, one past it. CI's mutants moved
            // every one of them (`>` to `>=`, `>` to `==`, `||` to `&&`) with
            // nothing failing, because nothing had tried an hour 24.
            "2024-01-01T24:00:00Z",
            "2024-01-01T12:60:00Z",
            "2024-01-01T12:00:61Z",
        ] {
            assert_eq!(seconds_from_timestamp(bad), None, "{bad:?}");
        }
        // The bounds themselves are inside: hour 23, minute 59, and second 60,
        // which is a leap second and is a time the platform can stamp.
        assert_eq!(
            seconds_from_timestamp("2024-01-01T23:59:60Z"),
            Some(19_723 * 86_400 + 23 * 3600 + 59 * 60 + 60)
        );
    }

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
