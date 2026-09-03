// SPDX-License-Identifier: Apache-2.0
//! Claims the analyst may not make, enforced rather than requested.
//!
//! # Why these are in code
//!
//! `GOAL.md` lists three constraints "to be designed in rather than
//! discovered", and the first is *"it states only what was measured. Never
//! 'this is a scam' — always the count, the history, the number."* A system
//! prompt asking for that is a request. This is the part that makes it true.
//!
//! The categories are not stylistic. Each one is a different kind of exposure,
//! and they are worth separating because they fail differently:
//!
//! - **A verdict about a person** — "scam", "rug", "fraud" — is a public,
//!   automated, at-scale statement about an identifiable project. Factual
//!   accuracy plus the absence of verdict words is the protection, and the
//!   second half is this file.
//! - **Reassurance** — "safe", "legit" — is worse than an accusation, because
//!   the reader acts on it. `GOAL.md` refuses a single safety score for exactly
//!   this reason: "a green shield is 'unknown rendered as safe'".
//! - **Advice** — "buy", "sell", "hold", a price prediction — moves measured
//!   commentary toward regulated investment advice. Measurements are not advice;
//!   recommendations are.
//! - **A cabal implied from a recipient count.** `0008` never resolved
//!   recipients to owners, and `0012` proves it cannot be done from this data:
//!   a destination is an `(owner, mint)` token account, so recipient sets cannot
//!   recur across mints. Saying "six wallets" or "six people" claims an identity
//!   the measurement does not carry.
//! - **Graduation history as a good sign.** `0011` measures organic graduations
//!   ending at a median −3,228 bps against −853 for tokens that never
//!   graduated. A creator whose tokens graduate is not a safer bet, and 0007's
//!   signal must never be published as though they were.
//!
//! # It is deliberately blunt
//!
//! This matches substrings, so it refuses a reply that says "this is not a
//! scam" as readily as one that says "this is a scam". That is the right trade:
//! the cost of a false positive is the deterministic template shipping instead,
//! and the cost of a false negative is a public accusation. A checker that tried
//! to read negation would be a checker arguing about meaning, and the point of
//! this one is that it does not argue.

/// A phrase the reply may not contain, and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rule {
    /// The phrase, lowercase.
    pub phrase: &'static str,
    /// Which line it crosses.
    pub because: &'static str,
}

/// Every forbidden phrase.
pub const RULES: &[Rule] = &[
    // A verdict about a person or a project.
    Rule {
        phrase: "scam",
        because: "a verdict about an identifiable project",
    },
    Rule {
        phrase: "rug",
        because: "a verdict about an identifiable project",
    },
    Rule {
        phrase: "fraud",
        because: "a verdict about an identifiable project",
    },
    Rule {
        phrase: "stole",
        because: "a verdict about an identifiable project",
    },
    Rule {
        phrase: "stolen",
        because: "a verdict about an identifiable project",
    },
    Rule {
        phrase: "criminal",
        because: "a verdict about an identifiable project",
    },
    Rule {
        phrase: "thief",
        because: "a verdict about an identifiable project",
    },
    // Reassurance. Worse than an accusation, because it is acted on.
    Rule {
        phrase: "is safe",
        because: "reassurance -- unknown rendered as safe",
    },
    Rule {
        phrase: "looks safe",
        because: "reassurance -- unknown rendered as safe",
    },
    Rule {
        phrase: "totally safe",
        because: "reassurance -- unknown rendered as safe",
    },
    Rule {
        phrase: "legit",
        because: "reassurance -- unknown rendered as safe",
    },
    Rule {
        phrase: "trustworthy",
        because: "reassurance -- unknown rendered as safe",
    },
    // Advice.
    Rule {
        phrase: "you should buy",
        because: "advice, not commentary",
    },
    Rule {
        phrase: "you should sell",
        because: "advice, not commentary",
    },
    Rule {
        phrase: "should buy",
        because: "advice, not commentary",
    },
    Rule {
        phrase: "should sell",
        because: "advice, not commentary",
    },
    Rule {
        phrase: "buy this",
        because: "advice, not commentary",
    },
    Rule {
        phrase: "sell this",
        because: "advice, not commentary",
    },
    Rule {
        phrase: "hold this",
        because: "advice, not commentary",
    },
    Rule {
        phrase: "ape in",
        because: "advice, not commentary",
    },
    Rule {
        phrase: "good investment",
        because: "advice, not commentary",
    },
    Rule {
        phrase: "bad investment",
        because: "advice, not commentary",
    },
    // NOT a rule: "financial advice". It was one, and the test asserting the
    // deterministic template passes this check caught it -- the template ends
    // "Not financial advice", so the analyst's own safest possible reply was
    // being refused by its own rule. The disclaimer is the thing ADR 0005's
    // unresolved precondition 3 asks for, and blocking it would have removed
    // the one sentence most worth keeping. The advice itself is caught by the
    // phrases above.
    Rule {
        phrase: "to the moon",
        because: "a price prediction",
    },
    Rule {
        phrase: "will pump",
        because: "a price prediction",
    },
    Rule {
        phrase: "will dump",
        because: "a price prediction",
    },
    Rule {
        phrase: "price target",
        because: "a price prediction",
    },
    Rule {
        phrase: "guaranteed",
        because: "a price prediction",
    },
    // A cabal identity the measurement cannot see. 0012.
    Rule {
        phrase: "wallets bought",
        because: "recipients are token accounts, not wallets (0012)",
    },
    Rule {
        phrase: "people bought",
        because: "recipients are token accounts, not people (0012)",
    },
    Rule {
        phrase: "insiders",
        because: "recipients are token accounts, not identified people (0012)",
    },
    Rule {
        phrase: "the same group",
        because: "recipient sets cannot recur across mints (0012)",
    },
    Rule {
        phrase: "cabal",
        because: "an identity the recipient count cannot carry (0012)",
    },
    Rule {
        phrase: "one person",
        because: "an identity the recipient count cannot carry (0012)",
    },
];

