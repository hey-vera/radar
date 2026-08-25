// SPDX-License-Identifier: Apache-2.0
//! The event study: does a creator's record predict their next launch?
//!
//! This is the question the whole system exists to answer, and until it is
//! answered every signal in the repository is a hypothesis with good manners.
//! `creator_edge` gates on organic graduation history because that is the
//! *plausible* rule, not because anyone has measured that it works.
//!
//! # How a study avoids answering itself
//!
//! The trap is trivially easy to fall into: compute a creator's graduation rate
//! over all their launches, then check whether creators with high rates have
//! launches that graduated. That correlates perfectly and means nothing, because
//! the same events are on both sides.
//!
//! So the study splits at a **pivot slot** and never lets the two halves touch:
//!
//! - The **prior** is what was knowable at the pivot — launches at or before it,
//!   scored only by outcomes *measured* at or before it. A launch that had
//!   happened but had not yet been measured contributes nothing, because at the
//!   pivot nobody knew.
//! - The **outcome** is what those creators' *later* launches did.
//!
//! A creator with no launches on one side of the pivot is not in the study at
//! all. That is most of them, and it is the price of asking the question
//! honestly.
//!
//! # What it refuses to say
//!
//! Rates over tiny samples are not reported, because a percentage from three
//! launches reads exactly like one from three hundred once it is a number. The
//! population base rate is always printed beside every group rate, since a group
//! rate without it is a number with no scale.

use std::collections::BTreeMap;

use radar_asof::AsOf;
use radar_store::{Event, GraduationMode, Outcome, Reader, StoreError, Table};
use radar_types::{Address, Slot};
use serde::Serialize;

/// Launches a creator must have before the pivot to be included.
///
/// The same floor `creator_track_record` uses. Below it a creator's "rate" is
/// arithmetic on noise, and including them would let a hundred one-launch
/// creators outvote the handful with a real record.
pub const MIN_PRIOR_LAUNCHES: u64 = 5;

/// Group rates below this many creators are withheld rather than printed.
pub const MIN_GROUP: usize = 5;

/// One creator, split across the pivot.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
pub struct Split {
    /// Launches at or before the pivot.
    pub prior_launches: u64,
    /// Of those, ones with any outcome measured at or before the pivot.
    pub prior_measured: u64,
    /// Of those, ones an outcome measured **at or before the pivot** called
    /// organic. This is the only graduation count the prior may use.
    pub prior_organic: u64,
    /// Launches after the pivot.
    pub later_launches: u64,
    /// Of those, ones that went on to graduate organically.
    pub later_organic: u64,
}

/// What the study found.
#[derive(Clone, Debug, Serialize)]
pub struct Study {
    /// The slot the record was split at.
    pub pivot: Slot,
    /// The watermark outcomes were read at.
    pub head: Slot,
    /// Creators with enough launches on both sides to say anything about.
    pub creators: usize,
    /// Their launches at or before the pivot.
    pub prior_launches: u64,
    /// How many of those had an outcome **measured** by the pivot.
    ///
    /// The study's power comes entirely from this number. A prior built from
    /// launches nobody had measured yet is not a weak prior, it is no prior —
    /// and without this reported, a store whose outcome pass had not yet run
    /// produces a table where every creator sits in "no organic graduation
    /// known", which reads exactly like the finding that creator history
    /// predicts nothing. It is not that finding. It is an empty column.
    pub prior_measured: u64,
    /// Later launches across every included creator.
    pub later_launches: u64,
    /// How many of those graduated organically.
    pub later_organic: u64,
    /// Creators grouped by what was known about them at the pivot.
    pub groups: Vec<Group>,
}

/// One band of prior record, and what its creators did next.
#[derive(Clone, Debug, Serialize)]
pub struct Group {
    /// What this band means, for a person reading the table.
    pub label: &'static str,
    /// Creators in it.
    pub creators: usize,
    /// Their launches after the pivot.
    pub later_launches: u64,
    /// How many of those graduated organically.
    pub later_organic: u64,
}

