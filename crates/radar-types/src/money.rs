// SPDX-License-Identifier: Apache-2.0
//! Integer money for cost accounting.

use core::fmt;
use core::iter::Sum;
use core::ops::{Add, AddAssign, Mul};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// US dollars in millionths.
///
/// Radar sums call costs across millions of invocations and compares the total
/// against a hard budget cap that is allowed to stop trading. Floating point
/// accumulates error over exactly that kind of summation, so the unit is an
/// integer and the smallest representable amount is one micro-dollar — a
/// thousandth of the cheapest call in the catalogue.
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema, Default,
)]
#[serde(transparent)]
pub struct MicroUsd(pub u64);

impl MicroUsd {
    /// Free.
    pub const ZERO: Self = Self(0);

    /// One US dollar.
    pub const DOLLAR: Self = Self(1_000_000);

    /// Whole micro-dollars.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Builds an amount from a decimal dollar figure, rounding to the nearest
    /// micro-dollar. For reading config and vendor price lists, which are written
    /// in dollars; never for arithmetic on an amount already in hand.
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "non-finite and non-positive inputs return early on the line above the cast"
    )]
    pub fn from_dollars(usd: f64) -> Self {
        if !usd.is_finite() || usd <= 0.0 {
            return Self::ZERO;
        }
        Self((usd * 1_000_000.0).round() as u64)
    }

    /// Saturating addition. A budget meter that panics mid-trade is worse than
    /// one that pins at the maximum and trips every cap it is checked against.
    #[must_use]
    pub const fn saturating_add(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }

    /// This amount repeated `n` times, saturating.
    #[must_use]
    pub const fn saturating_mul(self, n: u64) -> Self {
        Self(self.0.saturating_mul(n))
    }
}

impl Add for MicroUsd {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        self.saturating_add(other)
    }
}

impl AddAssign for MicroUsd {
    fn add_assign(&mut self, other: Self) {
        *self = self.saturating_add(other);
    }
}

impl Mul<u64> for MicroUsd {
    type Output = Self;
    fn mul(self, n: u64) -> Self {
        self.saturating_mul(n)
    }
}

impl Sum for MicroUsd {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Self::ZERO, Self::saturating_add)
    }
}

impl fmt::Display for MicroUsd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "${}.{:06}", self.0 / 1_000_000, self.0 % 1_000_000)
    }
}

impl fmt::Debug for MicroUsd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MicroUsd({self})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vendor_prices_parse_exactly() {
        // The four prices the whole cost model rests on.
        assert_eq!(MicroUsd::from_dollars(0.001), MicroUsd(1_000));
        assert_eq!(MicroUsd::from_dollars(0.005), MicroUsd(5_000));
        assert_eq!(MicroUsd::from_dollars(0.01), MicroUsd(10_000));
        assert_eq!(MicroUsd::from_dollars(0.05), MicroUsd(50_000));
    }

    #[test]
    fn a_million_cheap_calls_sum_without_drift() {
        // The same sum in f64 lands just off 1000.0. This is the entire reason
        // the type is an integer.
        let total: MicroUsd = std::iter::repeat_n(MicroUsd::from_dollars(0.001), 1_000_000).sum();
        assert_eq!(total, MicroUsd(1_000_000_000));
        assert_eq!(total.to_string(), "$1000.000000");
    }

    #[test]
    fn nonsense_dollar_figures_become_zero_rather_than_garbage() {
        assert_eq!(MicroUsd::from_dollars(f64::NAN), MicroUsd::ZERO);
        assert_eq!(MicroUsd::from_dollars(f64::INFINITY), MicroUsd::ZERO);
        assert_eq!(MicroUsd::from_dollars(-1.0), MicroUsd::ZERO);
    }

    #[test]
    fn addition_saturates_rather_than_overflowing() {
        assert_eq!(MicroUsd(u64::MAX) + MicroUsd(1), MicroUsd(u64::MAX));
    }
}
