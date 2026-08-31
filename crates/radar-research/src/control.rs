// SPDX-License-Identifier: Apache-2.0
//! A control that could actually have been traded.
//!
//! [`0014`] measured Radar's selection against the tokens it refused and found
//! the comparison unusable: **all 606 scoreable refusals were
//! `CapacityBelowFloor`**, so the control was composed entirely of tokens Radar
//! had measured and found it could not sell. It concluded that a real control
//! needs the strategy to refuse something *after* the exit probe, which it does
//! not do.
//!
//! That conclusion was too pessimistic, and [`0016`] is why. It found that the
//! selection's entry price is a **sell quote** while its exit is a **realised
//! fill** — two instruments, worth at least 128 bps of spurious return. The fix
//! for that also removes the obstacle here:
//!
//! **Price both cohorts from realised fills, and a population control becomes
//! available.** A token Radar never examined has no quote, which is why the
//! quote-based measurement could not include one. It does have outcome
//! measurements, and so does every token Radar decided on. Measured
//! outcome-to-outcome, the selected cohort and the population are priced by the
//! same instrument and can be compared.
//!
//! # What is matched, and why those two things
//!
//! A raw comparison of medians would be dominated by two confounders, and both
//! are already known to matter in this data.
//!
//! - **Token age at entry.** `creator_edge` acts around forty minutes after
//!   launch. A population token measured from its first checkpoint is being
//!   priced at a different point in its life.
//! - **Holding period.** [`0011`] says this outright: "`last_price` depends on
//!   which checkpoint a token was last measured at, and the mix of checkpoint
//!   ages changes as the store grows", and shows the population median moving
//!   from −1,340 to −863 bps for that reason alone. A cohort measured over
//!   shorter holds looks better than one measured over longer holds, whatever
//!   either contains.
//!
//! So returns are stratified on both, and only strata where **both** cohorts
//! clear the cohort floor contribute. A stratum the control cannot populate is
//! one where the comparison has nothing to say, and including it would let the
//! selection be measured against itself.
//!
//! # What this still cannot do
//!
//! It does not make the comparison causal. Radar chose its cohort on creator
//! history and the population did not choose itself at all, so a difference here
//! is evidence about the selection *rule* under the conditions it ran in, not
//! about what would happen if it ran somewhere else.
//!
//! [`0011`]: ../../docs/research/0011-graduation-predicts-volatility-not-profit.md
//! [`0014`]: ../../docs/research/0014-the-control-was-entirely-tokens-nobody-could-sell.md
//! [`0016`]: ../../docs/research/0016-the-entry-was-a-bid-and-the-exit-was-a-mid.md

use std::collections::{BTreeMap, BTreeSet};

use radar_store::{Conclusion, Decision, Outcome};
use radar_types::Address;
use serde::Serialize;

/// The smallest cohort worth reporting a percentile for, per stratum.
///
/// The same floor and reasoning as [`selection::MIN_COHORT`](crate::selection::MIN_COHORT),
/// applied per stratum rather than overall — because a pooled figure assembled
/// from strata that are individually noise is noise with more decimal places.
pub const MIN_STRATUM: usize = 20;

/// Token-age boundaries, in slots, that define the age strata.
///
/// At ~2.5 slots a second: twenty minutes, forty, ninety, and beyond.
/// `creator_edge`'s `max_token_age` is 6,000 slots, so its own decisions land in
/// the first three and the last exists to hold the population tokens that do
/// not.
pub const AGE_STRATA: [(&str, u64); 4] = [
    ("<20m", 3_000),
    ("<40m", 6_000),
    ("<90m", 13_500),
    ("90m+", u64::MAX),
];

/// Holding-period boundaries, in slots, that define the hold strata.
///
/// One hour, six, twenty-four, and beyond. The spread [`0011`] identifies as
/// moving the population median on its own.
///
/// [`0011`]: ../../docs/research/0011-graduation-predicts-volatility-not-profit.md
pub const HOLD_STRATA: [(&str, u64); 4] = [
    ("<1h", 9_000),
    ("<6h", 54_000),
    ("<24h", 216_000),
    ("24h+", u64::MAX),
];

/// One realised-to-realised return.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
pub struct Realised {
    /// Token age at the entry observation, in slots.
    pub entry_age_slots: u64,
    /// Slots held between the two observations.
    pub hold_slots: u64,
    /// The return, in basis points, entry observation to exit observation.
    pub bps: i64,
}

/// One (age, hold) cell holding both cohorts.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Default)]
pub struct Stratum {
    /// Age stratum label.
    pub age: String,
    /// Hold stratum label.
    pub hold: String,
    /// Returns from tokens Radar proposed, ascending.
    pub selected_bps: Vec<i64>,
    /// Returns from tokens Radar never decided on, ascending.
    pub control_bps: Vec<i64>,
}

impl Stratum {
    /// Whether both cohorts clear [`MIN_STRATUM`].
    ///
    /// Both, not either. A stratum only one cohort populates cannot compare
    /// anything, and letting it contribute would measure the selection against
    /// itself.
    #[must_use]
    pub fn is_comparable(&self) -> bool {
        self.selected_bps.len() >= MIN_STRATUM && self.control_bps.len() >= MIN_STRATUM
    }

