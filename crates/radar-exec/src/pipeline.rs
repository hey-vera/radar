// SPDX-License-Identifier: Apache-2.0
//! The order the stages run in, and what is recorded at each one.
//!
//! Written against traits rather than concrete clients so the whole sequence can
//! be tested without a network or a key. That is not a testing convenience: the
//! ordering *is* the safety property. The economics gate must run before the
//! signer sees anything, and a signer refusal must never be retried by
//! rebuilding — both are facts about this file, and neither is visible in the
//! modules it calls.

use radar_risk::Authorization;
use radar_types::{Address, MicroUsd, Signature};

use crate::economics::{Costs, Economics, FailureRisk, evaluate};
use crate::route::{Route, RouteError};

/// Something that can build a swap transaction.
pub trait Routing {
    /// Builds a buy of `mint` for `size_lamports`.
    ///
    /// # Errors
    ///
    /// Returns [`RouteError`] if no route exists or the router misbehaves.
    fn build_buy(
        &self,
        mint: &Address,
        wallet: &Address,
        size_lamports: u64,
    ) -> Result<Route, RouteError>;
}

/// Something that can sign a transaction against an authorization.
///
/// In production this is a pipe to another process. As a trait here so the
/// pipeline's ordering can be tested against a signer that refuses everything,
/// which is the case worth being sure about.
pub trait Signing {
    /// Returns the signed transaction, or the reasons it was refused.
    ///
    /// # Errors
    ///
    /// Returns the signer's refusal reasons verbatim.
    fn sign(&self, authorization: &Authorization, transaction: &str)
    -> Result<String, Vec<String>>;
}

/// Something that can send a transaction.
pub trait Sending {
    /// Sends it.
    ///
    /// # Errors
    ///
    /// Returns whatever the node or the transport said.
    fn send(&self, transaction: &str) -> Result<Signature, String>;
}

/// What the executor was asked to do.
#[derive(Debug, Clone)]
pub struct Attempt {
    /// The authorization the kernel issued.
    pub authorization: Authorization,
    /// The wallet trading.
    pub wallet: Address,
    /// How much to commit, in lamports.
    pub size_lamports: u64,
    /// The gross edge the strategy expects.
    pub expected_edge: MicroUsd,
    /// Costs known before routing: fees, tip, modelled slippage.
    ///
    /// Price impact is left at zero here and filled in from the route, because
    /// that is the one cost the router measures rather than the caller assuming.
    pub known_costs: Costs,
    /// The measured failure rate and what a failure costs.
    pub failure: FailureRisk,
    /// The notional one basis point of impact costs, used to price the route's
    /// reported impact.
    pub impact_per_bps: MicroUsd,
}

/// What happened.
///
/// Every variant records where the attempt stopped, because "no trade" covers
/// six very different situations and the research store needs to tell them
/// apart. A run that produced no trades because nothing routed is a data
/// problem; one that produced none because everything was uneconomic is the
/// system working.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Nothing would trade it.
    NoRoute {
        /// What the router said.
        why: String,
    },
    /// Routed, but the trade does not pay for itself.
    Uneconomic {
        /// The gate's finding.
        verdict: Economics,
        /// Where the route would have gone.
        venues: Vec<String>,
    },
    /// The signer refused.
    ///
    /// Terminal for this attempt. Rebuilding and resubmitting after a signer
    /// refusal is how a bounds violation becomes a retry loop against the one
    /// component that can stop it.
    Refused {
        /// The signer's reasons.
        reasons: Vec<String>,
    },
    /// Signed but not sent.
    NotSent {
        /// What the node or transport said.
        why: String,
    },
    /// Sent.
    Submitted {
        /// The signature to track.
        signature: Signature,
        /// What the route expected out.
        expected_out: u64,
        /// The venues it went through.
        venues: Vec<String>,
        /// The margin the gate expected.
        expected_net: MicroUsd,
    },
}

impl Outcome {
    /// The signature, if one was submitted.
    #[must_use]
    pub const fn signature(&self) -> Option<&Signature> {
        match self {
            Self::Submitted { signature, .. } => Some(signature),
            _ => None,
        }
    }

    /// A short label for the audit record.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::NoRoute { .. } => "no_route",
            Self::Uneconomic { .. } => "uneconomic",
            Self::Refused { .. } => "refused",
            Self::NotSent { .. } => "not_sent",
            Self::Submitted { .. } => "submitted",
        }
    }
}

