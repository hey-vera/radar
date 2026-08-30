// SPDX-License-Identifier: Apache-2.0
//! Did the selection beat the population it selected from?
//!
//! This is the question the project exists to answer, and until decisions were
//! recorded it could not be put at all. [Research 0009] measured the population:
//! a median held-to-end of **−13.4%** before costs, with **8.9%** of tokens
//! finishing above the 850 bps round trip. Radar's thesis is that refusing most
//! of that population is worth doing. This module is where that stops being a
//! thesis.
//!
//! # What is being compared, exactly
//!
//! The entry is [`Decision::entry_price`], recorded at the watermark the
//! decision was taken — **not** the token's first fill. `creator_edge` acts
//! around forty minutes after launch, and 0009 says plainly that entry at the
//! first fill is not Radar's entry. Measuring from the first fill would credit
//! Radar with a move it was not present for.
//!
//! The exit is the last price the outcome pass observed **after** the decision.
//! An outcome measured before the decision describes a market the decision had
//! not seen, and using it would be look-ahead pointing backwards.
//!
//! Both prices come from the sell side: the entry from the smallest rung of the
//! exit probe's ladder, the exit from realised fills. Bid-to-bid is the right
//! measure of what a position was worth, and the round-trip friction of getting
//! in and out is the separate `cost_bps` term — which is why it is subtracted
//! once rather than being folded into either price.
//!
//! # Why it refuses to report a small sample
//!
//! A percentile over eleven rows is a number with the shape of a finding and the
//! content of noise, and this repository has been caught by exactly that
//! ([LEARNINGS] entries 7, 10 and 11). [`Report::verdict`] returns
//! [`Verdict::NotEnoughData`] below [`MIN_COHORT`], and says how many more are
//! needed rather than printing percentiles nobody should read.
//!
//! [Research 0009]: ../../docs/research/0009-what-a-token-actually-does-to-your-money.md
//! [LEARNINGS]: https://github.com/hey-vera/radar/blob/main/LEARNINGS.md

use radar_store::{Conclusion, Decision, Outcome};
use serde::Serialize;

/// The population median held-to-end return, in basis points, as research 0009
/// measured it.
///
/// **Context, not the comparison.** It is measured a different way: entry at the
/// token's *first fill*, exit at the last price in a six-hour window. Radar's
/// cohort enters at the price its decision saw, around forty minutes later, and
/// exits at whatever the outcome pass last observed. Those two numbers cannot be
/// subtracted from each other, and an earlier version of this module put them
/// side by side as though they could.
///
/// It also moves with the cohort. The same query over 4,199 priced mints gave
/// −1,340; over 59,647 it gives −863, because `last_price` depends on which
/// checkpoint a token was last measured at, and the mix of checkpoint ages
/// changes as the store grows. A constant cannot track that.
///
/// The real comparison is [`Report::refused`] — the tokens Radar looked at,
/// priced the same way, in the same passes, and declined.
pub const POPULATION_MEDIAN_BPS_0009: i64 = -1_340;

/// The share of research 0009's population that finished above an 850 bps round
/// trip, per ten thousand. Context, on the same caveat as
/// [`POPULATION_MEDIAN_BPS_0009`].
pub const POPULATION_BEAT_COST_BPS_0009: u64 = 890;

/// The smallest cohort worth reporting percentiles for.
///
/// Not a statistical threshold so much as a refusal to be interesting too early.
/// Below this the honest output is how far there is to go.
pub const MIN_COHORT: usize = 30;

/// One cohort's realised returns.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct Cohort {
    /// How many decisions fell into this cohort at all.
    pub decisions: usize,
    /// How many of those could be scored — an entry price and a later
    /// observation. Never inferred: a decision with no exit probe has no entry.
    pub scored: usize,
    /// Returns in basis points from entry to the last observed price, ascending.
    ///
    /// Gross. Costs are applied by the caller through
    /// [`Report::cost_bps`](Report::cost_bps), so the same numbers can be read
    /// before and after them.
    pub returns_bps: Vec<i64>,
}

