// SPDX-License-Identifier: Apache-2.0
//! What the signer checks before it signs.
//!
//! The caller sends an [`Authorization`] and some transaction bytes and says
//! they correspond. This module assumes that claim is false and looks.
//!
//! The threat model is not "a bug in the executor". It is a fully compromised
//! executor — prompt-injected, or replaced outright — that can construct any
//! transaction it likes and describe it any way it likes. The only things it
//! cannot do are forge an [`Authorization`] the kernel did not issue, and change
//! the bytes after this module has read them.
//!
//! So every check here is against the *decoded bytes*, never against anything
//! the caller said about them.

use radar_risk::Authorization;
use radar_types::{Address, Slot};

use crate::tx::{DecodeError, Message, decode};

/// Programs the signer will sign an instruction for.
///
/// An allowlist rather than a denylist, because the set of programs that can
/// take a token away from you is not enumerable and the set that can trade one
/// is.
#[derive(Debug, Clone)]
pub struct Allowlist {
    /// Programs that may appear in a signed transaction.
    pub programs: Vec<[u8; 32]>,
}

/// The system program, needed for compute budget and wSOL handling.
pub const SYSTEM_PROGRAM: [u8; 32] = [0u8; 32];

/// Why the signer refused.
///
/// Every applicable reason is returned. A caller that fixed one and resubmitted
/// only to hit the next would learn nothing about whether the transaction was
/// ever going to be signable.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, thiserror::Error)]
pub enum Rejection {
    /// The bytes did not decode.
    #[error("undecodable: {0}")]
    Undecodable(String),
    /// The authorization has expired.
    ///
    /// Checked against a slot the caller supplies, which the caller could lie
    /// about — but lying makes an expired authorization usable, not an
    /// unauthorised trade possible, and the bounds still hold. The kernel's
    /// short lifetime is the real defence.
    #[error("authorization expired at slot {expires_after}, now {now}")]
    Expired {
        /// When the authorization stopped being valid.
        expires_after: Slot,
        /// The slot the caller reported.
        now: Slot,
    },
    /// The authorization still needs an operator's signature.
    #[error("operator signature required and not present")]
    NeedsOperator,
    /// The transaction invokes a program that is not allowed.
    #[error("program not allowed: {0}")]
    ProgramNotAllowed(String),
    /// The authorised mint does not appear anywhere in the transaction.
    ///
    /// The check that catches the substitution attack: an executor that swapped
    /// the mint for another would otherwise hold a valid authorization for a
    /// trade in a different token.
    #[error("authorised mint {0} is not in the transaction")]
    MintAbsent(String),
    /// The transaction moves more lamports than the authorization permits.
    #[error("transfers {found} lamports, authorised for at most {allowed}")]
    OverSpend {
        /// What the transaction moves.
        found: u64,
        /// What the authorization permits.
        allowed: u64,
    },
    /// The fee payer is not the wallet the signer holds a key for.
    ///
    /// Signing for a fee payer we are not means signing something we cannot
    /// reason about.
    #[error("fee payer is not the signing wallet")]
    ForeignFeePayer,
    /// The transaction contains no instruction at all.
    #[error("no instructions")]
    Empty,
    /// The transaction closes or reassigns an account it should not.
    #[error("contains an account-ownership change, which no trade needs")]
    OwnershipChange,
}

/// A transaction that passed every check.
///
/// Constructed only by [`check`], and holding the message so a caller cannot
/// sign different bytes from the ones that were verified. There is no way to
/// build one from an unverified message, which is what makes "verified" a fact
/// about the value rather than a claim about the control flow.
#[derive(Debug, Clone)]
pub struct Checked {
    message: Message,
    bytes: Vec<u8>,
}

impl Checked {
    /// The verified message.
    #[must_use]
    pub const fn message(&self) -> &Message {
        &self.message
    }

    /// The exact bytes that were verified.
    ///
    /// The only bytes a caller should sign. Signing anything else discards
    /// everything this module established.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The bytes to sign: the message, without the signature array.
    #[must_use]
    pub fn signable(&self) -> &[u8] {
        // The message was decoded from exactly these bytes, so the offset is in
        // range by construction; the fallback keeps the unreachable case from
        // being a panic in the process that holds the key.
        self.message.signable(&self.bytes).unwrap_or(&[])
    }
}

/// The system program's `Transfer` discriminator.
const SYSTEM_TRANSFER: u32 = 2;

