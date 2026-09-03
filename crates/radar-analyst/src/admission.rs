// SPDX-License-Identifier: Apache-2.0
//! Who gets answered, and how often.
//!
//! # Why this exists at all
//!
//! Every reply costs money — an X reply, a model call and a handful of RPC
//! reads — and **every one of them is triggered by a stranger**. The bill is
//! therefore unbounded by default, and it is unbounded by people who can see
//! that it is.
//!
//! There is **no rate limiter anywhere in `radar-serve` today**. This is the
//! first one in the repository, which is why it is a small pure type with its
//! own tests rather than a few counters inside a loop.
//!
//! # Rule 8: no configuration means nothing is answered
//!
//! A gate with no limits loaded refuses everything. That is the same shape as a
//! spend meter with no budget and a signer with no allowlist, and it is chosen
//! for the same reason: **spending nothing is always recoverable.** The failure
//! this prevents is a deploy that silently drops its config and answers the
//! world for free.
//!
//! # Pure, so the refusals are testable
//!
//! No clock and no I/O. The caller passes the current time in, exactly as
//! `radar-risk` takes the slot as an argument, so every refusal here can be
//! reproduced from a recording rather than by waiting a day.

use std::collections::HashMap;

/// What the gate decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Admitted {
    /// Answer it.
    Yes,
    /// Refused, with a reason worth telling the asker.
    No(Refused),
}

/// Why a mention was not answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    /// No limits were configured, so nothing is answered.
    Unconfigured,
    /// This asker has had their allowance today.
    SummonerDaily {
        /// What the cap is.
        cap: u32,
    },
    /// The account's whole daily allowance is spent.
    GlobalDaily {
        /// What the cap is.
        cap: u32,
    },
    /// This mint was answered recently; point at that answer instead.
    AlreadyAnswered {
        /// The reply that already exists.
        reply_id: String,
    },
    /// Radar itself, or an account it should not argue with.
    SelfOrIgnored,
}

/// The limits.
///
/// Deliberately has no `Default`. A caller that wants limits has to state them,
/// because a default here would be a policy invented by whoever typed it and
/// applied to real money.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Replies one summoner may get per day.
    pub per_summoner_daily: u32,
    /// Replies the account will send per day, in total.
    ///
    /// The backstop. X allows 10,000 posts per 24 hours per app, so this is a
    /// **cost** ceiling rather than an API one, and it should be set from what
    /// the operator is willing to spend.
    pub global_daily: u32,
    /// How long a mint's answer is reused rather than recomputed, in seconds.
    pub dedupe_seconds: u64,
}

/// The gate.
#[derive(Debug)]
pub struct Gate {
    limits: Option<Limits>,
    day: u64,
    per_summoner: HashMap<String, u32>,
    global: u32,
    answered: HashMap<String, (u64, String)>,
    ignored: Vec<String>,
}

impl Gate {
    /// A gate with limits.
    #[must_use]
    pub fn new(limits: Limits, ignored: Vec<String>) -> Self {
        Self {
            limits: Some(limits),
            day: 0,
            per_summoner: HashMap::new(),
            global: 0,
            answered: HashMap::new(),
            ignored,
        }
    }

    /// A gate with **no** limits, which answers nothing.
    ///
    /// Rule 8. This is what an instance with missing configuration gets, and it
    /// is not an error state — it is the safe one.
    #[must_use]
    pub fn unconfigured() -> Self {
        Self {
            limits: None,
            day: 0,
            per_summoner: HashMap::new(),
            global: 0,
            answered: HashMap::new(),
            ignored: Vec::new(),
        }
    }

    /// Moves the day forward if `now` is in a later one, clearing allowances.
    ///
    /// **Called by both [`Gate::admit`] and [`Gate::record`], and it has to
    /// be.** An earlier version rolled the day only in `admit`, so a `record`
    /// that arrived first was counted against the *old* day and then wiped by
    /// the next `admit` — every allowance silently reset to zero. The test that
    /// caught it is `allowances_reset_on_a_new_day_and_not_on_a_restart`.
    ///
    /// The day comes from the clock the caller passed rather than from how long
    /// this process has been alive. That distinction is `Agent::restore`'s
    /// defect exactly: the startup path began the day again at zero, so a crash
    /// loop under `Restart=always` handed out a fresh allowance per crash.
    fn roll_to(&mut self, now: u64) {
        let day = now / 86_400;
        if day != self.day {
            self.day = day;
            self.per_summoner.clear();
            self.global = 0;
        }
    }

    /// Decides one mention.
    ///
    /// `now` is seconds since the epoch, passed in rather than read, so a
    /// refusal is reproducible.
    pub fn admit(&mut self, summoner: &str, mint: &str, now: u64) -> Admitted {
        let Some(limits) = self.limits else {
            return Admitted::No(Refused::Unconfigured);
        };

        self.roll_to(now);

        if self.ignored.iter().any(|i| i == summoner) {
            // Answering Radar's own posts is a loop that costs money on every
            // pass, and arguing with another bot is the same thing more slowly.
            return Admitted::No(Refused::SelfOrIgnored);
        }

        // Dedupe first, and deliberately: pointing at an existing answer costs
        // no model call and no RPC, so it must not be refused by a cap it does
        // not spend against.
        if let Some((at, reply_id)) = self.answered.get(mint)
            && now.saturating_sub(*at) < limits.dedupe_seconds
        {
            return Admitted::No(Refused::AlreadyAnswered {
                reply_id: reply_id.clone(),
            });
        }

        if self.global >= limits.global_daily {
            return Admitted::No(Refused::GlobalDaily {
                cap: limits.global_daily,
            });
        }
        let used = self.per_summoner.get(summoner).copied().unwrap_or(0);
        if used >= limits.per_summoner_daily {
            return Admitted::No(Refused::SummonerDaily {
                cap: limits.per_summoner_daily,
            });
        }
        Admitted::Yes
    }

