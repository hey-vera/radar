// SPDX-License-Identifier: Apache-2.0
//! The pump.fun bonding-curve program.
//!
//! Every discriminator here was observed on mainnet and none were copied from a
//! reference. `tests/fixtures/pumpfun_instructions.json` carries the raw sample
//! each one came from, sourced from CryptoHouse (ADR 0002) so that rare
//! instructions are found rather than missed — an earlier RPC-sampled capture saw
//! only 14 of the 25 discriminators this program actually emits.
//!
//! Public references describe a program with roughly three instructions. The live
//! one has twenty-one named, four distinct buy variants, two launch paths, and a
//! per-user volume accumulator.

use radar_types::Address;

use crate::discriminator::Discriminator;

/// The pump.fun program address, `6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P`.
pub const PROGRAM_ID: Address = Address::new([
    0x01, 0x56, 0xe0, 0xf6, 0x93, 0x66, 0x5a, 0xcf, 0x44, 0xdb, 0x15, 0x68, 0xbf, 0x17, 0x5b, 0xaa,
    0x51, 0x89, 0xcb, 0x97, 0xf5, 0xd2, 0xff, 0x3b, 0x65, 0x5d, 0x2b, 0xb6, 0xfd, 0x6d, 0x18, 0xb0,
]);

/// Anchor's event-CPI tag.
///
/// Not derived from an instruction name — it is a fixed constant a program
/// self-CPIs with to emit a structured event. On pump.fun it is the second
/// highest-volume discriminator of all (382k in six hours, 80–707 byte payloads)
/// and it carries the trade details that would otherwise have to be
/// reconstructed from account balance deltas.
pub const ANCHOR_EVENT_CPI: Discriminator =
    Discriminator::new([0xe4, 0x45, 0xa5, 0x2e, 0x51, 0xcb, 0x9a, 0x1d]);

/// A pump.fun instruction, identified by discriminator.
///
/// Ordered by observed frequency, which is also roughly the order in which they
/// matter.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum Instruction {
    /// `buy_exact_sol_in` — by far the most common instruction on the program.
    BuyExactSolIn,
    /// `sell`
    Sell,
    /// `buy`
    Buy,
    /// `buy_exact_quote_in_v2`
    BuyExactQuoteInV2,
    /// `sell_v2`
    SellV2,
    /// `close_user_volume_accumulator`
    CloseUserVolumeAccumulator,
    /// `claim_cashback_v2`
    ClaimCashbackV2,
    /// `buy_v2`
    BuyV2,
    /// `init_user_volume_accumulator`
    InitUserVolumeAccumulator,
    /// `claim_cashback`
    ClaimCashback,
    /// `create_v2` — the current launch path. Carries name, symbol and URI, and
    /// in most launches performs the developer's first buy in the same
    /// transaction.
    CreateV2,
    /// `extend_account`
    ExtendAccount,
    /// `collect_creator_fee`
    CollectCreatorFee,
    /// `collect_creator_fee_v2`
    CollectCreatorFeeV2,
    /// `distribute_creator_fees`
    DistributeCreatorFees,
    /// `distribute_creator_fees_v2`
    DistributeCreatorFeesV2,
    /// `migrate_v2` — graduation to the AMM.
    MigrateV2,
    /// `sync_user_volume_accumulator`
    SyncUserVolumeAccumulator,
    /// `create` — the original launch path. Still live and still producing real
    /// launches, which is why [`is_launch`](Self::is_launch) must cover it.
    Create,
    /// `migrate` — the original graduation path.
    Migrate,
    /// `admin_set_creator`
    AdminSetCreator,
}

