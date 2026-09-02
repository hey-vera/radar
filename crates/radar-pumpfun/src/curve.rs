// SPDX-License-Identifier: Apache-2.0
//! The bonding curve, and what a trade against it actually fills at.
//!
//! # Why this replaces a quote ladder
//!
//! `radar-sim` prices an exit by asking Jupiter for quotes at several sizes and
//! reading how the price degrades. That is a real depth probe and it has two
//! problems research already named. A quote is not a fill — research
//! [0016](https://github.com/hey-vera/radar/blob/main/docs/research/0016-the-entry-was-a-bid-and-the-exit-was-a-mid.md)
//! measured the gap between the two instruments at **at least 128 bps**, six
//! times the signal it was hiding. And the vendor will not route this venue
//! legacy at all (0021), so the ladder describes a trade Radar could not make.
//!
//! A bonding curve does not need a quote. It is `x · y = k` over reserves
//! published in an account anyone can read, so the fill is **computed**. That is
//! unusual and worth saying plainly: on an order book the same question needs a
//! model and a guess about who else is there; here it is arithmetic, exact for
//! the state it is given.
//!
//! # Exact for a state, not for a future
//!
//! The reserves at execution are not the reserves at decision — other
//! transactions land in between. So this answers *"what would this size fill at,
//! against these reserves"*, which is the honest question. It is not a
//! prediction, and
//! [0022](https://github.com/hey-vera/radar/blob/main/docs/research/0022-capacity-was-a-budget-not-a-ceiling.md)
//! says so where it uses these numbers.

use radar_types::Address;

/// The pump.fun bonding-curve account.
///
/// The layout after the 8-byte Anchor discriminator, read from mainnet: five
/// `u64` reserves, a `complete` flag, and the creator.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BondingCurve {
    /// Virtual token reserves — the `x` of `x · y = k`.
    pub virtual_token_reserves: u64,
    /// Virtual SOL reserves, in lamports — the `y`.
    pub virtual_sol_reserves: u64,
    /// Tokens the curve actually holds.
    pub real_token_reserves: u64,
    /// Lamports the curve actually holds.
    pub real_sol_reserves: u64,
    /// The mint's total supply.
    pub token_total_supply: u64,
    /// Whether the curve has graduated to the AMM.
    ///
    /// **Load-bearing.** A complete curve has no reserves and trades nothing;
    /// pricing against one produces a division by zero or, worse, a plausible
    /// number from stale fields. One of three tokens sampled on 2026-09-01 was
    /// already complete.
    pub complete: bool,
    /// Who launched the token. Needed to derive the creator vault.
    pub creator: Address,
}

/// Why an account could not be read as the layout it was asked for.
///
/// Shared by every parser in this crate -- the curve, the fee schedule and the
/// global account -- because the two ways a Solana account can be the wrong
/// thing do not vary by which thing you wanted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Malformed {
    /// Fewer bytes than the layout needs.
    TooShort {
        /// How many arrived.
        len: usize,
        /// How many the layout requires.
        needed: usize,
    },
    /// The account's Anchor discriminator is not the one the layout expects.
    ///
    /// Refused rather than parsed anyway: every Solana account is bytes, and a
    /// different account of the right length would produce reserves that look
    /// entirely reasonable.
    WrongDiscriminator {
        /// What the first eight bytes actually were.
        found: [u8; 8],
    },
}

/// The discriminator every bonding-curve account starts with.
///
/// Observed on mainnet, not computed from a name — the same standard
/// `radar_decode::pumpfun` holds its instruction table to.
pub const DISCRIMINATOR: [u8; 8] = [0x17, 0xb7, 0xf8, 0x37, 0x60, 0xd8, 0xac, 0x60];

/// Bytes the layout needs: discriminator, five `u64`, the flag, the creator.
const LAYOUT_LEN: usize = 8 + 5 * 8 + 1 + 32;

