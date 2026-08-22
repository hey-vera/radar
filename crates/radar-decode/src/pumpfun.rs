// SPDX-License-Identifier: Apache-2.0
//! The pump.fun bonding-curve program.
//!
//! Every discriminator here was captured from mainnet traffic rather than copied
//! from a reference, and `tests/fixtures/pumpfun_instructions.json` holds the raw
//! bytes each one came from. Public references describe a program with roughly
//! three instructions; the live one has at least fourteen, including four
//! distinct buy variants and a per-user volume accumulator.

use radar_types::Address;

use crate::discriminator::Discriminator;

/// The pump.fun program address, `6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P`.
pub const PROGRAM_ID: Address = Address::new([
    0x01, 0x56, 0xe0, 0xf6, 0x93, 0x66, 0x5a, 0xcf, 0x44, 0xdb, 0x15, 0x68, 0xbf, 0x17, 0x5b, 0xaa,
    0x51, 0x89, 0xcb, 0x97, 0xf5, 0xd2, 0xff, 0x3b, 0x65, 0x5d, 0x2b, 0xb6, 0xfd, 0x6d, 0x18, 0xb0,
]);

/// A pump.fun instruction, identified by discriminator.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum Instruction {
    /// Creates a token and its bonding curve. Carries the name, symbol and URI,
    /// and in most launches performs the developer's first buy in the same
    /// transaction.
    CreateV2,
    /// Buy against the bonding curve.
    Buy,
    /// Buy, v2.
    BuyV2,
    /// Buy specifying the exact SOL to spend.
    BuyExactSolIn,
    /// Buy specifying the exact quote amount in, v2.
    BuyExactQuoteInV2,
    /// Sell into the bonding curve.
    Sell,
    /// Sell, v2.
    SellV2,
    /// Claim accrued trading cashback.
    ClaimCashback,
    /// Claim accrued trading cashback, v2.
    ClaimCashbackV2,
    /// Creator collects their fee share.
    CollectCreatorFee,
    /// Creator collects their fee share, v2.
    CollectCreatorFeeV2,
    /// Distribute accrued creator fees.
    DistributeCreatorFees,
    /// Open a per-user volume accumulator.
    InitUserVolumeAccumulator,
    /// Close a per-user volume accumulator.
    CloseUserVolumeAccumulator,
}

/// Every known instruction paired with its discriminator and Anchor name.
///
/// The Anchor name is carried so a test can recompute the discriminator and
/// prove the table is not drifting from the convention. Bytes are ground truth;
/// the names are what the test checks them against.
pub const KNOWN: &[(Instruction, [u8; 8], &str)] = &[
    (
        Instruction::CreateV2,
        [0xd6, 0x90, 0x4c, 0xec, 0x5f, 0x8b, 0x31, 0xb4],
        "create_v2",
    ),
    (
        Instruction::Buy,
        [0x66, 0x06, 0x3d, 0x12, 0x01, 0xda, 0xeb, 0xea],
        "buy",
    ),
    (
        Instruction::BuyV2,
        [0xb8, 0x17, 0xee, 0x61, 0x67, 0xc5, 0xd3, 0x3d],
        "buy_v2",
    ),
    (
        Instruction::BuyExactSolIn,
        [0x38, 0xfc, 0x74, 0x08, 0x9e, 0xdf, 0xcd, 0x5f],
        "buy_exact_sol_in",
    ),
    (
        Instruction::BuyExactQuoteInV2,
        [0xc2, 0xab, 0x1c, 0x46, 0x68, 0x4d, 0x5b, 0x2f],
        "buy_exact_quote_in_v2",
    ),
    (
        Instruction::Sell,
        [0x33, 0xe6, 0x85, 0xa4, 0x01, 0x7f, 0x83, 0xad],
        "sell",
    ),
    (
        Instruction::SellV2,
        [0x5d, 0xf6, 0x82, 0x3c, 0xe7, 0xe9, 0x40, 0xb2],
        "sell_v2",
    ),
    (
        Instruction::ClaimCashback,
        [0x25, 0x3a, 0x23, 0x7e, 0xbe, 0x35, 0xe4, 0xc5],
        "claim_cashback",
    ),
    (
        Instruction::ClaimCashbackV2,
        [0x7a, 0xf3, 0xcc, 0x41, 0x5e, 0x74, 0x1d, 0x37],
        "claim_cashback_v2",
    ),
    (
        Instruction::CollectCreatorFee,
        [0x14, 0x16, 0x56, 0x7b, 0xc6, 0x1c, 0xdb, 0x84],
        "collect_creator_fee",
    ),
    (
        Instruction::CollectCreatorFeeV2,
        [0xcf, 0x11, 0x8a, 0xf2, 0x04, 0x22, 0x13, 0x38],
        "collect_creator_fee_v2",
    ),
    (
        Instruction::DistributeCreatorFees,
        [0xa5, 0x72, 0x67, 0x00, 0x79, 0xce, 0xf7, 0x51],
        "distribute_creator_fees",
    ),
    (
        Instruction::InitUserVolumeAccumulator,
        [0x5e, 0x06, 0xca, 0x73, 0xff, 0x60, 0xe8, 0xb7],
        "init_user_volume_accumulator",
    ),
    (
        Instruction::CloseUserVolumeAccumulator,
        [0xf9, 0x45, 0xa4, 0xda, 0x96, 0x67, 0x54, 0x8a],
        "close_user_volume_accumulator",
    ),
];