    /// Whether a stratum is too thin on either side to compare.
    ///
    /// The complement of [`is_comparable`](Self::is_comparable), and it exists so
    /// callers need no `!`. A negation at a call site is a branch the predicate's
    /// own tests cannot reach, and `just mutants` deletes exactly that `!`.
    #[must_use]
    pub fn is_thin(&self) -> bool {
        !self.is_comparable()
    }

    /// The selected cohort's median, or `None` when it is empty.
    #[must_use]
    pub fn selected_median(&self) -> Option<i64> {
        median(&self.selected_bps)
    }

    /// The control cohort's median, or `None` when it is empty.
    #[must_use]
    pub fn control_median(&self) -> Option<i64> {
        median(&self.control_bps)
    }

    /// The share of a cohort that returned exactly zero, per ten thousand.
    ///
    /// Diagnostic, and it earned its place. The first two runs of this
    /// comparison reported a median of exactly 0 in the largest strata, and a
    /// median cannot distinguish "the distribution is centred on zero" from "more
    /// than half of it *is* zero". Those want opposite responses: the first is a
    /// finding, the second is a measurement still admitting tokens that never
    /// moved.
    ///
    /// A large share here is a warning about the cohort, not a fact about the
    /// market.
    #[must_use]
    pub fn zero_share_bps(returns: &[i64]) -> Option<u64> {
        if returns.is_empty() {
            return None;
        }
        let zeros = u64::try_from(returns.iter().filter(|r| **r == 0).count()).unwrap_or(0);
        let total = u64::try_from(returns.len()).unwrap_or(1);
        Some(zeros.saturating_mul(10_000) / total)
    }

    /// The reading at a percentile of a sorted cohort, or `None` when empty.
    #[must_use]
    pub fn percentile(sorted: &[i64], p: f64) -> Option<i64> {
        crate::percentile(sorted, p)
    }

    /// Selected minus control, in basis points, where both are present.
    ///
    /// Positive means Radar's cohort did better than a matched token it never
    /// looked at.
    #[must_use]
    pub fn edge_bps(&self) -> Option<i64> {
        Some(self.selected_median()? - self.control_median()?)
    }
}

/// The median of a sorted slice, or `None` when it is empty.
fn median(sorted: &[i64]) -> Option<i64> {
    if sorted.is_empty() {
        return None;
    }
    Some(sorted[sorted.len() / 2])
}

/// What the comparison is able to say.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum Verdict {
    /// No stratum has both cohorts populated.
    NoComparableStratum {
        /// Strata that held anything at all.
        populated: usize,
    },
    /// The selection's edge over a matched control.
    Measured {
        /// Strata contributing, each with both cohorts above the floor.
        strata: usize,
        /// Median of the per-stratum edges, in basis points.
        ///
        /// A median of medians rather than a pooled median: pooling would let a
        /// stratum with more rows dominate, and the strata exist precisely
        /// because their rows are not interchangeable.
        median_edge_bps: i64,
        /// How many contributing strata favour the selection.
        strata_favouring_selection: usize,
    },
}

/// The comparison.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct Report {
    /// Proposals that produced a realised-to-realised return.
    pub selected: usize,
    /// Control tokens that produced one.
    pub control: usize,
    /// Every stratum holding at least one return, in a stable order.
    pub strata: Vec<Stratum>,
}

impl Report {
    /// Strata where both cohorts clear the floor.
    #[must_use]
    pub fn comparable(&self) -> Vec<&Stratum> {
        self.strata.iter().filter(|s| s.is_comparable()).collect()
    }

    /// What can be said, given how the strata filled.
    #[must_use]
    pub fn verdict(&self) -> Verdict {
        let comparable = self.comparable();
        if comparable.is_empty() {
            return Verdict::NoComparableStratum {
                populated: self.strata.len(),
            };
        }
        let mut edges: Vec<i64> = comparable.iter().filter_map(|s| s.edge_bps()).collect();
        edges.sort_unstable();
        Verdict::Measured {
            strata: comparable.len(),
            median_edge_bps: median(&edges).unwrap_or(0),
            strata_favouring_selection: edges.iter().filter(|e| **e > 0).count(),
        }
    }
}

/// Exit-capacity bands, in micro-USD, that define the depth cohorts.
///
/// Read off the measured distribution rather than chosen for roundness. Across
/// 2,365 recorded proposals capacity has a p10 of $26.90, a median of $31.03 and
/// a p90 of $34.59 — **eighty per cent inside a ±13% band** — because every
/// pre-graduation pump.fun token rides the same bonding curve with the same
/// supply. So the bands are tight where the mass is and wide in the tail, which
/// is the only part that can differ.
///
/// The tail is the whole question. Twenty-eight of those 2,365 cleared $60 and
/// ten cleared $100, and nobody has looked at what they did.
pub const CAPACITY_BANDS: [(&str, u64); 5] = [
    ("<$25", 25_000_000),
    ("$25-30", 30_000_000),
    ("$30-35", 35_000_000),
    ("$35-60", 60_000_000),
    ("$60+", u64::MAX),
];

