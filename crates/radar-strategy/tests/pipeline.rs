// SPDX-License-Identifier: Apache-2.0
//! The seam between a strategy and the kernel.
//!
//! Unit tests either side prove each half in isolation, which is exactly the
//! shape of failure worth guarding against: a strategy whose proposals the
//! kernel always refuses is a broken pipeline that both halves' tests call
//! green. These tests run one real candidate all the way through.
//!
//! # The fixture is produced, not written
//!
//! This file used to hand-write its `ExitReport`: one curve point returning 2.4
//! SOL, and `can_be_stopped: false` beside `structure: None` — a combination
//! [`ExitReport::build`] never produces, since unread structure means unknown
//! rather than safe. The live probe was returning four thousand lamports at the
//! time, five orders of magnitude away, and the test written to catch exactly
//! that mismatch could not see it because its fixture was unreachable
//! (LEARNINGS entry 10).
//!
//! So the exit here is now built by running the real capacity search against a
//! stub quoter. If `discover_capacity` cannot produce a report of this shape,
//! this file stops compiling or stops passing — which is the only way a fixture
//! can keep being evidence about the system rather than about itself.

use std::collections::BTreeMap;

use radar_asof::AsOf;
use radar_risk::{Autonomy, Policy, PortfolioState, Refusal, Verdict, evaluate};
use radar_sim::ExitReport;
use radar_sim::exit::{Confidence, QuotePoint};
use radar_strategy::{Candidate, CreatorEdge, CreatorRecord, Decision, Strategy};
use radar_types::{Address, MicroUsd, Slot, SlotDelta};

const NOW: Slot = Slot(10_000);

/// A pool with depth expressed relative to the token's own supply.
///
/// Impact rises linearly with size and the route runs out past the pool, which
/// is close enough to a bonding curve for the search to behave as it does live.
struct Pool {
    depth_tokens: u64,
}

impl radar_sim::Quoter for Pool {
    fn quote_sell(
        &self,
        _mint: &Address,
        size_tokens: u64,
    ) -> Result<QuotePoint, radar_sim::QuoteError> {
        if size_tokens > self.depth_tokens {
            return Err(radar_sim::QuoteError::NoRoute { size_tokens });
        }
        let impact_bps =
            u32::try_from(u128::from(size_tokens) * 10_000 / u128::from(self.depth_tokens.max(1)))
                .unwrap_or(u32::MAX);
        Ok(QuotePoint {
            size_tokens,
            out_lamports: size_tokens / 40,
            impact_bps,
        })
    }
}

/// A mint account with a realistic pump.fun supply and nothing that can stop a
/// sale: six decimals, mint authority present, no freeze authority.
fn structure() -> radar_sim::MintStructure {
    let mut data = vec![0u8; 82];
    data[36..44].copy_from_slice(&SUPPLY.to_le_bytes());
    data[44] = 6;
    data[45] = 1;
    radar_sim::MintStructure::parse(&data, radar_sim::mint::TOKEN_PROGRAM).expect("parses")
}

/// 1e9 tokens at six decimals.
const SUPPLY: u64 = 1_000_000_000_000_000;

/// An exit report the real search actually produced.
fn measured_exit() -> ExitReport {
    let report = radar_sim::discover_capacity(
        &Pool {
            depth_tokens: SUPPLY / 100,
        },
        &Address::new([7u8; 32]),
        Some(structure()),
        radar_sim::Search::DEFAULT,
    );
    // If these stop holding, the search changed in a way that breaks the premise
    // of every test below, and that is worth failing loudly rather than quietly
    // running them against a candidate that can no longer propose.
    assert_eq!(
        report.confidence,
        Confidence::Measured,
        "the fixture must be a measured exit"
    );
    assert!(report.is_exitable(), "the fixture must be exitable");
    assert!(
        report
            .capacity_lamports(100)
            .is_some_and(|c| c > 1_000_000_000),
        "the fixture needs enough capacity to propose against, got {:?}",
        report.capacity_lamports(100)
    );
    report
}

fn candidate() -> Candidate {
    Candidate {
        mint: Address::new([7u8; 32]),
        creator: Address::new([8u8; 32]),
        launch_slot: Slot(1_000),
        coordination: None,
        as_of: AsOf::at(NOW),
        exit: Some(measured_exit()),
        creator_record: CreatorRecord {
            launches: 20,
            measured: 20,
            stillborn: 10,
            graduated: 3,
            graduated_organic: 3,
            launches_per_day: None,
        },
        sol_price_micro_usd: Some(MicroUsd::from_dollars(200.0)),
        token_observed_at: Slot(9_000),
        creator_observed_at: Slot(9_000),
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
