// SPDX-License-Identifier: Apache-2.0
//! Contest weeks: Monday 00:00 UTC to the next, numbered from the epoch.
//!
//! Design 0007 C1 fixes the boundary at Monday 00:00 UTC. A week is a plain
//! integer so that two machines reading the same clock agree on which week a
//! reply belongs to without a calendar library between them, and so that a
//! ledger file can be named by it.
//!
//! The epoch, 1970-01-01, was a Thursday. Week 0 is therefore the four-day
//! stub before the first Monday and is never a real contest week; every week
//! from 1 upward opens on a Monday. Nothing here needs a date before 1970.

use serde::{Deserialize, Serialize};

/// Seconds in a day.
pub const SECONDS_PER_DAY: u64 = 86_400;

/// Days in a week.
const DAYS_PER_WEEK: u64 = 7;

/// Days from the epoch's weekday to Monday, so that `(days + 3) / 7` counts
/// Monday-to-Sunday weeks. Thursday is three days before the following Monday
/// when counted the way this does.
const THURSDAY_TO_MONDAY: u64 = 3;

/// One contest week.
///
/// Ordered and hashable so a week can key a map and be compared with another,
/// and serialised as its number so a ledger names its file by it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Week(pub u64);

impl Week {
    /// The week a moment falls in.
    #[must_use]
    pub const fn of(secs: u64) -> Self {
        Self((secs / SECONDS_PER_DAY + THURSDAY_TO_MONDAY) / DAYS_PER_WEEK)
    }

    /// The first second of the week, Monday 00:00 UTC.
    ///
    /// Week 0 is the stub before the first Monday and opens at the epoch
    /// itself, which is the one place the arithmetic is clamped.
    #[must_use]
    pub const fn opens_at(self) -> u64 {
        (self.0 * DAYS_PER_WEEK).saturating_sub(THURSDAY_TO_MONDAY) * SECONDS_PER_DAY
    }

    /// The first second **after** the week: the next Monday 00:00 UTC.
    ///
    /// Exclusive, so that `opens_at() <= t < closes_at()` is the whole test.
    #[must_use]
    pub const fn closes_at(self) -> u64 {
        self.next().opens_at()
    }

    /// The week after this one.
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0 + 1)
    }

    /// Whether a moment falls inside the week.
    #[must_use]
    pub const fn contains(self, secs: u64) -> bool {
        secs >= self.opens_at() && secs < self.closes_at()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-09-07, a Monday, as days since the epoch. Cross-checked against
    /// `radar-cli`'s calendar test, which pins 2026-09-03 as day 20,699 and a
    /// Thursday.
    const MONDAY_2026_09_07: u64 = 20_703;

    #[test]
    fn a_week_opens_on_a_monday_at_midnight_utc_and_not_a_second_earlier() {
        // The boundary is the whole design decision in this file. Every
        // arithmetic mutation of `of` or `opens_at` moves it, and this pins it
        // on both sides of one known Monday.
        let monday = MONDAY_2026_09_07 * SECONDS_PER_DAY;
        let week = Week::of(monday);
        assert_eq!(week.opens_at(), monday, "Monday 00:00 opens its own week");
        assert_eq!(
            Week::of(monday - 1),
            Week(week.0 - 1),
            "23:59:59 Sunday is the week before"
        );
        assert_eq!(
            Week::of(monday + 6 * SECONDS_PER_DAY + 86_399),
            week,
            "Sunday 23:59:59 is still this week"
        );
        assert_eq!(
            Week::of(monday + 7 * SECONDS_PER_DAY),
            week.next(),
            "next Monday is the next week"
        );
    }

    #[test]
    fn a_week_is_exactly_seven_days_and_the_close_is_exclusive() {
        let week = Week::of(MONDAY_2026_09_07 * SECONDS_PER_DAY);
        assert_eq!(week.closes_at() - week.opens_at(), 7 * SECONDS_PER_DAY);
        assert!(week.contains(week.opens_at()));
        assert!(week.contains(week.closes_at() - 1));
        assert!(
            !week.contains(week.closes_at()),
            "the close belongs to the next week"
        );
        assert!(!week.contains(week.opens_at() - 1));
    }

    #[test]
    fn the_stub_week_before_the_first_monday_is_week_zero_and_opens_at_the_epoch() {
        // Never a contest week, and the one place the arithmetic clamps rather
        // than wrapping: without `saturating_sub` this underflows.
        assert_eq!(Week::of(0), Week(0));
        assert_eq!(Week(0).opens_at(), 0);
        // The first Monday, 1970-01-05, opens week 1.
        assert_eq!(Week(1).opens_at(), 4 * SECONDS_PER_DAY);
        assert_eq!(Week::of(4 * SECONDS_PER_DAY), Week(1));
        assert_eq!(Week::of(4 * SECONDS_PER_DAY - 1), Week(0));
    }

    #[test]
    fn a_week_serialises_as_its_number() {
        // The ledger file is named by it and the site reads it back; a struct
        // wrapper in the JSON would make both sides carry the type's shape.
        let json = serde_json::to_string(&Week(2958)).expect("serialises");
        assert_eq!(json, "2958");
        let back: Week = serde_json::from_str(&json).expect("round-trips");
        assert_eq!(back, Week(2958));
    }
}
