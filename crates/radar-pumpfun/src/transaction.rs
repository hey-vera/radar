// SPDX-License-Identifier: Apache-2.0
//! Assembling a legacy transaction the signer can read.
//!
//! [ADR 0003](https://github.com/hey-vera/radar/blob/main/docs/adr/0003-legacy-transactions-because-the-signer-must-be-able-to-read-them.md)
//! is the constraint this file exists to satisfy: `radar-signer` re-decodes what
//! it signs and refuses address lookup tables, because an account it cannot read
//! is an account it cannot check. So the message built here is **legacy** —
//! every account named in full, nothing resolved by lookup.
//!
//! # Bytes, not a client
//!
//! No blockhash is fetched and no key is held. The caller supplies the
//! blockhash; this returns bytes. That keeps the crate pure and, more
//! importantly, keeps the thing that *builds* a transaction separate from the
//! thing that *signs* one — which is rule 1's shape.
//!
//! # Ordering is a correctness property, not a formatting one
//!
//! Solana requires accounts sorted into four groups: writable signers, readonly
//! signers, writable non-signers, readonly non-signers, with the fee payer
//! first. That is a wire requirement. What is *not* obvious is that the order
//! **inside an instruction's own account list** is separately load-bearing:
//! pump.fun checks the position of its two trailing accounts and rejects a
//! transaction that carries both in the wrong order with a different error than
//! one that omits them
//! ([research 0023](https://github.com/hey-vera/radar/blob/main/docs/research/0023-the-fee-is-a-schedule-and-the-published-interface-is-incomplete.md)).
//!
//! So an instruction's accounts are held in the order the caller gave them, and
//! only the *message-level* key table is sorted. Conflating those two orders
//! would build a transaction that is well formed and means something else.

use radar_types::Address;

/// One account as an instruction refers to it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AccountMeta {
    /// The account.
    pub pubkey: Address,
    /// Whether it must sign.
    pub signer: bool,
    /// Whether the instruction may write to it.
    pub writable: bool,
}

impl AccountMeta {
    /// A read-only, non-signing account.
    #[must_use]
    pub const fn readonly(pubkey: Address) -> Self {
        Self {
            pubkey,
            signer: false,
            writable: false,
        }
    }

    /// A writable, non-signing account.
    #[must_use]
    pub const fn writable(pubkey: Address) -> Self {
        Self {
            pubkey,
            signer: false,
            writable: true,
        }
    }

    /// The signer, which on these instructions is also the payer.
    #[must_use]
    pub const fn signer(pubkey: Address) -> Self {
        Self {
            pubkey,
            signer: true,
            writable: true,
        }
    }
}

/// One instruction to include in a message.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Instruction {
    /// The program to invoke.
    pub program_id: Address,
    /// The accounts, **in the order the program expects them**.
    pub accounts: Vec<AccountMeta>,
    /// The instruction data.
    pub data: Vec<u8>,
}

/// Why a message could not be built.
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum Unbuildable {
    /// More than 255 distinct accounts.
    ///
    /// Account indices are one byte on the wire, so this is a hard ceiling
    /// rather than a policy. Refused rather than truncated: a truncated account
    /// table builds a transaction that names the wrong accounts.
    #[error("{count} accounts, and an index is one byte")]
    TooManyAccounts {
        /// How many distinct accounts were named.
        count: usize,
    },
    /// No instructions.
    #[error("a message with no instructions does nothing")]
    Empty,
}

/// The most bytes a compact-u16 can occupy.
///
/// The format is defined over a `u16` at seven bits a byte, so three is the
/// whole range: `16_383` is the largest two-byte value and `65_535` the largest
/// encodable one.
const COMPACT_U16_MAX_BYTES: usize = 3;

