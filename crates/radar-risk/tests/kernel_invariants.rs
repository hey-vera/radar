// SPDX-License-Identifier: Apache-2.0
//! The invariants the risk kernel exists to hold.
//!
//! These are not style preferences. A change that breaks one is wrong even if it
//! compiles and the rest of the suite passes — in which case the rest of the
//! suite is also wrong.

use std::collections::BTreeMap;

use radar_risk::{
    Action, Address, Autonomy, MicroUsd, Policy, PortfolioState, Proposal, Refusal, Slot,
    SlotDelta, Verdict, evaluate,
};

fn policy() -> Policy {
    Policy {
        autonomy: Autonomy::Capped,
        max_position: MicroUsd::from_dollars(50.0),
        max_deployed: MicroUsd::from_dollars(200.0),
        max_per_creator: MicroUsd::from_dollars(60.0),
        max_daily_loss: MicroUsd::from_dollars(25.0),
        max_round_trip_cost_percent: 5,
        max_canary: MicroUsd::from_dollars(1.0),
        max_input_staleness: SlotDelta(150),
        max_consecutive_failures: 3,
    }
}

fn mint(n: u8) -> Address {
    Address::new([n; 32])
}

fn buy(notional_dollars: f64) -> Proposal {
    Proposal {
        mint: mint(1),
        creator: mint(2),
        action: Action::Buy,
        notional: MicroUsd::from_dollars(notional_dollars),
        estimated_round_trip_cost: MicroUsd::from_dollars(notional_dollars * 0.02),
        oldest_input_slot: Slot(1_000),
        simulated_exit_capacity: Some(MicroUsd::from_dollars(notional_dollars * 5.0)),
    }
}

fn state() -> PortfolioState {
    PortfolioState::flat(Slot(1_050))
}

fn refusals(v: &Verdict) -> Vec<Refusal> {
    match v {
        Verdict::Refused { reasons } => reasons.clone(),
        Verdict::Authorised(_) => Vec::new(),
    }
}

// --- purity ------------------------------------------------------------------

#[test]
fn the_same_inputs_always_produce_the_same_verdict() {
    // The invariant everything else rests on. Without it, a replay cannot check
    // a decision and a refusal cannot be reproduced from a recording.
    let (proposal, portfolio, pol) = (buy(20.0), state(), policy());
    let first = evaluate(&proposal, &portfolio, &pol);
    for _ in 0..100 {
        assert_eq!(evaluate(&proposal, &portfolio, &pol), first);
    }
}

#[test]
fn the_verdict_does_not_depend_on_the_order_of_the_portfolio_map() {
    // BTreeMap iterates in key order, but a caller could build the same exposure
    // from insertions in any sequence. Same exposure must mean same verdict.
    let mut forwards = BTreeMap::new();
    forwards.insert(mint(2), MicroUsd::from_dollars(10.0));
    forwards.insert(mint(3), MicroUsd::from_dollars(20.0));

    let mut backwards = BTreeMap::new();
    backwards.insert(mint(3), MicroUsd::from_dollars(20.0));
    backwards.insert(mint(2), MicroUsd::from_dollars(10.0));

    let inserted_forwards = PortfolioState {
        per_creator: forwards,
        ..state()
    };
    let inserted_backwards = PortfolioState {
        per_creator: backwards,
        ..state()
    };
    assert_eq!(
        evaluate(&buy(20.0), &inserted_forwards, &policy()),
        evaluate(&buy(20.0), &inserted_backwards, &policy())
    );
}

#[test]
fn refusal_reasons_come_back_in_a_fixed_order() {
    // Purity has to hold for refusals too, or a replay can disagree with the
    // original about *why* something was blocked.
    let bad = Proposal {
        notional: MicroUsd::from_dollars(500.0),
        estimated_round_trip_cost: MicroUsd::from_dollars(400.0),
        oldest_input_slot: Slot(1),
        simulated_exit_capacity: None,
        ..buy(500.0)
    };
    let first = refusals(&evaluate(&bad, &state(), &policy()));
    let mut sorted = first.clone();
    sorted.sort_unstable();
    assert_eq!(first, sorted, "reasons must already be sorted");
    for _ in 0..20 {
        assert_eq!(refusals(&evaluate(&bad, &state(), &policy())), first);
    }
}

