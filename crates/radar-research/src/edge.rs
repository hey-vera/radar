// SPDX-License-Identifier: Apache-2.0
//! E3 — the walk-forward protocol, so that a null result is a result.
//!
//! Research 0017 measures Radar's selection edge at **0 bps** and research 0022
//! puts the bar at roughly **456**. This is the instrument that can move that
//! number, and the only one: nothing else in the repository asks whether any
//! stratum of recorded launches clears the bar on rows it was not fitted on.
//!
//! Design 0010 §6.2 sets the protocol and this implements it exactly, with the
//! two readings it left open written down in [`FOLDS`] and [`Reading::clears`].
//!
//! # The shape
//!
//! Five contiguous windows by launch slot, equal in rows, never shuffled — time
//! order is the whole point. The first three are the fitting period; the last
//! two are the test folds, which is what "measured on two non-overlapping test
//! folds" means with five windows.
//!
//! **Purged, with an embargo.** A label takes time to mature: a row's
//! twenty-four-hour return is not known until twenty-four hours after its
//! launch. So a fitting row whose label matures after the fitting period's
//! boundary is dropped (the purge), and each later fold begins
//! [`EMBARGO_SLOTS`] after the boundary before it (the embargo). Without both,
//! a row in the fitting period and a row in the test fold can share overlapping
//! label windows, and the test fold is then not independent of the fit. López
//! de Prado, *Advances in Financial Machine Learning* (2018), chapter 7.
//!
//! # Why the count of strata tried is printed
//!
//! A winner chosen from ten thousand candidates is expected to regress. The two
//! test folds are the control, and the count is what lets a reader discount the
//! result rather than take it at face value. It is on every verdict, including
//! the null ones.
//!
//! # What this cannot do
//!
//! Catch a leak. A leaked feature does not fail on the second fold, it wins
//! every fold, so the fold design is the wrong instrument for it entirely —
//! which is why the guard lives in [`features`](crate::features), at the point
//! a number enters a row. Overfitting *does* die on fold two, and
//! [`Options::noise_seed`] plants exactly that: a feature that is uniform noise
//! by construction, which the protocol must never report as `Found`.

use radar_roast::BaseRates;
use radar_types::Slot;

use crate::features::{FEATURES, FeatureTable, Row};
use crate::wilson_bounds;

/// Windows the rows are split into.
///
/// Five, and three of them fit: design 0010 §6.2 asks for five contiguous
/// windows and two non-overlapping test folds, which leaves the first three as
/// the fitting period. Said here rather than left to a reader to infer, because
/// "the fit fold" in the singular could as easily have meant one window and
/// three wasted.
pub const FOLDS: usize = 5;

/// How many of those windows are fitted on.
pub const FIT_FOLDS: usize = 3;

/// Rows a stratum must hold in a fold before any figure is reported for it.
///
/// A median over eleven rows is a story about eleven tokens.
pub const MIN_ROWS: usize = 100;

/// Rows a stratum should be expected to hold in a test fold before it is worth
/// fitting on.
///
/// [`MIN_ROWS`] plus two standard deviations of a count that size — a hundred
/// plus twenty. The acceptance floor is a hundred; this is the fitting-side
/// margin that keeps a stratum whose expectation sits exactly on the floor from
/// being chosen and then failing it by one row.
pub const TESTABLE_ROWS: usize = 120;

/// Slots between one fold's boundary and the next fold's first admissible row.
///
/// Twenty-four hours. The widest label this harness measures, so no row in a
/// later fold has a label window overlapping a row in an earlier one.
pub const EMBARGO_SLOTS: u64 = 216_000;

/// Terms a stratum may conjoin.
pub const MAX_TERMS: usize = 3;

/// A notional band from the snapshot's `by_notional` table, for sensitivity.
///
/// Not the default. See [`Cost`] for why the fresh-launch figure is.
pub const A_NOTIONAL_BAND: &str = "$2-$20";

/// What round trip a stratum's return is charged.
///
/// # Why this is not simply a band from `by_notional`
///
/// Design 0010 §6.1 said the round trip comes from the snapshot's `by_notional`
/// table, and plan 0007 Q2 asked whether charging a band *and* requiring 456 bps
/// on top double-charges. `docs/STATE.md`'s reconciliation of the three cost
/// figures answers both, and the answer is sharper than the question:
///
/// > 250 and 456 are the same measurement read in two different bands.
///
/// **456 is `by_notional["$20-$200"]`.** So requiring it beside a band's own
/// round trip charges one measurement twice, against one position, in two
/// bands at once. There is no second bar; the bar *is* the round trip, and
/// clearing it is exactly "the net pays for itself".
///
/// The same paragraph settles which round trip. `by_notional` is 0019's table
/// over **all** pump.fun trades in an hour. The rows this harness scores are
/// **fresh launches**, and 0019 measured that cohort separately at 850 bps —
/// a new associated token account is rent, and early curve positions carry more
/// slippage. It **declined to lower the constant** on that evidence, because
/// the population is wrong and the error direction is the dangerous one: a cost
/// rounded down launders a trade past the gate that should have refused it.
///
/// So the default charges 850, the figure the kernel itself assumes, and a
/// `by_notional` band is available for sensitivity rather than as the headline.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Cost {
    /// The measured all-in round trip for the fresh-launch cohort — the
    /// population these rows belong to. `round_trip_kernel` in the snapshot.
    FreshLaunch,
    /// One row of the snapshot's `by_notional` table, by name.
    Band(String),
}

impl Cost {
    /// The round trip in basis points, and how the report should name it.
    ///
    /// # Errors
    ///
    /// [`EdgeError::NoCostBand`] when the snapshot does not name the band. A
    /// refusal rather than a fallback: charging a neighbouring band because the
    /// named one was absent would make the report say one cost and charge
    /// another.
    pub fn resolve(&self, rates: &BaseRates) -> Result<(f64, String), EdgeError> {
        match self {
            Self::FreshLaunch => Ok((
                rates.round_trip_kernel,
                "the fresh-launch cohort (0019, the population these rows are)".to_owned(),
            )),
            Self::Band(name) => rates
                .cost_bands
                .iter()
                .find(|b| b.band == *name)
                .map(|b| (b.round_trip, format!("the {} notional band", b.band)))
                .ok_or_else(|| EdgeError::NoCostBand {
                    band: name.clone(),
                    available: rates
                        .cost_bands
                        .iter()
                        .map(|b| b.band.clone())
                        .collect::<Vec<_>>()
                        .join(", "),
                }),
        }
    }
}

/// Strata to try before stopping and saying so.
///
/// Not a correctness bound. A `Found` from a partial enumeration is still a
/// `Found` — the test folds are the control either way — but a `NotFound` from
/// one means "nothing in the part searched", and [`Report::enumeration`]
/// carries which happened so the note can say the weaker thing.
pub const DEFAULT_BUDGET: usize = 4_000_000;

/// Which return the protocol is run against.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Horizon {
    /// Entry to the six-hour checkpoint.
    SixHours,
    /// Entry to the twenty-four-hour checkpoint.
    TwentyFourHours,
}

impl Horizon {
    /// How the horizon is written in a report.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::SixHours => "6h",
            Self::TwentyFourHours => "24h",
        }
    }

    /// Slots after a launch before this label is known.
    ///
    /// What the purge is measured in: a fitting row whose label matures after
    /// the fitting boundary knows something the boundary does not.
    #[must_use]
    pub const fn maturity_slots(self) -> u64 {
        match self {
            Self::SixHours => 54_000,
            Self::TwentyFourHours => 216_000,
        }
    }

    /// This row's gross return, if it has one.
    #[must_use]
    pub const fn gross_of(self, row: &Row) -> Option<f64> {
        match self {
            Self::SixHours => row.gross_6h_bps,
            Self::TwentyFourHours => row.gross_24h_bps,
        }
    }
}

/// What stopped the protocol running.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EdgeError {
    /// The snapshot has no band by that name.
    ///
    /// A refusal rather than a fallback. Picking a neighbouring band because
    /// the named one was absent would charge a different cost than the report
    /// says it charged.
    #[error("the snapshot has no cost band named {band}; it has {available}")]
    NoCostBand {
        /// The band asked for.
        band: String,
        /// What the snapshot does carry.
        available: String,
    },
    /// Fewer labelled rows than the folds and the row floor need.
    #[error(
        "{rows} labelled rows is too few for {FOLDS} folds of at least {MIN_ROWS}; the protocol would be arithmetic on noise"
    )]
    TooFewRows {
        /// Labelled rows found.
        rows: usize,
    },
}

/// One term of a stratum: a feature against a threshold, in one direction.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Term {
    /// Index into [`FEATURES`], or [`FEATURES`]`.len()` for the planted noise.
    pub feature: usize,
    /// Whether the term is `>= threshold` rather than `< threshold`.
    pub at_least: bool,
    /// The threshold, taken from a fitting-period decile.
    pub threshold: f64,
}

impl Term {
    /// The feature's name, including the planted one.
    #[must_use]
    pub fn feature_name(&self) -> &'static str {
        FEATURES.get(self.feature).copied().unwrap_or(NOISE_FEATURE)
    }

    /// Whether a value satisfies this term. An absent value satisfies nothing:
    /// a stratum naming a feature nobody measured for this row must drop the
    /// row, not read the absence as a small number.
    #[must_use]
    pub fn admits(&self, value: Option<f64>) -> bool {
        value.is_some_and(|v| {
            if self.at_least {
                v >= self.threshold
            } else {
                v < self.threshold
            }
        })
    }
}