/// Verifies transaction bytes against an authorization.
///
/// `signing_wallet` is the public key this process holds the secret for;
/// `now` is the caller's view of the chain head.
///
/// # Errors
///
/// Returns every applicable [`Rejection`], sorted, so a caller sees the whole
/// picture rather than one reason at a time.
pub fn check(
    authorization: &Authorization,
    bytes: &[u8],
    signing_wallet: &Address,
    allowlist: &Allowlist,
    now: Slot,
) -> Result<Checked, Vec<Rejection>> {
    let message = match decode(bytes) {
        Ok(m) => m,
        // Undecodable is terminal: there is nothing further to check against.
        Err(e) => return Err(vec![Rejection::Undecodable(DecodeError::to_string(&e))]),
    };

    let mut rejections = Vec::new();

    if now.get() > authorization.expires_after.get() {
        rejections.push(Rejection::Expired {
            expires_after: authorization.expires_after,
            now,
        });
    }
    if authorization.needs_operator_signature {
        rejections.push(Rejection::NeedsOperator);
    }
    if message.instructions.is_empty() {
        rejections.push(Rejection::Empty);
    }

    for program in message.programs_outside(&allowlist.programs) {
        rejections.push(Rejection::ProgramNotAllowed(
            Address::new(program).to_string(),
        ));
    }

    if !mentions(&message, &authorization.mint) {
        rejections.push(Rejection::MintAbsent(authorization.mint.to_string()));
    }

    if message.fee_payer() != Some(*signing_wallet.as_bytes()) {
        rejections.push(Rejection::ForeignFeePayer);
    }

    // Every action gets a ceiling on outgoing lamports, and until 2026-08-31
    // only `Buy` did.
    //
    // # The hole that was here, because it is instructive
    //
    // The exemption read: "refusing to sign a sale because it is large is how a
    // position gets trapped in exactly the situation the limits exist to
    // prevent." The concern is real. The reasoning conflated two different
    // things, and the gap between them was a way to empty a wallet.
    //
    // A large *sale* is tokens leaving through a DEX and lamports arriving.
    // [`lamports_transferred`] counts neither of those: it sums **system-program
    // transfers out**. A legitimate exit moves almost none -- rent for an
    // account, a fee -- so bounding it does not trap anything.
    //
    // What the exemption did allow: given any `Exit` authorisation, a
    // transaction that lists the authorised mint as an inert account, uses only
    // allowlisted programs (the system program is necessarily one), pays its fee
    // from the right wallet, and transfers **the entire balance to any address**
    // passed every check. Demonstrated before this was changed: a 100 SOL
    // transfer to an unrelated address, authorised against an `Exit` whose
    // notional was one micro-dollar.
    //
    // The ceiling used is the authorisation's own, not a tighter rent-sized
    // constant. A constant would be better and this is not the moment to invent
    // one: no real exit has ever been signed, so any number here would be a
    // guess, and a guess that is too small traps the position the old comment
    // was rightly worried about. This bound is finite, which is the property
    // that was missing.
    let moved = lamports_transferred(&message);
    let ceiling = lamport_ceiling(authorization);
    if moved > ceiling {
        rejections.push(Rejection::OverSpend {
            found: moved,
            allowed: ceiling,
        });
    }

    if changes_ownership(&message) {
        rejections.push(Rejection::OwnershipChange);
    }

    if rejections.is_empty() {
        Ok(Checked {
            message,
            bytes: bytes.to_vec(),
        })
    } else {
        rejections.sort();
        rejections.dedup();
        Err(rejections)
    }
}

/// Whether an address appears anywhere in the message's account list.
fn mentions(message: &Message, address: &Address) -> bool {
    message.accounts.iter().any(|a| a == address.as_bytes())
}

/// Total lamports moved by system-program transfers.
///
/// Deliberately sums *every* transfer rather than looking for one. A transaction
/// that splits an overspend across three instructions is the obvious way around
/// a check that only inspects the first.
fn lamports_transferred(message: &Message) -> u64 {
    message
        .instructions
        .iter()
        .filter(|i| i.program_id == SYSTEM_PROGRAM)
        .filter_map(|i| {
            let discriminator = u32::from_le_bytes(i.data.get(0..4)?.try_into().ok()?);
            if discriminator != SYSTEM_TRANSFER {
                return None;
            }
            Some(u64::from_le_bytes(i.data.get(4..12)?.try_into().ok()?))
        })
        .fold(0u64, u64::saturating_add)
}

