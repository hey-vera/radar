// SPDX-License-Identifier: Apache-2.0
//! Rebuilding the kernel's view of the portfolio from what was recorded.
//!
//! Lives here because it is the one crate that depends on both the store and
//! the kernel: `radar-risk` owns [`PortfolioState`] and must stay pure, and
//! `radar-store` must not know what a risk kernel is.
//!
//! # The gap this closes
//!
//! Every caller built the kernel's state with `PortfolioState::flat()`. That
//! was *true* — nothing has ever traded — and it was a lie waiting to happen:
//! the day something does trade, `flat()` returns the same answer and every
//! limit the kernel enforces is measured against an empty portfolio.
//!
//! A position limit checked against a portfolio that is always empty is not a
//! limit. AGENTS.md lists this as a capability gap in as many words: *nothing
//! persists a position; if Radar bought, it would not know it held.*
//!
//! # What this can and cannot reconstruct
//!
//! Positions answer three of the six fields: what is deployed, what is deployed
//! per creator, and what was realised as loss today.
//!
//! They cannot answer the other two. `consecutive_failures` is a property of
//! *execution* — transactions that did not land — and nothing records those
//! yet. `halted` is a switch an operator throws.
//!
//! So this function **takes them as arguments** rather than defaulting them.
//! Defaulting `halted` to `false` and failures to `0` would be the permissive
//! answer arriving silently from a component that does not know, which is the
//! exact shape of rule 9. A caller has to say what it believes, and can be
//! grepped for.

use std::collections::BTreeMap;

use radar_risk::PortfolioState;
use radar_store::Position;
use radar_types::{Address, MicroUsd, Slot};

/// Slots in a day, at roughly 400ms each.
///
/// An approximation, and the daily-loss window is the only thing that uses it.
/// Solana's slot time drifts with network conditions, so this is a *nominal*
/// day rather than a wall-clock one — which is the right unit here anyway,
/// because every other input the kernel takes is measured in slots and mixing
/// the two would make a replay depend on when it was run.
pub const SLOTS_PER_DAY: u64 = 216_000;

/// What an operator has told the system, which positions cannot say.
///
/// A struct rather than two loose arguments so that adding a third thing the
/// portfolio cannot know does not silently reorder a call site.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Operator {
    /// Whether trading is halted.
    ///
    /// No default. A kill switch that defaults to off is one that is off
    /// whenever nobody remembered to set it.
    pub halted: bool,
    /// Transactions that failed in a row.
    ///
    /// Zero is the honest answer only while nothing executes. When an execution
    /// record exists this comes from it, and until then a caller passing zero is
    /// saying "nothing has executed", which is true.
    pub consecutive_failures: u32,
}

