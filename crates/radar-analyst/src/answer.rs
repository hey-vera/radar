// SPDX-License-Identifier: Apache-2.0
//! Answering one mention, from a mint to a log entry.
//!
//! # Why this is here rather than in the command that had it
//!
//! `radar analyst --mentions <file>` had this pipeline inline, which was right
//! while it was the only caller. The daemon is a second one, and two copies of
//! *what the account says* is the arrangement where the version somebody reads
//! two hundred replies from is not the version that posts them.
//!
//! So the pipeline lives here and the two callers differ only in what they do
//! with the result: the command prints it, the daemon publishes it. Neither
//! decides what it says.
//!
//! # It returns rather than prints
//!
//! [`Answered`] carries every outcome, including the ones that are not a reply.
//! A mention naming a symbol, a mention naming nothing, and a mention refused by
//! the gate are all *results* — the command renders them for a person and the
//! daemon counts them, and neither has to infer what happened from an empty
//! return.

use radar_onchain::{Budget as CallBudget, RpcClient};
use radar_roast::BaseRates;
use radar_types::Address;

use crate::admission::{Admitted, Gate, Refused};
use crate::log::Entry;
use crate::mention::Asked;
use crate::x::Mention;

/// Everything answering a mention needs that does not change between mentions.
pub struct Answering<'a> {
    /// The chain, read on demand.
    pub client: &'a RpcClient,
    /// The published base rates, or `None` when the snapshot is missing.
    ///
    /// `None` makes a reply say less rather than say more: a recipient count
    /// with no population to quote it against is a number without a meaning,
    /// and rule 9 says a missing rate means the claim cannot be made.
    pub rates: Option<&'a BaseRates>,
    /// Every creator's record, or `None` when the index is missing.
    ///
    /// The fact that makes one reply differ from another. Without it three
    /// coins launched in the same minute produce the same sentences, because
    /// the cost line is a constant and most launches sit in the same recipient
    /// band. `None` makes the reply say so rather than say nothing.
    pub creators: Option<&'a radar_roast::CreatorIndex>,
    /// The model, or `None` for the deterministic template.
    pub provider: Option<&'a dyn radar_model::Provider>,
    /// The analyst's own token, or `None` when no token is special.
    ///
    /// ADR 0013 constraint 5: a price or market-cap fact about this mint is
    /// dropped from the sheet before the model sees it. Read once from
    /// `RADAR_SELF_MINT` by the caller, which **stops** on a value that will not
    /// parse rather than passing `None` -- because `None` here means the rule
    /// is off, and a misspelt mint must not switch it off for the real token.
    pub self_mint: Option<&'a Address>,
    /// Seconds since the epoch, supplied rather than read.
    ///
    /// The gate's windows are computed from it, so a caller can drive a day of
    /// admissions through this without waiting one.
    pub now: u64,
}

/// What happened to one mention.
#[derive(Debug)]
pub enum Answered {
    /// A reply was built. The entry is not yet published.
    Reply(Box<Entry>),
    /// The mention named a symbol, which identifies nothing.
    ///
    /// Carries the reply that says so — the honest answer, and the best content
    /// available: guessing which token a symbol meant is how measurements get
    /// published about the wrong project.
    Ticker(String),
    /// Nothing usable was found in the mention.
    Nothing,
    /// The gate refused it.
    Refused(Refused),
    /// The mint parsed as base58 but is not an address.
    NotAnAddress,
    /// The chain could not be read within the call budget.
    Unreadable(String),
}