/// z for a 95% two-sided interval.
const Z_95: f64 = 1.96;

impl Group {
    /// A 95% Wilson score interval on the later organic rate, in basis points.
    ///
    /// Wilson rather than the textbook normal interval, because these are rare
    /// events over modest samples — 22 graduations in 1,279 launches — and the
    /// normal approximation misbehaves badly there, happily producing bounds
    /// below zero. Wilson stays inside [0, 1] and is well behaved at small
    /// counts.
    ///
    /// The interval is the point of reporting the rate at all. Two groups whose
    /// intervals overlap have not been shown to differ however far apart their
    /// midpoints look, and a table of bare percentages invites exactly that
    /// mistake.
    ///
    /// `None` under the same conditions as [`later_organic_bps`](Self::later_organic_bps).
    #[must_use]
    pub fn later_organic_ci_bps(&self) -> Option<(u64, u64)> {
        if self.creators < MIN_GROUP || self.later_launches == 0 {
            return None;
        }
        #[expect(
            clippy::cast_precision_loss,
            reason = "counts here are far below f64's exact-integer range"
        )]
        let (hits, n) = (self.later_organic as f64, self.later_launches as f64);
        let p = hits / n;
        let z2 = Z_95 * Z_95;
        let denom = 1.0 + z2 / n;
        let centre = (p + z2 / (2.0 * n)) / denom;
        let half = (Z_95 / denom) * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt();

        let to_bps = |v: f64| {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "clamped to a proportion before scaling"
            )]
            let out = (v.clamp(0.0, 1.0) * 10_000.0).round() as u64;
            out
        };
        Some((to_bps(centre - half), to_bps(centre + half)))
    }

    /// Whether this group's interval sits entirely above another's.
    ///
    /// The only comparison the study is entitled to make from two rates.
    #[must_use]
    pub fn clearly_above(&self, other: &Self) -> bool {
        match (self.later_organic_ci_bps(), other.later_organic_ci_bps()) {
            (Some((lo, _)), Some((_, hi))) => lo > hi,
            _ => false,
        }
    }

    /// Organic graduations per ten thousand later launches.
    ///
    /// `None` below [`MIN_GROUP`] creators, or with no later launches at all.
    /// Absent rather than zero: a band nobody is in has no rate.
    #[must_use]
    pub const fn later_organic_bps(&self) -> Option<u64> {
        if self.creators < MIN_GROUP || self.later_launches == 0 {
            return None;
        }
        Some(self.later_organic.saturating_mul(10_000) / self.later_launches)
    }
}

impl Study {
    /// Whether anything was known about these creators at the pivot at all.
    ///
    /// False means the table below is describing an absence of measurement
    /// rather than an absence of signal, and nothing in it may be read as
    /// evidence about creators.
    #[must_use]
    pub const fn prior_is_informative(&self) -> bool {
        self.prior_measured > 0
    }

    /// The population rate across every later launch in the study.
    ///
    /// The number every group rate has to be read against. A group at 300 bps
    /// means nothing until you know whether the population is at 30 or at 3,000.
    #[must_use]
    pub const fn base_rate_bps(&self) -> Option<u64> {
        if self.later_launches == 0 {
            return None;
        }
        Some(self.later_organic.saturating_mul(10_000) / self.later_launches)
    }
}