/// Every known instruction with its discriminator and Anchor name.
///
/// Bytes are ground truth, captured from mainnet. The name is carried so a test
/// can recompute `sha256("global:" + name)[..8]` and prove the table has not
/// drifted from the convention it claims to follow.
pub const KNOWN: &[(Instruction, [u8; 8], &str)] = &[
    (
        Instruction::BuyExactSolIn,
        [0x38, 0xfc, 0x74, 0x08, 0x9e, 0xdf, 0xcd, 0x5f],
        "buy_exact_sol_in",
    ),
    (
        Instruction::Sell,
        [0x33, 0xe6, 0x85, 0xa4, 0x01, 0x7f, 0x83, 0xad],
        "sell",
    ),
    (
        Instruction::Buy,
        [0x66, 0x06, 0x3d, 0x12, 0x01, 0xda, 0xeb, 0xea],
        "buy",
    ),
    (
        Instruction::BuyExactQuoteInV2,
        [0xc2, 0xab, 0x1c, 0x46, 0x68, 0x4d, 0x5b, 0x2f],
        "buy_exact_quote_in_v2",
    ),
    (
        Instruction::SellV2,
        [0x5d, 0xf6, 0x82, 0x3c, 0xe7, 0xe9, 0x40, 0xb2],
        "sell_v2",
    ),
    (
        Instruction::CloseUserVolumeAccumulator,
        [0xf9, 0x45, 0xa4, 0xda, 0x96, 0x67, 0x54, 0x8a],
        "close_user_volume_accumulator",
    ),
    (
        Instruction::ClaimCashbackV2,
        [0x7a, 0xf3, 0xcc, 0x41, 0x5e, 0x74, 0x1d, 0x37],
        "claim_cashback_v2",
    ),
    (
        Instruction::BuyV2,
        [0xb8, 0x17, 0xee, 0x61, 0x67, 0xc5, 0xd3, 0x3d],
        "buy_v2",
    ),
    (
        Instruction::InitUserVolumeAccumulator,
        [0x5e, 0x06, 0xca, 0x73, 0xff, 0x60, 0xe8, 0xb7],
        "init_user_volume_accumulator",
    ),
    (
        Instruction::ClaimCashback,
        [0x25, 0x3a, 0x23, 0x7e, 0xbe, 0x35, 0xe4, 0xc5],
        "claim_cashback",
    ),
    (
        Instruction::CreateV2,
        [0xd6, 0x90, 0x4c, 0xec, 0x5f, 0x8b, 0x31, 0xb4],
        "create_v2",
    ),
    (
        Instruction::ExtendAccount,
        [0xea, 0x66, 0xc2, 0xcb, 0x96, 0x48, 0x3e, 0xe5],
        "extend_account",
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
        Instruction::DistributeCreatorFeesV2,
        [0xff, 0xcb, 0x13, 0x4f, 0xf4, 0x44, 0x08, 0x9f],
        "distribute_creator_fees_v2",
    ),
    (
        Instruction::MigrateV2,
        [0xbb, 0xcb, 0x12, 0x1f, 0xce, 0xed, 0xfe, 0x29],
        "migrate_v2",
    ),
    (
        Instruction::SyncUserVolumeAccumulator,
        [0x56, 0x1f, 0xc0, 0x57, 0xa3, 0x57, 0x4f, 0xee],
        "sync_user_volume_accumulator",
    ),
    (
        Instruction::Create,
        [0x18, 0x1e, 0xc8, 0x28, 0x05, 0x1c, 0x07, 0x77],
        "create",
    ),
    (
        Instruction::Migrate,
        [0x9b, 0xea, 0xe7, 0x92, 0xec, 0x9e, 0xa2, 0x1e],
        "migrate",
    ),
    (
        Instruction::AdminSetCreator,
        [0x45, 0x19, 0xab, 0x8e, 0x39, 0xef, 0x0d, 0x04],
        "admin_set_creator",
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
    /// Ask this rather than comparing against a variant. Four buy instructions
    /// run concurrently, and code that checks for one is wrong about the other
    /// three — which is how a coordination detector ends up reporting a base rate
    /// of zero for its strongest signal. See LEARNINGS entry 3.
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

    /// Whether this instruction creates a token, across **both** launch paths.
    ///
    /// `create` predates `create_v2` and is still producing real launches. A
    /// check for `CreateV2` alone silently drops them, and a launch Radar never
    /// saw is one it can never evaluate.
    #[must_use]
    pub const fn is_launch(self) -> bool {
        matches!(self, Self::CreateV2 | Self::Create)
    }

    /// Whether this instruction graduates a token to the AMM, across both paths.
    #[must_use]
    pub const fn is_graduation(self) -> bool {
        matches!(self, Self::MigrateV2 | Self::Migrate)
    }

    /// Whether this instruction moves a position.
    ///
    /// Fee collection, cashback claims and accumulator bookkeeping do not, and
    /// counting them as activity overstates how much is happening around a token
    /// — which matters because they are numerous: accumulator and cashback
    /// instructions together outnumber `create_v2` more than tenfold.
    #[must_use]
    pub const fn is_trade(self) -> bool {
        self.is_buy() || self.is_sell()
    }

    /// Whether this instruction is administrative bookkeeping rather than
    /// user-driven activity.
    #[must_use]
    pub const fn is_bookkeeping(self) -> bool {
        matches!(
            self,
            Self::ClaimCashback
                | Self::ClaimCashbackV2
                | Self::CollectCreatorFee
                | Self::CollectCreatorFeeV2
                | Self::DistributeCreatorFees
                | Self::DistributeCreatorFeesV2
                | Self::InitUserVolumeAccumulator
                | Self::CloseUserVolumeAccumulator
                | Self::SyncUserVolumeAccumulator
                | Self::ExtendAccount
                | Self::AdminSetCreator
        )
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
        let buys: Vec<_> = KNOWN
            .iter()
            .map(|(ix, _, _)| *ix)
            .filter(|ix| ix.is_buy())
            .collect();
        assert_eq!(buys.len(), 4, "expected four buy variants, got {buys:?}");
    }

    #[test]
    fn both_launch_paths_count_as_launches() {
        // `create` predates `create_v2` and is still live. A launch Radar never
        // saw is one it can never evaluate.
        let launches: Vec<_> = KNOWN
            .iter()
            .map(|(ix, _, _)| *ix)
            .filter(|ix| ix.is_launch())
            .collect();
        assert_eq!(
            launches.len(),
            2,
            "expected two launch paths, got {launches:?}"
        );
        assert!(launches.contains(&Instruction::Create));
        assert!(launches.contains(&Instruction::CreateV2));
    }

    #[test]
    fn both_graduation_paths_count() {
        let g: Vec<_> = KNOWN
            .iter()
            .map(|(ix, _, _)| *ix)
            .filter(|ix| ix.is_graduation())
            .collect();
        assert_eq!(g.len(), 2, "expected two graduation paths, got {g:?}");
    }

    #[test]
    fn bookkeeping_is_never_a_trade_and_trades_are_never_bookkeeping() {
        for (ix, _, _) in KNOWN {
            assert!(
                !(ix.is_trade() && ix.is_bookkeeping()),
                "{ix:?} is classified as both a trade and bookkeeping"
            );
        }
    }

    #[test]
    fn every_instruction_is_classified_as_something() {
        // An instruction that is neither a trade, a launch, a graduation nor
        // bookkeeping is one nobody has thought about, and it will be silently
        // ignored by every consumer.
        for (ix, _, _) in KNOWN {
            assert!(
                ix.is_trade() || ix.is_launch() || ix.is_graduation() || ix.is_bookkeeping(),
                "{ix:?} falls into no category"
            );
        }
    }

    #[test]
    fn the_anchor_event_tag_is_not_an_instruction() {
        // It is a self-CPI marker, not something a user submits. Treating it as
        // an instruction would double-count every trade that emits an event.
        assert_eq!(Instruction::from_discriminator(ANCHOR_EVENT_CPI), None);
    }

    #[test]
    fn an_unrecognised_discriminator_is_not_forced_into_a_variant() {
        assert_eq!(
            Instruction::from_discriminator(Discriminator::new([0; 8])),
            None
        );
    }
}
