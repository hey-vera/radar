// SPDX-License-Identifier: Apache-2.0
//! The economics gate.
//!
//! Separate from the risk kernel and asked after it. The kernel answers "are we
//! allowed to do this"; this answers "is it worth doing". A trade can be
//! perfectly within policy and still lose money on costs alone, and at the sizes
//! Radar will trade that is the *usual* case rather than the exception.
//!
//! ```text
//! expected_net = edge
//!              − price_impact
//!              − slippage
//!              − dex_fee
//!              − priority_fee
//!              − tip
//!              − p(fail) × fail_cost
//! ```
//!
//! Refuse if negative. At small sizes this alone rejects most trades, and that
//! is the mechanism working rather than a threshold set too high — the base-rate
//! research says roughly 96% of pump.fun wallets lost money or made under $500,
//! and costs are a large part of why.
//!
//! Everything here is integer micro-USD and pure. Same inputs, same answer, no
//! clock, so a recorded decision replays.

use radar_types::MicroUsd;

/// What a round trip is expected to cost, itemised.
///
/// Itemised rather than totalled because the point of measuring costs is to find
/// out which one dominates. A single number would say a trade was too expensive
/// without saying what to fix.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Costs {
    /// Price impact on the way in and out, from the measured sell curve.
    pub price_impact: MicroUsd,
    /// Slippage beyond the quote, from the calibrated fill model.
    pub slippage: MicroUsd,
    /// The venue's fee, both legs.
    pub dex_fee: MicroUsd,
    /// Solana priority fee, both legs.
    pub priority_fee: MicroUsd,
    /// Jito or Sender tip, both legs.
    ///
    /// Modelled as an input, never a constant. Tip volume fell roughly half
    /// between Q1 and Q2 2026, so a figure hard-coded from a busy period would
    /// systematically overstate costs — and one from a quiet period would
    /// understate them at exactly the moment they matter.
    pub tip: MicroUsd,
}

impl Costs {
    /// Everything except the failure term.
    #[must_use]
    pub fn certain(&self) -> MicroUsd {
        MicroUsd(
            self.price_impact
                .get()
                .saturating_add(self.slippage.get())
                .saturating_add(self.dex_fee.get())
                .saturating_add(self.priority_fee.get())
                .saturating_add(self.tip.get()),
        )
    }

    /// The largest single line, and what it is.
    ///
    /// What an operator actually wants when a trade is refused on cost.
    #[must_use]
    pub fn dominant(&self) -> (&'static str, MicroUsd) {
        [
            ("price_impact", self.price_impact),
            ("slippage", self.slippage),
            ("dex_fee", self.dex_fee),
            ("priority_fee", self.priority_fee),
            ("tip", self.tip),
        ]
        .into_iter()
        // max_by_key returns the last maximum; ties resolve to the later item,
        // which keeps the answer stable rather than dependent on iteration luck.
        .max_by_key(|(_, amount)| amount.get())
        .unwrap_or(("none", MicroUsd::ZERO))
    }
}

/// A failed transaction still costs its fee.
///
/// Named separately from [`Costs`] because it is the only probabilistic term,
/// and mixing a certainty with an expectation in one struct is how the two get
/// confused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FailureRisk {
    /// Probability of the transaction failing, per ten thousand.
    ///
    /// Measured from recorded submissions, not assumed. Radar's own landed and
    /// failed counts are the only honest source for this number.
    pub probability_bps: u64,
    /// What a failure costs: the fee and tip that are spent anyway.
    pub cost: MicroUsd,
}

impl FailureRisk {
    /// The expected cost of failure.
    #[must_use]
    pub const fn expected(&self) -> MicroUsd {
        MicroUsd(self.cost.get().saturating_mul(self.probability_bps) / 10_000)
    }
}

/// What the gate decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Economics {
    /// Worth doing, by this margin.
    Worthwhile {
        /// Expected profit after every cost.
        expected_net: MicroUsd,
    },
    /// Not worth doing.
    Uneconomic {
        /// What the whole round trip is expected to cost.
        total_cost: MicroUsd,
        /// The edge that would have had to exist for it to break even.
        breakeven_edge: MicroUsd,
        /// The largest single cost line.
        dominant_cost: &'static str,
    },
}

impl Economics {
    /// Whether the trade should proceed.
    #[must_use]
    pub const fn is_worthwhile(&self) -> bool {
        matches!(self, Self::Worthwhile { .. })
    }
}

