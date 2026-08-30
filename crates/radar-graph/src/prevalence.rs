// SPDX-License-Identifier: Apache-2.0
//! How often a wallet turns up in launch blocks that are not its own.
//!
//! The second coordination feature, and the one the plan got wrong.
//!
//! # The plan asked for something that cannot exist
//!
//! Phase E named *"repeat co-occurrence of the same recipient set across
//! different creators"*. [`docs/research/0012`](../../docs/research/0012-recipient-sets-cannot-recur-authorities-can.md)
//! shows that is not computable: `destination` is a **token account**, a
//! `(owner, mint)` pair, so two mints cannot share one. Measured over ten
//! minutes of transfers, the maximum number of mints any destination touches is
//! **two**. There is no tail to find.
//!
//! [`LaunchBlockShape`](crate::LaunchBlockShape) already carried that caution —
//! it names the field `recipients` rather than `buyers` precisely because
//! resolving token accounts to owners is a join Radar has not done. The caution
//! turned out to be load-bearing.
//!
//! # What recurs instead
//!
//! `authority` — the wallet that signed the transfer. It is a wallet address
//! rather than a token account, so it recurs by construction, and it does.
//!
//! # Why the head has to be cut
//!
//! Over ninety minutes and 17,032 launches, **thirteen addresses appeared in the
//! launch blocks of 42% of them.** They are routers and fee sinks. Any two
//! launches share one, so a signal built on raw co-occurrence would fire on
//! nearly everything — which is not a signal, it is a restatement of how the
//! venue works.
//!
//! So prevalence is scored in bands, with the head excluded by a measured
//! threshold rather than by a hand-written denylist. A denylist would need
//! maintaining by whoever noticed the next router, and nobody notices.
//!
//! # This refuses nothing
//!
//! Deliberately. The measurement says *who* recurs; it says nothing yet about
//! whether recurrence predicts anything about money, because that needs a join
//! against Radar's own outcomes that has not been run. Recording it beside the
//! decision is what makes that join possible later — the same order 0008 and the
//! decisions table were built in.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::EvidenceTier;

/// Appearances below which a wallet is an ordinary participant.
///
/// 89.5% of authorities appear in exactly one launch block and 4.7% in two. A
/// wallet seen twice in ninety minutes is unremarkable; the population is large
/// enough that coincidence at two is common.
pub const REPEAT_FLOOR: u64 = 3;

/// Appearances above which a wallet is infrastructure rather than a participant.
///
/// Thirteen addresses sit above this and cover 42% of launches. Read off one
/// histogram, so it is provisional in the way [`crate::BUNDLE_CENTRE`] is
/// provisional — a tool with a default setting, and it will move.
pub const INFRASTRUCTURE_FLOOR: u64 = 100;

/// The window the bands were measured over.
///
/// Carried as a constant because a count is meaningless without it: forty
/// launches in ninety minutes and forty in a month are different facts, and a
/// caller comparing a count taken over a different window to these thresholds
/// would be comparing two different quantities.
pub const WINDOW_MINUTES: u64 = 90;

/// What an authority's launch-block prevalence looks like.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Prevalence {
    /// One or two launch blocks. What almost every wallet looks like.
    Ordinary,
    /// Three to a hundred. Neither a one-off nor a router.
    ///
    /// The band the finding is about: 0.6% of authorities, 4,031 launch
    /// appearances. The shape of somebody running a factory.
    Repeat,
    /// Above the infrastructure floor. A router, an aggregator or a fee sink.
    ///
    /// **Not evidence about the token.** Reporting this as coordination would
    /// mean reporting it about nearly every launch.
    Infrastructure,
}

impl Prevalence {
    /// Classifies a count of launch-block appearances.
    ///
    /// # Panics
    ///
    /// Never. Every count lands in exactly one band, including zero — which is
    /// `Ordinary`, because a wallet that has appeared in no launch block at all
    /// is the least remarkable thing there is.
    #[must_use]
    pub const fn of(launch_blocks: u64) -> Self {
        if launch_blocks >= INFRASTRUCTURE_FLOOR {
            Self::Infrastructure
        } else if launch_blocks >= REPEAT_FLOOR {
            Self::Repeat
        } else {
            Self::Ordinary
        }
    }

    /// How direct the evidence behind this reading is.
    ///
    /// Never better than [`EvidenceTier::Weak`], and that is the honest answer
    /// today. The count is observed directly, but "this wallet runs a factory"
    /// is an inference from one ninety-minute window with **no measured link to
    /// any outcome**. 0008's signal earns `Strong` because three populations
    /// were compared; this has compared none.
    #[must_use]
    pub const fn tier(self) -> EvidenceTier {
        EvidenceTier::Weak
    }