/// Runs the study over a store, splitting at `pivot`.
///
/// # Errors
///
/// Returns [`StoreError`] if the store cannot be read.
pub fn run(reader: &Reader, pivot: Slot, head: Slot) -> Result<Study, StoreError> {
    let at_head = AsOf::at(head);

    // Every launch the store knows about, with its creator.
    let mut launches: BTreeMap<Address, (Address, Slot)> = BTreeMap::new();
    for event in reader.read(Table::Launches, at_head)? {
        if let Event::Launch(l) = event {
            launches.insert(l.mint, (l.creator, l.envelope.slot));
        }
    }

    // Two views of the outcomes, and keeping them apart is the whole method.
    // `known_at_pivot` may only see measurements taken at or before the pivot;
    // `known_now` sees everything.
    let at_pivot_rows = reader.read_outcomes(AsOf::at(pivot))?;
    let now_rows = reader.read_outcomes(at_head)?;
    let known_at_pivot = latest_by_mint(&at_pivot_rows);
    let known_now = latest_by_mint(&now_rows);

    let mut splits: BTreeMap<Address, Split> = BTreeMap::new();
    for (mint, (creator, slot)) in &launches {
        let split = splits.entry(*creator).or_insert(Split {
            prior_launches: 0,
            prior_measured: 0,
            prior_organic: 0,
            later_launches: 0,
            later_organic: 0,
        });

        if *slot <= pivot {
            split.prior_launches += 1;
            if known_at_pivot.contains_key(mint) {
                split.prior_measured += 1;
            }
            if is_organic(known_at_pivot.get(mint)) {
                split.prior_organic += 1;
            }
        } else {
            split.later_launches += 1;
            if is_organic(known_now.get(mint)) {
                split.later_organic += 1;
            }
        }
    }

    // A creator has to exist on both sides to be evidence of anything: no prior
    // means nothing was known, and no later launches means nothing to predict.
    splits.retain(|_, s| s.prior_launches >= MIN_PRIOR_LAUNCHES && s.later_launches > 0);

    Ok(summarise(pivot, head, &splits))
}

