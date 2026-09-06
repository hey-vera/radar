// SPDX-License-Identifier: Apache-2.0
//! The same metered path, speaking Chat Completions instead of Messages.
//!
//! [`crate::ApiKey`] and this are the same design twice: a key held in memory, a
//! cost computed from the tokens the provider reported, no tool definitions
//! ever, and a `Debug` written by hand so the credential cannot reach a log. The
//! only differences are the ones the wire forces — the header, the shape of the
//! body, and where the token counts live in the response.
//!
//! # Why a second provider rather than a shape flag on the first
//!
//! A flag would put two wire formats in one `ask`, and the failure it invites is
//! specific: a request built for one shape and read against the other fails as an
//! `Unreadable` at three in the morning rather than as a refusal at startup.
//! Two types, one selected by which variable is set, is the same rule
//! [`crate::from_vars`] already applies to the subscription path.
//!
//! # Switching is configuration, not a deploy
//!
//! Both providers are compiled in, so moving between OpenAI and Anthropic —
//! `claude-sonnet-5`, say — is a change to `/etc/radar/analyst.env` and a
//! restart. Setting more than one is refused rather than resolved, for the
//! reason [`crate::Selection::Ambiguous`] gives.

use core::fmt;
use core::time::Duration;

use radar_types::MicroUsd;
use serde_json::{Value, json};

use crate::{
    Answer, ESTIMATED_INPUT_TOKENS, ESTIMATED_OUTPUT_TOKENS, Provider, Request, Unreachable,
    kind_of, non_empty,
};

/// A metered provider speaking the Chat Completions shape.
#[derive(Clone, PartialEq, Eq)]
pub struct OpenAi {
    key: String,
    endpoint: String,
    model: String,
    /// Micro-dollars per million input tokens.
    price_in: u64,
    /// Micro-dollars per million output tokens.
    price_out: u64,
    /// What to send as `reasoning_effort`, or `None` to omit the field.
    ///
    /// **Every current OpenAI model reasons by default**, and reasoning tokens
    /// bill at the *output* rate while never appearing in the reply. On a
    /// three-sentence write from a fixed sheet that is the whole bill and none
    /// of the product: the model can spend the entire `max_completion_tokens`
    /// ceiling thinking, return empty `content`, and be charged for a reply
    /// that then falls back to the template. `"none"` is what turns that off.
    ///
    /// Not defaulted, in either direction. Sending it always would 400 on the
    /// older non-reasoning models, which are the cheapest ones on the list;
    /// defaulting it to `"none"` would be a decision about answer quality made
    /// by whoever wrote this file. Unset sends nothing and the vendor decides,
    /// which is the only honest resting state -- and
    /// `deploy/analyst.env.example` says loudly which models need it.
    reasoning_effort: Option<String>,
}

/// Written by hand, for the reason [`crate::ApiKey`]'s is.
impl fmt::Debug for OpenAi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAi")
            .field("endpoint", &self.endpoint)
            .field("model", &self.model)
            .field("price_in", &self.price_in)
            .field("price_out", &self.price_out)
            .field("reasoning_effort", &self.reasoning_effort)
            .field("key", &"<redacted>")
            .finish()
    }
}

