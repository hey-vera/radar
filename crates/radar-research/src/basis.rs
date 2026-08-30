// SPDX-License-Identifier: Apache-2.0
//! Is the selection's headline a market fact or a measurement artefact?
//!
//! [`selection`](crate::selection) reports a decision's return as the move from
//! [`Decision::entry_price`] to a later [`Outcome::last_price`]. Those two
//! numbers are produced by different instruments, on different sides of the
//! book, and the difference between the instruments is not zero.
//!
//! - The **entry** is the smallest rung of the exit probe's ladder: a *sell
//!   quote*, which is a bid, net of the router's fee and of pump.fun's own.
//! - The **exit** is `argMax(lam / tok, (ts, sig))` over realised fills. The
//!   price query filters by transfer type and by size, and deliberately **not by
//!   side** — so buys and sells are pooled and the figure sits near the mid.
//!
//! A bid compared against a mid is **positive before the market has moved at
//! all**. `selection`'s own module documentation says "both prices come from the
//! sell side", and the second half of that sentence is not true of the exit.
//!
//! On 2026-08-30 the measured gross median of that comparison was **+21 bps**
//! ([`0014`]). pump.fun charges roughly 1% a leg, so an artefact of the same
//! order as the entire signal is not a remote possibility — it is the default
//! expectation. This module measures it instead of arguing about it.
//!
//! # How the artefact is separated from the market
//!
//! For one decision, the quoted and realised prices cannot be compared at the
//! same instant: the quote is taken when the decision is made and a realised
//! price only exists where a fill happened. So the two are always separated by
//! some time, and over that time the market really does move.
//!
//! The separation is the point rather than the problem. **Pair each decision
//! with the outcome measured nearest to it in time, and bucket by that gap.**
//! Real movement grows with the gap; the instrument difference does not. A basis
//! that is flat across the buckets is an artefact. One that grows with the gap is
//! the market. One that is zero at the tightest bucket is no artefact at all.
//!
//! Note this pairs on the **nearest** observation in either direction, where
//! [`selection`](crate::selection) deliberately takes the *latest* one after the
//! decision. That difference is the whole method: `selection` asks what followed,
//! and this asks what was true at the same moment. Symmetry also matters — taking
//! only later observations would fold this market's downward drift into the
//! answer, which is exactly the contamination being measured.
//!
//! # What this cannot do
//!
//! It cannot separate the fee from the spread, because a realised fill price
//! already has the protocol fee inside it. The number here is the whole gap
//! between the two instruments, which is what `selection` needs subtracted, and
//! not a decomposition of why.
//!
//! [`0014`]: ../../docs/research/0014-the-control-was-entirely-tokens-nobody-could-sell.md
//! [`Decision::entry_price`]: radar_store::Decision::entry_price
//! [`Outcome::last_price`]: radar_store::Outcome::last_price

use radar_store::{Decision, Outcome};
use serde::Serialize;

/// The smallest cohort worth reporting a percentile for.
///
/// The same floor and the same reasoning as
/// [`selection::MIN_COHORT`](crate::selection::MIN_COHORT): below this the
/// honest output is how far there is to go, not a median over a handful of rows.
pub const MIN_COHORT: usize = 30;

/// The gap buckets, in slots, with the label each one reports under.
///
/// Roughly one minute, five, twenty and sixty at the chain's ~2.5 slots a
/// second. Held as data so a test can sweep the boundaries, and ordered so the
/// first bucket a gap fits is the one it lands in.
///
/// The tightest bucket is the one that matters. It is also the smallest, which
/// is the trade this measurement cannot escape: the closer the pairing, the less
/// market movement contaminates it and the fewer pairs there are.
pub const BUCKETS: [(&str, u64); 5] = [
    ("<=1m", 150),
    ("<=5m", 750),
    ("<=20m", 3_000),
    ("<=1h", 9_000),
    (">1h", u64::MAX),
];

/// One decision paired with the realised price nearest it in time.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
pub struct Pair {
    /// Slots between the decision and the observation, in either direction.
    pub gap_slots: u64,
    /// The realised price against the quoted one, in basis points.
    ///
    /// Signed. Positive means the realised price sat **above** the quote, which
    /// is the direction that flatters every return `selection` reports.
    pub basis_bps: i64,
}

/// One gap bucket's pairs.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct Bucket {
    /// The label from [`BUCKETS`].
    pub label: String,
    /// Basis readings in this bucket, ascending.
    pub basis_bps: Vec<i64>,
}

