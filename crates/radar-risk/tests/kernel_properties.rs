// SPDX-License-Identifier: Apache-2.0
//! The kernel's invariants, over generated inputs rather than chosen ones.
//!
//! `kernel_invariants.rs` checks the cases somebody thought of. This checks the
//! ones nobody did. Several of AGENTS.md's rules are **universally quantified** —
//! *`Policy::CLOSED` refuses every proposal*, *the kernel is pure*, *absent is
//! not zero* — and a claim about every input is not established by a handful of
//! them, however carefully chosen.
//!
//! gitlocus ADR 0018 is the record of this repository's owner learning that the
//! hard way in a sibling project, and it is why the plan listed property tests
//! as a capability gap rather than as polish.
//!
//! # What is generated
//!
//! Whole `Proposal` and `PortfolioState` values, including the shapes a
//! hand-written test would not bother with: zero notionals, `u64::MAX` costs,
//! exit capacity absent, a portfolio already over every limit, an input from the
//! future. The generators deliberately produce **invalid** states as well as
//! plausible ones — the kernel is the thing that decides what is acceptable, so
//! a generator that only produced acceptable inputs would be assuming the
//! answer.

use std::collections::BTreeMap;

use proptest::prelude::*;
use radar_risk::{Action, Policy, PortfolioState, Proposal, Refusal, Verdict, evaluate};
use radar_types::{Address, MicroUsd, Slot, SlotDelta};

/// An address from a single byte, so shrinking produces readable failures.
fn address() -> impl Strategy<Value = Address> {
    any::<u8>().prop_map(|b| Address::new([b; 32]))
}

fn action() -> impl Strategy<Value = Action> {
    prop_oneof![Just(Action::Buy), Just(Action::Reduce), Just(Action::Exit)]
}

/// Money, weighted towards small values but reaching the top of the range.
///
/// The extremes matter here: the kernel multiplies notionals by basis points,
/// and an overflow would be a limit that wraps around into permission.
fn money() -> impl Strategy<Value = MicroUsd> {
    prop_oneof![
        3 => (0u64..10_000_000).prop_map(MicroUsd),
        1 => Just(MicroUsd::ZERO),
        1 => Just(MicroUsd(u64::MAX)),
        1 => (u64::MAX / 2..u64::MAX).prop_map(MicroUsd),
    ]
}

fn proposal() -> impl Strategy<Value = Proposal> {
    (
        address(),
        address(),
        action(),
        money(),
        money(),
        0u64..1_000_000_000,
        proptest::option::of(money()),
    )
        .prop_map(
            |(mint, creator, action, notional, cost, slot, capacity)| Proposal {
                mint,
                creator,
                action,
                notional,
                estimated_round_trip_cost: cost,
                oldest_input_slot: Slot(slot),
                simulated_exit_capacity: capacity,
            },
        )
}

fn portfolio() -> impl Strategy<Value = PortfolioState> {
    (
        0u64..1_000_000_000,
        money(),
        money(),
        any::<u32>(),
        any::<bool>(),
        proptest::collection::vec((address(), money()), 0..4),
    )
        .prop_map(
            |(now, deployed, loss, failures, halted, per_creator)| PortfolioState {
                now: Slot(now),
                deployed,
                per_creator: per_creator.into_iter().collect::<BTreeMap<_, _>>(),
                realised_loss_today: loss,
                consecutive_failures: failures,
                halted,
            },
        )
}

/// Policies across the whole range, including ones nobody would ship.
fn policy() -> impl Strategy<Value = Policy> {
    (
        prop_oneof![
            Just(radar_risk::Autonomy::Observe),
            Just(radar_risk::Autonomy::Alert),
            Just(radar_risk::Autonomy::Approve),
            Just(radar_risk::Autonomy::Canary),
            Just(radar_risk::Autonomy::Capped),
            Just(radar_risk::Autonomy::Auto),
        ],
        money(),
        money(),
        money(),
        money(),
        money(),
        any::<u32>(),
        any::<u32>(),
        0u64..100_000,
    )
        .prop_map(
            |(
                autonomy,
                max_canary,
                max_position,
                max_deployed,
                max_per_creator,
                max_daily_loss,
                max_failures,
                cost_bps,
                staleness,
            )| Policy {
                max_canary,
                autonomy,
                max_position,
                max_deployed,
                max_per_creator,
                max_daily_loss,
                max_consecutive_failures: max_failures,
                max_round_trip_cost_bps: cost_bps,
                max_input_staleness: SlotDelta(staleness),
            },
        )
}

