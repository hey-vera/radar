// SPDX-License-Identifier: Apache-2.0
//! The metered path: a key, an endpoint, and a cost computed from what came
//! back.
//!
//! This is the path that survives commercialisation. A personal subscription
//! credential is not a foundation for a sold product — the vendor's terms say
//! so and the considerations document's §115 says so — which is why the
//! subscription path is marked private-use-only and this one is what CI
//! exercises.
//!
//! # The key
//!
//! Read once from the environment and held in memory. It is never logged, never
//! rendered by `Debug`, and never placed in an error: [`Debug`] is written by
//! hand below rather than derived, because the derived one is what a panic
//! message and a `tracing` field both print, and a struct holding a key that
//! derives `Debug` is one `?` away from an incident.
//!
//! # The cost is measured, not declared
//!
//! Prices come from configuration and token counts come from the response, so
//! what the meter records is what the call actually cost. The alternative —
//! declaring a cost per call — is exactly the pattern AGENTS.md already flags in
//! `radar-instruments`, where each instrument states its cost by hand and
//! nothing notices when the declaration and the spend diverge. One instance of
//! that in the repository is enough.

use core::fmt;
use core::time::Duration;

use radar_types::MicroUsd;
use serde_json::{Value, json};

use crate::{
    Answer, ESTIMATED_INPUT_TOKENS, ESTIMATED_OUTPUT_TOKENS, Provider, Request, Unreachable,
    kind_of, non_empty,
};

/// A metered provider speaking the Messages API shape.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey {
    key: String,
    endpoint: String,
    model: String,
    /// Micro-dollars per million input tokens.
    price_in: u64,
    /// Micro-dollars per million output tokens.
    price_out: u64,
}

/// Written by hand so the key cannot reach a log, a panic or an error body.
///
/// The derived implementation would print it, and the places a `Debug` string
/// ends up are exactly the places a credential must not: a `tracing` field, an
/// `expect` message, an `anyhow` chain rendered into an HTTP response.
impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApiKey")
            .field("endpoint", &self.endpoint)
            .field("model", &self.model)
            .field("price_in", &self.price_in)
            .field("price_out", &self.price_out)
            // Named rather than omitted. A `Debug` that silently drops the
            // field reads, to the next person, as a struct that has no key --
            // and the useful thing to see in a log is that there *is* one.
            .field("key", &"<redacted>")
            .finish()
    }
}

impl ApiKey {
    /// Builds from the environment.
    ///
    /// # Errors
    ///
    /// Returns a message naming every variable that is missing. Naming *every*
    /// one rather than the first is the difference between one restart and
    /// four, and an operator setting these up is doing it at the point where
    /// nothing works yet.
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
        // Prices have no default, and that is deliberate. A default price is a
        // spending decision made by whoever wrote this file rather than by
        // whoever runs it, and it would be wrong the week the vendor changes
        // its rate card -- silently, in the direction of under-counting.
        if price_in.is_none() {
            missing.push("RADAR_MODEL_PRICE_IN");
        }
        if price_out.is_none() {
            missing.push("RADAR_MODEL_PRICE_OUT");
        }
        if !missing.is_empty() {
            return Err(format!(
                "RADAR_MODEL_API_KEY is set but {} {} missing (micro-dollars per million tokens)",
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
        })
    }

    /// What a call costs at these prices.
    ///
    /// Integer arithmetic throughout. Rounding a fraction of a micro-dollar
    /// down per call is a rounding error; doing the same sum in floating point
    /// and accumulating it across a day is a budget that drifts.
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
    /// and then somebody downstream has to decide what to do with the call —
    /// which is a decision this design does not want to have.
    #[must_use]
    pub fn body(&self, request: &Request) -> Value {
        json!({
            "model": self.model,
            "max_tokens": ESTIMATED_OUTPUT_TOKENS,
            "system": request.system(),
            "messages": [{ "role": "user", "content": request.render() }],
        })
    }

    /// Reads the answer out of a response body.
    ///
    /// # Errors
    ///
    /// Returns [`Unreachable::Unreadable`] when the shape is not what the
    /// Messages API documents. A provider that changed shape is a provider that
    /// must refuse, not one that returns an empty answer that reads as "the
    /// model had nothing to say".
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

        let text = value
            .get("content")
            .and_then(Value::as_array)
            .map(|blocks| {
                blocks
                    .iter()
                    .filter_map(|b| b.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("")
            })
            .filter(|t| !t.trim().is_empty())
            .ok_or_else(|| Unreachable::Unreadable("no text content in the response".to_owned()))?;

        // Rule 9 again, in the place it is easiest to get wrong: a response with
        // no usage block is a cost that is *unknown*, and an unknown cost
        // charged as zero is a call that was free. `None` makes the caller
        // charge the estimate.
        let usage = value.get("usage");
        let cost = usage.and_then(|u| {
            let input = u.get("input_tokens").and_then(Value::as_u64)?;
            let output = u.get("output_tokens").and_then(Value::as_u64)?;
            Some(self.price(input, output))
        });

        Ok(Answer { text, cost })
    }
}

