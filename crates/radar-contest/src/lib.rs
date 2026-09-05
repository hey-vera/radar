// SPDX-License-Identifier: Apache-2.0
//! The weekly contest, as a rule that can be replayed.
//!
//! # What this is
//!
//! ADR 0013 constraints 3 and 4, and design 0007 section 6, as code: every
//! summoned reply is an entry, the week's entries are scored by a published
//! rule over the bot's **own** replies' public metrics, the winner is paid the
//! week's creator fee, and everything -- the rule, the scores, the winner, the
//! claim, the payout -- is written down where anyone can check it.
//!
//! # Why it is pure
//!
//! The same shape as `radar-risk`: no clock, no network, no key. The caller
//! supplies the time, the metrics and what it knows about each account, and
//! this crate returns the ranking. Given the same inputs it returns the same
//! winner on any machine at any time, which is what lets a contested result be
//! settled by re-running the rule over the recorded inputs rather than by
//! arguing about it. Design 0007 section 6.2 says gaming will be visible rather
//! than impossible; replayability is what makes it visible.
//!
//! # Its callers
//!
//! `radar-serve`'s public leaderboard reads the [`ledger::Record`] this crate
//! defines; the analyst's week-close job writes one; `radar-payout` asks
//! [`ledger::Payout::permitted`] before it signs anything. Named here because a
//! crate nothing depends on is a document that compiles (AGENTS.md section 5),
//! and the first of those callers lands in the same plan as this crate.
//!
//! # What it does not do
//!
//! It does not read X, does not hold a key, does not know the vault's balance,
//! and does not decide the size of the prize. It never touches `radar-risk`,
//! the signer, or `Policy::CLOSED`.

#![forbid(unsafe_code)]

pub mod hunter;
pub mod ledger;
pub mod score;
pub mod week;

pub use ledger::{Claim, Payout, Record, Refusal, Vault, Winner, records_in};
pub use score::{Entry, Excluded, Metrics, Ranked, Ranking, Rules, Standing, rank};
pub use week::Week;