/// Decides whether a round trip pays for itself.
///
/// `edge` is the expected gross return, in the same units as the costs. Where it
/// comes from is the strategy's problem; whether it survives contact with the
/// cost model is this function's.
#[must_use]
pub fn evaluate(edge: MicroUsd, costs: &Costs, failure: FailureRisk) -> Economics {
    let total = MicroUsd(
        costs
            .certain()
            .get()
            .saturating_add(failure.expected().get()),
    );

    if edge.get() > total.get() {
        Economics::Worthwhile {
            expected_net: MicroUsd(edge.get() - total.get()),
        }
    } else {
        Economics::Uneconomic {
            total_cost: total,
            breakeven_edge: total,
            dominant_cost: costs.dominant().0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cents(n: u64) -> MicroUsd {
        MicroUsd(n * 10_000)
    }

    fn typical() -> Costs {
        Costs {
            price_impact: cents(30),
            slippage: cents(15),
            dex_fee: cents(20),
            priority_fee: cents(5),
            tip: cents(10),
        }
    }

    const RELIABLE: FailureRisk = FailureRisk {
        probability_bps: 500,
        cost: MicroUsd(150_000),
    };

    #[test]
    fn a_trade_whose_edge_clears_its_costs_proceeds() {
        let verdict = evaluate(MicroUsd::from_dollars(5.0), &typical(), RELIABLE);
        assert!(verdict.is_worthwhile(), "{verdict:?}");
    }

    #[test]
    fn a_trade_whose_edge_does_not_clear_its_costs_is_refused() {
        // The base rate says this is the common case, not the exception. A gate
        // that rarely fired would be a gate set to agree with whoever built it.
        let verdict = evaluate(cents(50), &typical(), RELIABLE);
        assert!(!verdict.is_worthwhile(), "{verdict:?}");
    }

    #[test]
    fn breaking_exactly_even_is_refused() {
        // Zero expected profit for real risk is a losing trade with extra steps.
        let costs = Costs {
            price_impact: MicroUsd::from_dollars(1.0),
            ..Costs::default()
        };
        let no_failures = FailureRisk {
            probability_bps: 0,
            cost: MicroUsd::ZERO,
        };
        assert!(!evaluate(MicroUsd::from_dollars(1.0), &costs, no_failures).is_worthwhile());
    }

    #[test]
    fn the_failure_term_can_be_what_refuses_a_trade() {
        // The term most cost models leave out, and the one that dominates when
        // the chain is congested — which is exactly when a launch is worth
        // trading.
        let costs = Costs {
            dex_fee: cents(10),
            ..Costs::default()
        };
        let edge = MicroUsd::from_dollars(1.0);
        let calm = FailureRisk {
            probability_bps: 100,
            cost: MicroUsd::from_dollars(0.5),
        };
        let congested = FailureRisk {
            probability_bps: 9_000,
            cost: MicroUsd::from_dollars(2.0),
        };
        assert!(evaluate(edge, &costs, calm).is_worthwhile());
        assert!(!evaluate(edge, &costs, congested).is_worthwhile());
    }

    #[test]
    fn a_refusal_names_the_cost_that_caused_it() {
        // "Too expensive" without saying which line is a diagnosis with no
        // treatment attached.
        let costs = Costs {
            tip: MicroUsd::from_dollars(9.0),
            dex_fee: cents(5),
            ..Costs::default()
        };
        let Economics::Uneconomic { dominant_cost, .. } = evaluate(cents(1), &costs, RELIABLE)
        else {
            panic!("expected a refusal");
        };
        assert_eq!(dominant_cost, "tip");
    }

    #[test]
    fn a_refusal_reports_the_edge_that_would_have_been_needed() {
        let Economics::Uneconomic {
            breakeven_edge,
            total_cost,
            ..
        } = evaluate(MicroUsd::ZERO, &typical(), RELIABLE)
        else {
            panic!("expected a refusal");
        };
        assert_eq!(breakeven_edge, total_cost);
        // Five lines at 30/15/20/5/10 cents, plus 5% of $0.15.
        assert_eq!(total_cost, MicroUsd(800_000 + 7_500));
    }

    #[test]
    fn costs_saturate_rather_than_wrapping() {
        // A wrapped total would be a small number describing a huge cost, which
        // is the direction that loses money.
        let absurd = Costs {
            price_impact: MicroUsd(u64::MAX),
            slippage: MicroUsd(u64::MAX),
            dex_fee: MicroUsd(u64::MAX),
            priority_fee: MicroUsd(u64::MAX),
            tip: MicroUsd(u64::MAX),
        };
        assert_eq!(absurd.certain(), MicroUsd(u64::MAX));
        assert!(!evaluate(MicroUsd(u64::MAX), &absurd, RELIABLE).is_worthwhile());
    }

    #[test]
    fn the_gate_is_pure() {
        let costs = typical();
        let first = evaluate(MicroUsd::from_dollars(2.0), &costs, RELIABLE);
        for _ in 0..64 {
            assert_eq!(
                evaluate(MicroUsd::from_dollars(2.0), &costs, RELIABLE),
                first
            );
        }
    }

    #[test]
    fn a_certain_failure_costs_its_whole_fee() {
        let certain = FailureRisk {
            probability_bps: 10_000,
            cost: MicroUsd::from_dollars(1.0),
        };
        assert_eq!(certain.expected(), MicroUsd::from_dollars(1.0));
    }
}
