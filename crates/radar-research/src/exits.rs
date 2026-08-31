// SPDX-License-Identifier: Apache-2.0
//! Would an exit rule have helped, and can this data say?
//!
//! # The claim being tested
//!
//! [`0009`] concluded that **"the exit rule is not where the edge is"**. Its
//! evidence was a table of take-profits:
//!
//! ```text
//! take-profit    hit rate    mean bps
//!      500bps         63%        -643
//!     1000bps         57%        -500
//!     2000bps         52%         -91
//! ```
//!
//! and its diagnosis of why they fail:
//!
//! > the 37–48% that miss the target do not miss it by a little. Their `HELD` is
//! > the p10 of −94.9%, and no achievable take-profit on the winners pays for
//! > that.
//!
//! **That diagnosis is the case for a stop, and 0009 never tested one.** A
//! take-profit truncates the right tail. Only a stop truncates the left one, and
//! the left one is the stated problem. The conclusion may still be correct; it
//! does not follow from the evidence offered for it.
//!
//! # What this data can and cannot support
//!
//! An [`Outcome`] is a checkpoint, and its `peak_price` and `trough_price` are
//! folded with `max` and `min` — so at each checkpoint the *running* extremes
//! since launch are known, and the interval in which each first moved can be
//! recovered by walking the series.
//!
//! What cannot be recovered is the **order within an interval**. If between two
//! checkpoints the running peak rose past a take-profit and the running trough
//! fell past a stop, the data does not say which happened first, and the two give
//! opposite answers.
//!
//! So every result here is a **pair of bounds**, not a number:
//!
//! - **Pessimistic** — assume the stop was hit first whenever both moved in one
//!   interval.
//! - **Optimistic** — assume the target was hit first.
//!
//! A rule is only worth anything if its *pessimistic* bound is worth having. A
//! backtest that reports the optimistic figure alone is the same error as
//! measuring MFE and calling it a return, which is [`0011`]'s correction to
//! [`0009`] in a different field.
//!
//! # What is deliberately not modelled
//!
//! Costs. Every figure is gross, and the measured round trip is 850 bps a pair
//! of legs — more at small notionals ([`0019`]). A rule that trades more often
//! pays that more often, and the comparison here does not charge it.
//!
//! [`0009`]: ../../docs/research/0009-what-a-token-actually-does-to-your-money.md
//! [`0011`]: ../../docs/research/0011-graduation-predicts-volatility-not-profit.md
//! [`0019`]: ../../docs/research/0019-the-round-trip-is-not-one-number.md

use std::collections::BTreeMap;

use radar_store::Outcome;
use radar_types::Address;
use serde::Serialize;

/// The smallest cohort worth reporting a percentile for.
pub const MIN_COHORT: usize = 30;

/// Where a position ended, under one rule.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
pub enum Ending {
    /// The take-profit was reached.
    Target,
    /// The stop was reached.
    Stopped,
    /// Neither; the position was carried to the last observation.
    Held,
}

/// One rule's outcome over a cohort, under one tie-breaking assumption.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Default)]
pub struct Outcomes {
    /// Returns in basis points, ascending.
    pub returns_bps: Vec<i64>,
    /// How many ended at the target.
    pub target: usize,
    /// How many ended at the stop.
    pub stopped: usize,
    /// How many were held to the last observation.
    pub held: usize,
}

impl Outcomes {
    /// Positions the rule was applied to.
    #[must_use]
    pub fn n(&self) -> usize {
        self.returns_bps.len()
    }

    /// The median return, or `None` when nothing was measured.
    #[must_use]
    pub fn median(&self) -> Option<i64> {
        crate::percentile(&self.returns_bps, 0.50)
    }

