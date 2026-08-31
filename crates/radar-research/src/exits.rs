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

    /// The return at a percentile, or `None` when nothing was measured.
    ///
    /// **The median alone is not enough here, and reporting it alone was a
    /// mistake this module made once.** The population's baseline median is
    /// exactly zero — most tokens on this venue trade a few times and stop — so
    /// *any* take-profit firing more than half the time shows a median at its
    /// target and appears to beat holding. That is an artefact of a point mass,
    /// not a finding, and it hides the losing tail that [`0009`] says is the
    /// whole problem: "the 37–48% that miss the target do not miss it by a
    /// little".
    ///
    /// [`0009`]: ../../docs/research/0009-what-a-token-actually-does-to-your-money.md
    #[must_use]
    pub fn percentile(&self, p: f64) -> Option<i64> {
        crate::percentile(&self.returns_bps, p)
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

    /// Whether this is the baseline — no target and no stop.
    ///
    /// A named predicate rather than a condition inside `find`, because there the
    /// `&&` is unreachable from any test: [`GRID`]'s first entry is the baseline
    /// and satisfies `||` equally, so `find` returns it either way and a mutant
    /// widening the condition survives. Here both halves are testable.
    #[must_use]
    pub const fn is_baseline(&self) -> bool {
        self.target_bps.is_none() && self.stop_bps.is_none()
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
        self.rules.iter().find(|r| r.is_baseline())
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
            .filter(|r| !r.is_baseline())
            .filter(|r| r.is_reportable())
            // The median is not enough. A rule must also not make the losing
            // tail worse -- otherwise a take-profit that fires on the majority
            // wins on the median while the minority it abandons goes to -95%,
            // which is exactly the trade 0009 rejected.
            .filter(|r| r.pessimistic.median().is_some_and(|m| m > base))
            .filter(|r| {
                match (
                    r.pessimistic.percentile(0.25),
                    self.baseline().and_then(|b| b.pessimistic.percentile(0.25)),
                ) {
                    (Some(rule_p25), Some(base_p25)) => rule_p25 >= base_p25,
                    _ => false,
                }
            })
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

    // The extremes at the entry checkpoint. `peak_price` and `trough_price` are
    // folded from **launch**, not from here, so a token that peaked before the
    // entry carries that peak forward — and crediting it would be a target the
    // position could never have taken. A threshold counts as crossed only when a
    // NEW extreme is set after entry.
    let entry_peak = path.first()?.peak;
    let entry_trough = path.first()?.trough;

    for step in path.iter().skip(1) {
        let new_high = step.peak > entry_peak;
        let new_low = step.trough < entry_trough;
        let hit_target = new_high && target.is_some_and(|t| i128::from(step.peak) >= t);
        let hit_stop = new_low && stop.is_some_and(|s| i128::from(step.trough) <= s);

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
            window_peak_price: None,
            window_trough_price: None,
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
    fn a_peak_set_before_entry_is_not_a_target_the_position_could_take() {
        // `peak_price` and `trough_price` are folded from LAUNCH, not from the
        // entry checkpoint. A token that already peaked carries that peak
        // forward, and crediting it would hand the position a gain that was over
        // before it started -- look-ahead wearing the shape of a fill.
        //
        // Here the peak is 5x the entry price and was set BEFORE entry; nothing
        // rises afterwards. No target may fire.
        let path = [
            step(1, 100, 5_000, 1_000, 1_000),
            step(1, 200, 5_000, 900, 900),
        ];
        let r = evaluate(&path);
        assert_eq!(
            rule_of(&r, Some(2_500), None).pessimistic.target,
            0,
            "a peak set before entry is not available to the position"
        );
        assert_eq!(
            rule_of(&r, Some(2_500), None).pessimistic.median(),
            Some(-1_000),
            "so it is held to the last price"
        );

        // A NEW high after entry is available, and does fire.
        let rising = [
            step(1, 100, 5_000, 1_000, 1_000),
            step(1, 200, 6_000, 1_000, 6_000),
        ];
        assert_eq!(
            rule_of(&evaluate(&rising), Some(2_500), None)
                .pessimistic
                .target,
            1
        );
    }

    #[test]
    fn a_trough_set_before_entry_does_not_stop_the_position_out() {
        // The same asymmetry on the other side. Without it every token that had
        // already fallen would be stopped out at its first checkpoint, for a
        // loss it never took.
        let path = [
            step(1, 100, 1_000, 100, 1_000),
            step(1, 200, 1_000, 100, 1_100),
        ];
        let r = evaluate(&path);
        assert_eq!(rule_of(&r, None, Some(2_500)).pessimistic.stopped, 0);
        assert_eq!(
            rule_of(&r, None, Some(2_500)).pessimistic.median(),
            Some(1_000)
        );
    }

    #[test]
    fn a_take_profit_can_win_on_the_median_without_helping_the_losing_tail() {
        // The mistake this module made on its first live run, pinned so the
        // report cannot make it again.
        //
        // The population baseline median is exactly zero -- most tokens on this
        // venue trade a few times and stop -- so a take-profit that fires on the
        // majority shows a median at its target and looks like it beats holding.
        // The minority it walks away from is untouched, and that minority is what
        // 0009 says decides the question.
        //
        // Forty tokens rise past +25% and come back to flat; twenty collapse.
        let mut outcomes = Vec::new();
        for i in 0u8..40 {
            outcomes.push(step(i, 100, 1_000, 1_000, 1_000));
            outcomes.push(step(i, 200, 1_300, 1_000, 1_000));
        }
        for i in 100u8..120 {
            outcomes.push(step(i, 100, 1_000, 1_000, 1_000));
            outcomes.push(step(i, 200, 1_000, 50, 50));
        }
        let r = evaluate(&outcomes);
        let tp = rule_of(&r, Some(2_500), None);
        let base = r.baseline().expect("a baseline");

        assert_eq!(base.pessimistic.median(), Some(0), "the point mass");
        assert_eq!(
            tp.pessimistic.median(),
            Some(2_500),
            "and the rule beats it"
        );

        // And does precisely nothing for the losing quarter.
        assert_eq!(tp.pessimistic.percentile(0.25), Some(-9_500));
        assert_eq!(
            tp.pessimistic.percentile(0.25),
            base.pessimistic.percentile(0.25),
            "the tail is identical, which is what the median hides"
        );
    }

    #[test]
    fn a_rule_that_deepens_the_losing_tail_is_not_reported_as_a_winner() {
        // A stop placed inside the typical drawdown takes a loss on tokens that
        // would have recovered. If that makes p25 worse, the rule must not appear
        // as beating the baseline however good its median looks.
        let mut outcomes = Vec::new();
        for i in 0u8..40 {
            // Dips below a 10% stop, then recovers to flat.
            outcomes.push(step(i, 100, 1_000, 1_000, 1_000));
            outcomes.push(step(i, 200, 1_000, 800, 1_000));
        }
        let r = evaluate(&outcomes);
        let stopped = rule_of(&r, None, Some(1_000));
        assert_eq!(stopped.pessimistic.stopped, 40, "every one is stopped out");
        assert_eq!(stopped.pessimistic.median(), Some(-1_000));
        assert!(
            !r.beats_baseline().iter().any(|x| std::ptr::eq(*x, stopped)),
            "a rule that only ever loses must not be reported as beating holding"
        );
    }

    #[test]
    fn an_outcomes_reports_how_many_positions_it_holds() {
        // The count is what says whether a median is worth reading. A constant
        // here silently changes how much weight the whole table carries.
        let mut o = Outcomes::default();
        assert_eq!(o.n(), 0);
        o.returns_bps = vec![-100, 0, 250];
        assert_eq!(o.n(), 3);
        assert_eq!(o.median(), Some(0));
        assert_eq!(o.percentile(0.0), Some(-100));
    }

    #[test]
    fn clearing_costs_is_strictly_above_them_and_scaled_by_ten_thousand() {
        // Strictly above, not at: a position returning exactly the round trip
        // broke even and paid for the privilege. And the share is a proportion,
        // so the division cannot become a remainder or a product.
        let o = Outcomes {
            returns_bps: vec![-1_000, 850, 851, 5_000],
            ..Outcomes::default()
        };
        assert_eq!(o.beat_cost_bps(850), Some(5_000), "two of four clear 850");
        assert_eq!(o.beat_cost_bps(0), Some(7_500), "three of four clear zero");
        assert_eq!(o.beat_cost_bps(10_000), Some(0), "none clear ten thousand");
        // Absent, never zero: nobody measured is not nobody made money (rule 9).
        assert_eq!(Outcomes::default().beat_cost_bps(850), None);
    }

    #[test]
    fn a_rule_labels_itself_with_both_of_its_thresholds() {
        // The label is how a reader tells one row from another. An empty or
        // constant one makes every row of the report indistinguishable.
        let rule = |t, s| Rule {
            target_bps: t,
            stop_bps: s,
            pessimistic: Outcomes::default(),
            optimistic: Outcomes::default(),
        };
        assert_eq!(rule(Some(2_500), Some(1_000)).label(), "+2500/-1000");
        assert_eq!(rule(Some(2_500), None).label(), "+2500/—");
        assert_eq!(rule(None, Some(1_000)).label(), "—/-1000");
        assert_eq!(rule(None, None).label(), "—/—", "the baseline");
    }

    #[test]
    fn reportable_requires_the_cohort_floor_and_is_true_above_it() {
        // Both directions, so it cannot be replaced by either constant.
        let with = |n: usize| Rule {
            target_bps: None,
            stop_bps: None,
            pessimistic: Outcomes {
                returns_bps: vec![0; n],
                ..Outcomes::default()
            },
            optimistic: Outcomes::default(),
        };
        assert!(!with(MIN_COHORT - 1).is_reportable());
        assert!(with(MIN_COHORT).is_reportable(), "the floor is inclusive");
        assert!(with(MIN_COHORT + 1).is_reportable());
    }

    #[test]
    fn the_baseline_is_the_rule_with_neither_threshold_not_either() {
        // `&&` widened to `||` would return the first rule carrying *no target*,
        // which is a stop-only rule -- and every comparison in the report would
        // then be against a rule rather than against holding.
        let r = evaluate(&[
            step(1, 100, 1_000, 1_000, 1_000),
            step(1, 200, 1_000, 1_000, 900),
        ]);
        let base = r.baseline().expect("a baseline");
        assert!(base.target_bps.is_none() && base.stop_bps.is_none());
        assert_eq!(base.label(), "—/—");
    }

    #[test]
    fn beating_the_baseline_needs_a_strictly_better_median_and_a_real_rule() {
        // Three properties at once, because they interact.
        //
        // A rule must beat the baseline STRICTLY -- equal is not better, and `>`
        // widened to `>=` would report every rule that changes nothing.
        //
        // The baseline must never appear in its own winners list, which is what
        // the `target.is_some() || stop.is_some()` filter is for: narrowed to
        // `&&`, single-sided rules would silently stop being eligible.
        let mut outcomes = Vec::new();
        for i in 0u8..40 {
            // Rises past +10% after entry and stays there: a target-only rule
            // captures it, and holding captures more.
            outcomes.push(step(i, 100, 1_000, 1_000, 1_000));
            outcomes.push(step(i, 200, 1_150, 1_000, 1_150));
        }
        let r = evaluate(&outcomes);
        let base_median = r.baseline().and_then(|b| b.pessimistic.median());
        assert_eq!(base_median, Some(1_500), "holding takes the whole move");

        let winners = r.beats_baseline();
        assert!(
            !winners
                .iter()
                .any(|w| w.target_bps.is_none() && w.stop_bps.is_none()),
            "the baseline must never beat itself"
        );
        // A +1000 target caps at 999, below the baseline's 1500, so nothing wins.
        assert!(
            winners.is_empty(),
            "no rule improves on holding here: {:?}",
            winners.iter().map(|w| w.label()).collect::<Vec<_>>()
        );

        // And a single-sided rule IS eligible when it does win -- proved by
        // giving one a path holding cannot capture.
        let mut better = Vec::new();
        for i in 0u8..40 {
            better.push(step(i, 100, 1_000, 1_000, 1_000));
            better.push(step(i, 200, 1_150, 1_000, 500));
        }
        let r = evaluate(&better);
        assert_eq!(
            r.baseline().and_then(|b| b.pessimistic.median()),
            Some(-5_000)
        );
        let winners = r.beats_baseline();
        assert!(
            winners
                .iter()
                .any(|w| w.target_bps == Some(1_000) && w.stop_bps.is_none()),
            "a target-only rule that beats holding must be reported"
        );
    }

    #[test]
    fn only_a_rule_with_neither_threshold_is_the_baseline() {
        // Both halves of the condition, in both directions. Inside `find` this
        // was untestable -- GRID's first entry satisfies `&&` and `||` alike, so
        // a widened condition returned the same rule and no test could see it.
        let rule = |t, s| Rule {
            target_bps: t,
            stop_bps: s,
            pessimistic: Outcomes::default(),
            optimistic: Outcomes::default(),
        };
        assert!(rule(None, None).is_baseline());
        assert!(!rule(Some(1), None).is_baseline(), "a target is a rule");
        assert!(!rule(None, Some(1)).is_baseline(), "so is a stop");
        assert!(!rule(Some(1), Some(1)).is_baseline());
    }

    #[test]
    fn every_ending_is_tallied_including_the_positions_that_were_just_held() {
        // `held` is the count nothing else asserted, so its tally could become a
        // no-op unnoticed -- and it is the largest of the three on this venue,
        // where 96% of paths never move again. A report claiming zero held
        // positions would be claiming every position was closed by a rule.
        // One that reaches a target, one that is stopped, one that just sits.
        let outcomes = [
            step(1, 100, 1_000, 1_000, 1_000),
            step(1, 200, 2_000, 1_000, 2_000),
            step(2, 100, 1_000, 1_000, 1_000),
            step(2, 200, 1_000, 500, 500),
            step(3, 100, 1_000, 1_000, 1_000),
            step(3, 200, 1_000, 1_000, 1_000),
        ];

        let r = evaluate(&outcomes);
        let both = rule_of(&r, Some(2_500), Some(2_500));
        assert_eq!(both.pessimistic.target, 1);
        assert_eq!(both.pessimistic.stopped, 1);
        assert_eq!(both.pessimistic.held, 1);
        assert_eq!(
            both.pessimistic.target + both.pessimistic.stopped + both.pessimistic.held,
            both.pessimistic.n(),
            "every position has exactly one ending"
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