/// One capacity band's realised returns.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Default)]
pub struct Band {
    /// The band's label.
    pub label: String,
    /// Realised returns from proposals in this band, ascending.
    pub returns_bps: Vec<i64>,
}

impl Band {
    /// How many proposals landed here.
    #[must_use]
    pub fn n(&self) -> usize {
        self.returns_bps.len()
    }

    /// The median, or `None` when the band is empty.
    #[must_use]
    pub fn median(&self) -> Option<i64> {
        median(&self.returns_bps)
    }

    /// The share returning exactly zero, per ten thousand.
    ///
    /// Carried for the same reason [`Stratum::zero_share_bps`] is: a median of
    /// zero over a cohort that is mostly zero is a report about the point mass.
    #[must_use]
    pub fn zero_share_bps(&self) -> Option<u64> {
        Stratum::zero_share_bps(&self.returns_bps)
    }
}

/// Whether no proposal landed in any capacity band.
///
/// Phrased positively so the caller needs no `!`. A negation at a call site is a
/// branch the predicate's own tests cannot reach — the same lesson `just
/// mutants` taught twice already today, in `radar selection` and in this file's
/// own `is_thin`.
#[must_use]
pub fn nothing_banded(bands: &[Band]) -> bool {
    bands.iter().all(|b| b.n() == 0)
}

/// Does the depth Radar measured predict what the token then did?
///
/// # Why this is the question the objective function forces
///
/// Radar sizes every position as a share of measured exit capacity, so capacity
/// decides how much money can move. Measured across 2,365 proposals the median
/// notional is **$6.21**, because capacity is close to a constant of the venue
/// rather than a property of the token. At that size an 850 bps round trip needs
/// a +8.5% move to break even, and no improvement to the *filter* changes it.
///
/// So the first question is not "can the selection be made better" but "is there
/// any depth here at all, and do the deeper tokens behave differently". Radar has
/// never selected **for** capacity — it selects on creator history and sizes off
/// whatever depth happens to be present.
///
/// Priced realised-to-realised, the same way [`evaluate`] prices both its
/// cohorts, because [`0016`] showed the quote-to-fill comparison carries an
/// artefact larger than anything it was measuring.
///
/// [`0016`]: ../../docs/research/0016-the-entry-was-a-bid-and-the-exit-was-a-mid.md
#[must_use]
pub fn by_capacity(decisions: &[Decision], outcomes: &[Outcome]) -> Vec<Band> {
    let by_mint = group_by_mint(outcomes);
    let mut bands: Vec<Band> = CAPACITY_BANDS
        .iter()
        .map(|(label, _)| Band {
            label: (*label).to_owned(),
            returns_bps: Vec::new(),
        })
        .collect();

    for decision in decisions {
        if !matches!(decision.conclusion, Conclusion::Proposed) {
            continue;
        }
        // Absent is not zero (rule 9). A proposal with no recorded capacity was
        // not measured as shallow; it was not measured.
        let Some(capacity) = decision.exit_capacity_micro_usd else {
            continue;
        };
        let Some(series) = by_mint.get(&decision.mint) else {
            continue;
        };
        let Some(r) = realised_from(series, decision.decided_at.get()) else {
            continue;
        };
        bands[stratum_of(&CAPACITY_BANDS, capacity)]
            .returns_bps
            .push(r.bps);
    }

    for band in &mut bands {
        band.returns_bps.sort_unstable();
    }
    bands
}

/// Compares Radar's proposals against matched tokens it never decided on.
///
/// Both cohorts are priced **outcome to outcome**, so the instrument is the same
/// on each side and on both ends. That is the whole reason this comparison is
/// possible where [`selection`](crate::selection)'s is not.
#[must_use]
pub fn evaluate(decisions: &[Decision], outcomes: &[Outcome]) -> Report {
    let by_mint = group_by_mint(outcomes);

    // Every mint Radar reached a decision on, proposed or refused. The control
    // excludes all of them, not just the proposals: a token Radar examined and
    // passed over is still a token its rules touched, and letting refusals into
    // the control is how 0014's comparison went wrong in the first place.
    let decided: BTreeSet<Address> = decisions.iter().map(|d| d.mint).collect();

    let mut cells: BTreeMap<(usize, usize), Stratum> = BTreeMap::new();
    let (mut selected, mut control) = (0usize, 0usize);

    for decision in decisions {
        if !matches!(decision.conclusion, Conclusion::Proposed) {
            continue;
        }
        let Some(series) = by_mint.get(&decision.mint) else {
            continue;
        };
        // Entry at the observation nearest the decision, exit at the last one
        // after it -- the same pairing `basis` uses for the entry, so the two
        // modules cannot disagree about what "the price when Radar decided" is.
        let Some(r) = realised_from(series, decision.decided_at.get()) else {
            continue;
        };
        selected += 1;
        push(&mut cells, r, true);
    }

    for (mint, series) in &by_mint {
        if decided.contains(mint) {
            continue;
        }
        // The population has no decision to anchor on, so every observation is a
        // candidate entry. Anchoring on the first gives one return per mint,
        // which keeps a heavily measured token from outvoting a quiet one.
        let Some(first) = series.first().map(|o| o.at) else {
            continue;
        };
        let Some(r) = realised_from(series, first) else {
            continue;
        };
        control += 1;
        push(&mut cells, r, false);
    }

    let mut strata: Vec<Stratum> = cells.into_values().collect();
    for s in &mut strata {
        s.selected_bps.sort_unstable();
        s.control_bps.sort_unstable();
    }

    Report {
        selected,
        control,
        strata,
    }
}

