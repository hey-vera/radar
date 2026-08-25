// SPDX-License-Identifier: Apache-2.0
//! Coordination detection: what the launch block says about who was waiting.
//!
//! A bundled launch is one where the buyers were arranged before the token
//! existed. That is invisible in any single transaction and obvious in the shape
//! of the block, which is why this reads a block rather than a trade.
//!
//! # The measurement this is built on
//!
//! [`docs/research/0008`](../../docs/research/0008-the-launch-block-gives-the-bundle-away.md)
//! counted distinct recipients of the new token inside its own launch block,
//! across three populations of eighty launches each:
//!
//! | recipients in the launch block | never graduated | graduated organically | graduated instantly |
//! |---|---|---|---|
//! | exactly six | 5% | 16% | **68%** |
//! | five to seven | 12% | 30% | **88%** |
//!
//! An ordinary launch has one to three recipients. An instantly-graduating one
//! has six, over and over, with **no** observed cases at two, three, four, seven,
//! eight or nine. A distribution that tight is not a market; it is a tool with a
//! default setting.
//!
//! # Why this refuses rather than buys
//!
//! The signal predicts graduation — a launch with exactly six recipients is 6.2×
//! likelier to graduate than average. It is tempting to read that as a buy.
//!
//! It is the opposite, and this is the whole point of the crate. The same
//! observation predicts *instant* graduation at **11.7×**, and an instant
//! graduation means the bonding curve was bought out by people who were ready
//! before the token existed. They now hold the supply, and the only thing left
//! for a later buyer to do is be the person they sell to. The graduation is real
//! and the opportunity belongs to somebody else.
//!
//! So a strong score here is a reason to stay away. Radar's whole thesis is that
//! the realistic edge is *not buying traps* rather than finding rockets, and this
//! is that thesis with a number attached.
//!
//! # What this crate is not
//!
//! It has no I/O. It scores an observation somebody else fetched, in the same way
//! `radar-risk` decides without knowing where a proposal came from. Fetching a
//! launch block is a Tier-1 investigation — one query for one candidate that has
//! already survived the free filters — and belongs on the provider path, not
//! here.

#![forbid(unsafe_code)]

use radar_types::EvidenceTier;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What was observed in a token's own launch block.
///
/// Deliberately small. Every field is something one query can answer, and
/// nothing here needs the funding graph — which is the expensive, unbuilt half
/// of coordination detection and is not required for the part that already
/// measures.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct LaunchBlockShape {
    /// Distinct accounts that received the token inside its launch block.
    ///
    /// The load-bearing number. Not "buyers": these are token accounts, and
    /// resolving them to owners is a separate join that this does not claim to
    /// have done. Naming it for what was counted keeps the claim the size of the
    /// evidence.
    pub recipients: u64,
    /// Distinct transactions touching the token in that block.
    pub transactions: u64,
}

/// How strongly a launch looks arranged in advance.
#[derive(
    Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, JsonSchema, PartialOrd, Ord,
)]
#[serde(rename_all = "snake_case")]
pub enum Coordination {
    /// Nothing unusual. One to four recipients, which is what most launches are.
    Unremarkable,
    /// Inside the band where bundles concentrate, but not at its centre.
    Suspected,
    /// The exact shape that 68% of instantly-graduating launches had, and 5% of
    /// launches that never graduated.
    Likely,
}

impl Coordination {
    /// Whether this is strong enough to act on.
    ///
    /// Only [`Self::Likely`]. `Suspected` fires on 13% of all launches and
    /// carries a four-fold enrichment; that is worth recording and too blunt to
    /// refuse on by itself.
    #[must_use]
    pub const fn is_actionable(self) -> bool {
        matches!(self, Self::Likely)
    }

    /// How direct the evidence behind this verdict is.
    ///
    /// Never better than [`EvidenceTier::Strong`]. The block is observed
    /// directly, but "these six accounts were arranged beforehand" is an
    /// inference from a distribution, not something the chain states. Calling it
    /// `Direct` would claim the conclusion is as solid as the observation.
    #[must_use]
    pub const fn tier(self) -> EvidenceTier {
        match self {
            Self::Unremarkable => EvidenceTier::Weak,
            Self::Suspected | Self::Likely => EvidenceTier::Strong,
        }
    }
}

/// The measured verdict on a launch block.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Assessment {
    /// What was observed.
    pub shape: LaunchBlockShape,
    /// What it looks like.
    pub coordination: Coordination,
    /// How much likelier this shape makes an *instant* graduation, in hundredths.
    ///
    /// Hundredths rather than a float, because a threshold compared as a float
    /// compares differently on a replay and a replay that disagrees with its
    /// recording is indistinguishable from a leak.
    pub instant_lift_x100: u64,
}

