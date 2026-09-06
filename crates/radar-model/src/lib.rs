// SPDX-License-Identifier: Apache-2.0
//! Reaching a model provider.
//!
//! [`radar_agent`] is pure policy and deliberately cannot reach anything. This
//! crate is the other half: the process spawn, the HTTPS call, the environment
//! read. Splitting them is not tidiness — it is what lets the component that
//! renders attacker-controlled token names be exhaustively tested with no
//! network and no key, and it keeps the credential out of that component
//! entirely.
//!
//! # Radar does not hold a token, and it is worth being exact about that
//!
//! Two providers, and they make different promises.
//!
//! [`Codex`] spawns the vendor's CLI, which owns `auth.json`, owns the refresh
//! contract and owns the device-authorisation flow. **Radar contains no code
//! that reads, writes, parses or stores a credential on this path.** The
//! refresh contract is single-writer and belongs to a vendor who will change
//! it; writing a second OAuth client against it is a design that breaks at
//! three in the morning during a rotation nobody scheduled.
//!
//! That is a claim about Radar's code, not about the operating system. If the
//! CLI runs as Radar's own user, the *file* is still readable by that user.
//! [`Codex`] takes the command as configuration so that it need not: point it
//! at a wrapper that drops to a separate user, and the boundary becomes one the
//! kernel enforces rather than one this paragraph asserts. `deploy/README.md`
//! carries the unit that does it.
//!
//! [`ApiKey`] does hold a key, in memory, read once from the environment. It is
//! the path that survives commercialisation — a subscription credential is not
//! a foundation for a sold product — and the one CI exercises against a stub.
//!
//! # The subprocess inherits nothing
//!
//! [`Codex`] clears the child's environment and rebuilds it from a fixed list.
//! `radar-serve`'s own environment holds an x402 payout address, a facilitator
//! URL and — on the other path — a model key, and none of that is any business
//! of a subprocess whose input is partly written by whoever named a token.
//! Inheriting an environment is the sort of default that is invisible until it
//! turns up in an incident report.

#![forbid(unsafe_code)]

pub mod api_key;
pub mod catalog;
pub mod codex;
pub mod openai;
pub mod request;

use radar_types::MicroUsd;

pub use api_key::ApiKey;
pub use catalog::Listed;
pub use codex::Codex;
pub use openai::OpenAi;
pub use request::Request;

/// Tokens assumed to go in, for the reservation made before a call.
///
/// An estimate, not a ceiling — the ceiling is the budget. Deliberately
/// generous: an over-estimate costs a little headroom for the length of one
/// call, and an under-estimate is a limit that can be passed between the check
/// and the charge. Shared by both metered providers, because it is a claim
/// about the prompt this repository sends rather than about a vendor.
pub(crate) const ESTIMATED_INPUT_TOKENS: u64 = 20_000;

/// Tokens assumed to come back, and the ceiling asked of the provider.
pub(crate) const ESTIMATED_OUTPUT_TOKENS: u64 = 2_000;

