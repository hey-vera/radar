// SPDX-License-Identifier: Apache-2.0
//! A grant is the kernel's numbers, or it is not a grant.
//!
//! This is invariant 1's seam for customer capital, and it is a seam where the
//! failure is silent: Privy's policy engine is what actually refuses a customer
//! transaction, so whatever it is told *is* the bound. A derivation that widened
//! a ceiling, extended a window, or substituted a default would not error, would
//! not appear in a log, and would produce a signer permitted to do more than the
//! kernel authorised — while every test of the kernel itself continued to pass.
//!
//! So these assert equality against the authorisation field by field, rather
//! than asserting that a grant came back.

use radar_customer::{Grant, NotGranted};
use radar_risk::{Action, Address, Authorization, MicroUsd, Slot};

fn mint() -> Address {
    Address::new([7u8; 32])
}

/// An authorisation the kernel would plausibly issue.
fn authorised() -> Authorization {
    Authorization {
        nonce: "b3-of-the-proposal-and-the-state".to_owned(),
        mint: mint(),
        action: Action::Buy,
        max_notional: MicroUsd(6_210_000),
        expires_after: Slot(500),
        needs_operator_signature: false,
    }
}

#[test]
fn every_bound_handed_to_the_policy_engine_is_the_one_the_kernel_issued() {
    // Field by field. A test asserting only that `derive` returned `Ok` would
    // pass against a derivation that halved the ceiling or doubled the window,
    // and doubling the window is the direction that costs money.
    let authorization = authorised();
    let grant = Grant::derive(&authorization, Slot(100)).expect("a live authorisation grants");

    assert_eq!(grant.bounds.mint, authorization.mint);
    assert_eq!(grant.bounds.max_notional, authorization.max_notional);
    assert_eq!(grant.bounds.expires_after, authorization.expires_after);
    assert_eq!(grant.bounds.action, authorization.action);
    assert_eq!(
        grant.nonce, authorization.nonce,
        "the grant must be traceable to the judgement it came from"
    );
}

#[test]
fn an_authorisation_that_still_needs_a_person_grants_nothing() {
    // `Autonomy::Approve` sets this, and the kernel means it: the trade is
    // *within policy*, and a human still has to say go. Deriving an unattended
    // grant from it would collapse those two into one, which is the distinction
    // invariant 1 is made of.
    let authorization = Authorization {
        needs_operator_signature: true,
        ..authorised()
    };
    assert_eq!(
        Grant::derive(&authorization, Slot(100)),
        Err(NotGranted::NeedsOperatorSignature)
    );
}

#[test]
fn a_closed_policys_authorisation_produces_no_grant_at_all() {
    // Not an inert grant with a zero ceiling — no grant. A signer registered
    // with a zero bound is a signer that exists, appears in the provider's
    // dashboard, and does nothing; the absence should be visible instead.
    let authorization = Authorization {
        max_notional: MicroUsd::ZERO,
        ..authorised()
    };
    assert_eq!(
        Grant::derive(&authorization, Slot(100)),
        Err(NotGranted::NoNotional)
    );
}

#[test]
fn an_authorisation_that_has_already_expired_grants_nothing() {
    // Swept across the boundary rather than sampled, because the interesting
    // case is the slot where it flips and an off-by-one here hands a policy
    // engine a window that has already closed.
    let authorization = authorised();
    let expiry = authorization.expires_after.0;

    assert!(
        Grant::derive(&authorization, Slot(expiry - 1)).is_ok(),
        "a slot before the expiry is still live"
    );
    assert_eq!(
        Grant::derive(&authorization, Slot(expiry)),
        Err(NotGranted::AlreadyExpired {
            expires_after: Slot(expiry),
            at: Slot(expiry),
        }),
        "`expires_after` means void *after* that slot, so the slot itself does not grant"
    );
    assert!(
        matches!(
            Grant::derive(&authorization, Slot(expiry + 1)),
            Err(NotGranted::AlreadyExpired { .. })
        ),
        "and every slot beyond it"
    );
}

#[test]
fn the_refusals_are_checked_before_the_bounds_are_read() {
    // Ordering, and it matters for one reason: an authorisation that is both
    // expired *and* still needs a person must not report the expiry, because a
    // reader fixing the expiry would then find a second refusal behind it. The
    // one that can never be satisfied by waiting is the one to report.
    let authorization = Authorization {
        needs_operator_signature: true,
        max_notional: MicroUsd::ZERO,
        expires_after: Slot(1),
        ..authorised()
    };
    assert_eq!(
        Grant::derive(&authorization, Slot(9_999)),
        Err(NotGranted::NeedsOperatorSignature)
    );
}