impl OpenAi {
    /// Builds from the environment.
    ///
    /// The four shared variables are the same ones the Anthropic path reads, so
    /// switching between them changes the key and the endpoint and nothing else.
    ///
    /// # Errors
    ///
    /// Returns a message naming every variable that is missing, for the reason
    /// [`crate::ApiKey::from_vars`] does: an operator setting these up is doing
    /// it at the point where nothing works yet, and naming the first is the
    /// difference between one restart and four.
    pub fn from_vars(key: String, get: &impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let endpoint = non_empty(get, "RADAR_MODEL_ENDPOINT");
        let model = non_empty(get, "RADAR_MODEL_NAME");
        let price_in = non_empty(get, "RADAR_MODEL_PRICE_IN").and_then(|v| v.parse::<u64>().ok());
        let price_out = non_empty(get, "RADAR_MODEL_PRICE_OUT").and_then(|v| v.parse::<u64>().ok());

        let mut missing = Vec::new();
        if endpoint.is_none() {
            missing.push("RADAR_MODEL_ENDPOINT");
        }
        if model.is_none() {
            missing.push("RADAR_MODEL_NAME");
        }
        // No default price, for the reason the sibling gives: a default is a
        // spending decision made by whoever wrote this file, and it goes wrong
        // silently in the direction of under-counting.
        if price_in.is_none() {
            missing.push("RADAR_MODEL_PRICE_IN");
        }
        if price_out.is_none() {
            missing.push("RADAR_MODEL_PRICE_OUT");
        }
        if !missing.is_empty() {
            return Err(format!(
                "RADAR_MODEL_OPENAI_KEY is set but {} {} missing (micro-dollars per million tokens)",
                missing.join(", "),
                if missing.len() == 1 { "is" } else { "are" }
            ));
        }

        Ok(Self {
            key,
            endpoint: endpoint.unwrap_or_default(),
            model: model.unwrap_or_default(),
            price_in: price_in.unwrap_or_default(),
            price_out: price_out.unwrap_or_default(),
            // Optional, so it is not in the missing-variable list above: an
            // instance that never sets it is correctly configured.
            reasoning_effort: non_empty(get, "RADAR_MODEL_REASONING_EFFORT"),
        })
    }

    /// What a call costs at these prices.
    ///
    /// Integer arithmetic, for the reason the sibling gives: a fraction of a
    /// micro-dollar rounded down per call is a rounding error, and the same sum
    /// in floating point accumulated across a day is a budget that drifts.
    #[must_use]
    pub const fn price(&self, input_tokens: u64, output_tokens: u64) -> MicroUsd {
        let cost = input_tokens.saturating_mul(self.price_in) / 1_000_000
            + output_tokens.saturating_mul(self.price_out) / 1_000_000;
        MicroUsd(cost)
    }

    /// The request body.
    ///
    /// Split out because it is the part worth asserting on: no tool
    /// definitions, ever. A model handed a tool schema can emit a call for it,
    /// and then somebody downstream has to decide what to do with the call.
    ///
    /// **`max_completion_tokens`, not `max_tokens`.** OpenAI rejects the older
    /// field outright on its reasoning models, and accepts the newer one on all
    /// of them — so the field that works everywhere is the one that does not
    /// have to be revisited the first time the model name changes.
    ///
    /// The system prompt goes in as a `system` message. OpenAI's newer models
    /// name that role `developer`; `system` is still accepted and mapped to it,
    /// and it is the one the older models understand, so it is the choice that
    /// spans the range of models this endpoint might be pointed at.
    #[must_use]
    pub fn body(&self, request: &Request) -> Value {
        let mut body = json!({
            "model": self.model,
            "max_completion_tokens": ESTIMATED_OUTPUT_TOKENS,
            "messages": [
                { "role": "system", "content": request.system() },
                { "role": "user", "content": request.render() },
            ],
        });
        // Added rather than always present, because the field itself is a 400
        // on the models that cannot reason.
        if let Some(effort) = &self.reasoning_effort {
            body["reasoning_effort"] = json!(effort);
        }
        body
    }

    /// Reads the answer out of a response body.
    ///
    /// # Errors
    ///
    /// [`Unreachable::Unreadable`] when the shape is not what Chat Completions
    /// documents, and [`Unreachable::Refused`] when the body is an error or the
    /// model declined. A provider that changed shape must refuse rather than
    /// return an empty answer that reads as "the model had nothing to say".
    pub fn read(&self, body: &str) -> Result<Answer, Unreachable> {
        let value: Value = serde_json::from_str(body)
            .map_err(|e| Unreachable::Unreadable(format!("not JSON: {e}")))?;

        if let Some(error) = value.get("error") {
            let kind = error
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("unspecified");
            return Err(Unreachable::Refused {
                status: kind.to_owned(),
            });
        }

        let message = value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|c| c.first())
            .and_then(|choice| choice.get("message"));

