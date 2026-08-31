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
    /// The slot the token reached an AMM, if it ever did.
    ///
    /// The slot rather than a flag, because *when* is the whole signal. A token
    /// whose bonding curve was bought out in its own launch block and one that
    /// filled over three days are both `graduated = true`, and treating them as
    /// the same outcome is what made the creator signal select for the thing it
    /// was meant to avoid. See [`GraduationMode`].
    ///
    /// `None` means no graduation was recorded — which is not quite "did not
    /// graduate", since the store only knows what it has seen.
    pub graduated_at: Option<Slot>,

    /// The price of the first observed fill, in [`PRICE_SCALE`] units.
    ///
    /// Every price here is `None` for a measurement taken before prices were
    /// recorded at all, and for a token that never traded. Absent, never zero: a
    /// price of zero is a claim that the token was worthless, and "nobody
    /// measured it" is a different fact (rule 9).
    pub first_price: Option<u64>,
    /// The price of the last observed fill.
    pub last_price: Option<u64>,
    /// The highest price any fill traded at — maximum favourable excursion.
    ///
    /// The number an exit rule is fit against. A position's best possible
    /// outcome is bounded by this, and the gap between it and `last_price` is
    /// what a take-profit rule is for.
    pub peak_price: Option<u64>,
    /// The lowest price any fill traded at — maximum adverse excursion.
    ///
    /// What a stop would have had to survive. A strategy whose MAE routinely
    /// exceeds its stop is one that would have been stopped out of its winners.
    pub trough_price: Option<u64>,
    /// The highest fill price **within the most recent price window**.
    ///
    /// # Why this exists beside `peak_price`
    ///
    /// [`peak_price`](Self::peak_price) is folded from **launch**, so it can only
    /// ever widen and it says nothing about when the peak happened. A token that
    /// spiked on its first day carries that spike forever, and
    /// [`0020`](https://github.com/hey-vera/radar) could not answer whether an
    /// exit rule helps because of exactly that: counting only *new all-time*
    /// extremes left 96% of price paths looking motionless.
    ///
    /// This is the same measurement without the fold — the extreme the price
    /// query saw in the window it just read.
    ///
    /// **The window overlaps, and the name says `window` rather than `interval`
    /// for that reason.** `WINDOW_HOURS` is six and the pass runs hourly, so a
    /// peak set five hours ago appears in six consecutive measurements. It is a
    /// bounded recent lookback, not the movement since the previous checkpoint,
    /// and reading it as the latter would overstate how fresh a move is.
    ///
    /// `None` on every row written before this column existed, which is most of
    /// the store (LEARNINGS 17).
    pub window_peak_price: Option<u64>,
    /// The lowest fill price within the most recent price window.
    ///
    /// The other half of [`window_peak_price`](Self::window_peak_price), and the
    /// same caveats apply.
    pub window_trough_price: Option<u64>,
    /// Volume-weighted average price across every observed fill.
    pub vwap: Option<u64>,
    /// Fills the prices above were computed from.
    ///
    /// Carried because a peak drawn from three fills and one drawn from three
    /// hundred are different evidence, and an MFE from a single trade is a
    /// quote rather than a market.
    ///
    /// **This over-counts, and it is not a fill count.** The price windows it is
    /// folded across overlap by five of their six hours, and the fold is
    /// `saturating_add` — so a fill inside the window is counted again on every
    /// hourly pass. The number grows while nothing trades, and two measurements
    /// of the same token are not comparable.
    ///
    /// Do not read it as evidence that something changed hands; use
    /// [`last_transfer_slot`](Self::last_transfer_slot), which is a `max` and
    /// cannot be inflated by re-reading. Recorded as
    /// [`LEARNINGS`](https://github.com/hey-vera/radar/blob/main/LEARNINGS.md)
    /// entry 19, which is also where the case for changing the fold is made —
    /// changing it would make new rows incomparable with every row already
    /// written, so it is a decision rather than a patch.
    pub fills: u64,
}