/// Files one return into its (age, hold) cell.
fn push(cells: &mut BTreeMap<(usize, usize), Stratum>, r: Realised, is_selected: bool) {
    let age = stratum_of(&AGE_STRATA, r.entry_age_slots);
    let hold = stratum_of(&HOLD_STRATA, r.hold_slots);
    let cell = cells.entry((age, hold)).or_insert_with(|| Stratum {
        age: AGE_STRATA[age].0.to_owned(),
        hold: HOLD_STRATA[hold].0.to_owned(),
        ..Stratum::default()
    });
    if is_selected {
        cell.selected_bps.push(r.bps);
    } else {
        cell.control_bps.push(r.bps);
    }
}

/// The index of the first stratum whose ceiling a value clears.
///
/// Total by construction — every table here ends at `u64::MAX` — which is
/// asserted by `every_stratum_table_admits_every_value` rather than assumed.
#[must_use]
pub fn stratum_of(table: &[(&str, u64)], value: u64) -> usize {
    table
        .iter()
        .position(|(_, ceiling)| value <= *ceiling)
        .unwrap_or(table.len().saturating_sub(1))
}

/// Builds one return from a mint's observation series, entering at `anchor`.
///
/// The entry is the observation nearest the anchor and the exit is the last
/// observation strictly after it **that recorded a trade in between**.
///
/// # Why a transfer must have landed in between
///
/// An [`Outcome`] reports what has happened *so far*, so once a token stops
/// trading every later observation repeats the same `last_price`. On this venue
/// most tokens die quickly, so pairing on time alone makes the majority of both
/// cohorts a return of exactly zero — and the first run of this comparison
/// produced a median of **0 bps in every stratum, on both sides**, over 201,465
/// control tokens. That is not a market observation. It is the absence of one,
/// wearing the shape of a number.
///
/// A price that has not moved because nobody traded is not a flat return; it is
/// no return at all, and averaging it in drags every statistic toward zero
/// regardless of what the live tokens did.
///
/// **The first gate for this used `Outcome::fills`, and it did not work.** That
/// field is folded with `saturating_add` across price windows overlapping by
/// five of their six hours, so it grows on every pass whether or not anything
/// traded — see [`LEARNINGS`](https://github.com/hey-vera/radar/blob/main/LEARNINGS.md)
/// entry 19. `last_transfer_slot` is a `max` over the transfer aggregate and
/// cannot be inflated by re-reading, so an advance in it is a real transfer.
///
/// Returns `None` when either end is missing, when nothing transferred between
/// them, or when the entry price is zero — absent is not zero, and a zero entry
/// would divide.
fn realised_from(series: &[Observation], anchor: u64) -> Option<Realised> {
    let entry = series.iter().min_by_key(|o| o.at.abs_diff(anchor))?;
    // The earliest observation reaching the highest fill count after entry --
    // which is the last moment a trade actually happened. Taking the *last* such
    // observation instead would price the token at a checkpoint that merely
    // repeated a stale figure, overstating the hold while leaving the price
    // unchanged, and so biasing every holding-period stratum.
    let exit = series
        .iter()
        .filter(|o| o.at > entry.at && traded_since(entry, o))
        .min_by_key(|o| (std::cmp::Reverse(o.last_transfer), o.at))?;

    let (entry_at, entry_price, launch) = (entry.at, entry.price, entry.launch);
    if entry_price == 0 {
        return None;
    }
    let delta = i128::from(exit.price) - i128::from(entry_price);
    let bps = i64::try_from(delta.saturating_mul(10_000) / i128::from(entry_price)).ok()?;

    Some(Realised {
        entry_age_slots: entry_at.saturating_sub(launch),
        hold_slots: exit.at.saturating_sub(entry_at),
        bps,
    })
}

/// One priced observation of a mint, reduced to what a return needs.
#[derive(Clone, Copy, Debug)]
struct Observation {
    /// Slot the measurement was taken at.
    at: u64,
    /// Last observed fill price.
    price: u64,
    /// Slot the token launched in.
    launch: u64,
    /// The last slot a transfer was observed in.
    ///
    /// **Not `fills`, and the difference cost a research note.** `Outcome::fills`
    /// is folded with `saturating_add` across price windows that overlap by five
    /// of their six hours, so a token whose single fill sits inside the window
    /// gains a fill on every hourly pass while its price never changes. It grows
    /// without anything trading, which makes it useless as evidence that
    /// something did.
    ///
    /// `last_transfer_slot` comes from `max(block_slot)` over the transfer
    /// aggregate. A maximum cannot be inflated by re-reading the same rows, so an
    /// advance in it is a transfer that actually happened.
    last_transfer: Option<u64>,
}