/// The centre of the bundle distribution.
///
/// 68% of instantly-graduating launches had exactly this many recipients,
/// against 5% of launches that never graduated.
pub const BUNDLE_CENTRE: u64 = 6;

/// The band bundles fall in.
///
/// 88% of instant graduations, 30% of organic ones, 12% of launches that never
/// graduated.
pub const BUNDLE_BAND: std::ops::RangeInclusive<u64> = 5..=7;

/// Instant-graduation lift at the centre of the band, in hundredths.
const CENTRE_LIFT_X100: u64 = 1_170;
/// Instant-graduation lift across the band, in hundredths.
const BAND_LIFT_X100: u64 = 670;

/// Somewhere a launch block's shape can be read from.
///
/// A trait rather than a concrete client for the same reason [`Quoter`] is one
/// in `radar-sim`: this crate is pure policy, and a policy crate that knows how
/// to open a socket is a policy crate that cannot be tested without one.
///
/// Implementations must answer **only** about the given slot. The whole signal
/// is that a bundle is visible in the launch block specifically, and a source
/// that widened the window to be helpful would dissolve it.
///
/// [`Quoter`]: https://github.com/hey-vera/radar
pub trait LaunchBlockSource {
    /// Why the shape could not be read.
    type Error;

    /// Counts the token's recipients inside one slot.
    ///
    /// # Errors
    ///
    /// Returns `Self::Error` if the source cannot answer. A source that cannot
    /// answer must say so rather than returning an empty shape — zero recipients
    /// is a real observation meaning nobody received the token, and it scores as
    /// `Unremarkable`, so a failure reported as zero would quietly clear a
    /// bundle.
    fn shape_at(
        &self,
        mint: &radar_types::Address,
        slot: radar_types::Slot,
    ) -> Result<LaunchBlockShape, Self::Error>;
}

/// Scores a launch block.
///
/// Pure and total: every shape gets a verdict, including shapes the measurement
/// never saw. A launch with forty recipients is not a bundle by this evidence
/// and is not claimed to be one.
#[must_use]
pub fn assess(shape: LaunchBlockShape) -> Assessment {
    let coordination = if shape.recipients == BUNDLE_CENTRE {
        Coordination::Likely
    } else if BUNDLE_BAND.contains(&shape.recipients) {
        Coordination::Suspected
    } else {
        Coordination::Unremarkable
    };

    Assessment {
        shape,
        coordination,
        instant_lift_x100: match coordination {
            Coordination::Likely => CENTRE_LIFT_X100,
            Coordination::Suspected => BAND_LIFT_X100,
            // Not "no lift" as a measured claim — simply that this shape carries
            // no evidence either way, which renders as the neutral multiplier.
            Coordination::Unremarkable => 100,
        },
    }
}

/// Share of all launches that 0008 measured at [`BUNDLE_CENTRE`], per ten
/// thousand.
///
/// The comparison point for a decay check. If the sampled rate falls a long way
/// below this and stays there, either the market changed or the bundler's
/// default moved off six — and the second is invisible without this number to
/// compare against.
pub const MEASURED_CENTRE_RATE_BPS: u64 = 580;

/// Share of all launches that 0008 measured inside [`BUNDLE_BAND`], per ten
/// thousand.
pub const MEASURED_BAND_RATE_BPS: u64 = 1_310;

/// What a sample of launch blocks looked like.
///
/// Exists because a detector that refuses nothing and a detector that has
/// stopped working produce identical output otherwise. `radar consider` reads a
/// launch block for every paid-tier candidate and reported only how many it
/// read; the shape it read was thrown away, so `0 refused on shape` carried no
/// information about whether the gate was awake. See [LEARNINGS] entry 5: a
/// check that reports absence the same way it reports success is not a check.
///
/// [LEARNINGS]: https://github.com/hey-vera/radar/blob/main/LEARNINGS.md
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Distribution {
    counts: std::collections::BTreeMap<u64, usize>,
    total: usize,
}

impl Distribution {
    /// An empty sample.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Folds one observed launch block in.
    pub fn observe(&mut self, shape: LaunchBlockShape) {
        *self.counts.entry(shape.recipients).or_default() += 1;
        self.total += 1;
    }

    /// How many launch blocks were observed.
    #[must_use]
    pub const fn total(&self) -> usize {
        self.total
    }

