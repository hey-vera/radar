// SPDX-License-Identifier: Apache-2.0
//! Local decoding of Solana program instructions.
//!
//! This crate is the reason Radar can afford to look at every launch. Buying
//! transactions parsed costs $0.05 each; fetching the block they are in costs
//! $0.001 for all of them. Measured over 45 mainnet blocks that is a **4,637×**
//! difference, so decoding is the step worth owning — see
//! [ADR 0001](https://github.com/hey-vera/radar/blob/main/docs/adr/0001-decode-locally-never-buy-parsed-transactions.md).
//!
//! Two rules follow from owning it, and both exist because the alternative
//! produces confident wrong answers rather than visible failures:
//!
//! 1. **Match on discriminator bytes, never on logged instruction names.** Names
//!    get versioned. pump.fun runs `Buy`, `BuyV2`, `BuyExactSolIn` and
//!    `BuyExactQuoteInV2` concurrently, and a matcher written against one
//!    spelling reports the other three as absent.
//! 2. **An unrecognised discriminator is [`Decoded::Unknown`], never a guess.**
//!    A decoder that has silently stopped understanding a program looks exactly
//!    like a program that has gone quiet, so the unknown rate is a signal and
//!    has to be preserved.
//!
//! Every discriminator in this crate was captured from live mainnet traffic
//! (`scripts/probe/capture_fixtures.py`) rather than copied from documentation,
//! and `tests/` asserts the table still matches both the Anchor naming
//! convention and the raw bytes those instructions actually carried.

#![forbid(unsafe_code)]

mod discriminator;
pub mod pumpfun;

pub use discriminator::Discriminator;

/// The result of decoding an instruction.
///
/// The `Unknown` arm is load-bearing. Radar records unknown discriminators and
/// alarms when their rate rises, because that is what a program upgrade looks
/// like from the outside — and pump.fun ships them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Decoded<T> {
    /// Recognised.
    Known(T),
    /// Not in the table. Carried rather than discarded so the rate can be
    /// measured and the bytes chased down.
    Unknown {
        /// The eight bytes that were not recognised.
        discriminator: Discriminator,
        /// Total length of the instruction data, which narrows down what it is.
        data_len: usize,
    },
    /// Fewer than eight bytes, so not an Anchor instruction at all.
    Malformed {
        /// How many bytes were present.
        data_len: usize,
    },
}

impl<T> Decoded<T> {
    /// The instruction if it was recognised.
    pub const fn known(&self) -> Option<&T> {
        match self {
            Self::Known(t) => Some(t),
            _ => None,
        }
    }

    /// Whether this decode failed to recognise the instruction, for either
    /// reason. Both count toward the unknown rate that gates the alarm.
    pub const fn is_unrecognised(&self) -> bool {
        !matches!(self, Self::Known(_))
    }
}

/// Decodes a pump.fun instruction from its data.
#[must_use]
pub fn decode_pumpfun(data: &[u8]) -> Decoded<pumpfun::Instruction> {
    let Some(d) = Discriminator::from_data(data) else {
        return Decoded::Malformed {
            data_len: data.len(),
        };
    };
    pumpfun::Instruction::from_discriminator(d).map_or(
        Decoded::Unknown {
            discriminator: d,
            data_len: data.len(),
        },
        Decoded::Known,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_known_instruction_decodes() {
        let mut data = pumpfun::Instruction::Buy
            .discriminator()
            .as_bytes()
            .to_vec();
        data.extend_from_slice(&[0u8; 17]);
        assert_eq!(
            decode_pumpfun(&data),
            Decoded::Known(pumpfun::Instruction::Buy)
        );
    }

    #[test]
    fn an_unknown_discriminator_is_carried_not_guessed() {
        let data = [0xAAu8; 24];
        let d = decode_pumpfun(&data);
        assert!(d.is_unrecognised());
        let Decoded::Unknown {
            discriminator,
            data_len,
        } = d
        else {
            panic!("expected Unknown, got {d:?}")
        };
        assert_eq!(discriminator.to_string(), "aaaaaaaaaaaaaaaa");
        // The length is kept because it is often the fastest way to work out
        // which new instruction a program upgrade added.
        assert_eq!(data_len, 24);
    }

    #[test]
    fn short_data_is_malformed_rather_than_unknown() {
        // Distinct from Unknown: this is not an Anchor instruction at all, and
        // conflating the two would pollute the unknown rate that gates the alarm.
        assert_eq!(
            decode_pumpfun(&[1, 2, 3]),
            Decoded::Malformed { data_len: 3 }
        );
    }
}
