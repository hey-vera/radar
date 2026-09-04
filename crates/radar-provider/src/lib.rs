// SPDX-License-Identifier: Apache-2.0
//! The spend meter: the thing standing between a bug and an unbounded bill.
//!
//! Radar buys most of its data per call, so every read is a purchase decision.
//! This crate makes that decision, and it makes it as **pure policy**: no HTTP,
//! no clock, no async. Instants and accounting days are arguments, and the
//! transport is the caller's problem.
//!
//! That shape is deliberate. A component authorised to spend real money has to
//! be exhaustively testable without a network, and its refusals have to be
//! reproducible from a recording.
//!
//! # What it holds, and what it used to hold
//!
//! [`Meter`], [`Budget`], [`Ledger`] and [`Commitment`] run in production:
//! `radar-agent` reserves before every model call, and `radar-serve`'s ledger
//! persists the reservation across a restart. That is AGENTS.md rule 8 enforced
//! in the running system rather than described.
//!
//! Until 2026-09-04 this crate also carried a cache, a circuit breaker and a
//! planner that composed them — about 1,300 lines against the 484 that run.
//! **They were deleted, and it is worth knowing why rather than looking for
//! them in the history by accident.** They had no caller anywhere outside this
//! crate and had had none since the crate was written: three separate documents
//! flagged it, AGENTS.md section 5 names the pattern as one this project has
//! produced three times, and LEARNINGS 1 and 9 are the same shape. A layer
//! nothing calls is not a design, it is a document that compiles.
//!
//! The work they were built for is real and is not lost. `radar-serve` has a
//! live cache with a key type parameter, which is the same idea with LEARNINGS
//! 27 already learned from it, and that is the one to extend. The retry policy
//! the breaker generalised belongs next to the client that needs it, where its
//! backoff can be read beside the endpoint it protects.
//!
//! One consequence is recorded because it is easy to miss: this crate's cache
//! was the only caller of [`radar_asof::Observed`] and `LookAhead` outside
//! `radar-asof`'s own tests, so those two types now have none. AGENTS.md rule 3
//! cited that cache as the place the watermark gate lived in the type system,
//! and it no longer does -- the rule changed in the same commit rather than
//! being left describing a deleted module.

#![forbid(unsafe_code)]

mod cost;

pub use cost::{Budget, Commitment, Ledger, Meter, Refusal};
