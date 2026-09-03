// SPDX-License-Identifier: Apache-2.0
//! A mutability-aware cache. The cheapest call is the one you do not make.

use std::collections::HashMap;

use radar_asof::{AsOf, LookAhead, Observed};
use radar_types::{Latch, MicroUsd, Mutability, Revalidation, Slot};
use serde_json::Value;

/// A content-addressed cache key over a namespace and canonicalised arguments.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct CacheKey([u8; 32]);

impl CacheKey {
    /// Derives a key from a namespace and its arguments.
    ///
    /// Arguments are canonicalised first, so `{"a":1,"b":2}` and `{"b":2,"a":1}`
    /// are the same key. Without that, map iteration order silently halves the
    /// hit rate and the cost shows up as a vendor bill rather than a bug.
    #[must_use]
    pub fn new(namespace: &str, args: &Value) -> Self {
        let mut h = blake3::Hasher::new();
        h.update(namespace.as_bytes());
        h.update(b"\0");
        let mut canonical = String::new();
        write_canonical(args, &mut canonical);
        h.update(canonical.as_bytes());
        Self(*h.finalize().as_bytes())
    }

    /// The key as bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Writes JSON with object keys sorted, so equal values hash equally.
fn write_canonical(v: &Value, out: &mut String) {
    match v {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_unstable();
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(k).unwrap_or_default());
                out.push(':');
                write_canonical(&map[*k], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&serde_json::to_string(other).unwrap_or_default()),
    }
}

/// A cached value with everything needed to decide whether it is still usable.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Entry {
    /// The serialised value.
    ///
    /// Private, and readable only through [`Entry::bytes`], which requires a
    /// watermark. AGENTS.md rule 3 says nothing reads past its watermark; this
    /// is that rule held by the type rather than by a call site remembering to
    /// ask. The check below used to be an `if` in `decide` that a later edit
    /// could have reordered past the freshness logic without breaking a test.
    bytes: Vec<u8>,
    /// Content hash, used as the validator in a conditional request.
    pub hash: [u8; 32],
    /// The slot this value was true at.
    pub observed_at: Slot,
    /// How often this fact can change.
    pub mutability: Mutability,
    /// For [`Mutability::Latched`] facts, whether the transition has happened.
    pub latch: Option<Latch>,
    /// What a full fetch of this value costs, used for eviction scoring.
    pub refetch_cost: MicroUsd,
    hits: u64,
}

impl Entry {
    /// The cached value, if this entry is admissible at `as_of`.
    ///
    /// # Errors
    ///
    /// Returns [`LookAhead`] when the entry was observed after the watermark.
    /// Callers deciding whether to serve treat that as a miss rather than an
    /// error; callers that expected the value are looking at a bug.
    pub fn bytes(&self, as_of: AsOf) -> Result<&[u8], LookAhead> {
        as_of.accept(Observed::new(self.bytes.as_slice(), self.observed_at))
    }

    /// Builds an entry, hashing the value.
    #[must_use]
    pub fn new(
        bytes: Vec<u8>,
        observed_at: Slot,
        mutability: Mutability,
        refetch_cost: MicroUsd,
    ) -> Self {
        let hash = *blake3::hash(&bytes).as_bytes();
        Self {
            bytes,
            hash,
            observed_at,
            mutability,
            latch: None,
            refetch_cost,
            hits: 0,
        }
    }

    /// Marks a latched fact's transition as having happened.
    #[must_use]
    pub const fn with_latch(mut self, latch: Latch) -> Self {
        self.latch = Some(latch);
        self
    }
}

/// What to do about a request, given what the cache holds.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Decision {
    /// Serve the cached bytes. Costs nothing.
    Serve(Vec<u8>),
    /// Ask the origin whether the value changed, sending this validator. Costs a
    /// fraction of a fetch where the origin supports conditional requests, and
    /// degrades to a fetch where it does not.
    Revalidate {
        /// The content hash the caller last saw.
        prior_hash: [u8; 32],
    },
    /// Fetch in full. Costs the catalogue price.
    Fetch,
}

/// Hit-rate and saving counters. The point of measuring is to be able to say
/// what the cache is actually worth rather than assuming it is worth something.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Stats {
    /// Served from cache at zero cost.
    pub served: u64,
    /// Conditional requests issued.
    pub revalidated: u64,
    /// Full fetches.
    pub fetched: u64,
    /// Entries dropped to stay under capacity.
    pub evicted: u64,
    /// What the served hits would have cost at catalogue price.
    pub avoided_cost: MicroUsd,
}

/// A bounded, mutability-aware response cache.
#[derive(Debug)]
pub struct Cache {
    entries: HashMap<CacheKey, Entry>,
    capacity: usize,
    stats: Stats,
}