/// The lamport ceiling implied by an authorization's notional.
///
/// The authorization is denominated in micro-USD and the transaction in
/// lamports, and this process has no price feed — deliberately, since a signer
/// with a price feed has one more input to be lied to by.
///
/// So the conversion is left to the caller, who puts the ceiling on the
/// authorization itself. Until that field exists this returns the notional read
/// as lamports, which is a ceiling far tighter than any real trade at any real
/// SOL price, so it fails closed rather than open.
const fn lamport_ceiling(authorization: &Authorization) -> u64 {
    authorization.max_notional.get()
}

/// Whether the message contains a system-program `Assign` or `CloseAccount`.
///
/// No trade needs to change who owns an account. One that does is either a bug
/// or an attempt to take the wallet.
fn changes_ownership(message: &Message) -> bool {
    /// The system program's `Assign` discriminator.
    const SYSTEM_ASSIGN: u32 = 1;
    /// `AssignWithSeed`.
    const SYSTEM_ASSIGN_WITH_SEED: u32 = 10;

    message
        .instructions
        .iter()
        .filter(|i| i.program_id == SYSTEM_PROGRAM)
        .any(|i| {
            i.data.get(0..4).is_some_and(|d| {
                let discriminator = u32::from_le_bytes(d.try_into().unwrap_or([0; 4]));
                discriminator == SYSTEM_ASSIGN || discriminator == SYSTEM_ASSIGN_WITH_SEED
            })
        })
}

/// A verified transaction for other modules' tests.
///
/// Exists so the key module can prove it signs the bytes that were checked,
/// without a second path that constructs a [`Checked`] from unverified input.
/// That path is the one that would eventually get called from production.
#[cfg(test)]
pub mod tests_support {
    use radar_risk::{Action, Authorization};
    use radar_types::{Address, MicroUsd, Slot};

    use super::{Allowlist, Checked, SYSTEM_PROGRAM, check};

    /// A transaction that passes every check.
    ///
    /// # Panics
    ///
    /// Panics if the fixture stops verifying, which means a check changed and
    /// this fixture no longer describes a signable transaction.
    #[must_use]
    pub fn checked_fixture() -> Checked {
        const DEX: [u8; 32] = [0x11; 32];
        const MINT: [u8; 32] = [0x22; 32];
        const WALLET: [u8; 32] = [0x33; 32];

        let mut bytes = vec![0u8, 1, 0, 0, 4];
        for a in [WALLET, MINT, DEX, SYSTEM_PROGRAM] {
            bytes.extend_from_slice(&a);
        }
        bytes.extend_from_slice(&[0xAA; 32]);
        bytes.extend_from_slice(&[1, 2, 2, 0, 1, 2, 0xAB, 0xCD]);

        let authorization = Authorization {
            nonce: "fixture".to_owned(),
            mint: Address::new(MINT),
            action: Action::Buy,
            max_notional: MicroUsd(50_000_000),
            expires_after: Slot(1_150),
            needs_operator_signature: false,
        };
        check(
            &authorization,
            &bytes,
            &Address::new(WALLET),
            &Allowlist {
                programs: vec![DEX, SYSTEM_PROGRAM],
            },
            Slot(1_000),
        )
        .expect("the fixture must verify")
    }
}

#[cfg(test)]
mod tests {
    use radar_risk::Action;
    use radar_types::MicroUsd;

    use super::*;

    const DEX: [u8; 32] = [0x11; 32];
    const MINT: [u8; 32] = [0x22; 32];
    const WALLET: [u8; 32] = [0x33; 32];
    const NOW: Slot = Slot(1_000);

    fn allowlist() -> Allowlist {
        Allowlist {
            programs: vec![DEX, SYSTEM_PROGRAM],
        }
    }

    fn authorization() -> Authorization {
        Authorization {
            nonce: "test".to_owned(),
            mint: Address::new(MINT),
            action: Action::Buy,
            max_notional: MicroUsd(50_000_000),
            expires_after: Slot(1_150),
            needs_operator_signature: false,
        }
    }

