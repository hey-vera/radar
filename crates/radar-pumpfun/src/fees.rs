// SPDX-License-Identifier: Apache-2.0
//! What a pump.fun trade pays on top of price impact.
//!
//! [research 0022](https://github.com/hey-vera/radar/blob/main/docs/research/0022-capacity-was-a-budget-not-a-ceiling.md)
//! costed the round trip with a 250 bps fee component, and said plainly that the
//! curve's own fee is not modelled there and is charged on top of impact -- so
//! those figures are optimistic by exactly this much. This module is that fee,
//! read from the chain rather than supplied by argument.
//!
//! # Two accounts disagree, and the newer one wins
//!
//! The global account carries `fee_basis_points` and `creator_fee_basis_points`.
//! The fee program's [`FeeConfig`] carries a tier schedule. On 2026-09-01 they
//! did not agree -- 100 bps against 125 -- and every `buy` and `sell` captured
//! from mainnet passes **both** the fee config and the fee program in its
//! account list, so the schedule is what the program consults.
//!
//! Radar therefore prices off [`FeeConfig`] and keeps [`global_fees`] only for
//! comparison. Nothing here averages them or falls back from one to the other:
//! rule 9's shape is that a fee which cannot be read is not a fee of zero, and
//! both parsers refuse rather than default.
//!
//! # Why this is not a constant
//!
//! It would be cheaper to write `125` in a file. The schedule is a **vector of
//! tiers keyed on market capitalisation**, published in an account whose admin
//! can update it, and it has already moved once -- the global field is the
//! fossil of the version before. A constant would be right until the day it
//! silently was not, and the direction it would be wrong in is the one that
//! makes a trade look cheaper than it is.

use crate::curve::Malformed;

/// The three fees a trade pays, in basis points.
///
/// Named as the program names them. `lp_bps` is zero on the bonding curve --
/// there are no liquidity providers before graduation -- and is kept rather than
/// dropped because it is non-zero on the AMM the curve graduates into.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Fees {
    /// To liquidity providers. Zero on the curve.
    pub lp_bps: u64,
    /// To the protocol.
    pub protocol_bps: u64,
    /// To whoever launched the token.
    pub creator_bps: u64,
}

impl Fees {
    /// Everything a trade pays, in basis points.
    ///
    /// One side of a round trip. A buy and a sell each pay this, which is why
    /// [`Self::round_trip_bps`] exists rather than leaving the doubling to a
    /// caller who might forget it.
    #[must_use]
    pub const fn total_bps(&self) -> u64 {
        self.lp_bps
            .saturating_add(self.protocol_bps)
            .saturating_add(self.creator_bps)
    }

    /// What entering and exiting costs in fees alone, in basis points.
    #[must_use]
    pub const fn round_trip_bps(&self) -> u64 {
        self.total_bps().saturating_mul(2)
    }

    /// The fee charged on `lamports`, rounded **up**.
    ///
    /// Up, for the reason every rounding decision in [`crate::curve`] goes the
    /// same way: a fee rounded down is a cost estimate rounded down, which 0019
    /// names as the direction that launders a trade past the risk kernel.
    ///
    /// Clamped at the amount it is charged on. A fee larger than the trade is
    /// nonsense, and clamping is the reading that cannot produce a negative
    /// proceed by underflow.
    #[must_use]
    pub fn charge(&self, lamports: u64) -> u64 {
        let paid = u128::from(lamports)
            .saturating_mul(u128::from(self.total_bps()))
            .div_ceil(10_000);
        u64::try_from(paid).unwrap_or(u64::MAX).min(lamports)
    }
}

/// One row of the fee schedule.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Tier {
    /// The market capitalisation, in lamports, at or above which this row applies.
    pub threshold_lamports: u128,
    /// What is charged there.
    pub fees: Fees,
}

/// The fee program's schedule.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FeeConfig {
    /// Charged on pools that are not pump's own.
    pub flat: Fees,
    /// The schedule for pump pools.
    pub tiers: Vec<Tier>,
}

/// The discriminator a fee-config account starts with.
///
/// Observed on mainnet, like every other discriminator in this crate.
pub const FEE_CONFIG_DISCRIMINATOR: [u8; 8] = [0x8f, 0x34, 0x92, 0xbb, 0xdb, 0x7b, 0x4c, 0x9b];

/// The discriminator the pump.fun global account starts with.
pub const GLOBAL_DISCRIMINATOR: [u8; 8] = [0xa7, 0xe8, 0xe8, 0xb1, 0xc8, 0x6c, 0x72, 0x7f];

