// SPDX-License-Identifier: Apache-2.0
//! Coordination as an **event**, not as a property of a launch.
//!
//! [`crate::assess`] reads a token's launch block once and the verdict stands
//! forever. That is wrong in a way that matters: a token can sit dormant for
//! years, be bundled by whoever picks it up, and go to the moon and get rugged —
//! and it was clean at launch, so it stays labelled clean on precisely the day
//! the label mattered.
//!
//! The detector was never actually about launches. [`LaunchBlockShape`] counts
//! recipients *in a block*, and nothing in that counting requires the block to
//! be the first one. This module applies the same shape test to any block.
//!
//! # What is borrowed from 0008, and what is not
//!
//! **The shape is borrowed. The odds are not**, and the separation is the whole
//! design of this file.
//!
//! [research 0008](https://github.com/hey-vera/radar/blob/main/docs/research/0008-the-launch-block-gives-the-bundle-away.md)
//! measured that 68% of instantly-graduating launches had exactly six
//! recipients in the **launch block**, against 5% of launches that never
//! graduated. Every number in it — the band, the lift, the base rate — was
//! measured on that population.
//!
//! A bundle-shaped block three years later is a **different population**, and
//! nobody has measured what it predicts. So [`Sighting`] reports the shape and
//! how far into the token's life it appeared, and carries **no lift**. Reusing
//! 0008's multiplier here would be quoting a measurement taken somewhere else,
//! which is the exact error `0014` made and `0016` had to correct.
//!
//! Rule 9's shape: unmeasured is not "no effect", it is unmeasured. This
//! produces an observation worth recording and refusing on. It does not produce
//! a number.

use serde::{Deserialize, Serialize};

use crate::{BUNDLE_BAND, BUNDLE_CENTRE, Coordination, LaunchBlockShape};

/// Where in a token's life a block sits.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum When {
    /// The token's own launch block, which is what 0008 measured.
    Launch,
    /// Some later block, `slots_after` slots into the token's life.
    Later {
        /// How far past the launch.
        slots_after: u64,
    },
}

impl When {
    /// Whether 0008's measured odds apply to a shape seen here.
    ///
    /// Only at the launch. Everywhere else the shape is an observation whose
    /// predictive value nobody has measured.
    #[must_use]
    pub const fn carries_measured_odds(self) -> bool {
        matches!(self, Self::Launch)
    }
}

/// A bundle-shaped block, wherever it appeared.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Sighting {
    /// What was counted.
    pub shape: LaunchBlockShape,
    /// The pattern it matches.
    pub coordination: Coordination,
    /// Where in the token's life it was seen.
    pub when: When,
}

impl Sighting {
    /// Whether this is worth acting on at all.
    ///
    /// `Likely` only, matching [`Coordination::is_actionable`]. A `Suspected`
    /// shape is recorded and not acted on, here for the same reason as at
    /// launch: the band is 30% of *organic* graduations too, so refusing on it
    /// would refuse a third of the honest population.
    #[must_use]
    pub const fn is_actionable(&self) -> bool {
        self.coordination.is_actionable()
    }
}

/// Looks for a bundle shape in one block.
///
/// `None` when the block is unremarkable — an absence of evidence, which is not
/// evidence the token is clean and is deliberately not represented as a
/// `Sighting` that says so.
#[must_use]
pub fn look(shape: LaunchBlockShape, when: When) -> Option<Sighting> {
    let coordination = if shape.recipients == BUNDLE_CENTRE {
        Coordination::Likely
    } else if BUNDLE_BAND.contains(&shape.recipients) {
        Coordination::Suspected
    } else {
        return None;
    };
    Some(Sighting {
        shape,
        coordination,
        when,
    })
}

