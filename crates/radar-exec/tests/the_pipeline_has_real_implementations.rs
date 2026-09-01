// SPDX-License-Identifier: Apache-2.0
//! The pipeline's traits have implementations that are not test stubs.
//!
//! Until 2026-09-01 they did not. `Routing` and `Sending` were implemented only
//! by `FixedRoute` and `CountingSender` inside `pipeline.rs`'s own test module,
//! so the executor could be composed **only** against a fixture — while
//! `route::Router` and `submit::Submitter`, which talk to Jupiter and to an RPC
//! node, sat beside them unconnected.
//!
//! That is exactly the shape [LEARNINGS](../../../LEARNINGS.md) 10 records: a
//! lane whose every stage passes its tests against something the real one would
//! never produce. A live run over 41,254 candidates then raised zero proposals,
//! because a hardcoded probe size made a proposal arithmetically impossible and
//! no fixture had ever been shaped like a real candidate.
//!
//! These tests cannot reach Jupiter or an RPC node, and should not: what they
//! establish is that the trait methods **delegate to the real ones** rather than
//! being present and inert. Both point at an address nothing answers on, so a
//! stub returning success would be visible immediately.

use radar_exec::pipeline::{Routing, Sending};
use radar_exec::route::Router;
use radar_exec::submit::Submitter;
use radar_types::Address;

/// An endpoint that refuses a connection immediately.
///
/// Port 1 on the loopback, deliberately, and not a black-holed address like
/// `192.0.2.1`. That was the first choice and it cost thirty seconds per run:
/// a black hole is not refused, it is waited on, so each of these tests paid a
/// full connect timeout to learn something a refusal says at once.
///
/// Port 1 needs root to bind and nothing does, so a refusal is what arrives.
const NOWHERE: &str = "http://127.0.0.1:1/";

#[test]
fn routing_reaches_the_real_router() {
    // A stub would answer without touching the network. The real one tries, and
    // fails, which is the observable difference.
    let router = Router::new(NOWHERE, NOWHERE);
    let outcome = Routing::build_buy(
        &router,
        &Address::new([0x22; 32]),
        &Address::new([0x33; 32]),
        1_000_000,
    );
    assert!(
        outcome.is_err(),
        "an unreachable Jupiter must produce an error, not a route"
    );
}

#[test]
fn the_trait_and_the_method_give_the_same_answer() {
    // The delegation itself. An implementation that ignored its arguments and
    // returned a fixed error would pass the test above; this one compares the
    // two paths to the same failure.
    let router = Router::new(NOWHERE, NOWHERE);
    let mint = Address::new([0x22; 32]);
    let wallet = Address::new([0x33; 32]);

    let direct = router
        .build_buy(&mint, &wallet, 1_000_000)
        .expect_err("unreachable");
    let through_trait =
        Routing::build_buy(&router, &mint, &wallet, 1_000_000).expect_err("unreachable");

    assert_eq!(
        format!("{direct}"),
        format!("{through_trait}"),
        "the trait method must be the inherent one, not a second implementation"
    );
}

#[test]
fn sending_reaches_the_real_submitter() {
    // Rule 7 lives in `Submitter`: it takes a direct RPC endpoint and never the
    // x402 lane. A stub here would quietly bypass that.
    let submitter = Submitter::new(NOWHERE);
    let outcome = Sending::send(&submitter, "AQAB");
    assert!(
        outcome.is_err(),
        "an unreachable node must produce an error, not a signature"
    );
}

#[test]
fn the_nodes_own_words_survive_to_the_caller() {
    // `Sending` carries whatever the node said rather than a category this crate
    // chose for it. An operator at three in the morning needs the node's words;
    // a label picked in advance by this code tells them less.
    let submitter = Submitter::new(NOWHERE);
    let through_trait = Sending::send(&submitter, "AQAB").expect_err("unreachable");
    let direct = submitter.send("AQAB").expect_err("unreachable");

    assert_eq!(
        through_trait,
        direct.to_string(),
        "the flattened error must be the real one's own message"
    );
    assert!(
        !through_trait.is_empty(),
        "an empty reason is not a reason: {through_trait}"
    );
}