/// Answers one mention.
///
/// Reads the chain, builds the fact sheet, writes the reply, and returns the
/// log entry — **without publishing it**. Publishing is
/// [`publish`](crate::publish::publish), which records before it says anything,
/// and keeping the two apart is what lets a caller build two hundred replies and
/// post none of them.
///
/// The gate is consulted here because a refusal must happen **before** the chain
/// is read: the read is the expensive part, and admitting first would mean a
/// refused mention still cost a dossier.
pub fn answer(mention: &Mention, gate: &mut Gate, ctx: &Answering<'_>) -> Answered {
    let mint_text = match crate::mention::read(&mention.text) {
        Asked::Mint(m) => m,
        Asked::Ticker(t) => return Answered::Ticker(crate::ticker_reply(&t)),
        Asked::Nothing => return Answered::Nothing,
    };

    if let Admitted::No(why) = gate.admit(&mention.author, &mint_text, ctx.now) {
        return Answered::Refused(why);
    }

    let Ok(mint) = mint_text.parse::<Address>() else {
        return Answered::NotAnAddress;
    };

    // `radar-onchain`'s own default, which is its three constants: sixty calls,
    // three pages and twenty seconds. Not restated here, because a stranger
    // chooses when this runs and how many run at once -- the ceiling is what
    // stops one prolific creator becoming an unbounded read, and a second copy
    // of it is a second thing to forget to change.
    //
    // An earlier version built the same values by hand. Mutation testing
    // replaced the whole function with `Default::default()` and nothing failed,
    // which was correct: they were the same budget written twice.
    let mut budget = CallBudget::default();
    let dossier = match radar_onchain::build(ctx.client, &mut budget, &mint) {
        Ok(d) => d,
        Err(e) => return Answered::Unreadable(e.to_string()),
    };

    let (sheet, reply) = radar_roast::roast(
        &dossier,
        ctx.rates,
        ctx.creators,
        ctx.provider,
        ctx.self_mint,
    );

    Answered::Reply(Box::new(Entry {
        at: ctx.now,
        mention_id: mention.id.clone(),
        summoner: mention.author.clone(),
        mint: Some(mint_text),
        read_at_slot: dossier.read_at.map(|s| s.0),
        // The evidence, not only the words. A log of replies without fact
        // sheets records what Radar said and not whether it was entitled to say
        // it, and the second is the half that settles an argument.
        fact_sheet: sheet.render(),
        reply: reply.text,
        fellback: reply.fellback.as_ref().map(|f| format!("{f:?}")),
        reply_id: None,
        // Counted where the sheet was built, carried here so the week-close
        // job scores from the record and never re-reads the chain.
        signals: Some(sheet.signals),
    }))
}