    /// The share clearing `cost_bps`, per ten thousand, or `None` when empty.
    ///
    /// `None` rather than zero: a cohort nobody could measure has no rate, and
    /// reporting one as zero reads as "none of them made money" (rule 9).
    #[must_use]
    pub fn beat_cost_bps(&self, cost_bps: i64) -> Option<u64> {
        if self.returns_bps.is_empty() {
            return None;
        }
        let hits =
            u64::try_from(self.returns_bps.iter().filter(|r| **r > cost_bps).count()).unwrap_or(0);
        let total = u64::try_from(self.returns_bps.len()).unwrap_or(1);
        Some(hits.saturating_mul(10_000) / total)
    }
}

/// One rule, measured under both tie-breaking assumptions.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct Rule {
    /// Take-profit in basis points, or `None` for no target.
    pub target_bps: Option<u64>,
    /// Stop in basis points below entry, or `None` for no stop.
    pub stop_bps: Option<u64>,
    /// Assuming the stop was hit first wherever both moved in one interval.
    pub pessimistic: Outcomes,
    /// Assuming the target was hit first.
    pub optimistic: Outcomes,
}

impl Rule {
    /// A short label, for a report.
    #[must_use]
    pub fn label(&self) -> String {
        let t = self
            .target_bps
            .map_or_else(|| "—".to_owned(), |b| format!("+{b}"));
        let s = self
            .stop_bps
            .map_or_else(|| "—".to_owned(), |b| format!("-{b}"));
        format!("{t}/{s}")
    }

    /// Whether both bounds cleared the cohort floor.
    #[must_use]
    pub fn is_reportable(&self) -> bool {
        self.pessimistic.n() >= MIN_COHORT
    }
}

/// The rules compared.
///
/// Held as data so the grid is visible and a test can sweep it. The first entry
/// is the baseline every other is judged against: **no rule at all**, held to the
/// last observation, which is what every figure in this repository already
/// measures. A rule that does not beat holding is a rule that costs money to
/// break even.
///
/// The stops bracket the population's median maximum adverse excursion, which
/// [`0011`] puts at 1,509 bps for tokens that never graduate — so a stop at 1,000
/// is inside the typical drawdown and one at 5,000 is well outside it. A grid
/// that sat entirely on one side of the MAE would answer nothing.
///
/// [`0011`]: ../../docs/research/0011-graduation-predicts-volatility-not-profit.md
pub const GRID: [(Option<u64>, Option<u64>); 10] = [
    (None, None),
    (None, Some(1_000)),
    (None, Some(2_500)),
    (None, Some(5_000)),
    (Some(1_000), None),
    (Some(2_500), None),
    (Some(5_000), None),
    (Some(1_000), Some(1_000)),
    (Some(2_500), Some(2_500)),
    (Some(5_000), Some(2_500)),
];

/// The comparison.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct Report {
    /// Mints that produced a usable price path.
    pub paths: usize,
    /// One entry per rule in [`GRID`] order.
    pub rules: Vec<Rule>,
}

impl Report {
    /// The baseline — no target, no stop — which is entry [`GRID`] holds first.
    #[must_use]
    pub fn baseline(&self) -> Option<&Rule> {
        self.rules
            .iter()
            .find(|r| r.target_bps.is_none() && r.stop_bps.is_none())
    }

    /// Rules whose pessimistic median beats the baseline's, best first.
    ///
    /// Judged on the **pessimistic** bound on purpose. A rule that only wins
    /// under the favourable tie-break has not been shown to win.
    #[must_use]
    pub fn beats_baseline(&self) -> Vec<&Rule> {
        let Some(base) = self.baseline().and_then(|b| b.pessimistic.median()) else {
            return Vec::new();
        };
        let mut out: Vec<&Rule> = self
            .rules
            .iter()
            .filter(|r| r.target_bps.is_some() || r.stop_bps.is_some())
            .filter(|r| r.is_reportable())
            .filter(|r| r.pessimistic.median().is_some_and(|m| m > base))
            .collect();
        out.sort_by_key(|r| std::cmp::Reverse(r.pessimistic.median().unwrap_or(i64::MIN)));
        out
    }
}

/// One step of a price path: the running extremes at a checkpoint.
#[derive(Clone, Copy, Debug)]
struct Step {
    peak: u64,
    trough: u64,
    last: u64,
}