/// The name of the planted noise feature.
pub const NOISE_FEATURE: &str = "planted_noise";

/// A conjunction of at most [`MAX_TERMS`] terms.
#[derive(Clone, PartialEq, Debug)]
pub struct Stratum {
    /// How the report names it. Fitted strata are named from their terms.
    pub name: String,
    /// The terms, all of which must hold.
    pub terms: Vec<Term>,
}

impl Stratum {
    /// A named stratum, for the fixed ones that are not fitted.
    #[must_use]
    pub fn named(name: impl Into<String>, terms: Vec<Term>) -> Self {
        Self {
            name: name.into(),
            terms,
        }
    }

    /// Whether a row is in this stratum.
    #[must_use]
    pub fn admits(&self, row: &Row, noise: Option<f64>) -> bool {
        self.terms.iter().all(|term| {
            let value = if term.feature < FEATURES.len() {
                row.value(term.feature)
            } else {
                noise
            };
            term.admits(value)
        })
    }

    /// The terms, written the way the report prints them.
    #[must_use]
    pub fn describe(&self) -> String {
        if self.terms.is_empty() {
            return "every row".to_owned();
        }
        self.terms
            .iter()
            .map(|t| {
                format!(
                    "{} {} {:.4}",
                    t.feature_name(),
                    if t.at_least { ">=" } else { "<" },
                    t.threshold
                )
            })
            .collect::<Vec<_>>()
            .join(" and ")
    }
}

/// What a stratum did over one fold.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Reading {
    /// Rows in the stratum, in this fold.
    pub n: usize,
    /// Median gross return, in basis points.
    pub median_gross: f64,
    /// Median return net of the band's round trip, in basis points.
    pub median_net: f64,
    /// Rows whose net return was above zero.
    pub positive: usize,
    /// Wilson 95% lower bound on the share above zero.
    pub wilson_lower: f64,
    /// Standard error of the median, from the interquartile range.
    ///
    /// What "this stratum is not measurably better than that one" is measured
    /// in. See [`search`]: a refinement whose apparent gain is inside this is a
    /// refinement fitted to noise.
    pub se_median: f64,
}

impl Reading {
    /// Whether this reading clears the acceptance conditions.
    ///
    /// Three of them, and no fourth. Plan 0007 Q2 asked whether charging a
    /// band's round trip *and* requiring 456 bps on top double-charges the
    /// cost. It does, and [`Cost`] carries the citation: 456 **is** the round
    /// trip for one of the bands, so there is no second bar to clear. What is
    /// left is the one condition that means anything — the return pays for the
    /// round trip — with a margin and a shape:
    ///
    /// 1. **Enough rows.** A median over eleven is a story about eleven tokens.
    /// 2. **The net is measurably above zero**, by more than the standard error
    ///    of its own median. Zero exactly is a round trip that consumed the
    ///    whole move, and a median that merely *looks* positive inside its own
    ///    noise has not been shown to be.
    /// 3. **More than half the rows paid**, at the Wilson lower bound. A median
    ///    over a point mass at zero is a report about the point mass, which is
    ///    what research 0017 found in its short-hold strata.
    #[must_use]
    pub fn clears(&self) -> bool {
        self.n >= MIN_ROWS && self.median_net > self.se_median && self.wilson_lower > 0.5
    }
}

/// A stratum with what it did in the fitting period and on each test fold.
#[derive(Clone, PartialEq, Debug)]
pub struct Candidate {
    /// The stratum.
    pub stratum: Stratum,
    /// What it did over the fitting period. `None` for a fixed stratum, which
    /// is never fitted.
    pub fit: Option<Reading>,
    /// What it did on each test fold, in order. `None` where the fold held
    /// fewer than [`MIN_ROWS`] of it.
    pub tests: Vec<Option<Reading>>,
    /// Whether it cleared the bar on **every** test fold.
    pub found: bool,
}

/// One window of rows.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Fold {
    /// The first launch slot in the window.
    pub from: Slot,
    /// The last launch slot in the window.
    pub to: Slot,
    /// Rows in it after the purge and the embargo.
    pub rows: usize,
}

/// Whether the search covered the whole grammar.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Enumeration {
    /// Every stratum in the grammar was tried.
    Exhaustive,
    /// The budget ran out first, so a null means "nothing in the part searched".
    StoppedAtBudget,
}

/// What the protocol found, or did not.
#[derive(Clone, PartialEq, Debug)]
pub struct Report {
    /// The store watermark the table was built at.
    pub watermark: Slot,
    /// Which return this was run against.
    pub horizon: Horizon,
    /// What was charged, in words, so the report says which cost it used.
    pub cost_source: String,
    /// Its round trip, in basis points.
    pub round_trip_bps: f64,
    /// `by_notional["$20-$200"]`, which circulates as "the bar". Printed for
    /// context and held to by nothing: it is the same measurement as the round
    /// trip above, read in a different band.
    pub band_bar_bps: f64,
    /// When the snapshot the two came from was measured.
    pub rates_measured_on: String,
    /// Labelled rows the protocol ran over.
    pub labelled_rows: usize,
    /// The windows.
    pub folds: Vec<Fold>,
    /// How many strata were tried.
    pub strata_tried: usize,
    /// Whether that was the whole grammar.
    pub enumeration: Enumeration,
    /// The best fitted stratum, tested. `None` when nothing in the grammar held
    /// [`MIN_ROWS`] over the fitting period.
    pub fitted: Option<Candidate>,
    /// The strata that are not fitted at all, tested on the same folds.
    pub fixed: Vec<Candidate>,
    /// Whether anything at all was found.
    pub found: bool,
}

/// How the protocol is run.
#[derive(Clone, Debug)]
pub struct Options {
    /// Which return to measure.
    pub horizon: Horizon,
    /// What round trip to charge.
    pub cost: Cost,
    /// Strata to try before stopping.
    pub budget: usize,
    /// When set, a uniform-noise feature is added to the grammar with this
    /// seed. The planted test: across seeds it must never be `Found`.
    pub noise_seed: Option<u64>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            horizon: Horizon::TwentyFourHours,
            cost: Cost::FreshLaunch,
            budget: DEFAULT_BUDGET,
            noise_seed: None,
        }
    }
}

/// One labelled row, reduced to what the protocol needs.
struct Point {
    launch_slot: Slot,
    gross: f64,
    net: f64,
    values: Vec<Option<f64>>,
}

/// Uniform noise in `[0, 1)`, from a seed and a mint.
///
/// Deterministic and seeded: the protocol reads no clock and no entropy, so a
/// planted-noise run reproduces exactly. `blake3` rather than a hand-rolled
/// mixer because the crate is already here and a weak mixer would correlate
/// with the mint bytes, which would make the planted test easier to pass than
/// it should be.
fn noise_for(seed: u64, row: &Row) -> f64 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&seed.to_le_bytes());
    hasher.update(row.mint.as_bytes());
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&hasher.finalize().as_bytes()[..8]);
    // 2^53 keeps every value exactly representable, so two runs cannot disagree
    // by a rounding step.
    #[expect(clippy::cast_precision_loss, reason = "masked below 2^53")]
    let out =
        (u64::from_le_bytes(bytes) & ((1 << 53) - 1)) as f64 / f64::from(1u32 << 31) / 4.194_304e6;
    out
}

/// Runs the protocol.
///
/// # Errors
///
/// [`EdgeError::NoCostBand`] when the snapshot does not name the band asked
/// for, [`EdgeError::TooFewRows`] when there are not enough labelled rows for
/// the folds to mean anything.
pub fn run(
    table: &FeatureTable,
    rates: &BaseRates,
    options: &Options,
) -> Result<Report, EdgeError> {
    let (round_trip, cost_source) = options.cost.resolve(rates)?;

    let points = points_of(table, options, round_trip);
    if points.len() < FOLDS * MIN_ROWS {
        return Err(EdgeError::TooFewRows { rows: points.len() });
    }

    let windows = split(&points);
    let (fit_rows, test_rows) = admissible(&points, &windows, options.horizon);

    // Each window reports its own admissible rows. The fitting period is three
    // of them and is fitted as one, but a table that piled all three counts
    // onto the third would make the purge look like it had emptied two windows.
    let folds = windows
        .iter()
        .enumerate()
        .map(|(index, (from, to))| Fold {
            from: *from,
            to: *to,
            rows: if index < FIT_FOLDS {
                fit_rows
                    .iter()
                    .filter(|i| points[**i].launch_slot >= *from && points[**i].launch_slot <= *to)
                    .count()
            } else {
                test_rows[index - FIT_FOLDS].len()
            },
        })
        .collect();

    let width = options
        .noise_seed
        .map_or(FEATURES.len(), |_| FEATURES.len() + 1);
    let terms = grammar(&points, &fit_rows, width);
    let floor = fitting_floor(fit_rows.len(), &test_rows);
    let (best, tried, enumeration) = search(
        &points,
        &fit_rows,
        &terms,
        round_trip,
        floor,
        options.budget,
    );

    let test = |stratum: &Stratum| -> Vec<Option<Reading>> {
        test_rows
            .iter()
            .map(|rows| read(&points, rows, stratum, round_trip))
            .collect()
    };
    let cleared = |readings: &[Option<Reading>]| -> bool {
        !readings.is_empty() && readings.iter().all(|r| r.is_some_and(|r| r.clears()))
    };

    let fitted = best.map(|(stratum, fit)| {
        let tests = test(&stratum);
        Candidate {
            stratum,
            fit: Some(fit),
            found: cleared(&tests),
            tests,
        }
    });

    let fixed: Vec<Candidate> = fixed_strata(rates)
        .into_iter()
        .map(|stratum| {
            let tests = test(&stratum);
            Candidate {
                stratum,
                fit: None,
                found: cleared(&tests),
                tests,
            }
        })
        .collect();

    let found = fitted.as_ref().is_some_and(|c| c.found) || fixed.iter().any(|c| c.found);

    Ok(Report {
        watermark: table.watermark,
        horizon: options.horizon,
        cost_source,
        round_trip_bps: round_trip,
        band_bar_bps: rates.round_trip_bar,
        rates_measured_on: rates.measured_on.clone(),
        labelled_rows: points.len(),
        folds,
        strata_tried: tried,
        enumeration,
        fitted,
        fixed,
        found,
    })
}

