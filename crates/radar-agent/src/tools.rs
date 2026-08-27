// SPDX-License-Identifier: Apache-2.0
//! What a model is allowed to ask for.
//!
//! # The model has no action tools, and that is the whole design
//!
//! The [considerations document](../../docs/research/vendor/chatgpt-radar-considerations.md)
//! §39 proposes `prepare_trade`, `request_approval` and `execute_trade` as
//! tools an agent should hold. This is the one place that document is wrong for
//! this system, and its own §14 says why: never let AI-generated instructions
//! bypass execution policy. Handing a model an action tool and then policing the
//! policing is one lock where two are free.
//!
//! So the allowlist admits **reads only**, and it is checked twice: a name has
//! to be registered, and it has to survive [`is_read_only`]. The second check is
//! redundant today, which is the point — it stays right when somebody registers
//! an instrument called `prepare_trade` a year from now and nobody remembers
//! this file.
//!
//! # What that buys
//!
//! A model fully persuaded by a token name can emit any text it likes and reach
//! nothing. There is no `Proposal` it can author, no `Authorization` it can
//! request, and no parser turning its output into either — [`crate::reply`]
//! renders model output as text and never interprets it. AGENTS.md rule 1 says
//! model judgement must never authorise capital; this is that rule made
//! structural rather than enforced.

use serde::{Deserialize, Serialize};

/// Verbs that describe doing something rather than looking at it.
///
/// Matched as substrings of a lowercased tool name, so `prepare_trade`,
/// `submitTransaction` and `radar.execute` all fail.
///
/// **Stems rather than whole words**, and the reason is a failure this list
/// already had: it read `approve`, and the considerations document's own
/// `request_approval` walked straight past it. The test naming that tool caught
/// it, which is the argument for testing against the specific names somebody
/// actually proposed rather than against a category.
///
/// Deliberately broad. `sign` also refuses a tool called `design_something`,
/// and that is the right trade: a false positive costs one rename, and a true
/// negative costs the guarantee the whole crate rests on.
const ACTION_VERBS: &[&str] = &[
    "approv", "authoris", "authoriz", "buy", "cancel", "delete", "execut", "prepare", "send",
    "sell", "set", "sign", "submit", "trade", "transfer", "withdraw", "write",
];

/// Whether a tool name describes a read.
///
/// The second of the two checks. A name is admitted only if it is registered
/// *and* passes this, and today every registered instrument passes trivially —
/// which is exactly why it is worth keeping.
#[must_use]
pub fn is_read_only(name: &str) -> bool {
    let lowered = name.to_ascii_lowercase();
    !ACTION_VERBS.iter().any(|verb| lowered.contains(verb))
}

/// Why a tool call was refused.
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error, Serialize, Deserialize)]
#[serde(tag = "refusal", rename_all = "snake_case")]
pub enum Refused {
    /// No such tool is registered.
    ///
    /// Distinct from a tool that exists and is not permitted: one is a model
    /// inventing a capability, the other is a capability someone added that
    /// should never have been reachable. They want different responses.
    #[error("no tool named `{name}`")]
    Unknown {
        /// What was asked for.
        name: String,
    },
    /// The tool exists and describes an action.
    #[error("`{name}` is not a read; a model may not reach it")]
    NotARead {
        /// What was asked for.
        name: String,
    },
    /// The budget will not pay for it.
    #[error("refused for want of budget")]
    OverBudget,
}

/// The tools a model may call.
///
/// Built from names the caller registers, which in practice is the read-only
/// instrument registry — every instrument receives a `&Reader` and structurally
/// cannot write.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Allowlist {
    names: std::collections::BTreeSet<String>,
}

impl Allowlist {
    /// An allowlist admitting nothing.
    ///
    /// The correct starting point, and what an unconfigured agent gets. Rule 8:
    /// a component with no configuration refuses rather than degrades.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a tool, if it is a read.
    ///
    /// Returns whether it was admitted. A name describing an action is dropped
    /// rather than accepted, so a mistake upstream cannot widen what a model can
    /// reach — and the caller learns about it, because the return value says so.
    pub fn allow(&mut self, name: impl Into<String>) -> bool {
        let name = name.into();
        if !is_read_only(&name) {
            return false;
        }
        self.names.insert(name);
        true
    }