    /// Whether this should change a decision.
    ///
    /// Always false, and it is a method rather than an absence so that the
    /// answer is written down where somebody would look for it. Nothing refuses
    /// on prevalence until the outcome join in 0012's *What this does not
    /// establish* has been run.
    #[must_use]
    pub const fn is_actionable(self) -> bool {
        false
    }

    /// How this reads in a brief.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Ordinary => "ordinary",
            Self::Repeat => "repeat launcher",
            Self::Infrastructure => "infrastructure",
        }
    }
}

/// The number of rows the public endpoint will return.
///
/// Fixed and unraisable — the user is `readonly=1`. A result of exactly this
/// many rows has been cut off, and a cut-off table is the dangerous case: every
/// authority the cut removed reads as `Ordinary`, which is the *least* alarming
/// answer. Rule 9 in its sharpest form — the truncation is silent and it fails
/// permissive.
pub const ROW_CAP: usize = 1_000;

/// Every authority that appears in at least [`REPEAT_FLOOR`] launch blocks.
///
/// Fetched once per run rather than once per candidate. The per-candidate
/// version of this query took **32 seconds** against the real endpoint; at forty
/// candidates an hour that is twenty minutes of query time per hour on an
/// endpoint Radar is a guest on. One window query answers for every candidate,
/// and the per-candidate cost stays the cheap single-block read it already was.
///
/// Only authorities at or above the floor are fetched, because everything below
/// it classifies as [`Prevalence::Ordinary`] anyway — which keeps the result
/// inside the row cap and makes an absent authority *correct* rather than
/// merely convenient.
/// Deliberately **not** `Default`. A derived default would be indistinguishable
/// from [`Table::unavailable`] — empty and incomplete — which is the safe answer
/// by accident rather than by design, and it would let `Table::default()` stand
/// in for a table somebody meant to fetch. A type whose default silently means
/// "cannot see" is one that will eventually be defaulted by mistake.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Table {
    counts: BTreeMap<String, u64>,
    complete: bool,
}

impl Table {
    /// Builds a table from rows the source returned.
    ///
    /// `complete` is false when the source hit its row cap, and the whole table
    /// then refuses to answer rather than answering `Ordinary` for the
    /// authorities the cut removed.
    #[must_use]
    pub fn new(rows: impl IntoIterator<Item = (String, u64)>) -> Self {
        let counts: BTreeMap<String, u64> = rows.into_iter().collect();
        let complete = counts.len() < ROW_CAP;
        Self { counts, complete }
    }

    /// A table that could not be read.
    ///
    /// Distinct from an empty one. An empty table is a real observation — no
    /// wallet reached the floor — and answers `Ordinary`; this answers nothing.
    #[must_use]
    pub fn unavailable() -> Self {
        Self {
            counts: BTreeMap::new(),
            complete: false,
        }
    }

    /// Whether this table can be trusted to answer.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.complete
    }

    /// How many authorities reached the floor.
    #[must_use]
    pub fn len(&self) -> usize {
        self.counts.len()
    }

    /// Whether no authority reached the floor.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.counts.is_empty()
    }

    /// What this authority looks like, or `None` if the table cannot say.
    ///
    /// An authority the table does not name is below the floor and therefore
    /// [`Prevalence::Ordinary`] — correct by construction, because the query
    /// asked for everything at or above it.
    #[must_use]
    pub fn of(&self, authority: &str) -> Option<Prevalence> {
        self.complete.then(|| {
            self.counts
                .get(authority)
                .map_or(Prevalence::Ordinary, |c| Prevalence::of(*c))
        })
    }

    /// The strongest reading among a block's authorities, or `None`.
    #[must_use]
    pub fn strongest_of(&self, authorities: &[String]) -> Option<Prevalence> {
        if !self.complete {
            return None;
        }
        let counts: Vec<u64> = authorities
            .iter()
            .map(|a| self.counts.get(a).copied().unwrap_or_default())
            .collect();
        Some(strongest(&counts))
    }
}

