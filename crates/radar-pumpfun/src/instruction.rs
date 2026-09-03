// SPDX-License-Identifier: Apache-2.0
//! The instruction data a pump.fun trade carries.
//!
//! # What is observed and what is inferred, kept apart
//!
//! The **encoding** is observed: every byte layout below is reproduced from a
//! real mainnet instruction, and the tests assert the exact bytes rather than a
//! round trip. A round trip would pass just as well against a layout the program
//! has never seen.
//!
//! The **names** are inferred, from the argument values and the accounts the
//! instruction carries — a `track_volume` flag beside a
//! `user_volume_accumulator` account is a reasonable reading, and it is a
//! reading. Where a name is a guess this file says so, because ADR 0009's
//! standard is that what comes from mainnet and what comes from inference are
//! not the same thing.
//!
//! # The discriminator comes from the decoder
//!
//! ADR 0009 precondition 2. A builder that learned its own discriminators could
//! come to disagree with `radar-decode` about the same program, and the
//! disagreement would be invisible until a transaction was rejected. So the
//! bytes are looked up in `radar_decode::pumpfun::KNOWN` by the instruction's
//! own name, and a lookup that fails is a refusal rather than a fallback.

use radar_decode::pumpfun::{Instruction, KNOWN};

/// A trade against the bonding curve.
///
/// Three variants because mainnet carries three, and they are not
/// interchangeable: the first fixes what you *receive*, the second what you
/// *spend*, and only the third exits.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Trade {
    /// Buy an exact number of tokens, spending no more than `max_sol_cost`.
    ///
    /// The ceiling is the slippage bound: the program refuses rather than
    /// filling above it.
    Buy {
        /// Tokens to receive, in base units.
        amount_tokens: u64,
        /// The most lamports that may be spent.
        max_sol_cost: u64,
        /// The trailing flag. **Name inferred**, encoding observed — it sits
        /// beside the volume-accumulator accounts and was `1` on the captured
        /// buy and `0` on the captured `buy_exact_sol_in`.
        track_volume: bool,
    },
    /// Spend an exact number of lamports, receiving whatever that buys.
    ///
    /// The common instruction on this program, and the one that matches how
    /// Radar sizes: it decides a notional, not a token count.
    BuyExactSolIn {
        /// Lamports to spend.
        lamports: u64,
        /// The slippage allowance. **Name inferred**: the captured value was
        /// 500, which reads as basis points beside a 3.5 SOL order.
        slippage_bps: u64,
        /// See [`Trade::Buy::track_volume`].
        track_volume: bool,
    },
    /// Sell tokens for at least `min_sol_output`.
    ///
    /// The exit, and the half Radar's whole thesis rests on.
    Sell {
        /// Tokens to sell, in base units.
        amount_tokens: u64,
        /// The fewest lamports that may be received.
        ///
        /// Zero on the captured instruction, which is a trader accepting any
        /// price. Radar should not: a floor of zero is a sell with no slippage
        /// bound at all.
        min_sol_output: u64,
    },
}

impl Trade {
    /// Which decoded instruction this builds.
    #[must_use]
    pub const fn instruction(&self) -> Instruction {
        match self {
            Self::Buy { .. } => Instruction::Buy,
            Self::BuyExactSolIn { .. } => Instruction::BuyExactSolIn,
            Self::Sell { .. } => Instruction::Sell,
        }
    }

    /// The eight-byte discriminator, from the decoder's table.
    ///
    /// `None` when the table does not carry it, which cannot happen for these
    /// three and is returned rather than panicked because a builder that
    /// panics on a table change is worse than one that refuses.
    #[must_use]
    pub fn discriminator(&self) -> Option<[u8; 8]> {
        let wanted = self.instruction();
        KNOWN
            .iter()
            .find(|(ix, _, _)| *ix == wanted)
            .map(|(_, bytes, _)| *bytes)
    }