impl Bucket {
    /// How many pairs landed here.
    #[must_use]
    pub fn n(&self) -> usize {
        self.basis_bps.len()
    }

    /// The reading at a percentile, or `None` when the bucket is empty.
    #[must_use]
    pub fn percentile(&self, p: f64) -> Option<i64> {
        if self.basis_bps.is_empty() {
            return None;
        }
        let last = self.basis_bps.len() - 1;
        #[expect(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "an index into a cohort orders of magnitude below f64's exact integer range"
        )]
        let idx = ((self.basis_bps.len() as f64 * p) as usize).min(last);
        Some(self.basis_bps[idx])
    }

    /// The median, or `None` when the bucket is empty.
    #[must_use]
    pub fn median(&self) -> Option<i64> {
        self.percentile(0.50)
    }
}

/// What the measurement is able to say about the artefact.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum Verdict {
    /// Too few pairs in the tightest bucket to say anything.
    ///
    /// The tightest bucket is the only one that isolates the instrument, so a
    /// verdict drawn from a well-populated hour-wide bucket would be a claim
    /// about the market wearing this measurement's name.
    NotEnoughData {
        /// Pairs in the tightest bucket.
        paired: usize,
        /// How many more are needed.
        needed: usize,
    },
    /// The basis at the tightest gap, which is the correction `selection` owes.
    Measured {
        /// Median basis in the tightest bucket, in basis points.
        tightest_median_bps: i64,
        /// Pairs it was drawn from.
        tightest_n: usize,
        /// Median basis in the widest populated bucket, for comparison.
        ///
        /// If this is close to the tightest, the basis does not grow with time
        /// and is therefore an artefact rather than the market. If it is much
        /// larger, most of what the wide bucket sees is real movement.
        widest_median_bps: Option<i64>,
    },
}

/// The measurement.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct Report {
    /// Decisions carrying an entry price, which are the only pairable ones.
    pub with_entry: usize,
    /// Decisions that found a realised price to pair against.
    pub paired: usize,
    /// Pairs by gap bucket, in [`BUCKETS`] order.
    pub buckets: Vec<Bucket>,
}

impl Report {
    /// What can be said, given how much landed in the tightest bucket.
    #[must_use]
    pub fn verdict(&self) -> Verdict {
        let tightest = self.buckets.first();
        let n = tightest.map_or(0, Bucket::n);
        if n < MIN_COHORT {
            return Verdict::NotEnoughData {
                paired: n,
                needed: MIN_COHORT - n,
            };
        }
        Verdict::Measured {
            // `n >= MIN_COHORT` above, so the bucket is non-empty and the median
            // is present. Defaulting rather than unwrapping keeps a panic out of
            // a reporting path.
            tightest_median_bps: tightest.and_then(Bucket::median).unwrap_or(0),
            tightest_n: n,
            widest_median_bps: self
                .buckets
                .iter()
                .rev()
                .find(|b| b.n() >= MIN_COHORT)
                .and_then(Bucket::median),
        }
    }
}

/// Pairs every decision carrying an entry price with the realised price
/// measured nearest to it.
///
/// A decision with no entry price never reached the exit probe and has no quote
/// to compare; it is counted and skipped rather than scored as zero, for the
/// same reason [`selection`](crate::selection) refuses to — folding "not
/// measurable" into "no difference" would report the artefact as absent.
#[must_use]
pub fn measure(decisions: &[Decision], outcomes: &[Outcome]) -> Report {
    let mut buckets: Vec<Bucket> = BUCKETS
        .iter()
        .map(|(label, _)| Bucket {
            label: (*label).to_owned(),
            basis_bps: Vec::new(),
        })
        .collect();

    let (mut with_entry, mut paired) = (0usize, 0usize);

    for decision in decisions {
        let Some(quoted) = decision.entry_price else {
            continue;
        };
        with_entry += 1;
        // A quote of zero is not a price. Dividing by it would produce a number
        // with the shape of a basis and no content, which is the failure this
        // repository keeps meeting.
        if quoted == 0 {
            continue;
        }
        let Some(pair) = nearest(decision, quoted, outcomes) else {
            continue;
        };
        paired += 1;
        buckets[bucket_of(pair.gap_slots)]
            .basis_bps
            .push(pair.basis_bps);
    }

    for bucket in &mut buckets {
        bucket.basis_bps.sort_unstable();
    }

    Report {
        with_entry,
        paired,
        buckets,
    }
}