/// The strongest reading among the authorities in one launch block.
///
/// `Repeat` outranks both others: a block containing one repeat launcher and
/// three routers is interesting because of the launcher, and taking the maximum
/// by count would return the router every time.
///
/// An empty block is `Ordinary` rather than absent, because a launch block with
/// no signing authority recorded is a normal observation — 0008 found launches
/// with a single transfer — and rule 9's "absent is not safe" is about a missing
/// *measurement*, not about a measured absence.
#[must_use]
pub fn strongest(counts: &[u64]) -> Prevalence {
    counts
        .iter()
        .map(|c| Prevalence::of(*c))
        .max_by_key(|p| match p {
            // Ordered by how much attention each deserves, which is not the
            // order the variants are declared in.
            Prevalence::Repeat => 2u8,
            Prevalence::Infrastructure => 1,
            Prevalence::Ordinary => 0,
        })
        .unwrap_or(Prevalence::Ordinary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bands_are_the_ones_that_were_measured() {
        // Each boundary asserted on both sides. A band whose edges are only
        // tested from the inside is a band whose edges could be anywhere.
        assert_eq!(Prevalence::of(0), Prevalence::Ordinary);
        assert_eq!(Prevalence::of(1), Prevalence::Ordinary);
        assert_eq!(Prevalence::of(2), Prevalence::Ordinary);
        assert_eq!(Prevalence::of(3), Prevalence::Repeat);
        assert_eq!(Prevalence::of(99), Prevalence::Repeat);
        assert_eq!(Prevalence::of(100), Prevalence::Infrastructure);
        assert_eq!(Prevalence::of(4_428), Prevalence::Infrastructure);
        assert_eq!(Prevalence::of(u64::MAX), Prevalence::Infrastructure);
    }

    #[test]
    fn infrastructure_is_not_reported_as_coordination() {
        // The whole reason the head is cut. Thirteen addresses appear in 42% of
        // launch blocks; a signal that counted them would fire on nearly every
        // launch, which is a restatement of how the venue works rather than an
        // observation about a token.
        assert_ne!(Prevalence::of(4_428), Prevalence::Repeat);
        assert_eq!(Prevalence::of(455), Prevalence::Infrastructure);
    }

    #[test]
    fn a_repeat_launcher_outranks_a_router_in_the_same_block() {
        // Taking the maximum by *count* would return the router every time,
        // because routers have far higher counts than launchers by definition.
        // The interesting reading is the launcher.
        assert_eq!(strongest(&[4_428, 12, 1]), Prevalence::Repeat);
        assert_eq!(strongest(&[4_428, 1]), Prevalence::Infrastructure);
        assert_eq!(strongest(&[1, 2, 1]), Prevalence::Ordinary);
    }

    #[test]
    fn an_empty_block_is_ordinary_rather_than_alarming() {
        // A launch block with no recorded signing authority is a normal
        // observation -- 0008 found launches with a single transfer. Rule 9's
        // "absent is not safe" is about a missing measurement, and this is a
        // measured absence.
        assert_eq!(strongest(&[]), Prevalence::Ordinary);
        assert_eq!(strongest(&[0]), Prevalence::Ordinary);
    }

    #[test]
    fn nothing_refuses_on_this_yet_and_the_evidence_says_why() {
        // The measurement says who recurs. It says nothing about money, because
        // the outcome join has not been run -- so the tier is Weak and the
        // verdict is not actionable, in every band including the interesting
        // one.
        for count in [0, 1, 3, 99, 100, 10_000] {
            let prevalence = Prevalence::of(count);
            assert!(!prevalence.is_actionable(), "{count} was actionable");
            assert_eq!(prevalence.tier(), EvidenceTier::Weak, "{count}");
        }
    }

    #[test]
    fn the_window_travels_with_the_thresholds() {
        // Forty launches in ninety minutes and forty in a month are different
        // facts. A caller comparing a count from a different window to these
        // bands is comparing two quantities that share a unit and nothing else.
        assert_eq!(WINDOW_MINUTES, 90);

        // The bands must be ordered and non-overlapping, which is a property of
        // `of` rather than of the constants: floors the wrong way round would
        // leave the repeat band empty and classify every wallet as either
        // ordinary or infrastructure, with nothing in between and nothing
        // saying so.
        assert_eq!(Prevalence::of(REPEAT_FLOOR - 1), Prevalence::Ordinary);
        assert_eq!(Prevalence::of(REPEAT_FLOOR), Prevalence::Repeat);
        assert_eq!(
            Prevalence::of(INFRASTRUCTURE_FLOOR - 1),
            Prevalence::Repeat,
            "the repeat band reaches the infrastructure floor"
        );
        assert_eq!(
            Prevalence::of(INFRASTRUCTURE_FLOOR),
            Prevalence::Infrastructure
        );
    }

    #[test]
    fn every_band_has_a_label_a_person_can_read() {
        assert_eq!(Prevalence::of(1).label(), "ordinary");
        assert_eq!(Prevalence::of(10).label(), "repeat launcher");
        assert_eq!(Prevalence::of(1_000).label(), "infrastructure");
    }

    #[test]
    fn a_table_that_hit_the_row_cap_refuses_to_answer() {
        // The sharp one, and it fails permissive if got wrong: every authority
        // the cut removed reads as `Ordinary`, which is the least alarming
        // answer available. Rule 9 -- a truncated measurement is not a
        // measurement.
        let capped: Vec<(String, u64)> = (0..ROW_CAP)
            .map(|i| (format!("authority-{i:04}"), 50))
            .collect();
        let table = Table::new(capped);

        assert!(!table.is_complete());
        assert_eq!(table.of("authority-0001"), None, "even one it does hold");
        assert_eq!(table.of("someone-else"), None);
        assert_eq!(table.strongest_of(&["authority-0001".to_owned()]), None);
    }

    #[test]
    fn a_table_below_the_cap_answers_and_absence_means_ordinary() {
        // The query asks for everything at or above the floor, so an authority
        // it does not name is below the floor -- `Ordinary` by construction
        // rather than by convenience.
        let table = Table::new([("router".to_owned(), 4_428), ("factory".to_owned(), 8)]);

        assert!(table.is_complete());
        assert_eq!(table.len(), 2);
        assert_eq!(table.of("router"), Some(Prevalence::Infrastructure));
        assert_eq!(table.of("factory"), Some(Prevalence::Repeat));
        assert_eq!(
            table.of("never-seen"),
            Some(Prevalence::Ordinary),
            "below the floor, which is what the query means"
        );
    }

    #[test]
    fn an_empty_table_and_an_unreadable_one_are_different() {
        // An empty table is a real observation: no wallet reached the floor in
        // this window. An unreadable one is a failure. Collapsing them would
        // report a broken query as a quiet market.
        let empty = Table::new([]);
        assert!(empty.is_complete() && empty.is_empty());
        assert_eq!(empty.len(), 0);
        assert_eq!(empty.of("anyone"), Some(Prevalence::Ordinary));

        // And a populated table is not empty. Asserted because `is_empty` tested
        // only where it is true survives a version that always returns true --
        // and a full table reading as empty is a monitor reporting a quiet
        // market while the factories run.
        let populated = Table::new([("factory".to_owned(), 8)]);
        assert!(!populated.is_empty());
        assert_eq!(populated.len(), 1);

        let broken = Table::unavailable();
        assert!(!broken.is_complete() && broken.is_empty());
        assert_eq!(broken.of("anyone"), None);
    }

    #[test]
    fn a_block_is_read_through_the_table_the_way_the_caller_will() {
        // End to end over the real shape: a block with a router, a factory
        // wallet and an ordinary one reads as the factory.
        let table = Table::new([
            ("BUBBLEKuVqw2UA7EaLnacjf3zM9SvDarWH9P9CvxZeJ1".to_owned(), 8),
            (
                "Ej8Yw4ky2DB2gvPEXsi6KFoHScPyDsimLXdwYB9c9uvu".to_owned(),
                4_428,
            ),
        ]);
        let block = [
            "Ej8Yw4ky2DB2gvPEXsi6KFoHScPyDsimLXdwYB9c9uvu".to_owned(),
            "BUBBLEKuVqw2UA7EaLnacjf3zM9SvDarWH9P9CvxZeJ1".to_owned(),
            "CYVw4KWtxXQQgpizkkXSfYsGGdc4axESMv1UB77dVy7G".to_owned(),
        ];
        assert_eq!(table.strongest_of(&block), Some(Prevalence::Repeat));

        // And a block of nothing but routers reads as infrastructure, not as a
        // factory.
        let routers = ["Ej8Yw4ky2DB2gvPEXsi6KFoHScPyDsimLXdwYB9c9uvu".to_owned()];
        assert_eq!(
            table.strongest_of(&routers),
            Some(Prevalence::Infrastructure)
        );
    }

    #[test]
    fn a_reading_survives_a_round_trip_through_json() {
        // It is recorded beside the decision, which is the only reason it
        // exists today -- so the recorded form has to come back as itself.
        for count in [1u64, 5, 500] {
            let prevalence = Prevalence::of(count);
            let json = serde_json::to_string(&prevalence).expect("serialises");
            let back: Prevalence = serde_json::from_str(&json).expect("deserialises");
            assert_eq!(back, prevalence, "{json}");
        }
        assert_eq!(
            serde_json::to_string(&Prevalence::Repeat).expect("serialises"),
            "\"repeat\""
        );
    }
}