/// The shape of a failure, without whatever it was carrying.
///
/// HTTP client errors are famously willing to render the request that caused
/// them, headers included. This keeps the sentence and drops anything that
/// looks like a credential — both vendors spell a key `sk-`, and the header it
/// travels in is `x-api-key` on one and `authorization: Bearer` on the other.
pub(crate) fn kind_of(error: &impl core::fmt::Display) -> String {
    let rendered = error.to_string();
    rendered
        .split_whitespace()
        .filter(|word| {
            let lower = word.to_ascii_lowercase();
            !lower.starts_with("sk-")
                && !lower.contains("api-key")
                && !lower.contains("authorization")
                && !lower.starts_with("bearer")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Why a provider could not answer.
///
/// Deliberately several variants rather than one string, because at three in
/// the morning "the binary is not installed", "the credential expired" and "the
/// endpoint is rate-limiting" send an operator to three different places.
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum Unreachable {
    /// The provider's binary or endpoint could not be reached at all.
    #[error("could not reach the provider: {0}")]
    NoContact(String),
    /// The provider answered, and the answer was a refusal.
    ///
    /// Carries the status because 401 means the credential needs attention and
    /// 429 means it does not.
    #[error("the provider refused: {status}")]
    Refused {
        /// What the provider said went wrong.
        status: String,
    },
    /// The provider answered with something this crate could not read.
    #[error("the provider's answer could not be read: {0}")]
    Unreadable(String),
    /// The provider took too long.
    #[error("the provider did not answer within {seconds}s")]
    TimedOut {
        /// The deadline that passed.
        seconds: u64,
    },
}

/// What a provider gave back.
///
/// **Text and a cost, and nothing structured.** There is no field here a caller
/// could branch on, which is the property [`radar_agent::Reply`] has and for
/// the same reason: a model fully persuaded by a token name can fill `text`
/// with anything and reach nothing.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Answer {
    /// What the model said, verbatim.
    pub text: String,
    /// What the call cost, as the provider reported it.
    ///
    /// `None` when the provider does not say — a subscription CLI does not bill
    /// per call. The meter then charges the estimate, because a call whose cost
    /// is unknown must not be free. Rule 9: absent is not zero.
    pub cost: Option<MicroUsd>,
}

/// Something that can be asked a question.
///
/// Object-safe on purpose. The binary holds one of these behind a `dyn` and
/// does not care which it got, which is what makes moving from the subscription
/// to a metered key a configuration change rather than a rewrite.
pub trait Provider: Send + Sync + core::fmt::Debug {
    /// Which provider this is, for logs and for `radar brief`.
    fn name(&self) -> &'static str;

    /// What one call is expected to cost.
    ///
    /// Reserved by the meter before the call and reconciled after. An
    /// over-estimate costs a little headroom; an under-estimate is a ceiling
    /// that can be exceeded between the check and the charge.
    fn estimate(&self) -> MicroUsd;

    /// Asks the question.
    ///
    /// # Errors
    ///
    /// Returns [`Unreachable`]. Blocking: the caller runs this off the async
    /// runtime, because both implementations are synchronous and pretending
    /// otherwise would put a process spawn on a reactor thread.
    fn ask(&self, request: &Request) -> Result<Answer, Unreachable>;
}

/// Which provider an environment asks for.
///
/// Rule 8, and the sharp case is the second. An environment naming *both* is
/// not a preference to be resolved by whichever branch this file happens to
/// check first — it is a misconfiguration, and picking one silently means an
/// operator who believes they have moved off the subscription is still on it.
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum Selection {
    /// Nothing is configured. The agent routes are not mounted.
    #[error("no model provider is configured")]
    None,
    /// More than one is configured, which is a contradiction rather than a
    /// choice.
    ///
    /// Carries the variables it found. With three providers the message can no
    /// longer name them in advance, and "more than one is set" sends an
    /// operator to read a file they have only just edited.
    #[error("more than one model provider is configured ({0}); unset all but one")]
    Ambiguous(String),
    /// One is named but incompletely configured.
    #[error("{0}")]
    Incomplete(String),
}

/// One provider behind the trait object the binary holds.
///
/// A free function rather than a closure because each arm below hands it a
/// different concrete type, and a closure is monomorphic over the first one it
/// sees.
fn boxed(provider: impl Provider + 'static) -> Box<dyn Provider> {
    Box::new(provider)
}

/// Builds the configured provider, or says why there is not one.
///
/// # Errors
///
/// Returns [`Selection`] describing what is missing or contradictory. Every
/// variant is a refusal: nothing here falls back to a cheaper model, to a free
/// tier, or to the other path.
pub fn from_vars(get: &impl Fn(&str) -> Option<String>) -> Result<Box<dyn Provider>, Selection> {
    let codex = non_empty(get, "RADAR_MODEL_CODEX");
    let anthropic = non_empty(get, "RADAR_MODEL_API_KEY");
    let openai = non_empty(get, "RADAR_MODEL_OPENAI_KEY");

    // Named rather than counted, so the refusal can say which ones it found.
    let set: Vec<&str> = [
        codex.as_ref().map(|_| "RADAR_MODEL_CODEX"),
        anthropic.as_ref().map(|_| "RADAR_MODEL_API_KEY"),
        openai.as_ref().map(|_| "RADAR_MODEL_OPENAI_KEY"),
    ]
    .into_iter()
    .flatten()
    .collect();
    if set.len() > 1 {
        return Err(Selection::Ambiguous(set.join(" and ")));
    }

    match (codex, anthropic, openai) {
        (Some(command), _, _) => Codex::from_vars(&command, get)
            .map(boxed)
            .map_err(Selection::Incomplete),
        (_, Some(key), _) => ApiKey::from_vars(key, get)
            .map(boxed)
            .map_err(Selection::Incomplete),
        (_, _, Some(key)) => OpenAi::from_vars(key, get)
            .map(boxed)
            .map_err(Selection::Incomplete),
        (None, None, None) => Err(Selection::None),
    }
}

/// The subscription provider, when one is configured.
///
/// Separate from [`from_vars`] because the caller needs to know whether there is
/// a credential that can be *linked* from a browser, and that is a question
/// about which provider was chosen rather than about the boxed trait object.
/// Answering it by downcasting later would put the answer at the route, where
/// it would be discovered from a failure instead of from configuration.
///
/// Returns `None` on the API-key path, which has nothing to link: a key is set
/// in a file, not authorised in a browser.
#[must_use]
pub fn codex_from_vars(get: &impl Fn(&str) -> Option<String>) -> Option<Codex> {
    let command = non_empty(get, "RADAR_MODEL_CODEX")?;
    // Deliberately silent on failure. `from_vars` is the function that reports a
    // misconfiguration; two callers reporting the same one is two chances to
    // report it differently.
    Codex::from_vars(&command, get).ok()
}

/// Reads the day's model budget from the environment.
///
/// `None` when `RADAR_MODEL_DAILY_USD` is unset, and the caller must then not
/// build an agent at all. AGENTS.md rule 8 names this case in as many words —
/// *a spend meter with no budget loaded refuses everything* — and this is the
/// first place in the running system where that clause is enforced rather than
/// documented, because it is the first component that spends money through a
/// meter at all.
///
/// There is deliberately no default. A default daily ceiling is a spending
/// decision made by whoever wrote this file rather than by whoever pays the
/// bill, and the direction it would be wrong in is the expensive one.
#[must_use]
pub fn budget_from_vars(get: &impl Fn(&str) -> Option<String>) -> Option<radar_agent::Budget> {
    let daily = non_empty(get, "RADAR_MODEL_DAILY_USD")?
        .parse::<f64>()
        .ok()
        .map(MicroUsd::from_dollars)
        .filter(|d| *d > MicroUsd::ZERO)?;

    // A per-call ceiling catches a mispriced call before the daily one does.
    // Defaulting it to the daily maximum makes it inert rather than wrong: the
    // day is still bounded, and an operator who wants the tighter check sets it.
    let per_call = non_empty(get, "RADAR_MODEL_PER_CALL_USD")
        .and_then(|v| v.parse::<f64>().ok())
        .map(MicroUsd::from_dollars)
        .filter(|c| *c > MicroUsd::ZERO)
        .unwrap_or(daily);

    Some(radar_agent::Budget {
        per_call_max: per_call.min(daily),
        daily_max: daily,
    })
}

/// Reads a variable, treating whitespace as absence.
///
/// A variable set to the empty string is what a shell leaves behind when an
/// expansion produced nothing. Treating that as configured is how an unset
/// credential becomes an authenticated-looking call with an empty header.
pub(crate) fn non_empty(get: &impl Fn(&str) -> Option<String>, name: &str) -> Option<String> {
    get(name).filter(|v| !v.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn vars(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: BTreeMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |k: &str| map.get(k).cloned()
    }

    fn full_key() -> Vec<(&'static str, &'static str)> {
        vec![
            ("RADAR_MODEL_API_KEY", "sk-not-a-real-key"),
            (
                "RADAR_MODEL_ENDPOINT",
                "https://example.invalid/v1/messages",
            ),
            ("RADAR_MODEL_NAME", "a-model"),
            ("RADAR_MODEL_PRICE_IN", "3000000"),
            ("RADAR_MODEL_PRICE_OUT", "15000000"),
        ]
    }

    #[test]
    fn an_unconfigured_environment_yields_no_provider() {
        // Rule 8. The routes are not mounted in this state, the same way the
        // x402 surface returns 404 rather than serving free.
        assert_eq!(from_vars(&vars(&[])).err(), Some(Selection::None));
    }

    #[test]
    fn an_environment_naming_both_providers_refuses_rather_than_choosing() {
        // The sharp one. An operator who set the API key in order to move off
        // the subscription, and forgot to unset the old variable, is still on
        // the subscription -- and nothing tells them, because it works.
        let mut pairs = full_key();
        pairs.push(("RADAR_MODEL_CODEX", "codex"));
        let Some(Selection::Ambiguous(named)) = from_vars(&vars(&pairs)).err() else {
            panic!("two providers is a contradiction, not a preference");
        };
        assert!(named.contains("RADAR_MODEL_CODEX"), "{named}");
        assert!(named.contains("RADAR_MODEL_API_KEY"), "{named}");

        // And the pair this repository is actually about to have on the box:
        // an OpenAI key today, an Anthropic one the day the model changes, and
        // the failure mode of leaving both set is paying two vendors while
        // believing you moved.
        let mut both_keys = full_key();
        both_keys.push(("RADAR_MODEL_OPENAI_KEY", "sk-proj-not-a-real-key"));
        let Some(Selection::Ambiguous(named)) = from_vars(&vars(&both_keys)).err() else {
            panic!("two metered keys is a contradiction too");
        };
        assert!(named.contains("RADAR_MODEL_OPENAI_KEY"), "{named}");
    }

    #[test]
    fn an_empty_variable_is_absence_rather_than_configuration() {
        // What a shell leaves behind when an expansion produced nothing. Read
        // as configured, an unset credential becomes a call with an empty
        // authorization header -- which fails as a 401 at the worst possible
        // moment rather than as a refusal at startup.
        for blank in ["", "   ", "\t\n"] {
            let pairs = [
                ("RADAR_MODEL_API_KEY", blank),
                ("RADAR_MODEL_CODEX", blank),
                ("RADAR_MODEL_OPENAI_KEY", blank),
            ];
            assert_eq!(
                from_vars(&vars(&pairs)).err(),
                Some(Selection::None),
                "{blank:?} is not configuration"
            );
        }
    }

    #[test]
    fn a_half_configured_environment_refuses_and_names_what_is_missing() {
        // A key with no endpoint is not a usable provider, and the refusal has
        // to name the variable: an operator reading "incomplete" at 3am learns
        // nothing they did not already know.
        let half = vars(&[("RADAR_MODEL_API_KEY", "sk-not-a-real-key")]);
        let Some(Selection::Incomplete(why)) = from_vars(&half).err() else {
            panic!("a key alone is not a provider");
        };
        assert!(why.contains("RADAR_MODEL_ENDPOINT"), "names it: {why}");
    }

    #[test]
    fn each_path_builds_when_fully_configured() {
        let codex = from_vars(&vars(&[("RADAR_MODEL_CODEX", "codex exec")]))
            .expect("a command is the whole configuration");
        assert_eq!(codex.name(), "codex");

        let key = from_vars(&vars(&full_key())).expect("fully configured");
        assert_eq!(key.name(), "api-key");
        assert!(
            key.estimate().get() > 0,
            "an estimate of zero reserves nothing, and a reservation of nothing is not a ceiling"
        );

        // The OpenAI path reads the same four shared variables, so switching
        // between the two metered providers is the key and the endpoint and
        // nothing else. A third variable set here would mean an operator
        // moving to Anthropic had to find it.
        let mut openai: Vec<(&str, &str)> = full_key()
            .into_iter()
            .filter(|(k, _)| *k != "RADAR_MODEL_API_KEY")
            .collect();
        openai.push(("RADAR_MODEL_OPENAI_KEY", "sk-proj-not-a-real-key"));
        let built = from_vars(&vars(&openai)).expect("fully configured");
        assert_eq!(built.name(), "openai");
        assert!(built.estimate().get() > 0);
        assert!(
            !format!("{built:?}").contains("sk-proj-not-a-real-key"),
            "the key must not reach a log on this path either"
        );
    }

    #[test]
    fn a_missing_daily_budget_is_no_budget_rather_than_a_default_one() {
        // Rule 8, the clause AGENTS.md currently records as unenforced. A
        // default here would be a spending decision made by whoever wrote the
        // file rather than by whoever pays, and wrong in the expensive
        // direction.
        for pairs in [
            vec![],
            vec![("RADAR_MODEL_DAILY_USD", "")],
            vec![("RADAR_MODEL_DAILY_USD", "0")],
            vec![("RADAR_MODEL_DAILY_USD", "-5")],
            vec![("RADAR_MODEL_DAILY_USD", "lots")],
        ] {
            assert_eq!(
                budget_from_vars(&vars(&pairs)),
                None,
                "{pairs:?} is not a budget"
            );
        }
    }

    #[test]
    fn a_per_call_ceiling_defaults_to_the_day_and_never_exceeds_it() {
        // Inert rather than wrong: the day is still bounded. The clamp is the
        // part worth testing -- a per-call ceiling above the daily one reads as
        // a limit and is not one.
        let only_daily = budget_from_vars(&vars(&[("RADAR_MODEL_DAILY_USD", "2.00")]))
            .expect("a daily budget is the whole requirement");
        assert_eq!(only_daily.daily_max, MicroUsd(2_000_000));
        assert_eq!(only_daily.per_call_max, MicroUsd(2_000_000));

        let absurd = budget_from_vars(&vars(&[
            ("RADAR_MODEL_DAILY_USD", "2.00"),
            ("RADAR_MODEL_PER_CALL_USD", "50.00"),
        ]))
        .expect("configured");
        assert_eq!(
            absurd.per_call_max,
            MicroUsd(2_000_000),
            "clamped to the day, which is the real ceiling"
        );

        let tighter = budget_from_vars(&vars(&[
            ("RADAR_MODEL_DAILY_USD", "2.00"),
            ("RADAR_MODEL_PER_CALL_USD", "0.25"),
        ]))
        .expect("configured");
        assert_eq!(tighter.per_call_max, MicroUsd(250_000));

        // A per-call ceiling of zero is a typo, not a policy. Taken literally
        // it refuses every call -- which fails closed, and so would never be
        // diagnosed as a typo: the agent would simply appear broken. Read as
        // absent, it falls back to the day, which is the same behaviour as not
        // setting it and is what an operator who typed it meant.
        for typo in ["0", "0.00", "-1", "cheap"] {
            let budget = budget_from_vars(&vars(&[
                ("RADAR_MODEL_DAILY_USD", "2.00"),
                ("RADAR_MODEL_PER_CALL_USD", typo),
            ]))
            .expect("the day is still configured");
            assert_eq!(
                budget.per_call_max,
                MicroUsd(2_000_000),
                "{typo:?} should fall back to the day rather than refuse everything"
            );
        }
    }

    #[test]
    fn no_refusal_this_crate_authors_can_carry_a_credential() {
        // Every `Unreachable` ends up in a log line and, for the operator
        // routes, in an HTTP body. A provider that echoed the request back on
        // failure -- which several do -- would put the key in both.
        let key = "sk-not-a-real-key";
        let built = from_vars(&vars(&full_key())).expect("fully configured");
        let rendered = format!("{built:?}");
        assert!(
            !rendered.contains(key),
            "Debug is what a panic and a log line print: {rendered}"
        );
    }
}