/// Whether a transfer landed between two observations of the same mint.
///
/// `None` on either side is "not measured", never "nothing happened" (rule 9) —
/// so an unmeasured end cannot be read as a quiet market, which is the direction
/// that would invent returns.
fn traded_since(entry: &Observation, exit: &Observation) -> bool {
    match (entry.last_transfer, exit.last_transfer) {
        (Some(before), Some(after)) => after > before,
        _ => false,
    }
}

/// Groups priced observations by mint, ascending by slot.
///
/// Observations with no price are dropped here rather than filtered at every use
/// — a mint nobody priced has no return, which is different from a return of
/// zero.
fn group_by_mint(outcomes: &[Outcome]) -> BTreeMap<Address, Vec<Observation>> {
    let mut by_mint: BTreeMap<Address, Vec<Observation>> = BTreeMap::new();
    for o in outcomes {
        let Some(price) = o.last_price.filter(|p| *p > 0) else {
            continue;
        };
        by_mint.entry(o.mint).or_default().push(Observation {
            at: o.measured_at.get(),
            price,
            launch: o.launch_slot.get(),
            last_transfer: o.last_transfer_slot.map(radar_types::Slot::get),
        });
    }
    for series in by_mint.values_mut() {
        series.sort_unstable_by_key(|o| o.at);
    }
    by_mint
}

#[cfg(test)]
mod tests {
    use super::*;
    use radar_types::Slot;

    fn decision(mint: u8, decided_at: u64, conclusion: Conclusion) -> Decision {
        Decision {
            mint: Address::new([mint; 32]),
            creator: Address::new([99u8; 32]),
            decided_at: Slot(decided_at),
            launch_slot: Slot(0),
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
            entry_price: None,
            inputs_digest: "d".to_owned(),
        }
    }

    fn outcome(mint: u8, measured_at: u64, price: u64) -> Outcome {
        outcome_traded_at(mint, measured_at, price, measured_at)
    }

    /// `last_transfer_slot` defaults to the measurement slot, so every fixture
    /// pair straddles a transfer unless a test deliberately says otherwise.
    fn outcome_traded_at(mint: u8, measured_at: u64, price: u64, last_transfer: u64) -> Outcome {
        Outcome {
            mint: Address::new([mint; 32]),
            measured_at: Slot(measured_at),
            launch_slot: Slot(0),
            first_transfer_slot: None,
            last_transfer_slot: Some(Slot(last_transfer)),
            transfers: 0,
            unique_senders: 0,
            unique_receivers: 0,
            graduated_at: None,
            first_price: None,
            last_price: Some(price),
            peak_price: None,
            trough_price: None,
            window_peak_price: None,
            window_trough_price: None,
            vwap: None,
            fills: 0,
        }
    }

    /// A mint observed at the decision slot and again an hour later.
    fn series(mint: u8, entry_at: u64, entry: u64, exit_at: u64, exit: u64) -> Vec<Outcome> {
        vec![outcome(mint, entry_at, entry), outcome(mint, exit_at, exit)]
    }

    #[test]
    fn both_cohorts_are_priced_by_the_same_instrument() {
        // The property the whole module exists for. A proposal and a control
        // token with identical observation series must produce identical
        // returns -- if they did not, the comparison would be measuring the
        // measurement.
        let mut outcomes = series(1, 4_000, 1_000, 10_000, 1_100);
        outcomes.extend(series(2, 4_000, 1_000, 10_000, 1_100));

        let r = evaluate(&[decision(1, 4_000, Conclusion::Proposed)], &outcomes);
        assert_eq!(r.selected, 1);
        assert_eq!(r.control, 1);
        let s = &r.strata[0];
        assert_eq!(s.selected_bps, vec![1_000]);
        assert_eq!(s.control_bps, vec![1_000]);
        assert_eq!(s.edge_bps(), Some(0));
    }

    #[test]
    fn a_token_radar_refused_is_kept_out_of_the_control() {
        // 0014's failure exactly: its control was tokens Radar had examined and
        // declined, which made it a statement about the refusal rule rather than
        // about the population. A refused mint belongs in neither cohort here.
        let outcomes = series(1, 4_000, 1_000, 10_000, 2_000);
        let r = evaluate(&[decision(1, 4_000, Conclusion::Passed)], &outcomes);
        assert_eq!(r.selected, 0, "a refusal is not a proposal");
        assert_eq!(r.control, 0, "nor is it an untouched token");
        assert!(r.strata.is_empty());
    }

    #[test]
    fn a_stratum_only_one_cohort_reaches_cannot_compare() {
        // Letting it contribute would measure the selection against itself.
        let mut outcomes = Vec::new();
        let mut decisions = Vec::new();
        for i in 0u8..40 {
            outcomes.extend(series(i, 4_000, 1_000, 10_000, 1_100));
            decisions.push(decision(i, 4_000, Conclusion::Proposed));
        }
        let r = evaluate(&decisions, &outcomes);
        assert!(
            r.selected >= MIN_STRATUM,
            "the selected side is well filled"
        );
        assert_eq!(r.control, 0);
        assert!(r.comparable().is_empty());
        assert!(matches!(r.verdict(), Verdict::NoComparableStratum { .. }));
    }

