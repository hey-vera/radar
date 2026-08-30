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
/// At the chain's ~2.5 slots a second these run from ten minutes to three
/// hours. Held as data so a test can sweep the boundaries, and ordered so the
/// first bucket a gap fits is the one it lands in.
///
/// **The range is chosen from the cadence, not from what would be nice.** The
/// outcome pass runs at `:17` and `radar consider` at `:37`, so no decision can
/// be paired with an observation closer than roughly twenty minutes, and the
/// first version of this constant spent its two tightest buckets on gaps that
/// cannot occur. Measured on the live store, every one of 1,779 pairs inside
/// twenty minutes sat between five and twenty, and a single bucket over that
/// span cannot show whether the basis is moving inside it. These subdivide the
/// range that exists.
pub const BUCKETS: [(&str, u64); 7] = [
    ("<=10m", 1_500),
    ("<=15m", 2_250),
    ("<=20m", 3_000),
    ("<=30m", 4_500),
    ("<=1h", 9_000),
    ("<=3h", 27_000),
    (">3h", u64::MAX),
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
    /// No bucket holds enough pairs to report a median.
    NotEnoughData {
        /// Pairs found across every bucket.
        paired: usize,
        /// How many more the largest bucket needs.
        needed: usize,
    },
    /// The basis at the tightest gap that could be measured.
    ///
    /// # Why this is a lower bound and not an estimate
    ///
    /// The basis at a gap is the instrument difference plus whatever the market
    /// did over that gap. Those cannot be separated within one bucket — but they
    /// can be bounded, and the direction of the market is known.
    ///
    /// This market drifts **down**: [`0011`] measures a population median
    /// held-to-last of −863 bps, and [`Report::drifts_down`] checks the same
    /// thing in this data rather than assuming it. A negative drift *subtracts*
    /// from the basis, so the figure measured at any positive gap is **less than**
    /// the instrument difference alone.
    ///
    /// So `tightest_median_bps` is a floor. The artefact is at least this large,
    /// and reporting it as an estimate would understate the correction that
    /// [`selection`](crate::selection) owes — in the direction that flatters the
    /// selection, which is the direction this whole module exists to catch.
    ///
    /// [`0011`]: ../../docs/research/0011-graduation-predicts-volatility-not-profit.md
    Measured {
        /// Median basis in the tightest populated bucket, in basis points.
        tightest_median_bps: i64,
        /// That bucket's label, so the reader knows the gap it was measured at.
        ///
        /// Load-bearing rather than decorative: a basis is only interpretable
        /// beside the gap it was taken over, and a bare number here would invite
        /// exactly the reading this type is shaped to prevent.
        tightest_label: String,
        /// Pairs it was drawn from.
        tightest_n: usize,
        /// Median basis in the widest populated bucket.
        ///
        /// The comparison that says which of the two components dominates. A
        /// figure close to the tightest means the basis does not move with time
        /// and is therefore instrument; a much lower one means the market is
        /// doing most of the work over the longer gap.
        widest_median_bps: Option<i64>,
        /// Whether the basis falls as the gap widens across populated buckets.
        ///
        /// The premise the lower-bound reading rests on, carried beside the
        /// conclusion so a reader can refuse it. If this is ever `false`, the
        /// drift is not negative in this sample and `tightest_median_bps` stops
        /// being a floor.
        drifts_down: bool,
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
    /// Populated buckets, tightest first.
    ///
    /// "Populated" means clearing [`MIN_COHORT`]. A bucket below it has a median
    /// with the shape of a finding and the content of noise, and this repository
    /// has been caught by exactly that three times.
    #[must_use]
    pub fn populated(&self) -> Vec<&Bucket> {
        self.buckets
            .iter()
            .filter(|b| b.n() >= MIN_COHORT)
            .collect()
    }

    /// Whether the basis falls as the gap widens, across populated buckets.
    ///
    /// Checked rather than assumed. The lower-bound reading of
    /// [`Verdict::Measured`] needs the market's contribution to be negative, and
    /// this is that premise measured in the same data as the conclusion.
    ///
    /// A single populated bucket cannot show a trend, so it reports `false` —
    /// the direction that withholds the stronger claim rather than granting it
    /// on no evidence.
    #[must_use]
    pub fn drifts_down(&self) -> bool {
        let medians: Vec<i64> = self.populated().iter().filter_map(|b| b.median()).collect();
        medians.len() >= 2 && medians.last() < medians.first()
    }

    /// What can be said, given how the pairs fell across the buckets.
    #[must_use]
    pub fn verdict(&self) -> Verdict {
        let populated = self.populated();
        let Some(tightest) = populated.first() else {
            let largest = self.buckets.iter().map(Bucket::n).max().unwrap_or(0);
            return Verdict::NotEnoughData {
                paired: self.paired,
                needed: MIN_COHORT.saturating_sub(largest),
            };
        };
        Verdict::Measured {
            tightest_median_bps: tightest.median().unwrap_or(0),
            tightest_label: tightest.label.clone(),
            tightest_n: tightest.n(),
            widest_median_bps: populated.last().and_then(|b| b.median()),
            drifts_down: self.drifts_down(),
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
    fn a_percentile_indexes_the_distribution_and_does_not_merely_return_a_member() {
        // Every other test here uses a fixture whose values are identical, so
        // any index returns the same number and the arithmetic is untested. This
        // one has a known, varied distribution and pins each percentile to a
        // distinct value, so `len * p` cannot be replaced by `len + p` or
        // `len / p` without landing somewhere else.
        let bucket = Bucket {
            label: "t".to_owned(),
            basis_bps: (0..100).map(|i| i * 10).collect(),
        };
        assert_eq!(bucket.percentile(0.00), Some(0));
        assert_eq!(bucket.percentile(0.25), Some(250));
        assert_eq!(bucket.median(), Some(500));
        assert_eq!(bucket.percentile(0.75), Some(750));
    }

    #[test]
    fn the_top_percentile_clamps_instead_of_indexing_past_the_end() {
        // `len * 1.0` is `len`, which is one past the last element. The `.min`
        // is what saves it, and `last = len - 1` is what `.min` clamps to -- so a
        // mutation of either is a panic on a call that is perfectly legitimate.
        // Nothing else in this module asks for the top of a distribution, which
        // is exactly why it went untested.
        let bucket = Bucket {
            label: "t".to_owned(),
            basis_bps: vec![-5, 0, 5],
        };
        assert_eq!(bucket.percentile(1.0), Some(5), "the maximum, not a panic");
        assert_eq!(bucket.percentile(2.0), Some(5), "clamped, not indexed");
    }

    #[test]
    fn an_unchanged_basis_across_buckets_is_not_a_downward_drift() {
        // Flat is not falling. The distinction is load-bearing rather than
        // pedantic: `drifts_down` is the premise that lets the tightest bucket be
        // read as a FLOOR, and a flat basis cannot distinguish "the drift is
        // zero" from "these buckets have no resolution". Withholding the
        // stronger claim is the direction that under-claims.
        let mut decisions = Vec::new();
        let mut outcomes = Vec::new();
        for i in 0u64..40 {
            decisions.push(decision(1, 1_000 + i, Some(10_000)));
            outcomes.push(outcome(1, 1_000 + i, Some(10_500)));
            decisions.push(decision(2, 1_000 + i, Some(10_000)));
            // Same basis, far larger gap.
            outcomes.push(outcome(2, 1_000 + i + 20_000, Some(10_500)));
        }
        let r = measure(&decisions, &outcomes);
        assert_eq!(r.populated().len(), 2, "two buckets to compare");
        assert!(!r.drifts_down(), "equal medians are not a fall");
    }

    #[test]
    fn the_last_bucket_catches_every_gap_so_the_search_cannot_fail() {
        // `bucket_of` falls back to the last index if no ceiling matches, and
        // that fallback is unreachable *because* the last ceiling is `u64::MAX`.
        // The unreachability is the reason two mutants of it are filed as
        // equivalent in `.cargo/mutants.toml`, so it is asserted here rather
        // than assumed: change the last ceiling and this fails, which is the
        // signal to re-examine that exemption instead of trusting it.
        assert_eq!(
            BUCKETS.last().expect("buckets are not empty").1,
            u64::MAX,
            "the last ceiling must admit every gap"
        );
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
    fn a_wide_only_measurement_is_labelled_rather_than_passed_off_as_tight() {
        // The first version of this module refused outright when the tightest
        // bucket was empty. Run against the live store, the two tightest buckets
        // were empty *by construction* -- the outcome pass runs at :17 and
        // `consider` at :37, so no pair can be closer than about twenty minutes
        // -- and a flat refusal threw away 1,779 usable pairs.
        //
        // The honest form is to report the tightest bucket that has data and
        // name the gap it was taken over, so the reader can discount it. A bare
        // number is what would mislead; a labelled one cannot.
        let decisions: Vec<_> = (0u64..40)
            .map(|i| decision(1, 100_000 + i, Some(10_000)))
            .collect();
        let outcomes: Vec<_> = (0u64..40)
            .map(|i| outcome(1, 100_000 + i + 20_000, Some(11_000)))
            .collect();

        let r = measure(&decisions, &outcomes);
        let Verdict::Measured { tightest_label, .. } = r.verdict() else {
            panic!("expected a measurement from a populated wide bucket");
        };
        assert_eq!(tightest_label, "<=3h", "the gap must be named, not hidden");
    }

    #[test]
    fn nothing_is_reported_when_no_bucket_clears_the_floor() {
        let r = measure(
            &[decision(1, 1_000, Some(10_000))],
            &[outcome(1, 1_010, Some(10_100))],
        );
        assert!(matches!(
            r.verdict(),
            Verdict::NotEnoughData { paired: 1, .. }
        ));
    }

    #[test]
    fn the_drift_premise_is_measured_and_not_assumed() {
        // `Verdict::Measured`'s lower-bound reading needs the market's
        // contribution to be negative. That is checked in the same data as the
        // conclusion, so a sample where it does not hold withholds the claim
        // rather than making it anyway.
        //
        // Tight pairs read +500, wide pairs read -500: the basis falls with the
        // gap, which is the shape the live store shows (+128 -> +72 -> -72).
        let mut decisions = Vec::new();
        let mut outcomes = Vec::new();
        for i in 0u64..40 {
            decisions.push(decision(1, 1_000 + i, Some(10_000)));
            outcomes.push(outcome(1, 1_000 + i, Some(10_500)));
            decisions.push(decision(2, 1_000 + i, Some(10_000)));
            outcomes.push(outcome(2, 1_000 + i + 20_000, Some(9_500)));
        }
        let r = measure(&decisions, &outcomes);
        assert!(r.drifts_down(), "tight +500 above wide -500 is a fall");

        // Inverted: the wide bucket reads higher, so the premise fails and the
        // floor claim is withheld.
        let rising: Vec<Outcome> = outcomes
            .iter()
            .map(|o| {
                let mut o = o.clone();
                o.last_price = Some(if o.measured_at.get() > 15_000 {
                    10_500
                } else {
                    9_500
                });
                o
            })
            .collect();
        let r = measure(&decisions, &rising);
        assert!(!r.drifts_down(), "a rising basis is not a downward drift");
    }

    #[test]
    fn one_populated_bucket_cannot_show_a_trend() {
        // Withholding the stronger claim on no evidence, rather than granting it.
        let decisions: Vec<_> = (0u64..40)
            .map(|i| decision(1, 1_000 + i, Some(10_000)))
            .collect();
        let outcomes = vec![outcome(1, 1_020, Some(10_150))];
        assert!(!measure(&decisions, &outcomes).drifts_down());
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