impl Cohort {
    /// The return at a percentile, or `None` when nothing was scored.
    #[must_use]
    pub fn percentile(&self, p: f64) -> Option<i64> {
        if self.returns_bps.is_empty() {
            return None;
        }
        let last = self.returns_bps.len() - 1;
        #[expect(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "an index into a cohort that is orders of magnitude below \
                      f64's exact integer range"
        )]
        let idx = ((self.returns_bps.len() as f64 * p) as usize).min(last);
        Some(self.returns_bps[idx])
    }

    /// The median, or `None` when nothing was scored.
    #[must_use]
    pub fn median(&self) -> Option<i64> {
        self.percentile(0.50)
    }

    /// How many cleared `cost_bps` net.
    #[must_use]
    pub fn beat_cost(&self, cost_bps: u64) -> usize {
        let cost = i64::try_from(cost_bps).unwrap_or(i64::MAX);
        self.returns_bps.iter().filter(|r| **r > cost).count()
    }

    /// The share that cleared `cost_bps`, per ten thousand, or `None` when
    /// nothing was scored.
    ///
    /// `None` rather than zero: a cohort nobody could score has no rate, and
    /// reporting one as zero would read as "none of them made money".
    #[must_use]
    pub fn beat_cost_bps(&self, cost_bps: u64) -> Option<u64> {
        if self.returns_bps.is_empty() {
            return None;
        }
        let hits = u64::try_from(self.beat_cost(cost_bps)).unwrap_or(u64::MAX);
        let total = u64::try_from(self.returns_bps.len()).unwrap_or(1);
        Some(hits.saturating_mul(10_000) / total)
    }
}

/// What the comparison is able to say.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum Verdict {
    /// Too few scored decisions to report anything.
    ///
    /// The only honest answer early, and the one this module will give for some
    /// time: recording began on 2026-08-26 and a pass records at most a few
    /// dozen.
    NotEnoughData {
        /// How many were scored.
        scored: usize,
        /// How many are needed.
        needed: usize,
    },
    /// The proposed cohort's median return, beside the refused cohort's.
    ///
    /// Both are priced the same way — entry at the watermark the decision was
    /// taken, exit at the last observation after it — and both come from the
    /// same passes over the same universe. That is what makes a difference
    /// between them attributable to the selection rather than to the
    /// measurement.
    ///
    /// Deliberately not phrased as "better" or "worse". A median above the
    /// control's over a few hundred tokens in one regime is not an edge, and
    /// naming it one here would put the conclusion where the reader expects the
    /// evidence.
    Measured {
        /// The proposed cohort's median return in basis points, net of costs.
        selection_median_bps: i64,
        /// The refused cohort's, net of costs — the matched control.
        ///
        /// `None` when too few refusals were scoreable to compare against, which
        /// is a different state from a control that broke even.
        control_median_bps: Option<i64>,
        /// The share of the proposed cohort clearing costs, per ten thousand.
        selection_beat_cost_bps: u64,
        /// The refused cohort's share, per ten thousand.
        control_beat_cost_bps: Option<u64>,
    },
}

/// The comparison.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct Report {
    /// Decisions considered.
    pub decisions: usize,
    /// Decisions whose token was later observed at a price.
    pub scored: usize,
    /// Decisions Radar proposed acting on.
    pub proposed: Cohort,
    /// Decisions Radar passed over, which is the counterfactual its thesis
    /// rests on.
    pub refused: Cohort,
    /// The round-trip cost applied, per ten thousand.
    pub cost_bps: u64,
}

