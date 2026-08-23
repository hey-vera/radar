// SPDX-License-Identifier: Apache-2.0
//! Execution: route, gate, sign, submit, reconcile.
//!
//! The last stage, and the one holding the least authority. By the time control
//! reaches here the kernel has already decided the trade is permitted and bounded
//! it; this crate's remaining jobs are to build a transaction that fits inside
//! those bounds, to check the trade still pays for itself after costs, and to
//! find out what actually happened.
//!
//! It cannot sign. The key is in another process, reached over a pipe, and that
//! process re-decodes whatever this one built. So a compromised executor can
//! waste fees and produce refusals — it cannot move funds outside an
//! authorization the kernel issued.
//!
//! ```text
//!   Authorization ──▶ route ──▶ economics gate ──▶ signer ──▶ submit ──▶ status
//!        (kernel)      (here)       (here)        (separate)   (here)
//! ```
//!
//! The economics gate sits *after* routing because it needs the route's measured
//! impact, and *before* signing because a trade that does not pay for itself
//! should never reach the process that holds the key.

pub mod economics;
pub mod pipeline;
pub mod route;
pub mod signer_client;
pub mod submit;

pub use economics::{Costs, Economics, FailureRisk};
pub use pipeline::{Attempt, Outcome, execute};
pub use route::{Route, RouteError, Router};
pub use signer_client::StreamSigner;
pub use submit::{Finality, SubmitError, Submitter};