/// Runs one attempt end to end.
///
/// The ordering is the point: route, then gate, then sign, then send. A trade
/// that does not pay for itself never reaches the process holding the key, and a
/// signer refusal ends the attempt rather than starting another.
pub fn execute<R: Routing, S: Signing, T: Sending>(
    attempt: &Attempt,
    router: &R,
    signer: &S,
    sender: &T,
) -> Outcome {
    let route = match router.build_buy(
        &attempt.authorization.mint,
        &attempt.wallet,
        attempt.size_lamports,
    ) {
        Ok(r) => r,
        Err(e) => {
            return Outcome::NoRoute { why: e.to_string() };
        }
    };

    // The route's measured impact is the last cost to land, and the only one the
    // caller could not have known before asking.
    let mut costs = attempt.known_costs;
    costs.price_impact = MicroUsd(
        attempt
            .impact_per_bps
            .get()
            .saturating_mul(u64::from(route.impact_bps)),
    );

    let verdict = evaluate(attempt.expected_edge, &costs, attempt.failure);
    let Economics::Worthwhile { expected_net } = verdict else {
        return Outcome::Uneconomic {
            verdict,
            venues: route.venues,
        };
    };

    let ready = match signer.sign(&attempt.authorization, &route.transaction) {
        Ok(s) => s,
        Err(reasons) => return Outcome::Refused { reasons },
    };

    match sender.send(&ready) {
        Ok(signature) => Outcome::Submitted {
            signature,
            expected_out: route.expected_out,
            venues: route.venues,
            expected_net,
        },
        Err(why) => Outcome::NotSent { why },
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use radar_risk::Action;
    use radar_types::Slot;

    use super::*;

    struct FixedRoute(Result<Route, RouteError>);

    impl Routing for FixedRoute {
        fn build_buy(&self, _: &Address, _: &Address, _: u64) -> Result<Route, RouteError> {
            self.0.clone()
        }
    }

    /// A signer that records whether it was asked at all.
    struct CountingSigner {
        asked: Cell<u32>,
        answer: Result<String, Vec<String>>,
    }

    impl CountingSigner {
        fn signing() -> Self {
            Self {
                asked: Cell::new(0),
                answer: Ok("c2lnbmVk".to_owned()),
            }
        }

        fn refusing() -> Self {
            Self {
                asked: Cell::new(0),
                answer: Err(vec!["mint absent".to_owned()]),
            }
        }
    }

    impl Signing for CountingSigner {
        fn sign(&self, _: &Authorization, _: &str) -> Result<String, Vec<String>> {
            self.asked.set(self.asked.get() + 1);
            self.answer.clone()
        }
    }

    struct CountingSender {
        asked: Cell<u32>,
        answer: Result<Signature, String>,
    }

    impl CountingSender {
        fn sending() -> Self {
            Self {
                asked: Cell::new(0),
                answer: Ok(Signature::new([7u8; 64])),
            }
        }
    }

    impl Sending for CountingSender {
        fn send(&self, _: &str) -> Result<Signature, String> {
            self.asked.set(self.asked.get() + 1);
            self.answer.clone()
        }
    }

    fn route(impact_bps: u32) -> Route {
        Route {
            transaction: "dHg=".to_owned(),
            expected_out: 1_000_000,
            impact_bps,
            venues: vec!["Pump.fun".to_owned()],
        }
    }

    fn attempt(edge: MicroUsd) -> Attempt {
        Attempt {
            authorization: Authorization {
                nonce: "n".to_owned(),
                mint: Address::new([2u8; 32]),
                action: Action::Buy,
                max_notional: MicroUsd::from_dollars(100.0),
                expires_after: Slot(1_150),
                needs_operator_signature: false,
            },
            wallet: Address::new([3u8; 32]),
            size_lamports: 10_000_000,
            expected_edge: edge,
            known_costs: Costs {
                dex_fee: MicroUsd::from_dollars(0.1),
                priority_fee: MicroUsd::from_dollars(0.02),
                tip: MicroUsd::from_dollars(0.05),
                ..Costs::default()
            },
            failure: FailureRisk {
                probability_bps: 500,
                cost: MicroUsd::from_dollars(0.07),
            },
            impact_per_bps: MicroUsd(1_000),
        }
    }

    #[test]
    fn a_worthwhile_trade_reaches_the_chain() {
        let signer = CountingSigner::signing();
        let sender = CountingSender::sending();
        let outcome = execute(
            &attempt(MicroUsd::from_dollars(5.0)),
            &FixedRoute(Ok(route(30))),
            &signer,
            &sender,
        );
        assert_eq!(outcome.label(), "submitted", "{outcome:?}");
        assert_eq!(outcome.signature(), Some(&Signature::new([7u8; 64])));
    }

    #[test]
    fn an_uneconomic_trade_never_reaches_the_signer() {
        // The ordering property. A trade that does not pay for itself must not
        // reach the process holding the key — not because signing it would be
        // unsafe, but because every request to that process is a chance to get
        // something wrong, and this one had no upside to begin with.
        let signer = CountingSigner::signing();
        let sender = CountingSender::sending();
        let outcome = execute(
            &attempt(MicroUsd::from_dollars(0.01)),
            &FixedRoute(Ok(route(30))),
            &signer,
            &sender,
        );
        assert_eq!(outcome.label(), "uneconomic", "{outcome:?}");
        assert_eq!(signer.asked.get(), 0, "the signer must not have been asked");
        assert_eq!(sender.asked.get(), 0);
    }

    #[test]
    fn a_high_impact_route_can_turn_a_good_trade_uneconomic() {
        // The cost the caller could not know before asking, and the reason the
        // gate runs after routing rather than before it.
        let signer = CountingSigner::signing();
        let sender = CountingSender::sending();
        let good = execute(
            &attempt(MicroUsd::from_dollars(1.0)),
            &FixedRoute(Ok(route(10))),
            &signer,
            &sender,
        );
        let bad = execute(
            &attempt(MicroUsd::from_dollars(1.0)),
            &FixedRoute(Ok(route(5_000))),
            &signer,
            &sender,
        );
        assert_eq!(good.label(), "submitted");
        assert_eq!(bad.label(), "uneconomic");
    }

    #[test]
    fn a_signer_refusal_ends_the_attempt_without_sending() {
        // Rebuilding after a signer refusal is how a bounds violation becomes a
        // retry loop against the one component that can stop it.
        let signer = CountingSigner::refusing();
        let sender = CountingSender::sending();
        let outcome = execute(
            &attempt(MicroUsd::from_dollars(5.0)),
            &FixedRoute(Ok(route(30))),
            &signer,
            &sender,
        );
        assert_eq!(outcome.label(), "refused", "{outcome:?}");
        assert_eq!(signer.asked.get(), 1, "asked exactly once, never retried");
        assert_eq!(sender.asked.get(), 0);
    }

    #[test]
    fn no_route_stops_before_anything_is_priced() {
        let signer = CountingSigner::signing();
        let sender = CountingSender::sending();
        let outcome = execute(
            &attempt(MicroUsd::from_dollars(5.0)),
            &FixedRoute(Err(RouteError::NoRoute {
                mint: "x".to_owned(),
                size_lamports: 1,
            })),
            &signer,
            &sender,
        );
        assert_eq!(outcome.label(), "no_route");
        assert_eq!(signer.asked.get(), 0);
    }

    #[test]
    fn a_send_failure_is_distinguishable_from_a_refusal() {
        // One means the transaction was never going to be valid; the other means
        // the network was in the way. Conflating them would make an outage look
        // like a policy violation.
        let signer = CountingSigner::signing();
        let sender = CountingSender {
            asked: Cell::new(0),
            answer: Err("connection reset".to_owned()),
        };
        let outcome = execute(
            &attempt(MicroUsd::from_dollars(5.0)),
            &FixedRoute(Ok(route(30))),
            &signer,
            &sender,
        );
        assert_eq!(outcome.label(), "not_sent", "{outcome:?}");
    }

    #[test]
    fn every_outcome_has_a_distinct_label() {
        // The research store groups by these. Two situations sharing a label
        // would silently pool a data problem with the system working.
        let labels = [
            Outcome::NoRoute { why: String::new() }.label(),
            Outcome::Refused { reasons: vec![] }.label(),
            Outcome::NotSent { why: String::new() }.label(),
            Outcome::Submitted {
                signature: Signature::new([0u8; 64]),
                expected_out: 0,
                venues: vec![],
                expected_net: MicroUsd::ZERO,
            }
            .label(),
        ];
        let mut sorted = labels;
        sorted.sort_unstable();
        let mut deduped = sorted.to_vec();
        deduped.dedup();
        assert_eq!(deduped.len(), labels.len());
    }
}
