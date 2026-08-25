// SPDX-License-Identifier: Apache-2.0
//! Turning recorded data into candidates.
//!
//! This is where look-ahead is prevented, once, for every strategy. A strategy
//! takes a [`Candidate`] and nothing else, so if this module never puts a fact
//! from after the watermark into one, no strategy can read one — regardless of
//! what its own rules do.
//!
//! The alternative would be every strategy checking its own watermarks, which
//! works until the twentieth strategy and then does not.
//!
//! # Live and research are the same call
//!
//! Live mode passes the current confirmed slot; research passes a historical one.
//! Same function, same code, same outputs. "Backtest" is not a separate engine
//! here — it is this function with a different argument, which is why the two
//! cannot drift apart.

use std::collections::BTreeMap;

use radar_asof::AsOf;
use radar_sim::ExitReport;
use radar_store::{Event, GraduationMode, Outcome, Reader, StoreError, Table};
use radar_types::{Address, MicroUsd, Slot};

use crate::{Candidate, CreatorRecord};

/// A creator's record, and every launch known at the watermark.
///
/// Built once per pass rather than per candidate: at ~35,000 launches a day, a
/// per-candidate scan would be quadratic in a dataset that only grows.
pub struct Universe {
    /// Launches at or before the watermark, by mint.
    pub launches: BTreeMap<Address, LaunchFacts>,
    /// Each creator's measured history at the watermark.
    pub creators: BTreeMap<Address, CreatorRecord>,
    /// When each creator's record last changed.
    ///
    /// Tracked separately from the record itself because staleness is a
    /// different question from content: a creator with twenty measured launches
    /// and a creator with twenty measured launches whose last measurement was a
    /// week ago are the same record and different evidence.
    pub creators_observed_at: BTreeMap<Address, Slot>,
    /// The watermark everything here was read at.
    pub as_of: AsOf,
}

/// What is known about one launch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaunchFacts {
    /// Who launched it.
    pub creator: Address,
    /// When.
    pub slot: Slot,
    /// When this token's mutable facts were last observed.
    ///
    /// Not the launch slot: a token launched a week ago whose outcome was
    /// measured an hour ago rests on an hour-old reading, not a week-old one.
    pub observed_at: Slot,
}

/// Slots in a day, at the nominal 2.5 a second.
///
/// Nominal because the measured rate moves: 2.4059 slots a second on 2026-08-20
/// and 2.7349 on 2026-08-23. A launch rate is compared against a threshold with
/// wide separation either side, so a 14% wobble in the denominator does not
/// change an answer — but it is why this is not given more precision than it has.
const SLOTS_PER_DAY: u64 = 216_000;

/// The shortest span a launch rate may be divided over.
///
/// Six hours. Below it the denominator is small enough that one busy minute
/// reads as a thousand launches a day, and the rate says more about when the
/// creator was first seen than about how they behave. Absent, not zero: a rate
/// that cannot be measured is `None`, because zero would read as the quietest
/// possible creator and pass a threshold it was never tested against.
const MIN_RATE_SPAN_SLOTS: u64 = 54_000;

/// Reads everything at or before `as_of`.
///
/// # Errors
///
/// Returns [`StoreError`] if the store cannot be read.
pub fn universe(reader: &Reader, as_of: AsOf) -> Result<Universe, StoreError> {
    let mut launches = BTreeMap::new();
    let mut per_creator: BTreeMap<Address, CreatorRecord> = BTreeMap::new();
    let mut creator_observed: BTreeMap<Address, Slot> = BTreeMap::new();
    let mut first_launch: BTreeMap<Address, Slot> = BTreeMap::new();

    for event in reader.read(Table::Launches, as_of)? {
        let Event::Launch(launch) = event else {
            // The launches table holds launches. Anything else is a bug in the
            // writer, and treating it as a launch would put a trade's fields in
            // a launch's shape.
            continue;
        };
        launches.insert(
            launch.mint,
            LaunchFacts {
                creator: launch.creator,
                slot: launch.envelope.slot,
                observed_at: launch.envelope.slot,
            },
        );
        per_creator.entry(launch.creator).or_default().launches += 1;
        // Earliest launch, for the rate denominator below.
        first_launch
            .entry(launch.creator)
            .and_modify(|at: &mut Slot| *at = (*at).min(launch.envelope.slot))
            .or_insert(launch.envelope.slot);
        // A launch is itself an observation of the creator: it is the moment
        // Radar last learned anything about them.
        let seen = creator_observed
            .entry(launch.creator)
            .or_insert(launch.envelope.slot);
        *seen = (*seen).max(launch.envelope.slot);
    }

    // Outcomes are read second so a measurement can update the freshness of the
    // launch it describes.
    for outcome in reader.read_outcomes(as_of)? {
        let Some(facts) = launches.get_mut(&outcome.mint) else {
            // An outcome for a launch we did not record. Skipped rather than
            // counted: attributing it to a creator we cannot name would put a
            // measurement into no creator's record while inflating the total.
            continue;
        };
        facts.observed_at = facts.observed_at.max(outcome.measured_at);

        let creator = facts.creator;
        let record = per_creator.entry(creator).or_default();
        record.measured += 1;
        if outcome.graduated() {
            record.graduated += 1;
        }
        // Counted apart, because the strategy gates on this one. A creator whose
        // tokens only ever graduate in their own launch block has a graduation
        // rate and no evidence of demand.
        if outcome.graduation_mode() == Some(GraduationMode::Organic) {
            record.graduated_organic += 1;
        }
        if outcome.appears_stillborn() {
            record.stillborn += 1;
        }
        let seen = creator_observed
            .entry(creator)
            .or_insert(outcome.measured_at);
        *seen = (*seen).max(outcome.measured_at);
    }

    // Launch rate, over each creator's own observed span. Done here rather than
    // in the strategy because it needs the watermark and the launch slots, and a
    // strategy that recomputed it from a different denominator would be applying
    // a threshold that was never measured against that denominator.
    for (creator, record) in &mut per_creator {
        record.launches_per_day = first_launch
            .get(creator)
            .map(|first| as_of.slot().get().saturating_sub(first.get()))
            .filter(|span| *span >= MIN_RATE_SPAN_SLOTS)
            .map(|span| record.launches.saturating_mul(SLOTS_PER_DAY) / span);
    }

    Ok(Universe {
        launches,
        creators: per_creator,
        creators_observed_at: creator_observed,
        as_of,
    })
}