/// The index in [`BUCKETS`] a gap belongs to: the first whose ceiling it clears.
///
/// Total by construction — the last ceiling is `u64::MAX` — so this cannot fail
/// to place a gap, and the caller indexes without a bounds check.
#[must_use]
pub fn bucket_of(gap_slots: u64) -> usize {
    BUCKETS
        .iter()
        .position(|(_, ceiling)| gap_slots <= *ceiling)
        .unwrap_or(BUCKETS.len() - 1)
}

/// The realised price observed nearest this decision, in either direction.
///
/// Nearest rather than latest, which is the difference from
/// [`selection`](crate::selection) and the reason this module exists. An
/// observation an hour later is a fact about the market; one taken beside the
/// decision is a fact about the instruments.
fn nearest(decision: &Decision, quoted: u64, outcomes: &[Outcome]) -> Option<Pair> {
    let (gap_slots, realised) = outcomes
        .iter()
        .filter(|o| o.mint == decision.mint)
        .filter_map(|o| {
            let price = o.last_price?;
            // Absent, never zero (rule 9): a mint measured with no fill has no
            // price, and reading that as free would invert the basis.
            (price > 0).then(|| (gap(o.measured_at.get(), decision.decided_at.get()), price))
        })
        .min_by_key(|(gap, _)| *gap)?;

    // i128 throughout: a realised price can exceed the quote by orders of
    // magnitude on this venue, and a wrapped basis would still look like a
    // number.
    let delta = i128::from(realised) - i128::from(quoted);
    let basis_bps = i64::try_from(delta.saturating_mul(10_000) / i128::from(quoted)).ok()?;

    Some(Pair {
        gap_slots,
        basis_bps,
    })
}