    /// The full instruction data: discriminator followed by arguments.
    ///
    /// Little-endian `u64`s, which is Solana's encoding throughout — and the
    /// tests assert the exact captured bytes, so a big-endian slip fails here
    /// rather than by transferring a wildly different amount.
    #[must_use]
    pub fn data(&self) -> Option<Vec<u8>> {
        let mut out = Vec::with_capacity(25);
        out.extend_from_slice(&self.discriminator()?);
        match *self {
            Self::Buy {
                amount_tokens,
                max_sol_cost,
                track_volume,
            } => {
                out.extend_from_slice(&amount_tokens.to_le_bytes());
                out.extend_from_slice(&max_sol_cost.to_le_bytes());
                out.push(u8::from(track_volume));
            }
            Self::BuyExactSolIn {
                lamports,
                slippage_bps,
                track_volume,
            } => {
                out.extend_from_slice(&lamports.to_le_bytes());
                out.extend_from_slice(&slippage_bps.to_le_bytes());
                out.push(u8::from(track_volume));
            }
            Self::Sell {
                amount_tokens,
                min_sol_output,
            } => {
                out.extend_from_slice(&amount_tokens.to_le_bytes());
                out.extend_from_slice(&min_sol_output.to_le_bytes());
            }
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Instruction data lifted verbatim from mainnet transactions on
    /// 2026-09-01, alongside the account layouts in
    /// `radar-decode/tests/fixtures/pumpfun_accounts.json`.
    const OBSERVED_BUY: &str = "66063d1201daebea68461a7418000000407c43000000000001";
    const OBSERVED_BUY_EXACT: &str = "38fc74089edfcd5f00c39dd000000000f40100000000000000";
    const OBSERVED_SELL: &str = "33e685a4017f83ad7bd4d7c31e0000000000000000000000";

    fn hex(bytes: &[u8]) -> String {
        use core::fmt::Write as _;
        bytes.iter().fold(String::new(), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
    }

    #[test]
    fn a_buy_encodes_to_the_bytes_mainnet_carried() {
        // Byte-for-byte against a real instruction, not a round trip. A round
        // trip passes just as well against a layout the program has never seen.
        let built = Trade::Buy {
            amount_tokens: 105_027_094_120,
            max_sol_cost: 4_422_720,
            track_volume: true,
        }
        .data()
        .expect("a discriminator");
        assert_eq!(hex(&built), OBSERVED_BUY);
    }

    #[test]
    fn a_buy_of_an_exact_sol_amount_encodes_to_the_bytes_mainnet_carried() {
        let built = Trade::BuyExactSolIn {
            lamports: 3_500_000_000,
            slippage_bps: 500,
            track_volume: false,
        }
        .data()
        .expect("a discriminator");
        assert_eq!(hex(&built), OBSERVED_BUY_EXACT);
    }

    #[test]
    fn a_sell_encodes_to_the_bytes_mainnet_carried() {
        // The exit, and one argument shorter than a buy -- no trailing flag.
        // Assuming symmetry would append a byte the program does not expect.
        let built = Trade::Sell {
            amount_tokens: 132_134_720_635,
            min_sol_output: 0,
        }
        .data()
        .expect("a discriminator");
        assert_eq!(hex(&built), OBSERVED_SELL);
        assert_eq!(built.len(), 24, "sell is 8 + 8 + 8, with no trailing flag");
    }

    #[test]
    fn the_discriminators_come_from_the_decoders_table() {
        // ADR 0009 precondition 2. If this crate held its own copy, the two
        // could drift and nothing would notice until a transaction was
        // rejected.
        for (trade, name) in [
            (
                Trade::Buy {
                    amount_tokens: 1,
                    max_sol_cost: 1,
                    track_volume: false,
                },
                "buy",
            ),
            (
                Trade::BuyExactSolIn {
                    lamports: 1,
                    slippage_bps: 1,
                    track_volume: false,
                },
                "buy_exact_sol_in",
            ),
            (
                Trade::Sell {
                    amount_tokens: 1,
                    min_sol_output: 1,
                },
                "sell",
            ),
        ] {
            let disc = trade.discriminator().expect("in the table");
            let (_, table_bytes, table_name) = KNOWN
                .iter()
                .find(|(_, d, _)| *d == disc)
                .expect("the table knows it");
            assert_eq!(*table_name, name);
            assert_eq!(*table_bytes, disc);
        }
    }

    #[test]
    fn the_numbers_are_little_endian() {
        // Solana's encoding throughout. A big-endian slip would still produce
        // 24 well-formed bytes and would transfer a wildly different amount.
        let built = Trade::Sell {
            amount_tokens: 1,
            min_sol_output: 0,
        }
        .data()
        .expect("data");
        assert_eq!(&built[8..16], &[1, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn the_two_buy_variants_are_not_interchangeable() {
        // One fixes what you receive and the other what you spend. They carry
        // the same argument *shape*, so a mixed-up call site produces valid
        // bytes for the wrong instruction -- which is why the discriminators
        // are asserted to differ.
        let a = Trade::Buy {
            amount_tokens: 100,
            max_sol_cost: 200,
            track_volume: true,
        };
        let b = Trade::BuyExactSolIn {
            lamports: 100,
            slippage_bps: 200,
            track_volume: true,
        };
        assert_ne!(a.discriminator(), b.discriminator());
        assert_eq!(
            a.data().expect("a")[8..],
            b.data().expect("b")[8..],
            "the arguments really are the same shape, which is the hazard"
        );
    }

    #[test]
    fn a_zero_floor_on_a_sell_is_representable_and_should_not_be_used() {
        // The captured sell had `min_sol_output: 0` -- a trader accepting any
        // price. This type can express that because mainnet does, and the field
        // documentation says why Radar must not: a floor of zero is a sell with
        // no slippage bound at all.
        let data = Trade::Sell {
            amount_tokens: 1,
            min_sol_output: 0,
        }
        .data()
        .expect("data");
        assert_eq!(&data[16..24], &[0u8; 8]);
    }
}