#[test]
fn the_nonce_is_content_addressed_rather_than_random() {
    // A recorded authorisation has to be recomputable, or "this is the one that
    // was issued" is a claim rather than a check.
    let (proposal, portfolio, pol) = (buy(20.0), state(), policy());
    let nonce = |v: &Verdict| v.authorisation().map(|x| x.nonce.clone());

    let first = evaluate(&proposal, &portfolio, &pol);
    let again = evaluate(&proposal, &portfolio, &pol);
    assert_eq!(nonce(&first), nonce(&again));

    // A different size is a different decision and must not share a nonce.
    let different = evaluate(&buy(21.0), &portfolio, &pol);
    assert_ne!(nonce(&first), nonce(&different));
}

// --- deny by default ---------------------------------------------------------

#[test]
fn the_default_policy_refuses_a_proposal_that_a_real_policy_would_allow() {
    let p = buy(20.0);
    assert!(evaluate(&p, &state(), &policy()).authorisation().is_some());
    assert!(evaluate(&p, &state(), &Policy::default()).is_refused());
}

#[test]
fn the_kill_switch_cannot_be_reasoned_past() {
    // Checked first and unconditionally. A halt that a good enough proposal can
    // talk its way through is not a halt.
    let halted = PortfolioState {
        halted: true,
        ..state()
    };
    let v = evaluate(&buy(1.0), &halted, &policy());
    assert!(refusals(&v).contains(&Refusal::Halted));
    // Even a perfect exit is refused while halted.
    let exit = Proposal {
        action: Action::Exit,
        ..buy(1.0)
    };
    assert!(refusals(&evaluate(&exit, &halted, &policy())).contains(&Refusal::Halted));
}

// --- the exit-first premise --------------------------------------------------

#[test]
fn an_unsimulated_exit_is_refused_rather_than_assumed_fine() {
    // Radar's whole premise is that most losses are inability to exit. An
    // unsimulated exit is the one thing it must not wave through.
    let p = Proposal {
        simulated_exit_capacity: None,
        ..buy(20.0)
    };
    assert!(refusals(&evaluate(&p, &state(), &policy())).contains(&Refusal::ExitNotSimulated));
}

#[test]
fn a_position_larger_than_its_exit_is_refused() {
    let p = Proposal {
        simulated_exit_capacity: Some(MicroUsd::from_dollars(5.0)),
        ..buy(20.0)
    };
    assert!(refusals(&evaluate(&p, &state(), &policy())).contains(&Refusal::ExitCapacityTooSmall));
}

#[test]
fn exiting_is_never_blocked_by_a_sizing_limit() {
    // Refusing an exit because the position is too large would trap capital in
    // exactly the situation the limits exist to prevent.
    let huge_exit = Proposal {
        action: Action::Exit,
        notional: MicroUsd::from_dollars(10_000.0),
        estimated_round_trip_cost: MicroUsd::from_dollars(500.0),
        simulated_exit_capacity: None,
        ..buy(10_000.0)
    };
    let v = evaluate(&huge_exit, &state(), &policy());
    let r = refusals(&v);
    for blocked_by_size in [
        Refusal::OverPositionLimit,
        Refusal::OverDeploymentLimit,
        Refusal::OverCreatorLimit,
        Refusal::ExitNotSimulated,
        Refusal::RoundTripTooExpensive,
    ] {
        assert!(
            !r.contains(&blocked_by_size),
            "an exit must not be refused for {blocked_by_size:?}"
        );
    }
    assert!(v.authorisation().is_some());
}

// --- limits ------------------------------------------------------------------

#[test]
fn creator_exposure_aggregates_across_their_tokens() {
    // A creator launching forty-two tokens in half an hour can otherwise be held
    // forty-two times over while every position individually looks small.
    let mut per_creator = BTreeMap::new();
    per_creator.insert(mint(2), MicroUsd::from_dollars(55.0));
    let concentrated = PortfolioState {
        per_creator,
        ..state()
    };

    // A different token, but the same creator.
    let p = Proposal {
        mint: mint(9),
        ..buy(20.0)
    };
    assert!(refusals(&evaluate(&p, &concentrated, &policy())).contains(&Refusal::OverCreatorLimit));
}

#[test]
fn every_applicable_reason_is_returned_not_just_the_first() {
    // A caller fixing one limit only to hit the next has learned nothing about
    // whether the trade was ever going to be allowed.
    let bad = Proposal {
        notional: MicroUsd::from_dollars(500.0),
        estimated_round_trip_cost: MicroUsd::from_dollars(400.0),
        oldest_input_slot: Slot(1),
        simulated_exit_capacity: None,
        ..buy(500.0)
    };
    let r = refusals(&evaluate(&bad, &state(), &policy()));
    assert!(r.len() >= 5, "expected several reasons, got {r:?}");
    for expected in [
        Refusal::OverPositionLimit,
        Refusal::OverDeploymentLimit,
        Refusal::OverCreatorLimit,
        Refusal::RoundTripTooExpensive,
        Refusal::ExitNotSimulated,
        Refusal::InputsTooStale,
    ] {
        assert!(r.contains(&expected), "missing {expected:?} in {r:?}");
    }
}