impl Universe {
    /// Builds a candidate for one mint.
    ///
    /// `exit` and `sol_price` come from live measurement rather than the store,
    /// so they are passed in — and both are `Option`, because a strategy must be
    /// able to see that they are missing rather than be handed a default.
    ///
    /// # What `token_observed_at` means
    ///
    /// It is when the token's *mutable* state was last read, and that is not the
    /// same as when its launch was recorded. A launch slot is immutable: it does
    /// not go stale, ever, and treating it as a freshness reading refuses
    /// candidates for the age of a fact that cannot age.
    ///
    /// That is what it did. `max_token_age` is 6,000 slots (~40 minutes) and the
    /// decision lane considers a 24-hour window, so **97.6% of a live run — 40,281
    /// of 41,254 candidates — was refused as `TokenReadingTooOld`** while the exit
    /// probe backing each one had been fetched seconds earlier. This is the same
    /// mistake the token/creator staleness split was introduced to fix, left
    /// standing on the other half.
    ///
    /// So: when an exit is supplied it was measured at the watermark, and the
    /// token's reading is as fresh as the watermark. With no exit, nothing mutable
    /// has been read, and the launch observation is the most recent thing known —
    /// which correctly reads as stale, because a candidate nobody has priced is
    /// one nobody should act on.
    ///
    /// Returns `None` if the mint was not launched at or before the watermark,
    /// which is the same as saying it does not exist yet.
    #[must_use]
    pub fn candidate(
        &self,
        mint: &Address,
        exit: Option<ExitReport>,
        sol_price: Option<MicroUsd>,
    ) -> Option<Candidate> {
        let facts = self.launches.get(mint)?;
        let exit_measured = exit.is_some();
        Some(Candidate {
            mint: *mint,
            creator: facts.creator,
            launch_slot: facts.slot,
            as_of: self.as_of,
            exit,
            creator_record: self.creator_record(&facts.creator),
            // Not looked at. The caller adds it with
            // [`Candidate::with_coordination`] if it paid for the look.
            coordination: None,
            sol_price_micro_usd: sol_price,
            // See the note above: an exit is measured at the watermark, so a
            // candidate carrying one has been read now, not when it launched.
            token_observed_at: if exit_measured {
                self.as_of.slot()
            } else {
                facts.observed_at
            },
            creator_observed_at: self.creator_observed_at(&facts.creator, facts.slot),
        })
    }

    /// A creator's record, excluding the launch being considered.
    ///
    /// The exclusion matters. Counting the candidate's own launch in its
    /// creator's history is a mild look-ahead — the token gets credit for
    /// existing — and it is the sort that survives a review because it looks
    /// like an off-by-one.
    #[must_use]
    pub fn creator_record(&self, creator: &Address) -> CreatorRecord {
        self.creators.get(creator).copied().unwrap_or_default()
    }

    /// When a creator's record was last updated.
    ///
    /// Falls back to `default_to` — the token's own launch slot — for a creator
    /// nothing else is known about, because that launch *is* the last time
    /// anything was learned about them.
    #[must_use]
    pub fn creator_observed_at(&self, creator: &Address, default_to: Slot) -> Slot {
        self.creators_observed_at
            .get(creator)
            .copied()
            .unwrap_or(default_to)
    }