/// Prices are lamports per base unit, scaled by 10^18.
///
/// Integer, because a threshold compared as a float compares differently on a
/// replay. The scale is large because the quantity is small: a pump.fun token
/// around launch trades near 2e-5 lamports per base unit, which is 21,002,820
/// billion at this scale and nowhere near overflowing a `u64`.
///
/// The ceiling is ~18 lamports per base unit, which a six-decimal token would
/// reach at roughly 18 billion lamports per whole token. Conversion saturates
/// rather than wrapping, so a token past that reads as implausibly expensive
/// instead of implausibly cheap.
pub const PRICE_SCALE: u128 = 1_000_000_000_000_000_000;

impl Outcome {
    /// Maximum favourable excursion against the first observed price, in bps.
    ///
    /// `None` unless both ends were measured. The headline number for fitting a
    /// take-profit: how far the price ever went in the holder's favour.
    #[must_use]
    pub fn mfe_bps(&self) -> Option<u64> {
        excursion_bps(self.first_price?, self.peak_price?)
    }

    /// Maximum adverse excursion against the first observed price, in bps.
    ///
    /// Returned as a positive magnitude of the fall. `None` unless both ends
    /// were measured — a token nobody priced has no drawdown, rather than none.
    #[must_use]
    pub fn mae_bps(&self) -> Option<u64> {
        let (first, trough) = (self.first_price?, self.trough_price?);
        if first == 0 {
            return None;
        }
        Some(
            u64::try_from(u128::from(first.saturating_sub(trough)) * 10_000 / u128::from(first))
                .unwrap_or(u64::MAX),
        )
    }

    /// Whether a position opened at the first price and held to the last one
    /// would have ended up ahead, before costs.
    ///
    /// Before costs on purpose, and named so. Round-trip cost is the strategy's
    /// to apply; folding an assumed one in here would bake a placeholder into
    /// the measurement it is supposed to be judged against.
    #[must_use]
    pub fn held_to_end_gain_bps(&self) -> Option<i64> {
        let (first, last) = (self.first_price?, self.last_price?);
        if first == 0 {
            return None;
        }
        let first = i128::from(first);
        let last = i128::from(last);
        i64::try_from((last - first) * 10_000 / first).ok()
    }
}

/// Rise from `from` to `to`, in basis points. `None` if the base is zero.
fn excursion_bps(from: u64, to: u64) -> Option<u64> {
    if from == 0 {
        return None;
    }
    Some(
        u64::try_from(u128::from(to.saturating_sub(from)) * 10_000 / u128::from(from))
            .unwrap_or(u64::MAX),
    )
}

/// How a token reached the AMM.
///
/// Measured, not assumed. Over 44 graduations with a recoverable subject mint,
/// slots from first transfer to migration ran `min 0, median 828, p90 274,163,
/// max 809,491` — a hard spike at zero and then a broad tail. The distribution
/// is bimodal, and the two modes are different events wearing one label.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraduationMode {
    /// The bonding curve completed within a few slots of the launch.
    ///
    /// Nobody discovers a token, decides, and buys out its entire curve inside
    /// a block. Capital that size arriving that fast was committed before the
    /// token existed, which makes this the signature of a bundled launch rather
    /// than of demand. 12 of 44 measured graduations were at *exactly* zero
    /// slots — the same block as the first transfer.
    ///
    /// It is named for what was observed rather than for what it implies:
    /// "instant" is a measurement, "bundled" would be a verdict.
    Instant,
    /// The curve filled over time, from buyers arriving separately.
    Organic,
}

/// Slots within which a graduation counts as [`GraduationMode::Instant`].
///
/// Three, not zero. The launch and the buy-out can straddle a slot boundary
/// without being any less simultaneous in intent, and the measured distribution
/// has nothing between 0 and 25 — so the threshold sits inside a gap rather than
/// on a slope, and moving it anywhere in that range changes no answer.
pub const INSTANT_WITHIN_SLOTS: u64 = 3;