    #[test]
    fn the_edge_is_the_difference_between_matched_medians() {
        // Selected returns +2,000 bps, control +1,000, in the same cell.
        let mut outcomes = Vec::new();
        let mut decisions = Vec::new();
        for i in 0u8..40 {
            outcomes.extend(series(i, 4_000, 1_000, 10_000, 1_200));
            decisions.push(decision(i, 4_000, Conclusion::Proposed));
        }
        for i in 100u8..140 {
            outcomes.extend(series(i, 4_000, 1_000, 10_000, 1_100));
        }
        let r = evaluate(&decisions, &outcomes);
        let comparable = r.comparable();
        assert_eq!(comparable.len(), 1);
        // Both directions. `a_stratum_carries_the_labels_of_the_cell_it_is`
        // covers the thin case, so asserting the full one here is what stops
        // `is_thin` being replaced by a constant in either direction.
        assert!(!comparable[0].is_thin(), "a full stratum is not thin");
        assert_eq!(comparable[0].selected_median(), Some(2_000));
        assert_eq!(comparable[0].control_median(), Some(1_000));

        let Verdict::Measured {
            median_edge_bps,
            strata,
            strata_favouring_selection,
        } = r.verdict()
        else {
            panic!("expected a measurement");
        };
        assert_eq!(strata, 1);
        assert_eq!(median_edge_bps, 1_000);
        assert_eq!(strata_favouring_selection, 1);
    }

    #[test]
    fn holding_period_separates_strata_that_would_otherwise_pool() {
        // 0011's confounder made structural. Two cohorts with identical entries
        // but very different holds must not land in one cell, because the
        // population median moves with the hold alone.
        let mut outcomes = Vec::new();
        let mut decisions = Vec::new();
        for i in 0u8..40 {
            // Held under an hour.
            outcomes.extend(series(i, 4_000, 1_000, 8_000, 1_100));
            decisions.push(decision(i, 4_000, Conclusion::Proposed));
        }
        for i in 100u8..140 {
            // Held over a day.
            outcomes.extend(series(i, 4_000, 1_000, 300_000, 1_100));
        }
        let r = evaluate(&decisions, &outcomes);
        assert_eq!(r.strata.len(), 2, "different holds are different strata");
        assert!(
            r.comparable().is_empty(),
            "and neither can be compared against the other"
        );
    }

    #[test]
    fn a_price_that_never_moved_because_nobody_traded_is_not_a_return() {
        // The bug that made the first live run meaningless: every stratum, both
        // cohorts, a median of exactly 0 bps over 201,465 control tokens.
        //
        // An `Outcome` reports what has happened so far, so a token that stopped
        // trading repeats the same `last_price` at every later checkpoint. On
        // this venue most tokens die quickly, so pairing on time alone makes the
        // majority of both cohorts a flat zero and drags every statistic toward
        // it regardless of what the live tokens did.
        //
        // A price that has not moved because nobody traded is the absence of a
        // return, not a flat one.
        let dead = [
            outcome_traded_at(1, 4_000, 1_000, 7),
            outcome_traded_at(1, 10_000, 1_000, 7),
            outcome_traded_at(1, 90_000, 1_000, 7),
        ];
        let r = evaluate(&[decision(1, 4_000, Conclusion::Proposed)], &dead);
        assert_eq!(r.selected, 0, "no trade means no return to measure");

        // The same series with one further fill is a real, measurable move.
        let alive = [
            outcome_traded_at(1, 4_000, 1_000, 7),
            outcome_traded_at(1, 10_000, 1_100, 8),
        ];
        let r = evaluate(&[decision(1, 4_000, Conclusion::Proposed)], &alive);
        assert_eq!(r.selected, 1);
        assert_eq!(r.strata[0].selected_bps, vec![1_000]);
    }

    #[test]
    fn the_exit_is_the_last_observation_that_actually_traded() {
        // Not simply the last observation. A token that traded, then went quiet,
        // must be priced at its last real fill rather than at a checkpoint that
        // merely repeated it -- otherwise the hold is overstated while the price
        // is not, which biases the holding-period strata.
        let series = [
            outcome_traded_at(1, 4_000, 1_000, 5),
            outcome_traded_at(1, 10_000, 1_200, 9),
            outcome_traded_at(1, 400_000, 1_200, 9),
        ];
        let r = evaluate(&[decision(1, 4_000, Conclusion::Proposed)], &series);
        assert_eq!(r.selected, 1);
        assert_eq!(
            r.strata[0].hold, "<1h",
            "the hold ends at the last trade, not the last checkpoint"
        );
    }

