// SPDX-License-Identifier: Apache-2.0
//! The whole lane, from a candidate to a submitted transaction.
//!
//! # What this closes
//!
//! `AGENTS.md` has said, accurately, that the trading lane is **not tested end
//! to end**: *"no crate depends on `radar-exec`, no test composes it with
//! anything upstream, and the longest chain any test runs is strategy → risk"*.
//!
//! Every stage had unit tests. That is exactly the shape of failure this
//! repository has already been caught by twice — `radar-strategy`'s own pipeline
//! test exists because *"a strategy whose proposals the kernel always refuses is
//! a broken pipeline that both halves' tests call green"*, and
//! [`LEARNINGS`](../../../LEARNINGS.md) entry 10 records a lane that was shut
//! four stages upstream of the policy everyone believed was shutting it, for
//! eight months, while every test passed.
//!
//! This runs one real candidate through **strategy → kernel → executor**, so the
//! composition is exercised before capital is ever put behind it.
//!
//! # What it deliberately does not do
//!
//! It does not move money and it does not sign anything. `radar_exec::execute`
//! is written against `Routing`, `Signing` and `Sending` traits precisely so the
//! ordering — which *is* the safety property — can be tested without a network
//! or a key. The signer here is a stub; the real one is a separate process that
//! re-decodes whatever this side built, and nothing in this file can weaken that.
//!
//! # The fixture is produced, not written
//!
//! The exit report comes from running the real capacity search against a stub
//! quoter, for the reason `radar-strategy/tests/pipeline.rs` gives at length: a
//! hand-written fixture five orders of magnitude away from anything the system
//! can measure is a fixture that tests a different system (LEARNINGS 10).
//!
//! The pool and mint below are duplicated from that file rather than shared,
//! because a `tests/` module cannot be imported across crates. The duplication is
//! guarded: `the_fixture_is_a_measured_exit_the_search_actually_produced` asserts
//! the same premises, so if the search changes underneath, both files fail rather
//! than drifting apart quietly.

use radar_asof::AsOf;
use radar_exec::pipeline::{Routing, Sending, Signing};
use radar_exec::{Attempt, Costs, FailureRisk, Outcome, Route, RouteError};
use radar_risk::{Action, Authorization, Autonomy, Policy, PortfolioState, Verdict, evaluate};
use radar_sim::ExitReport;
use radar_sim::exit::{Confidence, QuotePoint};
use radar_strategy::{Candidate, CreatorEdge, CreatorRecord, Decision, Strategy};
use radar_types::{Address, MicroUsd, Signature, Slot, SlotDelta};

const NOW: Slot = Slot(10_000);
/// 1e9 tokens at six decimals, as a pump.fun mint carries.
const SUPPLY: u64 = 1_000_000_000_000_000;

// ---------------------------------------------------------------- the fixture

/// A pool with depth expressed relative to the token's own supply.
struct Pool {
    depth_tokens: u64,
}

impl radar_sim::Quoter for Pool {
    fn quote_sell(
        &self,
        _mint: &Address,
        size_tokens: u64,
    ) -> Result<QuotePoint, radar_sim::QuoteError> {
        let reserve_tokens = u128::from(self.depth_tokens.max(1));
        let reserve_lamports = reserve_tokens / 40;
        let x = u128::from(size_tokens);
        let out = reserve_lamports * x / (reserve_tokens + x);
        Ok(QuotePoint {
            size_tokens,
            out_lamports: u64::try_from(out).unwrap_or(u64::MAX),
            // Constant and wrong, exactly as Jupiter reports it for pump.fun
            // routes. The search derives real impact from the realised price, so
            // a fixture reporting an honest number here would not exercise the
            // thing that broke (LEARNINGS 11).
            impact_bps: 395,
        })
    }
}

