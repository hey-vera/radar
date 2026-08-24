// SPDX-License-Identifier: Apache-2.0
//! `creator_track_record` — how a creator's previous tokens actually turned out.
//!
//! `creator_history` says what an address has *done*. This says what happened
//! *next*, which is the only half that could ever be predictive.
//!
//! It is the first instrument that touches outcomes, and it is deliberately
//! constrained in what it will claim. It reports rates and the counts behind
//! them, and it refuses to state a rate at all below a sample where a rate would
//! be noise. Two launches with one death is not a 50% failure rate; it is two
//! launches.

use radar_store::{Event, GraduationMode, Outcome, Table};
use radar_types::Mutability;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::registry::{Context, Instrument, InstrumentError};
use crate::spec::{Cost, Determinism, Latency, Spec, Version};

/// Below this many measured launches, no rate is reported.
///
/// Not a statistical threshold so much as a refusal to mislead: a percentage
/// computed from three tokens reads exactly like one computed from three
/// hundred, and nothing downstream can tell them apart once it is a number.
const MIN_SAMPLE_FOR_RATES: u64 = 5;

/// Which creator to look up.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct Input {
    /// The creator address, base58.
    pub creator: String,
}

/// How a creator's tokens have turned out, as of the watermark.
#[derive(Debug, Clone, Serialize, JsonSchema, PartialEq)]
pub struct Output {
    /// The address asked about.
    pub creator: String,
    /// Launches recorded at or before the watermark.
    pub launches: u64,
    /// Launches for which an outcome has been measured.
    ///
    /// Always at most `launches`. A gap means the outcome pass has not caught up,
    /// not that those tokens did nothing.
    pub measured: u64,
    /// Measured tokens that showed almost no life: five or fewer transfers
    /// within 300 slots of launch.
    pub stillborn: u64,
    /// Measured tokens that reached an AMM, however they got there.
    ///
    /// Reported for completeness and **not** the number to rank creators by.
    /// See `graduated_organic`.
    pub graduated: u64,
    /// Measured tokens whose curve filled over time rather than in a block.
    ///
    /// The one that carries information. A curve bought out within three slots
    /// of its launch was bought by capital committed before the token existed,
    /// so it is evidence of coordination rather than of demand — and a creator
    /// ranked on the undifferentiated count is ranked partly on how well they
    /// bundle. Measured across the store, 39% of graduations are instant.
    pub graduated_organic: u64,
    /// Measured tokens whose curve completed within three slots of the launch.
    pub graduated_instant: u64,
    /// Share of measured tokens that were stillborn, or `None` below the minimum
    /// sample.
    pub stillborn_rate: Option<f64>,
    /// Share that graduated by any route, or `None` below the minimum sample.
    pub graduation_rate: Option<f64>,
    /// Share that graduated organically, or `None` below the minimum sample.
    ///
    /// The rate `creator_edge` gates on.
    pub organic_graduation_rate: Option<f64>,
    /// Median slots survived across measured tokens.
    pub median_survival_slots: Option<u64>,
    /// The longest-surviving token's slot count.
    pub best_survival_slots: Option<u64>,
    /// Total transfers across all of this creator's measured tokens.
    pub total_transfers: u64,
    /// Why a rate is absent, when one is.
    pub sample_note: Option<String>,
}

/// Reports how a creator's tokens turned out.
pub struct CreatorTrackRecord;

impl Instrument for CreatorTrackRecord {
    type Input = Input;
    type Output = Output;

    fn spec(&self) -> Spec {
        Spec {
            name: "creator_track_record",
            version: Version::new(1, 0),
            summary: "How a creator's previous tokens actually turned out: how many died \
                      on arrival, how many reached an AMM, and how long the rest survived.",
            latency: Latency::Warm,
            cost: Cost::FREE,
            // Outcomes only accumulate, and a launch that happened cannot
            // un-happen, so an answer as of a slot stays true for that slot.
            freshness: Mutability::Slow,
            determinism: Determinism::Pure,
        }
    }

    fn run(&self, input: Input, ctx: &Context<'_>) -> Result<Output, InstrumentError> {
        let launches = ctx.store.read(Table::Launches, ctx.as_of).map_err(|e| {
            InstrumentError::OutOfRange {
                as_of: ctx.as_of.to_string(),
                detail: e.to_string(),
            }
        })?;
        let outcomes =
            ctx.store
                .read_outcomes(ctx.as_of)
                .map_err(|e| InstrumentError::OutOfRange {
                    as_of: ctx.as_of.to_string(),
                    detail: e.to_string(),
                })?;

        let mints: Vec<radar_types::Address> = launches
            .iter()
            .filter_map(|e| match e {
                Event::Launch(l) if l.creator.to_string() == input.creator => Some(l.mint),
                _ => None,
            })
            .collect();

        // A mint can be measured more than once. Keep the latest measurement at
        // or before the watermark, because that is what was known then.
        let mut latest: std::collections::BTreeMap<radar_types::Address, &Outcome> =
            std::collections::BTreeMap::new();
        for outcome in &outcomes {
            if !mints.contains(&outcome.mint) {
                continue;
            }
            latest
                .entry(outcome.mint)
                .and_modify(|held| {
                    if outcome.measured_at > held.measured_at {
                        *held = outcome;
                    }
                })
                .or_insert(outcome);
        }

        let measured = latest.len() as u64;
        let stillborn = latest.values().filter(|o| o.appears_stillborn()).count() as u64;
        let graduated = latest.values().filter(|o| o.graduated()).count() as u64;
        let graduated_organic = latest
            .values()
            .filter(|o| o.graduation_mode() == Some(GraduationMode::Organic))
            .count() as u64;
        let graduated_instant = latest
            .values()
            .filter(|o| o.graduation_mode() == Some(GraduationMode::Instant))
            .count() as u64;
        let total_transfers: u64 = latest.values().map(|o| o.transfers).sum();

        let mut survivals: Vec<u64> = latest.values().map(|o| o.survived_slots()).collect();
        survivals.sort_unstable();

        let enough = measured >= MIN_SAMPLE_FOR_RATES;
        let rate = |count: u64| {
            #[expect(
                clippy::cast_precision_loss,
                reason = "a display ratio over small counts"
            )]
            enough.then(|| count as f64 / measured as f64)
        };

        Ok(Output {
            creator: input.creator,
            launches: mints.len() as u64,
            measured,
            stillborn,
            graduated,
            graduated_organic,
            graduated_instant,
            stillborn_rate: rate(stillborn),
            graduation_rate: rate(graduated),
            organic_graduation_rate: rate(graduated_organic),
            median_survival_slots: survivals.get(survivals.len() / 2).copied(),
            best_survival_slots: survivals.last().copied(),
            total_transfers,
            sample_note: (!enough).then(|| {
                format!(
                    "{measured} measured launches is below the {MIN_SAMPLE_FOR_RATES} needed \
                     to state a rate; counts are given instead"
                )
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spec_is_free_and_slow_moving() {
        let spec = Instrument::spec(&CreatorTrackRecord);
        assert_eq!(spec.name, "creator_track_record");
        assert_eq!(spec.cost.upstream, radar_types::MicroUsd::ZERO);
        assert_eq!(spec.freshness, Mutability::Slow);
        // Reads files, so it must never sit on an execution path.
        assert!(!spec.safe_on_execution_path());
    }
}
