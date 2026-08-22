// SPDX-License-Identifier: Apache-2.0
//! `creator_history` — what an address has launched before now.
//!
//! Creator identity is recorded in the launch instruction itself, so unlike
//! wallet reputation it is structurally verifiable rather than inferred. That is
//! why it is the first instrument: it costs nothing beyond a store read, needs no
//! paid data, and thirty minutes of recorded chain already separates addresses
//! launching one token from an address launching forty-two.
//!
//! It reports; it does not judge. There is no "spam" flag here, because whether a
//! launch rate predicts anything is a question for the research store to answer
//! against outcomes, not for this instrument to assume.

use std::collections::{BTreeMap, BTreeSet};

use radar_store::{Event, Table};
use radar_types::Mutability;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::registry::{Context, Instrument, InstrumentError};
use crate::spec::{Cost, Determinism, Latency, Spec, Version};

/// Which creator to look up.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct Input {
    /// The creator address, base58.
    pub creator: String,
}

/// What the creator has done, as of the watermark.
#[derive(Debug, Clone, Serialize, JsonSchema, PartialEq)]
pub struct Output {
    /// The address asked about.
    pub creator: String,
    /// Launches recorded at or before the watermark.
    pub launches: u64,
    /// Earliest recorded launch slot.
    pub first_launch_slot: Option<u64>,
    /// Latest recorded launch slot.
    pub last_launch_slot: Option<u64>,
    /// Slots spanned by those launches.
    pub span_slots: u64,
    /// Distinct symbols used.
    ///
    /// Far fewer symbols than launches means the same token is being relaunched
    /// under one name, which is a different behaviour from launching many
    /// different ones.
    pub distinct_symbols: u64,
    /// Launches whose name, symbol and URI exactly match an earlier launch by
    /// the same creator.
    pub duplicate_metadata_launches: u64,
    /// The most launches this creator put in a single slot. More than one means
    /// they are submitting in parallel rather than by hand.
    pub max_launches_in_one_slot: u64,
    /// Launches whose transaction failed. A creator with many is either fighting
    /// for blockspace or mis-configured.
    pub failed_launches: u64,
    /// Launches per hour of chain across the observed span, or `None` when the
    /// span is too short to divide by.
    pub launches_per_hour: Option<f64>,
    /// Signatures backing the counts, newest first, capped.
    ///
    /// Every claim here is checkable against an explorer. A conclusion with no
    /// way back to the evidence is an assertion.
    pub evidence: Vec<String>,
}

/// How many signatures to return. Enough to spot-check, not enough to make the
/// response a data dump.
const MAX_EVIDENCE: usize = 20;

/// Reports what a creator has launched.
pub struct CreatorHistory;

impl Instrument for CreatorHistory {
    type Input = Input;
    type Output = Output;

    fn spec(&self) -> Spec {
        Spec {
            name: "creator_history",
            version: Version::new(1, 0),
            summary: "What an address has launched before now: counts, cadence, \
                      repeated metadata, and the signatures backing each figure.",
            latency: Latency::Warm,
            // A store read. No vendor is paid, which is what makes this callable
            // on every candidate rather than on a chosen few.
            cost: Cost::FREE,
            // A creator's history only ever grows, and a launch that happened
            // cannot un-happen, so an answer as of a slot stays true for it.
            freshness: Mutability::Slow,
            determinism: Determinism::Pure,
        }
    }

    fn run(&self, input: Input, ctx: &Context<'_>) -> Result<Output, InstrumentError> {
        let events = ctx.store.read(Table::Launches, ctx.as_of).map_err(|e| {
            InstrumentError::OutOfRange {
                as_of: ctx.as_of.to_string(),
                detail: e.to_string(),
            }
        })?;

        let mut slots: Vec<u64> = Vec::new();
        let mut symbols: BTreeSet<String> = BTreeSet::new();
        let mut metadata_seen: BTreeSet<(String, String, String)> = BTreeSet::new();
        let mut per_slot: BTreeMap<u64, u64> = BTreeMap::new();
        let mut duplicates = 0u64;
        let mut failed = 0u64;
        let mut evidence: Vec<(u64, String)> = Vec::new();

        for event in &events {
            let Event::Launch(l) = event else { continue };
            if l.creator.to_string() != input.creator {
                continue;
            }
            let slot = l.envelope.slot.get();
            slots.push(slot);
            symbols.insert(l.symbol.clone());
            *per_slot.entry(slot).or_default() += 1;
            if !metadata_seen.insert((l.name.clone(), l.symbol.clone(), l.uri.clone())) {
                duplicates += 1;
            }
            if !l.envelope.succeeded {
                failed += 1;
            }
            evidence.push((slot, l.envelope.signature.to_string()));
        }

        let launches = slots.len() as u64;
        let first = slots.iter().min().copied();
        let last = slots.iter().max().copied();
        let span = match (first, last) {
            (Some(a), Some(b)) => b - a,
            _ => 0,
        };

        evidence.sort_by_key(|(slot, _)| std::cmp::Reverse(*slot));
        evidence.truncate(MAX_EVIDENCE);

        Ok(Output {
            creator: input.creator,
            launches,
            first_launch_slot: first,
            last_launch_slot: last,
            span_slots: span,
            distinct_symbols: symbols.len() as u64,
            duplicate_metadata_launches: duplicates,
            max_launches_in_one_slot: per_slot.values().copied().max().unwrap_or(0),
            failed_launches: failed,
            launches_per_hour: rate_per_hour(launches, span),
            evidence: evidence.into_iter().map(|(_, sig)| sig).collect(),
        })
    }
}

/// Launches per hour of chain, at roughly 2.5 slots a second.
///
/// `None` below a slot of span rather than a huge number: dividing by a span of
/// one would report a creator with two launches as doing thousands an hour, which
/// is the kind of figure that looks like a finding and is an artefact.
#[must_use]
fn rate_per_hour(launches: u64, span_slots: u64) -> Option<f64> {
    const SLOTS_PER_HOUR: f64 = 9_000.0;
    if span_slots < 100 || launches == 0 {
        return None;
    }
    #[expect(clippy::cast_precision_loss, reason = "a display rate, not accounting")]
    Some(launches as f64 * SLOTS_PER_HOUR / span_slots as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rate_is_not_reported_from_too_short_a_span() {
        // Two launches a slot apart is not 18,000 an hour; it is not enough
        // evidence to state a rate at all.
        assert_eq!(rate_per_hour(2, 1), None);
        assert_eq!(rate_per_hour(0, 100_000), None);
        assert!(rate_per_hour(10, 9_000).is_some());
    }

    #[test]
    fn a_rate_over_a_full_hour_of_chain_is_the_launch_count() {
        let rate = rate_per_hour(42, 9_000).expect("enough span");
        assert!((rate - 42.0).abs() < 0.001, "{rate}");
    }

    #[test]
    fn the_spec_is_free_and_therefore_callable_on_every_candidate() {
        let spec = Instrument::spec(&CreatorHistory);
        assert_eq!(spec.cost.upstream, radar_types::MicroUsd::ZERO);
        assert_eq!(spec.name, "creator_history");
        // Warm rather than hot: it reads files, so it must not sit on an
        // execution path.
        assert!(!spec.safe_on_execution_path());
    }
}