/// Solana's compact-u16 length prefix.
///
/// # The loop is bounded, and that is not a style choice
///
/// This was `loop { .. }`, and mutation testing turned `remaining == 0` into
/// `remaining != 0`. For a value of zero that never terminates -- and every
/// iteration pushes a byte, so it is an unbounded *allocation*, not a spin. It
/// took a CI runner out of memory and killed the whole job three minutes in,
/// before `cargo mutants`' own 300s per-mutant timeout could fire, which
/// presented as one shard being mysteriously cancelled on every run while the
/// other three passed.
///
/// A tool cannot time out a mutant that kills the machine it is running on. So
/// the bound is the fix: with it, that mutation produces a wrong prefix instead
/// of an OOM, and `compact_u16_matches_solanas_encoding` catches it.
///
/// # Panics
///
/// Debug builds only, when `value` does not fit the format. Every call site
/// passes a count the message builder has already bounded -- accounts at 255 by
/// [`Unbuildable::TooManyAccounts`], and the rest by what a transaction can
/// hold -- so a value past `u16::MAX` here means a caller changed, not that
/// input got large.
fn compact_u16(value: usize, out: &mut Vec<u8>) {
    let mut remaining = value;
    for _ in 0..COMPACT_U16_MAX_BYTES {
        let byte = u8::try_from(remaining & 0x7f).unwrap_or(0);
        remaining >>= 7;
        if remaining == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
    debug_assert!(
        remaining == 0,
        "{value} does not fit a compact-u16; the prefix written is truncated"
    );
}

/// Builds the legacy message bytes for `instructions`, paid for by `payer`.
///
/// The returned bytes are the **message**, which is exactly what a Solana
/// signature covers. Prepending the signature array is
/// [`transaction`](self::transaction)'s job.
///
/// # Errors
///
/// [`Unbuildable`] when there are no instructions, or more accounts than a
/// one-byte index can address.
///
/// # Panics
///
/// Cannot. The two `expect`s below are on invariants established earlier in the
/// same function: every key looked up was collected into `keys` a few lines
/// above, and the count was bounded against `u8::MAX` before any index is cast.
/// They are `expect` rather than `?` because a fallible conversion from a value
/// already proven in range has no error worth propagating.
pub fn message(
    payer: &Address,
    instructions: &[Instruction],
    recent_blockhash: &[u8; 32],
) -> Result<Vec<u8>, Unbuildable> {
    if instructions.is_empty() {
        return Err(Unbuildable::Empty);
    }

    // Collect each account once, keeping the strongest privilege any instruction
    // asked for. Taking the *last* mention rather than the union would silently
    // drop a writable flag, and the transaction would fail at execution with an
    // error about permissions rather than about the bug.
    let mut seen: Vec<(Address, bool, bool)> = vec![(*payer, true, true)];
    let mut note = |meta: &AccountMeta| {
        if let Some(entry) = seen.iter_mut().find(|(k, _, _)| *k == meta.pubkey) {
            entry.1 = entry.1 || meta.signer;
            entry.2 = entry.2 || meta.writable;
        } else {
            seen.push((meta.pubkey, meta.signer, meta.writable));
        }
    };
    for instruction in instructions {
        for meta in &instruction.accounts {
            note(meta);
        }
        // A program is named as a readonly, non-signing account.
        note(&AccountMeta::readonly(instruction.program_id));
    }

    // The fee payer stays first; everything else sorts into the four groups the
    // runtime requires.
    let (payer_entry, rest) = seen.split_first().expect("the payer was pushed first");
    let group = |signer: bool, writable: bool| {
        rest.iter()
            .filter(move |(_, s, w)| *s == signer && *w == writable)
            .map(|(k, _, _)| *k)
    };
    let mut keys = vec![payer_entry.0];
    keys.extend(group(true, true));
    let readonly_signed = group(true, false).count();
    keys.extend(group(true, false));
    keys.extend(group(false, true));
    let readonly_unsigned = group(false, false).count();
    keys.extend(group(false, false));

    let count = keys.len();
    if count > usize::from(u8::MAX) {
        return Err(Unbuildable::TooManyAccounts { count });
    }
    let required_signatures = 1 + rest.iter().filter(|(_, s, _)| *s).count();

    let index_of = |key: &Address| {
        u8::try_from(
            keys.iter()
                .position(|k| k == key)
                .expect("every key was collected above"),
        )
        .expect("the count was bounded above")
    };

    let mut out = Vec::with_capacity(64 + count * 32);
    out.push(u8::try_from(required_signatures).unwrap_or(u8::MAX));
    out.push(u8::try_from(readonly_signed).unwrap_or(u8::MAX));
    out.push(u8::try_from(readonly_unsigned).unwrap_or(u8::MAX));
    compact_u16(count, &mut out);
    for key in &keys {
        out.extend_from_slice(key.as_bytes());
    }
    out.extend_from_slice(recent_blockhash);
    compact_u16(instructions.len(), &mut out);
    for instruction in instructions {
        out.push(index_of(&instruction.program_id));
        compact_u16(instruction.accounts.len(), &mut out);
        for meta in &instruction.accounts {
            // The caller's order, deliberately. See the module docs.
            out.push(index_of(&meta.pubkey));
        }
        compact_u16(instruction.data.len(), &mut out);
        out.extend_from_slice(&instruction.data);
    }
    Ok(out)
}

/// The full transaction bytes, with an **unfilled** signature array.
///
/// The signatures are zeroed. That is what a transaction looks like before it
/// reaches the signer, and it is the form `radar_signer::verify::check` is meant
/// to be handed: the check runs over bytes nobody has signed yet, so a refusal
/// happens before a signature exists rather than after.
///
/// # Errors
///
/// As [`message`].
pub fn transaction(
    payer: &Address,
    instructions: &[Instruction],
    recent_blockhash: &[u8; 32],
) -> Result<Vec<u8>, Unbuildable> {
    let body = message(payer, instructions, recent_blockhash)?;
    let signatures = 1 + instructions
        .iter()
        .flat_map(|i| &i.accounts)
        .filter(|m| m.signer && m.pubkey != *payer)
        .map(|m| m.pubkey)
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let mut out = Vec::with_capacity(1 + signatures * 64 + body.len());
    compact_u16(signatures, &mut out);
    out.extend(std::iter::repeat_n(0u8, signatures * 64));
    out.extend_from_slice(&body);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(byte: u8) -> Address {
        Address::new([byte; 32])
    }

    fn ix(program: u8, accounts: Vec<AccountMeta>) -> Instruction {
        Instruction {
            program_id: addr(program),
            accounts,
            data: vec![1, 2, 3],
        }
    }

    #[test]
    fn the_fee_payer_is_the_first_account() {
        // Not a convention. `Message::fee_payer` reads index zero, and the
        // runtime charges whoever is there.
        let payer = addr(1);
        let bytes = message(
            &payer,
            &[ix(9, vec![AccountMeta::writable(addr(2))])],
            &[0u8; 32],
        )
        .expect("builds");
        assert_eq!(&bytes[3 + 1..3 + 1 + 32], payer.as_bytes());
    }

    #[test]
    fn an_instructions_own_account_order_is_preserved() {
        // The property research 0023 measured: pump.fun rejects a transaction
        // whose two trailing accounts are present but transposed, with a
        // *different* error than one that omits them. Sorting an instruction's
        // account list -- as the message-level key table is sorted -- would build
        // a well formed transaction that means something else.
        let payer = addr(1);
        let accounts = vec![
            AccountMeta::readonly(addr(200)),
            AccountMeta::readonly(addr(3)),
            AccountMeta::readonly(addr(100)),
        ];
        let bytes = message(&payer, &[ix(9, accounts)], &[0u8; 32]).expect("builds");
        // Walk to the instruction's account index list.
        let count = usize::from(bytes[3]);
        let at = 3 + 1 + count * 32 + 32 + 1 + 1 + 1;
        let indices = &bytes[at..at + 3];
        let keys_at = |i: u8| {
            let start = 3 + 1 + usize::from(i) * 32;
            bytes[start..start + 32].to_vec()
        };
        assert_eq!(keys_at(indices[0]), addr(200).as_bytes().to_vec());
        assert_eq!(keys_at(indices[1]), addr(3).as_bytes().to_vec());
        assert_eq!(keys_at(indices[2]), addr(100).as_bytes().to_vec());
    }

    #[test]
    fn an_account_named_twice_keeps_the_stronger_privilege() {
        // Taking the last mention would drop the writable flag, and the failure
        // would surface at execution as a permissions error rather than as the
        // bug it is.
        let payer = addr(1);
        let bytes = message(
            &payer,
            &[
                ix(9, vec![AccountMeta::readonly(addr(5))]),
                ix(9, vec![AccountMeta::writable(addr(5))]),
            ],
            &[0u8; 32],
        )
        .expect("builds");
        let count = usize::from(bytes[3]);
        let readonly_unsigned = usize::from(bytes[2]);
        // Account 5 must not be in the readonly tail.
        let tail: Vec<Vec<u8>> = (count - readonly_unsigned..count)
            .map(|i| bytes[3 + 1 + i * 32..3 + 1 + i * 32 + 32].to_vec())
            .collect();
        assert!(
            !tail.contains(&addr(5).as_bytes().to_vec()),
            "a writable account must not land in the readonly group"
        );
    }

    #[test]
    fn an_empty_message_is_refused() {
        assert_eq!(message(&addr(1), &[], &[0u8; 32]), Err(Unbuildable::Empty));
    }

    #[test]
    fn a_transaction_reserves_room_for_every_signature() {
        // One signature here: the payer. The extra signer in the second
        // instruction is the payer again, and counting it twice would reserve a
        // slot the runtime does not expect and shift the whole message.
        let payer = addr(1);
        let bytes = transaction(
            &payer,
            &[ix(9, vec![AccountMeta::signer(payer)])],
            &[0u8; 32],
        )
        .expect("builds");
        assert_eq!(bytes[0], 1);
        assert!(bytes[1..65].iter().all(|b| *b == 0), "unsigned");
    }

    #[test]
    fn compact_u16_matches_solanas_encoding() {
        let mut out = Vec::new();
        compact_u16(0, &mut out);
        assert_eq!(out, vec![0]);
        out.clear();
        compact_u16(127, &mut out);
        assert_eq!(out, vec![127]);
        out.clear();
        // The boundary. A single-byte encoding here would shift every
        // subsequent field, and 128 accounts is a reachable transaction.
        compact_u16(128, &mut out);
        assert_eq!(out, vec![0x80, 0x01]);
        out.clear();
        compact_u16(16_384, &mut out);
        assert_eq!(out, vec![0x80, 0x80, 0x01]);
        out.clear();
        // The top of the format's range, and the reason the loop's bound is
        // three rather than two. A bound one too small would truncate here
        // rather than looping, which is the failure this test exists to name.
        compact_u16(65_535, &mut out);
        assert_eq!(out, vec![0xff, 0xff, 0x03]);
    }

    #[test]
    fn the_prefix_is_never_longer_than_the_format_allows() {
        // The bound, asserted rather than trusted. An unbounded loop here is
        // an unbounded *allocation* -- every iteration pushes -- and it killed
        // a CI runner outright when a mutant made the exit condition never
        // fire. A tool cannot time out a mutant that takes the machine with it.
        for value in [0usize, 1, 127, 128, 16_383, 16_384, 65_535] {
            let mut out = Vec::new();
            compact_u16(value, &mut out);
            assert!(
                out.len() <= COMPACT_U16_MAX_BYTES,
                "{value} encoded to {} bytes",
                out.len()
            );
            assert!(!out.is_empty(), "{value} encoded to nothing");
        }
    }

    #[test]
    fn an_account_named_twice_keeps_the_signer_flag_too() {
        // The sibling of the writable case. `entry.1 = entry.1 || meta.signer`
        // and `entry.2 = entry.2 || meta.writable` are separate lines, and a
        // test covering only one leaves the other free.
        let payer = addr(1);
        let other = addr(5);
        let bytes = message(
            &payer,
            &[
                ix(9, vec![AccountMeta::readonly(other)]),
                ix(9, vec![AccountMeta::signer(other)]),
            ],
            &[0u8; 32],
        )
        .expect("builds");
        // Two signers now: the payer and `other`.
        assert_eq!(
            bytes[0], 2,
            "a signer seen first as a reader is still a signer"
        );
    }

    #[test]
    fn the_signature_count_is_one_per_distinct_signer() {
        // Guards the `1 +` that counts the payer. Under `-` this is zero and
        // under `*` it stays one, and both produce a message the runtime would
        // reject for a reason that says nothing about the bug.
        let payer = addr(1);
        let cosigner = addr(2);

        let alone = message(
            &payer,
            &[ix(9, vec![AccountMeta::writable(addr(3))])],
            &[0u8; 32],
        )
        .expect("builds");
        assert_eq!(alone[0], 1, "just the payer");

        let together = message(
            &payer,
            &[ix(9, vec![AccountMeta::signer(cosigner)])],
            &[0u8; 32],
        )
        .expect("builds");
        assert_eq!(together[0], 2, "the payer and one cosigner");

        // And the transaction reserves exactly that many 64-byte slots.
        let tx = transaction(
            &payer,
            &[ix(9, vec![AccountMeta::signer(cosigner)])],
            &[0u8; 32],
        )
        .expect("builds");
        assert_eq!(tx[0], 2);
        assert!(tx[1..129].iter().all(|b| *b == 0), "two empty signatures");
        // The message begins straight after them.
        assert_eq!(tx[129], 2, "the message's own signature count");
    }

    #[test]
    fn the_four_account_groups_are_partitioned_exactly() {
        // The header's two counts are derived from the same predicate that
        // orders the keys, so a filter comparing the wrong way round produces a
        // header that disagrees with the table it describes.
        let payer = addr(1);
        let bytes = message(
            &payer,
            &[ix(
                9,
                vec![
                    AccountMeta::signer(addr(2)),   // writable signer
                    AccountMeta::writable(addr(3)), // writable non-signer
                    AccountMeta::readonly(addr(4)), // readonly non-signer
                ],
            )],
            &[0u8; 32],
        )
        .expect("builds");

        let required = usize::from(bytes[0]);
        let readonly_signed = usize::from(bytes[1]);
        let readonly_unsigned = usize::from(bytes[2]);
        let count = usize::from(bytes[3]);

        assert_eq!(required, 2, "the payer and the writable signer");
        assert_eq!(readonly_signed, 0, "no readonly signers here");
        // addr(4) and the program are both readonly non-signers.
        assert_eq!(readonly_unsigned, 2);
        assert_eq!(count, 5, "payer, signer, writable, readonly, program");
        // Every account is in exactly one group, and the writable non-signers
        // are what is left over. Written as a subtraction rather than as
        // `a + (b - a) == b`, which the first version of this test asserted and
        // which is true of any three numbers.
        let writable_unsigned = count - required - readonly_unsigned;
        assert_eq!(writable_unsigned, 1, "just addr(3)");
    }

    #[test]
    fn the_account_ceiling_is_where_a_one_byte_index_runs_out() {
        // 255 accounts fit; 256 do not. A boundary written `>=` would refuse a
        // buildable transaction and one written `==` would let 256 through and
        // emit an index that wrapped -- which names the wrong account rather
        // than failing.
        let payer = addr(1);
        let metas = |n: u16| {
            (0..n)
                .map(|i| {
                    let mut bytes = [0u8; 32];
                    bytes[0..2].copy_from_slice(&i.to_le_bytes());
                    bytes[31] = 0x5A;
                    AccountMeta::readonly(Address::new(bytes))
                })
                .collect::<Vec<_>>()
        };
        // 253 distinct + payer + program = 255.
        let fits = message(&payer, &[ix(9, metas(253))], &[0u8; 32]);
        assert!(fits.is_ok(), "255 accounts must build");

        // One more tips it over.
        let over = message(&payer, &[ix(9, metas(254))], &[0u8; 32]);
        assert!(
            matches!(over, Err(Unbuildable::TooManyAccounts { count: 256 })),
            "256 accounts must be refused, got {over:?}"
        );
    }
}