impl Provider for ApiKey {
    fn name(&self) -> &'static str {
        "api-key"
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
            .header("x-api-key", &self.key)
            .header("anthropic-version", "2023-06-01")
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

    fn provider() -> ApiKey {
        ApiKey::from_vars(
            "sk-not-a-real-key".to_owned(),
            &vars(&[
                (
                    "RADAR_MODEL_ENDPOINT",
                    "https://example.invalid/v1/messages",
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
        // rendered into an HTTP body all print. A derived one here would put
        // the credential in every one of them.
        let rendered = format!("{:?}", provider());
        assert!(!rendered.contains("sk-not-a-real-key"), "{rendered}");
        assert!(rendered.contains("<redacted>"));
        assert!(
            rendered.contains("a-model"),
            "and it still says enough to be useful"
        );
    }

    #[test]
    fn a_partial_configuration_names_every_missing_variable_at_once() {
        // Naming the first is the difference between one restart and four, at
        // the point where nothing works yet and there is no signal but this.
        let why = ApiKey::from_vars("sk-not-a-real-key".to_owned(), &vars(&[]))
            .expect_err("nothing else is set");
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
    fn a_price_that_is_not_a_number_is_a_missing_price() {
        // The tempting reading is zero, which is a provider that costs nothing
        // and therefore never runs out. Rule 9.
        let why = ApiKey::from_vars(
            "sk-not-a-real-key".to_owned(),
            &vars(&[
                ("RADAR_MODEL_ENDPOINT", "https://example.invalid"),
                ("RADAR_MODEL_NAME", "m"),
                ("RADAR_MODEL_PRICE_IN", "three dollars"),
                ("RADAR_MODEL_PRICE_OUT", "15000000"),
            ]),
        )
        .expect_err("that is not a number");
        assert!(why.contains("RADAR_MODEL_PRICE_IN"), "{why}");
    }

    #[test]
    fn cost_is_computed_from_the_tokens_the_provider_reported() {
        // At $3/Mtok in and $15/Mtok out, a thousand in and a hundred out is
        // 3000 + 1500 micro-dollars. Worked by hand so the test is a
        // measurement rather than a restatement of the expression.
        let answer = provider()
            .read(
                r#"{"content":[{"type":"text","text":"hello"}],
                    "usage":{"input_tokens":1000,"output_tokens":100}}"#,
            )
            .expect("a well-formed response");
        assert_eq!(answer.text, "hello");
        assert_eq!(answer.cost, Some(MicroUsd(4_500)));
    }

    #[test]
    fn a_response_with_no_usage_block_has_an_unknown_cost_rather_than_no_cost() {
        // Rule 9 in the place it is easiest to get wrong. `Some(ZERO)` here is
        // a call that was free, and a free call is one the meter never counts.
        let answer = provider()
            .read(r#"{"content":[{"type":"text","text":"hello"}]}"#)
            .expect("text is present");
        assert_eq!(answer.cost, None, "unknown, which the caller charges");
    }

    #[test]
    fn a_provider_error_is_a_refusal_carrying_its_kind() {
        let failure = provider()
            .read(r#"{"error":{"type":"authentication_error","message":"bad key"}}"#)
            .expect_err("an error body is not an answer");
        assert_eq!(
            failure,
            Unreachable::Refused {
                status: "authentication_error".to_owned()
            }
        );
    }

    #[test]
    fn an_empty_answer_is_unreadable_rather_than_a_model_with_nothing_to_say() {
        // The distinction matters because one of these is a bug in the provider
        // and the other would be rendered to an operator as a real reply.
        for body in [
            r#"{"content":[]}"#,
            r#"{"content":[{"type":"text","text":"   "}]}"#,
            r#"{"stop_reason":"end_turn"}"#,
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
        // A model handed a tool schema can emit a call for it, and then
        // somebody downstream has to decide what to do with that call.
        let body = provider().body(&Request::new("You are Radar.", "why?"));
        assert!(body.get("tools").is_none(), "no tool definitions: {body}");
        assert!(body.get("tool_choice").is_none());
        assert_eq!(body["system"], "You are Radar.");
    }

    #[test]
    fn an_error_string_carrying_a_key_is_stripped_before_it_is_reported() {
        // HTTP clients are famously willing to render the request that caused a
        // failure, headers included. That string reaches a log and an HTTP body.
        let leaky = "http error x-api-key: sk-not-a-real-key connection refused";
        let cleaned = kind_of(&leaky);
        assert!(!cleaned.contains("sk-not-a-real-key"), "{cleaned}");
        assert!(cleaned.contains("connection refused"), "{cleaned}");
    }

    #[test]
    fn the_estimate_reserves_more_than_a_typical_call_costs() {
        // An under-estimate is a ceiling that can be passed between the check
        // and the charge; an over-estimate costs headroom for one call.
        let provider = provider();
        assert!(provider.estimate() > provider.price(1_000, 100));
        assert_eq!(provider.name(), "api-key");
    }
}
