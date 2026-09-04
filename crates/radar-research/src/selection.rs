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
//! **The two prices are not measured the same way, and an earlier version of
//! this paragraph claimed they were.** It said "both prices come from the sell
//! side". The entry does: it is the smallest rung of the exit probe's ladder, a
//! sell quote. The exit does not — it is `argMax(lam / tok, (ts, sig))` over
//! realised fills, which pools buys and sells and therefore sits near the mid.
//!
//! A bid measured against a mid is positive before the market has moved at all,
//! so every return this module reports carries an upward artefact.
//! [`0016`](../../docs/research/0016-the-entry-was-a-bid-and-the-exit-was-a-mid.md)
//! measures it at **at least +128 bps**, against a gross median here of +21 —
//! six times the signal, in the direction that flatters the selection. Read
//! [`basis`](crate::basis) before reading any figure below.
//!
//! It is deliberately **not** subtracted here. 0016's figure is a floor rather
//! than a point estimate, and baking a floor into a headline would overclaim in
//! the opposite direction. The fix is to stop comparing across instruments —
//! price the exit sell-side, or record a contemporaneous mid — not to apply a
//! correction after the fact.
//!
//! The round-trip friction of getting in and out is the separate `cost_bps`
//! term, which is why it is subtracted once rather than folded into either
//! price.
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

use std::collections::BTreeMap;

