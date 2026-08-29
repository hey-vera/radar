// SPDX-License-Identifier: Apache-2.0
//! The chat route.
//!
//! Three crates meet here and the split is the design. [`radar_agent`] decides
//! whether a question may be asked and what the model may see, with no network
//! and no key. [`radar_model`] reaches a provider, with no opinion about
//! policy. This module is the seam, and it is deliberately thin: everything it
//! could get wrong is a thing one of the other two already decided.
//!
//! # What a reply cannot do
//!
//! It is rendered. That is the whole list.
//!
//! There is no branch keyed on what the model said, no field extracted from it,
//! no tool call parsed out of it and no decision it can reach — `radar-serve`
//! does not depend on `radar-risk`'s authorisation path, on `radar-exec` or on
//! `radar-signer`, and `repo-conformance` holds that. This is AGENTS.md rule 1
//! made structural rather than enforced: a model fully persuaded by a token name
//! can write anything into `text` and reach nothing but a `<p>`.
//!
//! # Where the untrusted content comes in
//!
//! The operator's question is placed as a question. Everything Radar looked up
//! in order to answer it — token metadata, a creator's history, social copy — is
//! fenced by [`radar_model::Request::observing`], which escapes the marker
//! before placing it. AGENTS.md rule 4, at the one place in the system where a
//! stranger's text and a language model are in the same buffer.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use radar_agent::{Agent, Unavailable};
use radar_model::{Provider, Request};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::AppState;

/// Radar's own framing, and the only text in a system position.
///
/// Short on purpose. A long system prompt full of prohibitions is a prompt that
/// invites negotiation, and the prohibitions that matter here are not enforced
/// by asking: the model has no action tools and its output is never parsed.
/// What is left for a system prompt to do is set the register — say what Radar
/// is, and that a claim without a source is worse than no claim.
pub const SYSTEM: &str = "You are the reading assistant for Radar, a Solana \
    research recorder. Radar's product is an honest account of what it refused \
    and why, not a profit forecast. The population Radar selects from has a \
    median return around -13% before costs and fewer than one token in ten \
    finishes above a round trip. Answer from the recorded evidence you are \
    given, name the source when you use one, and say plainly when the evidence \
    does not settle the question. Never recommend buying or selling anything: \
    you cannot see a position, a balance or a price, and Radar's trading policy \
    is closed regardless of what you say.";

/// A question from the operator.
#[derive(Clone, Debug, Deserialize)]
pub struct Ask {
    /// What was typed.
    pub question: String,
}

/// The answer, as the interface receives it.
#[derive(Clone, Debug, Serialize)]
pub struct Answered {
    /// What the model said, verbatim and unparsed.
    pub text: String,
    /// Which recorded sources were placed in the prompt.
    ///
    /// The provenance a reader needs. The interface renders an uncited reply
    /// differently from a cited one, because an uncited *claim* has the shape
    /// of a fabrication and a reader must be able to see which they have.
    pub citations: Vec<String>,
    /// Whether anything was consulted at all.
    pub uncited: bool,
}

/// The longest question that will be considered.
///
/// A prompt is charged by the token and the box is on the internet behind an
/// identity check that could one day be misconfigured. A megabyte pasted into
/// it should cost a refusal, not a bill.
pub const MAX_QUESTION_BYTES: usize = 4_000;

/// Whether a question is too long to send.
///
/// Bytes rather than characters, deliberately. Cost tracks bytes far more
/// closely than it tracks characters, and a limit counted in characters lets
/// four bytes of emoji through for the price of one `a` — which is the shape
/// somebody would use to find the ceiling.
#[must_use]
pub fn overlong(question: &str) -> bool {
    question.len() > MAX_QUESTION_BYTES
}

