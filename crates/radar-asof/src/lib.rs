// SPDX-License-Identifier: Apache-2.0
//! Point-in-time correctness, enforced by the type system.
//!
//! Look-ahead bias is the failure that makes a backtest lie, and in this asset
//! class it is the difference between a strategy that looks profitable and one
//! that is. Research on crypto backtests puts the combined inflation from
//! survivorship and look-ahead at 17–22% annually, and memecoins are the worst
//! case: a token that died in ten minutes leaves no trace unless something
//! recorded it at the time.
//!
//! Discipline does not prevent this. A structure does. Every read in Radar is
//! gated by an [`AsOf`] watermark, and a value observed after that watermark
//! cannot be unwrapped — not "should not", cannot, because the only way to get
//! at the inner value is through [`AsOf::accept`], which checks.
//!
//! ```
//! use radar_asof::{AsOf, Observed};
//! use radar_types::Slot;
//!
//! let watermark = AsOf::at(Slot(1_000));
//!
//! // A fact from before the watermark is admissible.
//! assert!(watermark.accept(Observed::new("reserves", Slot(999))).is_ok());
//!
//! // A fact from after it is not, and there is no way around this short of
//! // constructing a different watermark.
//! assert!(watermark.accept(Observed::new("reserves", Slot(1_001))).is_err());
//! ```

#![forbid(unsafe_code)]

use core::fmt;

use radar_types::{Slot, SlotDelta};

/// A point-in-time watermark. Nothing observed after this slot may inform a
/// decision made at it.
///
/// Live mode constructs one from the current confirmed slot; research mode
/// constructs one from a historical slot. The code under it does not know or
/// care which, and that is the whole point: a replay runs the same instruments
/// over the same gate, so a divergence between a recorded live output and its
/// replay is a leak or a non-determinism bug, never an artefact of running in
/// "backtest mode".
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub struct AsOf(Slot);

/// A value together with the slot it was observed at.
///
/// The inner value is private. The only way to reach it is [`AsOf::accept`],
/// which is what makes the watermark unavoidable rather than advisory.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Observed<T> {
    value: T,
    at: Slot,
}

/// A value observed after the watermark was offered to a decision made at it.
#[derive(Debug, thiserror::Error, PartialEq, Eq, Clone, Copy)]
#[error(
    "look-ahead: value observed at slot {observed} offered to a decision as of slot {watermark} ({ahead} ahead)"
)]
pub struct LookAhead {
    /// The watermark the decision is being made at.
    pub watermark: Slot,
    /// The slot the value was actually observed at.
    pub observed: Slot,
    /// How far past the watermark the observation is.
    pub ahead: SlotDelta,
}

impl AsOf {
    /// A watermark at a given slot.
    #[must_use]
    pub const fn at(slot: Slot) -> Self {
        Self(slot)
    }

    /// The slot this watermark stands at.
    #[must_use]
    pub const fn slot(self) -> Slot {
        self.0
    }

    /// Whether a value observed at `observed` is admissible here.
    #[must_use]
    pub const fn admits(self, observed: Slot) -> bool {
        observed.get() <= self.0.get()
    }

    /// Unwraps an observation if it happened at or before this watermark.
    ///
    /// # Errors
    ///
    /// Returns [`LookAhead`] if the observation is from the future relative to
    /// this watermark. That is always a bug — either a store returned data past
    /// the watermark it was given, or a caller mixed watermarks between stages.
    pub fn accept<T>(self, observed: Observed<T>) -> Result<T, LookAhead> {
        if self.admits(observed.at) {
            Ok(observed.value)
        } else {
            Err(LookAhead {
                watermark: self.0,
                observed: observed.at,
                ahead: observed.at.saturating_since(self.0),
            })
        }
    }

    /// How stale a value observed at `observed` is relative to this watermark.
    ///
    /// Zero for a value from the future, which callers must reject via
    /// [`accept`](Self::accept) rather than reason about.
    #[must_use]
    pub const fn staleness(self, observed: Slot) -> SlotDelta {
        self.0.saturating_since(observed)
    }

    /// A watermark rolled back by `delta` slots.
    ///
    /// For deliberately evaluating an instrument as it would have looked
    /// earlier — the mechanism behind "what would this have decided an hour ago".
    #[must_use]
    pub fn rewound(self, delta: SlotDelta) -> Self {
        Self(self.0 - delta)
    }
}