/// A phrase that must not be published, found in a reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Violation {
    /// The phrase found.
    pub phrase: &'static str,
    /// Why it is refused.
    pub because: &'static str,
}

/// Every forbidden phrase in a reply.
///
/// Case-insensitive, because a capitalised accusation is the same accusation.
#[must_use]
pub fn check(reply: &str) -> Vec<Violation> {
    let lower = reply.to_lowercase();
    RULES
        .iter()
        .filter(|r| lower.contains(r.phrase))
        .map(|r| Violation {
            phrase: r.phrase,
            because: r.because,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_verdict_about_a_project_is_refused() {
        assert_eq!(check("This is a scam.")[0].phrase, "scam");
        assert_eq!(check("Classic RUG.")[0].phrase, "rug");
    }

    #[test]
    fn reassurance_is_refused_as_readily_as_accusation() {
        // GOAL.md refuses a single safety score because a green shield is
        // "unknown rendered as safe". This is the sentence form of one.
        assert!(!check("This one is safe.").is_empty());
        assert!(!check("Looks legit to me.").is_empty());
    }

    #[test]
    fn advice_and_price_predictions_are_refused() {
        assert!(!check("You should buy this.").is_empty());
        assert!(!check("This is going to the moon.").is_empty());
        assert!(!check("Guaranteed returns.").is_empty());
    }

    #[test]
    fn an_identity_the_measurement_cannot_see_is_refused() {
        // 0012: a destination is an (owner, mint) token account, so recipient
        // sets cannot recur across mints and "six wallets" claims something the
        // data does not hold.
        assert!(!check("Six wallets bought it in the launch block.").is_empty());
        assert!(!check("All one person.").is_empty());
        assert!(!check("The same group as last time.").is_empty());
    }

    #[test]
    fn a_denial_is_refused_too_and_that_is_deliberate() {
        // Blunt on purpose. A checker that read negation would be a checker
        // arguing about meaning; the cost of this false positive is the
        // deterministic template, and the cost of the false negative is a
        // public accusation.
        assert!(!check("This is not a scam.").is_empty());
    }

    #[test]
    fn an_ordinary_measured_reply_passes() {
        let reply = "Eleven recipients in the launch block. 0.5% of launches that never \
                     graduated look like that. The round trip at $50 is about 4.6%. \
                     Radar has no record of this creator.";
        assert!(check(reply).is_empty(), "{:?}", check(reply));
    }

    #[test]
    fn every_rule_is_lowercase_or_it_can_never_match() {
        // The check lowercases the reply, so an uppercase rule would be dead
        // and would look like it was working.
        for rule in RULES {
            assert_eq!(rule.phrase, rule.phrase.to_lowercase(), "{}", rule.phrase);
            assert!(!rule.because.is_empty());
        }
    }
}