/// Everything the route needs beyond the store.
///
/// Absent entirely when no provider is configured, which is what makes rule 8
/// structural here: there is no half-built agent to accidentally call.
pub struct Chat {
    /// The policy boundary. Behind a mutex because the meter is the one piece
    /// of state a chat route mutates, and it must not be possible for two
    /// questions in flight to each see the budget before the other spent it.
    pub agent: std::sync::Mutex<Agent>,
    /// How to reach a model.
    pub provider: Box<dyn Provider>,
    /// The same provider, when it is one whose credential can be linked from
    /// the interface.
    ///
    /// A second handle rather than a downcast, because the question "can this be
    /// linked" is answered when the provider is built and should not be
    /// re-derived at the route. `None` on the API-key path, which has nothing to
    /// link: a key is set in a file, not authorised in a browser.
    pub linkable: Option<radar_model::Codex>,
    /// How the last call went.
    ///
    /// Recorded because a health check that only says "a provider is
    /// configured" is the shape LEARNINGS records repeatedly: a working
    /// component and a dead one reporting the same thing. A credential that
    /// lapsed after a fortnight of inactivity leaves configuration untouched and
    /// every call failing, and this is what makes that visible.
    pub last: std::sync::Mutex<LastCall>,
}

/// The outcome of the most recent model call.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize)]
#[serde(tag = "last_call", rename_all = "snake_case")]
pub enum LastCall {
    /// Nothing has been asked since this process started.
    ///
    /// Not a failure and not a success. Saying so plainly beats reporting it as
    /// either: a restart makes this the normal state, so alarming on it would
    /// alarm on every deploy, and reporting it as healthy would call an
    /// untested provider working.
    #[default]
    Never,
    /// The last call succeeded.
    Ok,
    /// The last call failed, and this is why.
    Failed {
        /// The refusal, as the provider gave it.
        why: String,
    },
}

/// Answers a question, or says why it cannot.
///
/// # Errors
///
/// Never returns `Err`; every failure is a status and a JSON body, because this
/// is answering a browser.
pub async fn ask(State(state): State<Arc<AppState>>, Json(body): Json<Ask>) -> Response {
    let Some(chat) = state.chat.as_ref() else {
        // Not 503. An unconfigured route does not exist, the same way the
        // unconfigured paid routes do not: a surface that announces its own
        // shape is halfway to one that serves.
        return (StatusCode::NOT_FOUND, Json(json!({ "error": "not found" }))).into_response();
    };

    let question = body.question.trim();
    if question.is_empty() {
        return refuse(StatusCode::BAD_REQUEST, "ask a question");
    }
    if overlong(question) {
        return refuse(
            StatusCode::PAYLOAD_TOO_LARGE,
            &format!("questions are limited to {MAX_QUESTION_BYTES} bytes"),
        );
    }

    let day = today_utc();
    let estimate = chat.provider.estimate();

    // Reserve before calling. Checking a budget and then spending it is a race
    // whenever two questions are in flight, and this box is one browser tab
    // with a retry button.
    let commitment = {
        let Ok(mut agent) = chat.agent.lock() else {
            return refuse(StatusCode::SERVICE_UNAVAILABLE, "the meter is poisoned");
        };
        match agent.begin(estimate, day) {
            Ok(c) => c,
            Err(why) => return unavailable(&why),
        }
    };

    let request = Request::new(SYSTEM, question);
    let provider = &chat.provider;
    let outcome = tokio::task::block_in_place(|| provider.ask(&request));

    let Ok(mut agent) = chat.agent.lock() else {
        return refuse(StatusCode::SERVICE_UNAVAILABLE, "the meter is poisoned");
    };

    match outcome {
        Ok(answer) => {
            // Rule 9: a cost the provider did not report is unknown, not zero.
            // Charging the estimate is what stops a subscription -- which never
            // reports one -- from being free forever.
            agent.settle(commitment, answer.cost.unwrap_or(estimate));
            if let Ok(mut last) = chat.last.lock() {
                *last = LastCall::Ok;
            }
            let citations: Vec<String> = Vec::new();
            Json(Answered {
                text: answer.text,
                uncited: citations.is_empty(),
                citations,
            })
            .into_response()
        }
        Err(why) => {
            // A provider that failed did not charge. Holding the reservation
            // would let a flapping provider exhaust a budget it never spent,
            // which is a self-inflicted outage rather than a safety measure.
            agent.abandon(commitment);
            if let Ok(mut last) = chat.last.lock() {
                *last = LastCall::Failed {
                    why: why.to_string(),
                };
            }
            unavailable(&Unavailable::Unreachable(why.to_string()))
        }
    }
}