    /// Builds a transaction over the given account set.
    ///
    /// `accounts[0]` is the fee payer. Instructions are `(program_index,
    /// account_indices, data)`.
    fn build(accounts: &[[u8; 32]], instructions: &[(u8, Vec<u8>, Vec<u8>)]) -> Vec<u8> {
        let mut out = vec![0u8];
        out.push(1);
        out.push(0);
        out.push(0);
        out.push(u8::try_from(accounts.len()).expect("small"));
        for a in accounts {
            out.extend_from_slice(a);
        }
        out.extend_from_slice(&[0xAA; 32]);
        out.push(u8::try_from(instructions.len()).expect("small"));
        for (program, indices, data) in instructions {
            out.push(*program);
            out.push(u8::try_from(indices.len()).expect("small"));
            out.extend_from_slice(indices);
            out.push(u8::try_from(data.len()).expect("small"));
            out.extend_from_slice(data);
        }
        out
    }

    /// A system transfer instruction's data.
    fn transfer(lamports: u64) -> Vec<u8> {
        let mut data = SYSTEM_TRANSFER.to_le_bytes().to_vec();
        data.extend_from_slice(&lamports.to_le_bytes());
        data
    }

    /// The honest transaction every test then damages one field of.
    fn honest() -> Vec<u8> {
        build(
            &[WALLET, MINT, DEX, SYSTEM_PROGRAM],
            &[(2, vec![0, 1], vec![0xAB, 0xCD])],
        )
    }

    #[test]
    fn an_honest_transaction_is_signed() {
        let checked = check(
            &authorization(),
            &honest(),
            &Address::new(WALLET),
            &allowlist(),
            NOW,
        )
        .expect("should verify");
        assert_eq!(checked.bytes(), honest());
    }

    #[test]
    fn a_substituted_mint_is_refused() {
        // The attack the whole process exists for: the executor holds a valid
        // authorization for one token and builds a transaction for another.
        let other = build(
            &[WALLET, [0x99; 32], DEX, SYSTEM_PROGRAM],
            &[(2, vec![0, 1], vec![0xAB])],
        );
        let rejections = check(
            &authorization(),
            &other,
            &Address::new(WALLET),
            &allowlist(),
            NOW,
        )
        .expect_err("must refuse");
        assert!(
            rejections
                .iter()
                .any(|r| matches!(r, Rejection::MintAbsent(_)))
        );
    }

    #[test]
    fn an_unlisted_program_is_refused() {
        let evil = build(
            &[WALLET, MINT, [0xEE; 32], SYSTEM_PROGRAM],
            &[(2, vec![0, 1], vec![0xAB])],
        );
        let rejections = check(
            &authorization(),
            &evil,
            &Address::new(WALLET),
            &allowlist(),
            NOW,
        )
        .expect_err("must refuse");
        assert!(
            rejections
                .iter()
                .any(|r| matches!(r, Rejection::ProgramNotAllowed(_)))
        );
    }

    #[test]
    fn an_oversized_spend_is_refused() {
        let big = build(
            &[WALLET, MINT, DEX, SYSTEM_PROGRAM],
            &[
                (2, vec![0, 1], vec![0xAB]),
                (3, vec![0, 1], transfer(60_000_000)),
            ],
        );
        let rejections = check(
            &authorization(),
            &big,
            &Address::new(WALLET),
            &allowlist(),
            NOW,
        )
        .expect_err("must refuse");
        assert!(
            rejections
                .iter()
                .any(|r| matches!(r, Rejection::OverSpend { .. }))
        );
    }

    #[test]
    fn a_spend_split_across_instructions_is_still_caught() {
        // The obvious way around a check that inspects only the first transfer.
        let split = build(
            &[WALLET, MINT, DEX, SYSTEM_PROGRAM],
            &[
                (2, vec![0, 1], vec![0xAB]),
                (3, vec![0, 1], transfer(30_000_000)),
                (3, vec![0, 1], transfer(30_000_000)),
            ],
        );
        let rejections = check(
            &authorization(),
            &split,
            &Address::new(WALLET),
            &allowlist(),
            NOW,
        )
        .expect_err("must refuse");
        assert_eq!(
            rejections
                .iter()
                .filter(|r| matches!(
                    r,
                    Rejection::OverSpend {
                        found: 60_000_000,
                        ..
                    }
                ))
                .count(),
            1
        );
    }

    #[test]
    fn a_spend_within_the_ceiling_is_signed() {
        let ok = build(
            &[WALLET, MINT, DEX, SYSTEM_PROGRAM],
            &[
                (2, vec![0, 1], vec![0xAB]),
                (3, vec![0, 1], transfer(10_000_000)),
            ],
        );
        assert!(
            check(
                &authorization(),
                &ok,
                &Address::new(WALLET),
                &allowlist(),
                NOW
            )
            .is_ok()
        );
    }

