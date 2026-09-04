// SPDX-License-Identifier: Apache-2.0
//! The summoned-reply loop.
//!
//! ```text
//!   mention ──▶ mention::read ──▶ Mint | Ticker | Nothing
//!                                   │      strict parse; everything else
//!                                   │      is discarded before a model sees it
//!                                   ▼
//!                            admission::Gate       per-summoner, global,
//!                                   │              per-mint dedupe; no config
//!                                   │              means nothing is answered
//!                                   ▼
//!               radar-onchain ──▶ radar-roast ──▶ reply + fact sheet
//!                                   │
//!                                   ▼
//!                            log::append           evidence, then
//!                                   │              publish::publish
//!                                   ▼
//!                            Publisher             DryRun by default:
//!                                                  holds no credential
//! ```
//!
//! # What is finished and what is not
//!
//! Finished: the parser, the gate, the log, and the publishing seam. All of it
//! is pure or file-backed, and all of it is tested without a network.
//!
//! Not finished: **the X client**, and the reason is a price rather than a
//! design. Two billing questions gate it — see [`publish`] — and AGENTS.md
//! section 2 says a decision turning on a price needs that price verified
//! first. ADR 0011 is the standing example of what happens otherwise. Both are
//! settleable in the Developer Console with one live test post and neither is
//! settleable by reading more.
//!
//! Nothing else in the loop depends on either answer, which is why everything
//! else is done.
//!
//! # Untrusted from the first byte
//!
//! A mention is written by a stranger. Rule 4 applies to all of it, and the
//! defence is that only two token shapes survive parsing — so there is no field
//! an instruction can travel in, rather than a filter that has to anticipate the
//! phrasing.

#![forbid(unsafe_code)]

pub mod admission;
pub mod answer;
pub mod daemon;
pub mod log;
pub mod mention;
pub mod poll;
pub mod publish;
pub mod spend;
pub mod x;

pub use admission::{Admitted, Gate, Limits, Refused};
pub use answer::{Answered, Answering, answer, describe};
pub use log::{Entry, latest};
pub use mention::{Asked, read};
pub use poll::{interval, next_cursor, read_cursor, write_cursor};
pub use publish::{DryRun, Publisher, Undeliverable};
pub use spend::{Cost, Prices, Spend};
pub use x::{Mention, Unreachable, X, backoff};

/// What to say when a mention named a symbol rather than a mint.
///
/// The honest answer, and the plan is right that it is also the best content:
/// a symbol is creator-chosen, unowned and endlessly duplicated, so resolving
/// one means guessing which project someone meant. Guessing wrong publishes
/// measurements about the wrong token under a name that promises accuracy.
///
/// It carries no count. Saying "there are 340 tokens with that symbol" would be
/// a number Radar has not measured, and this crate is downstream of a fidelity
/// check built precisely to stop that.
#[must_use]
pub fn ticker_reply(ticker: &str) -> String {
    format!(
        "${ticker} is a symbol, not a token -- anyone can mint one and many people have. \
         Give me the contract address and I will tell you what is in its launch block."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ticker_reply_states_no_number_it_has_not_measured() {
        let reply = ticker_reply("BONK");
        assert!(reply.contains("$BONK"));
        // No digits at all: the tempting version of this line quotes how many
        // tokens share the symbol, and Radar has not counted them.
        assert!(
            !reply.chars().any(|c| c.is_ascii_digit()),
            "the ticker reply must not carry an unmeasured count: {reply}"
        );
    }
}