/// The day the meter accounts against.
///
/// Public because the binary needs the same day when it builds the agent, and
/// two functions computing "today" independently is how a meter starts a day
/// behind its own ledger.
///
/// Whole days since the epoch, in UTC. The meter takes the day as an argument
/// rather than reading a clock — that is what makes it replayable — so the
/// impurity lives here, at the edge, where it is one line.
#[must_use]
pub fn today_utc() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or_default()
}

/// A refusal with a reason a person can act on.
fn refuse(status: StatusCode, why: &str) -> Response {
    (status, Json(json!({ "error": why }))).into_response()
}

/// Maps a policy refusal to a status.
///
/// Separated because the distinction is the useful part: 402 says the operator
/// has spent the day's budget and tomorrow will work, 503 says something is
/// broken now. Collapsing them into one status is how a spent budget gets
/// diagnosed as an outage.
fn unavailable(why: &Unavailable) -> Response {
    let status = match why {
        Unavailable::NoProvider | Unavailable::NoBudget => StatusCode::NOT_FOUND,
        Unavailable::OverBudget => StatusCode::PAYMENT_REQUIRED,
        Unavailable::Unreachable(_) => StatusCode::SERVICE_UNAVAILABLE,
    };
    refuse(status, &why.to_string())
}

/// What `radar brief` reads to decide whether the agent is healthy.
///
/// Reports in both directions. A check that can only say "ok" is the failure
/// LEARNINGS records repeatedly: a healthy component and a dead one printing
/// the same thing.
#[must_use]
pub fn status(chat: Option<&Chat>) -> serde_json::Value {
    match chat {
        None => json!({ "configured": false }),
        Some(chat) => {
            let ledger = chat.agent.lock().ok().map(|a| a.ledger());
            let last = chat.last.lock().map_or_else(
                |_| LastCall::Failed {
                    why: "the record is poisoned".to_owned(),
                },
                |l| l.clone(),
            );
            json!({
                "configured": true,
                "provider": chat.provider.name(),
                "linkable": chat.linkable.is_some(),
                "estimate_micro_usd": chat.provider.estimate().get(),
                "spent_micro_usd": ledger.as_ref().map(|l| l.spent),
                "tools": chat.agent.lock().ok().map_or(0, |a| a.allowlist().len()),
                "last": last,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use radar_model::{Answer, Unreachable};
    use radar_types::MicroUsd;

    /// A provider that never reaches anything, so the seam can be tested
    /// without a key, a subprocess or a network.
    #[derive(Debug)]
    struct Stub(Result<Answer, Unreachable>);

    impl Provider for Stub {
        fn name(&self) -> &'static str {
            "stub"
        }
        fn estimate(&self) -> MicroUsd {
            MicroUsd(1_000)
        }
        fn ask(&self, _: &Request) -> Result<Answer, Unreachable> {
            self.0.clone()
        }
    }

    fn chat(outcome: Result<Answer, Unreachable>) -> Chat {
        let mut allowlist = radar_agent::Allowlist::new();
        allowlist.allow("creator_history");
        Chat {
            agent: std::sync::Mutex::new(Agent::new(
                radar_agent::Config {
                    budget: radar_agent::Budget {
                        per_call_max: MicroUsd(10_000),
                        daily_max: MicroUsd(20_000),
                    },
                    allowlist,
                },
                today_utc(),
            )),
            provider: Box::new(Stub(outcome)),
            linkable: None,
            last: std::sync::Mutex::new(LastCall::Never),
        }
    }

    #[test]
    fn the_system_prompt_never_invites_a_recommendation() {
        // The register matters more than the prohibitions: a model asked what
        // to buy, by an operator, with no tools, will still answer -- and an
        // answer that reads as advice is the failure mode of this whole
        // feature, because it is the one a reader would act on.
        assert!(SYSTEM.contains("Never recommend buying or selling"));
        assert!(SYSTEM.contains("-13%"), "the base rate is in the framing");
    }

    #[test]
    fn a_spent_budget_and_a_broken_provider_are_different_statuses() {
        // Collapsing these is how a spent budget gets diagnosed as an outage at
        // three in the morning. 402 means tomorrow will work.
        assert_eq!(
            unavailable(&Unavailable::OverBudget).status(),
            StatusCode::PAYMENT_REQUIRED
        );
        assert_eq!(
            unavailable(&Unavailable::Unreachable("the CLI exited".to_owned())).status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        // And an unconfigured agent does not exist rather than being broken.
        assert_eq!(
            unavailable(&Unavailable::NoProvider).status(),
            StatusCode::NOT_FOUND
        );
    }

    #[test]
    fn a_status_report_says_configured_or_not_rather_than_only_ok() {
        // Both directions. A check that can only say "ok" is a healthy
        // component and a dead one printing the same thing, which LEARNINGS
        // records three times.
        assert_eq!(status(None)["configured"], false);
        let chat = chat(Ok(Answer {
            text: "hello".to_owned(),
            cost: None,
        }));
        let live = status(Some(&chat));
        assert_eq!(live["configured"], true);
        assert_eq!(live["provider"], "stub");
        assert_eq!(live["tools"], 1);
        assert_eq!(live["spent_micro_usd"], 0);
    }

    #[test]
    fn a_provider_that_reports_no_cost_is_charged_the_estimate() {
        // Rule 9, at the seam. A subscription never reports a cost, and a call
        // charged as zero is a call the meter never counts -- so the budget
        // never runs out and a chat box left open in a loop runs forever.
        let chat = chat(Ok(Answer {
            text: "hello".to_owned(),
            cost: None,
        }));
        let mut agent = chat.agent.lock().expect("fresh");
        let commitment = agent.begin(MicroUsd(1_000), today_utc()).expect("fits");
        agent.settle(commitment, MicroUsd(1_000));
        assert_eq!(agent.ledger().spent, 1_000, "the estimate, not nothing");
    }

    #[test]
    fn a_failed_call_releases_its_reservation() {
        // A provider that failed did not charge. Holding the estimate would let
        // a flapping provider exhaust a budget it never spent.
        let chat = chat(Err(Unreachable::NoContact("no such binary".to_owned())));
        let mut agent = chat.agent.lock().expect("fresh");
        let commitment = agent.begin(MicroUsd(1_000), today_utc()).expect("fits");
        agent.abandon(commitment);
        assert_eq!(agent.ledger().spent, 0);
    }

    #[test]
    fn the_length_limit_counts_bytes_rather_than_characters() {
        // Truncating would send a mangled question to a metered provider and
        // charge for the answer. The limit exists because a prompt is charged
        // by the token.
        assert!(!overlong(&"x".repeat(MAX_QUESTION_BYTES)), "exactly at it");
        assert!(overlong(&"x".repeat(MAX_QUESTION_BYTES + 1)), "one past it");

        // Four bytes per character: comfortably under the limit counted one
        // way, comfortably over it counted the other.
        let emoji = "\u{1f680}".repeat(MAX_QUESTION_BYTES / 2);
        assert!(emoji.chars().count() < MAX_QUESTION_BYTES);
        assert!(overlong(&emoji), "counted in bytes, this is over");
    }

    #[test]
    fn the_day_advances_and_is_not_a_constant() {
        // The meter's day is an argument precisely so it can be replayed, which
        // means the impurity is this one function -- and a `today` stuck at zero
        // would make the daily ceiling a lifetime one.
        //
        // A range known in advance rather than a lower bound. `>` alone passes
        // for any arithmetic that grows -- seconds instead of days multiplies
        // by 86,400 and is still "greater than 20,000", which is a meter whose
        // day never repeats and whose ceiling therefore never binds.
        let today = today_utc();
        assert!(
            (20_000..30_000).contains(&today),
            "days since 1970 is ~20,700 in 2026 and ~30,000 in 2052; got {today}"
        );
    }
}