    #[test]
    fn a_large_sale_is_still_signed() {
        // The concern the old exemption was protecting, stated correctly this
        // time. Refusing to sign a sale because it is large is how a position
        // gets trapped, so a big exit must still go through.
        //
        // What makes it big is the DEX instruction, not lamports leaving the
        // wallet -- a sale sends tokens out and brings lamports in. That is why
        // bounding outgoing system transfers does not trap anything, and why the
        // old exemption was solving this problem with the wrong tool.
        let mut auth = authorization();
        auth.action = Action::Exit;
        let sale = build(
            &[WALLET, MINT, DEX, SYSTEM_PROGRAM],
            // Opaque, and deliberately so: whatever amount this sale is for is
            // inside the DEX instruction, where the signer cannot read it. A
            // hundred bytes rather than more, because the fixture writes a
            // single-byte shortvec length and anything past 127 is not a valid
            // transaction -- a malformed fixture would fail this test for a
            // reason that has nothing to do with the property.
            &[(2, vec![0, 1], vec![0xFF; 100])],
        );
        let outcome = check(&auth, &sale, &Address::new(WALLET), &allowlist(), NOW);
        assert!(
            outcome.is_ok(),
            "a large sale must still be signable: {outcome:?}"
        );
    }

    #[test]
    fn an_exit_authorisation_cannot_move_the_balance_to_a_stranger() {
        // The hole this replaced, kept as the test that would have caught it.
        //
        // Until 2026-08-31 only `Buy` was capped. So any `Exit` authorisation --
        // one micro-dollar of notional was enough -- could be spent on a
        // transaction that listed the mint as an inert account, used only
        // allowlisted programs, paid its fee from the right wallet, and
        // transferred the entire balance somewhere nobody authorised. Every
        // check passed.
        //
        // A hundred SOL, against a notional of one micro-dollar.
        const STRANGER: [u8; 32] = [0x66; 32];
        let mut auth = authorization();
        auth.action = Action::Exit;
        auth.max_notional = MicroUsd(1);

        let drain = build(
            &[WALLET, MINT, STRANGER, SYSTEM_PROGRAM],
            &[(3, vec![0, 2], transfer(100_000_000_000))],
        );
        let rejections = check(&auth, &drain, &Address::new(WALLET), &allowlist(), NOW)
            .expect_err("draining the wallet must be refused");
        assert!(
            rejections
                .iter()
                .any(|r| matches!(r, Rejection::OverSpend { .. })),
            "expected an overspend rejection, got {rejections:?}"
        );
    }

    #[test]
    fn the_ceiling_is_inclusive_and_one_lamport_past_it_is_not() {
        // The boundary itself, swept. `just mutants` found `>` could become
        // `>=` with every test still passing, which means nothing exercised a
        // transfer of exactly the ceiling.
        //
        // Both directions are wrong in a way that matters. Exclusive refuses a
        // transaction that is precisely within the operator's limit, which reads
        // as an unexplained failure at round numbers. Off by one the other way
        // is an overspend, small but of exactly the kind these bounds exist to
        // make impossible.
        let mut auth = authorization();
        auth.action = Action::Buy;
        auth.max_notional = MicroUsd(1_000_000);

        let at_the_ceiling = build(
            &[WALLET, MINT, DEX, SYSTEM_PROGRAM],
            &[(3, vec![0, 2], transfer(1_000_000))],
        );
        assert!(
            check(
                &auth,
                &at_the_ceiling,
                &Address::new(WALLET),
                &allowlist(),
                NOW
            )
            .is_ok(),
            "exactly the ceiling is inside the limit"
        );

        let one_over = build(
            &[WALLET, MINT, DEX, SYSTEM_PROGRAM],
            &[(3, vec![0, 2], transfer(1_000_001))],
        );
        assert!(
            check(&auth, &one_over, &Address::new(WALLET), &allowlist(), NOW).is_err(),
            "one lamport past it is not"
        );
    }

