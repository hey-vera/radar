// SPDX-License-Identifier: Apache-2.0
//! What a model costs, read from a maintained catalog rather than typed.
//!
//! # Why this is a lookup and not a price
//!
//! [`crate::ApiKey`] and [`crate::OpenAi`] both refuse to default a price, and
//! the reason stands: a price written into this binary goes stale the week a
//! vendor changes its rate card, silently, in the direction of under-counting.
//! A budget that looks respected and is not is worse than no budget.
//!
//! That argument is about **hard-coding**, and it was read for a while as an
//! argument against automation of any kind. It is not. There is a maintained,
//! MIT-licensed, provider-agnostic catalog at [`CATALOG`] that carries every
//! model this repository can reach, and copying two numbers out of it by hand
//! is not more trustworthy than reading them — it is the same numbers with a
//! transcription error available.
//!
//! # Why the running daemon does not call this
//!
//! Nothing here is reached from `radar-analyst`. The catalog produces the two
//! lines an operator pastes into `analyst.env`, and the number that governs
//! spending is the one in that file — pinned, dated, and theirs.
//!
//! The alternative was a fetch at start-up with the environment as an
//! override. It is more automatic and it moves the failure somewhere worse: a
//! third party's number would land inside the budget's own accounting, so a
//! stale or wrong entry would under-count silently and the day's ceiling would
//! be measured against a fiction. That is precisely the failure the
//! no-default-price rule exists to prevent, relocated rather than removed. The
//! catalog is the right source; the runtime is the wrong place for it.
//!
//! # Rounding goes up
//!
//! Catalog prices are dollars per million tokens as decimals; the meter counts
//! whole micro-dollars per million. A price that does not land on a whole
//! micro-dollar is rounded **up**, so the conversion can only ever over-count.
//! Rule 9's direction: the error that costs headroom is recoverable and the
//! error that under-counts is the one that spends money nobody budgeted.

use radar_types::MicroUsd;
use serde_json::Value;

/// The catalog. MIT-licensed, provider-agnostic, one document.
pub const CATALOG: &str = "https://models.dev/api.json";

/// The providers this repository can actually send a request to.
///
/// The catalog lists hundreds, including resellers that re-list the same model
/// id at their own price. Searching all of them would make `gpt-5.6-luna`
/// ambiguous between OpenAI and a dozen brokers Radar has no credential for,
/// and the cheapest of those would win a naive search. These two are the ones
/// [`crate::from_vars`] can build.
pub const REACHABLE: [&str; 2] = ["openai", "anthropic"];

/// What the catalog says about one model.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Listed {
    /// The provider it was found under.
    pub provider: String,
    /// The model id, exactly as `RADAR_MODEL_NAME` wants it.
    pub id: String,
    /// Micro-dollars per million input tokens.
    pub input: MicroUsd,
    /// Micro-dollars per million output tokens.
    pub output: MicroUsd,
    /// Whether it reasons.
    ///
    /// Carried because it decides a second variable. Reasoning tokens bill at
    /// the output rate and never reach the reply, so a reasoning model asked
    /// for three sentences needs `RADAR_MODEL_REASONING_EFFORT=none` or it is
    /// paying for thinking nobody reads.
    pub reasoning: bool,
}

/// Dollars per million tokens as a whole number of micro-dollars per million.
///
/// Rounds up. See the module note: the error that over-counts costs headroom,
/// and the error that under-counts spends money nobody budgeted.
#[must_use]
fn micro_per_million(dollars: f64) -> Option<MicroUsd> {
    if !dollars.is_finite() || dollars < 0.0 {
        return None;
    }
    let micro = (dollars * 1_000_000.0).ceil();
    // Beyond this a price is not a price, it is a parse error wearing one.
    if micro > 1_000_000_000_000.0 {
        return None;
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "bounded above and below three lines up"
    )]
    Some(MicroUsd(micro as u64))
}

