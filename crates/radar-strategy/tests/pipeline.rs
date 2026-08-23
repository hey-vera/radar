// SPDX-License-Identifier: Apache-2.0
//! The seam between a strategy and the kernel.
//!
//! Unit tests either side prove each half in isolation, which is exactly the
//! shape of failure worth guarding against: a strategy whose proposals the
//! kernel always refuses is a broken pipeline that both halves' tests call
//! green. These tests run one real candidate all the way through.

use std::collections::BTreeMap;

use radar_asof::AsOf;
use radar_risk::{Autonomy, Policy, PortfolioState, Refusal, Verdict, evaluate};
use radar_sim::ExitReport;
use radar_sim::exit::{Confidence, QuotePoint};
use radar_strategy::{Candidate, CreatorEdge, CreatorRecord, Decision, Strategy};
use radar_types::{Address, MicroUsd, Slot, SlotDelta};

const NOW: Slot = Slot(10_000);

fn candidate() -> Candidate {
    Candidate {
        mint: Address::new([7u8; 32]),
        creator: Address::new([8u8; 32]),
        launch_slot: Slot(1_000),
        as_of: AsOf::at(NOW),
        exit: Some(ExitReport {
            mint: Address::new([7u8; 32]),
            structure: None,
            curve: vec![QuotePoint {
                size_tokens: 5_000_000,
                out_lamports: 2_400_000_000,
                impact_bps: 80,
            }],
            no_route_at: Vec::new(),
            structural_threats: Vec::new(),
            can_be_stopped: false,
            confidence: Confidence::Measured,
        }),
        creator_record: CreatorRecord {
            launches: 20,
            measured: 20,
            stillborn: 10,
            graduated: 3,
        },
        sol_price_micro_usd: Some(MicroUsd::from_dollars(200.0)),
        oldest_input_slot: Slot(9_000),
    }
}

/// A policy that would actually let something through.
fn capped() -> Policy {
    Policy {
        autonomy: Autonomy::Capped,
        max_position: MicroUsd::from_dollars(250.0),
        max_deployed: MicroUsd::from_dollars(1_000.0),
        max_per_creator: MicroUsd::from_dollars(250.0),
        max_daily_loss: MicroUsd::from_dollars(100.0),
        max_round_trip_cost_percent: 5,
        max_canary: MicroUsd::from_dollars(1.0),
        max_input_staleness: SlotDelta(6_000),
        max_consecutive_failures: 3,
    }
}

fn propose() -> radar_risk::Proposal {
    match CreatorEdge::default().consider(&candidate()) {
        Decision::Propose(p) => *p,
        Decision::Pass(reasons) => panic!("the fixture should qualify; passed for {reasons:?}"),
    }
}

#[test]
fn a_strategys_proposal_can_actually_be_authorised() {
    // The test that would fail if the strategy and the kernel disagreed about
    // what a fundable proposal looks like — while both crates' own tests passed.
    let verdict = evaluate(&propose(), &PortfolioState::flat(NOW), &capped());
    assert!(
        verdict.authorisation().is_some(),
        "strategy and kernel disagree: {verdict:?}"
    );
}

#[test]
fn the_default_policy_refuses_it() {
    // Radar ships closed. Building the trading lane does not deploy capital;
    // only Josh changing the policy does.
    let verdict = evaluate(&propose(), &PortfolioState::flat(NOW), &Policy::CLOSED);
    assert!(verdict.is_refused());
}

#[test]
fn the_authorisation_never_exceeds_what_the_strategy_asked_for() {
    // Authority flows one way. The kernel may shrink a proposal, never grow it.
    let proposal = propose();
    let verdict = evaluate(&proposal, &PortfolioState::flat(NOW), &capped());
    let auth = verdict.authorisation().expect("authorised");
    assert!(auth.max_notional <= proposal.notional);
}

#[test]
fn a_creator_already_at_the_limit_is_refused() {
    // The limit that exists because one creator launching forty tokens can
    // otherwise be held forty times over while each position looks small.
    let proposal = propose();
    let mut state = PortfolioState::flat(NOW);
    state.per_creator = BTreeMap::from([(proposal.creator, MicroUsd::from_dollars(250.0))]);
    state.deployed = MicroUsd::from_dollars(250.0);

    let Verdict::Refused { reasons } = evaluate(&proposal, &state, &capped()) else {
        panic!("expected a refusal");
    };
    assert!(reasons.contains(&Refusal::OverCreatorLimit));
}

#[test]
fn a_halt_beats_a_perfect_proposal() {
    let mut state = PortfolioState::flat(NOW);
    state.halted = true;
    let Verdict::Refused { reasons } = evaluate(&propose(), &state, &capped()) else {
        panic!("a kill switch that can be reasoned past is not a kill switch");
    };
    assert!(reasons.contains(&Refusal::Halted));
}

#[test]
fn the_whole_pipeline_is_replayable() {
    // Strategy purity plus kernel purity should compose into pipeline purity.
    // Asserted end to end rather than inferred from the two halves, because the
    // composition is where an impure step would actually show up.
    let candidate = candidate();
    let strategy = CreatorEdge::default();
    let state = PortfolioState::flat(NOW);
    let policy = capped();

    let run = || {
        let Decision::Propose(p) = strategy.consider(&candidate) else {
            panic!("expected a proposal");
        };
        evaluate(&p, &state, &policy)
    };

    let first = run();
    for _ in 0..32 {
        assert_eq!(run(), first);
    }
}