use radar_store::{Conclusion, Decision, Outcome};
use radar_types::{Address, Slot};
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
    // Indexed once. Scanning the table per decision is four billion comparisons
    // against the live store and was measured at sixty seconds.
    let priced = LastPriced::of(outcomes);
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
        let Some(bps) = scored_return(decision, &priced) else {
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

/// The proposed cohort, split by whether the coordination gate actually ran.
///
/// # Why this split and not another
///
/// `consider.rs` records `coordination = None` when CryptoHouse cannot serve the
/// launch block, and `creator_edge` correctly declines to let `None` refuse —
/// inventing evidence would refuse the whole population every time the vendor
/// hiccups. The consequence is that a candidate whose block went unread is
/// proposed **without** the screen [`0008`] measures at 11.7× on instant
/// graduation.
///
/// Measured on 2026-08-30 over 2,706 paid-tier candidates, 528 had an unreadable
/// launch block, and they were proposed at **55.0%** against 51.8% overall. So
/// roughly a fifth of the cohort every figure in this module reports was never
/// screened at all, and the unscreened share was proposed slightly *more* often
/// rather than less.
///
/// That makes the headline a blend of two populations — one Radar selected and
/// one it merely failed to reject — and those are different claims. This
/// separates them, and it needs no re-run because the column was recorded all
/// along.
///
/// Returns `(screened, unscreened)`.
///
/// [`0008`]: ../../docs/research/0008-the-launch-block-gives-the-bundle-away.md
#[must_use]
pub fn by_screening(decisions: &[Decision], outcomes: &[Outcome]) -> (Cohort, Cohort) {
    // Indexed once, as `evaluate` does and for the same reason.
    let priced = LastPriced::of(outcomes);
    let empty = || Cohort {
        decisions: 0,
        scored: 0,
        returns_bps: Vec::new(),
    };
    let (mut screened, mut unscreened) = (empty(), empty());

    for decision in decisions {
        if !matches!(decision.conclusion, Conclusion::Proposed) {
            continue;
        }
        // `Some` means a verdict was reached, whatever it was. `None` means the
        // block could not be read -- which is the distinction here, not whether
        // coordination was found.
        let cohort = if decision.coordination.is_some() {
            &mut screened
        } else {
            &mut unscreened
        };
        cohort.decisions += 1;
        if let Some(bps) = scored_return(decision, &priced) {
            cohort.scored += 1;
            cohort.returns_bps.push(bps);
        }
    }

    screened.returns_bps.sort_unstable();
    unscreened.returns_bps.sort_unstable();
    (screened, unscreened)
}

/// The refused cohort, split by the reason it was refused.
///
/// # The question this exists to answer
///
/// `evaluate` compares Radar's proposals to its refusals, and on 2026-08-30 the
/// refusals came out **ahead**: median −462 bps against −829, and 46.5% clearing
/// costs against 8.1%. Read flat, that says the selection is anti-predictive.
///
/// It may not say that. Refusals are not one population. A token refused for
/// `NoExitSimulated` or because its exit capacity was below the floor was
/// refused *precisely because nobody could have sold it* — and a paper return on
/// a token with no exit is not money anyone could have taken. That cohort's p75
/// of +6,059 bps is exactly the shape of unrealisable gains.
///
/// So the flat comparison cannot distinguish "the selection is wrong" from "the
/// control is flattered by returns that could not be realised", and those want
/// opposite responses. This splits it.
///
/// One decision contributes to **every** reason it carries, so the groups
/// overlap and their sizes do not sum to the cohort. That is deliberate: the
/// question is "what did tokens refused for X do", not "what did tokens refused
/// only for X do", and the second is a much smaller and stranger population.
#[must_use]
pub fn by_reason(decisions: &[Decision], outcomes: &[Outcome]) -> BTreeMap<String, Cohort> {
    // Indexed once, as `evaluate` does and for the same reason.
    let priced = LastPriced::of(outcomes);
    let mut groups: BTreeMap<String, Cohort> = BTreeMap::new();

    for decision in decisions {
        if matches!(decision.conclusion, Conclusion::Proposed) {
            continue;
        }
        let scored = scored_return(decision, &priced);
        for reason in &decision.reasons {
            let cohort = groups.entry(reason.clone()).or_insert_with(|| Cohort {
                decisions: 0,
                scored: 0,
                returns_bps: Vec::new(),
            });
            cohort.decisions += 1;
            if let Some(bps) = scored {
                cohort.scored += 1;
                cohort.returns_bps.push(bps);
            }
        }
    }

    for cohort in groups.values_mut() {
        cohort.returns_bps.sort_unstable();
    }
    groups
}

/// The last priced observation of every mint, by mint.
///
/// # Why this exists
///
/// [`scored_return`] used to scan the whole outcome table for every decision.
/// Against the live store — 4,374 decisions and ~900,000 outcomes — that is four
/// billion comparisons, and it was measured at **sixty seconds** for one call to
/// [`evaluate`]. `/v1/scoreboard` is a customer-facing route, so that is not a
/// slow report: it is a minute of CPU per request, on a shared box, reachable by
/// anyone the customer lane admits.
///
/// Built once, it is a single pass and then a lookup per decision.
///
/// # Why the *last* observation is enough
///
/// [`scored_return`] wants the greatest `measured_at` **among observations after
/// the decision**. Sorting a mint's priced observations by `measured_at` and
/// keeping only the greatest is sufficient, and the argument is short: the
/// greatest overall is the greatest of any subset it belongs to. If it is after
/// the decision it is the answer; if it is not, then nothing is, because nothing
/// is later than the maximum.
///
/// So the index holds one entry per mint and the look-ahead rule is still
/// applied per decision, at the point of use. It is **not** applied here, and
/// that separation matters: an index that pre-filtered by one decision's
/// watermark would be wrong for every other decision on the same mint.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct LastPriced(BTreeMap<Address, (Slot, u64)>);

impl LastPriced {
    /// Indexes the outcome table.
    #[must_use]
    pub fn of(outcomes: &[Outcome]) -> Self {
        let mut index: BTreeMap<Address, (Slot, u64)> = BTreeMap::new();
        for outcome in outcomes {
            // A measurement with no price says nothing about what a position
            // was worth. Rule 9: absent is not zero, and an unpriced
            // observation must not displace a priced earlier one.
            let Some(price) = outcome.last_price else {
                continue;
            };
            index
                .entry(outcome.mint)
                .and_modify(|held| {
                    // Strictly greater, so the **first** observation at a given
                    // slot wins and a later one at the same slot does not
                    // displace it. Two priced rows for one mint at one
                    // `measured_at` should not exist -- the outcome pass writes
                    // one per mint per measurement -- but "should not" is not
                    // "cannot", and the kernel is replayable only if this is
                    // deterministic. `>=` would make the answer depend on the
                    // order the store happened to return files in.
                    if outcome.measured_at > held.0 {
                        *held = (outcome.measured_at, price);
                    }
                })
                .or_insert((outcome.measured_at, price));
        }
        Self(index)
    }

    /// The last price observed for `mint` **strictly after** `after`.
    ///
    /// `None` when the mint was never priced, or when everything priced about it
    /// predates the moment asked about. The second case is the look-ahead rule
    /// and it is the reason this takes a slot at all: an observation the decision
    /// had not seen describes a market it did not act in.
    #[must_use]
    pub fn after(&self, mint: &Address, after: Slot) -> Option<u64> {
        let (measured_at, price) = self.0.get(mint)?;
        (*measured_at > after).then_some(*price)
    }

    /// How many mints were priced at all.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether nothing was priced.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// The return a decision earned, if it can be scored at all.
///
/// Factored out of [`evaluate`] so the two cannot drift: a breakdown that scored
/// decisions differently from the headline would produce groups that do not
/// reconcile with the cohort they came from, and the discrepancy would look like
/// a finding.
fn scored_return(decision: &Decision, priced: &LastPriced) -> Option<i64> {
    // Only an observation taken *after* the decision says anything about what
    // followed it. An earlier one describes a market the decision had not seen.
    let later = priced.after(&decision.mint, decision.decided_at)?;
    decision.return_bps(later)
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
            launch_recipients: None,
            launch_transactions: None,
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
            window_peak_price: None,
            window_trough_price: None,
            vwap: None,
            fills: 3,
        }
    }

    /// A refused decision carrying the given reasons.
    fn refused_for(mint: u8, entry: Option<u64>, reasons: &[&str]) -> Decision {
        let mut d = decision(mint, Conclusion::Passed, entry);
        d.reasons = reasons.iter().map(|r| (*r).to_owned()).collect();
        d
    }

    #[test]
    fn a_refusal_counts_under_every_reason_it_carries() {
        // Deliberate overlap. The question is what tokens refused for X did, not
        // what tokens refused ONLY for X did -- the second is a much smaller and
        // stranger population, and answering it would make each group describe a
        // different, rarer kind of token.
        let decisions = vec![refused_for(
            1,
            Some(1_000),
            &["ExitCapacityTooSmall", "TokenReadingTooOld"],
        )];
        let outcomes = vec![outcome(1, 12_000, Some(2_000))];

        let groups = by_reason(&decisions, &outcomes);
        assert_eq!(groups.len(), 2, "one decision, two groups");
        for reason in ["ExitCapacityTooSmall", "TokenReadingTooOld"] {
            let cohort = groups.get(reason).unwrap_or_else(|| panic!("{reason}"));
            assert_eq!(cohort.decisions, 1);
            assert_eq!(cohort.scored, 1);
            assert_eq!(cohort.returns_bps, vec![10_000], "doubled, so +100%");
        }
    }

    #[test]
    fn the_counts_are_counts_and_not_something_that_merely_moves() {
        // `decisions` and `scored` are the denominators every rate in the
        // breakdown is computed against. An increment that ran backwards, or
        // stayed at zero, would leave the medians looking right and every
        // percentage wrong -- which is the harder failure to notice.
        let decisions = vec![
            refused_for(1, Some(1_000), &["ExitCapacityTooSmall"]),
            refused_for(2, Some(1_000), &["ExitCapacityTooSmall"]),
            refused_for(3, Some(1_000), &["ExitCapacityTooSmall"]),
        ];
        let outcomes = vec![
            outcome(1, 12_000, Some(2_000)),
            outcome(2, 12_000, Some(500)),
            outcome(3, 12_000, Some(1_000)),
        ];

        let groups = by_reason(&decisions, &outcomes);
        let cohort = &groups["ExitCapacityTooSmall"];
        assert_eq!(cohort.decisions, 3);
        assert_eq!(cohort.scored, 3);
        assert_eq!(cohort.returns_bps.len(), 3);
    }

    #[test]
    fn a_decision_that_cannot_be_scored_still_counts_as_a_decision() {
        // The two counters answer different questions and must move
        // independently. Collapsing them would make "how often was this
        // refusal used" and "how often could we tell whether it was right"
        // the same number, and they are not: an unscoreable refusal is
        // evidence about the filter's reach and about nothing else.
        let decisions = vec![
            // No entry price: nothing to compute a return from.
            refused_for(1, None, &["NoExitSimulated"]),
            refused_for(2, Some(1_000), &["NoExitSimulated"]),
        ];
        let outcomes = vec![outcome(2, 12_000, Some(1_500))];

        let cohort = &by_reason(&decisions, &outcomes)["NoExitSimulated"];
        assert_eq!(cohort.decisions, 2, "both were refused for it");
        assert_eq!(cohort.scored, 1, "only one could be priced");
        assert_eq!(cohort.returns_bps, vec![5_000]);
    }

    #[test]
    fn proposals_are_not_in_the_breakdown_at_all() {
        // It is a split of the CONTROL. A proposal appearing here would be
        // compared against itself, and the reasons a proposal carries are the
        // ones it survived rather than the ones it failed.
        let mut proposed = decision(1, Conclusion::Proposed, Some(1_000));
        proposed.reasons = vec!["ExitCapacityTooSmall".to_owned()];

        let groups = by_reason(&[proposed], &[outcome(1, 12_000, Some(9_000))]);
        assert!(groups.is_empty(), "{groups:?}");
    }

    #[test]
    fn the_breakdown_reconciles_with_the_headline_it_qualifies() {
        // Groups overlap, so their scored counts do not sum to the control's.
        // What must hold is that no group is larger than the control and every
        // scored decision appears somewhere -- a breakdown that scored
        // decisions differently from `evaluate` would produce a discrepancy
        // that reads as a finding.
        let decisions = vec![
            refused_for(1, Some(1_000), &["ExitCapacityTooSmall"]),
            refused_for(2, Some(1_000), &["ExitCapacityTooSmall", "NoPrice"]),
            refused_for(3, Some(1_000), &["NoPrice"]),
        ];
        let outcomes = vec![
            outcome(1, 12_000, Some(2_000)),
            outcome(2, 12_000, Some(500)),
            outcome(3, 12_000, Some(1_000)),
        ];

        let report = evaluate(&decisions, &outcomes, 850);
        let groups = by_reason(&decisions, &outcomes);

        assert_eq!(report.refused.scored, 3);
        for (reason, cohort) in &groups {
            assert!(
                cohort.scored <= report.refused.scored,
                "{reason} has more scored than the whole control"
            );
        }
        // Overlap is real: two groups totalling four across three decisions.
        let total: usize = groups.values().map(|c| c.scored).sum();
        assert_eq!(total, 4, "one decision counted twice, by design");
    }

    #[test]
    fn a_control_with_no_reasons_produces_no_groups() {
        // Not an error. A refusal recorded without a reason is a gap in the
        // record, and inventing a bucket for it would put an unnamed cohort
        // beside the named ones as though it meant something.
        let groups = by_reason(
            &[refused_for(1, Some(1_000), &[])],
            &[outcome(1, 12_000, Some(2_000))],
        );
        assert!(groups.is_empty());
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
    fn the_screening_split_separates_the_two_populations_and_ignores_the_verdict() {
        // `Some` means the gate ran, whatever it concluded. A mutant keying on
        // the verdict's *value* -- say `== Some("Likely")` -- would put an
        // ordinary screened token in the unscreened cohort, which is the whole
        // distinction this function exists to draw.
        let mut screened_clean = decision(1, Conclusion::Proposed, Some(1_000));
        screened_clean.coordination = Some("Unlikely".to_owned());
        let mut screened_likely = decision(2, Conclusion::Proposed, Some(1_000));
        screened_likely.coordination = Some("Likely".to_owned());
        let unread = decision(3, Conclusion::Proposed, Some(1_000));

        let (screened, unscreened) = by_screening(
            &[screened_clean, screened_likely, unread],
            &[
                outcome(1, 20_000, Some(1_100)),
                outcome(2, 20_000, Some(1_100)),
                outcome(3, 20_000, Some(900)),
            ],
        );
        assert_eq!(screened.decisions, 2, "both verdicts count as screened");
        assert_eq!(unscreened.decisions, 1);
        assert_eq!(screened.scored, 2);
        assert_eq!(unscreened.median(), Some(-1_000));
    }

    #[test]
    fn the_screening_split_covers_proposals_only() {
        // A refusal is not part of the cohort whose headline this qualifies, and
        // pooling them would put the refused population back into a number that
        // exists to describe the proposed one.
        let mut refused = decision(1, Conclusion::Passed, Some(1_000));
        refused.coordination = None;
        let (screened, unscreened) = by_screening(&[refused], &[outcome(1, 20_000, Some(1_100))]);
        assert_eq!(screened.decisions, 0);
        assert_eq!(unscreened.decisions, 0);
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

    fn priced_at(mint: u8, measured_at: u64, price: Option<u64>) -> Outcome {
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
            last_price: price,
            peak_price: None,
            trough_price: None,
            window_peak_price: None,
            window_trough_price: None,
            vwap: None,
            fills: 0,
        }
    }

    #[test]
    fn the_index_keeps_the_latest_priced_observation_not_the_last_one_seen() {
        // The store returns rows in file order, not time order. An index that
        // keeps whichever came last in iteration order scores a decision against
        // an older price, and nothing about the result looks wrong.
        let table = vec![
            priced_at(1, 30_000, Some(300)),
            priced_at(1, 10_000, Some(100)),
            priced_at(1, 20_000, Some(200)),
        ];
        let index = LastPriced::of(&table);
        assert_eq!(index.after(&Address::new([1u8; 32]), Slot(0)), Some(300));
    }

    #[test]
    fn an_unpriced_observation_never_displaces_a_priced_one() {
        // Rule 9. A later measurement with no price says nothing about what a
        // position was worth, and letting it win would turn a scoreable decision
        // into an unscoreable one -- silently, and only for the tokens that
        // stopped trading, which is a selected sample.
        let table = vec![priced_at(1, 10_000, Some(100)), priced_at(1, 30_000, None)];
        let index = LastPriced::of(&table);
        assert_eq!(index.after(&Address::new([1u8; 32]), Slot(0)), Some(100));
    }

    #[test]
    fn an_observation_before_the_moment_asked_about_is_refused() {
        // The look-ahead rule, which is the whole reason `after` takes a slot.
        // An observation the decision had not seen describes a market it did not
        // act in.
        let index = LastPriced::of(&[priced_at(1, 10_000, Some(100))]);
        let mint = Address::new([1u8; 32]);

        assert_eq!(index.after(&mint, Slot(9_999)), Some(100));
        // Strictly after: the same slot is not after it.
        assert_eq!(index.after(&mint, Slot(10_000)), None);
        assert_eq!(index.after(&mint, Slot(10_001)), None);
    }

    #[test]
    fn a_tie_on_measured_at_resolves_the_same_way_every_time() {
        // Replay depends on it. Two priced rows for one mint at one slot should
        // not exist, but "should not" is not "cannot" -- and with `>=` the answer
        // would depend on the order the store returned files in, which is not a
        // property anything guarantees.
        let first_wins = LastPriced::of(&[
            priced_at(1, 10_000, Some(100)),
            priced_at(1, 10_000, Some(999)),
        ]);
        assert_eq!(
            first_wins.after(&Address::new([1u8; 32]), Slot(0)),
            Some(100),
            "the first observation at a slot is kept"
        );

        // And the reverse order gives the reverse answer, which is what makes
        // the assertion above about the *rule* rather than about these numbers.
        let reversed = LastPriced::of(&[
            priced_at(1, 10_000, Some(999)),
            priced_at(1, 10_000, Some(100)),
        ]);
        assert_eq!(reversed.after(&Address::new([1u8; 32]), Slot(0)), Some(999));
    }

    #[test]
    fn an_index_with_something_in_it_does_not_claim_to_be_empty() {
        // `a_mint_nobody_priced_has_no_answer_rather_than_a_zero` asserts
        // `is_empty()` is true, and that assertion passes just as well if the
        // method always returns true. This is the other side, without which the
        // first one says nothing.
        let index = LastPriced::of(&[priced_at(1, 10_000, Some(100))]);
        assert!(!index.is_empty());
        assert_eq!(index.len(), 1);
    }

    #[test]
    fn a_mint_nobody_priced_has_no_answer_rather_than_a_zero() {
        let index = LastPriced::of(&[priced_at(1, 10_000, None)]);
        assert_eq!(index.after(&Address::new([1u8; 32]), Slot(0)), None);
        assert_eq!(index.after(&Address::new([2u8; 32]), Slot(0)), None);
        assert!(index.is_empty(), "an unpriced mint is not in the index");
    }

    #[test]
    fn mints_do_not_borrow_each_others_prices() {
        // The index is keyed by mint, and the obvious way to break it is to key
        // it by anything else.
        let table = vec![
            priced_at(1, 10_000, Some(100)),
            priced_at(2, 30_000, Some(999)),
        ];
        let index = LastPriced::of(&table);
        assert_eq!(index.after(&Address::new([1u8; 32]), Slot(0)), Some(100));
        assert_eq!(index.after(&Address::new([2u8; 32]), Slot(0)), Some(999));
        assert_eq!(index.len(), 2);
    }

    #[test]
    fn the_index_agrees_with_the_scan_it_replaced() {
        // The property that matters most: this was a rewrite of a correct
        // function for speed, so it has to produce the same answers. The scan is
        // reproduced here rather than referenced, so that deleting the original
        // cannot make this test vacuous.
        let table: Vec<Outcome> = (0..40u64)
            .map(|i| {
                let mint = u8::try_from(i % 5).expect("small");
                let price = (i % 7 != 0).then_some(100 + i * 3);
                priced_at(mint, 10_000 + i * 137, price)
            })
            .collect();
        let index = LastPriced::of(&table);

        for mint_byte in 0..6u8 {
            let mint = Address::new([mint_byte; 32]);
            for at in [0u64, 10_000, 12_000, 14_000, 16_000, 99_999] {
                let scanned = table
                    .iter()
                    .filter(|o| o.mint == mint && o.measured_at > Slot(at))
                    .filter_map(|o| o.last_price.map(|p| (o.measured_at, p)))
                    .max_by_key(|(m, _)| *m)
                    .map(|(_, p)| p);
                assert_eq!(
                    index.after(&mint, Slot(at)),
                    scanned,
                    "mint {mint_byte} at {at}"
                );
            }
        }
    }
}
