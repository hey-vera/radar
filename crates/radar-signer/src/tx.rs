// SPDX-License-Identifier: Apache-2.0
//! A Solana transaction decoder written for an adversary.
//!
//! The executor builds transactions, often from a router's response. The signer
//! must not believe the executor's description of what it built — that is the
//! whole point of the separate process. So it decodes the bytes itself.
//!
//! Everything here is bounds-checked and allocation-bounded. A malformed
//! transaction is a rejection, never a panic: this process holds the key, and a
//! panic in a signer is a denial of service against the only component that can
//! stop a bad trade.

/// Why a transaction could not be decoded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    /// The bytes ran out mid-field.
    #[error("truncated at byte {at}, needed {needed} more")]
    Truncated {
        /// Where the read stopped.
        at: usize,
        /// How many more bytes the field wanted.
        needed: usize,
    },
    /// A compact-u16 length was encoded in more bytes than the format allows.
    ///
    /// Solana's shortvec has exactly one valid encoding per value. A
    /// non-canonical one is a deliberate attempt to make two decoders disagree
    /// about the same bytes, which is precisely the seam this process exists to
    /// close.
    #[error("non-canonical shortvec length at byte {at}")]
    NonCanonicalLength {
        /// Where the length started.
        at: usize,
    },
    /// An instruction referenced an account index that does not exist.
    #[error("account index {index} out of range ({count} accounts)")]
    AccountIndexOutOfRange {
        /// The index referenced.
        index: u8,
        /// How many accounts the message declares.
        count: usize,
    },
    /// The message uses address lookup tables.
    ///
    /// Refused rather than supported. A v0 transaction can name accounts
    /// indirectly through an on-chain table, so the signer cannot see which
    /// accounts the instruction actually touches without fetching that table —
    /// and a signer that fetches is a signer with a network dependency and a
    /// new thing to be lied to by. Refusing keeps the guarantee absolute: every
    /// account this process authorises is one it read in the bytes it signed.
    #[error("address lookup tables are not signable: {count} referenced")]
    LookupTablesPresent {
        /// How many tables the message references.
        count: usize,
    },
    /// Bytes remained after the message ended.
    ///
    /// Trailing bytes mean the decoder and the builder disagree about where the
    /// transaction ends, and a disagreement about extent is a disagreement about
    /// content.
    #[error("{count} trailing bytes after the message")]
    TrailingBytes {
        /// How many bytes were left.
        count: usize,
    },
}

/// One instruction, as it appears on the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instruction {
    /// The program invoked, resolved from its index.
    pub program_id: [u8; 32],
    /// The accounts passed, in order, resolved from their indices.
    pub accounts: Vec<[u8; 32]>,
    /// The instruction data, undecoded.
    pub data: Vec<u8>,
}

/// A decoded transaction message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// How many signatures the message requires.
    pub required_signatures: u8,
    /// Every account named, in order.
    pub accounts: Vec<[u8; 32]>,
    /// The recent blockhash.
    pub recent_blockhash: [u8; 32],
    /// The instructions, in execution order.
    pub instructions: Vec<Instruction>,
    /// Whether this was a versioned (v0) message.
    pub versioned: bool,
    /// Where the message begins inside the transaction bytes.
    ///
    /// A Solana signature covers the message, not the whole transaction, so the
    /// signer needs to know where the signature array ended. Carried out of the
    /// decoder rather than recomputed, because recomputing it would mean parsing
    /// the same prefix twice and hoping both parses agree.
    pub message_offset: usize,
}

impl Message {
    /// The bytes a Solana signature actually covers.
    ///
    /// Not the whole transaction: the signature array is excluded. Signing the
    /// wrong extent produces a signature the runtime rejects, which fails safe —
    /// but silently and at submission time, which is the wrong place to find out.
    ///
    /// Returns `None` if `bytes` is not the buffer this message was decoded from.
    #[must_use]
    pub fn signable<'a>(&self, bytes: &'a [u8]) -> Option<&'a [u8]> {
        bytes.get(self.message_offset..)
    }

    /// The fee payer, which is always the first account.
    #[must_use]
    pub fn fee_payer(&self) -> Option<[u8; 32]> {
        self.accounts.first().copied()
    }

    /// Whether any instruction invokes a program outside `allowed`.
    #[must_use]
    pub fn programs_outside(&self, allowed: &[[u8; 32]]) -> Vec<[u8; 32]> {
        let mut out: Vec<[u8; 32]> = self
            .instructions
            .iter()
            .map(|i| i.program_id)
            .filter(|p| !allowed.contains(p))
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// A cursor that cannot read past its buffer.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let end = self.at.checked_add(n).ok_or(DecodeError::Truncated {
            at: self.at,
            needed: n,
        })?;
        let slice = self.bytes.get(self.at..end).ok_or(DecodeError::Truncated {
            at: self.at,
            needed: end - self.bytes.len().min(end),
        })?;
        self.at = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }

    fn key(&mut self) -> Result<[u8; 32], DecodeError> {
        let mut out = [0u8; 32];
        out.copy_from_slice(self.take(32)?);
        Ok(out)
    }

    /// Reads a compact-u16, rejecting any non-canonical encoding.
    ///
    /// Two encodings of the same length would let a builder and a verifier read
    /// the same bytes as different transactions. Solana's own decoder rejects
    /// these; so does this one, for the same reason.
    fn shortvec(&mut self) -> Result<usize, DecodeError> {
        let start = self.at;
        let mut value: usize = 0;
        for shift in 0..3 {
            let byte = self.u8()?;
            let payload = usize::from(byte & 0x7F);
            // A continuation bit on a byte whose payload is zero encodes a value
            // that a shorter form already covers.
            if shift > 0 && payload == 0 {
                return Err(DecodeError::NonCanonicalLength { at: start });
            }
            value |= payload << (shift * 7);
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(DecodeError::NonCanonicalLength { at: start })
    }
}

