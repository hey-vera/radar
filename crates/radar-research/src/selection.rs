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

/// The population median held-to-end return, in basis points.
///
/// Measured in research 0009 over 177 tokens and reproduced over 4,199. Any
/// selection has to beat this to be worth making.
pub const POPULATION_MEDIAN_BPS: i64 = -1_340;

/// The share of the population that finished above an 850 bps round trip, per
/// ten thousand.
pub const POPULATION_BEAT_COST_BPS: u64 = 890;

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
    /// The selection's median return, beside the population's.
    ///
    /// Deliberately not phrased as "better" or "worse". A median above the
    /// population's over a few hundred tokens in one regime is not an edge, and
    /// naming it one here would put the conclusion where the reader expects the
    /// evidence.
    Measured {
        /// The selection's median return in basis points, net of costs.
        selection_median_bps: i64,
        /// The population's, from research 0009.
        population_median_bps: i64,
        /// The share of the selection clearing costs, per ten thousand.
        selection_beat_cost_bps: u64,
        /// The population's, from research 0009.
        population_beat_cost_bps: u64,
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
        Verdict::Measured {
            selection_median_bps: self.proposed.median().unwrap_or(0) - cost,
            population_median_bps: POPULATION_MEDIAN_BPS,
            selection_beat_cost_bps: self.proposed.beat_cost_bps(self.cost_bps).unwrap_or(0),
            population_beat_cost_bps: POPULATION_BEAT_COST_BPS,
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
                population_median_bps: POPULATION_MEDIAN_BPS,
                selection_beat_cost_bps: 10_000,
                population_beat_cost_bps: POPULATION_BEAT_COST_BPS,
            }
        );
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
