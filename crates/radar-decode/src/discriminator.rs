// SPDX-License-Identifier: Apache-2.0
//! Anchor instruction discriminators.

use core::fmt;

/// The first eight bytes of an Anchor instruction's data.
///
/// Anchor derives these as `sha256("global:" + snake_case_name)[..8]`. Radar
/// matches on these bytes rather than on the instruction names a program logs,
/// because log names get versioned — pump.fun runs `Buy`, `BuyV2`,
/// `BuyExactSolIn` and `BuyExactQuoteInV2` concurrently — and a matcher written
/// against one spelling silently reports the others as absent. That mistake was
/// made here once already; see LEARNINGS entry 3.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Discriminator([u8; 8]);

impl Discriminator {
    /// Wraps eight bytes.
    #[must_use]
    pub const fn new(bytes: [u8; 8]) -> Self {
        Self(bytes)
    }

    /// Reads the discriminator from the front of an instruction's data.
    ///
    /// Returns `None` for data shorter than eight bytes, which is malformed for
    /// an Anchor program and must not be guessed at.
    #[must_use]
    pub const fn from_data(data: &[u8]) -> Option<Self> {
        if data.len() < 8 {
            return None;
        }
        Some(Self([
            data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
        ]))
    }

    /// The bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 8] {
        &self.0
    }
}

impl fmt::Display for Discriminator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Discriminator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Discriminator({self})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_data_yields_no_discriminator_rather_than_a_guess() {
        assert!(Discriminator::from_data(&[1, 2, 3]).is_none());
        assert!(Discriminator::from_data(&[]).is_none());
    }

    #[test]
    fn exactly_eight_bytes_is_enough() {
        // Several pump.fun instructions carry no arguments at all.
        let d = Discriminator::from_data(&[1, 2, 3, 4, 5, 6, 7, 8]).expect("eight bytes");
        assert_eq!(d.as_bytes(), &[1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn trailing_arguments_do_not_change_the_discriminator() {
        let a = Discriminator::from_data(&[1, 2, 3, 4, 5, 6, 7, 8]).expect("eight");
        let b = Discriminator::from_data(&[1, 2, 3, 4, 5, 6, 7, 8, 99, 99]).expect("ten");
        assert_eq!(a, b);
    }

    #[test]
    fn display_is_hex_so_it_can_be_grepped_against_an_explorer() {
        let d = Discriminator::new([0xd6, 0x90, 0x4c, 0xec, 0x5f, 0x8b, 0x31, 0xb4]);
        assert_eq!(d.to_string(), "d6904cec5f8b31b4");
    }
}