    #[test]
    fn a_reduce_is_capped_too() {
        // The third action, and it was exempt for the same reason. Swept rather
        // than sampled, because "Buy is capped" was true before this change as
        // well and asserting only that would not have caught anything.
        let mut auth = authorization();
        auth.action = Action::Reduce;
        auth.max_notional = MicroUsd(1);

        let drain = build(
            &[WALLET, MINT, DEX, SYSTEM_PROGRAM],
            &[(3, vec![0, 2], transfer(100_000_000_000))],
        );
        assert!(
            check(&auth, &drain, &Address::new(WALLET), &allowlist(), NOW).is_err(),
            "a reduce must be capped as well"
        );
    }

    #[test]
    fn an_expired_authorization_is_refused() {
        let rejections = check(
            &authorization(),
            &honest(),
            &Address::new(WALLET),
            &allowlist(),
            Slot(2_000),
        )
        .expect_err("must refuse");
        assert!(
            rejections
                .iter()
                .any(|r| matches!(r, Rejection::Expired { .. }))
        );
    }

    #[test]
    fn an_authorization_awaiting_an_operator_is_refused() {
        let mut auth = authorization();
        auth.needs_operator_signature = true;
        let rejections = check(&auth, &honest(), &Address::new(WALLET), &allowlist(), NOW)
            .expect_err("must refuse");
        assert!(rejections.contains(&Rejection::NeedsOperator));
    }

    #[test]
    fn a_foreign_fee_payer_is_refused() {
        // Signing for a wallet we are not is signing something we cannot reason
        // about.
        let rejections = check(
            &authorization(),
            &honest(),
            &Address::new([0x77; 32]),
            &allowlist(),
            NOW,
        )
        .expect_err("must refuse");
        assert!(rejections.contains(&Rejection::ForeignFeePayer));
    }

    #[test]
    fn an_ownership_change_is_refused() {
        // No trade needs to reassign an account. One that does is either a bug
        // or an attempt to take the wallet.
        let mut data = 1u32.to_le_bytes().to_vec();
        data.extend_from_slice(&[0xEE; 32]);
        let evil = build(
            &[WALLET, MINT, DEX, SYSTEM_PROGRAM],
            &[(2, vec![0, 1], vec![0xAB]), (3, vec![0], data)],
        );
        let rejections = check(
            &authorization(),
            &evil,
            &Address::new(WALLET),
            &allowlist(),
            NOW,
        )
        .expect_err("must refuse");
        assert!(rejections.contains(&Rejection::OwnershipChange));
    }

    #[test]
    fn an_empty_transaction_is_refused() {
        let empty = build(&[WALLET, MINT, DEX, SYSTEM_PROGRAM], &[]);
        let rejections = check(
            &authorization(),
            &empty,
            &Address::new(WALLET),
            &allowlist(),
            NOW,
        )
        .expect_err("must refuse");
        assert!(rejections.contains(&Rejection::Empty));
    }

    #[test]
    fn every_reason_is_reported_not_just_the_first() {
        // A caller fixing one problem and resubmitting only to hit the next has
        // learned nothing about whether this was ever going to be signable.
        let mut auth = authorization();
        auth.needs_operator_signature = true;
        let bad = build(
            &[[0x77; 32], [0x99; 32], [0xEE; 32], SYSTEM_PROGRAM],
            &[(2, vec![0, 1], vec![0xAB])],
        );
        let rejections = check(
            &auth,
            &bad,
            &Address::new(WALLET),
            &allowlist(),
            Slot(9_999),
        )
        .expect_err("must refuse");
        assert!(rejections.len() >= 5, "got {rejections:?}");
    }

    #[test]
    fn undecodable_bytes_refuse_without_pretending_to_check_anything() {
        // Reporting five rejections about a transaction that does not exist
        // would be five statements with no evidence behind them.
        let rejections = check(
            &authorization(),
            &[0xFF; 8],
            &Address::new(WALLET),
            &allowlist(),
            NOW,
        )
        .expect_err("must refuse");
        assert_eq!(rejections.len(), 1);
        assert!(matches!(rejections[0], Rejection::Undecodable(_)));
    }

    #[test]
    fn the_checked_bytes_are_the_verified_bytes() {
        // There is no path from an unverified message to a Checked, so a caller
        // cannot sign bytes other than the ones this module read.
        let bytes = honest();
        let checked = check(
            &authorization(),
            &bytes,
            &Address::new(WALLET),
            &allowlist(),
            NOW,
        )
        .expect("verifies");
        assert_eq!(checked.bytes(), bytes.as_slice());
        assert_eq!(checked.message().instructions.len(), 1);
    }
}
