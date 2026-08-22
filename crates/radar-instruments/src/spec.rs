// SPDX-License-Identifier: Apache-2.0
//! What an instrument declares about itself.

use radar_types::{MicroUsd, Mutability};
use serde::{Deserialize, Serialize};

/// How long an instrument takes, and therefore where it may be used.
///
/// This is a promise, not a measurement. An instrument that declares itself hot
/// and then makes a paid call has mis-declared, and the recorded latency will
/// say so.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Latency {
    /// Under ~50ms. Local data only. Safe on a decision path.
    Hot,
    /// Under ~1s. May touch a fast local index.
    Warm,
    /// Seconds. May make paid calls, which on the x402 lane settle on-chain
    /// before responding — so a cold instrument must never gate an execution.
    Cold,
}

/// Whether an instrument returns the same answer for the same inputs.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Determinism {
    /// A pure function of `(arguments, as_of)` and the recorded store.
    ///
    /// Only these can be replayed and compared byte for byte, which is what the
    /// leakage test relies on.
    Pure,
    /// Depends on a live source. Replaying it may legitimately differ, so a
    /// difference proves nothing and the leakage test skips it.
    Live,
}

/// What one invocation costs.
///
/// Split because the two halves behave differently: compute is ours and scales
/// with hardware, upstream is a vendor's and scales with the bill. An instrument
/// whose upstream cost is zero can be called freely; one whose is not needs a
/// reason each time.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
pub struct Cost {
    /// Expected spend on paid upstream calls.
    pub upstream: MicroUsd,
    /// Notional cost of the compute, for ranking rather than billing.
    pub compute: MicroUsd,
}

impl Cost {
    /// Costs nothing but local work.
    pub const FREE: Self = Self {
        upstream: MicroUsd::ZERO,
        compute: MicroUsd::ZERO,
    };

    /// A cost that is entirely upstream spend.
    #[must_use]
    pub const fn upstream(amount: MicroUsd) -> Self {
        Self {
            upstream: amount,
            compute: MicroUsd::ZERO,
        }
    }

    /// Total expected cost.
    #[must_use]
    pub const fn total(self) -> MicroUsd {
        self.upstream.saturating_add(self.compute)
    }
}

/// A semantic version.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, PartialOrd, Ord)]
pub struct Version {
    /// Breaking changes to the output shape or meaning.
    pub major: u16,
    /// Additive changes.
    pub minor: u16,
}

impl Version {
    /// A version.
    #[must_use]
    pub const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }
}

impl core::fmt::Display for Version {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

/// Everything an instrument declares about itself.
///
/// Declared once, and every surface is derived from it: the internal call, the
/// HTTP endpoint, the x402 price, and the MCP tool description. That is the point
/// — three surfaces that each carried their own copy of the price and the schema
/// would drift, and the one that drifted would be the paid one.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Spec {
    /// Stable identifier. Also the MCP tool name and the last path segment of the
    /// HTTP route, so it must be a valid identifier in all three.
    pub name: &'static str,
    /// Version of the output contract.
    pub version: Version,
    /// One line, shown to an operator and to a model choosing tools.
    pub summary: &'static str,
    /// Where this may be used.
    pub latency: Latency,
    /// What one call is expected to cost.
    pub cost: Cost,
    /// How quickly this instrument's answer goes stale, which decides how long a
    /// result may be cached and served.
    pub freshness: Mutability,
    /// Whether replaying it must reproduce the same answer.
    pub determinism: Determinism,
}

impl Spec {
    /// The price to charge an external caller.
    ///
    /// Cost plus margin, floored so that a free instrument still cannot be used
    /// as an unmetered denial-of-service. Derived rather than configured: a
    /// price list maintained separately from the cost model is a price list that
    /// eventually sells below cost.
    #[must_use]
    pub fn public_price(&self, margin_percent: u64) -> MicroUsd {
        let base = self.cost.total();
        let marked_up =
            base.saturating_add(MicroUsd(base.get().saturating_mul(margin_percent) / 100));
        MicroUsd(marked_up.get().max(MIN_PUBLIC_PRICE.get()))
    }

    /// Whether this instrument may be called on an execution path.
    ///
    /// Cold instruments may make paid calls, and on the x402 lane a paid call
    /// settles on-chain before the response returns — hundreds of milliseconds
    /// that must never sit between a decision and a submission.
    #[must_use]
    pub const fn safe_on_execution_path(&self) -> bool {
        matches!(self.latency, Latency::Hot) && self.cost.upstream.get() == 0
    }
}

/// The floor for any public price. A free instrument still costs us the request.
pub const MIN_PUBLIC_PRICE: MicroUsd = MicroUsd(1_000);

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(latency: Latency, cost: Cost) -> Spec {
        Spec {
            name: "example",
            version: Version::new(1, 0),
            summary: "an example",
            latency,
            cost,
            freshness: Mutability::Fast,
            determinism: Determinism::Pure,
        }
    }

    #[test]
    fn a_free_instrument_still_has_a_price_floor() {
        // Otherwise the public surface is an unmetered way to make us do work.
        let s = spec(Latency::Hot, Cost::FREE);
        assert_eq!(s.public_price(50), MIN_PUBLIC_PRICE);
    }

    #[test]
    fn the_price_is_derived_from_the_cost_model_rather_than_configured() {
        // A price list maintained apart from the cost model eventually sells
        // below cost, and the drift is invisible until the bill arrives.
        let s = spec(Latency::Cold, Cost::upstream(MicroUsd::from_dollars(0.02)));
        assert_eq!(s.public_price(50), MicroUsd::from_dollars(0.03));
        assert_eq!(s.public_price(0), MicroUsd::from_dollars(0.02));
    }

    #[test]
    fn only_hot_free_instruments_are_allowed_on_an_execution_path() {
        // A paid call on the x402 lane settles on-chain before responding. That
        // latency must never sit between a decision and a submission.
        assert!(spec(Latency::Hot, Cost::FREE).safe_on_execution_path());
        assert!(!spec(Latency::Warm, Cost::FREE).safe_on_execution_path());
        assert!(!spec(Latency::Cold, Cost::FREE).safe_on_execution_path());
        assert!(
            !spec(Latency::Hot, Cost::upstream(MicroUsd(1))).safe_on_execution_path(),
            "hot but paid is still paid"
        );
    }

    #[test]
    fn latency_classes_order_from_fastest() {
        assert!(Latency::Hot < Latency::Warm);
        assert!(Latency::Warm < Latency::Cold);
    }
}
