// SPDX-License-Identifier: Apache-2.0
//! The signer's verification core.
//!
//! Split from the binary on purpose. The rules that decide whether a
//! transaction may be signed are the most security-critical code in Radar, and
//! they are worth being able to test without a key, a socket or a process.
//!
//! The binary is deliberately thin: read a request, call [`verify::check`],
//! sign the verified bytes or refuse. Everything that decides anything is here.
//!
//! # What this defends against
//!
//! Not a bug in the executor — a *replaced* executor. Assume it can build any
//! transaction and describe it any way it likes. The two things it cannot do
//! are forge an [`radar_risk::Authorization`] the kernel never issued, and
//! change the bytes between this module reading them and the signature covering
//! them. Every check is against the decoded bytes; nothing the caller says
//! about a transaction is an input.

pub mod canonical;
pub mod key;
pub mod privy;
pub mod protocol;
pub mod turnkey;
pub mod tx;
pub mod verify;

pub use key::{Key, KeyError};
pub use tx::{DecodeError, Instruction, Message, decode};
pub use verify::{Allowlist, Checked, Rejection, check};