/// Finds one model in a catalog document.
///
/// # Errors
///
/// A message naming what went wrong: the document did not parse, the model is
/// in none of the [`REACHABLE`] providers, it is in more than one, or its entry
/// carries no usable price. Every one of those is a thing an operator has to
/// decide about, so none of them is silently resolved.
pub fn find(document: &str, model: &str) -> Result<Listed, String> {
    let catalog: Value = serde_json::from_str(document)
        .map_err(|e| format!("the catalog is not JSON ({e}); {CATALOG} may be down or changed"))?;

    let mut found: Vec<Listed> = Vec::new();
    for provider in REACHABLE {
        let Some(entry) = catalog
            .get(provider)
            .and_then(|p| p.get("models"))
            .and_then(|m| m.get(model))
        else {
            continue;
        };
        let cost = entry.get("cost");
        let input = cost
            .and_then(|c| c.get("input"))
            .and_then(Value::as_f64)
            .and_then(micro_per_million);
        let output = cost
            .and_then(|c| c.get("output"))
            .and_then(Value::as_f64)
            .and_then(micro_per_million);
        let (Some(input), Some(output)) = (input, output) else {
            // Embedding and image models list one side or neither. Reporting
            // that plainly beats returning half a price, which would meter the
            // input and let the output through free.
            return Err(format!(
                "{provider}/{model} is in the catalog but carries no usable input and output \
                 price -- it may not be a text model"
            ));
        };
        found.push(Listed {
            provider: provider.to_owned(),
            id: model.to_owned(),
            input,
            output,
            reasoning: entry
                .get("reasoning")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        });
    }

    match found.len() {
        0 => Err(format!(
            "no model called {model} under {}. Check the spelling against {CATALOG}",
            REACHABLE.join(" or ")
        )),
        1 => Ok(found.remove(0)),
        // Not resolved by preferring one, because the two would be reached with
        // different credentials and billed to different accounts.
        _ => Err(format!(
            "{model} is listed under more than one provider ({}); \
             this repository cannot tell which credential you mean",
            found
                .iter()
                .map(|l| l.provider.as_str())
                .collect::<Vec<_>>()
                .join(" and ")
        )),
    }
}

/// Every text model in the [`REACHABLE`] providers, cheapest input first.
///
/// For the operator who does not yet know what to name. Returns
/// `(provider, id, input, output, reasoning)` so a caller can render a table
/// without re-reading the document.
///
/// # Errors
///
/// The document did not parse.
pub fn list(document: &str) -> Result<Vec<Listed>, String> {
    let catalog: Value = serde_json::from_str(document)
        .map_err(|e| format!("the catalog is not JSON ({e}); {CATALOG} may be down or changed"))?;

    let mut all: Vec<Listed> = Vec::new();
    for provider in REACHABLE {
        let Some(models) = catalog
            .get(provider)
            .and_then(|p| p.get("models"))
            .and_then(Value::as_object)
        else {
            continue;
        };
        for (id, entry) in models {
            let cost = entry.get("cost");
            let input = cost
                .and_then(|c| c.get("input"))
                .and_then(Value::as_f64)
                .and_then(micro_per_million);
            let output = cost
                .and_then(|c| c.get("output"))
                .and_then(Value::as_f64)
                .and_then(micro_per_million);
            // Both, or it is not a text model being priced for this use.
            let (Some(input), Some(output)) = (input, output) else {
                continue;
            };
            if output == MicroUsd::ZERO {
                // An embedding model: priced on input alone. It cannot write a
                // reply, so listing it invites naming it.
                continue;
            }
            all.push(Listed {
                provider: provider.to_owned(),
                id: id.clone(),
                input,
                output,
                reasoning: entry
                    .get("reasoning")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            });
        }
    }
    all.sort_by(|a, b| a.input.cmp(&b.input).then_with(|| a.id.cmp(&b.id)));
    Ok(all)
}