        // A declined answer is the provider refusing, not a malformed body, and
        // the two send an operator to different places. It arrives as a
        // populated `refusal` beside a null `content`, so it has to be checked
        // before the content is read or it reads as an empty answer.
        if let Some(why) = message
            .and_then(|m| m.get("refusal"))
            .and_then(Value::as_str)
            .filter(|w| !w.trim().is_empty())
        {
            return Err(Unreachable::Refused {
                status: format!("the model declined: {why}"),
            });
        }

        let text = message
            .and_then(|m| m.get("content"))
            .and_then(Value::as_str)
            .filter(|t| !t.trim().is_empty())
            .ok_or_else(|| {
                Unreachable::Unreadable("no message content in the response".to_owned())
            })?
            .to_owned();

        // Rule 9: a response with no usage block is a cost that is *unknown*,
        // and an unknown cost charged as zero is a call that was free. `None`
        // makes the caller charge what it reserved.
        //
        // `completion_tokens` includes reasoning tokens on the models that emit
        // them, which is correct here: OpenAI bills for those at the output
        // rate, so counting them is what makes this the real price rather than
        // the visible one.
        let cost = value.get("usage").and_then(|u| {
            let input = u.get("prompt_tokens").and_then(Value::as_u64)?;
            let output = u.get("completion_tokens").and_then(Value::as_u64)?;
            Some(self.price(input, output))
        });

        Ok(Answer { text, cost })
    }
}