/// The labelled rows, in launch order, with the planted noise if one is asked
/// for.
///
/// A row with no label cannot be scored and is not evidence of anything, so it
/// is not carried at all rather than carried and skipped.
fn points_of(table: &FeatureTable, options: &Options, round_trip: f64) -> Vec<Point> {
    let mut points: Vec<Point> = table
        .rows
        .iter()
        .filter_map(|row| {
            let gross = options.horizon.gross_of(row)?;
            let mut values = row.values.clone();
            if let Some(seed) = options.noise_seed {
                values.push(Some(noise_for(seed, row)));
            }
            Some(Point {
                launch_slot: row.launch_slot,
                gross,
                net: gross - round_trip,
                values,
            })
        })
        .collect();
    points.sort_by_key(|p| p.launch_slot);
    points
}

/// Splits the points into [`FOLDS`] contiguous windows, equal in rows.
///
/// By position rather than by slot range: a window of equal slot width holds
/// wildly unequal numbers of launches, and a fold of forty rows cannot be
/// compared with one of forty thousand.
fn split(points: &[Point]) -> Vec<(Slot, Slot)> {
    let per = points.len() / FOLDS;
    (0..FOLDS)
        .map(|index| {
            let start = index * per;
            let end = if index == FOLDS - 1 {
                points.len() - 1
            } else {
                (start + per).saturating_sub(1)
            };
            (points[start].launch_slot, points[end].launch_slot)
        })
        .collect()
}

/// Applies the purge and the embargo.
///
/// Returns the fitting period's row indices and one set per test fold.
///
/// - **Purge**: a fitting row whose label matures after the fitting period's
///   boundary is dropped. It knows something the boundary does not.
/// - **Embargo**: a test fold's rows must launch at least [`EMBARGO_SLOTS`]
///   after the boundary before them, so no two rows either side of a boundary
///   share an overlapping label window.
fn admissible(
    points: &[Point],
    windows: &[(Slot, Slot)],
    horizon: Horizon,
) -> (Vec<usize>, Vec<Vec<usize>>) {
    let fit_boundary = windows[FIT_FOLDS - 1].1;
    let maturity = horizon.maturity_slots();

    let fit = points
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            p.launch_slot <= fit_boundary
                && p.launch_slot.get().saturating_add(maturity) <= fit_boundary.get()
        })
        .map(|(i, _)| i)
        .collect();

    let mut tests = Vec::new();
    for index in FIT_FOLDS..FOLDS {
        let (from, to) = windows[index];
        let previous_boundary = windows[index - 1].1;
        let earliest = previous_boundary.get().saturating_add(EMBARGO_SLOTS);
        tests.push(
            points
                .iter()
                .enumerate()
                .filter(|(_, p)| {
                    p.launch_slot >= from && p.launch_slot <= to && p.launch_slot.get() >= earliest
                })
                .map(|(i, _)| i)
                .collect(),
        );
    }
    (fit, tests)
}

/// Every term the grammar admits, from the fitting period's deciles.
///
/// Thresholds from the **fitting** rows only. Deciles taken over the whole
/// table would be a threshold fitted to the test folds, which is the leak the
/// fold design exists to prevent.
fn grammar(points: &[Point], fit: &[usize], width: usize) -> Vec<Term> {
    let mut terms = Vec::new();
    for feature in 0..width {
        let mut values: Vec<f64> = fit
            .iter()
            .filter_map(|i| points[*i].values.get(feature).copied().flatten())
            .collect();
        if values.len() < MIN_ROWS {
            // A feature measured for fewer rows than the floor cannot produce a
            // stratum that clears the floor, so it produces no terms at all.
            continue;
        }
        values.sort_by(f64::total_cmp);
        let mut seen: Vec<u64> = Vec::new();
        for decile in 1..10usize {
            let index = values.len() * decile / 10;
            let Some(threshold) = values.get(index).copied() else {
                continue;
            };
            // A constant feature has nine identical deciles, and nine copies of
            // one term is nine times the work for one answer. Compared by bits
            // rather than within an epsilon: every threshold here *is* a value
            // some row held, so two deciles are either the same reading or two
            // different ones, and a tolerance would quietly merge two thresholds
            // a hair apart into one statement.
            if seen.contains(&threshold.to_bits()) {
                continue;
            }
            seen.push(threshold.to_bits());
            terms.push(Term {
                feature,
                at_least: true,
                threshold,
            });
            terms.push(Term {
                feature,
                at_least: false,
                threshold,
            });
        }
    }
    terms
}

/// A bitmap of the rows a term admits, indexed against `rows`.
struct Mask {
    term: Term,
    bits: Vec<u64>,
    count: usize,
}

fn mask_of(points: &[Point], rows: &[usize], term: Term) -> Mask {
    let mut bits = vec![0u64; rows.len().div_ceil(64)];
    let mut count = 0;
    for (position, row) in rows.iter().enumerate() {
        let value = points[*row].values.get(term.feature).copied().flatten();
        if term.admits(value) {
            bits[position / 64] |= 1 << (position % 64);
            count += 1;
        }
    }
    Mask { term, bits, count }
}

/// The intersection of two masks, and how many rows it holds.
fn intersect(left: &[u64], right: &[u64]) -> (Vec<u64>, usize) {
    let bits: Vec<u64> = left.iter().zip(right).map(|(a, b)| a & b).collect();
    let count = bits.iter().map(|w| w.count_ones() as usize).sum();
    (bits, count)
}

/// Enumerates the grammar over the fitting period and returns the winner.
///
/// Bitmaps rather than a filter per stratum: a conjunction is then a word-wise
/// AND, which is what makes an exhaustive search over three-term conjunctions
/// finish at all. The row floor prunes: a conjunction below it cannot be
/// rescued by a third term, since intersecting can only remove rows.
fn search(
    points: &[Point],
    fit: &[usize],
    terms: &[Term],
    round_trip: f64,
    floor: usize,
    budget: usize,
) -> (Option<(Stratum, Reading)>, usize, Enumeration) {
    let masks: Vec<Mask> = terms
        .iter()
        .map(|t| mask_of(points, fit, *t))
        .filter(|m| m.count >= floor)
        .collect();

    let mut tried = 0usize;
    let mut best: Option<(Stratum, Reading)> = None;
    let mut enumeration = Enumeration::Exhaustive;

    let consider = |terms: Vec<Term>,
                    bits: &[u64],
                    tried: &mut usize,
                    best: &mut Option<(Stratum, Reading)>| {
        *tried += 1;
        let Some(reading) = read_bits(points, fit, bits, round_trip) else {
            return;
        };
        if reading.n < floor {
            return;
        }
        if prefer(&reading, terms.len(), best.as_ref()) {
            *best = Some((
                Stratum {
                    name: "fitted".to_owned(),
                    terms,
                },
                reading,
            ));
        }
    };

    'outer: for (i, first) in masks.iter().enumerate() {
        if tried >= budget {
            enumeration = Enumeration::StoppedAtBudget;
            break;
        }
        consider(vec![first.term], &first.bits, &mut tried, &mut best);

        for (j, second) in masks.iter().enumerate().skip(i + 1) {
            // Two terms on one feature is a range, which the grammar reaches by
            // a different pair; two thresholds on the same feature in the same
            // direction is one of them restated.
            if second.term.feature == first.term.feature {
                continue;
            }
            let (pair_bits, pair_count) = intersect(&first.bits, &second.bits);
            if pair_count < floor {
                continue;
            }
            if tried >= budget {
                enumeration = Enumeration::StoppedAtBudget;
                break 'outer;
            }
            consider(
                vec![first.term, second.term],
                &pair_bits,
                &mut tried,
                &mut best,
            );

            // Past `second`, not past `first`. Skipping only past `first`
            // enumerated every triple twice -- the same conjunction reached
            // through two different pairs -- which doubled the search and
            // doubled the count a reader uses to discount the winner.
            for third in masks.iter().skip(j + 1) {
                if third.term.feature == first.term.feature
                    || third.term.feature == second.term.feature
                {
                    continue;
                }
                let (triple_bits, triple_count) = intersect(&pair_bits, &third.bits);
                if triple_count < floor {
                    continue;
                }
                if tried >= budget {
                    enumeration = Enumeration::StoppedAtBudget;
                    break 'outer;
                }
                consider(
                    vec![first.term, second.term, third.term],
                    &triple_bits,
                    &mut tried,
                    &mut best,
                );
            }
        }
    }

    if let Some((stratum, _)) = best.as_mut() {
        stratum.name = stratum.describe();
    }
    (best, tried, enumeration)
}

