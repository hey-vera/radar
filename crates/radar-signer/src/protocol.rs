// SPDX-License-Identifier: Apache-2.0
//! The wire format between the executor and the signer.
//!
//! Newline-delimited JSON over stdin and stdout, because the transport is a
//! pipe from the parent process and nothing else. No socket, no port, no
//! listener: the signer's attack surface is one file descriptor held by one
//! process, and the operating system decides who that process is.
//!
//! That is a stronger property than an authenticated socket, and it is much
//! easier to check. `ss -tlnp` on the VPS should never list this binary.

use radar_risk::Authorization;
use radar_types::Slot;
use serde::{Deserialize, Serialize};

/// A request to sign.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Request {
    /// The authorization the kernel issued.
    pub authorization: Authorization,
    /// The unsigned transaction, base64.
    pub transaction: String,
    /// The caller's view of the chain head.
    ///
    /// Supplied rather than read, so the signer needs no RPC. A caller that
    /// lies here can make an expired authorization usable — it cannot make an
    /// unauthorised trade possible, because every bound is still checked
    /// against the bytes.
    pub now_slot: u64,
}

/// The answer.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Response {
    /// Signed, and here is the signature.
    Signed {
        /// The signature, base58.
        signature: String,
        /// The wallet it was made with, base58.
        wallet: String,
        /// The signed transaction, base64, ready to submit.
        transaction: String,
    },
    /// Refused, for these reasons.
    ///
    /// A refusal is a normal outcome, not an error. The signer refusing is the
    /// system working.
    Refused {
        /// Every applicable reason, in a fixed order.
        reasons: Vec<String>,
    },
}

impl Response {
    /// A refusal from a single message.
    #[must_use]
    pub fn refused(reason: impl Into<String>) -> Self {
        Self::Refused {
            reasons: vec![reason.into()],
        }
    }

    /// Whether this response carries a signature.
    #[must_use]
    pub const fn is_signed(&self) -> bool {
        matches!(self, Self::Signed { .. })
    }
}

/// The slot a request reports.
#[must_use]
pub const fn slot_of(request: &Request) -> Slot {
    Slot(request.now_slot)
}

/// Places a signature into a transaction's signature array.
///
/// The signer produces a complete, submittable transaction rather than a bare
/// signature, so no other component has to know the wire format well enough to
/// assemble one — and so a component that assembled it wrongly could not attach
/// this signature to different bytes.
///
/// Returns `None` if the buffer has no room for a signature at `index`, which
/// means the executor sent a transaction whose signature array is smaller than
/// its header requires.
#[must_use]
pub fn place_signature(
    bytes: &[u8],
    message_offset: usize,
    index: usize,
    signature: &[u8; 64],
) -> Option<Vec<u8>> {
    let start = 1usize.checked_add(index.checked_mul(64)?)?;
    let end = start.checked_add(64)?;
    if end > message_offset {
        return None;
    }
    let mut out = bytes.to_vec();
    out.get_mut(start..end)?.copy_from_slice(signature);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signature_lands_in_the_first_slot() {
        let mut bytes = vec![1u8];
        bytes.extend_from_slice(&[0u8; 64]);
        bytes.extend_from_slice(&[0xAB; 10]);

        let out = place_signature(&bytes, 65, 0, &[0x7F; 64]).expect("fits");
        assert_eq!(&out[1..65], &[0x7F; 64]);
        assert_eq!(&out[65..], &[0xAB; 10], "the message must not move");
        assert_eq!(out.len(), bytes.len());
    }

    #[test]
    fn a_signature_that_would_overwrite_the_message_is_refused() {
        // Writing past the signature array would change the bytes that were
        // verified, which is the one thing the whole process exists to prevent.
        let mut bytes = vec![1u8];
        bytes.extend_from_slice(&[0u8; 64]);
        bytes.extend_from_slice(&[0xAB; 10]);
        assert_eq!(place_signature(&bytes, 65, 1, &[0x7F; 64]), None);
    }

    #[test]
    fn an_unsigned_transaction_has_nowhere_to_put_one() {
        let bytes = vec![0u8, 1, 2, 3];
        assert_eq!(place_signature(&bytes, 1, 0, &[0x7F; 64]), None);
    }

    #[test]
    fn a_refusal_round_trips_as_an_outcome_not_an_error() {
        // The executor must be able to tell "refused" from "crashed", because
        // one is the system working and the other is not.
        let json = serde_json::to_string(&Response::refused("nope")).expect("serialises");
        assert!(json.contains("\"outcome\":\"refused\""));
        let back: Response = serde_json::from_str(&json).expect("round trips");
        assert!(!back.is_signed());
    }
}
