// SPDX-License-Identifier: Apache-2.0
//! What Radar holds, and what it held.
//!
//! # Why this exists before anything can trade
//!
//! `PortfolioState` is the kernel's view of the world, and until now every
//! caller built it with `PortfolioState::flat()` — no exposure, no realised
//! loss, nothing held. That was true, because nothing has ever traded. It was
//! also a **lie waiting to happen**: the day something does trade, `flat()`
//! keeps returning the same answer and every limit the kernel enforces is
//! measured against zero.
//!
//! A position limit checked against a portfolio that is always empty is not a
//! limit. So the record comes first, and the reconstruction with it, while the
//! answer is still trivially verifiable.
//!
//! # Append-only, like everything else here
//!
//! A position is not updated in place. Opening writes one row and closing writes
//! another with the same `opened_at`, which is what makes the history readable
//! at any watermark — the same point-in-time guarantee the rest of the store
//! gives (AGENTS.md rule 3). A mutable row would answer "what did Radar hold on
//! Tuesday" with today's state.

use radar_types::{Address, Slot};
use serde::{Deserialize, Serialize};

/// One position, as opened or as closed.
///
/// The pair `(mint, opened_at)` identifies it. A row with `closed_at` set
/// supersedes the row without, and reading folds them together.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Position {
    /// Which token.
    pub mint: Address,
    /// Who created it, carried so exposure can be aggregated by creator without
    /// a second lookup — the same reason [`radar_risk::Proposal`] carries it.
    ///
    /// [`radar_risk::Proposal`]: https://github.com/hey-vera/radar
    pub creator: Address,
    /// When it was opened. Half of the identity.
    pub opened_at: Slot,
    /// What was committed, in micro-dollars.
    pub notional_micro_usd: u64,
    /// The price paid, scaled the way outcomes are.
    ///
    /// `None` when the fill price was not recorded, which is a gap rather than
    /// a price of zero — a position whose entry is unknown cannot have its
    /// return computed, and reporting it as zero would report a total loss.
    pub entry_price: Option<u64>,
    /// When it was closed, if it has been.
    ///
    /// `None` is an **open** position, which is the load-bearing reading: an
    /// open position counts against every exposure limit.
    pub closed_at: Option<Slot>,
    /// The price it was closed at.
    pub exit_price: Option<u64>,
    /// What the round trip made or lost, in micro-dollars, signed.
    ///
    /// `None` on an open position and on a closed one whose prices were not
    /// both recorded. Never zero as a stand-in: zero is a round trip that broke
    /// exactly even, and a realised-loss limit reading an unknown as zero is a
    /// limit that never binds.
    pub realised_micro_usd: Option<i64>,
}

impl Position {
    /// Whether this position is still held.
    #[must_use]
    pub const fn is_open(&self) -> bool {
        self.closed_at.is_none()
    }

    /// What this position lost, in micro-dollars, or zero if it did not lose.
    ///
    /// Losses only, because that is what the kernel's daily limit is about. A
    /// profitable day does not buy permission to lose more, so gains are not
    /// netted off — a netting version would let one large winner fund several
    /// losers inside the same ceiling.
    #[must_use]
    pub fn loss_micro_usd(&self) -> u64 {
        self.realised_micro_usd
            .filter(|r| *r < 0)
            .and_then(i64::checked_neg)
            .and_then(|r| u64::try_from(r).ok())
            .unwrap_or_default()
    }
}