impl<T> Observed<T> {
    /// Tags a value with the slot it was observed at.
    pub const fn new(value: T, at: Slot) -> Self {
        Self { value, at }
    }

    /// The slot this value was observed at. Readable without unwrapping, so a
    /// caller can sort or compare observations before deciding to admit one.
    pub const fn observed_at(&self) -> Slot {
        self.at
    }

    /// Transforms the value, preserving the observation slot.
    ///
    /// Deriving a new fact from an observed one cannot make it fresher, so the
    /// slot is carried rather than re-taken.
    pub fn map<U, F: FnOnce(T) -> U>(self, f: F) -> Observed<U> {
        Observed {
            value: f(self.value),
            at: self.at,
        }
    }
}

/// A store that can answer questions as of a given watermark.
///
/// Implementations must never return data observed after `as_of`. Returning an
/// [`Observed`] rather than a bare value is what lets the caller verify that
/// rather than trust it.
pub trait PointInTime {
    /// What went wrong reading the store.
    type Error;

    /// The most recent slot this store holds data for. A watermark beyond this
    /// means the store cannot answer, which is different from answering "none".
    ///
    /// # Errors
    ///
    /// Returns `Self::Error` if the store cannot be read.
    fn watermark(&self) -> Result<Slot, Self::Error>;

    /// Whether this store can answer as of the given watermark at all.
    ///
    /// # Errors
    ///
    /// Returns `Self::Error` if the store cannot be read.
    fn can_answer(&self, as_of: AsOf) -> Result<bool, Self::Error> {
        Ok(self.watermark()? >= as_of.slot())
    }
}

impl fmt::Display for AsOf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "as of slot {}", self.0)
    }
}

impl fmt::Debug for AsOf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AsOf({})", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_from_the_past_is_admitted() {
        let w = AsOf::at(Slot(1_000));
        assert_eq!(w.accept(Observed::new(42, Slot(999))), Ok(42));
    }

    #[test]
    fn a_value_from_the_same_slot_is_admitted() {
        // The watermark is inclusive: a decision made at slot N may use
        // everything that happened in slot N, which is what "as of" means.
        let w = AsOf::at(Slot(1_000));
        assert_eq!(w.accept(Observed::new(42, Slot(1_000))), Ok(42));
    }

    #[test]
    fn a_value_from_the_future_is_refused_with_the_distance() {
        let w = AsOf::at(Slot(1_000));
        let err = w
            .accept(Observed::new(42, Slot(1_005)))
            .expect_err("must refuse");
        assert_eq!(err.watermark, Slot(1_000));
        assert_eq!(err.observed, Slot(1_005));
        assert_eq!(err.ahead, SlotDelta(5));
    }

    #[test]
    fn mapping_does_not_make_a_value_fresher() {
        // Deriving a conclusion from stale inputs yields a stale conclusion. If
        // map re-stamped the slot, every derived feature would launder its own
        // staleness away.
        let o = Observed::new(2, Slot(500)).map(|v| v * 10);
        assert_eq!(o.observed_at(), Slot(500));
        assert!(AsOf::at(Slot(499)).accept(o).is_err());
    }

    #[test]
    fn staleness_is_measured_from_the_watermark() {
        let w = AsOf::at(Slot(1_000));
        assert_eq!(w.staleness(Slot(850)), SlotDelta(150));
        assert_eq!(w.staleness(Slot(1_000)), SlotDelta(0));
    }

    #[test]
    fn rewinding_narrows_what_is_admissible() {
        let w = AsOf::at(Slot(1_000));
        let earlier = w.rewound(SlotDelta(100));
        assert_eq!(earlier.slot(), Slot(900));
        assert!(w.admits(Slot(950)));
        assert!(!earlier.admits(Slot(950)));
    }

    struct FakeStore(Slot);
    impl PointInTime for FakeStore {
        type Error = core::convert::Infallible;
        fn watermark(&self) -> Result<Slot, Self::Error> {
            Ok(self.0)
        }
    }

    #[test]
    fn a_store_cannot_answer_past_its_own_watermark() {
        // Asking a store for slot 2000 when it has only ingested to 1000 must be
        // "I cannot answer", never an empty result that reads as "nothing there".
        let store = FakeStore(Slot(1_000));
        assert_eq!(store.can_answer(AsOf::at(Slot(999))), Ok(true));
        assert_eq!(store.can_answer(AsOf::at(Slot(1_000))), Ok(true));
        assert_eq!(store.can_answer(AsOf::at(Slot(1_001))), Ok(false));
    }
}
