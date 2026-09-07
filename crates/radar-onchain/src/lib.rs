// SPDX-License-Identifier: Apache-2.0
//! Reading a token's launch block and curve from RPC, on demand.
//!
//! # Why this crate exists
//!
//! Radar's public analyst answers a question about a specific mint, in public,
//! while the thread it was asked in is still alive. Three facts about the store
//! say it cannot serve that, and each was measured rather than assumed:
//!
//! - **The store cannot be asked a question.** `Reader::read` opens every
//!   partition at or below the watermark, decodes every row and sorts the whole
//!   thing. There is no index, no predicate pushdown and no point lookup.
//!   Decoding 167,987 launch events to learn that there are 167,987 of them took
//!   ten seconds against the live store; it now holds 483,629.
//! - **The headline fact is not in the store at all.** The launch block's
//!   recipient count is computed from a live query at decision time and only the
//!   resulting *label* is persisted. The count is answerable from the store for
//!   7,543 mints out of 483,629 — and essentially never for the one somebody is
//!   asking about.
//! - **The store is behind on purpose.** The follow recorder runs with a
//!   five-minute lag against an endpoint Radar is a guest on, so it is roughly
//!   six to eleven minutes behind chain by design.
//!
//! The launch block is one Solana block and it is on chain the instant it
//! happens. Reading it directly is faster, fresher, and works for a mint that is
//! forty seconds old. **The store's job changes from lookup to base rates** —
//! those live in `docs/research/data/0024-base-rates.json`.
//!
//! # What this crate is not
//!
//! It does not trade, sign, hold a key, or have a function that takes one. It is
//! read-only end to end, and it touches neither the signer nor the execution
//! path. `Policy::CLOSED` is untouched by anything here.
//!
//! It also does not *phrase* anything. It produces a [`Dossier`] of facts;
//! deciding what matters, what the verdict is and how to say it is Phase 2's
//! job, and keeping the two apart is what makes it possible to check afterwards
//! that every number in a published sentence came from a measurement.
//!
//! # Its caller
//!
//! `radar dossier <mint>`, in `radar-cli`. AGENTS.md section 5 asks for a
//! layer's caller to be named before it is built, because this project has
//! produced three crates nothing depends on. The CLI subcommand ships in the
//! same change as this crate, and it is how the whole path is exercised without
//! any part of the X adapter existing.

#![forbid(unsafe_code)]

pub mod budget;
pub mod dossier;
pub mod launch;
pub mod rpc;

pub use budget::{Budget, Count, Exhausted};
pub use dossier::{CurveFacts, Dossier, Unavailable, build};
pub use launch::{LaunchBlock, Metadata, NotALaunch};
pub use rpc::{AccountRead, RpcClient, RpcError};