    /// Whether anything was observed at all.
    ///
    /// The distinction this type exists for. An empty sample means the source
    /// was not consulted; a non-empty sample with nothing at the centre means it
    /// was consulted and found nothing. Those are opposite findings and they
    /// must never render alike.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.total == 0
    }

    /// How many blocks had exactly this many recipients.
    #[must_use]
    pub fn count(&self, recipients: u64) -> usize {
        self.counts.get(&recipients).copied().unwrap_or(0)
    }

    /// Every observed recipient count and its frequency, ascending.
    pub fn iter(&self) -> impl Iterator<Item = (u64, usize)> + '_ {
        self.counts.iter().map(|(k, v)| (*k, *v))
    }

    /// How many scored each way.
    #[must_use]
    pub fn verdicts(&self) -> Verdicts {
        let mut v = Verdicts::default();
        for (recipients, count) in self.iter() {
            match assess(LaunchBlockShape {
                recipients,
                transactions: 0,
            })
            .coordination
            {
                Coordination::Likely => v.likely += count,
                Coordination::Suspected => v.suspected += count,
                Coordination::Unremarkable => v.unremarkable += count,
            }
        }
        v
    }

    /// The observed rate at the centre, per ten thousand, or `None` when
    /// nothing was observed.
    ///
    /// `None` rather than zero, because "no sample" and "a sample containing no
    /// bundles" are the two states this whole type exists to keep apart.
    /// AGENTS.md rule 9: absent is not zero.
    #[must_use]
    pub fn centre_rate_bps(&self) -> Option<u64> {
        self.rate_bps(self.verdicts().likely)
    }

    /// The observed rate across the band, per ten thousand, or `None` when
    /// nothing was observed.
    #[must_use]
    pub fn band_rate_bps(&self) -> Option<u64> {
        let v = self.verdicts();
        self.rate_bps(v.likely + v.suspected)
    }

    fn rate_bps(&self, hits: usize) -> Option<u64> {
        if self.total == 0 {
            return None;
        }
        // Integer throughout: a rate compared as a float compares differently on
        // a replay, and this one is destined for a threshold.
        Some(
            u64::try_from(hits)
                .unwrap_or(u64::MAX)
                .saturating_mul(10_000)
                / self.total as u64,
        )
    }
}