/// The row floor a stratum must clear over the **fitting period**.
///
/// Not [`MIN_ROWS`]. The floor that matters is on a test fold, and the fitting
/// period is larger — three windows against one — so a stratum holding exactly
/// a hundred fitting rows holds roughly a third of that in each test fold and
/// can never be accepted. Fitting on strata that cannot be tested is how a
/// real edge gets displaced: `an_engineered_edge_is_found` failed against a
/// flat floor because the winner was the top decile of the feature carrying
/// the edge, perfect on the fitting period and twenty-one rows on a test fold.
///
/// So the floor is scaled: a stratum must hold at least the share of the
/// fitting period that would leave [`TESTABLE_ROWS`] in the **smallest** test
/// fold.
///
/// [`TESTABLE_ROWS`] rather than [`MIN_ROWS`] because a share whose *expected*
/// test-fold count is exactly the floor falls below it about half the time.
/// The engineered edge failed a second time on precisely that: ninety-nine rows
/// where a hundred were needed, one short, on a stratum that was right.
fn fitting_floor(fit_rows: usize, test_rows: &[Vec<usize>]) -> usize {
    let smallest = test_rows.iter().map(Vec::len).min().unwrap_or(0);
    if smallest == 0 {
        return MIN_ROWS;
    }
    (TESTABLE_ROWS * fit_rows).div_ceil(smallest).max(MIN_ROWS)
}

/// Whether a candidate should replace the one held, under the one-standard-error
/// rule.
///
/// # Why this is not simply "the highest median"
///
/// Design 0010 §6.2 says the fit-fold winner is the stratum with the highest
/// median net return. Taken literally that is what this did, and the test
/// `an_engineered_edge_is_found` failed against it: with a real 3,000 bps edge
/// planted in one feature, the maximiser preferred a three-term refinement that
/// had sliced the fitting period into rows whose noise happened to run high.
/// The refinement then failed the test folds, as it should — and took the real
/// edge with it, because only the winner is tested.
///
/// So a candidate has to be better by **more than one standard error of the
/// held median** to replace it. Inside that band the two have not been shown to
/// differ, and the tie is broken toward the simpler stratum — fewer terms, then
/// more rows. This is the one-standard-error rule, and it is the difference
/// between a harness that can see an edge and one that cannot.
///
/// It is a deviation from the design's sentence, found by a test rather than
/// argued from first principles, and plan 0007 Q3 records it as one.
fn prefer(candidate: &Reading, terms: usize, held: Option<&(Stratum, Reading)>) -> bool {
    let Some((held_stratum, held_reading)) = held else {
        return true;
    };
    // The larger of the two standard errors, not the incumbent's. A wide
    // stratum's median has a small one, so comparing against it alone lets any
    // narrow, noisy candidate displace it -- which is the failure this rule
    // exists to prevent, arriving from the other direction.
    let noise = held_reading.se_median.max(candidate.se_median);
    if candidate.median_net > held_reading.median_net + noise {
        return true;
    }
    if held_reading.median_net > candidate.median_net + noise {
        return false;
    }
    // Within one standard error the two medians have not been shown to differ,
    // and the second acceptance condition decides instead: the Wilson lower
    // bound on the share of rows that actually paid. That is a confidence
    // bound, so it already prefers more rows at an equal share and a higher
    // share at comparable rows -- which is exactly the trade a tie-break here
    // has to make. Fewer terms breaks a tie on that.
    if candidate.wilson_lower > held_reading.wilson_lower {
        return true;
    }
    if candidate.wilson_lower < held_reading.wilson_lower {
        return false;
    }
    terms < held_stratum.terms.len()
}

/// Reads a stratum over a set of rows.
fn read(points: &[Point], rows: &[usize], stratum: &Stratum, round_trip: f64) -> Option<Reading> {
    let matched: Vec<usize> = rows
        .iter()
        .copied()
        .filter(|i| {
            stratum
                .terms
                .iter()
                .all(|term| term.admits(points[*i].values.get(term.feature).copied().flatten()))
        })
        .collect();
    summarise(points, &matched, round_trip)
}

/// The same, from a bitmap.
fn read_bits(points: &[Point], rows: &[usize], bits: &[u64], round_trip: f64) -> Option<Reading> {
    let matched: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(position, _)| bits[position / 64] & (1 << (position % 64)) != 0)
        .map(|(_, row)| *row)
        .collect();
    summarise(points, &matched, round_trip)
}

/// The figures a reading reports.
fn summarise(points: &[Point], matched: &[usize], round_trip: f64) -> Option<Reading> {
    if matched.is_empty() {
        return None;
    }
    let mut gross: Vec<f64> = matched.iter().map(|i| points[*i].gross).collect();
    let median_gross = quantile(&mut gross, 0.5);
    let lower_quartile = quantile(&mut gross, 0.25);
    let upper_quartile = quantile(&mut gross, 0.75);
    let positive = matched.iter().filter(|i| points[**i].net > 0.0).count();
    #[expect(
        clippy::cast_precision_loss,
        reason = "row counts, far below f64's exact-integer range"
    )]
    let n = matched.len() as f64;
    // The textbook large-sample standard error of a median, with the spread
    // estimated from the interquartile range rather than from a variance: these
    // return distributions have a heavy tail and a point mass, and a variance
    // over them is dominated by a handful of rows.
    let se_median = 1.253_3 * ((upper_quartile - lower_quartile) / 1.349) / n.sqrt();
    let (wilson_lower, _) = wilson_bounds(positive as u64, matched.len() as u64)?;

    Some(Reading {
        n: matched.len(),
        median_gross,
        median_net: median_gross - round_trip,
        positive,
        wilson_lower,
        se_median,
    })
}

/// The value at `p` of a slice, by selection rather than by sorting.
///
/// A value a row actually held, never an interpolation between two: an average
/// of two returns is not a return either token had. Selection rather than a
/// sort because the enumeration calls this once per stratum, and the difference
/// between `O(n)` and `O(n log n)` there is the difference between a pass that
/// finishes and one that is abandoned.
///
/// # Panics
///
/// Never on a non-empty slice; callers hold that.
fn quantile(values: &mut [f64], p: f64) -> f64 {
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "an index into a cohort far below f64's exact-integer range"
    )]
    let index = ((values.len() as f64 * p) as usize).min(values.len() - 1);
    let (_, at, _) = values.select_nth_unstable_by(index, f64::total_cmp);
    *at
}

/// The strata that are not fitted, and are measured for that reason.
///
/// Design 0010 §6.2: `creator_edge`'s own thresholds as one stratum, and the
/// refusal signals' complements, so the **refusal edge** — how much worse the
/// refused set does than the rest — gets a number of its own. That is the
/// product Radar sells today and it has never had one.
///
/// The creator rule here is an approximation of [`radar_strategy::CreatorEdge`]
/// and says so in its name: the table counts a creator's *prior launches* where
/// the strategy counts *measured* ones, and expresses "at least
/// `min_graduation_bps` of them graduated" as "at least one did". Both
/// differences make this stratum wider than the rule, so a null here is not
/// quite a null for the rule — which is why the name carries `approx`.
fn fixed_strata(rates: &BaseRates) -> Vec<Stratum> {
    let thresholds = radar_strategy::creator_edge::Thresholds::DEFAULT;
    let index = |name: &str| crate::features::feature_index(name).expect("a name from FEATURES");
    #[expect(
        clippy::cast_precision_loss,
        reason = "thresholds are small integers, exact in f64"
    )]
    let launches = thresholds.min_measured_launches as f64;
    #[expect(
        clippy::cast_precision_loss,
        reason = "thresholds are small integers, exact in f64"
    )]
    let per_day = thresholds.max_launches_per_day as f64;

    let mut strata = vec![
        Stratum::named(
            "creator-edge-approx",
            vec![
                Term {
                    feature: index("creator_prior_launches"),
                    at_least: true,
                    threshold: launches,
                },
                Term {
                    feature: index("creator_prior_organic"),
                    at_least: true,
                    threshold: 1.0,
                },
                Term {
                    feature: index("creator_launches_per_day"),
                    at_least: false,
                    threshold: per_day,
                },
            ],
        ),
        Stratum::named(
            "refused: a record and no organic graduation",
            vec![
                Term {
                    feature: index("creator_prior_launches"),
                    at_least: true,
                    threshold: launches,
                },
                Term {
                    feature: index("creator_prior_organic"),
                    at_least: false,
                    threshold: 1.0,
                },
            ],
        ),
        Stratum::named(
            "refused: launching too fast",
            vec![Term {
                feature: index("creator_launches_per_day"),
                at_least: true,
                threshold: per_day,
            }],
        ),
    ];

    // The strongest band's lower edge, read from the snapshot rather than
    // written here. Research 0024 moved that band from six to ten-to-thirteen,
    // and a rule naming the number would have fired on the wrong launches from
    // the day it moved.
    if let Some(band) = rates.strongest_band() {
        strata.push(Stratum::named(
            format!("refused: launch block at {} recipients or above", band.lo),
            vec![Term {
                feature: index("decision_launch_recipients"),
                at_least: true,
                threshold: f64::from(band.lo),
            }],
        ));
    }
    strata
}