/// Absolute distance between two slots.
///
/// Unsigned on purpose: the direction of the gap carries no information here,
/// and taking it in one direction only would discard half the pairs.
const fn gap(a: u64, b: u64) -> u64 {
    a.abs_diff(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use radar_store::Conclusion;
    use radar_types::{Address, Slot};

    fn decision(mint: u8, decided_at: u64, entry: Option<u64>) -> Decision {
        Decision {
            mint: Address::new([mint; 32]),
            creator: Address::new([99u8; 32]),
            decided_at: Slot(decided_at),
            launch_slot: Slot(1),
            strategy: "creator_edge".to_owned(),
            strategy_version: "0.1.0".to_owned(),
            conclusion: Conclusion::Proposed,
            reasons: Vec::new(),
            notional_micro_usd: None,
            exit_capacity_micro_usd: None,
            assumed_round_trip_bps: 850,
            coordination: None,
            authority_prevalence: None,
            kernel_outcome: None,
            kernel_reasons: Vec::new(),
            entry_price: entry,
            inputs_digest: "d".to_owned(),
        }
    }

    fn outcome(mint: u8, measured_at: u64, last: Option<u64>) -> Outcome {
        Outcome {
            mint: Address::new([mint; 32]),
            measured_at: Slot(measured_at),
            launch_slot: Slot(1),
            first_transfer_slot: None,
            last_transfer_slot: None,
            transfers: 0,
            unique_senders: 0,
            unique_receivers: 0,
            graduated_at: None,
            first_price: None,
            last_price: last,
            peak_price: None,
            trough_price: None,
            vwap: None,
            fills: 0,
        }
    }

    #[test]
    fn a_realised_price_above_the_quote_is_a_positive_basis() {
        // The direction that matters. A sell quote sits below a mid, so this is
        // the sign the artefact is expected to carry, and a basis reported with
        // the opposite sign would tell the reader to *add* the correction.
        let r = measure(
            &[decision(1, 1_000, Some(10_000))],
            &[outcome(1, 1_050, Some(10_100))],
        );
        assert_eq!(r.paired, 1);
        assert_eq!(r.buckets[0].median(), Some(100));
    }

    #[test]
    fn the_nearest_observation_wins_and_not_the_latest() {
        // The property that separates this from `selection::scored_return`,
        // which takes the *latest* observation after the decision. A mutant
        // swapping `min_by_key` for `max_by_key` passes every other test here
        // and fails this one.
        //
        // The far observation is deliberately the one that would give a large
        // basis, so picking it is loud rather than a rounding difference.
        let r = measure(
            &[decision(1, 1_000, Some(10_000))],
            &[
                outcome(1, 1_010, Some(10_050)),
                outcome(1, 90_000, Some(50_000)),
            ],
        );
        assert_eq!(r.buckets[0].median(), Some(50), "took the far observation");
        assert_eq!(r.buckets[0].n(), 1);
        assert!(r.buckets.iter().skip(1).all(|b| b.n() == 0));
    }

    #[test]
    fn an_observation_before_the_decision_pairs_just_as_well() {
        // Symmetry is deliberate: taking only later observations would fold this
        // market's downward drift into the basis, which is the contamination
        // being measured. A mutant that filters to `measured_at > decided_at`
        // fails here.
        let r = measure(
            &[decision(1, 10_000, Some(10_000))],
            &[outcome(1, 9_900, Some(10_200))],
        );
        assert_eq!(r.paired, 1);
        assert_eq!(r.buckets[0].median(), Some(200));
    }

    #[test]
    fn a_decision_with_no_entry_price_is_neither_counted_nor_paired() {
        // It never reached the exit probe, so there is no quote to compare. It
        // must not read as a basis of zero -- that would report the artefact as
        // absent on the strength of a decision that could not measure it.
        let r = measure(
            &[decision(1, 1_000, None)],
            &[outcome(1, 1_010, Some(10_000))],
        );
        assert_eq!(r.with_entry, 0);
        assert_eq!(r.paired, 0);
    }

    #[test]
    fn a_zero_price_on_either_side_is_refused_rather_than_divided() {
        // Rule 9: absent is not zero. A zero quote would divide, and a zero
        // realised price would report -10,000 bps -- a token that went to
        // nothing rather than one nobody priced.
        let zero_quote = measure(
            &[decision(1, 1_000, Some(0))],
            &[outcome(1, 1_010, Some(10_000))],
        );
        assert_eq!(zero_quote.with_entry, 1, "counted");
        assert_eq!(zero_quote.paired, 0, "not paired");

        let zero_realised = measure(
            &[decision(1, 1_000, Some(10_000))],
            &[outcome(1, 1_010, Some(0))],
        );
        assert_eq!(zero_realised.paired, 0);
    }

    #[test]
    fn an_outcome_for_a_different_mint_never_pairs() {
        let r = measure(
            &[decision(1, 1_000, Some(10_000))],
            &[outcome(2, 1_000, Some(10_000))],
        );
        assert_eq!(r.with_entry, 1);
        assert_eq!(r.paired, 0);
    }

    #[test]
    fn every_bucket_boundary_falls_on_the_tighter_side() {
        // Swept rather than sampled. A ceiling is inclusive, so a gap exactly at
        // one belongs to that bucket and one slot past it to the next -- an
        // off-by-one here silently moves pairs into a looser bucket, which is
        // the direction that hides the artefact.
        for (i, (_, ceiling)) in BUCKETS.iter().enumerate() {
            assert_eq!(bucket_of(*ceiling), i, "ceiling {ceiling} left its bucket");
            if *ceiling != u64::MAX {
                assert_eq!(bucket_of(ceiling + 1), i + 1, "past {ceiling} did not move");
            }
        }
        assert_eq!(bucket_of(0), 0);
        assert_eq!(bucket_of(u64::MAX), BUCKETS.len() - 1);
    }

    #[test]
    fn a_thin_tightest_bucket_refuses_to_report_however_full_the_rest_are() {
        // The tightest bucket is the only one that isolates the instrument. A
        // verdict drawn from a well-populated hour-wide bucket would be a claim
        // about the market wearing this measurement's name, so a full wide
        // bucket must not rescue an empty tight one.
        let decisions: Vec<_> = (0u64..40)
            .map(|i| decision(1, 100_000 + i, Some(10_000)))
            .collect();
        let outcomes: Vec<_> = (0u64..40)
            .map(|i| outcome(1, 100_000 + i + 20_000, Some(11_000)))
            .collect();

        let r = measure(&decisions, &outcomes);
        assert!(r.paired >= MIN_COHORT, "the wide bucket is well populated");
        assert!(matches!(
            r.verdict(),
            Verdict::NotEnoughData { paired: 0, .. }
        ));
    }

    #[test]
    fn a_full_tightest_bucket_reports_its_median() {
        let decisions: Vec<_> = (0..MIN_COHORT)
            .map(|i| decision(1, 1_000 + u64::try_from(i).unwrap(), Some(10_000)))
            .collect();
        // One outcome, close to all of them, so every pair lands in `<=1m`.
        let outcomes = vec![outcome(1, 1_020, Some(10_150))];

        let r = measure(&decisions, &outcomes);
        let Verdict::Measured {
            tightest_median_bps,
            tightest_n,
            ..
        } = r.verdict()
        else {
            panic!("expected a measurement");
        };
        assert_eq!(tightest_n, MIN_COHORT);
        assert_eq!(tightest_median_bps, 150);
    }
}
