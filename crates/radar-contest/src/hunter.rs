// SPDX-License-Identifier: Apache-2.0
//! The hunter rank: status, never money.
//!
//! Design 0009 section 5, M3. Per summoned reply, the number of refusal
//! signals the fact sheet carried at the time -- the launch block in a
//! high band, a creator with launches and no organic graduation, whichever
//! else the sheet states -- summed per summoner, with the admission gate's
//! per-summoner daily cap applied again here so that volume cannot win.
//!
//! What the rule rewards is finding launches worth refusing, which is the
//! skill the bot teaches every time it answers. What it deliberately does not
//! count: earliness (a script wins on speed), engagement (it can be bought),
//! and outcomes (they need a window; a later version can add "and it died"
//! once the daily post exists). Design 0009 section 10 says where this is
//! weakest and what happens if the first leaderboard is a script: the rule
//! changes and the change is recorded.
//!
//! The count itself is made where the sheet is built, not here. This crate
//! sums what it is given.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::week::SECONDS_PER_DAY;

/// One summoned reply, as the hunter rule sees it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sighting {
    /// Who summoned it.
    pub summoner: String,
    /// When, as seconds since the epoch.
    pub at: u64,
    /// How many refusal signals the fact sheet carried.
    pub signals: u32,
}

/// One summoner's standing on the board.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placing {
    /// Who.
    pub summoner: String,
    /// Signals found, summed over the counted sightings.
    pub signals: u64,
    /// Sightings that counted toward the sum.
    pub counted: u64,
    /// Sightings past the daily cap, which did not.
    pub over_cap: u64,
}

/// Sums signals per summoner, counting at most `per_summoner_daily` sightings
/// per summoner per UTC day, earliest first.
///
/// Best first; ties go to the fewer sightings (more signal per look), then to
/// the name, so the order is total. A cap of zero counts nothing, which is the
/// admission gate's own reading of an unconfigured limit.
#[must_use]
pub fn tally(sightings: &[Sighting], per_summoner_daily: u32) -> Vec<Placing> {
    let mut ordered: Vec<&Sighting> = sightings.iter().collect();
    ordered.sort_by(|a, b| a.at.cmp(&b.at).then_with(|| a.summoner.cmp(&b.summoner)));

    let mut per_day: BTreeMap<(&str, u64), u32> = BTreeMap::new();
    let mut board: BTreeMap<&str, Placing> = BTreeMap::new();
    for s in ordered {
        let day = s.at / SECONDS_PER_DAY;
        let seen = per_day.entry((s.summoner.as_str(), day)).or_insert(0);
        let standing = board.entry(s.summoner.as_str()).or_insert_with(|| Placing {
            summoner: s.summoner.clone(),
            signals: 0,
            counted: 0,
            over_cap: 0,
        });
        if *seen < per_summoner_daily {
            *seen += 1;
            standing.signals = standing.signals.saturating_add(u64::from(s.signals));
            standing.counted += 1;
        } else {
            standing.over_cap += 1;
        }
    }

    let mut ranked: Vec<Placing> = board.into_values().collect();
    ranked.sort_by(|a, b| {
        b.signals
            .cmp(&a.signals)
            .then_with(|| a.counted.cmp(&b.counted))
            .then_with(|| a.summoner.cmp(&b.summoner))
    });
    ranked
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(summoner: &str, at: u64, signals: u32) -> Sighting {
        Sighting {
            summoner: summoner.to_owned(),
            at,
            signals,
        }
    }

    #[test]
    fn signals_are_summed_and_the_most_found_ranks_first() {
        let board = tally(&[seen("a", 1, 2), seen("b", 2, 5), seen("a", 3, 2)], 10);
        let names: Vec<(&str, u64)> = board
            .iter()
            .map(|s| (s.summoner.as_str(), s.signals))
            .collect();
        assert_eq!(names, [("b", 5), ("a", 4)]);
    }

    #[test]
    fn volume_cannot_win_because_the_daily_cap_applies_again_here() {
        // A script summoning fifty launches a day collects the average signal
        // density fifty times. The cap is what stops that outrunning somebody
        // who looked three times and found something each time. Re-apply the
        // bug by dropping the cap comparison and the script ranks first.
        let mut script: Vec<Sighting> = (0..50).map(|i| seen("script", i, 1)).collect();
        script.extend([
            seen("hunter", 100, 3),
            seen("hunter", 200, 3),
            seen("hunter", 300, 3),
        ]);
        let board = tally(&script, 3);
        assert_eq!(board[0].summoner, "hunter");
        assert_eq!(board[0].signals, 9);
        let script = board
            .iter()
            .find(|s| s.summoner == "script")
            .expect("on the board");
        assert_eq!(script.counted, 3, "three counted");
        assert_eq!(script.over_cap, 47, "forty-seven did not, and it says so");
        assert_eq!(script.signals, 3);
    }

    #[test]
    fn the_cap_is_per_utc_day_and_the_earliest_sightings_count() {
        // Two days, cap 1: one sighting counts on each day, and it is the
        // earliest, so a late high-signal look does not displace an earlier
        // one. Re-apply the bug by sorting descending and the 9 counts.
        let day = SECONDS_PER_DAY;
        let board = tally(
            &[
                seen("a", 10, 1),
                seen("a", 20, 9),
                seen("a", day + 10, 1),
                seen("a", day + 20, 9),
            ],
            1,
        );
        assert_eq!(board[0].signals, 2);
        assert_eq!(board[0].counted, 2);
        assert_eq!(board[0].over_cap, 2);
    }

    #[test]
    fn a_cap_of_zero_counts_nothing_which_is_the_gates_own_reading() {
        let board = tally(&[seen("a", 1, 5)], 0);
        assert_eq!(board[0].signals, 0);
        assert_eq!(board[0].over_cap, 1);
    }

    #[test]
    fn ties_go_to_fewer_looks_then_to_the_name_so_the_board_is_total() {
        let board = tally(
            &[
                seen("many", 1, 1),
                seen("many", 2, 1),
                seen("few", 3, 2),
                seen("also-few", 4, 2),
            ],
            10,
        );
        let names: Vec<&str> = board.iter().map(|s| s.summoner.as_str()).collect();
        assert_eq!(names, ["also-few", "few", "many"]);
    }

    #[test]
    fn an_empty_board_is_empty_rather_than_a_default_row() {
        assert!(tally(&[], 3).is_empty());
    }
}
