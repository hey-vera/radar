// SPDX-License-Identifier: Apache-2.0
//! One computed answer, kept only for the watermark it was computed at.
//!
//! `/v1/scoreboard` and `/v1/tokens/{mint}` each scan the whole store — 6,185
//! decisions against 1,147,649 outcomes when this was measured — and the
//! interface's budget is p95 under 500ms. They were 1.7s and 3.2s.
//!
//! The work is identical for every caller asking the same question at the same
//! watermark, and the store only changes when the recorder writes. So it is
//! computed once per watermark.
//!
//! # The key is a type, not a convention
//!
//! `/v1/scoreboard` is answered by the watermark alone; `/v1/tokens/{mint}` needs
//! the mint too. An earlier shape here keyed on the watermark only and left the
//! mint for the handler to compare -- and the handler got it wrong, falling
//! through to a lookup that matched on the watermark and returned another
//! token's evidence. Serving one mint's evidence for another is a far worse bug
//! than a slow response, so the key is part of this type and no caller can
//! forget to check it.
//!
//! # Rule 3 is the whole design
//!
//! **A value computed at one watermark is never served for another.** The
//! watermark is part of the key, not metadata beside it, so there is no path
//! that returns a cached answer to a caller who asked a different question.
//!
//! That matters more here than ordinary cache correctness. Rule 3 says a replay
//! must not be served a live-populated entry from the future — and a cache keyed
//! only on the *mint*, say, would do precisely that: a research replay at an old
//! `AsOf` would receive today's answer and see data that did not exist yet. The
//! look-ahead would be invisible, and it would be in the direction that flatters
//! a result.
//!
//! # Exact is not enough on a store that is always moving
//!
//! Keyed exactly on the watermark, this cache missed about half the time in
//! production. The recorder advances the watermark roughly every fifty seconds,
//! so an answer is only reusable inside one flush window, and measured on the
//! box a miss costs 4.4 to 5.2 seconds against a 500ms budget:
//!
//! ```text
//! 0.148s  4.790s  0.216s  0.213s  4.386s  0.181s  5.193s  0.175s
//! ```
//!
//! So [`Cache::recent`] will offer an answer computed at a slightly older
//! watermark, and the caller serves it **labelled with the watermark it was
//! actually computed at**. That is staleness, not look-ahead: rule 3 forbids
//! reading *past* a watermark, and this reads further behind one. An answer
//! about 7,543 historical decisions does not change in a minute, and outcomes
//! are measured hourly.
//!
//! The labelling is the part that makes it honest, and it is not optional. An
//! older answer served as though it were current is rule 9's shape — unknown
//! rendered as fresh — so the response carries its own `as_of` and the caller
//! has no way to serve one without it.
//!
//! # One entry, not a map
//!
//! Almost every request asks about the current watermark. A map would hold every
//! historical answer a replay ever asked for, which is unbounded and reachable
//! by anyone who can vary `AsOf`. A single entry serves the common case and
//! costs a recomputation for the rare one.
//!
//! # Errors are not cached
//!
//! A failed read is a fact about a moment, not about the watermark. Caching one
//! would make a transient store error permanent until the recorder next wrote.

use std::sync::{Arc, Mutex};

use radar_types::Slot;

/// The most recent answer, the watermark it was computed at, and what was asked.
///
/// `K` is the rest of the question. `()` when the watermark is the whole of it,
/// as it is for the scoreboard.
#[derive(Debug)]
pub struct Cache<T, K = ()> {
    entry: Mutex<Option<(Slot, K, Arc<T>)>>,
}

impl<T, K> Default for Cache<T, K> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, K> Cache<T, K> {
    /// An empty cache.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entry: Mutex::new(None),
        }
    }
}

impl<T, K: PartialEq> Cache<T, K> {
    /// The value for `at`, computing it if this cache does not already hold it.
    ///
    /// `compute` runs **outside the lock**. Two callers arriving together on a
    /// cold cache will both compute, which wastes one scan; holding the lock
    /// across a three-second read would instead block every other request on the
    /// process for three seconds. Duplicated work is the cheaper mistake.
    ///
    /// # Errors
    ///
    /// Whatever `compute` returns. A failure is not stored.
    pub fn get_or_compute<E>(
        &self,
        at: Slot,
        key: K,
        compute: impl FnOnce() -> Result<T, E>,
    ) -> Result<Arc<T>, E> {
        if let Some(hit) = self.peek(at, &key) {
            return Ok(hit);
        }
        let value = Arc::new(compute()?);
        let mut entry = self
            .entry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Overwrites whatever is there, including a *newer* watermark computed
        // by a racing caller. Both are correct answers to their own questions,
        // and keeping the newer one would need a comparison that buys nothing:
        // the next request at the newer watermark simply recomputes.
        *entry = Some((at, key, Arc::clone(&value)));
        Ok(value)
    }