fn u64_at(data: &[u8], at: usize) -> Option<u64> {
    let bytes: [u8; 8] = data.get(at..at.checked_add(8)?)?.try_into().ok()?;
    Some(u64::from_le_bytes(bytes))
}

fn fees_at(data: &[u8], at: usize) -> Option<Fees> {
    Some(Fees {
        lp_bps: u64_at(data, at)?,
        protocol_bps: u64_at(data, at.checked_add(8)?)?,
        creator_bps: u64_at(data, at.checked_add(16)?)?,
    })
}

fn discriminator(data: &[u8], want: [u8; 8]) -> Result<(), Malformed> {
    let head: [u8; 8] =
        data.get(..8)
            .and_then(|s| s.try_into().ok())
            .ok_or(Malformed::TooShort {
                len: data.len(),
                needed: 8,
            })?;
    if head == want {
        Ok(())
    } else {
        Err(Malformed::WrongDiscriminator { found: head })
    }
}

impl FeeConfig {
    /// Reads the schedule out of a fee-config account.
    ///
    /// # Errors
    ///
    /// [`Malformed`] when the discriminator is wrong or the account is shorter
    /// than the fields it claims. A truncated vector is refused rather than read
    /// as far as it goes: a schedule missing its top rows prices a large trade at
    /// a small trade's fee, which is the optimistic direction.
    pub fn parse(data: &[u8]) -> Result<Self, Malformed> {
        discriminator(data, FEE_CONFIG_DISCRIMINATOR)?;
        // 8 discriminator, 1 bump, 32 admin, then the flat fees.
        let mut at = 41usize;
        let short = |needed: usize| Malformed::TooShort {
            len: data.len(),
            needed,
        };
        let flat = fees_at(data, at).ok_or_else(|| short(at + 24))?;
        at += 24;
        let count = {
            let bytes: [u8; 4] = data
                .get(at..at + 4)
                .and_then(|s| s.try_into().ok())
                .ok_or_else(|| short(at + 4))?;
            u32::from_le_bytes(bytes) as usize
        };
        at += 4;
        // The capacity is bounded rather than trusted. `count` is four bytes read
        // out of an account, so an allocation sized by it is an allocation sized
        // by whatever those bytes happen to be.
        let mut tiers = Vec::with_capacity(count.min(64));
        for _ in 0..count {
            let bytes: [u8; 16] = data
                .get(at..at + 16)
                .and_then(|s| s.try_into().ok())
                .ok_or_else(|| short(at + 16))?;
            at += 16;
            let fees = fees_at(data, at).ok_or_else(|| short(at + 24))?;
            at += 24;
            tiers.push(Tier {
                threshold_lamports: u128::from_le_bytes(bytes),
                fees,
            });
        }
        Ok(Self { flat, tiers })
    }

    /// What a pump-pool trade pays at a given market capitalisation.
    ///
    /// The highest tier whose threshold the capitalisation reaches.
    ///
    /// `None` when no tier applies -- an empty schedule, or one whose lowest
    /// threshold is above this token. **Not a fee of zero.** A schedule that does
    /// not cover a trade is a fee that could not be read, and rule 9 says that is
    /// a refusal rather than free.
    #[must_use]
    pub fn fees_at_market_cap(&self, market_cap_lamports: u128) -> Option<Fees> {
        self.tiers
            .iter()
            .filter(|t| market_cap_lamports >= t.threshold_lamports)
            .max_by_key(|t| t.threshold_lamports)
            .map(|t| t.fees)
    }
}

/// The fee fields on the pump.fun global account.
///
/// Kept for comparison rather than for pricing. On 2026-09-01 this said 100 bps
/// while the fee schedule said 125, and a test asserts they still disagree -- so
/// the day pump.fun reconciles them, something fails loudly rather than Radar
/// quietly pricing off whichever one happened to be read.
///
/// # Errors
///
/// [`Malformed`] when the discriminator is wrong or the account is too short.
pub fn global_fees(data: &[u8]) -> Result<Fees, Malformed> {
    discriminator(data, GLOBAL_DISCRIMINATOR)?;
    let short = |needed: usize| Malformed::TooShort {
        len: data.len(),
        needed,
    };
    // 8 discriminator, 1 initialized, 32 authority, 32 fee_recipient, four u64
    // reserve fields, then fee_basis_points at 105. The creator fee sits past
    // withdraw_authority (32), enable_migrate (1) and pool_migration_fee (8).
    let protocol_bps = u64_at(data, 105).ok_or_else(|| short(113))?;
    let creator_bps = u64_at(data, 154).ok_or_else(|| short(162))?;
    Ok(Fees {
        lp_bps: 0,
        protocol_bps,
        creator_bps,
    })
}
