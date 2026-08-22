// SPDX-License-Identifier: Apache-2.0
//! Slots: the only clock Radar has.

use core::fmt;
use core::ops::{Add, Sub};
use core::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A Solana slot number.
///
/// Radar makes no decision on wall-clock time. A slot is the chain's own ordering
/// of events, it is what a replay can be pinned to, and it is the thing a data
/// provider can be held to when it claims a response is current. Wall-clock time
/// appears only in logs and in human-facing rendering.
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema, Default,
)]
#[serde(transparent)]
pub struct Slot(pub u64);

/// A number of slots, used for freshness budgets and expiry windows.
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema, Default,
)]
#[serde(transparent)]
pub struct SlotDelta(pub u64);

impl Slot {
    /// Slots elapsed since `earlier`, saturating at zero if `earlier` is later.
    #[must_use]
    pub const fn saturating_since(self, earlier: Self) -> SlotDelta {
        SlotDelta(self.0.saturating_sub(earlier.0))
    }

    /// The raw slot number.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl SlotDelta {
    /// One slot is roughly 400ms of wall-clock time, and "roughly" is doing real
    /// work: slot times vary with network conditions, and Alpenglow changes the
    /// relationship again. Use this for rendering and for coarse budgeting only —
    /// never to decide whether data is stale, which is what slot comparison is for.
    #[must_use]
    pub const fn approx_duration(self) -> Duration {
        Duration::from_millis(self.0.saturating_mul(400))
    }

    /// The number of slots.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Roughly this many minutes of slots, at 150 slots per minute.
    #[must_use]
    pub const fn approx_minutes(minutes: u64) -> Self {
        Self(minutes.saturating_mul(150))
    }
}

impl Add<SlotDelta> for Slot {
    type Output = Self;
    fn add(self, d: SlotDelta) -> Self {
        Self(self.0.saturating_add(d.0))
    }
}

impl Sub<SlotDelta> for Slot {
    type Output = Self;
    fn sub(self, d: SlotDelta) -> Self {
        Self(self.0.saturating_sub(d.0))
    }
}

impl fmt::Display for Slot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Debug for Slot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Slot({})", self.0)
    }
}

impl fmt::Display for SlotDelta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} slots", self.0)
    }
}

impl fmt::Debug for SlotDelta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SlotDelta({})", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_arithmetic_saturates_rather_than_wrapping() {
        // A wrapped slot is a silently valid-looking value pointing at the wrong
        // era of the chain. Saturating is wrong too, but visibly so.
        assert_eq!(Slot(5) - SlotDelta(10), Slot(0));
        assert_eq!(Slot(u64::MAX) + SlotDelta(10), Slot(u64::MAX));
    }

    #[test]
    fn saturating_since_does_not_report_negative_age() {
        assert_eq!(Slot(100).saturating_since(Slot(40)), SlotDelta(60));
        assert_eq!(Slot(40).saturating_since(Slot(100)), SlotDelta(0));
    }

    #[test]
    fn slot_serialises_as_a_bare_number() {
        assert_eq!(serde_json::to_string(&Slot(42)).expect("serialize"), "42");
    }
}