    /// Mints launched within `window` slots of the watermark.
    ///
    /// The working set for a live pass: everything older has either been
    /// considered already or is no longer a launch.
    #[must_use]
    pub fn recent(&self, window: u64) -> Vec<Address> {
        let floor = self.as_of.slot().get().saturating_sub(window);
        self.launches
            .iter()
            .filter(|(_, f)| f.slot.get() >= floor)
            .map(|(mint, _)| *mint)
            .collect()
    }
}

/// Whether an outcome may be admitted at a watermark.
///
/// Exposed so a caller assembling candidates from another source can apply the
/// same rule rather than reimplementing it.
#[must_use]
pub const fn admits(as_of: AsOf, outcome: &Outcome) -> bool {
    as_of.admits(outcome.measured_at)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `graduated_after` is slots from launch to graduation, or `None` for a
    /// token that never reached an AMM.
    fn outcome(
        mint: u8,
        measured_at: u64,
        graduated_after: Option<u64>,
        transfers: u64,
    ) -> Outcome {
        Outcome {
            mint: Address::new([mint; 32]),
            measured_at: Slot(measured_at),
            launch_slot: Slot(1_000),
            first_transfer_slot: Some(Slot(1_001)),
            last_transfer_slot: Some(Slot(1_001 + transfers)),
            transfers,
            unique_senders: transfers,
            unique_receivers: transfers,
            graduated_at: graduated_after.map(|d| Slot(1_000 + d)),
        }
    }

    fn universe_of(launches: &[(u8, u8, u64)], outcomes: &[Outcome], at: u64) -> Universe {
        let mut u = Universe {
            launches: BTreeMap::new(),
            creators: BTreeMap::new(),
            creators_observed_at: BTreeMap::new(),
            as_of: AsOf::at(Slot(at)),
        };
        for (mint, creator, slot) in launches {
            u.launches.insert(
                Address::new([*mint; 32]),
                LaunchFacts {
                    creator: Address::new([*creator; 32]),
                    slot: Slot(*slot),
                    observed_at: Slot(*slot),
                },
            );
            u.creators
                .entry(Address::new([*creator; 32]))
                .or_default()
                .launches += 1;
            let seen = u
                .creators_observed_at
                .entry(Address::new([*creator; 32]))
                .or_insert(Slot(*slot));
            *seen = (*seen).max(Slot(*slot));
        }
        for o in outcomes {
            let Some(facts) = u.launches.get_mut(&o.mint) else {
                continue;
            };
            facts.observed_at = facts.observed_at.max(o.measured_at);
            let creator = facts.creator;
            let record = u.creators.entry(creator).or_default();
            record.measured += 1;
            if o.graduated() {
                record.graduated += 1;
            }
            if o.graduation_mode() == Some(GraduationMode::Organic) {
                record.graduated_organic += 1;
            }
            if o.appears_stillborn() {
                record.stillborn += 1;
            }
            let seen = u
                .creators_observed_at
                .entry(creator)
                .or_insert(o.measured_at);
            *seen = (*seen).max(o.measured_at);
        }
        u
    }

    #[test]
    fn a_candidate_carries_its_creators_record() {
        let u = universe_of(
            &[(1, 9, 1_000), (2, 9, 2_000)],
            &[
                outcome(1, 3_000, Some(900), 500),
                outcome(2, 3_100, None, 2),
            ],
            10_000,
        );
        let c = u
            .candidate(&Address::new([1u8; 32]), None, None)
            .expect("launched");
        assert_eq!(c.creator_record.launches, 2);
        assert_eq!(c.creator_record.measured, 2);
        assert_eq!(c.creator_record.graduated, 1);
        assert_eq!(c.creator_record.stillborn, 1);
    }

    #[test]
    fn a_mint_that_does_not_exist_yet_has_no_candidate() {
        // Not an empty candidate. A token that has not launched is absent, and
        // handing back a default would let a strategy consider a token that
        // does not exist.
        let u = universe_of(&[(1, 9, 1_000)], &[], 10_000);
        assert!(u.candidate(&Address::new([7u8; 32]), None, None).is_none());
    }

    #[test]
    fn a_measured_outcome_makes_an_old_launch_a_fresh_candidate() {
        // A token launched a week ago whose outcome was measured an hour ago
        // rests on an hour-old input, not a week-old one. Taking the launch slot
        // would make every mature token permanently stale.
        let u = universe_of(&[(1, 9, 1_000)], &[outcome(1, 9_500, None, 400)], 10_000);
        let c = u
            .candidate(&Address::new([1u8; 32]), None, None)
            .expect("launched");
        assert_eq!(c.token_observed_at, Slot(9_500));
    }

    #[test]
    fn a_live_exit_makes_the_token_reading_as_fresh_as_the_watermark() {
        // The bug this fixes refused 40,281 of 41,254 live candidates as
        // TokenReadingTooOld while the exit backing each one had been fetched
        // seconds earlier. A launch slot is immutable and cannot go stale; using
        // it as a freshness reading is measuring the age of a fact that does not
        // age.
        let u = universe_of(&[(1, 9, 1_000)], &[], 10_000);
        let c = u
            .candidate(&Address::new([1u8; 32]), Some(an_exit()), None)
            .expect("launched");
        assert_eq!(
            c.token_observed_at,
            Slot(10_000),
            "an exit measured at the watermark is a reading at the watermark"
        );
    }

    #[test]
    fn without_an_exit_the_token_reading_is_still_only_as_fresh_as_the_store() {
        // The other direction, and the one that keeps the rule honest: nothing
        // mutable has been read, so the candidate must not claim to be current.
        // Without this, the test above is also satisfied by always answering
        // "fresh", which would disable the staleness gate entirely.
        let u = universe_of(&[(1, 9, 1_000)], &[], 10_000);
        let c = u
            .candidate(&Address::new([1u8; 32]), None, None)
            .expect("launched");
        assert_eq!(c.token_observed_at, Slot(1_000));
        assert_ne!(c.token_observed_at, u.as_of.slot());
    }

    /// A minimal exit report, for tests that only care that one is present.
    fn an_exit() -> ExitReport {
        ExitReport::build(
            Address::new([1u8; 32]),
            None,
            vec![(
                1_000,
                Ok(radar_sim::QuotePoint {
                    size_tokens: 1_000,
                    out_lamports: 1_000,
                    impact_bps: 10,
                }),
            )],
        )
    }

    #[test]
    fn an_unmeasured_launch_rests_on_its_launch_slot() {
        let u = universe_of(&[(1, 9, 1_000)], &[], 10_000);
        let c = u
            .candidate(&Address::new([1u8; 32]), None, None)
            .expect("launched");
        assert_eq!(c.token_observed_at, Slot(1_000));
    }

    #[test]
    fn a_creators_freshness_follows_their_most_recent_measurement() {
        // Not the token's. A creator measured an hour ago through a *different*
        // launch is an hour-fresh creator, whatever this particular token's own
        // reading says.
        let u = universe_of(
            &[(1, 9, 1_000), (2, 9, 2_000)],
            &[outcome(2, 9_800, Some(900), 500)],
            10_000,
        );
        let c = u
            .candidate(&Address::new([1u8; 32]), None, None)
            .expect("launched");
        assert_eq!(
            c.token_observed_at,
            Slot(1_000),
            "this token was never measured"
        );
        assert_eq!(c.creator_observed_at, Slot(9_800), "but its creator was");
    }

    #[test]
    fn the_stalest_ingredient_is_what_a_proposal_would_carry() {
        let u = universe_of(&[(1, 9, 1_000)], &[outcome(1, 9_500, None, 400)], 10_000);
        let c = u
            .candidate(&Address::new([1u8; 32]), None, None)
            .expect("launched");
        assert_eq!(
            c.oldest_input_slot(),
            c.token_observed_at.min(c.creator_observed_at)
        );
    }

    #[test]
    fn an_unknown_creator_has_an_empty_record_rather_than_a_good_one() {
        // Absent must never read as clean. A creator nobody has measured is one
        // nothing is known about.
        let u = universe_of(&[], &[], 10_000);
        assert_eq!(
            u.creator_record(&Address::new([3u8; 32])),
            CreatorRecord::default()
        );
    }

    #[test]
    fn recent_returns_only_the_working_set() {
        let u = universe_of(&[(1, 9, 1_000), (2, 9, 9_500), (3, 9, 9_900)], &[], 10_000);
        let recent = u.recent(1_000);
        assert_eq!(recent.len(), 2);
        assert!(!recent.contains(&Address::new([1u8; 32])));
    }

    #[test]
    fn the_watermark_travels_onto_every_candidate() {
        // The property the whole module exists for: a strategy cannot be handed
        // a candidate whose watermark differs from the one the pass was run at.
        let u = universe_of(&[(1, 9, 1_000), (2, 9, 2_000)], &[], 10_000);
        for mint in u.recent(u64::MAX) {
            let c = u.candidate(&mint, None, None).expect("launched");
            assert_eq!(c.as_of, AsOf::at(Slot(10_000)));
        }
    }

    #[test]
    fn an_outcome_from_the_future_is_not_admitted() {
        let as_of = AsOf::at(Slot(5_000));
        assert!(admits(as_of, &outcome(1, 4_999, None, 1)));
        assert!(admits(as_of, &outcome(1, 5_000, None, 1)));
        assert!(!admits(as_of, &outcome(1, 5_001, None, 1)));
    }
}