    #[test]
    fn a_growing_fill_count_is_not_evidence_that_anything_traded() {
        // The defect this gate was rewritten for, pinned as a property.
        //
        // `Outcome::fills` is folded with `saturating_add` across price windows
        // that overlap by five of their six hours, so a token whose single fill
        // sits inside the window gains a fill on every hourly pass while nothing
        // trades and its price never moves. A gate keyed on `fills` therefore
        // admits pairs that straddle no trade at all, which is how research 0017
        // came to report 64-91% of its short holds as exactly zero.
        //
        // Here `fills` climbs 3 -> 9 -> 27 while `last_transfer_slot` never
        // advances. Nothing traded. A gate reading `fills` would pair these.
        let mut a = outcome_traded_at(1, 4_000, 1_000, 3_900);
        a.fills = 3;
        let mut b = outcome_traded_at(1, 10_000, 1_000, 3_900);
        b.fills = 9;
        let mut c = outcome_traded_at(1, 16_000, 1_000, 3_900);
        c.fills = 27;

        let r = evaluate(&[decision(1, 4_000, Conclusion::Proposed)], &[a, b, c]);
        assert_eq!(
            r.selected, 0,
            "a fill count inflated by window overlap is not a trade"
        );
    }

    #[test]
    fn an_unmeasured_transfer_slot_is_not_read_as_a_quiet_market() {
        // Rule 9 on the gate itself. `None` means nobody measured, and treating
        // it as "no transfer" would silently drop live tokens, while treating it
        // as "a transfer" would invent returns. Neither end may pair.
        let mut a = outcome_traded_at(1, 4_000, 1_000, 3_900);
        a.last_transfer_slot = None;
        let b = outcome_traded_at(1, 10_000, 1_100, 9_000);
        let r = evaluate(&[decision(1, 4_000, Conclusion::Proposed)], &[a, b]);
        assert_eq!(r.selected, 0);
    }

    #[test]
    fn a_single_observation_yields_no_return() {
        // An entry with nothing after it is not a round trip. Reporting it as
        // zero would fold "not measurable" into "broke even".
        let outcomes = [outcome(1, 4_000, 1_000)];
        let r = evaluate(&[decision(1, 4_000, Conclusion::Proposed)], &outcomes);
        assert_eq!(r.selected, 0);
        assert_eq!(r.control, 0);
    }

    #[test]
    fn a_zero_or_absent_price_never_becomes_an_entry() {
        // Rule 9. A zero entry would divide; an absent one is not a price at all.
        let mut outcomes = [outcome(1, 4_000, 0), outcome(1, 10_000, 1_100)];
        outcomes[0].last_price = Some(0);
        let mut none_priced = outcome(2, 4_000, 1);
        none_priced.last_price = None;

        let r = evaluate(
            &[decision(1, 4_000, Conclusion::Proposed)],
            &[outcomes[0].clone(), outcomes[1].clone(), none_priced],
        );
        // The zero observation is dropped, leaving one priced observation and so
        // no pair.
        assert_eq!(r.selected, 0);
    }

    #[test]
    fn the_zero_share_counts_equality_and_scales_by_ten_thousand() {
        // The diagnostic that made the live result readable, and it went in
        // untested -- `==` could become `!=` and the division could become a
        // remainder without anything noticing.
        assert_eq!(Stratum::zero_share_bps(&[0, 0, 0, 1]), Some(7_500));
        assert_eq!(Stratum::zero_share_bps(&[1, 2, 3]), Some(0));
        assert_eq!(Stratum::zero_share_bps(&[0, 0]), Some(10_000));
        // Absent is not zero: an empty cohort has no share, and reporting 0
        // would read as "none of these were flat" (rule 9).
        assert_eq!(Stratum::zero_share_bps(&[]), None);
    }

    #[test]
    fn a_stratum_that_exactly_ties_does_not_favour_the_selection() {
        // `strata_favouring_selection` counts edges strictly above zero. A tie is
        // not a win, and widening it to `>=` would report the live result -- where
        // two strata come out at exactly 0 -- as three of four favouring Radar.
        let mut outcomes = Vec::new();
        let mut decisions = Vec::new();
        for i in 0u8..40 {
            outcomes.extend(series(i, 4_000, 1_000, 10_000, 1_100));
            decisions.push(decision(i, 4_000, Conclusion::Proposed));
        }
        for i in 100u8..140 {
            outcomes.extend(series(i, 4_000, 1_000, 10_000, 1_100));
        }
        let r = evaluate(&decisions, &outcomes);
        let Verdict::Measured {
            median_edge_bps,
            strata_favouring_selection,
            ..
        } = r.verdict()
        else {
            panic!("expected a measurement");
        };
        assert_eq!(median_edge_bps, 0);
        assert_eq!(strata_favouring_selection, 0, "a tie is not a win");
    }

    #[test]
    fn a_stratum_carries_the_labels_of_the_cell_it_is() {
        // The labels are how a reader knows which cell they are looking at, and
        // `..Default::default()` will happily leave them empty. A row labelled
        // "" is a row nobody can act on.
        let outcomes = series(1, 4_000, 1_000, 10_000, 1_100);
        let r = evaluate(&[decision(1, 4_000, Conclusion::Proposed)], &outcomes);
        let s = &r.strata[0];
        assert!(s.is_thin(), "one row is below the floor");
        assert!(!s.age.is_empty(), "the age label must be set");
        assert!(!s.hold.is_empty(), "the hold label must be set");
        assert_eq!(s.age, "<40m");
        assert_eq!(s.hold, "<1h");
    }