impl Outcome {
    /// Whether the token reached an AMM at all.
    #[must_use]
    pub const fn graduated(&self) -> bool {
        self.graduated_at.is_some()
    }

    /// How the token graduated, or `None` if it did not.
    ///
    /// Deliberately not defaulting to [`GraduationMode::Organic`] for a token
    /// that never graduated: absent is not zero, and a non-event has no mode.
    #[must_use]
    pub const fn graduation_mode(&self) -> Option<GraduationMode> {
        let Some(at) = self.graduated_at else {
            return None;
        };
        Some(
            if at.get().saturating_sub(self.launch_slot.get()) <= INSTANT_WITHIN_SLOTS {
                GraduationMode::Instant
            } else {
                GraduationMode::Organic
            },
        )
    }

    /// Slots from launch to graduation, if it graduated.
    #[must_use]
    pub const fn slots_to_graduate(&self) -> Option<u64> {
        let Some(at) = self.graduated_at else {
            return None;
        };
        Some(at.get().saturating_sub(self.launch_slot.get()))
    }

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
            graduated_at: None,
            first_price: None,
            last_price: None,
            peak_price: None,
            trough_price: None,
            window_peak_price: None,
            window_trough_price: None,
            vwap: None,
            fills: 0,
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

    /// The outcome of a token that graduated `after` slots past its launch.
    fn graduated(after: u64) -> Outcome {
        Outcome {
            graduated_at: Some(Slot(1_000 + after)),
            ..outcome(500, 1_000, Some(1_000 + after))
        }
    }

    #[test]
    fn a_token_that_never_graduated_has_no_mode_rather_than_a_default_one() {
        // Absent is not zero. Defaulting a non-event to Organic would put every
        // dead token into the population the signal is trying to select for.
        let never = outcome(10, 1_000, Some(2_000));
        assert!(!never.graduated());
        assert_eq!(never.graduation_mode(), None);
        assert_eq!(never.slots_to_graduate(), None);
    }

    #[test]
    fn a_curve_bought_out_in_the_launch_block_is_instant() {
        // 12 of 44 measured graduations were at exactly zero slots. Nobody
        // discovers a token and buys out its whole curve inside one block.
        let same_block = graduated(0);
        assert!(same_block.graduated());
        assert_eq!(same_block.slots_to_graduate(), Some(0));
        assert_eq!(same_block.graduation_mode(), Some(GraduationMode::Instant));
    }

    #[test]
    fn the_boundary_sits_inside_the_gap_the_measurements_left() {
        // The measured distribution jumps from 0 straight to 25, so the exact
        // threshold is unobservable anywhere in between -- which is the point of
        // putting it there. These two assertions pin both sides of it.
        assert_eq!(
            graduated(INSTANT_WITHIN_SLOTS).graduation_mode(),
            Some(GraduationMode::Instant)
        );
        assert_eq!(
            graduated(INSTANT_WITHIN_SLOTS + 1).graduation_mode(),
            Some(GraduationMode::Organic)
        );
    }

    #[test]
    fn the_real_median_graduation_is_organic() {
        // The measured median was 828 slots, about five minutes -- a curve that
        // filled from buyers arriving separately.
        let median = graduated(828);
        assert_eq!(median.graduation_mode(), Some(GraduationMode::Organic));
        assert_eq!(median.slots_to_graduate(), Some(828));
    }

    #[test]
    fn a_graduation_recorded_before_its_launch_is_zero_rather_than_wrapping() {
        // Saturating, so a clock or ordering fault cannot produce a duration of
        // eighteen quintillion slots and read as the most organic token on file.
        let impossible = Outcome {
            graduated_at: Some(Slot(500)),
            ..outcome(10, 1_000, Some(1_000))
        };
        assert_eq!(impossible.slots_to_graduate(), Some(0));
        assert_eq!(impossible.graduation_mode(), Some(GraduationMode::Instant));
    }
}