/// Rebuilds the kernel's state from recorded positions.
///
/// `now` is the watermark the decision is being taken at, and it is what the
/// daily-loss window is measured back from — not a clock. That is what keeps a
/// replay reproducing the original verdict (AGENTS.md rule 2).
///
/// Expects rows already folded by [`radar_store::fold_positions`]: one row per
/// position, closes having superseded opens. Handed raw rows it would count an
/// opened-then-closed position twice, once as open.
#[must_use]
pub fn state_from(positions: &[Position], now: Slot, operator: Operator) -> PortfolioState {
    let mut deployed: u64 = 0;
    let mut per_creator: BTreeMap<Address, MicroUsd> = BTreeMap::new();
    let mut realised_loss: u64 = 0;

    let day_began = now.get().saturating_sub(SLOTS_PER_DAY);

    for position in positions {
        if position.is_open() {
            // Saturating, not wrapping. A portfolio whose total exposure
            // overflowed would come back as a small number, and small is the
            // direction that gets permission.
            deployed = deployed.saturating_add(position.notional_micro_usd);
            let entry = per_creator
                .entry(position.creator)
                .or_insert(MicroUsd::ZERO);
            *entry = MicroUsd(entry.get().saturating_add(position.notional_micro_usd));
            continue;
        }

        // Closed. It contributes to the day's realised loss if it closed inside
        // the window, and to nothing else — a closed position is not exposure.
        let closed_inside_the_day = position
            .closed_at
            .is_some_and(|closed| closed.get() >= day_began && closed <= now);
        if closed_inside_the_day {
            realised_loss = realised_loss.saturating_add(position.loss_micro_usd());
        }
    }

    PortfolioState {
        now,
        deployed: MicroUsd(deployed),
        per_creator,
        realised_loss_today: MicroUsd(realised_loss),
        consecutive_failures: operator.consecutive_failures,
        halted: operator.halted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: Slot = Slot(1_000_000);

    fn quiet() -> Operator {
        Operator {
            halted: false,
            consecutive_failures: 0,
        }
    }

    fn open(mint: u8, creator: u8, notional: u64) -> Position {
        Position {
            mint: Address::new([mint; 32]),
            creator: Address::new([creator; 32]),
            opened_at: Slot(999_000),
            notional_micro_usd: notional,
            entry_price: Some(1_000),
            closed_at: None,
            exit_price: None,
            realised_micro_usd: None,
        }
    }

    fn closed(mint: u8, creator: u8, at: u64, realised: i64) -> Position {
        let mut position = open(mint, creator, 5_000_000);
        position.closed_at = Some(Slot(at));
        position.exit_price = Some(870);
        position.realised_micro_usd = Some(realised);
        position
    }

    #[test]
    fn an_empty_portfolio_is_flat_and_that_is_the_only_time_flat_is_right() {
        let state = state_from(&[], NOW, quiet());
        assert_eq!(state, PortfolioState::flat(NOW));
    }

    #[test]
    fn open_positions_are_exposure_and_closed_ones_are_not() {
        // The whole point. A closed position counting as exposure would refuse
        // trades against capital that is no longer committed; an open one not
        // counting is a limit measured against an empty portfolio.
        let state = state_from(
            &[
                open(1, 100, 3_000_000),
                open(2, 100, 2_000_000),
                closed(3, 100, 999_500, -400_000),
            ],
            NOW,
            quiet(),
        );

        assert_eq!(state.deployed, MicroUsd(5_000_000), "the two open ones");
        assert_eq!(
            state.creator_exposure(&Address::new([100u8; 32])),
            MicroUsd(5_000_000)
        );
    }

    #[test]
    fn exposure_is_aggregated_per_creator() {
        // The limit exists because one creator's tokens fail together, so two
        // positions in different mints from one creator are one concentration.
        let state = state_from(
            &[open(1, 100, 3_000_000), open(2, 200, 2_000_000)],
            NOW,
            quiet(),
        );
        assert_eq!(
            state.creator_exposure(&Address::new([100u8; 32])),
            MicroUsd(3_000_000)
        );
        assert_eq!(
            state.creator_exposure(&Address::new([200u8; 32])),
            MicroUsd(2_000_000)
        );
        // And a creator with no position has no exposure rather than an entry
        // of zero, which is what `creator_exposure` already promises.
        assert_eq!(
            state.creator_exposure(&Address::new([9u8; 32])),
            MicroUsd::ZERO
        );
    }

    #[test]
    fn only_losses_closed_inside_the_day_count_against_the_daily_limit() {
        // The window is measured back from the watermark, not from a clock --
        // which is what lets a replay reproduce the original verdict.
        let inside = closed(1, 100, NOW.get() - 1_000, -400_000);
        let outside = closed(2, 100, NOW.get() - SLOTS_PER_DAY - 1, -900_000);

        let state = state_from(&[inside, outside], NOW, quiet());
        assert_eq!(
            state.realised_loss_today,
            MicroUsd(400_000),
            "yesterday's loss is not today's"
        );
    }

    #[test]
    fn the_day_boundary_is_inclusive_at_its_start() {
        // A loss closed exactly a day ago is inside the day. Exclusive, the
        // limit would forget a loss one slot early -- every day, forever.
        let edge = closed(1, 100, NOW.get() - SLOTS_PER_DAY, -400_000);
        assert_eq!(
            state_from(&[edge], NOW, quiet()).realised_loss_today,
            MicroUsd(400_000)
        );
    }

    #[test]
    fn a_position_closed_after_the_watermark_is_not_counted() {
        // Rule 3. A close that happened after the watermark had not happened as
        // of it, and counting it would let a decision be informed by its own
        // future.
        let ahead = closed(1, 100, NOW.get() + 1, -400_000);
        assert_eq!(
            state_from(&[ahead], NOW, quiet()).realised_loss_today,
            MicroUsd::ZERO
        );
    }

    #[test]
    fn gains_do_not_offset_losses_in_the_daily_total() {
        // A profitable trade does not buy permission to lose more. Netting
        // would let one winner fund several losers inside the same ceiling.
        let state = state_from(
            &[
                closed(1, 100, NOW.get() - 1_000, 5_000_000),
                closed(2, 100, NOW.get() - 1_000, -400_000),
            ],
            NOW,
            quiet(),
        );
        assert_eq!(state.realised_loss_today, MicroUsd(400_000));
    }

    #[test]
    fn what_positions_cannot_say_is_supplied_rather_than_defaulted() {
        // Rule 9. `halted: false` arriving silently from a component that does
        // not know is the permissive answer by omission, and a kill switch that
        // defaults to off is off whenever nobody remembered.
        let halted = state_from(
            &[],
            NOW,
            Operator {
                halted: true,
                consecutive_failures: 3,
            },
        );
        assert!(halted.halted);
        assert_eq!(halted.consecutive_failures, 3);
    }

    #[test]
    fn an_absurd_total_saturates_rather_than_wrapping_into_permission() {
        // Two positions at `u64::MAX` cannot happen and the arithmetic must not
        // pretend otherwise: wrapping would return a small deployed figure, and
        // small is the direction that gets permission.
        let state = state_from(
            &[open(1, 100, u64::MAX), open(2, 100, u64::MAX)],
            NOW,
            quiet(),
        );
        assert_eq!(state.deployed, MicroUsd(u64::MAX));
        assert_eq!(
            state.creator_exposure(&Address::new([100u8; 32])),
            MicroUsd(u64::MAX)
        );
    }
}
