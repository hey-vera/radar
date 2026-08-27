// SPDX-License-Identifier: Apache-2.0
//! The metered, cached, health-aware data plane.
//!
//! Radar buys most of its data per call, so every read is a purchase decision.
//! This crate makes that decision, and it makes it as **pure policy**: no HTTP,
//! no clock, no async. Instants and accounting days are arguments, the transport
//! is the caller's problem, and the result is a [`Plan`] describing what should
//! happen next.
//!
//! That shape is deliberate. A component authorised to spend real money is one
//! that has to be exhaustively testable without a network, and one whose
//! refusals have to be reproducible from a recording. Every rule here is
//! exercised by a unit test that runs in microseconds.
//!
//! The design is borrowed from ClawNet's two-layer cache model and rebuilt:
//! mutability is a type rather than a config value, the one-way latch is
//! enforced by the compiler, costs are integers, and the pieces ClawNet
//! documented but never shipped — cost-weighted eviction, conditional
//! revalidation — are here and tested.
//!
//! ```
//! use radar_asof::AsOf;
//! use radar_provider::{Budget, Offer, Planner, Plan, ProviderId, Request};
//! use radar_types::{MicroUsd, Mutability, Slot};
//! use serde_json::json;
//!
//! let budget = Budget {
//!     per_call_max: MicroUsd::from_dollars(0.10),
//!     daily_max: MicroUsd::from_dollars(5.00),
//! };
//! let mut planner = Planner::new(budget, 1_000, 0);
//! let helius = ProviderId::new("clawapis:helius-rpc");
//! let offers = [Offer::full_only(helius.clone(), MicroUsd::from_dollars(0.001))];
//!
//! // A token's structural facts never change, so the second read is free.
//! let request = Request::new("token_structure", json!({"mint": "So111"}), Mutability::Immutable);
//!
//! let plan = planner.plan(&request, AsOf::at(Slot(100)), &offers, 0, 0);
//! let Plan::Call { commitment, .. } = plan else { panic!("first read must fetch") };
//! planner.record_success(&request, commitment, b"...".to_vec(), Slot(100), MicroUsd::from_dollars(0.001), 900);
//!
//! let plan = planner.plan(&request, AsOf::at(Slot(999_999)), &offers, 0, 0);
//! assert!(matches!(plan, Plan::Serve(_)));
//! assert_eq!(planner.spent_today(), MicroUsd::from_dollars(0.001));
//! ```

#![forbid(unsafe_code)]

mod cache;
mod cost;
mod health;

use std::collections::HashMap;

use radar_asof::AsOf;
use radar_types::{Latch, MicroUsd, Mutability, Slot};
use serde_json::Value;

pub use cache::{Cache, CacheKey, Decision, Entry, Stats};
pub use cost::{Budget, Commitment, Ledger, Meter, Refusal};
pub use health::{Breaker, BreakerConfig, State};

/// Identifies a provider lane: a vendor endpoint, a flat-rate account, or a
/// local decoder.
#[derive(Clone, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct ProviderId(String);

impl ProviderId {
    /// Names a provider.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// The name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What a provider charges for a piece of data.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Offer {
    /// Which provider.
    pub provider: ProviderId,
    /// Price of a full fetch.
    pub full: MicroUsd,
    /// Price of a conditional request that comes back unchanged, if the provider
    /// supports one. `None` means it does not, and a revalidation there costs
    /// the same as a fetch.
    pub conditional: Option<MicroUsd>,
}

impl Offer {
    /// An offer from a provider with no conditional-request support — the common
    /// case today, and the reason request #11 to clawapis is worth making.
    #[must_use]
    pub const fn full_only(provider: ProviderId, full: MicroUsd) -> Self {
        Self {
            provider,
            full,
            conditional: None,
        }
    }

    /// What this offer charges for the given kind of call.
    #[must_use]
    pub fn price(&self, kind: CallKind) -> MicroUsd {
        match kind {
            CallKind::Full => self.full,
            CallKind::Conditional { .. } => self.conditional.unwrap_or(self.full),
        }
    }
}

/// A request for a piece of data.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Request {
    /// Namespace, typically the instrument or RPC method name.
    pub namespace: String,
    /// Arguments. Canonicalised before hashing, so key order does not matter.
    pub args: Value,
    /// How often this fact can change.
    pub mutability: Mutability,
}

impl Request {
    /// Describes a request.
    #[must_use]
    pub fn new(namespace: impl Into<String>, args: Value, mutability: Mutability) -> Self {
        Self {
            namespace: namespace.into(),
            args,
            mutability,
        }
    }