/// Applies every rule in [`GRID`] to every mint's path.
#[must_use]
pub fn evaluate(outcomes: &[Outcome]) -> Report {
    let paths = paths_of(outcomes);
    let mut rules: Vec<Rule> = GRID
        .iter()
        .map(|(target_bps, stop_bps)| Rule {
            target_bps: *target_bps,
            stop_bps: *stop_bps,
            pessimistic: Outcomes::default(),
            optimistic: Outcomes::default(),
        })
        .collect();

    for path in paths.values() {
        for rule in &mut rules {
            if let Some((bps, ending)) = simulate(path, rule.target_bps, rule.stop_bps, true) {
                rule.pessimistic.returns_bps.push(bps);
                count(&mut rule.pessimistic, ending);
            }
            if let Some((bps, ending)) = simulate(path, rule.target_bps, rule.stop_bps, false) {
                rule.optimistic.returns_bps.push(bps);
                count(&mut rule.optimistic, ending);
            }
        }
    }

    for rule in &mut rules {
        rule.pessimistic.returns_bps.sort_unstable();
        rule.optimistic.returns_bps.sort_unstable();
    }

    Report {
        paths: paths.len(),
        rules,
    }
}

/// Tallies an ending.
fn count(into: &mut Outcomes, ending: Ending) {
    match ending {
        Ending::Target => into.target += 1,
        Ending::Stopped => into.stopped += 1,
        Ending::Held => into.held += 1,
    }
}

/// Walks one path under one rule.
///
/// `stop_first` is the tie-break: when both the target and the stop are crossed
/// within the same interval, the order is unrecoverable and the caller decides
/// which to assume. Running it both ways is the point — see the module
/// documentation.
///
/// The entry is the first step's `last` price. Returns `None` when the path
/// cannot be priced.
fn simulate(
    path: &[Step],
    target_bps: Option<u64>,
    stop_bps: Option<u64>,
    stop_first: bool,
) -> Option<(i64, Ending)> {
    let entry = path.first()?.last;
    if entry == 0 {
        return None;
    }
    let entry128 = i128::from(entry);

    // Thresholds as prices, so the comparison is integer throughout. A threshold
    // compared as a float compares differently on a replay.
    let target = target_bps.map(|b| entry128 * (10_000 + i128::from(b)) / 10_000);
    let stop = stop_bps.map(|b| entry128 * (10_000 - i128::from(b)).max(0) / 10_000);

    for step in path.iter().skip(1) {
        let hit_target = target.is_some_and(|t| i128::from(step.peak) >= t);
        let hit_stop = stop.is_some_and(|s| i128::from(step.trough) <= s);

        // Both crossed inside this interval and the order is unknown.
        let (first, price) = match (hit_target, hit_stop) {
            // Both crossed inside this interval, and the caller's assumption
            // decides. This is the only arm where the data is silent.
            (true, true) if stop_first => (Ending::Stopped, stop?),
            // Either only the target crossed, or both did and the caller assumes
            // the target came first -- the same ending by two routes.
            (true, _) => (Ending::Target, target?),
            (false, true) => (Ending::Stopped, stop?),
            (false, false) => continue,
        };
        return Some((bps_between(entry128, price), first));
    }

    let last = i128::from(path.last()?.last);
    Some((bps_between(entry128, last), Ending::Held))
}

/// Return from `entry` to `exit`, in basis points.
fn bps_between(entry: i128, exit: i128) -> i64 {
    i64::try_from((exit - entry).saturating_mul(10_000) / entry).unwrap_or(i64::MAX)
}