    /// Records that a reply was actually sent.
    ///
    /// Separate from [`Gate::admit`] on purpose. A mention that was admitted and
    /// then failed to post has cost nothing on X, and charging it against the
    /// day's allowance would let a broken publisher silence the account by
    /// spending a budget it never used.
    pub fn record(&mut self, summoner: &str, mint: &str, reply_id: &str, now: u64) {
        self.roll_to(now);
        *self.per_summoner.entry(summoner.to_owned()).or_insert(0) += 1;
        self.global += 1;
        self.answered
            .insert(mint.to_owned(), (now, reply_id.to_owned()));
    }

    /// How many replies have gone out today.
    #[must_use]
    pub const fn sent_today(&self) -> u32 {
        self.global
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = 86_400;

    fn limits() -> Limits {
        Limits {
            per_summoner_daily: 2,
            global_daily: 5,
            dedupe_seconds: 3600,
        }
    }

    #[test]
    fn an_unconfigured_gate_answers_nothing() {
        // Rule 8, and the failure it prevents is a deploy that dropped its
        // config and started answering the world for free.
        let mut gate = Gate::unconfigured();
        assert_eq!(
            gate.admit("alice", "MintOne", DAY),
            Admitted::No(Refused::Unconfigured)
        );
    }

    #[test]
    fn one_summoner_cannot_spend_the_whole_budget() {
        let mut gate = Gate::new(limits(), Vec::new());
        for i in 0..2 {
            assert_eq!(gate.admit("alice", &format!("Mint{i}"), DAY), Admitted::Yes);
            gate.record("alice", &format!("Mint{i}"), "r", DAY);
        }
        assert_eq!(
            gate.admit("alice", "MintThree", DAY),
            Admitted::No(Refused::SummonerDaily { cap: 2 })
        );
        // And somebody else is unaffected, or one loud account could silence
        // the service for everyone.
        assert_eq!(gate.admit("bob", "MintThree", DAY), Admitted::Yes);
    }

    #[test]
    fn the_global_cap_stops_a_crowd() {
        // The per-summoner cap does nothing against many accounts, which is the
        // cheap attack: a hundred throwaway accounts asking once each.
        let mut gate = Gate::new(limits(), Vec::new());
        for i in 0..5 {
            let who = format!("user{i}");
            assert_eq!(gate.admit(&who, &format!("Mint{i}"), DAY), Admitted::Yes);
            gate.record(&who, &format!("Mint{i}"), "r", DAY);
        }
        assert_eq!(
            gate.admit("fresh", "MintSix", DAY),
            Admitted::No(Refused::GlobalDaily { cap: 5 })
        );
    }

    #[test]
    fn a_mint_answered_recently_points_at_the_answer() {
        let mut gate = Gate::new(limits(), Vec::new());
        gate.record("alice", "MintOne", "reply-1", DAY);
        assert_eq!(
            gate.admit("bob", "MintOne", DAY + 60),
            Admitted::No(Refused::AlreadyAnswered {
                reply_id: "reply-1".to_owned()
            })
        );
        // And it expires, so a token that moved is answerable again.
        assert_eq!(gate.admit("bob", "MintOne", DAY + 3601), Admitted::Yes);
    }

    #[test]
    fn dedupe_is_checked_before_the_caps_it_does_not_spend() {
        // Pointing at an existing answer costs no model call and no RPC. If a
        // spent global cap refused it, the account would go silent rather than
        // giving the cheapest useful reply it has.
        let mut gate = Gate::new(limits(), Vec::new());
        gate.record("alice", "MintOne", "reply-1", DAY);
        for i in 0..5 {
            gate.record(&format!("u{i}"), &format!("Other{i}"), "r", DAY);
        }
        assert!(matches!(
            gate.admit("bob", "MintOne", DAY + 60),
            Admitted::No(Refused::AlreadyAnswered { .. })
        ));
    }

    #[test]
    fn the_account_does_not_answer_itself() {
        // A reply to its own post is a loop that costs money on every pass.
        let mut gate = Gate::new(limits(), vec!["radar".to_owned()]);
        assert_eq!(
            gate.admit("radar", "MintOne", DAY),
            Admitted::No(Refused::SelfOrIgnored)
        );
    }

    #[test]
    fn allowances_reset_on_a_new_day_and_not_on_a_restart() {
        // The day comes from the clock the caller passed, not from how long the
        // process has been alive. `Agent::restore` had exactly this defect: the
        // startup path began the day again at zero, so a crash loop handed out
        // a fresh allowance per crash.
        let mut gate = Gate::new(limits(), Vec::new());
        for i in 0..5 {
            let who = format!("user{i}");
            gate.record(&who, &format!("Mint{i}"), "r", DAY);
        }
        assert!(matches!(
            gate.admit("fresh", "MintSix", DAY),
            Admitted::No(Refused::GlobalDaily { .. })
        ));
        assert_eq!(gate.admit("fresh", "MintSix", 2 * DAY), Admitted::Yes);
        assert_eq!(gate.sent_today(), 0);
    }

    #[test]
    fn admitting_without_sending_spends_nothing() {
        // A publisher that is failing must not be able to silence the account by
        // burning an allowance it never used.
        let mut gate = Gate::new(limits(), Vec::new());
        for _ in 0..10 {
            assert_eq!(gate.admit("alice", "MintOne", DAY), Admitted::Yes);
        }
        assert_eq!(gate.sent_today(), 0);
    }
}