/// Downloads the catalog.
///
/// # Errors
///
/// A message naming the URL. Nothing retries: this runs from a command an
/// operator typed, and a person who can read a failure can run it again.
pub fn fetch(timeout_seconds: u64) -> Result<String, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(core::time::Duration::from_secs(timeout_seconds)))
        .build()
        .into();
    agent
        .get(CATALOG)
        .call()
        .map_err(|e| format!("could not reach {CATALOG}: {e}"))?
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("could not read {CATALOG}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real shape, trimmed. Taken from `models.dev/api.json` on 2026-09-06.
    const DOC: &str = r#"{
      "openai": { "models": {
        "gpt-5.6-luna": { "cost": { "input": 0.2, "output": 1.2 }, "reasoning": true },
        "gpt-4o-mini":  { "cost": { "input": 0.15, "output": 0.6 }, "reasoning": false },
        "text-embedding-3-small": { "cost": { "input": 0.02, "output": 0 } },
        "gpt-image-2": { "cost": { "input": 5 } }
      } },
      "anthropic": { "models": {
        "claude-sonnet-5": { "cost": { "input": 2, "output": 10 } }
      } },
      "some-reseller": { "models": {
        "gpt-5.6-luna": { "cost": { "input": 0.01, "output": 0.02 } }
      } }
    }"#;

    #[test]
    fn a_price_becomes_whole_micro_dollars_per_million() {
        // Worked by hand so this is a measurement rather than a restatement:
        // $0.20 per million is 200,000 micro-dollars per million.
        let luna = find(DOC, "gpt-5.6-luna").expect("listed");
        assert_eq!(luna.provider, "openai");
        assert_eq!(luna.input, MicroUsd(200_000));
        assert_eq!(luna.output, MicroUsd(1_200_000));
        assert!(luna.reasoning);

        let sonnet = find(DOC, "claude-sonnet-5").expect("listed");
        assert_eq!(sonnet.provider, "anthropic");
        assert_eq!(sonnet.input, MicroUsd(2_000_000));
        assert_eq!(sonnet.output, MicroUsd(10_000_000));
        assert!(!sonnet.reasoning, "absent means not a reasoning model");
    }

    #[test]
    fn a_fractional_price_rounds_up_rather_than_to_nearest() {
        // Rule 9's direction. Over-counting costs headroom for one call;
        // under-counting spends money nobody budgeted, every call, invisibly.
        //
        // Re-apply by rounding: 0.0000004 becomes 0 and the model is free.
        assert_eq!(micro_per_million(0.000_000_4), Some(MicroUsd(1)));
        assert_eq!(micro_per_million(1.000_000_1), Some(MicroUsd(1_000_001)));
        // A whole number is not nudged upward by the rounding.
        assert_eq!(micro_per_million(0.15), Some(MicroUsd(150_000)));
        assert_eq!(micro_per_million(0.0), Some(MicroUsd::ZERO));
    }

    #[test]
    fn a_price_that_is_not_a_price_is_refused_rather_than_clamped() {
        for absurd in [-1.0, f64::NAN, f64::INFINITY, 1e18] {
            assert_eq!(micro_per_million(absurd), None, "{absurd}");
        }
    }

    #[test]
    fn the_upper_bound_is_a_million_dollars_per_million_tokens_and_it_is_inclusive() {
        // The ceiling exists to catch a parse error wearing a price -- a
        // catalog field that arrived as a token count, say. Where exactly it
        // sits matters less than that it is pinned: CI mutated `>` into `>=`
        // and nothing failed, so the boundary was decoration.
        //
        // A million dollars per million tokens is a dollar a token. Nothing
        // real is anywhere near it, and it is accepted rather than refused
        // because a bound that rejects its own stated limit is off by one.
        assert_eq!(
            micro_per_million(1_000_000.0),
            Some(MicroUsd(1_000_000_000_000)),
            "the limit itself is a price"
        );
        assert_eq!(
            micro_per_million(1_000_000.000_001),
            None,
            "and anything past it is not"
        );
    }

    #[test]
    fn a_reseller_listing_the_same_id_does_not_win_the_lookup() {
        // The catalog lists hundreds of providers, including brokers that
        // re-list a model at their own price. `some-reseller` quotes
        // gpt-5.6-luna at a twentieth of OpenAI's, and Radar has no credential
        // for it -- a search across every provider would take that number and
        // meter a real bill against a price nobody is charging.
        let luna = find(DOC, "gpt-5.6-luna").expect("listed");
        assert_eq!(luna.provider, "openai");
        assert_eq!(luna.input, MicroUsd(200_000));
    }

    #[test]
    fn a_model_in_both_reachable_providers_is_refused_rather_than_preferred() {
        // The two would be reached with different credentials and billed to
        // different accounts, so preferring one is picking somebody's bill.
        let both = r#"{
          "openai": { "models": { "shared": { "cost": { "input": 1, "output": 2 } } } },
          "anthropic": { "models": { "shared": { "cost": { "input": 3, "output": 4 } } } }
        }"#;
        let why = find(both, "shared").expect_err("ambiguous");
        assert!(why.contains("openai"), "{why}");
        assert!(why.contains("anthropic"), "{why}");
    }

    #[test]
    fn a_model_with_half_a_price_is_reported_rather_than_half_metered() {
        // `gpt-image-2` lists input and no output. Returning it would meter the
        // input and let every output token through free.
        let why = find(DOC, "gpt-image-2").expect_err("half a price is not a price");
        assert!(why.contains("no usable input and output price"), "{why}");
    }

    #[test]
    fn an_unknown_model_says_where_to_check_the_spelling() {
        let why = find(DOC, "gpt-9-imaginary").expect_err("not listed");
        assert!(why.contains("gpt-9-imaginary"), "{why}");
        assert!(why.contains(CATALOG), "{why}");
    }

    #[test]
    fn a_catalog_that_did_not_parse_is_not_an_empty_catalog() {
        // The failure mode worth naming: "no such model" would send an
        // operator to check a spelling that was never the problem.
        let why = find("<html>502 Bad Gateway</html>", "gpt-4o-mini").expect_err("not JSON");
        assert!(why.contains("not JSON"), "{why}");
        assert!(why.contains(CATALOG), "{why}");
    }

    #[test]
    fn the_listing_is_cheapest_first_and_leaves_out_what_cannot_write_a_reply() {
        let all = list(DOC).expect("parses");
        let ids: Vec<&str> = all.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, ["gpt-4o-mini", "gpt-5.6-luna", "claude-sonnet-5"]);
        // An embedding model is priced on input alone and cannot write
        // anything. Listing it invites naming it.
        assert!(!ids.contains(&"text-embedding-3-small"), "{ids:?}");
        assert!(!ids.contains(&"gpt-image-2"), "{ids:?}");
        // And the reseller stays out of it here too.
        assert!(all.iter().all(|l| l.provider != "some-reseller"));
    }
}