#[cfg(test)]
mod tests {
    use super::*;
    use radar_types::Address;
    use std::collections::BTreeSet;

    /// The repository's own snapshot, which is where the bar and the round trip
    /// have to come from.
    fn rates() -> BaseRates {
        BaseRates::load("../../docs/research/data/0024-base-rates.json").expect("the snapshot")
    }

    /// A point with one feature value and a gross return.
    fn point(slot: u64, values: Vec<Option<f64>>, gross: f64, round_trip: f64) -> Point {
        Point {
            launch_slot: Slot(slot),
            gross,
            net: gross - round_trip,
            values,
        }
    }

    /// A table of `n` rows, one feature, labelled with `gross`.
    fn table_of(n: usize, gross: impl Fn(usize) -> f64) -> FeatureTable {
        let rows = (0..n)
            .map(|index| {
                let mut values = vec![None; FEATURES.len()];
                #[expect(clippy::cast_precision_loss, reason = "a small index")]
                let value = (index % 10) as f64;
                values[0] = Some(value);
                let mut bytes = [0u8; 32];
                bytes[..8].copy_from_slice(&(index as u64).to_le_bytes());
                Row {
                    mint: Address::new(bytes),
                    creator: Address::new([1; 32]),
                    launch_slot: Slot(index as u64 * EMBARGO_SLOTS),
                    t: Slot(index as u64 * EMBARGO_SLOTS + 6_000),
                    values,
                    gross_6h_bps: Some(gross(index)),
                    gross_24h_bps: Some(gross(index)),
                    mode: None,
                }
            })
            .collect();
        FeatureTable {
            watermark: Slot(n as u64 * EMBARGO_SLOTS),
            entry_offset: crate::features::ENTRY_OFFSET_SLOTS,
            rows,
        }
    }

    #[test]
    fn a_stratum_reads_the_feature_its_term_names_and_not_the_planted_column() {
        // The other side of the index test above. A term on a real feature must
        // read that feature: reading the planted noise instead would make every
        // stratum a statement about a random number, and it would still look
        // like a stratum.
        let mut values = vec![None; FEATURES.len()];
        values[0] = Some(1.0);
        let row = Row {
            mint: Address::new([9; 32]),
            creator: Address::new([1; 32]),
            launch_slot: Slot(1),
            t: Slot(6_001),
            values,
            gross_6h_bps: None,
            gross_24h_bps: None,
            mode: None,
        };
        let real = Stratum::named(
            "real",
            vec![Term {
                feature: 0,
                at_least: true,
                threshold: 5.0,
            }],
        );
        assert!(
            !real.admits(&row, Some(9.0)),
            "the feature is 1.0 and the noise is 9.0; the term is about the feature"
        );
    }

    #[test]
    fn the_last_window_reaches_the_last_row_even_when_the_rows_do_not_divide() {
        // 503 rows into five windows is 100 apiece with three left over, and
        // the last window has to carry them. Computing its end the way the
        // others are computed would drop the final three rows from the protocol
        // entirely, silently.
        let points: Vec<Point> = (0..503u64)
            .map(|i| point(i * 10, Vec::new(), 0.0, 0.0))
            .collect();
        let windows = split(&points);

        assert_eq!(
            windows.last().copied(),
            Some((Slot(4_000), Slot(5_020))),
            "the last window ends at the last row, not at its own hundredth"
        );
        assert_eq!(windows[0], (Slot(0), Slot(990)));
    }

    #[test]
    fn a_fitting_window_reports_only_the_rows_inside_it() {
        // The window filter is an `and`. As an `or` every window would report
        // every fitting row, because each row is either at or after one edge or
        // at or before the other.
        let rates = rates();
        let table = table_of(FOLDS * MIN_ROWS * 4 + 1, |i| {
            #[expect(clippy::cast_precision_loss, reason = "a small index")]
            let v = (i % 7) as f64 * 100.0;
            v
        });
        let report = run(&table, &rates, &Options::default()).expect("runs");

        let fitting: usize = report.folds[..FIT_FOLDS].iter().map(|f| f.rows).sum();
        for fold in &report.folds[..FIT_FOLDS] {
            assert!(
                fold.rows < fitting,
                "one window reported the whole fitting period: {:?}",
                report.folds
            );
        }

        // The two test folds are different windows and must be read as such: a
        // row count taken from the wrong one would report a fold that was never
        // measured.
        assert_ne!(
            report.folds[FIT_FOLDS].rows,
            report.folds[FIT_FOLDS + 1].rows,
            "this table's last window is one row longer, so the two differ"
        );
    }

    #[test]
    fn the_planted_seed_widens_the_grammar_by_exactly_one_feature() {
        // The noise column has to reach the grammar, or the planted test is a
        // run with nothing planted in it -- which would pass, every time,
        // proving nothing.
        let rates = rates();
        let mut table = table_of(FOLDS * MIN_ROWS * 2, |i| {
            #[expect(clippy::cast_precision_loss, reason = "a small index")]
            let v = (i % 7) as f64 * 100.0;
            v
        });
        // A value in the last real feature as well, so a grammar one column too
        // narrow is visible too.
        for (index, row) in table.rows.iter_mut().enumerate() {
            #[expect(clippy::cast_precision_loss, reason = "a small index")]
            let v = (index % 3) as f64;
            let last = FEATURES.len() - 1;
            row.values[last] = Some(v);
        }

        let plain = run(&table, &rates, &Options::default()).expect("runs");
        let planted = run(
            &table,
            &rates,
            &Options {
                noise_seed: Some(1),
                ..Options::default()
            },
        )
        .expect("runs");

        assert!(
            planted.strata_tried > plain.strata_tried,
            "planting a column must add strata: {} against {}",
            planted.strata_tried,
            plain.strata_tried
        );
    }

    #[test]
    fn the_enumeration_is_exactly_the_conjunctions_the_grammar_allows() {
        // Every rule of the loop, pinned as one number. Five terms over four
        // features -- the first and the last share one -- with thresholds
        // chosen so that three pairs land exactly on the floor:
        //
        //   singles                                        4 (one term is
        //                                                     below the floor)
        //   pairs, features differing                      5
        //   triples, third past the second, features apart 2
        //                                                 --
        //                                                 11
        //
        // A floor applied with `<=` drops the three pairs on it and the triples
        // under them; a third term taken from past the *first* enumerates two
        // of these twice; an `and` between the two feature guards admits a
        // conjunction with two terms on one feature. Each shows up here as a
        // different number.
        let points: Vec<Point> = (0..300u64)
            .map(|i| {
                #[expect(clippy::cast_precision_loss, reason = "a small index")]
                let v = i as f64;
                point(i, vec![Some(v), Some(v), Some(v), Some(v)], v, 0.0)
            })
            .collect();
        let rows: Vec<usize> = (0..points.len()).collect();
        let terms = vec![
            Term {
                feature: 0,
                at_least: true,
                threshold: 100.0,
            },
            Term {
                feature: 1,
                at_least: true,
                threshold: 150.0,
            },
            Term {
                feature: 2,
                at_least: true,
                threshold: 200.0,
            },
            Term {
                feature: 3,
                at_least: true,
                threshold: 250.0,
            },
            Term {
                feature: 0,
                at_least: true,
                threshold: 120.0,
            },
        ];

        let (_, tried, enumeration) = search(&points, &rows, &terms, 0.0, 100, 10_000);
        assert_eq!(enumeration, Enumeration::Exhaustive);
        assert_eq!(tried, 11, "the grammar allows exactly eleven here");
    }

    #[test]
    fn a_clearly_better_incumbent_is_kept_however_sure_the_candidate_looks() {
        // The second comparison, which no test reached: the incumbent is better
        // by more than the noise, so nothing below it is consulted. Without it
        // a candidate with a higher Wilson bound would displace a stratum whose
        // median is plainly better.
        let held_stratum = Stratum::named(
            "held",
            vec![
                Term {
                    feature: 0,
                    at_least: true,
                    threshold: 1.0,
                },
                Term {
                    feature: 1,
                    at_least: true,
                    threshold: 1.0,
                },
                Term {
                    feature: 2,
                    at_least: true,
                    threshold: 1.0,
                },
            ],
        );
        let held = Reading {
            n: 400,
            median_gross: 400.0,
            median_net: 150.0,
            positive: 300,
            wilson_lower: 0.70,
            se_median: 2.0,
        };
        let worse = Reading {
            median_net: 100.0,
            se_median: 1.0,
            wilson_lower: 0.99,
            ..held
        };
        assert!(
            !prefer(&worse, 1, Some(&(held_stratum.clone(), held))),
            "fifty basis points worse against a noise of two is worse, and the \
             Wilson bound does not get a say"
        );

        // The boundary, in the same direction as the first comparison's: worse
        // by exactly the noise is not worse.
        let exactly = Reading {
            median_net: 148.0,
            se_median: 2.0,
            wilson_lower: 0.99,
            ..held
        };
        assert!(
            prefer(&exactly, 1, Some(&(held_stratum, held))),
            "exactly one standard error worse has not been shown to be worse, \
             so the Wilson bound decides and the candidate is surer"
        );
    }

