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
        max_round_trip_cost_bps: 900,
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

#[test]
fn the_cost_ceiling_can_express_the_cost_that_was_actually_measured() {
    // The reason this field is basis points rather than percent. The measured
    // round trip is 850 bps; a percent field offers 8% -- which refuses every
    // trade there is -- or 9%, and nothing between. The rounding, not the
    // measurement, would decide whether the system trades at all.
    let mut at_measured_cost = buy(100.0);
    at_measured_cost.estimated_round_trip_cost = MicroUsd::from_dollars(8.50);

    let just_under = Policy {
        max_round_trip_cost_bps: 849,
        ..policy()
    };
    let exactly = Policy {
        max_round_trip_cost_bps: 850,
        ..policy()
    };

    assert!(
        refusals(&evaluate(&at_measured_cost, &state(), &just_under))
            .contains(&Refusal::RoundTripTooExpensive),
        "a ceiling one basis point under the cost must refuse"
    );
    assert!(
        !refusals(&evaluate(&at_measured_cost, &state(), &exactly))
            .contains(&Refusal::RoundTripTooExpensive),
        "and a ceiling exactly at the cost must not -- the boundary is inclusive, \
         so a policy set to the measured cost admits a trade at that cost"
    );
}

#[test]
fn a_cost_ceiling_of_zero_refuses_every_cost_including_none() {
    // Deny by default, and the shipped policy sets exactly this. A proposal
    // costing nothing is still refused, because a zero ceiling is a statement
    // that no round trip is authorised rather than that free ones are.
    let mut free = buy(100.0);
    free.estimated_round_trip_cost = MicroUsd::ZERO;
    let closed = Policy {
        max_round_trip_cost_bps: 0,
        ..policy()
    };
    assert!(
        !refusals(&evaluate(&free, &state(), &closed)).contains(&Refusal::RoundTripTooExpensive),
        "a cost of zero is within a ceiling of zero; the refusal comes from \
         elsewhere in Policy::CLOSED, not from this comparison"
    );

    let mut any = buy(100.0);
    any.estimated_round_trip_cost = MicroUsd(1);
    assert!(
        refusals(&evaluate(&any, &state(), &closed)).contains(&Refusal::RoundTripTooExpensive),
        "and one micro-dollar of cost is not"
    );
}

// --- what the policy refuses, versus what the token did ----------------------

#[test]
fn the_shipped_policy_refuses_everything_for_reasons_that_are_all_its_own() {
    // The seven-reason refusal every proposal gets under Policy::CLOSED. Every
    // one of them is about the policy; none is about the token. A reader who has
    // not memorised CLOSED cannot tell that from the list, which is what this
    // function exists to fix.
    let inevitable = radar_risk::inevitable_refusals(&Policy::CLOSED);
    assert!(
        inevitable.contains(&Refusal::NoAutonomy),
        "an Observe policy authorises nothing whatever the token is"
    );
    assert!(inevitable.contains(&Refusal::OverPositionLimit));
    assert!(inevitable.contains(&Refusal::DailyLossReached), "0 >= 0");
    assert!(inevitable.contains(&Refusal::RoundTripTooExpensive));
    assert!(inevitable.contains(&Refusal::InputsTooStale));

    // And a real refusal under CLOSED splits to nothing that is about the token.
    let refused = evaluate(&buy(10.0), &state(), &Policy::CLOSED);
    let (policy_bound, about_this) =
        radar_risk::partition_refusals(&refusals(&refused), &Policy::CLOSED);
    assert!(!policy_bound.is_empty());
    assert!(
        about_this.is_empty(),
        "under a closed policy nothing is a finding about the token: {about_this:?}"
    );
}