#[test]
fn stale_inputs_are_refused_for_every_action() {
    // Exiting on an hour-old view of liquidity is its own hazard, not a safe
    // default.
    for action in [Action::Buy, Action::Reduce, Action::Exit] {
        let p = Proposal {
            action,
            oldest_input_slot: Slot(1),
            ..buy(10.0)
        };
        assert!(
            refusals(&evaluate(&p, &state(), &policy())).contains(&Refusal::InputsTooStale),
            "{action:?} should be refused on stale inputs"
        );
    }
}

#[test]
fn a_zero_size_buy_is_refused_rather_than_dividing_by_zero() {
    let p = Proposal {
        notional: MicroUsd::ZERO,
        ..buy(0.0)
    };
    assert!(evaluate(&p, &state(), &policy()).is_refused());
}

#[test]
fn repeated_failures_stop_trading() {
    let failing = PortfolioState {
        consecutive_failures: 3,
        ..state()
    };
    assert!(
        refusals(&evaluate(&buy(1.0), &failing, &policy())).contains(&Refusal::TooManyFailures)
    );
}

// --- autonomy ----------------------------------------------------------------

#[test]
fn observe_and_alert_authorise_nothing() {
    for autonomy in [Autonomy::Observe, Autonomy::Alert] {
        let p = Policy {
            autonomy,
            ..policy()
        };
        assert!(
            refusals(&evaluate(&buy(1.0), &state(), &p)).contains(&Refusal::NoAutonomy),
            "{autonomy:?} must not authorise"
        );
    }
}

#[test]
fn approve_authorises_within_policy_but_still_asks_a_human() {
    // The kernel says the trade is within policy; a person still says go.
    let p = Policy {
        autonomy: Autonomy::Approve,
        ..policy()
    };
    let auth = evaluate(&buy(20.0), &state(), &p);
    let auth = auth.authorisation().expect("within policy");
    assert!(auth.needs_operator_signature);
}

#[test]
fn canary_authorises_dust_and_refuses_anything_larger() {
    let p = Policy {
        autonomy: Autonomy::Canary,
        ..policy()
    };
    assert!(evaluate(&buy(0.5), &state(), &p).authorisation().is_some());
    assert!(refusals(&evaluate(&buy(20.0), &state(), &p)).contains(&Refusal::OverCanaryLimit));
}

#[test]
fn a_past_decision_can_be_re_judged_under_a_tighter_policy() {
    // The reason autonomy is a policy value rather than a code path: "what would
    // Capped have done last week" is answerable without ever having run it.
    let (proposal, portfolio) = (buy(20.0), state());
    assert!(
        evaluate(&proposal, &portfolio, &policy())
            .authorisation()
            .is_some()
    );

    let tighter = Policy {
        max_position: MicroUsd::from_dollars(5.0),
        ..policy()
    };
    assert!(
        refusals(&evaluate(&proposal, &portfolio, &tighter)).contains(&Refusal::OverPositionLimit)
    );
}

// --- the authorisation itself ------------------------------------------------

#[test]
fn an_authorisation_carries_hard_bounds_and_an_expiry() {
    // The signer checks the transaction against these. "About this much" would
    // be unenforceable.
    let v = evaluate(&buy(20.0), &state(), &policy());
    let a = v.authorisation().expect("authorised");
    assert_eq!(a.max_notional, MicroUsd::from_dollars(20.0));
    assert_eq!(a.mint, mint(1));
    assert_eq!(a.action, Action::Buy);
    assert!(
        a.expires_after > state().now,
        "an authorisation without an expiry never goes stale"
    );
    assert!(a.expires_after.saturating_since(state().now) <= SlotDelta(300));
}

#[test]
fn a_verdict_round_trips_through_json() {
    // Verdicts are recorded, so they have to survive storage to be replayed.
    let v = evaluate(&buy(20.0), &state(), &policy());
    let s = serde_json::to_string(&v).expect("serialize");
    assert_eq!(serde_json::from_str::<Verdict>(&s).expect("deserialize"), v);

    let refused = evaluate(&buy(500.0), &state(), &policy());
    let s = serde_json::to_string(&refused).expect("serialize");
    assert_eq!(
        serde_json::from_str::<Verdict>(&s).expect("deserialize"),
        refused
    );
}
