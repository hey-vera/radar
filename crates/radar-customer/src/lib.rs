// SPDX-License-Identifier: Apache-2.0
//! Radar's customer model.
//!
//! Pure, like the risk kernel and for the same reason: no clock, no network, no
//! ambient state. Everything here is a function of its inputs, so a grant can be
//! replayed and a refusal reproduced from a recording.
//!
//! # What is deliberately absent
//!
//! There is **no `Account` type**, and that is
//! [ADR 0006](https://github.com/hey-vera/radar/blob/main/docs/adr/0006-radar-records-only-what-it-cannot-recover.md)
//! rather than an oversight. A customer's identity arrives in a verified token
//! on every request, their wallet address belongs to Privy, their grant is
//! enforced by Privy's policy engine, and their balance and fills are on the
//! chain. Five of the six pieces of a customer account are already held by
//! something more authoritative than a Radar-side copy would be, and a mirror
//! that diverges from an authority is wrong by construction.
//!
//! The sixth — how many signatures were made on a customer's behalf — is held
//! nowhere durable, and ADR 0005's precondition 5 turns on it: it decides whether
//! the provider's pricing stays acceptable, and it cannot be taken
//! retroactively. So it is the one thing this crate persists.

#![forbid(unsafe_code)]

mod grant;
mod signatures;
pub mod siws;

pub use grant::{Bounds, Grant, NotGranted};
pub use signatures::{Allowance, MIN_SALT_BYTES, Meter, Reading, Subject, SubjectError};
pub use siws::{Challenge, SignIn};