impl BondingCurve {
    /// Reads a bonding curve out of raw account data.
    ///
    /// # Errors
    ///
    /// [`Malformed`] when the account is too short or is not a bonding curve.
    /// Both are refusals rather than best-effort parses, for the reason
    /// [`Malformed::WrongDiscriminator`] gives.
    ///
    /// # Panics
    ///
    /// Cannot, and the reason is the length check on the first line: every slice
    /// below is inside `LAYOUT_LEN`, which has already been established. The
    /// `expect`s are there because a fallible conversion from a slice of proven
    /// length has no error worth propagating.
    pub fn parse(data: &[u8]) -> Result<Self, Malformed> {
        if data.len() < LAYOUT_LEN {
            return Err(Malformed::TooShort {
                len: data.len(),
                needed: LAYOUT_LEN,
            });
        }
        let found: [u8; 8] = data[..8].try_into().expect("checked above");
        if found != DISCRIMINATOR {
            return Err(Malformed::WrongDiscriminator { found });
        }

        let at = |i: usize| -> u64 {
            let start = 8 + i * 8;
            u64::from_le_bytes(data[start..start + 8].try_into().expect("checked above"))
        };
        let creator: [u8; 32] = data[49..81].try_into().expect("checked above");

        Ok(Self {
            virtual_token_reserves: at(0),
            virtual_sol_reserves: at(1),
            real_token_reserves: at(2),
            real_sol_reserves: at(3),
            token_total_supply: at(4),
            complete: data[48] != 0,
            creator: Address::new(creator),
        })
    }

    /// Whether this curve can be traded against at all.
    ///
    /// Rule 9's shape: a curve that cannot be traded is not a curve with a price
    /// of zero. Every pricing function below returns `None` for one.
    #[must_use]
    pub const fn is_tradeable(&self) -> bool {
        !self.complete && self.virtual_token_reserves > 0 && self.virtual_sol_reserves > 0
    }

    /// What `lamports` of SOL buys, and at what cost.
    ///
    /// `None` when the curve is not tradeable, or when the size is so small it
    /// rounds to no tokens — which is a real answer rather than a zero-cost
    /// trade.
    #[must_use]
    pub fn buy(&self, lamports: u64) -> Option<Fill> {
        if !self.is_tradeable() || lamports == 0 {
            return None;
        }
        // `u128` throughout. `virtual_token_reserves` is ~1e15 and
        // `virtual_sol_reserves` ~3e10, so the product overflows `u64` by orders
        // of magnitude -- and an overflow here would silently produce a price
        // that looks plausible.
        let x = u128::from(self.virtual_token_reserves);
        let y = u128::from(self.virtual_sol_reserves);
        let dy = u128::from(lamports);
        let k = x.checked_mul(y)?;
        // Rounding **up** on the divisor side, so the trader never receives more
        // than the curve would give. A rounding error in the trader's favour is
        // a cost estimate rounded down, which 0019 names as the direction that
        // launders a trade past the kernel.
        let new_x = k.div_ceil(y.checked_add(dy)?);
        let tokens = x.checked_sub(new_x)?;
        if tokens == 0 {
            return None;
        }
        Some(Fill {
            lamports,
            tokens: u64::try_from(tokens).ok()?,
            impact_bps: impact_bps(dy, y)?,
        })
    }

    /// What selling `tokens` returns, and at what cost.
    ///
    /// `None` on an untradeable curve, a zero size, or a size that returns no
    /// lamports.
    #[must_use]
    pub fn sell(&self, tokens: u64) -> Option<Fill> {
        if !self.is_tradeable() || tokens == 0 {
            return None;
        }
        let x = u128::from(self.virtual_token_reserves);
        let y = u128::from(self.virtual_sol_reserves);
        let dx = u128::from(tokens);
        let k = x.checked_mul(y)?;
        // Rounding **down** what the trader receives, for the same reason `buy`
        // rounds the other way: every rounding decision here favours the curve.
        let new_y = k / x.checked_add(dx)?;
        let lamports = y.checked_sub(new_y)?;
        if lamports == 0 {
            return None;
        }
        Some(Fill {
            lamports: u64::try_from(lamports).ok()?,
            tokens,
            impact_bps: impact_bps(lamports, y)?,
        })
    }

