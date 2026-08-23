// SPDX-License-Identifier: Apache-2.0
//! Deciding which tokens still need measuring.
//!
//! Measuring every token on every pass does not scale: at ~35,000 launches a day
//! an hourly pass would re-measure a month's tokens a million times over, almost
//! all of them long settled.
//!
//! But measuring once is worse than not scaling — it is wrong. A token measured
//! an hour after launch and the same token measured a day later are different
//! observations, and the second is the one that says whether the first meant
//! anything. Measuring once would record only the first.
//!
//! So: **checkpoints**. Each token is measured as it crosses a fixed set of ages
//! and then never again. Work per pass is bounded by launch rate rather than by
//! accumulated history, and the dataset still holds the "one hour in, one day in"
//! pair that makes an early signal checkable against a later outcome.

use radar_types::{Slot, SlotDelta};

/// Ages at which a token is measured, in slots after launch.
///
/// Roughly one hour, six hours and a day, at ~2.5 slots a second. Three
/// measurements per token, not one per pass.
///
/// The last one settles it. A token that has been dead for a day is dead, and a
/// token still trading after a day is not going to be reclassified by a fourth
/// look — but it *would* keep costing queries forever if nothing declared it
/// finished.
pub const CHECKPOINTS: &[SlotDelta] = &[SlotDelta(9_000), SlotDelta(54_000), SlotDelta(216_000)];

/// The largest checkpoint. Past this, a token is settled.
#[must_use]
pub fn settled_after() -> SlotDelta {
    CHECKPOINTS.last().copied().unwrap_or(SlotDelta(0))
}

/// Whether a token needs measuring, given how old it is and how mature its most
/// recent measurement was.
///
/// `age_at_last_measurement` is `None` when it has never been measured.
#[must_use]
pub fn needs_measuring(current_age: SlotDelta, age_at_last_measurement: Option<SlotDelta>) -> bool {
    // Never measured: worth a look as soon as it clears the first checkpoint.
    // Measuring in the first minutes would record "no activity yet" as an
    // outcome, which is a statement about the clock rather than the token.
    let Some(previous) = age_at_last_measurement else {
        return current_age >= CHECKPOINTS[0];
    };

    if previous >= settled_after() {
        return false;
    }

    // Due if the token has crossed a checkpoint that the last measurement was
    // taken before.
    CHECKPOINTS
        .iter()
        .any(|c| current_age >= *c && previous < *c)
}

/// How old a token is at a given slot.
#[must_use]
pub fn age_of(launch_slot: Slot, at: Slot) -> SlotDelta {
    at.saturating_since(launch_slot)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: SlotDelta = SlotDelta(9_000);
    const SIX_HOURS: SlotDelta = SlotDelta(54_000);
    const DAY: SlotDelta = SlotDelta(216_000);

    #[test]
    fn a_brand_new_token_is_left_alone() {
        // Measuring a token minutes old records "nothing has happened yet",
        // which describes the clock rather than the token.
        assert!(!needs_measuring(SlotDelta(10), None));
        assert!(!needs_measuring(SlotDelta(8_999), None));
    }

    #[test]
    fn an_unmeasured_token_is_due_once_it_clears_the_first_checkpoint() {
        assert!(needs_measuring(HOUR, None));
        assert!(needs_measuring(DAY, None), "and long past it");
    }

    #[test]
    fn each_checkpoint_earns_exactly_one_more_measurement() {
        // Measured at an hour: not due again until six hours.
        assert!(!needs_measuring(SlotDelta(20_000), Some(HOUR)));
        assert!(needs_measuring(SIX_HOURS, Some(HOUR)));

        // Measured at six hours: not due again until a day.
        assert!(!needs_measuring(SlotDelta(100_000), Some(SIX_HOURS)));
        assert!(needs_measuring(DAY, Some(SIX_HOURS)));
    }

    #[test]
    fn a_settled_token_is_never_measured_again() {
        // The reason work per pass is bounded by launch rate rather than by
        // accumulated history. Without this the cost grows forever.
        assert!(!needs_measuring(DAY, Some(DAY)));
        assert!(!needs_measuring(SlotDelta(50_000_000), Some(DAY)));
        assert!(!needs_measuring(
            SlotDelta(50_000_000),
            Some(SlotDelta(300_000))
        ));
    }

    #[test]
    fn a_token_that_skipped_a_checkpoint_is_still_measured_once() {
        // A pass that did not run for a day must not queue three measurements
        // for the same token; one look now covers every checkpoint it crossed.
        assert!(needs_measuring(DAY, None));
        assert!(needs_measuring(DAY, Some(SlotDelta(100))));
    }

    #[test]
    fn a_full_days_launches_settle_in_three_measurements() {
        // The scaling claim, checked rather than asserted in prose: one token
        // followed from launch to settled costs three measurements, not one per
        // pass forever.
        let mut age = SlotDelta(0);
        let mut last: Option<SlotDelta> = None;
        let mut measurements = 0;

        // Step through two days of chain in ten-minute increments.
        while age.get() < 432_000 {
            if needs_measuring(age, last) {
                last = Some(age);
                measurements += 1;
            }
            age = SlotDelta(age.get() + 1_500);
        }
        assert_eq!(measurements, CHECKPOINTS.len());
    }

    #[test]
    fn age_is_measured_from_launch() {
        assert_eq!(age_of(Slot(1_000), Slot(10_000)), SlotDelta(9_000));
        // A head behind the launch slot means zero age, not a wrapped number.
        assert_eq!(age_of(Slot(10_000), Slot(1_000)), SlotDelta(0));
    }
}