    #[test]
    fn an_observation_at_the_entry_slot_is_not_an_exit() {
        // The exit must be strictly later. Widened to `>=`, the entry could pair
        // with a second observation taken at the same slot -- a hold of zero,
        // which is not a holding period and would land in the tightest stratum.
        let same_slot = [
            outcome_traded_at(1, 4_000, 1_000, 5),
            outcome_traded_at(1, 4_000, 2_000, 9),
        ];
        let r = evaluate(&[decision(1, 4_000, Conclusion::Proposed)], &same_slot);
        assert_eq!(r.selected, 0, "a hold of zero slots is not a hold");
    }

    #[test]
    fn capacity_bands_separate_proposals_by_the_depth_that_was_measured() {
        let mut outcomes = Vec::new();
        let mut decisions = Vec::new();
        // A shallow proposal that lost, and a deep one that gained.
        let mut shallow = decision(1, 4_000, Conclusion::Proposed);
        shallow.exit_capacity_micro_usd = Some(20_000_000);
        outcomes.extend(series(1, 4_000, 1_000, 10_000, 900));
        decisions.push(shallow);

        let mut deep = decision(2, 4_000, Conclusion::Proposed);
        deep.exit_capacity_micro_usd = Some(120_000_000);
        outcomes.extend(series(2, 4_000, 1_000, 10_000, 1_500));
        decisions.push(deep);

        let bands = by_capacity(&decisions, &outcomes);
        assert_eq!(bands[0].label, "<$25");
        assert_eq!(bands[0].median(), Some(-1_000));
        assert_eq!(bands[4].label, "$60+");
        assert_eq!(bands[4].median(), Some(5_000));
        assert!(bands[1].returns_bps.is_empty());
    }

    #[test]
    fn a_band_reports_its_own_count_and_zero_share() {
        // Both accessors went in untested, and `just mutants` replaced each with
        // a constant without anything failing. A band's count is what tells a
        // reader whether its median is worth reading -- the $60+ band's n=16 is
        // the entire caveat of research 0018 -- so a constant there is not
        // cosmetic.
        let band = Band {
            label: "t".to_owned(),
            returns_bps: vec![-100, 0, 0, 500],
        };
        assert_eq!(band.n(), 4);
        assert_eq!(band.median(), Some(0));
        assert_eq!(band.zero_share_bps(), Some(5_000));

        let empty = Band {
            label: "t".to_owned(),
            returns_bps: Vec::new(),
        };
        assert_eq!(empty.n(), 0);
        // Absent, never zero (rule 9): an empty band has no share, and reporting
        // 0 would read as "none of these were flat".
        assert_eq!(empty.zero_share_bps(), None);
        assert_eq!(empty.median(), None);
    }

    #[test]
    fn the_bands_are_printed_only_when_something_landed_in_one() {
        let empty: Vec<Band> = CAPACITY_BANDS
            .iter()
            .map(|(label, _)| Band {
                label: (*label).to_owned(),
                returns_bps: Vec::new(),
            })
            .collect();
        assert!(nothing_banded(&empty));

        let mut some = empty.clone();
        some[2].returns_bps.push(-1);
        assert!(!nothing_banded(&some), "one banded proposal is enough");
    }

    #[test]
    fn a_proposal_with_no_measured_capacity_is_not_banded_as_shallow() {
        // Rule 9. `None` means the depth was never measured, not that it was
        // small -- and filing it in the lowest band would invent evidence that
        // the deepest cohort is thinner than it is.
        let outcomes = series(1, 4_000, 1_000, 10_000, 1_100);
        let mut d = decision(1, 4_000, Conclusion::Proposed);
        d.exit_capacity_micro_usd = None;
        let bands = by_capacity(&[d], &outcomes);
        assert!(bands.iter().all(|b| b.returns_bps.is_empty()));
    }

    #[test]
    fn refusals_never_enter_a_capacity_band() {
        // The question is what Radar's *proposals* did at each depth. A refusal
        // is not a position it would have taken.
        let outcomes = series(1, 4_000, 1_000, 10_000, 1_100);
        let mut d = decision(1, 4_000, Conclusion::Passed);
        d.exit_capacity_micro_usd = Some(31_000_000);
        let bands = by_capacity(&[d], &outcomes);
        assert!(bands.iter().all(|b| b.returns_bps.is_empty()));
    }

    #[test]
    fn every_stratum_table_admits_every_value() {
        // `stratum_of` falls back to the last index, and that fallback is
        // unreachable only while every table ends at `u64::MAX`. Asserted rather
        // than assumed, so narrowing a table fails here instead of silently
        // routing values into the fallback.
        for table in [&AGE_STRATA[..], &HOLD_STRATA[..], &CAPACITY_BANDS[..]] {
            assert_eq!(
                table.last().expect("a stratum table is not empty").1,
                u64::MAX,
                "the last stratum must admit every value"
            );
            for (i, (_, ceiling)) in table.iter().enumerate() {
                assert_eq!(stratum_of(table, *ceiling), i);
                if *ceiling != u64::MAX {
                    assert_eq!(stratum_of(table, ceiling + 1), i + 1);
                }
            }
        }
    }
}