impl Cache {
    /// A cache holding at most `capacity` entries.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            capacity,
            stats: Stats::default(),
        }
    }

    /// Counters so far.
    #[must_use]
    pub const fn stats(&self) -> Stats {
        self.stats
    }

    /// Number of entries held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Decides what to do about a request as of a watermark.
    ///
    /// A cached value observed *after* the watermark is unusable, and reading
    /// one is not possible here: the bytes come out of [`Entry::bytes`], which
    /// takes the watermark. See the comment at the call site.
    pub fn decide(&mut self, key: CacheKey, mutability: Mutability, as_of: AsOf) -> Decision {
        if !mutability.is_cacheable() {
            self.stats.fetched += 1;
            return Decision::Fetch;
        }

        let Some(entry) = self.entries.get(&key) else {
            self.stats.fetched += 1;
            return Decision::Fetch;
        };

        // The point-in-time guarantee reaching into the cache. A replay served a
        // live-populated entry from the future would silently reintroduce the
        // look-ahead the whole design exists to prevent -- so reading the value
        // *is* the check, rather than an `if` above it that a later edit could
        // reorder past the freshness logic without breaking a test.
        let Ok(bytes) = entry.bytes(as_of) else {
            self.stats.fetched += 1;
            return Decision::Fetch;
        };

        let serve = match mutability.revalidation() {
            Revalidation::Never => true,
            Revalidation::UntilLatched => entry.latch.is_some_and(Latch::is_closed),
            Revalidation::After(budget) => as_of.staleness(entry.observed_at) <= budget,
            Revalidation::Always => false,
        };

        if serve {
            let bytes = bytes.to_vec();
            let cost = entry.refetch_cost;
            if let Some(e) = self.entries.get_mut(&key) {
                e.hits += 1;
            }
            self.stats.served += 1;
            self.stats.avoided_cost = self.stats.avoided_cost.saturating_add(cost);
            Decision::Serve(bytes)
        } else {
            let prior_hash = entry.hash;
            self.stats.revalidated += 1;
            Decision::Revalidate { prior_hash }
        }
    }

    /// Reads an entry without affecting statistics or the decision path.
    #[must_use]
    pub fn peek(&self, key: CacheKey) -> Option<&Entry> {
        self.entries.get(&key)
    }

    /// Stores a value, evicting if over capacity.
    pub fn put(&mut self, key: CacheKey, entry: Entry) {
        if !entry.mutability.is_cacheable() {
            return;
        }
        if self.entries.len() >= self.capacity && !self.entries.contains_key(&key) {
            self.evict_one();
        }
        self.entries.insert(key, entry);
    }

    /// Records that a revalidation came back unchanged, refreshing the entry's
    /// observation slot without paying for the body.
    ///
    /// This is the whole economic case for conditional requests: the value is
    /// already in hand, and all that was bought is the knowledge that it is
    /// still current.
    pub fn touch(&mut self, key: CacheKey, now: Slot) {
        if let Some(e) = self.entries.get_mut(&key) {
            e.observed_at = now;
        }
    }

    /// Drops the least valuable entry.
    ///
    /// Value is hits × refetch cost: an entry that is asked for often and is
    /// expensive to rebuild earns its space, one that is cheap and rarely wanted
    /// does not. Immutable entries are never evicted — they can never be
    /// invalidated, so their space is bought once and pays forever.
    fn evict_one(&mut self) {
        let victim = self
            .entries
            .iter()
            .filter(|(_, e)| e.mutability != Mutability::Immutable)
            .min_by_key(|(k, e)| (e.hits.saturating_mul(e.refetch_cost.get()), **k))
            .map(|(k, _)| *k);

        if let Some(k) = victim {
            self.entries.remove(&k);
            self.stats.evicted += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn key() -> CacheKey {
        CacheKey::new("token_structure", &json!({ "mint": "So111" }))
    }

    fn entry(at: Slot, m: Mutability) -> Entry {
        Entry::new(b"value".to_vec(), at, m, MicroUsd::from_dollars(0.01))
    }

    #[test]
    fn argument_order_does_not_change_the_key() {
        // Two spellings of the same request must hit the same entry, or the hit
        // rate silently halves and the cost lands on the vendor bill.
        let a = CacheKey::new("ns", &json!({ "a": 1, "b": 2 }));
        let b = CacheKey::new("ns", &json!({ "b": 2, "a": 1 }));
        assert_eq!(a, b);
    }

    #[test]
    fn different_namespaces_do_not_collide() {
        let a = CacheKey::new("token_structure", &json!({ "mint": "x" }));
        let b = CacheKey::new("holder_concentration", &json!({ "mint": "x" }));
        assert_ne!(a, b);
    }

    #[test]
    fn array_order_does_change_the_key() {
        // Arrays are ordered in JSON and often ordered in meaning too.
        let a = CacheKey::new("ns", &json!({ "xs": [1, 2] }));
        let b = CacheKey::new("ns", &json!({ "xs": [2, 1] }));
        assert_ne!(a, b);
    }

    #[test]
    fn an_immutable_fact_is_fetched_once_and_never_again() {
        // This is the single largest saving in the system and it needs nothing
        // from any vendor.
        let mut c = Cache::new(16);
        assert_eq!(
            c.decide(key(), Mutability::Immutable, AsOf::at(Slot(100))),
            Decision::Fetch
        );
        c.put(key(), entry(Slot(100), Mutability::Immutable));

        for slot in [101, 10_000, 100_000_000] {
            let d = c.decide(key(), Mutability::Immutable, AsOf::at(Slot(slot)));
            assert_eq!(
                d,
                Decision::Serve(b"value".to_vec()),
                "refetched at slot {slot}"
            );
        }
        assert_eq!(c.stats().served, 3);
        assert_eq!(c.stats().avoided_cost, MicroUsd::from_dollars(0.03));
    }

    #[test]
    fn a_latched_fact_revalidates_until_it_closes_then_never() {
        let mut c = Cache::new(16);
        c.put(key(), entry(Slot(100), Mutability::Latched));

        // Latch open: keep asking, because the authority may still be revoked.
        assert!(matches!(
            c.decide(key(), Mutability::Latched, AsOf::at(Slot(200))),
            Decision::Revalidate { .. }
        ));

        // Latch closed: revocation cannot be undone, so stop asking forever.
        c.put(
            key(),
            entry(Slot(200), Mutability::Latched).with_latch(Latch::Closed),
        );
        assert_eq!(
            c.decide(key(), Mutability::Latched, AsOf::at(Slot(9_999_999))),
            Decision::Serve(b"value".to_vec())
        );
    }

    #[test]
    fn a_fast_fact_is_served_inside_its_budget_and_revalidated_outside_it() {
        let mut c = Cache::new(16);
        c.put(key(), entry(Slot(1_000), Mutability::Fast));

        // Fast budget is 150 slots.
        assert_eq!(
            c.decide(key(), Mutability::Fast, AsOf::at(Slot(1_100))),
            Decision::Serve(b"value".to_vec())
        );
        assert!(matches!(
            c.decide(key(), Mutability::Fast, AsOf::at(Slot(1_200))),
            Decision::Revalidate { .. }
        ));
    }

    #[test]
    fn a_realtime_fact_is_never_cached_or_served() {
        // Caching a route quote is not an optimisation, it is a wrong answer.
        let mut c = Cache::new(16);
        c.put(key(), entry(Slot(100), Mutability::Realtime));
        assert!(c.is_empty());
        assert_eq!(
            c.decide(key(), Mutability::Realtime, AsOf::at(Slot(100))),
            Decision::Fetch
        );
    }

    #[test]
    fn a_cached_value_from_the_future_is_refused_during_replay() {
        // The point-in-time guarantee has to reach into the cache, or a replay
        // serves a live-populated entry from the future and reintroduces exactly
        // the look-ahead the design exists to prevent.
        let mut c = Cache::new(16);
        c.put(key(), entry(Slot(5_000), Mutability::Immutable));
        assert_eq!(
            c.decide(key(), Mutability::Immutable, AsOf::at(Slot(1_000))),
            Decision::Fetch
        );
    }

    #[test]
    fn a_confirmed_unchanged_value_is_refreshed_without_paying_for_the_body() {
        let mut c = Cache::new(16);
        c.put(key(), entry(Slot(1_000), Mutability::Fast));
        assert!(matches!(
            c.decide(key(), Mutability::Fast, AsOf::at(Slot(1_500))),
            Decision::Revalidate { .. }
        ));
        c.touch(key(), Slot(1_500));
        assert_eq!(
            c.decide(key(), Mutability::Fast, AsOf::at(Slot(1_600))),
            Decision::Serve(b"value".to_vec())
        );
    }

    #[test]
    fn eviction_spares_immutable_entries() {
        // An immutable entry can never be invalidated, so its space is bought
        // once and pays forever. Evicting one to make room for a value that
        // expires in a minute is strictly backwards.
        let mut c = Cache::new(2);
        let immutable = CacheKey::new("ns", &json!({ "k": "immutable" }));
        c.put(immutable, entry(Slot(1), Mutability::Immutable));
        c.put(
            CacheKey::new("ns", &json!({ "k": "fast1" })),
            entry(Slot(1), Mutability::Fast),
        );
        c.put(
            CacheKey::new("ns", &json!({ "k": "fast2" })),
            entry(Slot(1), Mutability::Fast),
        );

        assert_eq!(c.stats().evicted, 1);
        assert!(c.peek(immutable).is_some(), "immutable entry must survive");
    }
}