/// A band's name and the test for belonging to it.
type Band = (&'static str, fn(&Split) -> bool);

/// Buckets the splits and totals them.
fn summarise(pivot: Slot, head: Slot, splits: &BTreeMap<Address, Split>) -> Study {
    // Deliberately coarse. Finer buckets over this sample would be reading tea
    // leaves, and "any organic graduation at all" is the rule `creator_edge`
    // actually applies, so it is the one worth testing first.
    let bands: [Band; 3] = [
        ("no organic graduation known", |s| s.prior_organic == 0),
        ("exactly one", |s| s.prior_organic == 1),
        ("two or more", |s| s.prior_organic >= 2),
    ];

    let groups = bands
        .iter()
        .map(|(label, belongs)| {
            let members: Vec<&Split> = splits.values().filter(|s| belongs(s)).collect();
            Group {
                label,
                creators: members.len(),
                later_launches: members.iter().map(|s| s.later_launches).sum(),
                later_organic: members.iter().map(|s| s.later_organic).sum(),
            }
        })
        .collect::<Vec<_>>();

    Study {
        pivot,
        head,
        creators: splits.len(),
        prior_launches: splits.values().map(|s| s.prior_launches).sum(),
        prior_measured: splits.values().map(|s| s.prior_measured).sum(),
        later_launches: splits.values().map(|s| s.later_launches).sum(),
        later_organic: splits.values().map(|s| s.later_organic).sum(),
        groups,
    }
}

/// The latest measurement per mint, which is what was known at that watermark.
fn latest_by_mint(outcomes: &[Outcome]) -> BTreeMap<Address, &Outcome> {
    let mut latest: BTreeMap<Address, &Outcome> = BTreeMap::new();
    for outcome in outcomes {
        latest
            .entry(outcome.mint)
            .and_modify(|held| {
                if outcome.measured_at > held.measured_at {
                    *held = outcome;
                }
            })
            .or_insert(outcome);
    }
    latest
}

/// Whether a measurement, if there is one, says the token graduated organically.
///
/// An unmeasured launch is not organic — but it is not counted as a failure
/// either, it simply does not contribute. Absent is not zero.
fn is_organic(outcome: Option<&&Outcome>) -> bool {
    outcome.is_some_and(|o| o.graduation_mode() == Some(GraduationMode::Organic))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_group_below_the_floor_reports_no_rate() {
        // Four creators graduating everything looks like a 100% signal and is
        // four creators.
        let group = Group {
            label: "x",
            creators: MIN_GROUP - 1,
            later_launches: 40,
            later_organic: 40,
        };
        assert_eq!(group.later_organic_bps(), None);
    }

    #[test]
    fn a_group_at_the_floor_reports_one() {
        let group = Group {
            label: "x",
            creators: MIN_GROUP,
            later_launches: 40,
            later_organic: 4,
        };
        assert_eq!(group.later_organic_bps(), Some(1_000));
    }

    #[test]
    fn a_band_nobody_is_in_has_no_rate_rather_than_zero() {
        let group = Group {
            label: "x",
            creators: 50,
            later_launches: 0,
            later_organic: 0,
        };
        assert_eq!(group.later_organic_bps(), None);
    }

    fn group(creators: usize, later_launches: u64, later_organic: u64) -> Group {
        Group {
            label: "x",
            creators,
            later_launches,
            later_organic,
        }
    }

    #[test]
    fn the_interval_brackets_the_rate_and_stays_a_proportion() {
        // A normal-approximation interval on 22 events in 1,279 trials happily
        // returns a negative lower bound. Wilson does not, and rare events over
        // modest samples are the only kind this study has.
        let g = group(22, 1_279, 22);
        let (lo, hi) = g.later_organic_ci_bps().expect("an interval");
        let rate = g.later_organic_bps().expect("a rate");
        assert!(lo < rate && rate < hi, "{lo} < {rate} < {hi}");
        assert!(hi <= 10_000, "a proportion cannot exceed 100%");
    }

    #[test]
    fn zero_events_still_gives_a_bounded_interval_starting_at_zero() {
        // The case that breaks naive intervals: p = 0 gives a zero-width normal
        // interval, which would read as certainty that the rate is exactly zero.
        let (lo, hi) = group(50, 5_000, 0)
            .later_organic_ci_bps()
            .expect("interval");
        assert_eq!(lo, 0);
        assert!(hi > 0, "zero observed events is not proof of a zero rate");
    }

    #[test]
    fn a_wider_sample_narrows_the_interval() {
        let narrow = group(50, 10_000, 100).later_organic_ci_bps().expect("a");
        let wide = group(50, 500, 5).later_organic_ci_bps().expect("b");
        assert!(
            narrow.1 - narrow.0 < wide.1 - wide.0,
            "more evidence must mean less uncertainty"
        );
    }

    #[test]
    fn overlapping_intervals_are_not_a_separation() {
        // The comparison the study is entitled to make, and the mistake it exists
        // to prevent: two midpoints far apart whose intervals overlap have not
        // been shown to differ.
        let high = group(20, 300, 6); // 2.00%
        let low = group(500, 7_000, 105); // 1.50%
        assert!(
            !high.clearly_above(&low),
            "overlapping intervals must not read as a difference"
        );

        // The live shape, which does separate: 22/1279 against 60/7328.
        let measured_high = group(22, 1_279, 22);
        let measured_low = group(552, 7_328, 60);
        assert!(measured_high.clearly_above(&measured_low));
    }

    #[test]
    fn a_group_too_small_to_rate_is_never_clearly_above_anything() {
        let tiny = group(MIN_GROUP - 1, 10, 10);
        assert!(!tiny.clearly_above(&group(500, 7_000, 70)));
    }

    #[test]
    fn the_base_rate_is_over_the_studied_population_not_the_whole_store() {
        // Every group rate is read against this, so it has to describe the same
        // population the groups are drawn from.
        let study = Study {
            pivot: Slot(100),
            head: Slot(200),
            creators: 10,
            prior_launches: 100,
            prior_measured: 100,
            later_launches: 500,
            later_organic: 15,
            groups: Vec::new(),
        };
        assert_eq!(study.base_rate_bps(), Some(300));
    }

    #[test]
    fn an_empty_study_has_no_base_rate() {
        let study = Study {
            pivot: Slot(100),
            head: Slot(200),
            creators: 0,
            prior_launches: 0,
            prior_measured: 0,
            later_launches: 0,
            later_organic: 0,
            groups: Vec::new(),
        };
        assert_eq!(study.base_rate_bps(), None);
    }
}