    /// The largest buy whose price impact stays within `max_bps`.
    ///
    /// Binary search rather than the closed form, because the closed form would
    /// have to agree with [`buy`](Self::buy)'s rounding exactly and would drift
    /// from it the moment either changed. Searching *uses* `buy`, so it cannot
    /// disagree with what a trade would actually cost.
    ///
    /// `None` when even one lamport exceeds the budget, which is a real answer
    /// about a curve with no depth.
    #[must_use]
    pub fn buy_within_impact(&self, max_bps: u32, ceiling_lamports: u64) -> Option<u64> {
        if !self.is_tradeable() {
            return None;
        }
        let fits = |size: u64| {
            self.buy(size)
                .is_some_and(|f| f.impact_bps <= u64::from(max_bps))
        };
        if !fits(1) {
            return None;
        }
        let (mut lo, mut hi) = (1u64, ceiling_lamports.max(1));
        if fits(hi) {
            return Some(hi);
        }
        // Invariant: `lo` fits and `hi` does not. Sixty-four iterations is more
        // than enough to close any `u64` range, and a fixed bound cannot loop
        // forever the way a `while lo < hi` can when the midpoint stops moving.
        //
        // There is no early break, and its absence is deliberate. Once
        // `hi - lo == 1` the midpoint is `lo`, which fits, so `lo` is reassigned
        // to itself and the remaining iterations change nothing. A `break` there
        // would be an optimisation on a loop that runs at most sixty-four times
        // -- and mutation testing showed it was untestable, because removing it
        // cannot change an answer. An untestable branch is one to delete rather
        // than one to write a test around.
        for _ in 0..64 {
            let mid = lo + (hi - lo) / 2;
            if fits(mid) { lo = mid } else { hi = mid }
        }
        Some(lo)
    }
}

/// Price impact of a trade, in basis points of the reserve it moves.
///
/// The effective price against the spot price, which for a constant product is
/// `size / reserves` to first order and exactly this in the limit that matters.
fn impact_bps(size: u128, reserves: u128) -> Option<u64> {
    u64::try_from(size.checked_mul(10_000)? / reserves.max(1)).ok()
}