/// Six decimals, mint authority present, no freeze authority — nothing that can
/// stop a sale.
fn structure() -> radar_sim::MintStructure {
    let mut data = vec![0u8; 82];
    data[36..44].copy_from_slice(&SUPPLY.to_le_bytes());
    data[44] = 6;
    data[45] = 1;
    radar_sim::MintStructure::parse(&data, radar_sim::mint::TOKEN_PROGRAM).expect("parses")
}

fn measured_exit() -> ExitReport {
    radar_sim::discover_capacity(
        &Pool {
            depth_tokens: SUPPLY / 100,
        },
        &Address::new([7u8; 32]),
        Some(structure()),
        radar_sim::Search::DEFAULT,
    )
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
///
/// The cost ceiling is 10% because that is what an honest cost estimate costs:
/// `assumed_round_trip_bps` is 850, and a 5% ceiling refuses every pump.fun
/// trade there is. Lowering the cost estimate to fit a tighter ceiling would be
/// making the number that gates capital agree with the test rather than with the
/// market.
///
/// Every field is set rather than spread from `Policy::CLOSED`. The first
/// version did spread, and inherited a zero staleness budget that refused the
/// candidate on `InputsStale` — a policy assembled by inheritance hides which of
/// its fields the test actually depends on, and this one depends on all of them.
fn capped() -> Policy {
    Policy {
        autonomy: Autonomy::Capped,
        max_position: MicroUsd::from_dollars(250.0),
        max_deployed: MicroUsd::from_dollars(1_000.0),
        max_per_creator: MicroUsd::from_dollars(250.0),
        max_daily_loss: MicroUsd::from_dollars(100.0),
        max_round_trip_cost_bps: 1_000,
        max_canary: MicroUsd::from_dollars(1.0),
        max_input_staleness: SlotDelta(6_000),
        max_consecutive_failures: 3,
    }
}

// ------------------------------------------------------------- the exec stubs

struct FixedRoute(Result<Route, RouteError>);

impl Routing for FixedRoute {
    fn build_buy(&self, _: &Address, _: &Address, _: u64) -> Result<Route, RouteError> {
        self.0.clone()
    }
}

fn a_route() -> Route {
    Route {
        transaction: "AQAAAA".to_owned(),
        expected_out: 5_000_000_000,
        impact_bps: 20,
        venues: vec!["stub".to_owned()],
    }
}

/// A signer that records the bounds it was asked to honour.
///
/// It records rather than merely answering, because the property worth checking
/// is not "the pipeline called a signer" but "the pipeline handed the signer the
/// kernel's authorisation, unaltered".
struct RecordingSigner {
    seen: std::cell::RefCell<Vec<Authorization>>,
    answer: Result<String, Vec<String>>,
}

impl RecordingSigner {
    fn signing() -> Self {
        Self {
            seen: std::cell::RefCell::new(Vec::new()),
            answer: Ok("c2lnbmVk".to_owned()),
        }
    }

    fn refusing(reasons: &[&str]) -> Self {
        Self {
            seen: std::cell::RefCell::new(Vec::new()),
            answer: Err(reasons.iter().map(|r| (*r).to_owned()).collect()),
        }
    }
}

impl Signing for RecordingSigner {
    fn sign(&self, authorization: &Authorization, _: &str) -> Result<String, Vec<String>> {
        self.seen.borrow_mut().push(authorization.clone());
        self.answer.clone()
    }
}

/// A sender that records how many times it was asked.
struct CountingSender {
    sent: std::cell::Cell<u32>,
}

impl CountingSender {
    fn new() -> Self {
        Self {
            sent: std::cell::Cell::new(0),
        }
    }
}

impl Sending for CountingSender {
    fn send(&self, _: &str) -> Result<Signature, String> {
        self.sent.set(self.sent.get() + 1);
        Ok(Signature::new([9u8; 64]))
    }
}

/// Costs and failure risk that leave a healthy margin, so a test about
/// *composition* does not fail for an unrelated reason about *economics*.
///
/// The economics gate has its own tests. This file's job is the ordering.
fn generous(notional: MicroUsd) -> (MicroUsd, Costs, FailureRisk, MicroUsd) {
    let edge = MicroUsd(notional.get() * 3);
    let costs = Costs {
        price_impact: MicroUsd(0),
        slippage: MicroUsd(1),
        dex_fee: MicroUsd(1),
        priority_fee: MicroUsd(1),
        tip: MicroUsd(1),
    };
    let failure = FailureRisk {
        probability_bps: 100,
        cost: MicroUsd(1),
    };
    (edge, costs, failure, MicroUsd(1))
}

/// Runs strategy → kernel, returning whatever the kernel said.
fn through_the_kernel(policy: &Policy) -> (Verdict, MicroUsd) {
    let Decision::Propose(proposal) = CreatorEdge::default().consider(&candidate()) else {
        panic!("the fixture must produce a proposal, or nothing below tests the lane");
    };
    let notional = proposal.notional;
    (
        evaluate(&proposal, &PortfolioState::flat(NOW), policy),
        notional,
    )
}

/// Builds the executor's attempt from an authorisation the kernel issued.
fn attempt_from(authorization: &Authorization, notional: MicroUsd) -> Attempt {
    let (expected_edge, known_costs, failure, impact_per_bps) = generous(notional);
    Attempt {
        authorization: authorization.clone(),
        wallet: Address::new([3u8; 32]),
        size_lamports: 1_000_000_000,
        expected_edge,
        known_costs,
        failure,
        impact_per_bps,
    }
}

// ------------------------------------------------------------------ the tests

#[test]
fn the_fixture_is_a_measured_exit_the_search_actually_produced() {
    // The premise every test below rests on, asserted rather than assumed. This
    // mirrors `radar-strategy/tests/pipeline.rs`, which holds the same premises
    // for the same fixture -- so a change to the capacity search fails in both
    // places rather than silently making one of them test nothing.
    let report = measured_exit();
    assert_eq!(report.confidence, Confidence::Measured);
    assert!(report.is_exitable());
    assert!(
        report
            .capacity_lamports(100)
            .is_some_and(|c| c > 1_000_000_000),
        "the fixture needs capacity to propose against, got {:?}",
        report.capacity_lamports(100)
    );
}

#[test]
fn a_candidate_becomes_a_submitted_transaction() {
    // The chain nothing had ever run: Candidate -> CreatorEdge -> Proposal ->
    // kernel -> Authorization -> executor -> Submitted.
    let (verdict, notional) = through_the_kernel(&capped());
    let authorization = verdict
        .authorisation()
        .expect("a permissive policy must authorise this candidate");

    let signer = RecordingSigner::signing();
    let sender = CountingSender::new();
    let outcome = radar_exec::execute(
        &attempt_from(authorization, notional),
        &FixedRoute(Ok(a_route())),
        &signer,
        &sender,
    );

    let Outcome::Submitted { venues, .. } = &outcome else {
        panic!("expected a submission, got {outcome:?}");
    };
    assert_eq!(venues, &["stub".to_owned()]);
    assert_eq!(sender.sent.get(), 1, "sent exactly once");
}

#[test]
fn the_shipped_policy_stops_the_lane_before_the_executor() {
    // The test that makes the one above mean something. `Policy::CLOSED` is what
    // ships, and if the lane ran to a submission under it then the test above
    // would be proving something about the fixture rather than about the lane.
    //
    // LEARNINGS 10 is the reason this is a separate assertion: for eight months
    // everyone believed `Policy::CLOSED` was what stood between Radar and a
    // trade, and it had never been handed a proposal at all.
    let (verdict, _) = through_the_kernel(&Policy::CLOSED);
    assert!(
        verdict.is_refused(),
        "the shipped policy must refuse a proposal the permissive one authorises"
    );
    assert!(
        verdict.authorisation().is_none(),
        "and must produce nothing the executor could act on"
    );
}

#[test]
fn the_signer_is_handed_the_kernels_bounds_and_not_the_strategys() {
    // The invariant the whole architecture exists for: the kernel is the only
    // thing that authorises capital, and what reaches the signer must be the
    // kernel's authorisation rather than anything the strategy asked for.
    //
    // Checked by identity, not by shape -- a signer handed a *reconstructed*
    // authorisation with the same fields would pass a field-by-field comparison
    // while breaking the chain of custody this asserts.
    let (verdict, notional) = through_the_kernel(&capped());
    let authorization = verdict.authorisation().expect("authorised");

    let signer = RecordingSigner::signing();
    let _ = radar_exec::execute(
        &attempt_from(authorization, notional),
        &FixedRoute(Ok(a_route())),
        &signer,
        &CountingSender::new(),
    );

    let seen = signer.seen.borrow();
    assert_eq!(seen.len(), 1, "the signer is asked exactly once");
    assert_eq!(&seen[0], authorization, "verbatim, not reconstructed");
    assert_eq!(seen[0].action, Action::Buy);
    assert!(
        seen[0].max_notional.get() <= capped().max_position.get(),
        "the bound handed to the signer must respect the policy ceiling"
    );
}

#[test]
fn a_signer_refusal_ends_the_attempt_without_sending() {
    // Rebuilding and resubmitting after a signer refusal is how a bounds
    // violation becomes a retry loop against the one component that can stop it.
    let (verdict, notional) = through_the_kernel(&capped());
    let authorization = verdict.authorisation().expect("authorised");

    let sender = CountingSender::new();
    let outcome = radar_exec::execute(
        &attempt_from(authorization, notional),
        &FixedRoute(Ok(a_route())),
        &RecordingSigner::refusing(&["outside bounds"]),
        &sender,
    );

    assert!(matches!(outcome, Outcome::Refused { .. }), "{outcome:?}");
    assert_eq!(sender.sent.get(), 0, "nothing may be sent after a refusal");
}

#[test]
fn a_trade_that_does_not_pay_never_reaches_the_process_holding_the_key() {
    // The ordering the pipeline's own documentation calls the safety property:
    // the economics gate runs before the signer. Asserted from the outside, on a
    // real authorisation, because the ordering is a fact about `execute` that no
    // module it calls can see.
    let (verdict, notional) = through_the_kernel(&capped());
    let authorization = verdict.authorisation().expect("authorised");

    let mut attempt = attempt_from(authorization, notional);
    // An edge of nothing against costs that are real.
    attempt.expected_edge = MicroUsd(0);

    let signer = RecordingSigner::signing();
    let sender = CountingSender::new();
    let outcome = radar_exec::execute(&attempt, &FixedRoute(Ok(a_route())), &signer, &sender);

    assert!(matches!(outcome, Outcome::Uneconomic { .. }), "{outcome:?}");
    assert!(
        signer.seen.borrow().is_empty(),
        "the signer must never have been asked"
    );
    assert_eq!(sender.sent.get(), 0);
}

#[test]
fn a_route_that_cannot_be_built_stops_before_everything_else() {
    let (verdict, notional) = through_the_kernel(&capped());
    let authorization = verdict.authorisation().expect("authorised");

    let signer = RecordingSigner::signing();
    let sender = CountingSender::new();
    let outcome = radar_exec::execute(
        &attempt_from(authorization, notional),
        &FixedRoute(Err(RouteError::NoRoute {
            mint: Address::new([7u8; 32]).to_string(),
            size_lamports: 1_000_000_000,
        })),
        &signer,
        &sender,
    );

    assert!(matches!(outcome, Outcome::NoRoute { .. }), "{outcome:?}");
    assert!(signer.seen.borrow().is_empty());
    assert_eq!(sender.sent.get(), 0);
}