/// A refusal, in words worth telling somebody.
#[must_use]
pub fn describe(why: &Refused) -> String {
    match why {
        Refused::Unconfigured => "no limits configured, so nothing is answered".to_owned(),
        Refused::SummonerDaily { cap } => format!("this account has had its {cap} replies today"),
        Refused::GlobalDaily { cap } => format!("the daily cap of {cap} replies is spent"),
        Refused::AlreadyAnswered { reply_id } => {
            format!("already answered for this mint, see {reply_id}")
        }
        Refused::SelfOrIgnored => "Radar does not answer itself".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admission::Limits;

    fn mention(text: &str) -> Mention {
        Mention {
            id: "m1".to_owned(),
            author: "a1".to_owned(),
            text: text.to_owned(),
            parent: None,
        }
    }

    fn gate() -> Gate {
        Gate::new(
            Limits {
                per_summoner_daily: 3,
                global_daily: 50,
                dedupe_seconds: 3600,
            },
            vec!["radar".to_owned()],
        )
    }

    fn ctx(client: &RpcClient) -> Answering<'_> {
        Answering {
            client,
            rates: None,
            creators: None,
            provider: None,
            self_mint: None,
            now: 1_788_000_000,
        }
    }

    /// A client pointed at an address nothing answers on.
    ///
    /// Every test below returns before the client is used. It exists so the
    /// context can be built, and a test that accidentally reached the network
    /// would fail rather than quietly depend on it.
    fn unreachable_client() -> RpcClient {
        RpcClient::new("http://127.0.0.1:1".to_owned())
    }

    #[test]
    fn a_symbol_is_answered_by_asking_for_the_address() {
        // Guessing which token a symbol meant is how measurements get published
        // about the wrong project.
        let client = unreachable_client();
        let out = answer(
            &mention("@radar what about $ABC"),
            &mut gate(),
            &ctx(&client),
        );
        match out {
            Answered::Ticker(reply) => {
                assert!(reply.contains("$ABC"), "{reply}");
                assert!(reply.contains("contract address"), "{reply}");
            }
            other => panic!("a symbol must not be resolved: {other:?}"),
        }
    }

    #[test]
    fn a_mention_naming_nothing_is_not_an_error() {
        let client = unreachable_client();
        let out = answer(&mention("@radar hello"), &mut gate(), &ctx(&client));
        assert!(matches!(out, Answered::Nothing), "{out:?}");
    }

    #[test]
    fn the_gate_refuses_before_the_chain_is_read() {
        // The ordering that matters for the bill: the read is the expensive
        // part, so a refusal must come first. With no limits configured the
        // gate refuses everything, and this returns without touching the
        // unreachable client -- which it would hang on for twenty seconds if
        // the order were wrong.
        let mut closed = Gate::new(
            Limits {
                per_summoner_daily: 0,
                global_daily: 0,
                dedupe_seconds: 0,
            },
            Vec::new(),
        );
        let client = unreachable_client();
        let out = answer(
            &mention("@radar So11111111111111111111111111111111111111112"),
            &mut closed,
            &ctx(&client),
        );
        assert!(matches!(out, Answered::Refused(_)), "{out:?}");
    }

    #[test]
    fn radar_does_not_answer_itself() {
        let client = unreachable_client();
        let mut g = gate();
        let mut m = mention("@radar So11111111111111111111111111111111111111112");
        m.author = "radar".to_owned();
        let out = answer(&m, &mut g, &ctx(&client));
        assert!(
            matches!(out, Answered::Refused(Refused::SelfOrIgnored)),
            "{out:?}"
        );
    }

    #[test]
    fn every_refusal_is_described_in_words_worth_telling_somebody() {
        // The strings go in front of an operator reading a run. A refusal
        // rendered as a debug enum is a refusal nobody acts on.
        for why in [
            Refused::Unconfigured,
            Refused::SummonerDaily { cap: 3 },
            Refused::GlobalDaily { cap: 50 },
            Refused::AlreadyAnswered {
                reply_id: "r1".to_owned(),
            },
            Refused::SelfOrIgnored,
        ] {
            let text = describe(&why);
            // Each says something only it could say. Asserting "not empty and
            // starts with a letter" passed for every refusal rendered as the
            // same string, which mutation testing found by replacing the whole
            // function with one word.
            let distinctive = match why {
                Refused::Unconfigured => "no limits",
                Refused::SummonerDaily { .. } => "this account",
                Refused::GlobalDaily { .. } => "daily cap",
                Refused::AlreadyAnswered { .. } => "already answered",
                Refused::SelfOrIgnored => "itself",
            };
            assert!(
                text.contains(distinctive),
                "{text:?} should name why it refused, not merely refuse"
            );
        }
    }

    #[test]
    fn the_call_budget_is_bounded_on_every_axis() {
        // A stranger chooses when this runs and how many run at once, so an
        // unbounded axis is an unbounded read. Asserted against
        // `radar-onchain`'s own constants rather than numbers written here: a
        // test restating them would pass while the two drifted apart.
        let mut budget = CallBudget::default();
        for i in 0..radar_onchain::budget::DEFAULT_MAX_CALLS {
            assert!(budget.take_call().is_ok(), "call {i} is within budget");
        }
        assert!(
            budget.take_call().is_err(),
            "the call after the ceiling must be refused"
        );

        let mut pages = CallBudget::default();
        for i in 0..radar_onchain::budget::DEFAULT_MAX_PAGES {
            assert!(pages.take_page().is_ok(), "page {i} is within budget");
        }
        assert!(
            pages.take_page().is_err(),
            "the page after the ceiling must be refused"
        );
    }
}