/// What a trade against the curve would fill at.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Fill {
    /// Lamports paid, on a buy, or received, on a sell.
    pub lamports: u64,
    /// Tokens received, on a buy, or sold.
    pub tokens: u64,
    /// How far this size moves the price, in basis points.
    pub impact_bps: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The curve behind `BfVg4yLn…`, read from mainnet on 2026-09-01.
    ///
    /// Real reserves rather than round numbers, because a test written against
    /// tidy inputs cannot catch an overflow that only appears at 1e15.
    fn live() -> BondingCurve {
        BondingCurve {
            virtual_token_reserves: 889_566_950_293_959,
            virtual_sol_reserves: 36_186_150_833,
            real_token_reserves: 609_666_950_293_959,
            real_sol_reserves: 6_186_150_833,
            token_total_supply: 1_000_000_000_000_000,
            complete: false,
            creator: Address::new([1u8; 32]),
        }
    }

    #[test]
    fn a_buy_fills_at_what_mainnet_reserves_say() {
        // 0.03 SOL against the live curve. Recorded so a change to the
        // arithmetic has to be deliberate.
        //
        // **736,881,416,025, not ...026.** The naive floor gives ...026; this
        // rounds the divisor up, so the trader is told they receive one token
        // less than the exact quotient. That one token is the whole point: a
        // `min_tokens_out` derived from an over-estimate is a slippage bound the
        // curve can miss, and 0019 names rounding in the trader's favour as the
        // direction that launders a trade past the kernel.
        let fill = live().buy(30_000_000).expect("a fill");
        assert_eq!(fill.tokens, 736_881_416_025);
        assert_eq!(fill.lamports, 30_000_000);
        // 0.03 SOL against 36.19 SOL of reserves is ~8 bps.
        assert_eq!(fill.impact_bps, 8);
    }

    #[test]
    fn the_arithmetic_does_not_overflow_at_real_reserve_sizes() {
        // `virtual_token_reserves * virtual_sol_reserves` is ~3e25, which
        // overflows u64 by six orders of magnitude. In u64 this panics in debug
        // and wraps in release -- and a wrapped product produces a price that
        // looks entirely reasonable.
        let curve = live();
        for size in [1, 1_000, 1_000_000, 1_000_000_000, 50_000_000_000] {
            let fill = curve.buy(size).expect("a fill");
            assert!(fill.tokens > 0, "size {size} filled nothing");
        }
    }

    #[test]
    fn every_rounding_decision_favours_the_curve() {
        // 0019: a cost estimate rounded down is the direction that launders a
        // trade past the kernel. So a buy must never report more tokens than the
        // exact quotient, and a sell never more lamports.
        let curve = live();
        let x = u128::from(curve.virtual_token_reserves);
        let y = u128::from(curve.virtual_sol_reserves);
        let dy = 30_000_000u128;
        let exact = x - (x * y) / (y + dy);

        let got = u128::from(curve.buy(30_000_000).expect("a fill").tokens);
        assert!(
            got <= exact,
            "buy rounded in the trader's favour: {got} > {exact}"
        );
    }

    #[test]
    fn a_completed_curve_prices_nothing_rather_than_zero() {
        // Rule 9. A graduated curve has no reserves; pricing against one would
        // divide by zero or read stale fields as a real market. One of three
        // tokens sampled on 2026-09-01 was already complete.
        let graduated = BondingCurve {
            complete: true,
            ..live()
        };
        assert!(!graduated.is_tradeable());
        assert_eq!(graduated.buy(30_000_000), None);
        assert_eq!(graduated.sell(1_000_000), None);
        assert_eq!(graduated.buy_within_impact(100, 50_000_000_000), None);
    }

    #[test]
    fn an_empty_curve_prices_nothing() {
        // The other shape of the same thing: reserves of zero, which a sampled
        // token really had.
        let empty = BondingCurve {
            virtual_sol_reserves: 0,
            virtual_token_reserves: 0,
            ..live()
        };
        assert!(!empty.is_tradeable());
        assert_eq!(empty.buy(1), None);
    }

    #[test]
    fn a_size_too_small_to_fill_returns_nothing_rather_than_a_free_trade() {
        // A buy that rounds to zero tokens is not a trade that cost nothing.
        let thin = BondingCurve {
            virtual_token_reserves: 1_000,
            virtual_sol_reserves: 1_000_000_000_000,
            ..live()
        };
        assert_eq!(thin.buy(1), None);
    }

    #[test]
    fn the_impact_search_agrees_with_what_a_trade_would_cost() {
        // The search uses `buy`, so it cannot report a size whose actual impact
        // exceeds the budget. Asserted rather than assumed, because a closed-form
        // search would drift from `buy`'s rounding.
        let curve = live();
        for budget in [1u32, 10, 100, 500, 850] {
            let size = curve
                .buy_within_impact(budget, 500_000_000_000)
                .expect("a size");
            let fill = curve.buy(size).expect("a fill");
            assert!(
                fill.impact_bps <= u64::from(budget),
                "budget {budget}: size {size} actually costs {} bps",
                fill.impact_bps
            );
            // And one lamport more should exceed it, or the search stopped early.
            if let Some(next) = curve.buy(size + 1) {
                assert!(
                    next.impact_bps > u64::from(budget) || size + 1 >= 500_000_000_000,
                    "budget {budget}: the search stopped short at {size}"
                );
            }
        }
    }

    #[test]
    fn the_one_percent_budget_reproduces_the_figure_0022_reported() {
        // 0022's central claim: 1% impact on a ~36 SOL curve is ~0.36 SOL, and
        // the ~$31 capacity Radar reports is that budget rather than the venue's
        // limit. If this drifts, that note is wrong.
        let size = live()
            .buy_within_impact(100, 500_000_000_000)
            .expect("a size");
        // In lamports, so the assertion needs no float. 0.35 to 0.37 SOL.
        assert!(
            (350_000_000..=370_000_000).contains(&size),
            "1% of a 36.19 SOL curve should be ~0.362 SOL, got {size} lamports"
        );
    }

    #[test]
    fn parsing_refuses_an_account_that_is_not_a_bonding_curve() {
        // Every Solana account is bytes. A different account of the right length
        // parses into reserves that look entirely reasonable, so the
        // discriminator is checked rather than assumed.
        let mut data = vec![0u8; LAYOUT_LEN];
        data[..8].copy_from_slice(&[9, 9, 9, 9, 9, 9, 9, 9]);
        assert!(matches!(
            BondingCurve::parse(&data),
            Err(Malformed::WrongDiscriminator { .. })
        ));

        assert!(matches!(
            BondingCurve::parse(&[0u8; 4]),
            Err(Malformed::TooShort { .. })
        ));
    }

    #[test]
    fn parsing_round_trips_the_layout_read_from_mainnet() {
        // Built from the observed field order, so a reordering of the struct
        // fails here rather than silently reading the SOL reserve as the token
        // reserve -- which would price every trade by a factor of 1e5.
        let mut data = vec![0u8; LAYOUT_LEN];
        data[..8].copy_from_slice(&DISCRIMINATOR);
        for (i, v) in [
            889_566_950_293_959u64,
            36_186_150_833,
            609_666_950_293_959,
            6_186_150_833,
            1_000_000_000_000_000,
        ]
        .iter()
        .enumerate()
        {
            data[8 + i * 8..16 + i * 8].copy_from_slice(&v.to_le_bytes());
        }
        data[48] = 0;
        data[49..81].copy_from_slice(&[7u8; 32]);

        let parsed = BondingCurve::parse(&data).expect("parses");
        assert_eq!(parsed.virtual_token_reserves, 889_566_950_293_959);
        assert_eq!(parsed.virtual_sol_reserves, 36_186_150_833);
        assert_eq!(parsed.token_total_supply, 1_000_000_000_000_000);
        assert!(!parsed.complete);
        assert_eq!(parsed.creator, Address::new([7u8; 32]));
    }

    #[test]
    fn a_sell_returns_what_mainnet_reserves_say() {
        // The exit half, pinned to an exact number for the same reason the buy
        // is. Every previous test of `sell` went through `capacity_lamports`,
        // which takes a maximum -- so the arithmetic could have been wrong in
        // ways a maximum hides. Mutation testing found exactly that gap.
        let fill = live().sell(1_000_000_000_000).expect("a fill");
        assert_eq!(fill.lamports, 40_632_713);
        assert_eq!(fill.tokens, 1_000_000_000_000);
        assert_eq!(fill.impact_bps, 11);
    }

    #[test]
    fn a_round_trip_through_the_curve_loses_money_before_any_fee() {
        // Buy 0.03 SOL of this token and immediately sell it back: 30,000,000
        // lamports in, 29,950,340 out. About 16.5 bps, and that is *before* the
        // 125 bps the venue charges each way (see `crate::fees`).
        //
        // Worth pinning because it is the cheapest possible demonstration that a
        // round trip is not free even at zero impact-budget, which is the fact
        // 0019 and 0022 are both about.
        let curve = live();
        let bought = curve.buy(30_000_000).expect("a fill");
        let back = curve.sell(bought.tokens).expect("a fill");
        assert_eq!(back.lamports, 29_950_340);
        assert!(back.lamports < 30_000_000);
    }

    #[test]
    fn selling_nothing_is_nothing_and_selling_something_is_something() {
        // Both halves in one test, because either alone passes under a mutation
        // that inverts the zero check.
        let curve = live();
        assert_eq!(curve.sell(0), None);
        assert!(curve.sell(1_000_000_000_000).is_some());
    }

    #[test]
    fn the_layout_length_is_exactly_the_fields_it_names() {
        // 8 discriminator + 5 * 8 reserves + 1 flag + 32 creator = 81. A wrong
        // constant that is *larger* than the real layout still parses the
        // mainnet capture, because that account carries trailing bytes -- so the
        // boundary has to be tested at exactly the layout length.
        assert_eq!(LAYOUT_LEN, 81);
        let mut exact = vec![0u8; LAYOUT_LEN];
        exact[..8].copy_from_slice(&DISCRIMINATOR);
        // Non-zero reserves, so the parse yields a tradeable curve rather than
        // one that is refused for a different reason.
        exact[8..16].copy_from_slice(&1_000_000u64.to_le_bytes());
        exact[16..24].copy_from_slice(&1_000_000u64.to_le_bytes());
        assert!(BondingCurve::parse(&exact).is_ok(), "exactly the layout");
        assert!(
            BondingCurve::parse(&exact[..LAYOUT_LEN - 1]).is_err(),
            "one byte short is short"
        );
    }

    #[test]
    fn the_impact_search_lands_on_the_boundary_and_not_past_it() {
        // A tidy curve so the answer is checkable by hand: 1e12 tokens against
        // 1e6 lamports, a 1% budget, and a ceiling well above the answer.
        //
        // The reserves are lopsided on purpose. With *equal* reserves one
        // lamport buys zero tokens, `buy` returns `None` for it, and the search
        // refuses before it starts -- which is correct behaviour and makes the
        // curve useless for testing the search. The first version of this test
        // used equal reserves and failed for that reason.
        //
        // 10,099 lamports is exactly 100 bps; 10,100 is 101. A search that
        // stopped one short, or overshot by one, would still return a plausible
        // number -- which is why the neighbour is asserted too.
        let curve = BondingCurve {
            virtual_token_reserves: 1_000_000_000_000,
            virtual_sol_reserves: 1_000_000,
            real_token_reserves: 1_000_000_000_000,
            real_sol_reserves: 0,
            token_total_supply: 1_000_000_000_000,
            complete: false,
            creator: Address::new([1u8; 32]),
        };
        let found = curve.buy_within_impact(100, 1_000_000).expect("depth");
        assert_eq!(found, 10_099);
        assert_eq!(curve.buy(found).expect("a fill").impact_bps, 100);
        assert_eq!(curve.buy(found + 1).expect("a fill").impact_bps, 101);
    }
}