impl Report {
    /// What can be said, given how much was scored.
    #[must_use]
    pub fn verdict(&self) -> Verdict {
        let scored = self.proposed.scored;
        if scored < MIN_COHORT {
            return Verdict::NotEnoughData {
                scored,
                needed: MIN_COHORT - scored,
            };
        }
        let cost = i64::try_from(self.cost_bps).unwrap_or(i64::MAX);
        // The control is held to the same cohort floor. A "control" of four
        // tokens would be a number with no content, and putting it beside a real
        // one would lend it the other's credibility.
        let control_ready = self.refused.scored >= MIN_COHORT;
        Verdict::Measured {
            selection_median_bps: self.proposed.median().unwrap_or(0) - cost,
            control_median_bps: control_ready.then(|| self.refused.median().unwrap_or(0) - cost),
            selection_beat_cost_bps: self.proposed.beat_cost_bps(self.cost_bps).unwrap_or(0),
            control_beat_cost_bps: control_ready
                .then(|| self.refused.beat_cost_bps(self.cost_bps).unwrap_or(0)),
        }
    }
}

/// Joins decisions to what their tokens went on to do.
///
/// `cost_bps` is the assumed round trip, subtracted once when a verdict is
/// formed rather than baked into the stored returns, so the same cohort can be
/// read gross and net.
#[must_use]
pub fn evaluate(decisions: &[Decision], outcomes: &[Outcome], cost_bps: u64) -> Report {
    let mut proposed = Cohort {
        decisions: 0,
        scored: 0,
        returns_bps: Vec::new(),
    };
    let mut refused = proposed.clone();

    for decision in decisions {
        let cohort = if matches!(decision.conclusion, Conclusion::Proposed) {
            &mut proposed
        } else {
            &mut refused
        };
        cohort.decisions += 1;

        // Only an observation taken *after* the decision says anything about
        // what followed it. An earlier one describes a market the decision had
        // not seen.
        let Some(later) = outcomes
            .iter()
            .filter(|o| o.mint == decision.mint && o.measured_at > decision.decided_at)
            .filter_map(|o| o.last_price.map(|p| (o.measured_at, p)))
            .max_by_key(|(at, _)| *at)
            .map(|(_, price)| price)
        else {
            continue;
        };
        let Some(bps) = decision.return_bps(later) else {
            continue;
        };
        cohort.scored += 1;
        cohort.returns_bps.push(bps);
    }

    proposed.returns_bps.sort_unstable();
    refused.returns_bps.sort_unstable();

    Report {
        decisions: decisions.len(),
        scored: proposed.scored + refused.scored,
        proposed,
        refused,
        cost_bps,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use radar_types::{Address, Slot};

    fn decision(mint: u8, conclusion: Conclusion, entry: Option<u64>) -> Decision {
        Decision {
            mint: Address::new([mint; 32]),
            creator: Address::new([99u8; 32]),
            decided_at: Slot(10_000),
            launch_slot: Slot(4_000),
            strategy: "creator_edge".to_owned(),
            strategy_version: "0.1.0".to_owned(),
            conclusion,
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

    fn outcome(mint: u8, measured_at: u64, last_price: Option<u64>) -> Outcome {
        Outcome {
            mint: Address::new([mint; 32]),
            measured_at: Slot(measured_at),
            launch_slot: Slot(4_000),
            first_transfer_slot: None,
            last_transfer_slot: None,
            transfers: 0,
            unique_senders: 0,
            unique_receivers: 0,
            graduated_at: None,
            first_price: Some(500),
            last_price,
            peak_price: None,
            trough_price: None,
            vwap: None,
            fills: 3,
        }
    }

    #[test]
    fn the_population_baseline_is_a_loss_and_stays_one() {
        // Deleting the minus sign turns the bar Radar has to clear from -13.4%
        // into +13.4%, which inverts every comparison this module exists to
        // make. Nothing else in the suite reads the sign.
        //
        // Equality rather than `< 0`: an `assert!` over a constant is a lint,
        // and pinning the value kills the same mutant.
        assert_eq!(
            POPULATION_MEDIAN_BPS_0009, -1_340,
            "-13.4%, from research 0009"
        );
        assert_eq!(
            POPULATION_BEAT_COST_BPS_0009, 890,
            "8.9% of that population cleared an 850 bps round trip"
        );
    }

    #[test]
    fn breaking_even_exactly_on_costs_does_not_count_as_beating_them() {
        // The boundary is the whole point of the figure: a round trip that
        // returns precisely its own cost has made nothing. Counting it would
        // overstate the share that cleared costs by every token sitting exactly
        // on the line.
        let cohort = Cohort {
            decisions: 3,
            scored: 3,
            returns_bps: vec![849, 850, 851],
        };
        assert_eq!(cohort.beat_cost(850), 1, "only the 851 cleared it");
        assert_eq!(cohort.beat_cost_bps(850), Some(3_333));
    }

    #[test]
    fn exactly_the_minimum_cohort_is_enough_to_report() {
        // Off by one here either withholds a result that is ready or reports one
        // that is not. Both directions are checked, because a threshold tested
        // on one side is a threshold half tested.
        let cohort = |n: usize| Cohort {
            decisions: n,
            scored: n,
            returns_bps: vec![100; n],
        };
        let report = |n: usize| Report {
            decisions: n,
            scored: n,
            proposed: cohort(n),
            refused: Cohort {
                decisions: 0,
                scored: 0,
                returns_bps: Vec::new(),
            },
            cost_bps: 850,
        };

        assert!(
            matches!(
                report(MIN_COHORT - 1).verdict(),
                Verdict::NotEnoughData { .. }
            ),
            "one short is not enough"
        );
        assert!(
            matches!(report(MIN_COHORT).verdict(), Verdict::Measured { .. }),
            "exactly the minimum is enough"
        );
    }

    #[test]
    fn an_outcome_measured_at_the_decision_slot_is_not_after_it() {
        // The tightest boundary in the module. An outcome stamped with exactly
        // the decision's watermark describes the market AT the decision, not
        // what followed it -- scoring against it would measure a return of
        // roughly nothing and dilute the cohort with non-observations.
        let decisions = vec![decision(1, Conclusion::Proposed, Some(1_000))];

        let at_the_decision = vec![outcome(1, 10_000, Some(5_000))];
        assert_eq!(
            evaluate(&decisions, &at_the_decision, 850).proposed.scored,
            0,
            "an observation at the decision's own watermark says nothing about what followed"
        );

        let one_slot_later = vec![outcome(1, 10_001, Some(5_000))];
        assert_eq!(
            evaluate(&decisions, &one_slot_later, 850).proposed.scored,
            1,
            "and one slot later does"
        );
    }

    #[test]
    fn an_observation_taken_before_the_decision_does_not_score_it() {
        // The decision had not seen that market. Scoring against it would be
        // look-ahead pointing backwards, and it would usually flatter the
        // result, because the earlier price is nearer the launch.
        let decisions = vec![decision(1, Conclusion::Proposed, Some(1_000))];
        let outcomes = vec![outcome(1, 9_000, Some(5_000))];

        let report = evaluate(&decisions, &outcomes, 850);
        assert_eq!(report.proposed.decisions, 1, "the decision is counted");
        assert_eq!(
            report.proposed.scored, 0,
            "but nothing after it was observed, so it cannot be scored"
        );
    }

    #[test]
    fn the_latest_observation_after_the_decision_is_the_one_used() {
        // Outcomes accumulate at checkpoints, so a token has several. The most
        // recent is the one that says where it ended up.
        let decisions = vec![decision(1, Conclusion::Proposed, Some(1_000))];
        let outcomes = vec![
            outcome(1, 11_000, Some(2_000)),
            outcome(1, 60_000, Some(500)),
            outcome(1, 30_000, Some(1_500)),
        ];

        let report = evaluate(&decisions, &outcomes, 850);
        assert_eq!(report.proposed.returns_bps, vec![-5_000], "ended at half");
    }

    #[test]
    fn refusals_are_scored_too_because_they_are_the_counterfactual() {
        // Radar's thesis is that refusing is worth doing. A report that scored
        // only proposals could say what the tokens it liked did, and nothing
        // about whether refusing the rest was right.
        let decisions = vec![
            decision(1, Conclusion::Proposed, Some(1_000)),
            decision(2, Conclusion::Passed, Some(1_000)),
        ];
        let outcomes = vec![
            outcome(1, 60_000, Some(1_200)),
            outcome(2, 60_000, Some(300)),
        ];

        let report = evaluate(&decisions, &outcomes, 850);
        assert_eq!(report.proposed.returns_bps, vec![2_000]);
        assert_eq!(report.refused.returns_bps, vec![-7_000]);
        assert_eq!(report.scored, 2);
    }

    #[test]
    fn a_decision_with_no_entry_price_is_counted_and_not_scored() {
        // A refusal that never reached the exit probe has no entry, so it has no
        // return. Counting it as zero would put break-even into a population
        // whose median is well below it.
        let decisions = vec![decision(1, Conclusion::Passed, None)];
        let outcomes = vec![outcome(1, 60_000, Some(9_999))];

        let report = evaluate(&decisions, &outcomes, 850);
        assert_eq!(report.refused.decisions, 1);
        assert_eq!(report.refused.scored, 0);
        assert!(report.refused.returns_bps.is_empty());
    }

    #[test]
    fn a_small_cohort_reports_how_far_short_it_is_rather_than_a_percentile() {
        // The failure this module is most likely to cause is being interesting
        // too early. A median over three rows has the shape of a finding and the
        // content of noise -- LEARNINGS 7, 10 and 11.
        let decisions: Vec<Decision> = (1..=3)
            .map(|i| decision(i, Conclusion::Proposed, Some(1_000)))
            .collect();
        let outcomes: Vec<Outcome> = (1..=3).map(|i| outcome(i, 60_000, Some(5_000))).collect();

        let report = evaluate(&decisions, &outcomes, 850);
        assert_eq!(
            report.verdict(),
            Verdict::NotEnoughData {
                scored: 3,
                needed: MIN_COHORT - 3
            },
            "a 400% median over three tokens must not be reported as a result"
        );
    }

    #[test]
    fn a_full_cohort_reports_the_median_net_of_costs_beside_the_population() {
        // The comparison is the point. A selection median quoted without the
        // population's is a number with nothing to beat.
        let decisions: Vec<Decision> = (1..=40)
            .map(|i| decision(i, Conclusion::Proposed, Some(1_000)))
            .collect();
        // Every token doubles: +10,000 bps gross, +9,150 net of an 850 bps trip.
        let outcomes: Vec<Outcome> = (1..=40).map(|i| outcome(i, 60_000, Some(2_000))).collect();

        let report = evaluate(&decisions, &outcomes, 850);
        assert_eq!(
            report.verdict(),
            Verdict::Measured {
                selection_median_bps: 9_150,
                // No refusals were scored, so there is nothing to compare
                // against -- and saying so beats quoting a figure measured a
                // different way as though it were a control.
                control_median_bps: None,
                selection_beat_cost_bps: 10_000,
                control_beat_cost_bps: None,
            }
        );
    }

    #[test]
    fn the_control_is_the_refusals_priced_the_same_way() {
        // The comparison that means something. Both cohorts enter at the price
        // their own decision saw and exit at the last observation after it, in
        // the same passes over the same universe -- so a difference between them
        // is attributable to the selection rather than to the measurement.
        //
        // An earlier version compared against research 0009's population median,
        // which enters at the token's FIRST FILL. Those are not the same
        // quantity and subtracting one from the other was meaningless.
        let mut decisions = Vec::new();
        let mut outcomes = Vec::new();
        for i in 0..40u8 {
            decisions.push(decision(i, Conclusion::Proposed, Some(1_000)));
            outcomes.push(outcome(i, 60_000, Some(1_500)));
        }
        for i in 40..80u8 {
            decisions.push(decision(i, Conclusion::Passed, Some(1_000)));
            outcomes.push(outcome(i, 60_000, Some(500)));
        }

        let report = evaluate(&decisions, &outcomes, 850);
        let Verdict::Measured {
            selection_median_bps,
            control_median_bps,
            ..
        } = report.verdict()
        else {
            panic!("both cohorts clear the floor")
        };
        assert_eq!(
            selection_median_bps, 4_150,
            "+50% gross less an 850 bps trip"
        );
        assert_eq!(
            control_median_bps,
            Some(-5_850),
            "the refusals halved, and are reported net on the same basis"
        );
    }

    #[test]
    fn a_control_too_small_to_mean_anything_is_withheld() {
        // A control of four tokens beside a cohort of forty would borrow the
        // larger one's credibility. Absent is the honest report.
        let mut decisions: Vec<Decision> = (0..40u8)
            .map(|i| decision(i, Conclusion::Proposed, Some(1_000)))
            .collect();
        let mut outcomes: Vec<Outcome> =
            (0..40u8).map(|i| outcome(i, 60_000, Some(1_500))).collect();
        for i in 40..44u8 {
            decisions.push(decision(i, Conclusion::Passed, Some(1_000)));
            outcomes.push(outcome(i, 60_000, Some(500)));
        }

        let Verdict::Measured {
            control_median_bps,
            control_beat_cost_bps,
            ..
        } = evaluate(&decisions, &outcomes, 850).verdict()
        else {
            panic!("the proposed cohort clears the floor")
        };
        assert_eq!(control_median_bps, None, "four refusals is not a control");
        assert_eq!(control_beat_cost_bps, None);
    }

    #[test]
    fn costs_are_subtracted_once_and_are_visible_in_the_verdict() {
        // Gross returns are stored so the same cohort can be read either way,
        // and the cost is applied where a verdict is formed. Folding it into the
        // stored numbers would make a later change of cost assumption
        // unauditable -- and that assumption moved by a factor of four on
        // 2026-08-25.
        let decisions: Vec<Decision> = (1..=40)
            .map(|i| decision(i, Conclusion::Proposed, Some(1_000)))
            .collect();
        let outcomes: Vec<Outcome> = (1..=40).map(|i| outcome(i, 60_000, Some(1_100))).collect();

        let gross = evaluate(&decisions, &outcomes, 0);
        let net = evaluate(&decisions, &outcomes, 850);
        assert_eq!(gross.proposed.median(), Some(1_000), "gross is unchanged");
        assert_eq!(
            net.proposed.median(),
            Some(1_000),
            "and stored gross either way"
        );

        let Verdict::Measured {
            selection_median_bps,
            ..
        } = net.verdict()
        else {
            panic!("forty scored decisions is a measurable cohort")
        };
        assert_eq!(
            selection_median_bps, 150,
            "1,000 gross less an 850 bps trip"
        );
    }

    #[test]
    fn nothing_scored_reports_no_rate_rather_than_a_rate_of_zero() {
        // Absent is not zero. "No cohort" and "a cohort where none made money"
        // are opposite findings, and the second is a claim about the market.
        let empty = Cohort {
            decisions: 5,
            scored: 0,
            returns_bps: Vec::new(),
        };
        assert_eq!(empty.median(), None);
        assert_eq!(empty.beat_cost_bps(850), None);
        assert_eq!(empty.percentile(0.9), None);
    }

    #[test]
    fn the_percentile_index_stays_inside_the_cohort() {
        // The hundredth percentile is the largest element, not one past the end.
        let cohort = Cohort {
            decisions: 3,
            scored: 3,
            returns_bps: vec![-100, 0, 100],
        };
        assert_eq!(cohort.percentile(1.0), Some(100));
        assert_eq!(cohort.percentile(0.0), Some(-100));
        assert_eq!(cohort.median(), Some(0));
    }
}