/// How a sample of launch blocks scored.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Verdicts {
    /// Exactly at the centre. These are refused.
    pub likely: usize,
    /// Inside the band but not at its centre. Recorded, not refused.
    pub suspected: usize,
    /// Everything else.
    pub unremarkable: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(recipients: u64) -> LaunchBlockShape {
        LaunchBlockShape {
            recipients,
            transactions: 4,
        }
    }

    #[test]
    fn the_measured_centre_reads_as_likely() {
        // 68% of instant graduations against 5% of launches that never
        // graduated. This is the shape the crate exists to notice.
        let a = assess(shape(BUNDLE_CENTRE));
        assert_eq!(a.coordination, Coordination::Likely);
        assert!(a.coordination.is_actionable());
        assert_eq!(a.instant_lift_x100, CENTRE_LIFT_X100);
    }

    #[test]
    fn an_ordinary_launch_is_unremarkable() {
        // Most launches have one to three recipients. If those read as
        // coordinated the signal would fire on everything and mean nothing --
        // the failure research 0004 recorded for the same-slot-buy heuristic,
        // which was present in 91% of launches.
        for recipients in [0, 1, 2, 3, 4] {
            let a = assess(shape(recipients));
            assert_eq!(
                a.coordination,
                Coordination::Unremarkable,
                "{recipients} recipients"
            );
            assert!(!a.coordination.is_actionable());
        }
    }

    #[test]
    fn the_band_around_the_centre_is_suspected_but_not_actionable() {
        for recipients in [5, 7] {
            let a = assess(shape(recipients));
            assert_eq!(a.coordination, Coordination::Suspected, "{recipients}");
            assert!(
                !a.coordination.is_actionable(),
                "13% of all launches land here; refusing on it alone is too blunt"
            );
        }
    }

    #[test]
    fn a_crowded_launch_block_is_not_a_bundle() {
        // The tempting mistake is "more buyers means more coordination". The
        // measurement says the opposite: bundles are tightly clustered at six,
        // and the wide tail belongs to organic launches.
        for recipients in [12, 40, 500] {
            assert_eq!(
                assess(shape(recipients)).coordination,
                Coordination::Unremarkable,
                "{recipients} recipients is not evidence of arrangement"
            );
        }
    }

    #[test]
    fn no_verdict_claims_direct_evidence() {
        // The block is observed directly; "these accounts were arranged in
        // advance" is inferred from a distribution. Claiming Direct would make
        // the conclusion look as solid as the observation.
        // `EvidenceTier` lists Direct first, so its derived ordering runs
        // strongest-to-weakest and `>= Strong` means "no more direct than
        // Strong". Reading it the intuitive way round is how this test failed
        // on its first run.
        for recipients in [0, 5, 6, 7, 99] {
            let tier = assess(shape(recipients)).coordination.tier();
            assert_ne!(tier, EvidenceTier::Direct, "{recipients}");
            assert!(tier >= EvidenceTier::Strong, "{recipients}");
        }
    }

    fn sample(recipients: &[u64]) -> Distribution {
        let mut d = Distribution::new();
        for r in recipients {
            d.observe(shape(*r));
        }
        d
    }

    #[test]
    fn an_unread_sample_and_a_clean_one_are_not_the_same_state() {
        // The reason this type exists. `radar consider` read 25 launch blocks,
        // refused 0, and printed a line that a broken source would have printed
        // identically. Nothing downstream could tell "looked and found nothing"
        // from "never looked" -- LEARNINGS 5, and it fails permissive, because
        // the gate being off looks exactly like the population being clean.
        let unread = Distribution::new();
        let clean = sample(&[1, 2, 2, 3, 3, 3]);

        assert!(unread.is_empty());
        assert!(!clean.is_empty());
        assert_eq!(unread.centre_rate_bps(), None, "no sample has no rate");
        assert_eq!(
            clean.centre_rate_bps(),
            Some(0),
            "a sample with no bundles has a rate, and it is zero"
        );
        assert_ne!(unread.centre_rate_bps(), clean.centre_rate_bps());
    }

    #[test]
    fn the_verdict_tally_sums_to_what_was_observed() {
        let d = sample(&[1, 2, 3, 4, 5, 6, 6, 7, 12, 40]);
        let v = d.verdicts();
        assert_eq!(v.likely, 2, "two at exactly six");
        assert_eq!(v.suspected, 2, "one five, one seven");
        assert_eq!(v.unremarkable, 6);
        assert_eq!(v.likely + v.suspected + v.unremarkable, d.total());
        assert_eq!(d.total(), 10);
    }

    #[test]
    fn the_observed_rate_is_comparable_with_what_was_measured() {
        // The decay check. 0008 measured 5.8% of launches at the centre; a
        // sample running far below that for long enough is either a changed
        // market or a bundler default that has moved off six, and without this
        // comparison neither is visible.
        let d = sample(&[6, 1, 1, 1, 1, 1, 1, 1, 1, 1]);
        assert_eq!(d.centre_rate_bps(), Some(1_000), "1 in 10 is 1000 bps");

        // A sample matching the measurement exactly.
        let mut matching = sample(&[6, 6, 6, 6, 6]);
        for _ in 0..95 {
            matching.observe(shape(2));
        }
        assert_eq!(matching.centre_rate_bps(), Some(500));
        assert!(
            matching.centre_rate_bps().unwrap() < MEASURED_CENTRE_RATE_BPS,
            "5% against a measured 5.8% -- close, and the point is that the \
             comparison is possible at all"
        );
    }

    #[test]
    fn the_band_rate_counts_the_centre_too() {
        // The band is 5..=7 inclusive of six. A band rate that excluded the
        // centre would be smaller than the centre rate, which is nonsense.
        let d = sample(&[5, 6, 7, 1, 1, 1, 1, 1, 1, 1]);
        assert_eq!(d.band_rate_bps(), Some(3_000));
        assert_eq!(d.centre_rate_bps(), Some(1_000));
        assert!(d.band_rate_bps() >= d.centre_rate_bps());
    }

    #[test]
    fn counts_are_kept_per_recipient_value_not_bucketed() {
        // The signal is a spike at exactly six with holes either side. Bucketing
        // "5 to 7" together would destroy the shape that makes it convincing --
        // 0008's argument is the holes, not the peak.
        let d = sample(&[5, 6, 6, 7]);
        assert_eq!(d.count(5), 1);
        assert_eq!(d.count(6), 2);
        assert_eq!(d.count(7), 1);
        assert_eq!(d.count(4), 0);
        assert_eq!(
            d.iter().collect::<Vec<_>>(),
            vec![(5, 1), (6, 2), (7, 1)],
            "ascending, so a histogram reads left to right"
        );
    }

    #[test]
    fn an_empty_sample_has_no_rate_in_either_direction() {
        let d = Distribution::new();
        assert_eq!(d.total(), 0);
        assert_eq!(d.centre_rate_bps(), None);
        assert_eq!(d.band_rate_bps(), None);
        assert_eq!(d.verdicts(), Verdicts::default());
    }

    #[test]
    fn scoring_is_pure() {
        // Same observation, same answer, every time -- the property that lets a
        // recorded assessment be replayed.
        let s = shape(BUNDLE_CENTRE);
        let first = assess(s);
        for _ in 0..100 {
            assert_eq!(assess(s), first);
        }
    }
}
