// SPDX-License-Identifier: Apache-2.0
//! Outcome measurements: what happened to a token after it launched.
//!
//! Outcomes are what every signal has to be validated against. Without them
//! Radar records launches and can never say whether anything it noticed predicts
//! anything — which is the difference between a recorder and the thing this
//! project exists to build.
//!
//! **An outcome is an observation, not a fact.** A token measured an hour after
//! launch and the same token measured a week later give different answers, and
//! both are correct about their own moment. So every measurement carries the slot
//! it was taken at, and a later measurement is a new row rather than an update.
//! Overwriting would quietly destroy the ability to ask "what did this look like
//! at the point a decision was made", which is the only question a backtest is
//! allowed to ask.

use radar_types::{Address, Slot};
use serde::{Deserialize, Serialize};

/// What became of a token, as measured at a particular slot.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Outcome {
    /// The token.
    pub mint: Address,
    /// The slot this measurement was taken at.
    ///
    /// Load-bearing. A measurement without one cannot be admitted through a
    /// watermark, so it could never be used in a replay.
    pub measured_at: Slot,
    /// The slot the token was created in.
    pub launch_slot: Slot,
    /// First observed transfer.
    pub first_transfer_slot: Option<Slot>,
    /// Last observed transfer.
    pub last_transfer_slot: Option<Slot>,
    /// Transfers observed.
    pub transfers: u64,
    /// Distinct sending token accounts.
    ///
    /// Token accounts rather than owners — the transfer table names accounts,
    /// and resolving them to owners is a separate join. An upper bound on
    /// distinct participants, and named as accounts so nobody reads it as people.
    pub unique_senders: u64,
    /// Distinct receiving token accounts.
    pub unique_receivers: u64,
    /// Whether the token reached an AMM.
    pub graduated: bool,
}

impl Outcome {
    /// Slots between launch and the last observed transfer.
    ///
    /// The bluntest useful label: a token that traded for four slots and one
    /// that traded for four hundred thousand are different things, and the gap
    /// between them is visible without any pricing at all.
    #[must_use]
    pub fn survived_slots(&self) -> u64 {
        self.last_transfer_slot
            .map_or(0, |last| last.get().saturating_sub(self.launch_slot.get()))
    }

    /// Whether the token showed no life beyond its launch.
    ///
    /// Deliberately conservative: a handful of transfers within a couple of
    /// minutes is the signature of a launch nobody engaged with. It is a
    /// description, not a verdict — whether it predicts anything is a question
    /// for the research store, and calling it "rug" here would be assuming the
    /// answer.
    #[must_use]
    pub fn appears_stillborn(&self) -> bool {
        self.transfers <= 5 && self.survived_slots() < 300
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(transfers: u64, launch: u64, last: Option<u64>) -> Outcome {
        Outcome {
            mint: Address::new([1; 32]),
            measured_at: Slot(500_000),
            launch_slot: Slot(launch),
            first_transfer_slot: last.map(|_| Slot(launch)),
            last_transfer_slot: last.map(Slot),
            transfers,
            unique_senders: 3,
            unique_receivers: 2,
            graduated: false,
        }
    }

    #[test]
    fn survival_is_measured_from_launch_not_from_the_first_trade() {
        // A token whose first trade came late was still dead in between, and
        // measuring from the first trade would hide exactly that.
        let o = outcome(10, 1_000, Some(5_000));
        assert_eq!(o.survived_slots(), 4_000);
    }

    #[test]
    fn a_token_that_never_traded_survived_nothing() {
        assert_eq!(outcome(0, 1_000, None).survived_slots(), 0);
    }

    #[test]
    fn the_real_examples_separate_cleanly() {
        // Both measured from mainnet. One traded for four slots with three
        // transfers; the other for 374,582 slots with 1,535.
        let dead = outcome(3, 440_624_864, Some(440_624_868));
        let alive = outcome(1_535, 440_623_612, Some(440_998_194));

        assert_eq!(dead.survived_slots(), 4);
        assert!(dead.appears_stillborn());

        assert_eq!(alive.survived_slots(), 374_582);
        assert!(!alive.appears_stillborn());
    }

    #[test]
    fn a_quiet_but_long_lived_token_is_not_called_stillborn() {
        // Few transfers spread over hours is a different thing from few
        // transfers in seconds, and conflating them would label slow tokens as
        // dead ones.
        let quiet = outcome(4, 1_000, Some(50_000));
        assert!(!quiet.appears_stillborn());
    }

    #[test]
    fn a_busy_but_brief_token_is_not_called_stillborn_either() {
        // Heavy trading that stops abruptly is a rug, not a stillbirth, and the
        // two want different labels.
        let brief = outcome(900, 1_000, Some(1_100));
        assert!(!brief.appears_stillborn());
    }
}