proptest! {
    /// AGENTS.md: *the shipped policy is `Policy::CLOSED`, which refuses every
    /// proposal.* Stated as an absolute, so it is checked as one.
    #[test]
    fn the_closed_policy_refuses_every_proposal(
        proposal in proposal(),
        state in portfolio(),
    ) {
        let verdict = evaluate(&proposal, &state, &Policy::CLOSED);
        prop_assert!(
            matches!(verdict, Verdict::Refused { .. }),
            "CLOSED authorised {proposal:?} against {state:?}"
        );
    }

    /// Rule 2: *the risk kernel is pure.* No clock, no ambient state, and no
    /// dependence on the order of its inputs — which is what makes a refusal
    /// reproducible from a recording rather than a matter of trust.
    #[test]
    fn evaluating_twice_gives_the_same_verdict(
        proposal in proposal(),
        state in portfolio(),
        policy in policy(),
    ) {
        let first = evaluate(&proposal, &state, &policy);
        let second = evaluate(&proposal, &state, &policy);
        // Nonces differ by construction, so the comparison is over everything
        // else. A verdict that differed in its *reasons* between two identical
        // calls would mean a replay could not reproduce a refusal.
        match (&first, &second) {
            (Verdict::Refused { reasons: a }, Verdict::Refused { reasons: b }) => {
                prop_assert_eq!(a, b, "the same refusal, twice");
            }
            (Verdict::Authorised(a), Verdict::Authorised(b)) => {
                prop_assert_eq!(a.max_notional, b.max_notional);
                prop_assert_eq!(a.expires_after, b.expires_after);
                prop_assert_eq!(a.needs_operator_signature, b.needs_operator_signature);
            }
            _ => prop_assert!(false, "one call authorised and the other refused"),
        }
    }

    /// A refusal always says why. A `Refused` carrying no reasons is a dead end
    /// for whoever has to act on it, and it is the shape a partially-written
    /// check produces.
    #[test]
    fn a_refusal_is_never_silent(
        proposal in proposal(),
        state in portfolio(),
        policy in policy(),
    ) {
        if let Verdict::Refused { reasons } = evaluate(&proposal, &state, &policy) {
            prop_assert!(!reasons.is_empty(), "refused for no stated reason");
        }
    }

    /// The reasons are in a fixed order and carry no duplicates.
    ///
    /// Order matters because the list is recorded and compared across runs; a
    /// set that came back in a different order each time would make two
    /// identical refusals look different in the store.
    #[test]
    fn refusal_reasons_are_sorted_and_unique(
        proposal in proposal(),
        state in portfolio(),
        policy in policy(),
    ) {
        if let Verdict::Refused { reasons } = evaluate(&proposal, &state, &policy) {
            let mut sorted = reasons.clone();
            sorted.sort_unstable();
            sorted.dedup();
            prop_assert_eq!(&sorted, &reasons, "a fixed order, without repeats");
        }
    }

    /// Rule 9: *a capacity that could not be measured is `None`, and `None`
    /// means "cannot exit", never "no limit found".*
    ///
    /// The one Radar's whole premise rests on: most losses are an inability to
    /// exit, so an unsimulated exit is the thing it must not wave through.
    #[test]
    fn an_unsimulated_exit_is_never_authorised_to_buy(
        mut proposal in proposal(),
        state in portfolio(),
        policy in policy(),
    ) {
        proposal.action = Action::Buy;
        proposal.simulated_exit_capacity = None;

        let verdict = evaluate(&proposal, &state, &policy);
        prop_assert!(
            verdict.authorisation().is_none(),
            "authorised a buy with no simulated exit: {proposal:?}"
        );
    }

    /// A halt is checked first and unconditionally. *A kill switch that can be
    /// reasoned past is not a kill switch* — including by a policy that would
    /// otherwise permit everything.
    #[test]
    fn a_halt_cannot_be_reasoned_past(
        proposal in proposal(),
        mut state in portfolio(),
        policy in policy(),
    ) {
        state.halted = true;
        let verdict = evaluate(&proposal, &state, &policy);
        prop_assert!(verdict.authorisation().is_none(), "authorised while halted");
        if let Verdict::Refused { reasons } = verdict {
            prop_assert!(
                reasons.contains(&Refusal::Halted),
                "halted, and the reasons do not say so: {reasons:?}"
            );
        }
    }

    /// An authorisation to *increase exposure* never exceeds the position limit.
    ///
    /// This is the number the signer re-checks against the decoded transaction,
    /// so an authorisation exceeding it would be a bound that means nothing —
    /// the last line before capital moves.
    ///
    /// The qualifier is not hedging. The first version of this property said
    /// "an authorisation never exceeds the position limit" and failed
    /// immediately on an `Exit` of `u64::MAX`. The kernel was right and the
    /// property was wrong: **sizing limits apply only to actions that increase
    /// exposure**, because refusing an exit for being too large would trap
    /// capital in precisely the situation the limits exist to prevent. Writing
    /// the property forced the invariant to be stated exactly, and the exact
    /// statement is narrower than the obvious one.
    #[test]
    fn an_authorisation_to_buy_never_exceeds_the_position_limit(
        mut proposal in proposal(),
        state in portfolio(),
        policy in policy(),
    ) {
        proposal.action = Action::Buy;
        if let Some(authorisation) = evaluate(&proposal, &state, &policy).authorisation() {
            prop_assert!(
                authorisation.max_notional <= policy.max_position,
                "authorised {} against a limit of {}",
                authorisation.max_notional.get(),
                policy.max_position.get()
            );
            prop_assert!(
                authorisation.max_notional <= proposal.notional,
                "authorised more than was asked for"
            );
        }
    }

    /// The other half of that exemption, which nothing else tests.
    ///
    /// Getting out must never be refused for a sizing reason. A position that
    /// grew past every limit is the one it is most urgent to close, and a
    /// kernel that refused to close it would be enforcing the limit by holding
    /// the thing that breached it.
    #[test]
    fn getting_out_is_never_refused_for_being_too_large(
        mut proposal in proposal(),
        state in portfolio(),
        policy in policy(),
        exit in prop_oneof![Just(Action::Exit), Just(Action::Reduce)],
    ) {
        proposal.action = exit;

        if let Verdict::Refused { reasons } = evaluate(&proposal, &state, &policy) {
            for sizing in [
                Refusal::OverPositionLimit,
                Refusal::OverDeploymentLimit,
                Refusal::OverCreatorLimit,
                Refusal::OverCanaryLimit,
                Refusal::ExitCapacityTooSmall,
                Refusal::ExitNotSimulated,
                Refusal::RoundTripTooExpensive,
            ] {
                prop_assert!(
                    !reasons.contains(&sizing),
                    "{exit:?} refused for {sizing:?}, which would trap capital"
                );
            }
        }
    }

    /// An authorisation is for the mint and the action that were proposed.
    ///
    /// Cheap to state and the sort of thing a refactor breaks silently: an
    /// authorisation naming a different mint would be checked by the signer
    /// against the wrong transaction and pass.
    #[test]
    fn an_authorisation_is_for_what_was_proposed(
        proposal in proposal(),
        state in portfolio(),
        policy in policy(),
    ) {
        if let Some(authorisation) = evaluate(&proposal, &state, &policy).authorisation() {
            prop_assert_eq!(authorisation.mint, proposal.mint);
            prop_assert_eq!(authorisation.action, proposal.action);
            prop_assert!(
                authorisation.expires_after >= state.now,
                "an authorisation that expired before it was issued"
            );
        }
    }

    /// Nothing overflows into permission.
    ///
    /// The kernel multiplies a notional by basis points, and at `u64::MAX` a
    /// wrapping multiply produces a small number — which is a cost ceiling that
    /// silently becomes generous exactly where the numbers are most absurd.
    /// Generating the top of the range is the whole point of this one.
    #[test]
    fn an_enormous_cost_is_never_authorised_under_a_tight_ceiling(
        mut proposal in proposal(),
        state in portfolio(),
    ) {
        proposal.action = Action::Buy;
        proposal.estimated_round_trip_cost = MicroUsd(u64::MAX);
        proposal.notional = MicroUsd(1_000_000);

        let policy = Policy {
            autonomy: radar_risk::Autonomy::Auto,
            max_canary: MicroUsd(u64::MAX),
            max_position: MicroUsd(u64::MAX),
            max_deployed: MicroUsd(u64::MAX),
            max_per_creator: MicroUsd(u64::MAX),
            max_daily_loss: MicroUsd(u64::MAX),
            max_consecutive_failures: u32::MAX,
            // One basis point: a cost of one micro-dollar would be too much.
            max_round_trip_cost_bps: 1,
            max_input_staleness: SlotDelta(u64::MAX),
        };

        let verdict = evaluate(&proposal, &state, &policy);
        prop_assert!(
            verdict.authorisation().is_none(),
            "the largest cost representable was authorised under a one-bps ceiling"
        );
    }
}