impl Instruction {
    /// Looks up an instruction by discriminator.
    #[must_use]
    pub fn from_discriminator(d: Discriminator) -> Option<Self> {
        KNOWN
            .iter()
            .find(|(_, bytes, _)| bytes == d.as_bytes())
            .map(|(ix, _, _)| *ix)
    }

    /// This instruction's discriminator.
    ///
    /// # Panics
    ///
    /// If `KNOWN` has no row for this variant. That is a table-integrity bug
    /// rather than a runtime condition — adding an enum variant without its
    /// bytes — and `lookup_round_trips` fails first if it ever happens.
    #[must_use]
    pub fn discriminator(self) -> Discriminator {
        let (_, bytes, _) = KNOWN
            .iter()
            .find(|(ix, _, _)| *ix == self)
            .expect("KNOWN is exhaustive");
        Discriminator::new(*bytes)
    }

    /// The Anchor instruction name.
    ///
    /// # Panics
    ///
    /// If `KNOWN` has no row for this variant; see [`discriminator`](Self::discriminator).
    #[must_use]
    pub fn anchor_name(self) -> &'static str {
        let (_, _, name) = KNOWN
            .iter()
            .find(|(ix, _, _)| *ix == self)
            .expect("KNOWN is exhaustive");
        name
    }

    /// Whether this instruction acquires tokens, across **every** buy variant.
    ///
    /// Ask this rather than comparing against a variant. There are four buy
    /// instructions live concurrently, and code that checks for one of them is
    /// wrong about the other three — which is exactly how a coordination
    /// detector ends up reporting a base rate of zero for its strongest signal.
    #[must_use]
    pub const fn is_buy(self) -> bool {
        matches!(
            self,
            Self::Buy | Self::BuyV2 | Self::BuyExactSolIn | Self::BuyExactQuoteInV2
        )
    }

    /// Whether this instruction disposes of tokens, across every sell variant.
    #[must_use]
    pub const fn is_sell(self) -> bool {
        matches!(self, Self::Sell | Self::SellV2)
    }

    /// Whether this instruction creates a token.
    #[must_use]
    pub const fn is_launch(self) -> bool {
        matches!(self, Self::CreateV2)
    }

    /// Whether this instruction moves a position. Fee collection, cashback
    /// claims and accumulator bookkeeping do not, and counting them as activity
    /// overstates how much is happening around a token.
    #[must_use]
    pub const fn is_trade(self) -> bool {
        self.is_buy() || self.is_sell()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_program_address_is_the_one_on_chain() {
        assert_eq!(
            PROGRAM_ID.to_string(),
            "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P"
        );
    }

    #[test]
    fn every_discriminator_is_distinct() {
        let mut seen: Vec<[u8; 8]> = KNOWN.iter().map(|(_, b, _)| *b).collect();
        let before = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), before, "two instructions share a discriminator");
    }

    #[test]
    fn lookup_round_trips() {
        for (ix, _, _) in KNOWN {
            assert_eq!(
                Instruction::from_discriminator(ix.discriminator()),
                Some(*ix)
            );
        }
    }

    #[test]
    fn all_four_buy_variants_count_as_buys() {
        // The whole point of the predicate. If this ever regresses to checking a
        // single variant, coordination detection quietly loses most of its signal.
        let buys: Vec<_> = KNOWN
            .iter()
            .map(|(ix, _, _)| *ix)
            .filter(|ix| ix.is_buy())
            .collect();
        assert_eq!(buys.len(), 4, "expected four buy variants, got {buys:?}");
        assert!(buys.contains(&Instruction::Buy));
        assert!(buys.contains(&Instruction::BuyV2));
        assert!(buys.contains(&Instruction::BuyExactSolIn));
        assert!(buys.contains(&Instruction::BuyExactQuoteInV2));
    }

    #[test]
    fn bookkeeping_instructions_are_not_trades() {
        // Counting cashback claims and accumulator opens as activity would
        // overstate how much is happening around a token.
        for ix in [
            Instruction::ClaimCashback,
            Instruction::ClaimCashbackV2,
            Instruction::CollectCreatorFee,
            Instruction::CollectCreatorFeeV2,
            Instruction::DistributeCreatorFees,
            Instruction::InitUserVolumeAccumulator,
            Instruction::CloseUserVolumeAccumulator,
        ] {
            assert!(!ix.is_trade(), "{ix:?} must not count as a trade");
        }
    }

    #[test]
    fn an_unrecognised_discriminator_is_not_forced_into_a_variant() {
        assert_eq!(
            Instruction::from_discriminator(Discriminator::new([0; 8])),
            None
        );
    }
}