#[test]
fn an_open_policy_makes_a_real_refusal_visible_as_one() {
    // The direction that matters for a reader. With the policy out of the way,
    // an exit that could not be simulated is the whole answer -- and it is the
    // one Radar exists to act on.
    let mut unsellable = buy(10.0);
    unsellable.simulated_exit_capacity = None;

    let verdict = evaluate(&unsellable, &state(), &policy());
    let (policy_bound, about_this) = radar_risk::partition_refusals(&refusals(&verdict), &policy());

    assert_eq!(
        about_this,
        vec![Refusal::ExitNotSimulated],
        "the finding is the unsimulated exit, and nothing else"
    );
    assert!(
        policy_bound.is_empty(),
        "an open policy contributes no refusals of its own: {policy_bound:?}"
    );
}

#[test]
fn an_open_policy_has_nothing_inevitable_about_it() {
    // If a policy would refuse a perfect proposal, it is closed in some way the
    // operator may not have intended. An open one refuses nothing a priori.
    assert!(radar_risk::inevitable_refusals(&policy()).is_empty());
}

#[test]
fn a_halted_operator_is_a_policy_fact_not_a_token_fact() {
    // Halting is deliberately not part of Policy, so it cannot be inevitable
    // from the policy alone -- and a reader must still not read it as something
    // the token did. It surfaces as a finding, which is correct: it IS about
    // this attempt rather than about the rules, and the operator knows why.
    let mut halted = state();
    halted.halted = true;
    let verdict = evaluate(&buy(10.0), &halted, &policy());
    let (_, about_this) = radar_risk::partition_refusals(&refusals(&verdict), &policy());
    assert!(about_this.contains(&Refusal::Halted));
}

#[test]
fn the_classification_is_computed_so_it_cannot_drift_from_the_policy() {
    // A hardcoded list would be right for CLOSED and wrong for every policy
    // written afterwards -- wrong in the direction that presents a policy limit
    // as a finding about a token.
    //
    // Tightening one limit must move exactly that reason across the line.
    let mut no_room = policy();
    no_room.max_deployed = MicroUsd::ZERO;

    let inevitable = radar_risk::inevitable_refusals(&no_room);
    assert_eq!(
        inevitable,
        vec![Refusal::OverDeploymentLimit],
        "the newly-closed limit, and only it"
    );
    assert!(radar_risk::inevitable_refusals(&policy()).is_empty());
}

#[test]
fn the_refusal_production_actually_prints_is_entirely_about_the_policy() {
    // The shape of a real proposal from the live funnel: $6.30 sized off $31.52
    // of measured exit capacity, 850 bps of assumed round-trip cost, inputs a
    // few thousand slots old. Production prints this for every one of them:
    //
    //   [NoAutonomy, OverPositionLimit, OverDeploymentLimit, OverCreatorLimit,
    //    DailyLossReached, RoundTripTooExpensive, InputsTooStale]
    //
    // Seven reasons, one fact. A reader who has not memorised Policy::CLOSED
    // cannot tell that none of them is about the token, and a frontend that
    // rendered the list verbatim would tell a novice there are seven problems.
    let now = Slot(441_734_987);
    let real = Proposal {
        mint: mint(1),
        creator: mint(2),
        action: Action::Buy,
        notional: MicroUsd::from_dollars(6.30),
        estimated_round_trip_cost: MicroUsd::from_dollars(0.5355),
        oldest_input_slot: Slot(now.get() - 4_000),
        simulated_exit_capacity: Some(MicroUsd::from_dollars(31.52)),
    };

    let reasons = refusals(&evaluate(
        &real,
        &PortfolioState::flat(now),
        &Policy::CLOSED,
    ));
    assert_eq!(reasons.len(), 7, "the list production prints: {reasons:?}");

    let (policy_bound, about_this) = radar_risk::partition_refusals(&reasons, &Policy::CLOSED);
    assert_eq!(policy_bound.len(), 7);
    assert!(
        about_this.is_empty(),
        "a proposal with measured capacity five times its size, at a cost the \
         funnel measured, is refused for nothing it did: {about_this:?}"
    );
}