/// Groups outcomes into per-mint price paths, ascending by measurement slot.
///
/// A path needs at least two steps: an entry and something after it. Steps
/// missing any of the three prices are dropped — absent is not zero, and a
/// missing trough read as zero would trigger every stop ever written.
fn paths_of(outcomes: &[Outcome]) -> BTreeMap<Address, Vec<Step>> {
    let mut by_mint: BTreeMap<Address, Vec<(u64, Step)>> = BTreeMap::new();
    for o in outcomes {
        let (Some(peak), Some(trough), Some(last)) = (o.peak_price, o.trough_price, o.last_price)
        else {
            continue;
        };
        if peak == 0 || trough == 0 || last == 0 {
            continue;
        }
        by_mint
            .entry(o.mint)
            .or_default()
            .push((o.measured_at.get(), Step { peak, trough, last }));
    }

    by_mint
        .into_iter()
        .filter_map(|(mint, mut steps)| {
            steps.sort_unstable_by_key(|(at, _)| *at);
            let path: Vec<Step> = steps.into_iter().map(|(_, s)| s).collect();
            (path.len() >= 2).then_some((mint, path))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use radar_types::Slot;

    fn step(mint: u8, at: u64, peak: u64, trough: u64, last: u64) -> Outcome {
        Outcome {
            mint: Address::new([mint; 32]),
            measured_at: Slot(at),
            launch_slot: Slot(0),
            first_transfer_slot: None,
            last_transfer_slot: None,
            transfers: 0,
            unique_senders: 0,
            unique_receivers: 0,
            graduated_at: None,
            first_price: Some(1_000),
            last_price: Some(last),
            peak_price: Some(peak),
            trough_price: Some(trough),
            vwap: None,
            fills: 0,
        }
    }

    fn rule_of(r: &Report, target: Option<u64>, stop: Option<u64>) -> &Rule {
        r.rules
            .iter()
            .find(|x| x.target_bps == target && x.stop_bps == stop)
            .expect("rule in the grid")
    }

    #[test]
    fn a_stop_truncates_the_left_tail_where_a_take_profit_cannot() {
        // The property 0009's conclusion turns on, and the test it never had.
        // A token that goes straight down: a take-profit does nothing, and a stop
        // is the only thing that changes the outcome.
        let path = [
            step(1, 100, 1_000, 1_000, 1_000),
            step(1, 200, 1_000, 500, 500),
            step(1, 300, 1_000, 100, 100),
        ];
        let r = evaluate(&path);

        // Held to the end: down 90%.
        assert_eq!(rule_of(&r, None, None).pessimistic.median(), Some(-9_000));
        // A take-profit changes nothing -- it was never reached.
        assert_eq!(
            rule_of(&r, Some(2_500), None).pessimistic.median(),
            Some(-9_000)
        );
        // A stop at 25% caps the loss there.
        assert_eq!(
            rule_of(&r, None, Some(2_500)).pessimistic.median(),
            Some(-2_500)
        );
        assert_eq!(rule_of(&r, None, Some(2_500)).pessimistic.stopped, 1);
    }

    #[test]
    fn a_take_profit_truncates_the_right_tail() {
        let path = [
            step(1, 100, 1_000, 1_000, 1_000),
            step(1, 200, 5_000, 1_000, 5_000),
        ];
        let r = evaluate(&path);
        assert_eq!(rule_of(&r, None, None).pessimistic.median(), Some(40_000));
        assert_eq!(
            rule_of(&r, Some(2_500), None).pessimistic.median(),
            Some(2_500),
            "capped at the target"
        );
        assert_eq!(rule_of(&r, Some(2_500), None).pessimistic.target, 1);
    }

    #[test]
    fn an_interval_crossing_both_reports_two_bounds_that_differ() {
        // The whole reason every figure here is a pair. Between the first and
        // second checkpoint the running peak rose past +25% and the running
        // trough fell past -25%. The data cannot say which came first, and the
        // two assumptions give opposite answers.
        let path = [
            step(1, 100, 1_000, 1_000, 1_000),
            step(1, 200, 2_000, 500, 700),
        ];
        let r = evaluate(&path);
        let both = rule_of(&r, Some(2_500), Some(2_500));

        assert_eq!(
            both.pessimistic.median(),
            Some(-2_500),
            "stop assumed first"
        );
        assert_eq!(
            both.optimistic.median(),
            Some(2_500),
            "target assumed first"
        );
        assert_ne!(
            both.pessimistic.median(),
            both.optimistic.median(),
            "a rule whose bounds agree here would mean the tie-break is not applied"
        );
        assert_eq!(both.pessimistic.stopped, 1);
        assert_eq!(both.optimistic.target, 1);
    }

    #[test]
    fn a_rule_is_judged_on_its_pessimistic_bound() {
        // A rule that only wins under the favourable tie-break has not been
        // shown to win, which is 0011's correction to 0009 in a different field:
        // an aggregate over extremes finds the tail it is looking for.
        let mut outcomes = Vec::new();
        for i in 0u8..40 {
            outcomes.push(step(i, 100, 1_000, 1_000, 1_000));
            // Peak and trough both cross in the same interval, ending flat.
            outcomes.push(step(i, 200, 2_000, 500, 1_000));
        }
        let r = evaluate(&outcomes);
        let both = rule_of(&r, Some(2_500), Some(2_500));
        assert_eq!(both.pessimistic.median(), Some(-2_500));
        assert_eq!(both.optimistic.median(), Some(2_500));

        // The baseline holds flat at 0, so the optimistic bound beats it and the
        // pessimistic does not. It must not appear.
        assert!(
            !r.beats_baseline().iter().any(|x| std::ptr::eq(*x, both)),
            "a rule winning only on the optimistic bound must not be reported as beating the baseline"
        );
    }

    #[test]
    fn a_path_of_one_observation_is_not_a_path() {
        // An entry with nothing after it cannot be exited, and scoring it as
        // held-flat would put a zero into every rule's distribution.
        let r = evaluate(&[step(1, 100, 1_000, 1_000, 1_000)]);
        assert_eq!(r.paths, 0);
        assert_eq!(rule_of(&r, None, None).pessimistic.n(), 0);
    }

    #[test]
    fn an_absent_or_zero_price_never_becomes_a_step() {
        // Rule 9. A missing trough read as zero would trigger every stop ever
        // written, and a zero entry would divide.
        let mut missing = step(1, 200, 1_000, 1_000, 1_000);
        missing.trough_price = None;
        let r = evaluate(&[step(1, 100, 1_000, 1_000, 1_000), missing]);
        assert_eq!(r.paths, 0);

        let zero = step(1, 200, 1_000, 0, 1_000);
        let r = evaluate(&[step(1, 100, 1_000, 1_000, 1_000), zero]);
        assert_eq!(r.paths, 0);
    }

    #[test]
    fn the_baseline_is_in_the_grid_and_carries_no_rule() {
        // Every other row is judged against it, so its absence would make the
        // comparison meaningless rather than merely incomplete.
        assert_eq!(GRID[0], (None, None));
        let r = evaluate(&[
            step(1, 100, 1_000, 1_000, 1_000),
            step(1, 200, 1_000, 1_000, 1_000),
        ]);
        let base = r.baseline().expect("a baseline");
        assert!(base.target_bps.is_none() && base.stop_bps.is_none());
    }

    #[test]
    fn the_grid_brackets_the_populations_typical_drawdown() {
        // 0011 puts the median MAE for never-graduated tokens at 1,509 bps. A
        // grid entirely inside or entirely outside that answers nothing, so the
        // stops must straddle it.
        let stops: Vec<u64> = GRID.iter().filter_map(|(_, s)| *s).collect();
        assert!(
            stops.iter().any(|s| *s < 1_509),
            "a stop inside the typical drawdown"
        );
        assert!(stops.iter().any(|s| *s > 1_509), "and one outside it");
    }

    #[test]
    fn beating_the_baseline_requires_clearing_the_cohort_floor() {
        // One lucky path must not appear as a rule that works.
        let r = evaluate(&[
            step(1, 100, 1_000, 1_000, 1_000),
            step(1, 200, 5_000, 1_000, 200),
        ]);
        assert!(
            r.beats_baseline().is_empty(),
            "a single path is not a cohort"
        );
    }
}
