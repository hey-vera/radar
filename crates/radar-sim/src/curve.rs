// SPDX-License-Identifier: Apache-2.0
//! Pricing an exit off the bonding curve instead of off a quote ladder.
//!
//! [research 0022](../../../docs/research/0022-capacity-was-a-budget-not-a-ceiling.md)
//! asks for this by name: *"price the exit off the curve rather than off a quote
//! ladder"*. This is that, and it changes three things at once.
//!
//! # It is the same instrument on both sides
//!
//! [`0016`](../../../docs/research/0016-the-entry-was-a-bid-and-the-exit-was-a-mid.md)
//! is the most expensive mistake in this repository's history: the entry was a
//! **sell quote** and the exit was a **mid**, and the gap between those two
//! instruments was at least 128 bps against a signal of 21. A curve has no bid
//! and no mid. `sell(n)` is what the program pays for `n` tokens, computed from
//! reserves anyone can read, so there is no second instrument to be confused
//! with the first.
//!
//! # It is exact for a state, and still not a fill
//!
//! Worth being precise, because "computed rather than estimated" invites more
//! confidence than it earns. The arithmetic is exact for the reserves it is
//! handed. A real fill lands after other transactions in the same block, so the
//! reserves at execution are not the reserves at decision. This removes
//! *quote* error. It does not remove *timing* error, and nothing here should be
//! read as a realised trade.
//!
//! # It costs one account read, not eight quotes
//!
//! [`Search`](crate::exit::Search) budgets `max_quotes` because every quote was
//! an HTTP request that cost money and time. A curve is read **once** and then
//! answers every size in the search for free, so the budget stops binding — and
//! [`crate::exit::discover_capacity`] can sweep for the real capacity rather
//! than the best of eight guesses.
//!
//! # The fee is subtracted, and that is not cosmetic
//!
//! [`radar_pumpfun::curve`] prices the curve alone. pump.fun also charges 125
//! bps, read from the fee schedule rather than assumed
//! ([`radar_pumpfun::fees`]). An exit priced gross of it overstates what a
//! position returns by that much on every size, and overstating the exit is
//! precisely the direction that lets a position through the risk kernel that
//! should not have been.

use radar_pumpfun::{BondingCurve, Fees};
use radar_types::Address;

use crate::exit::{QuoteError, QuotePoint, Quoter};

/// A [`Quoter`] backed by one token's bonding curve.
///
/// Pure. It holds reserves and a fee schedule, not a client — so every rule that
/// gates capital stays exercisable offline, which is the property
/// [`crate::exit`] was built around.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Depth {
    mint: Address,
    curve: BondingCurve,
    fees: Fees,
}

impl Depth {
    /// A quoter for `mint`, against reserves read at some slot.
    ///
    /// The mint is stored rather than inferred because a `BondingCurve` does not
    /// carry the mint it belongs to — see [`Self::quote_sell`] for why that
    /// matters more than it looks.
    #[must_use]
    pub const fn new(mint: Address, curve: BondingCurve, fees: Fees) -> Self {
        Self { mint, curve, fees }
    }

    /// The curve these prices come from.
    #[must_use]
    pub const fn curve(&self) -> &BondingCurve {
        &self.curve
    }

    /// The fee subtracted from every quote.
    #[must_use]
    pub const fn fees(&self) -> Fees {
        self.fees
    }
}