    /// The cached value if it answers this question at a watermark no more than
    /// `max_lag` slots behind `at`.
    ///
    /// Returns the watermark it was computed at as well, because a caller that
    /// cannot say how old the answer is must not serve it. That is why this
    /// returns a pair rather than a value: the label is not an extra the caller
    /// may forget.
    ///
    /// A cached entry from the *future* is never offered. It cannot arise from
    /// this endpoint, whose watermark only advances, but a saturating
    /// subtraction would silently treat one as fresh rather than as the bug it
    /// would be.
    #[must_use]
    pub fn recent(&self, at: Slot, key: &K, max_lag: u64) -> Option<(Slot, Arc<T>)> {
        let entry = self
            .entry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        entry
            .as_ref()
            .filter(|(cached, asked, _)| {
                asked == key && cached.get() <= at.get() && at.get() - cached.get() <= max_lag
            })
            .map(|(cached, _, value)| (*cached, Arc::clone(value)))
    }

    /// The cached value, if it answers exactly this question at exactly this
    /// watermark.
    #[must_use]
    pub fn peek(&self, at: Slot, key: &K) -> Option<Arc<T>> {
        let entry = self
            .entry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        entry
            .as_ref()
            .filter(|(cached, asked, _)| *cached == at && asked == key)
            .map(|(_, _, value)| Arc::clone(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Counts how often the expensive thing actually ran.
    fn counting(count: &AtomicUsize, value: u64) -> impl FnOnce() -> Result<u64, ()> + '_ {
        move || {
            count.fetch_add(1, Ordering::SeqCst);
            Ok(value)
        }
    }

    #[test]
    fn a_second_ask_at_the_same_watermark_does_not_recompute() {
        let cache = Cache::new();
        let runs = AtomicUsize::new(0);

        let first = cache
            .get_or_compute(Slot(100), (), counting(&runs, 7))
            .expect("ok");
        let second = cache
            .get_or_compute(Slot(100), (), counting(&runs, 9))
            .expect("ok");

        assert_eq!(*first, 7);
        assert_eq!(*second, 7, "the cached answer, not the new closure's");
        assert_eq!(runs.load(Ordering::SeqCst), 1, "computed once");
    }

    #[test]
    fn a_different_watermark_never_receives_the_cached_answer() {
        // Rule 3, and the reason this type exists rather than a plain memo. A
        // research replay at an older `AsOf` must not be handed today's answer:
        // it would see data that did not exist yet, invisibly, and in the
        // direction that flatters a result.
        let cache = Cache::new();
        let runs = AtomicUsize::new(0);

        let now = cache
            .get_or_compute(Slot(200), (), counting(&runs, 7))
            .expect("ok");
        let replay = cache
            .get_or_compute(Slot(100), (), counting(&runs, 3))
            .expect("ok");

        assert_eq!(*now, 7);
        assert_eq!(*replay, 3, "the replay's own answer");
        assert_eq!(runs.load(Ordering::SeqCst), 2, "both computed");
    }

    #[test]
    fn a_watermark_one_slot_away_is_a_miss() {
        // Swept rather than sampled: an equality that had become a `>=` would
        // serve a stale answer to every later watermark, which is exactly the
        // look-ahead above with the sign flipped.
        let cache = Cache::new();
        let runs = AtomicUsize::new(0);
        cache
            .get_or_compute(Slot(100), (), counting(&runs, 1))
            .expect("ok");

        assert!(cache.peek(Slot(99), &()).is_none(), "one before");
        assert!(cache.peek(Slot(101), &()).is_none(), "one after");
        assert_eq!(*cache.peek(Slot(100), &()).expect("exact"), 1);
    }

    #[test]
    fn a_failure_is_not_remembered() {
        // A failed read is a fact about a moment, not about the watermark.
        // Caching one would make a transient store error permanent until the
        // recorder next wrote -- turning a blip into an outage.
        let cache: Cache<u64> = Cache::new();
        let failed: Result<Arc<u64>, &str> =
            cache.get_or_compute(Slot(100), (), || Err("store down"));
        assert_eq!(failed.err(), Some("store down"));
        assert!(cache.peek(Slot(100), &()).is_none(), "nothing was stored");

        let runs = AtomicUsize::new(0);
        let recovered = cache
            .get_or_compute(Slot(100), (), counting(&runs, 5))
            .expect("ok");
        assert_eq!(*recovered, 5, "the next attempt is allowed to succeed");
    }

    #[test]
    fn advancing_the_watermark_replaces_the_entry_rather_than_growing() {
        // One entry, not a map. A map would hold every historical answer a
        // replay ever asked for, which is unbounded and reachable by anyone who
        // can vary `AsOf`.
        let cache = Cache::new();
        let runs = AtomicUsize::new(0);
        for slot in 1..=5u64 {
            cache
                .get_or_compute(Slot(slot), (), counting(&runs, slot))
                .expect("ok");
        }
        assert_eq!(runs.load(Ordering::SeqCst), 5);
        assert_eq!(*cache.peek(Slot(5), &()).expect("the newest"), 5);
        for slot in 1..=4u64 {
            assert!(cache.peek(Slot(slot), &()).is_none(), "{slot} was evicted");
        }
    }

    #[test]
    fn readers_share_one_allocation() {
        // `Arc`, so a 938k-row answer is not cloned per request.
        let cache = Cache::new();
        let runs = AtomicUsize::new(0);
        let a = cache
            .get_or_compute(Slot(1), (), counting(&runs, 42))
            .expect("ok");
        let b = cache.peek(Slot(1), &()).expect("hit");
        assert!(Arc::ptr_eq(&a, &b));
    }

    #[test]
    fn a_recent_enough_answer_is_offered_with_the_watermark_it_was_computed_at() {
        // The production fix. Keyed exactly, this missed about half the time --
        // the recorder advances the watermark every ~50s -- and a miss measured
        // 4.4 to 5.2 seconds against a 500ms budget.
        let cache: Cache<u64> = Cache::new();
        let runs = AtomicUsize::new(0);
        cache
            .get_or_compute(Slot(1000), (), counting(&runs, 7))
            .expect("ok");

        // Inside the allowance: offered, and it says how old it is. The label is
        // returned rather than left to the caller, because an older answer
        // served as though it were current is rule 9's shape.
        let (as_of, value) = cache
            .recent(Slot(1100), &(), 150)
            .expect("within the allowance");
        assert_eq!(*value, 7);
        assert_eq!(as_of, Slot(1000), "labelled with when it was computed");

        // The exact boundary, swept from both sides.
        assert!(
            cache.recent(Slot(1150), &(), 150).is_some(),
            "exactly at it"
        );
        assert!(cache.recent(Slot(1151), &(), 150).is_none(), "one past it");

        // And the allowance does not weaken the key.
        let mints: Cache<u64, String> = Cache::new();
        mints
            .get_or_compute(Slot(1000), "mint-a".to_owned(), counting(&runs, 1))
            .expect("ok");
        assert!(
            mints
                .recent(Slot(1010), &"mint-b".to_owned(), 150)
                .is_none(),
            "a different mint is still a miss, however recent"
        );
    }

    #[test]
    fn an_answer_from_the_future_is_never_offered_as_recent() {
        // Cannot arise from an endpoint whose watermark only advances, but a
        // saturating subtraction would report a future entry as zero slots old
        // -- treating the one genuinely alarming case as the freshest possible
        // answer. Rule 3 in the direction that actually matters.
        let cache: Cache<u64> = Cache::new();
        let runs = AtomicUsize::new(0);
        cache
            .get_or_compute(Slot(2000), (), counting(&runs, 7))
            .expect("ok");
        assert!(
            cache.recent(Slot(1000), &(), 150).is_none(),
            "an entry 1000 slots ahead is not a recent answer"
        );
    }

    #[test]
    fn a_different_key_at_the_same_watermark_is_a_miss() {
        // How `/v1/tokens/{mint}` uses this, and the bug that put the key into
        // the type. With the mint left to the handler to compare, a request for
        // one mint fell through to a lookup keyed on the watermark alone and was
        // handed ANOTHER token's evidence. That is a far worse bug than a slow
        // response, and nothing but this test stood between it and the wire.
        let cache: Cache<u64, String> = Cache::new();
        let runs = AtomicUsize::new(0);
        let ask = |mint: &str, value: u64| {
            cache
                .get_or_compute(Slot(10), mint.to_owned(), || {
                    runs.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, ()>(value)
                })
                .expect("ok")
        };

        assert_eq!(*ask("mint-a", 1), 1);
        assert_eq!(*ask("mint-b", 2), 2, "not mint-a's answer");
        assert_eq!(runs.load(Ordering::SeqCst), 2, "both computed");
        assert!(
            cache.peek(Slot(10), &"mint-a".to_owned()).is_none(),
            "mint-b displaced it -- one entry, not a map"
        );
        assert_eq!(*ask("mint-b", 9), 2, "and mint-b is still cached");
        assert_eq!(runs.load(Ordering::SeqCst), 2);
    }
}