/// Folds the recorded rows into the current state of each position.
///
/// Later rows for the same `(mint, opened_at)` supersede earlier ones, which is
/// how a close replaces an open in an append-only store. The result is one row
/// per position, in the order they were opened.
///
/// Separate from the read on purpose. A reader that resolved supersession as it
/// went would make "what did Radar hold on Tuesday" unanswerable without
/// re-reading — the fold is a view of the rows, not a replacement for them.
#[must_use]
pub fn fold_positions(rows: Vec<Position>) -> Vec<Position> {
    let mut latest: std::collections::BTreeMap<(radar_types::Address, Slot), Position> =
        std::collections::BTreeMap::new();
    for row in rows {
        let key = (row.mint, row.opened_at);
        match latest.get(&key) {
            // A close supersedes an open. An open never supersedes a close:
            // rows can arrive in any order across files, and letting the open
            // win would resurrect a position that had been shut.
            Some(existing) if !existing.is_open() && row.is_open() => {}
            _ => {
                latest.insert(key, row);
            }
        }
    }
    let mut out: Vec<Position> = latest.into_values().collect();
    out.sort_by_key(|p| (p.opened_at, p.mint));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(mint: u8) -> Position {
        Position {
            mint: Address::new([mint; 32]),
            creator: Address::new([200u8; 32]),
            opened_at: Slot(10_000),
            notional_micro_usd: 5_000_000,
            entry_price: Some(1_000),
            closed_at: None,
            exit_price: None,
            realised_micro_usd: None,
        }
    }

    #[test]
    fn an_unclosed_position_is_open_and_that_is_what_limits_read() {
        // The load-bearing reading. An open position counts against every
        // exposure limit, so `closed_at: None` must not be mistaken for a
        // position that no longer exists.
        assert!(open(1).is_open());

        let mut closed = open(1);
        closed.closed_at = Some(Slot(12_000));
        assert!(!closed.is_open());
    }

    #[test]
    fn an_unknown_realised_amount_is_not_a_loss_of_zero() {
        // Rule 9. A daily-loss limit that read every unknown as zero would be a
        // limit that never binds, on exactly the days the record is worst.
        let mut position = open(1);
        position.closed_at = Some(Slot(12_000));
        position.realised_micro_usd = None;
        assert_eq!(position.loss_micro_usd(), 0, "unknown contributes nothing");
        assert_eq!(position.realised_micro_usd, None, "and stays unknown");
    }

    #[test]
    fn gains_are_not_netted_against_losses() {
        // A profitable day does not buy permission to lose more. Netting would
        // let one large winner fund several losers inside the same ceiling,
        // which is the opposite of what a daily-loss limit is for.
        let mut winner = open(1);
        winner.realised_micro_usd = Some(4_000_000);
        assert_eq!(winner.loss_micro_usd(), 0);

        let mut loser = open(2);
        loser.realised_micro_usd = Some(-3_000_000);
        assert_eq!(loser.loss_micro_usd(), 3_000_000);
    }

    #[test]
    fn a_break_even_round_trip_is_not_a_loss() {
        let mut flat = open(1);
        flat.realised_micro_usd = Some(0);
        assert_eq!(flat.loss_micro_usd(), 0);
    }

    #[test]
    fn the_most_negative_amount_representable_does_not_wrap_into_a_gain() {
        // `i64::MIN` has no positive counterpart, so negating it overflows. In
        // release builds that wraps back to `i64::MIN` and a `as u64` cast would
        // turn the largest possible loss into a number that reads as one --
        // which is the direction that loses money.
        let mut extreme = open(1);
        extreme.realised_micro_usd = Some(i64::MIN);
        assert_eq!(
            extreme.loss_micro_usd(),
            0,
            "unrepresentable is dropped, never wrapped"
        );

        let mut large = open(2);
        large.realised_micro_usd = Some(i64::MIN + 1);
        assert_eq!(large.loss_micro_usd(), i64::MAX as u64);
    }

    #[test]
    fn a_close_supersedes_the_open_it_shares_an_identity_with() {
        // Append-only: opening writes a row and closing writes another with the
        // same `(mint, opened_at)`. The fold has to resolve them, or a closed
        // position keeps counting against every exposure limit forever.
        let opened = open(1);
        let mut closed = open(1);
        closed.closed_at = Some(Slot(12_000));
        closed.realised_micro_usd = Some(-650_000);

        let folded = fold_positions(vec![opened, closed.clone()]);
        assert_eq!(folded, vec![closed]);
    }

    #[test]
    fn an_open_row_arriving_late_does_not_resurrect_a_closed_position() {
        // Rows come from several files and arrive in no guaranteed order. If a
        // late open could supersede a close, a position that had been shut
        // would come back -- and it would come back counting against limits, on
        // a portfolio that does not hold it.
        let opened = open(1);
        let mut closed = open(1);
        closed.closed_at = Some(Slot(12_000));

        let folded = fold_positions(vec![closed.clone(), opened]);
        assert_eq!(folded, vec![closed], "the close wins whatever the order");
    }

    #[test]
    fn positions_in_different_mints_are_not_folded_together() {
        let a = open(1);
        let b = open(2);
        assert_eq!(fold_positions(vec![a.clone(), b.clone()]).len(), 2);

        // Nor are two positions in the same mint opened at different slots --
        // buying the same token twice is two positions, and folding them would
        // halve the recorded exposure.
        let mut later = open(1);
        later.opened_at = Slot(11_000);
        assert_eq!(fold_positions(vec![a, later]).len(), 2);
    }

    #[test]
    fn folding_nothing_yields_nothing() {
        assert!(fold_positions(Vec::new()).is_empty());
    }

    #[test]
    fn a_position_survives_a_round_trip_through_json() {
        let mut closed = open(1);
        closed.closed_at = Some(Slot(12_000));
        closed.exit_price = Some(870);
        closed.realised_micro_usd = Some(-650_000);

        let json = serde_json::to_string(&closed).expect("serialises");
        let back: Position = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(back, closed);
    }
}