    /// Whether a call is permitted.
    ///
    /// # Errors
    ///
    /// Returns [`Refused`] naming which of the two checks failed.
    pub fn check(&self, name: &str) -> Result<(), Refused> {
        if !self.names.contains(name) {
            return Err(Refused::Unknown {
                name: name.to_owned(),
            });
        }
        if !is_read_only(name) {
            return Err(Refused::NotARead {
                name: name.to_owned(),
            });
        }
        Ok(())
    }

    /// Every permitted tool, in name order.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.names.iter().map(String::as_str)
    }

    /// How many tools are permitted.
    #[must_use]
    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// Whether nothing is permitted.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tools_the_considerations_document_proposes_are_all_refused() {
        // §39 lists these as tools an agent should hold. This is the one place
        // that document is wrong for this system, and its own §14 says why.
        for name in [
            "prepare_trade",
            "request_approval",
            "execute_trade",
            "cancel",
            "sign_transaction",
            "submit_transaction",
        ] {
            assert!(!is_read_only(name), "{name} must not read as a read");
            let mut list = Allowlist::new();
            assert!(!list.allow(name), "{name} must not be admissible");
            assert!(list.is_empty());
        }
    }

    #[test]
    fn the_instruments_that_exist_today_are_all_admissible() {
        // The check has to admit the real registry, or it would be turned off.
        let mut list = Allowlist::new();
        for name in ["creator_history", "creator_track_record", "simulate_exit"] {
            assert!(list.allow(name), "{name} should be admissible");
        }
        assert_eq!(list.len(), 3);
    }

    #[test]
    fn an_action_verb_is_caught_whatever_the_casing_or_shape_of_the_name() {
        // The guard is for a name nobody has written yet. It has to survive
        // camelCase, dots, and a verb buried in the middle.
        for name in [
            "submitTransaction",
            "radar.execute",
            "doExecuteNow",
            "SIGN",
            "wallet_withdraw_all",
        ] {
            assert!(!is_read_only(name), "{name} slipped through");
        }
    }

    #[test]
    fn the_allowlist_can_say_what_is_on_it() {
        // `iter` is how a caller renders the tool list into a prompt, so a
        // version of it returning nothing — or one invented name — would send a
        // model a menu unrelated to what `check` will admit. Both halves are
        // asserted: the names, and that they arrive in order.
        let mut list = Allowlist::new();
        for name in ["simulate_exit", "creator_history"] {
            assert!(list.allow(name));
        }

        assert_eq!(
            list.iter().collect::<Vec<_>>(),
            ["creator_history", "simulate_exit"],
            "every registered name, in name order"
        );
        // And emptiness reported in the direction that is easy to leave untested:
        // `is_empty` asserted only when it is true survives a mutant hardcoding
        // it, and then an allowlist full of tools reads as an unconfigured one.
        assert!(!list.is_empty());
        assert!(Allowlist::new().iter().next().is_none());
    }

    #[test]
    fn an_unregistered_tool_and_a_forbidden_one_are_different_refusals() {
        // A model inventing a capability and a capability somebody added that
        // should never have been reachable want different responses: the first
        // is a model being wrong, the second is a bug in the registry.
        let mut list = Allowlist::new();
        list.allow("creator_history");

        assert!(matches!(
            list.check("no_such_tool"),
            Err(Refused::Unknown { .. })
        ));
        assert!(list.check("creator_history").is_ok());
    }

    #[test]
    fn an_unconfigured_allowlist_admits_nothing() {
        // Rule 8. A component with no configuration refuses rather than
        // degrades, and an agent that has not been told what it may read may
        // read nothing.
        let list = Allowlist::new();
        assert!(list.is_empty());
        assert!(matches!(
            list.check("creator_history"),
            Err(Refused::Unknown { .. })
        ));
    }

    #[test]
    fn the_second_check_would_catch_a_registration_that_should_not_have_happened() {
        // `allow` refuses an action name, so `check`'s own test is redundant
        // today. That is the point: it stays right if somebody bypasses `allow`
        // or widens it, a year from now, without reading this file.
        let mut list = Allowlist {
            names: std::collections::BTreeSet::new(),
        };
        // Deliberately inserted past `allow`, which is the failure being modelled.
        list.names.insert("execute_trade".to_owned());

        assert!(
            matches!(list.check("execute_trade"), Err(Refused::NotARead { .. }),),
            "registration is not permission"
        );
    }
}