    /// The cache key for this request.
    #[must_use]
    pub fn key(&self) -> CacheKey {
        CacheKey::new(&self.namespace, &self.args)
    }
}

/// The kind of call a plan calls for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CallKind {
    /// Fetch the whole thing.
    Full,
    /// Ask whether it changed, sending this validator.
    Conditional {
        /// The content hash last seen.
        prior_hash: [u8; 32],
    },
}

/// Why no call could be planned.
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum Blocked {
    /// Every provider offering this data is circuit-broken.
    #[error("all {tried} provider(s) for this data are circuit-broken")]
    NoHealthyProvider {
        /// How many were considered.
        tried: usize,
    },
    /// No provider offers this data at all.
    #[error("no provider offers this data")]
    NoOffer,
    /// An audit was asked for a value that is not held. Auditing something you
    /// do not have is just an expensive fetch nobody asked for.
    #[error("nothing cached to audit")]
    NothingToAudit,
    /// The spend meter refused.
    #[error("spend refused: {0}")]
    Budget(#[from] Refusal),
}

/// What should happen about a request.
#[derive(Debug)]
pub enum Plan {
    /// Already held and still valid. Costs nothing, touches no network.
    Serve(Vec<u8>),
    /// Make this call against this provider, having reserved this much budget.
    Call {
        /// Which provider to call.
        provider: ProviderId,
        /// Full fetch or conditional revalidation.
        kind: CallKind,
        /// The budget reservation, to be settled or released afterwards.
        commitment: Commitment,
    },
    /// Nothing can be done right now, and why.
    Refuse(Blocked),
}

/// Combines the cache, the spend meter and per-provider breakers into one
/// decision.
#[derive(Debug)]
pub struct Planner {
    cache: Cache,
    meter: Meter,
    breakers: HashMap<ProviderId, Breaker>,
    breaker_config: BreakerConfig,
}

impl Planner {
    /// A planner with the given budget and cache capacity, starting on `day`.
    #[must_use]
    pub fn new(budget: Budget, cache_capacity: usize, day: u64) -> Self {
        Self {
            cache: Cache::new(cache_capacity),
            meter: Meter::new(budget, day),
            breakers: HashMap::new(),
            breaker_config: BreakerConfig::default(),
        }
    }

    /// Overrides the breaker configuration used for providers seen from now on.
    #[must_use]
    pub fn with_breaker_config(mut self, config: BreakerConfig) -> Self {
        self.breaker_config = config;
        self
    }

    /// Cache counters.
    #[must_use]
    pub const fn cache_stats(&self) -> Stats {
        self.cache.stats()
    }

    /// Total committed or settled today.
    #[must_use]
    pub const fn spent_today(&self) -> MicroUsd {
        self.meter.spent_today()
    }

    /// Decides what to do about a request.
    ///
    /// `now` is a monotonic instant for breaker cooldowns; `day` is the
    /// accounting day for the spend meter. Both are supplied rather than read,
    /// which is what keeps the whole path replayable.
    pub fn plan(
        &mut self,
        request: &Request,
        as_of: AsOf,
        offers: &[Offer],
        now: u64,
        day: u64,
    ) -> Plan {
        let key = request.key();

        let kind = match self.cache.decide(key, request.mutability, as_of) {
            Decision::Serve(bytes) => return Plan::Serve(bytes),
            Decision::Revalidate { prior_hash } => CallKind::Conditional { prior_hash },
            Decision::Fetch => CallKind::Full,
        };

        if offers.is_empty() {
            return Plan::Refuse(Blocked::NoOffer);
        }

        // Cheapest healthy provider wins. Health is a gate rather than a term in
        // the ranking: a provider that is down is not cheap, it is unavailable.
        let config = self.breaker_config;
        let breakers = &mut self.breakers;
        let chosen = offers
            .iter()
            .filter(|o| {
                breakers
                    .entry(o.provider.clone())
                    .or_insert_with(|| Breaker::new(config))
                    .allows(now)
            })
            .min_by(|a, b| {
                a.price(kind)
                    .cmp(&b.price(kind))
                    .then_with(|| a.provider.cmp(&b.provider))
            });

        let Some(offer) = chosen else {
            return Plan::Refuse(Blocked::NoHealthyProvider {
                tried: offers.len(),
            });
        };

        match self.meter.authorize(offer.price(kind), day) {
            Ok(commitment) => Plan::Call {
                provider: offer.provider.clone(),
                kind,
                commitment,
            },
            Err(refusal) => Plan::Refuse(Blocked::Budget(refusal)),
        }
    }

    /// Plans a full fetch regardless of what the cache holds.
    ///
    /// Cached-forever is the right default and a standing risk: an `Immutable`
    /// or closed-`Latched` entry is never re-read, so a provider that served a
    /// wrong value once serves it forever. This is the audit path — run rarely,
    /// on a sample, to verify that what Radar believes still matches the chain.
    /// It is the only way a [`LatchReopened`](radar_types::LatchReopened) can be
    /// detected, because the normal path stops asking.
    ///
    /// Still subject to the breaker and the spend meter: an audit is a purchase
    /// like any other.
    pub fn plan_audit(&mut self, request: &Request, offers: &[Offer], now: u64, day: u64) -> Plan {
        if self.cache.peek(request.key()).is_none() {
            return Plan::Refuse(Blocked::NothingToAudit);
        }
        if offers.is_empty() {
            return Plan::Refuse(Blocked::NoOffer);
        }
        let kind = CallKind::Full;
        let config = self.breaker_config;
        let breakers = &mut self.breakers;
        let chosen = offers
            .iter()
            .filter(|o| {
                breakers
                    .entry(o.provider.clone())
                    .or_insert_with(|| Breaker::new(config))
                    .allows(now)
            })
            .min_by(|a, b| {
                a.price(kind)
                    .cmp(&b.price(kind))
                    .then_with(|| a.provider.cmp(&b.provider))
            });

        let Some(offer) = chosen else {
            return Plan::Refuse(Blocked::NoHealthyProvider {
                tried: offers.len(),
            });
        };

        match self.meter.authorize(offer.price(kind), day) {
            Ok(commitment) => Plan::Call {
                provider: offer.provider.clone(),
                kind,
                commitment,
            },
            Err(refusal) => Plan::Refuse(Blocked::Budget(refusal)),
        }
    }

    /// Records a completed full fetch: caches the value and settles the spend.
    pub fn record_success(
        &mut self,
        request: &Request,
        commitment: Commitment,
        bytes: Vec<u8>,
        observed_at: Slot,
        actual_cost: MicroUsd,
        latency_micros: u64,
    ) {
        let refetch_cost = commitment.reserved();
        self.meter.settle(commitment, actual_cost);
        self.cache.put(
            request.key(),
            Entry::new(bytes, observed_at, request.mutability, refetch_cost),
        );
        let _ = latency_micros;
    }

    /// Records a completed full fetch of a latched fact, folding the observed
    /// latch state into the entry.
    ///
    /// # Errors
    ///
    /// Returns [`radar_types::LatchReopened`] if a previously closed latch is
    /// reported open. That is never reconciled — it means the provider is wrong,
    /// is serving another token's data, or is being manipulated.
    pub fn record_latched(
        &mut self,
        request: &Request,
        commitment: Commitment,
        bytes: Vec<u8>,
        observed_at: Slot,
        actual_cost: MicroUsd,
        observed_closed: bool,
    ) -> Result<(), radar_types::LatchReopened> {
        let prior = self
            .cache
            .peek(request.key())
            .and_then(|e| e.latch)
            .unwrap_or(Latch::Open);
        let latch = prior.observe(observed_closed)?;

        let refetch_cost = commitment.reserved();
        self.meter.settle(commitment, actual_cost);
        self.cache.put(
            request.key(),
            Entry::new(bytes, observed_at, request.mutability, refetch_cost).with_latch(latch),
        );
        Ok(())
    }

    /// Records a conditional request that came back unchanged. Refreshes the
    /// entry's observation slot without paying for a body.
    pub fn record_unchanged(
        &mut self,
        request: &Request,
        commitment: Commitment,
        now: Slot,
        actual_cost: MicroUsd,
    ) {
        self.meter.settle(commitment, actual_cost);
        self.cache.touch(request.key(), now);
    }

    /// Records a failed call: releases the reservation and trips the breaker
    /// toward open.
    pub fn record_failure(&mut self, provider: &ProviderId, commitment: Commitment, now: u64) {
        self.meter.release(commitment);
        let config = self.breaker_config;
        self.breakers
            .entry(provider.clone())
            .or_insert_with(|| Breaker::new(config))
            .record_failure(now);
    }

    /// Records a provider responding successfully, for health tracking.
    pub fn record_healthy(&mut self, provider: &ProviderId, latency_micros: u64) {
        let config = self.breaker_config;
        self.breakers
            .entry(provider.clone())
            .or_insert_with(|| Breaker::new(config))
            .record_success(latency_micros);
    }

    /// Health for a provider, if it has been seen.
    #[must_use]
    pub fn health(&self, provider: &ProviderId) -> Option<&Breaker> {
        self.breakers.get(provider)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn budget() -> Budget {
        Budget {
            per_call_max: MicroUsd::from_dollars(0.10),
            daily_max: MicroUsd::from_dollars(1.00),
        }
    }

    fn cheap() -> ProviderId {
        ProviderId::new("cheap")
    }
    fn pricey() -> ProviderId {
        ProviderId::new("pricey")
    }

    fn offers() -> Vec<Offer> {
        vec![
            Offer::full_only(pricey(), MicroUsd::from_dollars(0.05)),
            Offer::full_only(cheap(), MicroUsd::from_dollars(0.001)),
        ]
    }

    fn req(m: Mutability) -> Request {
        Request::new("token_structure", json!({ "mint": "So111" }), m)
    }

    #[test]
    fn the_cheapest_healthy_provider_is_chosen() {
        let mut p = Planner::new(budget(), 64, 0);
        let plan = p.plan(&req(Mutability::Fast), AsOf::at(Slot(1)), &offers(), 0, 0);
        let Plan::Call { provider, .. } = plan else {
            panic!("expected a call")
        };
        assert_eq!(provider, cheap());
    }

    #[test]
    fn a_broken_cheap_provider_falls_through_to_a_working_expensive_one() {
        // Availability is a gate, not a term in the ranking. A provider that is
        // down is not cheap, it is unavailable.
        let mut p = Planner::new(budget(), 64, 0).with_breaker_config(BreakerConfig {
            failure_threshold: 1,
            cooldown: 1_000,
            half_open_successes: 1,
        });

        let plan = p.plan(&req(Mutability::Fast), AsOf::at(Slot(1)), &offers(), 0, 0);
        let Plan::Call { commitment, .. } = plan else {
            panic!("expected a call")
        };
        p.record_failure(&cheap(), commitment, 0);

        let plan = p.plan(&req(Mutability::Fast), AsOf::at(Slot(1)), &offers(), 0, 0);
        let Plan::Call { provider, .. } = plan else {
            panic!("expected a call")
        };
        assert_eq!(provider, pricey());
    }

    #[test]
    fn every_provider_broken_refuses_rather_than_calling_anyway() {
        let mut p = Planner::new(budget(), 64, 0).with_breaker_config(BreakerConfig {
            failure_threshold: 1,
            cooldown: 1_000,
            half_open_successes: 1,
        });
        for who in [cheap(), pricey()] {
            let plan = p.plan(&req(Mutability::Fast), AsOf::at(Slot(1)), &offers(), 0, 0);
            let Plan::Call { commitment, .. } = plan else {
                panic!("expected a call")
            };
            p.record_failure(&who, commitment, 0);
        }
        let plan = p.plan(&req(Mutability::Fast), AsOf::at(Slot(1)), &offers(), 0, 0);
        assert!(matches!(
            plan,
            Plan::Refuse(Blocked::NoHealthyProvider { tried: 2 })
        ));
    }

    #[test]
    fn an_immutable_fact_costs_money_exactly_once() {
        let mut p = Planner::new(budget(), 64, 0);
        let r = req(Mutability::Immutable);

        let Plan::Call { commitment, .. } = p.plan(&r, AsOf::at(Slot(100)), &offers(), 0, 0) else {
            panic!("first read must fetch")
        };
        p.record_success(
            &r,
            commitment,
            b"structure".to_vec(),
            Slot(100),
            MicroUsd::from_dollars(0.001),
            900,
        );

        for slot in [200, 5_000, 50_000_000] {
            let plan = p.plan(&r, AsOf::at(Slot(slot)), &offers(), 0, 0);
            assert!(matches!(plan, Plan::Serve(_)), "paid again at slot {slot}");
        }
        assert_eq!(p.spent_today(), MicroUsd::from_dollars(0.001));
        assert_eq!(p.cache_stats().avoided_cost, MicroUsd::from_dollars(0.003));
    }

    #[test]
    fn a_stale_fast_fact_plans_a_conditional_request_when_one_is_offered() {
        let mut p = Planner::new(budget(), 64, 0);
        let r = req(Mutability::Fast);
        let with_conditional = vec![Offer {
            provider: cheap(),
            full: MicroUsd::from_dollars(0.01),
            conditional: Some(MicroUsd::from_dollars(0.001)),
        }];

        let Plan::Call { commitment, .. } =
            p.plan(&r, AsOf::at(Slot(1_000)), &with_conditional, 0, 0)
        else {
            panic!("first read must fetch")
        };
        p.record_success(
            &r,
            commitment,
            b"reserves".to_vec(),
            Slot(1_000),
            MicroUsd::from_dollars(0.01),
            900,
        );

        // Past the 150-slot Fast budget: revalidate, at a tenth the price.
        let plan = p.plan(&r, AsOf::at(Slot(1_500)), &with_conditional, 0, 0);
        let Plan::Call {
            kind, commitment, ..
        } = plan
        else {
            panic!("expected a call")
        };
        assert!(matches!(kind, CallKind::Conditional { .. }));
        assert_eq!(commitment.reserved(), MicroUsd::from_dollars(0.001));

        p.record_unchanged(&r, commitment, Slot(1_500), MicroUsd::from_dollars(0.001));
        assert!(matches!(
            p.plan(&r, AsOf::at(Slot(1_600)), &with_conditional, 0, 0),
            Plan::Serve(_)
        ));
        assert_eq!(p.spent_today(), MicroUsd::from_dollars(0.011));
    }

    #[test]
    fn a_provider_without_conditional_support_pays_full_price_to_revalidate() {
        // The honest default today, and the reason the conditional-request ask
        // is worth making upstream.
        let o = Offer::full_only(cheap(), MicroUsd::from_dollars(0.01));
        assert_eq!(
            o.price(CallKind::Conditional {
                prior_hash: [0; 32]
            }),
            MicroUsd::from_dollars(0.01)
        );
    }

    #[test]
    fn exhausting_the_daily_budget_refuses_rather_than_overspending() {
        let mut p = Planner::new(
            Budget {
                per_call_max: MicroUsd::from_dollars(0.10),
                daily_max: MicroUsd::from_dollars(0.005),
            },
            64,
            0,
        );
        // Five cheap calls fit; the sixth does not. Distinct args so none hit cache.
        for i in 0..5 {
            let r = Request::new("x", json!({ "i": i }), Mutability::Fast);
            assert!(matches!(
                p.plan(&r, AsOf::at(Slot(1)), &offers(), 0, 0),
                Plan::Call { .. }
            ));
        }
        let r = Request::new("x", json!({ "i": 99 }), Mutability::Fast);
        assert!(matches!(
            p.plan(&r, AsOf::at(Slot(1)), &offers(), 0, 0),
            Plan::Refuse(Blocked::Budget(_))
        ));
    }

    #[test]
    fn a_latch_reopening_is_surfaced_rather_than_stored() {
        let mut p = Planner::new(budget(), 64, 0);
        let r = req(Mutability::Latched);

        let Plan::Call { commitment, .. } = p.plan(&r, AsOf::at(Slot(100)), &offers(), 0, 0) else {
            panic!("expected a call")
        };
        p.record_latched(
            &r,
            commitment,
            b"revoked".to_vec(),
            Slot(100),
            MicroUsd::from_dollars(0.001),
            true,
        )
        .expect("latch closes");

        // The normal path stops asking once the latch closes, which is the whole
        // saving — so a reopen can only surface on the audit path.
        assert!(matches!(
            p.plan(&r, AsOf::at(Slot(200)), &offers(), 0, 0),
            Plan::Serve(_)
        ));

        // A provider now claiming the revoked authority is live again must raise,
        // not silently un-disqualify a token already rejected.
        let Plan::Call { commitment, .. } = p.plan_audit(&r, &offers(), 0, 0) else {
            panic!("audit must plan a call")
        };
        let err = p
            .record_latched(
                &r,
                commitment,
                b"live".to_vec(),
                Slot(200),
                MicroUsd::from_dollars(0.001),
                false,
            )
            .expect_err("must refuse to store a reopened latch");
        assert_eq!(err, radar_types::LatchReopened);
    }

    #[test]
    fn no_offers_refuses_without_consuming_budget() {
        let mut p = Planner::new(budget(), 64, 0);
        let plan = p.plan(&req(Mutability::Fast), AsOf::at(Slot(1)), &[], 0, 0);
        assert!(matches!(plan, Plan::Refuse(Blocked::NoOffer)));
        assert_eq!(p.spent_today(), MicroUsd::ZERO);
    }
}