    #[test]
    fn a_less_sure_candidate_loses_even_when_it_is_simpler() {
        // The fourth comparison. Simplicity breaks a tie on the bound; it does
        // not overrule one. A one-term stratum whose rows paid less often than
        // a three-term stratum's is not the better statement.
        let held_stratum = Stratum::named(
            "held",
            vec![
                Term {
                    feature: 0,
                    at_least: true,
                    threshold: 1.0,
                },
                Term {
                    feature: 1,
                    at_least: true,
                    threshold: 1.0,
                },
                Term {
                    feature: 2,
                    at_least: true,
                    threshold: 1.0,
                },
            ],
        );
        let held = Reading {
            n: 400,
            median_gross: 1_000.0,
            median_net: 750.0,
            positive: 300,
            wilson_lower: 0.70,
            se_median: 40.0,
        };
        let simpler_but_less_sure = Reading {
            wilson_lower: 0.60,
            ..held
        };
        assert!(!prefer(
            &simpler_but_less_sure,
            1,
            Some(&(held_stratum, held))
        ));
    }

    #[test]
    fn the_intersection_is_an_and_and_counts_what_survives_it() {
        // A conjunction is this and nothing else. An `or` here would make every
        // three-term stratum wider than its terms, and a reading over the wrong
        // rows is indistinguishable from a reading over the right ones.
        let (bits, count) = intersect(&[0b1011, 0b1111], &[0b0011, 0b0101]);
        assert_eq!(bits, vec![0b0011, 0b0101]);
        assert_eq!(count, 4);

        let (empty, none) = intersect(&[0b1010], &[0b0101]);
        assert_eq!(empty, vec![0]);
        assert_eq!(none, 0);
    }

    #[test]
    fn a_reading_is_the_median_the_cost_and_the_share_that_paid() {
        // Every figure in a reading, pinned against arithmetic done by hand.
        // Nine rows, gross 100..900, charged 250: the median is the fifth, 500,
        // the net is 250, and the rows that paid are the four above 250.
        let round_trip = 250.0;
        let points: Vec<Point> = (1..=9u32)
            .map(|i| point(u64::from(i), Vec::new(), f64::from(i) * 100.0, round_trip))
            .collect();
        let matched: Vec<usize> = (0..9).collect();

        let reading = summarise(&points, &matched, round_trip).expect("nine rows");
        assert_eq!(reading.n, 9);
        assert!((reading.median_gross - 500.0).abs() < f64::EPSILON);
        assert!(
            (reading.median_net - 250.0).abs() < f64::EPSILON,
            "the net is the median less the round trip, not plus it"
        );
        assert_eq!(
            reading.positive, 7,
            "300 through 900 pay; 200 does not, and 250 exactly would not either"
        );
        // 1.2533 * (IQR / 1.349) / sqrt(9), with the quartiles at 300 and 700.
        let expected = 1.253_3 * ((700.0 - 300.0) / 1.349) / 3.0;
        assert!(
            (reading.se_median - expected).abs() < 1e-9,
            "{} against {expected}",
            reading.se_median
        );
        assert!(
            summarise(&points, &[], round_trip).is_none(),
            "no rows, no reading"
        );
    }

    #[test]
    fn a_row_that_exactly_pays_its_costs_has_not_paid() {
        // The boundary of "positive". A net of exactly zero is a round trip
        // that consumed the whole move, and counting it as a win would put the
        // point mass at zero on the winning side -- which is the failure
        // research 0017 found in its short-hold strata.
        let round_trip = 250.0;
        let points = vec![
            point(1, Vec::new(), 250.0, round_trip),
            point(2, Vec::new(), 251.0, round_trip),
        ];
        let reading = summarise(&points, &[0, 1], round_trip).expect("two rows");
        assert_eq!(reading.positive, 1);
    }