/// The strongest sighting across a run of blocks.
///
/// Blocks arrive as `(slots_after_launch, shape)`. A `slots_after` of zero is
/// the launch block itself, so one sweep covers both questions and a caller
/// cannot accidentally ask only the old one.
///
/// Strongest rather than most recent: a `Likely` from a year ago outranks a
/// `Suspected` from this morning, because the question being answered is
/// "has this token ever been bundled", and the answer does not expire.
#[must_use]
pub fn strongest<I>(blocks: I) -> Option<Sighting>
where
    I: IntoIterator<Item = (u64, LaunchBlockShape)>,
{
    blocks
        .into_iter()
        .filter_map(|(slots_after, shape)| {
            let when = if slots_after == 0 {
                When::Launch
            } else {
                When::Later { slots_after }
            };
            look(shape, when)
        })
        // `max_by_key` keeps the *last* maximum, so ties resolve to the later
        // block. That is the right tie-break: of two equally strong sightings,
        // the recent one is the more actionable fact about the token now.
        .max_by_key(|s| s.coordination)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(recipients: u64) -> LaunchBlockShape {
        LaunchBlockShape {
            recipients,
            transactions: recipients,
        }
    }

    #[test]
    fn a_bundle_years_after_launch_is_still_found() {
        // The case this module exists for. The token was clean at launch, so
        // `assess` labelled it clean and the label stood -- on exactly the day
        // it mattered least and then on the day it mattered most.
        let sighting = look(
            shape(BUNDLE_CENTRE),
            When::Later {
                slots_after: 200_000_000,
            },
        )
        .expect("a bundle shape is a bundle shape whenever it appears");
        assert_eq!(sighting.coordination, Coordination::Likely);
        assert!(sighting.is_actionable());
    }

    #[test]
    fn a_later_sighting_does_not_borrow_0008s_odds() {
        // The honesty constraint, and the reason this is a separate type rather
        // than a second call into `assess`. 0008 measured launch blocks. A
        // bundle three years in is a different population and nobody has
        // measured what it predicts, so there is no lift to quote.
        //
        // Quoting one anyway is exactly what 0014 did and 0016 had to correct.
        assert!(When::Launch.carries_measured_odds());
        assert!(!When::Later { slots_after: 1 }.carries_measured_odds());
        assert!(
            !When::Later {
                slots_after: 200_000_000
            }
            .carries_measured_odds()
        );
    }

    #[test]
    fn an_unremarkable_block_produces_nothing_rather_than_a_clean_verdict() {
        // Rule 9. An absence of evidence is not evidence of absence, and a
        // `Sighting` saying "unremarkable" would be a record asserting the token
        // was looked at and found fine.
        for recipients in [0, 1, 2, 3, 4, 8, 20, 500] {
            assert_eq!(look(shape(recipients), When::Launch), None, "{recipients}");
        }
    }

    #[test]
    fn the_band_is_recorded_but_not_actionable() {
        // 5..=7 is 30% of *organic* graduations too, so refusing on it would
        // refuse a third of the honest population. Same rule as at launch.
        for recipients in [5, 7] {
            let sighting = look(shape(recipients), When::Launch).expect("in the band");
            assert_eq!(sighting.coordination, Coordination::Suspected);
            assert!(
                !sighting.is_actionable(),
                "{recipients} must not be acted on"
            );
        }
        let centre = look(shape(BUNDLE_CENTRE), When::Launch).expect("the centre");
        assert!(centre.is_actionable());
    }

    #[test]
    fn a_sweep_covers_the_launch_and_everything_after_it() {
        // One call answers both questions, so a caller cannot accidentally ask
        // only the old one.
        let found = strongest([
            (0, shape(3)),
            (100, shape(9)),
            (5_000_000, shape(BUNDLE_CENTRE)),
        ])
        .expect("the late bundle");
        assert_eq!(found.coordination, Coordination::Likely);
        assert_eq!(
            found.when,
            When::Later {
                slots_after: 5_000_000
            }
        );
    }

    #[test]
    fn slot_zero_is_the_launch_block() {
        let found = strongest([(0, shape(BUNDLE_CENTRE))]).expect("a launch bundle");
        assert_eq!(found.when, When::Launch);
        assert!(found.when.carries_measured_odds(), "0008 applies here");
    }

    #[test]
    fn the_strongest_sighting_wins_over_the_most_recent() {
        // "Has this token ever been bundled" is the question, and the answer
        // does not expire. A `Likely` from long ago outranks a `Suspected` from
        // this morning.
        let found =
            strongest([(10, shape(BUNDLE_CENTRE)), (9_000_000, shape(5))]).expect("something");
        assert_eq!(found.coordination, Coordination::Likely);
        assert_eq!(found.when, When::Later { slots_after: 10 });
    }

    #[test]
    fn equal_strength_resolves_to_the_later_block() {
        // Of two equally strong sightings the recent one is the more actionable
        // fact about the token as it is now.
        let found = strongest([
            (10, shape(BUNDLE_CENTRE)),
            (9_000_000, shape(BUNDLE_CENTRE)),
        ])
        .expect("something");
        assert_eq!(
            found.when,
            When::Later {
                slots_after: 9_000_000
            }
        );
    }

    #[test]
    fn a_sweep_of_nothing_finds_nothing() {
        assert_eq!(strongest(Vec::new()), None);
        assert_eq!(strongest([(0, shape(1)), (5, shape(100))]), None);
    }
}