impl Quoter for Depth {
    /// What selling `size_tokens` returns, net of the curve's fee.
    ///
    /// # Errors
    ///
    /// [`QuoteError::NoRoute`] when the curve cannot fill the size at all — a
    /// completed curve, a zero size, or a size so small it returns no lamports.
    /// That is the same answer a router gives, and it means the same thing.
    ///
    /// [`QuoteError::Unavailable`] when asked about a **different mint**. This
    /// quoter knows exactly one token, and answering for another with this
    /// token's reserves would be the worst available failure: a plausible price
    /// for the wrong asset, which no downstream check could catch.
    fn quote_sell(&self, mint: &Address, size_tokens: u64) -> Result<QuotePoint, QuoteError> {
        if *mint != self.mint {
            return Err(QuoteError::Unavailable(format!(
                "this curve is {}, not {mint}",
                self.mint
            )));
        }
        let fill = self
            .curve
            .sell(size_tokens)
            .ok_or(QuoteError::NoRoute { size_tokens })?;
        // Net, not gross. The fee is charged on the proceeds and `charge` clamps
        // at them, so the subtraction cannot underflow.
        let out_lamports = fill.lamports - self.fees.charge(fill.lamports);
        if out_lamports == 0 {
            // A sale whose entire proceeds go to the fee is not a sale at a
            // price of zero, it is a size this venue will not fill -- and the
            // curve does reach it: one base unit of a standard launch returns
            // exactly one lamport, all of which is fee. Returning a quote here
            // would put a zero into the exit curve, where `capacity_lamports`
            // takes a maximum and a zero is harmless, but `probe` reports
            // `Confidence::Measured` for it -- a measured exit worth nothing.
            return Err(QuoteError::NoRoute { size_tokens });
        }
        Ok(QuotePoint {
            size_tokens,
            // Rule 9: an impact too large to represent is maximum impact, never
            // zero. `u32::MAX` is what the rest of the system already reads as
            // "unknown", and zero would read as "free".
            impact_bps: u32::try_from(fill.impact_bps).unwrap_or(u32::MAX),
            out_lamports,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exit::{Search, discover_capacity};
    use crate::mint::{MintStructure, TOKEN_PROGRAM};

    /// A mint whose only load-bearing field here is the supply the search
    /// scales its first rung against.
    fn structure(supply: u64) -> MintStructure {
        let mut data = vec![0u8; 82];
        data[36..44].copy_from_slice(&supply.to_le_bytes());
        data[44] = 6;
        data[45] = 1;
        MintStructure::parse(&data, TOKEN_PROGRAM).expect("a well formed mint account")
    }

    fn mint() -> Address {
        Address::new([7u8; 32])
    }

    fn other() -> Address {
        Address::new([9u8; 32])
    }

    /// A pump.fun launch at the reserves the global account publishes as the
    /// defaults: 1,073,000,000,000,000 tokens against 30 SOL.
    fn fresh() -> BondingCurve {
        BondingCurve {
            virtual_token_reserves: 1_073_000_000_000_000,
            virtual_sol_reserves: 30_000_000_000,
            real_token_reserves: 793_100_000_000_000,
            real_sol_reserves: 0,
            token_total_supply: 1_000_000_000_000_000,
            complete: false,
            creator: Address::new([1u8; 32]),
        }
    }

    fn fees() -> Fees {
        Fees {
            lp_bps: 0,
            protocol_bps: 95,
            creator_bps: 30,
        }
    }

    fn depth() -> Depth {
        Depth::new(mint(), fresh(), fees())
    }

    #[test]
    fn a_quote_is_net_of_the_fee() {
        let gross = fresh().sell(1_000_000_000_000).expect("a fill");
        let net = depth()
            .quote_sell(&mint(), 1_000_000_000_000)
            .expect("a quote");
        assert!(
            net.out_lamports < gross.lamports,
            "the fee must actually come off: gross {}, net {}",
            gross.lamports,
            net.out_lamports
        );
        assert_eq!(
            net.out_lamports,
            gross.lamports - fees().charge(gross.lamports)
        );
        // And the size of the bite is the schedule's, not something incidental.
        let taken = gross.lamports - net.out_lamports;
        let bps = taken * 10_000 / gross.lamports;
        assert_eq!(bps, 125, "125 bps, as the fee schedule charges");
    }

    #[test]
    fn a_quote_for_another_mint_is_refused_rather_than_answered() {
        // The failure this prevents does not look like a failure. These reserves
        // would produce an entirely plausible price for a token they have
        // nothing to do with, and no check downstream could tell.
        let refused = depth().quote_sell(&other(), 1_000_000_000_000);
        assert!(matches!(refused, Err(QuoteError::Unavailable(_))));
    }

    #[test]
    fn a_completed_curve_has_no_route_rather_than_a_price_of_zero() {
        // One of three tokens sampled on 2026-09-01 had already graduated.
        // Rule 9: it cannot be sold here, which is not the same as being
        // worthless here.
        let mut done = fresh();
        done.complete = true;
        let quoter = Depth::new(mint(), done, fees());
        assert!(matches!(
            quoter.quote_sell(&mint(), 1_000_000_000_000),
            Err(QuoteError::NoRoute { .. })
        ));
    }

    #[test]
    fn a_size_too_small_to_return_a_lamport_has_no_route() {
        // Not a free trade, and not a zero-value one. It is a size the venue
        // will not fill, which is what `NoRoute` means everywhere else.
        assert!(matches!(
            depth().quote_sell(&mint(), 1),
            Err(QuoteError::NoRoute { size_tokens: 1 })
        ));
    }

    #[test]
    fn selling_more_returns_more_but_at_a_worse_price() {
        // The property the whole exit analysis rests on. If it did not hold, a
        // capacity search would be meaningless.
        let quoter = depth();
        let small = quoter
            .quote_sell(&mint(), 1_000_000_000_000)
            .expect("a quote");
        let large = quoter
            .quote_sell(&mint(), 10_000_000_000_000)
            .expect("a quote");
        assert!(large.out_lamports > small.out_lamports);
        assert!(
            large.impact_bps > small.impact_bps,
            "ten times the size must cost more impact"
        );
        // Per token, the larger sale is worse -- that is what impact *is*.
        //
        // Scaled by 1e12 rather than 1e3: at these reserves a token is worth
        // about 2.8e-5 lamports, so a smaller scale truncates both sides to the
        // same integer and the assertion passes without testing anything. The
        // first version of this test did exactly that.
        let each = |q: &QuotePoint| {
            u128::from(q.out_lamports) * 1_000_000_000_000 / u128::from(q.size_tokens.max(1))
        };
        assert!(
            each(&large) < each(&small),
            "per token: small {} large {}",
            each(&small),
            each(&large)
        );
    }

    #[test]
    fn capacity_can_be_discovered_without_a_single_network_call() {
        // The point of the whole module. `discover_capacity` was written against
        // a quoter that cost money per question, so it budgets `max_quotes`.
        // Against a curve the budget stops binding, and the search finds a real
        // number rather than the best of eight guesses.
        let quoter = depth();
        let report = discover_capacity(
            &quoter,
            &mint(),
            Some(structure(1_000_000_000_000_000)),
            Search {
                max_impact_bps: 100,
                ..Search::DEFAULT
            },
        );
        let capacity = report
            .capacity_lamports(100)
            .expect("a fresh curve has depth at 1% impact");
        // 1% of 30 SOL is about 0.3 SOL, which is research 0022's figure for a
        // standard launch -- reproduced here from the curve rather than from a
        // vendor's quote ladder.
        assert!(
            (250_000_000..=310_000_000).contains(&capacity),
            "expected roughly 0.3 SOL of capacity at 1% impact, got {capacity}"
        );
    }

    #[test]
    fn the_impact_budget_is_what_moves_the_capacity() {
        // 0022's actual finding: the ±13% band around $31 was a fact about
        // `max_impact_bps`, not about the venue. Raising the budget raises the
        // capacity, and this is that claim as a test.
        let quoter = depth();
        let at = |bps| {
            discover_capacity(
                &quoter,
                &mint(),
                Some(structure(1_000_000_000_000_000)),
                Search {
                    max_impact_bps: bps,
                    ..Search::DEFAULT
                },
            )
            .capacity_lamports(bps)
            .expect("depth exists at every budget here")
        };
        let one_percent = at(100);
        let round_trip = at(850);
        assert!(
            round_trip > one_percent * 5,
            "8.5% should admit many times what 1% does: {one_percent} then {round_trip}"
        );
    }

    #[test]
    fn a_fee_of_zero_prices_higher_than_the_real_schedule() {
        // Guards the direction that matters. If the fee were ever dropped or
        // defaulted away, every exit would look better than it is -- and a
        // better-looking exit is what lets a position past the risk kernel.
        let free = Depth::new(mint(), fresh(), Fees::default());
        let real = depth();
        let size = 1_000_000_000_000;
        assert!(
            free.quote_sell(&mint(), size).expect("q").out_lamports
                > real.quote_sell(&mint(), size).expect("q").out_lamports
        );
    }
}