    #[test]
    fn the_net_a_point_carries_is_its_gross_less_the_round_trip() {
        let table = table_of(10, |_| 1_000.0);
        let points = points_of(
            &table,
            &Options {
                noise_seed: None,
                ..Options::default()
            },
            250.0,
        );
        assert_eq!(points.len(), 10);
        assert!((points[0].gross - 1_000.0).abs() < f64::EPSILON);
        assert!((points[0].net - 750.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_planted_noise_seed_adds_exactly_one_column_and_nothing_else_does() {
        let table = table_of(10, |_| 1_000.0);
        let without = points_of(&table, &Options::default(), 0.0);
        let with = points_of(
            &table,
            &Options {
                noise_seed: Some(3),
                ..Options::default()
            },
            0.0,
        );
        assert_eq!(without[0].values.len(), FEATURES.len());
        assert_eq!(with[0].values.len(), FEATURES.len() + 1);
        assert!(with[0].values[FEATURES.len()].is_some());
    }

    #[test]
    fn a_stratum_reads_the_planted_column_and_not_a_feature_beside_it() {
        // The term index past the end of FEATURES is the planted noise, and it
        // has to reach the noise value rather than fall off the row.
        let mut values = vec![None; FEATURES.len()];
        values[0] = Some(1.0);
        let row = Row {
            mint: Address::new([9; 32]),
            creator: Address::new([1; 32]),
            launch_slot: Slot(1),
            t: Slot(6_001),
            values,
            gross_6h_bps: None,
            gross_24h_bps: None,
            mode: None,
        };
        let planted = Stratum::named(
            "noise",
            vec![Term {
                feature: FEATURES.len(),
                at_least: true,
                threshold: 0.5,
            }],
        );
        assert!(planted.admits(&row, Some(0.9)));
        assert!(!planted.admits(&row, Some(0.1)));
        assert!(
            !planted.admits(&row, None),
            "no noise value means the row is not in the stratum"
        );
        assert_eq!(planted.terms[0].feature_name(), NOISE_FEATURE);
    }

    #[test]
    fn the_windows_are_contiguous_equal_slices_in_launch_order() {
        // Off by one in either edge shifts every fold boundary, which shifts
        // the purge and the embargo with it.
        let points: Vec<Point> = (0..500u64)
            .map(|i| point(i * 10, Vec::new(), 0.0, 0.0))
            .collect();
        let windows = split(&points);

        assert_eq!(
            windows,
            vec![
                (Slot(0), Slot(990)),
                (Slot(1_000), Slot(1_990)),
                (Slot(2_000), Slot(2_990)),
                (Slot(3_000), Slot(3_990)),
                (Slot(4_000), Slot(4_990)),
            ]
        );
    }

    #[test]
    fn a_feature_with_exactly_the_floors_worth_of_values_still_produces_terms() {
        // The boundary of the grammar's own floor. A feature measured for
        // exactly MIN_ROWS rows can produce a stratum that clears MIN_ROWS, so
        // excluding it would drop a testable statement.
        let mut points: Vec<Point> = (0..MIN_ROWS as u64)
            .map(|i| {
                #[expect(clippy::cast_precision_loss, reason = "a small index")]
                let v = i as f64;
                point(i, vec![Some(v)], 0.0, 0.0)
            })
            .collect();
        assert!(
            !grammar(&points, &(0..points.len()).collect::<Vec<_>>(), 1).is_empty(),
            "exactly the floor is enough"
        );

        points.pop();
        assert!(
            grammar(&points, &(0..points.len()).collect::<Vec<_>>(), 1).is_empty(),
            "one below the floor is not"
        );
    }

    #[test]
    fn two_thresholds_that_are_merely_close_are_both_kept() {
        // The dedupe is for a constant feature, whose nine deciles are the same
        // number. Two genuinely different thresholds a hair apart are two
        // different statements, and collapsing them would silently shrink the
        // grammar.
        let points: Vec<Point> = (0..200u64)
            .map(|i| {
                let v = if i < 100 { 1.0 } else { 1.000_1 };
                point(i, vec![Some(v)], 0.0, 0.0)
            })
            .collect();
        let terms = grammar(&points, &(0..points.len()).collect::<Vec<_>>(), 1);
        let thresholds: BTreeSet<u64> = terms.iter().map(|t| t.threshold.to_bits()).collect();
        assert_eq!(thresholds.len(), 2, "{terms:?}");
    }

    #[test]
    fn the_search_enumerates_each_conjunction_once_and_never_two_terms_on_one_feature() {
        // Three terms on three features. Singles are three; pairs are the three
        // ordered by index; and exactly one triple, because a third term is
        // taken from past the second rather than past the first. Enumerating a
        // triple twice would double both the work and the count a reader uses
        // to discount the winner.
        let points: Vec<Point> = (0..300u64)
            .map(|i| {
                #[expect(clippy::cast_precision_loss, reason = "a small index")]
                let v = i as f64;
                point(i, vec![Some(v), Some(v), Some(v)], v, 0.0)
            })
            .collect();
        let rows: Vec<usize> = (0..points.len()).collect();
        let terms = vec![
            Term {
                feature: 0,
                at_least: true,
                threshold: 0.0,
            },
            Term {
                feature: 1,
                at_least: true,
                threshold: 0.0,
            },
            Term {
                feature: 2,
                at_least: true,
                threshold: 0.0,
            },
        ];

        let (best, tried, enumeration) = search(&points, &rows, &terms, 0.0, MIN_ROWS, 1_000);
        assert_eq!(tried, 7, "three singles, three pairs, one triple");
        assert_eq!(enumeration, Enumeration::Exhaustive);
        let (stratum, _) = best.expect("something holds three hundred rows");
        let features: BTreeSet<usize> = stratum.terms.iter().map(|t| t.feature).collect();
        assert_eq!(
            features.len(),
            stratum.terms.len(),
            "no two terms may sit on one feature: {stratum:?}"
        );
    }

    #[test]
    fn the_floor_is_applied_to_every_width_of_conjunction() {
        // A floor that only reached singles would let a pair or a triple below
        // it become the winner, and a winner that cannot be tested displaces
        // one that can.
        let points: Vec<Point> = (0..300u64)
            .map(|i| {
                #[expect(clippy::cast_precision_loss, reason = "a small index")]
                let v = i as f64;
                point(i, vec![Some(v), Some(v)], v, 0.0)
            })
            .collect();
        let rows: Vec<usize> = (0..points.len()).collect();
        // Each term alone holds 150 rows; together they hold 150 as well, since
        // the two features are the same values.
        let terms = vec![
            Term {
                feature: 0,
                at_least: true,
                threshold: 150.0,
            },
            Term {
                feature: 1,
                at_least: true,
                threshold: 150.0,
            },
        ];

        let (best, _, _) = search(&points, &rows, &terms, 0.0, 200, 1_000);
        assert!(best.is_none(), "nothing holds two hundred rows: {best:?}");

        let (found, _, _) = search(&points, &rows, &terms, 0.0, 150, 1_000);
        assert!(found.is_some(), "exactly the floor is enough");
    }

    #[test]
    fn the_budget_stops_the_search_and_says_so() {
        let points: Vec<Point> = (0..300u64)
            .map(|i| {
                #[expect(clippy::cast_precision_loss, reason = "a small index")]
                let v = i as f64;
                point(i, vec![Some(v), Some(v), Some(v)], v, 0.0)
            })
            .collect();
        let rows: Vec<usize> = (0..points.len()).collect();
        let terms = vec![
            Term {
                feature: 0,
                at_least: true,
                threshold: 0.0,
            },
            Term {
                feature: 1,
                at_least: true,
                threshold: 0.0,
            },
            Term {
                feature: 2,
                at_least: true,
                threshold: 0.0,
            },
        ];

        let (_, tried, enumeration) = search(&points, &rows, &terms, 0.0, MIN_ROWS, 2);
        assert_eq!(enumeration, Enumeration::StoppedAtBudget);
        assert!(tried <= 3, "the budget bounds the count: {tried}");
    }

    #[test]
    fn a_gain_of_exactly_one_standard_error_is_not_a_gain() {
        // The boundary of the rule. Strictly greater, in both directions: a
        // candidate exactly one standard error better has not been shown to be
        // better, and neither has the incumbent.
        let simple = Stratum::named(
            "simple",
            vec![Term {
                feature: 0,
                at_least: true,
                threshold: 1.0,
            }],
        );
        let held = Reading {
            n: 400,
            median_gross: 1_000.0,
            median_net: 750.0,
            positive: 300,
            wilson_lower: 0.7,
            se_median: 40.0,
        };

        let exactly = Reading {
            median_net: 790.0,
            se_median: 40.0,
            ..held
        };
        assert!(
            !prefer(&exactly, 1, Some(&(simple.clone(), held))),
            "exactly one standard error better is not better"
        );
        let past = Reading {
            median_net: 790.1,
            ..exactly
        };
        assert!(prefer(&past, 1, Some(&(simple.clone(), held))));

        // The noise is the larger of the two, so a noisy candidate cannot
        // displace a quiet incumbent on a small apparent gain.
        let noisy = Reading {
            median_net: 800.0,
            se_median: 200.0,
            ..held
        };
        assert!(
            !prefer(&noisy, 1, Some(&(simple.clone(), held))),
            "fifty basis points inside the candidate's own two hundred is noise"
        );
    }

    #[test]
    fn an_equal_bound_falls_through_to_the_simpler_stratum_and_no_further() {
        let three = Stratum::named(
            "three",
            vec![
                Term {
                    feature: 0,
                    at_least: true,
                    threshold: 1.0,
                },
                Term {
                    feature: 1,
                    at_least: true,
                    threshold: 1.0,
                },
                Term {
                    feature: 2,
                    at_least: true,
                    threshold: 1.0,
                },
            ],
        );
        let held = Reading {
            n: 400,
            median_gross: 1_000.0,
            median_net: 750.0,
            positive: 300,
            wilson_lower: 0.7,
            se_median: 40.0,
        };
        assert!(
            prefer(&held, 1, Some(&(three.clone(), held))),
            "one term beats three"
        );
        assert!(
            !prefer(&held, 3, Some(&(three.clone(), held))),
            "three against three is not an improvement"
        );
        assert!(!prefer(&held, 4, Some(&(three, held))), "and four is worse");
    }

    #[test]
    fn exactly_enough_rows_runs_and_one_fewer_is_refused() {
        // The boundary of the only refusal that is about the table rather than
        // about the snapshot.
        let rates = rates();
        let table = table_of(FOLDS * MIN_ROWS, |i| {
            #[expect(clippy::cast_precision_loss, reason = "a small index")]
            let v = (i % 7) as f64 * 100.0;
            v
        });
        assert!(run(&table, &rates, &Options::default()).is_ok());

        let mut short = table;
        short.rows.pop();
        assert!(matches!(
            run(&short, &rates, &Options::default()),
            Err(EdgeError::TooFewRows { .. })
        ));
    }

    #[test]
    fn every_window_reports_its_own_rows_and_the_test_folds_are_the_last_two() {
        // A fold table that piled the fitting period's count onto one window
        // would make the purge look as though it had emptied the other two.
        let rates = rates();
        let table = table_of(FOLDS * MIN_ROWS * 4, |i| {
            #[expect(clippy::cast_precision_loss, reason = "a small index")]
            let v = (i % 7) as f64 * 100.0;
            v
        });
        let report = run(&table, &rates, &Options::default()).expect("runs");

        assert_eq!(report.folds.len(), FOLDS);
        for (index, fold) in report.folds.iter().enumerate() {
            assert!(
                fold.rows > 0,
                "window {index} reported no rows: {:?}",
                report.folds
            );
            assert!(fold.from <= fold.to);
        }
        let fitted: usize = report.folds[..FIT_FOLDS].iter().map(|f| f.rows).sum();
        let tested: usize = report.folds[FIT_FOLDS..].iter().map(|f| f.rows).sum();
        assert!(
            fitted > tested,
            "three windows fit and two test, so the fitting period is the larger"
        );
    }

    fn row(mint: u8, slot: u64, feature: usize, value: f64, gross: f64) -> Row {
        let mut values = vec![None; FEATURES.len()];
        values[feature] = Some(value);
        Row {
            mint: Address::new([mint; 32]),
            creator: Address::new([1; 32]),
            launch_slot: Slot(slot),
            t: Slot(slot + 6_000),
            values,
            gross_6h_bps: Some(gross),
            gross_24h_bps: Some(gross),
            mode: None,
        }
    }

    #[test]
    fn a_term_admits_nothing_when_the_value_is_absent() {
        // Absent is not small. A stratum naming a feature nobody measured for
        // this row must drop the row, and reading `None` as zero would sweep
        // every unmeasured token into the "below the threshold" side.
        let below = Term {
            feature: 0,
            at_least: false,
            threshold: 10.0,
        };
        assert!(below.admits(Some(9.0)));
        assert!(!below.admits(Some(10.0)), "the boundary belongs to >=");
        assert!(!below.admits(None));

        let at_least = Term {
            feature: 0,
            at_least: true,
            threshold: 10.0,
        };
        assert!(at_least.admits(Some(10.0)));
        assert!(!at_least.admits(Some(9.999)));
        assert!(!at_least.admits(None));
    }

    #[test]
    fn a_reading_needs_every_condition_and_not_merely_a_good_median() {
        let clearing = Reading {
            n: MIN_ROWS,
            median_gross: 900.0,
            median_net: 10.0,
            positive: MIN_ROWS,
            wilson_lower: 0.96,
            se_median: 5.0,
        };
        assert!(clearing.clears());

        // Each condition, removed alone.
        assert!(
            !Reading {
                n: MIN_ROWS - 1,
                ..clearing
            }
            .clears(),
            "too few rows"
        );
        assert!(
            !Reading {
                median_net: -1.0,
                ..clearing
            }
            .clears(),
            "the net must pay for the round trip"
        );
        assert!(
            !Reading {
                median_net: 5.0,
                ..clearing
            }
            .clears(),
            "a net of exactly one standard error is inside its own noise"
        );
        assert!(
            !Reading {
                wilson_lower: 0.5,
                ..clearing
            }
            .clears(),
            "a half is not above a half: a median over a point mass at zero is a report about the point mass"
        );
        assert!(
            Reading {
                median_gross: 1.0,
                ..clearing
            }
            .clears(),
            "there is no separate bar on the gross figure -- 456 is the round              trip read in another band, and charging it here would charge one              measurement twice"
        );
    }

    #[test]
    fn a_quantile_is_a_value_a_row_actually_held_and_does_not_need_sorting() {
        let mut unsorted = [3.0, 1.0, 4.0, 2.0, 5.0];
        assert!((quantile(&mut unsorted, 0.5) - 3.0).abs() < f64::EPSILON);
        let mut again = [3.0, 1.0, 4.0, 2.0, 5.0];
        assert!(
            (quantile(&mut again, 0.25) - 2.0).abs() < f64::EPSILON,
            "an order statistic, never an interpolation between two rows"
        );
        let mut one = [7.0];
        assert!((quantile(&mut one, 0.5) - 7.0).abs() < f64::EPSILON);
        let mut top = [1.0, 2.0, 3.0];
        assert!(
            (quantile(&mut top, 1.0) - 3.0).abs() < f64::EPSILON,
            "the top must clamp rather than index one past the end"
        );
    }

    #[test]
    fn the_fitting_floor_is_scaled_from_the_smallest_test_fold() {
        // A stratum holding a hundred fitting rows out of sixteen hundred holds
        // about twenty-five in a four-hundred-row test fold, and can never be
        // accepted. Fitting on it displaces strata that could have been.
        assert_eq!(
            fitting_floor(1_600, &[vec![0; 400], vec![0; 400]]),
            480,
            "a stratum must hold the share that leaves TESTABLE_ROWS in a test fold"
        );
        assert_eq!(
            fitting_floor(1_600, &[vec![0; 400], vec![0; 200]]),
            960,
            "the smallest test fold sets it, not the average"
        );
        assert_eq!(
            fitting_floor(100, &[vec![0; 1_000]]),
            MIN_ROWS,
            "the acceptance floor is the lower bound; a scaled floor below it is not a floor"
        );
        assert_eq!(
            fitting_floor(1_600, &[Vec::new()]),
            MIN_ROWS,
            "an empty test fold cannot scale anything, and dividing by it must not panic"
        );
    }

    #[test]
    fn a_refinement_inside_one_standard_error_does_not_displace_a_simpler_stratum() {
        // The rule that made `an_engineered_edge_is_found` pass. A three-term
        // stratum that looks 5 bps better than a one-term stratum whose median
        // carries a standard error of 40 has not been shown to be better, and
        // preferring it is how a real edge gets replaced by noise fitted around
        // it -- and then dies on the test folds, taking the edge with it.
        let simple = Stratum::named(
            "simple",
            vec![Term {
                feature: 0,
                at_least: true,
                threshold: 1.0,
            }],
        );
        let held = Reading {
            n: 400,
            median_gross: 1_000.0,
            median_net: 750.0,
            positive: 300,
            wilson_lower: 0.7,
            se_median: 40.0,
        };

        let inside = Reading {
            median_net: 755.0,
            n: 120,
            ..held
        };
        assert!(
            !prefer(&inside, 3, Some(&(simple.clone(), held))),
            "five basis points inside a forty-point standard error is not better"
        );

        let outside = Reading {
            median_net: 900.0,
            n: 120,
            ..held
        };
        assert!(
            prefer(&outside, 3, Some(&(simple.clone(), held))),
            "a gain larger than the standard error is a real one"
        );

        // Inside the band the second acceptance condition decides: the stratum
        // whose rows more reliably paid wins, however wide or narrow it is.
        let surer = Reading {
            wilson_lower: 0.95,
            ..held
        };
        assert!(prefer(&surer, 3, Some(&(simple.clone(), held))));
        let less_sure = Reading {
            wilson_lower: 0.55,
            ..held
        };
        assert!(!prefer(&less_sure, 1, Some(&(simple.clone(), held))));

        // Only a dead-level tie falls through to simplicity.
        let level = Reading { n: 500, ..held };
        assert!(
            prefer(
                &level,
                1,
                Some(&(
                    Stratum::named(
                        "three",
                        vec![
                            Term {
                                feature: 0,
                                at_least: true,
                                threshold: 1.0
                            },
                            Term {
                                feature: 1,
                                at_least: true,
                                threshold: 1.0
                            },
                            Term {
                                feature: 2,
                                at_least: true,
                                threshold: 1.0
                            },
                        ]
                    ),
                    held
                ))
            ),
            "on a dead-level tie, fewer terms wins"
        );
        assert!(prefer(&held, 1, None), "anything beats nothing held");
    }

    #[test]
    fn the_windows_are_equal_in_rows_and_in_launch_order() {
        let points: Vec<Point> = (0..500u64)
            .map(|i| Point {
                launch_slot: Slot(i * 10),
                gross: 0.0,
                net: 0.0,
                values: Vec::new(),
            })
            .collect();
        let windows = split(&points);

        assert_eq!(windows.len(), FOLDS);
        assert_eq!(windows[0].0, Slot(0));
        assert_eq!(windows[FOLDS - 1].1, Slot(4_990));
        for pair in windows.windows(2) {
            assert!(pair[0].1 < pair[1].0, "windows must not overlap: {pair:?}");
        }
    }

    #[test]
    fn the_purge_drops_a_fitting_row_whose_label_matures_after_the_boundary() {
        // The row launched inside the fitting period, but its twenty-four-hour
        // return was not known until after it. Keeping it fits on something the
        // boundary could not have known.
        let maturity = Horizon::TwentyFourHours.maturity_slots();
        let points: Vec<Point> = (0..500u64)
            .map(|i| Point {
                launch_slot: Slot(i * maturity / 100),
                gross: 0.0,
                net: 0.0,
                values: Vec::new(),
            })
            .collect();
        let windows = split(&points);
        let (fit, tests) = admissible(&points, &windows, Horizon::TwentyFourHours);

        let boundary = windows[FIT_FOLDS - 1].1;
        assert!(
            fit.iter()
                .all(|i| points[*i].launch_slot.get() + maturity <= boundary.get()),
            "every fitting row's label matured before the boundary"
        );
        assert!(
            fit.len() < 300,
            "the purge must remove rows near the boundary, and it removed {}",
            300 - fit.len()
        );
        assert_eq!(tests.len(), FOLDS - FIT_FOLDS);
    }

    #[test]
    fn the_embargo_holds_a_test_fold_off_the_boundary_before_it() {
        let points: Vec<Point> = (0..500u64)
            .map(|i| Point {
                launch_slot: Slot(i * EMBARGO_SLOTS / 10),
                gross: 0.0,
                net: 0.0,
                values: Vec::new(),
            })
            .collect();
        let windows = split(&points);
        let (_, tests) = admissible(&points, &windows, Horizon::SixHours);

        for (offset, rows) in tests.iter().enumerate() {
            let previous = windows[FIT_FOLDS + offset - 1].1;
            assert!(
                rows.iter()
                    .all(|i| points[*i].launch_slot.get() >= previous.get() + EMBARGO_SLOTS),
                "fold {offset} holds a row inside the embargo"
            );
        }
    }

    #[test]
    fn the_grammar_ignores_a_feature_measured_for_too_few_rows() {
        // A feature with eleven values cannot produce a stratum holding a
        // hundred, so it produces no terms rather than terms that always fail.
        let mut points: Vec<Point> = (0..200u64)
            .map(|i| Point {
                launch_slot: Slot(i),
                gross: 0.0,
                net: 0.0,
                values: vec![None; FEATURES.len()],
            })
            .collect();
        for point in points.iter_mut().take(11) {
            point.values[0] = Some(1.0);
        }
        for (i, point) in points.iter_mut().enumerate() {
            #[expect(clippy::cast_precision_loss, reason = "small counts")]
            let value = i as f64;
            point.values[1] = Some(value);
        }
        let rows: Vec<usize> = (0..points.len()).collect();
        let terms = grammar(&points, &rows, FEATURES.len());

        assert!(
            terms.iter().all(|t| t.feature != 0),
            "a feature below the row floor produces no terms"
        );
        assert!(
            terms.iter().any(|t| t.feature == 1),
            "a feature with enough values produces some"
        );
    }

    #[test]
    fn a_constant_feature_produces_one_threshold_not_nine() {
        // Nine identical deciles are nine copies of one term: nine times the
        // search for one answer, and nine entries in the count a reader uses to
        // discount the result.
        let points: Vec<Point> = (0..200u64)
            .map(|i| Point {
                launch_slot: Slot(i),
                gross: 0.0,
                net: 0.0,
                values: vec![Some(4.0)],
            })
            .collect();
        let rows: Vec<usize> = (0..points.len()).collect();
        let terms = grammar(&points, &rows, 1);

        assert_eq!(
            terms.len(),
            2,
            "one threshold, in two directions: {terms:?}"
        );
    }

    #[test]
    fn noise_is_uniform_seeded_and_reproducible() {
        let a = row(1, 10, 0, 1.0, 0.0);
        let b = row(2, 20, 0, 1.0, 0.0);

        assert!((noise_for(7, &a) - noise_for(7, &a)).abs() < f64::EPSILON);
        assert!(
            (noise_for(7, &a) - noise_for(8, &a)).abs() > f64::EPSILON,
            "the seed must change the value, or ten seeds are one run"
        );
        assert!(
            (noise_for(7, &a) - noise_for(7, &b)).abs() > f64::EPSILON,
            "two rows must differ, or the feature is a constant"
        );
        for seed in 0..50u64 {
            let value = noise_for(seed, &a);
            assert!((0.0..1.0).contains(&value), "{value} is outside [0, 1)");
        }
    }

    #[test]
    fn a_stratum_describes_itself_in_the_grammars_own_words() {
        let stratum = Stratum::named(
            "x",
            vec![Term {
                feature: 0,
                at_least: true,
                threshold: 3.0,
            }],
        );
        assert_eq!(stratum.describe(), "launch_traders >= 3.0000");
        assert_eq!(Stratum::named("y", Vec::new()).describe(), "every row");
    }
}