impl Provider for OpenAi {
    fn name(&self) -> &'static str {
        "openai"
    }

    fn estimate(&self) -> MicroUsd {
        self.price(ESTIMATED_INPUT_TOKENS, ESTIMATED_OUTPUT_TOKENS)
    }

    fn ask(&self, request: &Request) -> Result<Answer, Unreachable> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(request.timeout_seconds)))
            .build()
            .into();

        let mut response = agent
            .post(&self.endpoint)
            .content_type("application/json")
            .header("authorization", &format!("Bearer {}", self.key))
            .send(self.body(request).to_string())
            .map_err(|e| {
                // `e` can carry the request in some shapes, and the request
                // carries the header. The status is what an operator needs and
                // is all that is safe to keep.
                e.to_string().find("status code").map_or_else(
                    || Unreachable::NoContact(kind_of(&e)),
                    |_| Unreachable::Refused {
                        status: kind_of(&e),
                    },
                )
            })?;

        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|e| Unreachable::Unreadable(kind_of(&e)))?;
        self.read(&body)
    }
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

    fn provider() -> OpenAi {
        OpenAi::from_vars(
            "sk-proj-not-a-real-key".to_owned(),
            &vars(&[
                (
                    "RADAR_MODEL_ENDPOINT",
                    "https://example.invalid/v1/chat/completions",
                ),
                ("RADAR_MODEL_NAME", "a-model"),
                ("RADAR_MODEL_PRICE_IN", "3000000"),
                ("RADAR_MODEL_PRICE_OUT", "15000000"),
            ]),
        )
        .expect("fully configured")
    }

    #[test]
    fn the_key_is_not_in_the_debug_output() {
        // `Debug` is what a panic message, a `tracing` field and an error chain
        // rendered into an HTTP body all print.
        let rendered = format!("{:?}", provider());
        assert!(!rendered.contains("sk-proj-not-a-real-key"), "{rendered}");
        assert!(rendered.contains("<redacted>"));
        assert!(
            rendered.contains("a-model"),
            "and it still says enough to be useful"
        );
    }

    #[test]
    fn a_partial_configuration_names_every_missing_variable_at_once() {
        let why = OpenAi::from_vars("sk-proj-not-a-real-key".to_owned(), &vars(&[]))
            .expect_err("nothing else is set");
        assert!(why.contains("RADAR_MODEL_OPENAI_KEY"), "{why}");
        for name in [
            "RADAR_MODEL_ENDPOINT",
            "RADAR_MODEL_NAME",
            "RADAR_MODEL_PRICE_IN",
            "RADAR_MODEL_PRICE_OUT",
        ] {
            assert!(why.contains(name), "{name} is not named in {why}");
        }
    }

    #[test]
    fn cost_is_computed_from_the_tokens_the_provider_reported() {
        // At $3/Mtok in and $15/Mtok out, a thousand in and a hundred out is
        // 3000 + 1500 micro-dollars. Worked by hand, so this is a measurement
        // rather than a restatement of the expression -- and it is the same
        // arithmetic the Anthropic path is checked against, from a body whose
        // token counts live under different names.
        let answer = provider()
            .read(
                r#"{"choices":[{"message":{"role":"assistant","content":"hello"}}],
                    "usage":{"prompt_tokens":1000,"completion_tokens":100}}"#,
            )
            .expect("a well-formed response");
        assert_eq!(answer.text, "hello");
        assert_eq!(answer.cost, Some(MicroUsd(4_500)));
    }

    #[test]
    fn a_response_with_no_usage_block_has_an_unknown_cost_rather_than_no_cost() {
        // Rule 9. `Some(ZERO)` here is a call that was free, and a free call is
        // one the meter never counts.
        let answer = provider()
            .read(r#"{"choices":[{"message":{"content":"hello"}}]}"#)
            .expect("text is present");
        assert_eq!(answer.cost, None, "unknown, which the caller charges");
    }

    #[test]
    fn a_usage_block_missing_half_its_counts_is_unknown_rather_than_half_priced() {
        // The tempting reading is to price the half that is there. That is a
        // cost that is wrong in the cheap direction, reported as if it were
        // measured -- worse than an unknown, which at least charges the
        // reservation.
        for usage in [
            r#""usage":{"prompt_tokens":1000}"#,
            r#""usage":{"completion_tokens":100}"#,
            r#""usage":{}"#,
        ] {
            let body = format!(r#"{{"choices":[{{"message":{{"content":"hi"}}}}],{usage}}}"#);
            assert_eq!(
                provider().read(&body).expect("text is present").cost,
                None,
                "{usage} is not a price"
            );
        }
    }

    #[test]
    fn a_provider_error_is_a_refusal_carrying_its_kind() {
        let failure = provider()
            .read(
                r#"{"error":{"message":"Incorrect API key provided",
                    "type":"invalid_request_error","code":"invalid_api_key"}}"#,
            )
            .expect_err("an error body is not an answer");
        assert_eq!(
            failure,
            Unreachable::Refused {
                status: "invalid_request_error".to_owned()
            }
        );
    }

    #[test]
    fn a_declined_answer_is_a_refusal_rather_than_an_empty_one() {
        // It arrives as a populated `refusal` beside a null `content`, so a
        // reader that went for the content first would report it as a
        // malformed body -- and send an operator to look for a broken provider
        // instead of at what the model was asked.
        let failure = provider()
            .read(
                r#"{"choices":[{"message":{"content":null,
                    "refusal":"I can't help with that"}}]}"#,
            )
            .expect_err("a declined answer is not an answer");
        match failure {
            Unreachable::Refused { status } => {
                assert!(status.contains("declined"), "{status}");
                assert!(status.contains("I can't help with that"), "{status}");
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn an_empty_answer_is_unreadable_rather_than_a_model_with_nothing_to_say() {
        // One of these is a bug in the provider and the other would be rendered
        // to a reader as a real reply.
        for body in [
            r#"{"choices":[]}"#,
            r#"{"choices":[{"message":{"content":"   "}}]}"#,
            r#"{"choices":[{"message":{"content":null}}]}"#,
            r#"{"choices":[{"finish_reason":"stop"}]}"#,
            r#"{"object":"chat.completion"}"#,
            "not json at all",
        ] {
            assert!(
                matches!(provider().read(body), Err(Unreachable::Unreadable(_))),
                "{body} should not read as an answer"
            );
        }
    }

    #[test]
    fn the_request_body_offers_the_model_no_tools() {
        // The property the whole design rests on, asserted where it is decided.
        let body = provider().body(&Request::new("You are Radar.", "why?"));
        assert!(body.get("tools").is_none(), "no tool definitions: {body}");
        assert!(body.get("tool_choice").is_none());
        assert!(body.get("functions").is_none(), "nor the older spelling");
    }

    #[test]
    fn the_system_prompt_is_the_first_message_and_the_question_is_the_second() {
        // Order is the property, not a formatting preference: Chat Completions
        // has no separate system field, so Radar's own instructions are a
        // message like any other and only their position says they come first.
        let body = provider().body(&Request::new("You are Radar.", "why?"));
        let messages = body["messages"].as_array().expect("messages");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[0]["content"], "You are Radar.");
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(messages[1]["content"], "why?");
    }

    #[test]
    fn the_token_ceiling_uses_the_field_every_model_accepts() {
        // `max_tokens` is rejected outright by OpenAI's reasoning models. The
        // failure would arrive as a 400 the first time the model name changed,
        // which is a deploy that looks like a broken credential.
        let body = provider().body(&Request::new("s", "q"));
        assert_eq!(body["max_completion_tokens"], ESTIMATED_OUTPUT_TOKENS);
        assert!(body.get("max_tokens").is_none(), "{body}");
    }

    #[test]
    fn the_estimate_reserves_more_than_a_typical_call_costs() {
        // An under-estimate is a ceiling that can be passed between the check
        // and the charge; an over-estimate costs headroom for one call.
        let provider = provider();
        assert!(provider.estimate() > provider.price(1_000, 100));
        assert_eq!(provider.name(), "openai");
    }

    #[test]
    fn the_reasoning_budget_is_sent_only_when_it_is_configured() {
        // Every current OpenAI model reasons by default, and reasoning tokens
        // bill at the OUTPUT rate while never appearing in the reply. On a
        // three-sentence write that is the whole bill and none of the product:
        // the model can spend the entire `max_completion_tokens` ceiling
        // thinking, return empty `content`, and be charged for a reply that
        // then falls back to the template.
        //
        // It is absent by default rather than "none", because the field itself
        // is a 400 on the older non-reasoning models -- which are the cheapest
        // ones available.
        let bare = provider().body(&Request::new("s", "q"));
        assert!(
            bare.get("reasoning_effort").is_none(),
            "unset must send nothing: {bare}"
        );

        let quiet = OpenAi::from_vars(
            "sk-proj-not-a-real-key".to_owned(),
            &vars(&[
                (
                    "RADAR_MODEL_ENDPOINT",
                    "https://example.invalid/v1/chat/completions",
                ),
                ("RADAR_MODEL_NAME", "a-model"),
                ("RADAR_MODEL_PRICE_IN", "200000"),
                ("RADAR_MODEL_PRICE_OUT", "1200000"),
                ("RADAR_MODEL_REASONING_EFFORT", "none"),
            ]),
        )
        .expect("fully configured");
        assert_eq!(
            quiet.body(&Request::new("s", "q"))["reasoning_effort"],
            "none"
        );

        // Blank is absence, for the reason every other variable treats it so:
        // it is what a shell leaves behind when an expansion produced nothing.
        let blank = OpenAi::from_vars(
            "sk-proj-not-a-real-key".to_owned(),
            &vars(&[
                (
                    "RADAR_MODEL_ENDPOINT",
                    "https://example.invalid/v1/chat/completions",
                ),
                ("RADAR_MODEL_NAME", "a-model"),
                ("RADAR_MODEL_PRICE_IN", "200000"),
                ("RADAR_MODEL_PRICE_OUT", "1200000"),
                ("RADAR_MODEL_REASONING_EFFORT", "   "),
            ]),
        )
        .expect("fully configured");
        assert!(
            blank
                .body(&Request::new("s", "q"))
                .get("reasoning_effort")
                .is_none()
        );
    }
}