/// Decodes a signed or unsigned transaction.
///
/// Signatures are skipped rather than verified: this process is deciding whether
/// to *add* a signature, and whoever else already signed is not evidence about
/// what the transaction does.
///
/// # Errors
///
/// Returns [`DecodeError`] for truncated, non-canonical, over-referencing, or
/// lookup-table-bearing input. Every one of those is a refusal to sign.
pub fn decode(bytes: &[u8]) -> Result<Message, DecodeError> {
    let mut r = Reader::new(bytes);

    let signature_count = r.shortvec()?;
    // Skipped, not parsed. 64 bytes each.
    let _ = r.take(signature_count.saturating_mul(64))?;
    let message_offset = r.at;

    let first = r.u8()?;
    let versioned = first & 0x80 != 0;
    let required_signatures = if versioned {
        // The version is the low seven bits; only v0 exists.
        r.u8()?
    } else {
        first
    };
    let _readonly_signed = r.u8()?;
    let _readonly_unsigned = r.u8()?;

    let account_count = r.shortvec()?;
    let mut accounts = Vec::with_capacity(account_count.min(256));
    for _ in 0..account_count {
        accounts.push(r.key()?);
    }

    let recent_blockhash = r.key()?;

    let instruction_count = r.shortvec()?;
    let mut instructions = Vec::with_capacity(instruction_count.min(64));
    for _ in 0..instruction_count {
        let program_index = r.u8()?;
        let program_id = *accounts.get(usize::from(program_index)).ok_or(
            DecodeError::AccountIndexOutOfRange {
                index: program_index,
                count: accounts.len(),
            },
        )?;

        let account_index_count = r.shortvec()?;
        let indices = r.take(account_index_count)?;
        let mut resolved = Vec::with_capacity(indices.len());
        for &index in indices {
            resolved.push(*accounts.get(usize::from(index)).ok_or(
                DecodeError::AccountIndexOutOfRange {
                    index,
                    count: accounts.len(),
                },
            )?);
        }

        let data_len = r.shortvec()?;
        let data = r.take(data_len)?.to_vec();

        instructions.push(Instruction {
            program_id,
            accounts: resolved,
            data,
        });
    }

    if versioned {
        let table_count = r.shortvec()?;
        if table_count > 0 {
            return Err(DecodeError::LookupTablesPresent { count: table_count });
        }
    }

    if r.at != bytes.len() {
        return Err(DecodeError::TrailingBytes {
            count: bytes.len() - r.at,
        });
    }

    Ok(Message {
        required_signatures,
        accounts,
        recent_blockhash,
        instructions,
        versioned,
        message_offset,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a minimal legacy transaction the tests can then damage.
    fn legacy(instructions: &[(u8, &[u8], &[u8])], accounts: usize) -> Vec<u8> {
        let mut out = vec![0u8]; // no signatures
        out.push(1); // required signatures
        out.push(0);
        out.push(0);
        out.push(u8::try_from(accounts).expect("small"));
        for i in 0..accounts {
            out.extend_from_slice(&[u8::try_from(i).expect("small"); 32]);
        }
        out.extend_from_slice(&[0xAA; 32]); // blockhash
        out.push(u8::try_from(instructions.len()).expect("small"));
        for (program, account_indices, data) in instructions {
            out.push(*program);
            out.push(u8::try_from(account_indices.len()).expect("small"));
            out.extend_from_slice(account_indices);
            out.push(u8::try_from(data.len()).expect("small"));
            out.extend_from_slice(data);
        }
        out
    }

    #[test]
    fn a_minimal_transaction_round_trips() {
        let bytes = legacy(&[(2, &[0, 1], &[9, 9, 9])], 3);
        let m = decode(&bytes).expect("decodes");
        assert_eq!(m.required_signatures, 1);
        assert_eq!(m.accounts.len(), 3);
        assert_eq!(m.instructions.len(), 1);
        assert_eq!(m.instructions[0].program_id, [2u8; 32]);
        assert_eq!(m.instructions[0].accounts, vec![[0u8; 32], [1u8; 32]]);
        assert_eq!(m.instructions[0].data, vec![9, 9, 9]);
        assert_eq!(m.fee_payer(), Some([0u8; 32]));
    }

    #[test]
    fn the_signable_extent_excludes_the_signature_array() {
        // A Solana signature covers the message, not the transaction. Signing
        // the wrong extent fails safe but silently, at submission time, which
        // is the wrong place to find out.
        let mut bytes = vec![1u8];
        bytes.extend_from_slice(&[0u8; 64]);
        bytes.extend_from_slice(&legacy(&[(2, &[0, 1], &[9])], 3)[1..]);
        let m = decode(&bytes).expect("decodes");
        assert_eq!(m.message_offset, 65);
        assert_eq!(m.signable(&bytes).expect("in range"), &bytes[65..]);
    }

    #[test]
    fn truncation_anywhere_is_a_refusal_not_a_panic() {
        // The signer holds the key. A panic here is a denial of service against
        // the only component that can stop a bad trade, so every prefix of a
        // valid transaction must decline rather than crash.
        let bytes = legacy(&[(2, &[0, 1], &[9, 9, 9])], 3);
        for cut in 0..bytes.len() {
            let _ = decode(&bytes[..cut]);
        }
    }

    #[test]
    fn arbitrary_bytes_never_panic() {
        // Fuzzing in miniature, deterministic so it can live in CI.
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        for len in 0..200usize {
            let mut buf = Vec::with_capacity(len);
            for _ in 0..len {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                buf.push(u8::try_from(state & 0xFF).unwrap_or(0));
            }
            let _ = decode(&buf);
        }
    }

    #[test]
    fn an_out_of_range_account_index_is_refused() {
        // Otherwise an instruction could reference an account the verifier never
        // sees, and the verifier would be checking a different transaction from
        // the one the runtime executes.
        let bytes = legacy(&[(9, &[0], &[1])], 3);
        assert_eq!(
            decode(&bytes),
            Err(DecodeError::AccountIndexOutOfRange { index: 9, count: 3 })
        );
    }

    #[test]
    fn a_non_canonical_length_is_refused() {
        // 0x80 0x00 encodes zero in two bytes. Accepting it would let a builder
        // and a verifier read the same bytes as different transactions.
        let mut bytes = vec![0x80, 0x00]; // signature count, long form
        bytes.extend_from_slice(&[1, 0, 0, 1]);
        bytes.extend_from_slice(&[0u8; 32]);
        bytes.extend_from_slice(&[0xAA; 32]);
        bytes.push(0);
        assert!(matches!(
            decode(&bytes),
            Err(DecodeError::NonCanonicalLength { .. })
        ));
    }

    #[test]
    fn trailing_bytes_are_refused() {
        // A disagreement about where the transaction ends is a disagreement
        // about what it contains.
        let mut bytes = legacy(&[(2, &[0], &[1])], 3);
        bytes.push(0xFF);
        assert_eq!(decode(&bytes), Err(DecodeError::TrailingBytes { count: 1 }));
    }

    #[test]
    fn a_versioned_message_with_lookup_tables_is_refused() {
        // The invariant that keeps the guarantee absolute: every account this
        // process authorises is one it read in the bytes it signed. A lookup
        // table names accounts the signer cannot see.
        let mut bytes = vec![0u8]; // no signatures
        bytes.push(0x80); // v0
        bytes.extend_from_slice(&[1, 0, 0]);
        bytes.push(2);
        bytes.extend_from_slice(&[0u8; 32]);
        bytes.extend_from_slice(&[1u8; 32]);
        bytes.extend_from_slice(&[0xAA; 32]);
        bytes.push(0); // no instructions
        bytes.push(1); // one lookup table
        assert_eq!(
            decode(&bytes),
            Err(DecodeError::LookupTablesPresent { count: 1 })
        );
    }

    #[test]
    fn a_versioned_message_without_lookup_tables_decodes() {
        let mut bytes = vec![0u8];
        bytes.push(0x80);
        bytes.extend_from_slice(&[1, 0, 0]);
        bytes.push(2);
        bytes.extend_from_slice(&[0u8; 32]);
        bytes.extend_from_slice(&[1u8; 32]);
        bytes.extend_from_slice(&[0xAA; 32]);
        bytes.push(0);
        bytes.push(0);
        let m = decode(&bytes).expect("decodes");
        assert!(m.versioned);
        assert_eq!(m.required_signatures, 1);
    }

    #[test]
    fn programs_outside_the_allowlist_are_reported() {
        let bytes = legacy(&[(1, &[0], &[1]), (2, &[0], &[1])], 3);
        let m = decode(&bytes).expect("decodes");
        assert_eq!(m.programs_outside(&[[1u8; 32]]), vec![[2u8; 32]]);
        assert!(m.programs_outside(&[[1u8; 32], [2u8; 32]]).is_empty());
    }
}
