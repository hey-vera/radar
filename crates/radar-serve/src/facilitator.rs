// SPDX-License-Identifier: Apache-2.0
//! Verifying and settling an x402 payment.
//!
//! Radar does not verify signatures or submit settlement transactions itself.
//! Settling a Solana payment means building, signing and confirming a
//! transaction, and a request handler is the wrong place for any of that — it
//! would put a second signing key inside the process that serves the public
//! surface, which is the exact arrangement the trading lane spends a whole
//! separate binary avoiding.
//!
//! So the facilitator is asked. What matters here is *how* it is asked, and
//! there are three rules:
//!
//! **1. Fail closed, always.** A timeout, an unreachable facilitator, an
//! unparseable answer, a missing field — every one of them refuses the request.
//! There is no path through this module that serves a response without an
//! explicit `isValid: true`. Accepting a payment that cannot be verified is
//! worse than refusing the request.
//!
//! **2. Radar states the requirements, never the client.** The payload the
//! client sends says what it *paid*; the requirements say what was *owed*, and
//! those are built here from the instrument's own price. A facilitator asked to
//! check a payment against the payer's idea of the price would approve every
//! payment ever made.
//!
//! **3. Verify, then work, then settle.** Verification comes first, so an unpaid
//! caller cannot use the paid route as a free health check — the status alone
//! would otherwise say whether the instrument was going to answer. The instrument
//! runs next, and settlement happens only if it succeeded, so nobody is charged
//! for a call that failed. Settling before returning also means a caller who
//! disconnects has not been given the answer for free.

use std::time::Duration;

use serde_json::{Value, json};

use crate::x402::Config;

/// How long a facilitator has to answer.
///
/// Short. The challenge quotes a sixty-second window for the whole exchange, and
/// a facilitator that needs more than a few seconds for one call has already
/// made the endpoint unusable.
const TIMEOUT: Duration = Duration::from_secs(8);

/// Why a payment was not accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rejected {
    /// The header was not a base64 JSON payload.
    Malformed(String),
    /// The facilitator could not be reached.
    Unreachable(String),
    /// The facilitator answered, but not in a shape this understands.
    ///
    /// Distinct from a rejection: it means the facilitator is speaking a
    /// protocol Radar does not, and treating that as a valid payment would be
    /// reading silence as consent.
    Unreadable(String),
    /// The facilitator said no.
    Invalid(String),
    /// Verification passed but settlement did not.
    NotSettled(String),
}

impl Rejected {
    /// A short reason for the response body.
    #[must_use]
    pub fn reason(&self) -> String {
        match self {
            Self::Malformed(d) => format!("payment header is malformed: {d}"),
            Self::Unreachable(d) => format!("facilitator unreachable: {d}"),
            Self::Unreadable(d) => format!("facilitator answered unreadably: {d}"),
            Self::Invalid(d) => format!("payment rejected: {d}"),
            Self::NotSettled(d) => format!("payment did not settle: {d}"),
        }
    }

    /// Whether the caller could succeed by retrying unchanged.
    ///
    /// Only the transport cases. A rejected payment retried identically is
    /// rejected identically, and telling a caller otherwise wastes their money
    /// as well as their time.
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        matches!(self, Self::Unreachable(_))
    }
}

/// A settled payment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settled {
    /// The settlement transaction, for the response header.
    pub transaction: String,
    /// Who paid, as the facilitator resolved it.
    pub payer: String,
}

/// What Radar requires for one call, in the shape a facilitator expects.
///
/// Built from the instrument's own price. The client's payload is never consulted
/// in constructing this, which is the whole point.
#[must_use]
pub fn requirements(config: &Config, resource: &str, description: &str, price: u64) -> Value {
    json!({
        "scheme": "exact",
        "network": config.network.caip2,
        "asset": config.network.asset,
        "payTo": config.pay_to,
        "maxAmountRequired": price.to_string(),
        "resource": resource,
        "description": description,
        "mimeType": "application/json",
        "maxTimeoutSeconds": 60,
    })
}

/// Decodes the payment header into the payload the facilitator expects.
///
/// x402 sends the payload base64-encoded in a header. It is decoded rather than
/// forwarded verbatim so a header that is not a payment is refused here, before
/// it becomes a request to somebody else's service.
///
/// # Errors
///
/// Returns [`Rejected::Malformed`] if the header is not base64 JSON.
pub fn decode_payload(header: &str) -> Result<Value, Rejected> {
    // Some clients send the JSON directly rather than base64. Accept both: the
    // facilitator validates the contents either way, and refusing a
    // well-formed payment over an encoding preference is a lost sale, not a
    // security property.
    if let Ok(direct) = serde_json::from_str::<Value>(header.trim())
        && direct.is_object()
    {
        return Ok(direct);
    }

    let bytes = radar_types::b64::decode(header.trim())
        .ok_or_else(|| Rejected::Malformed("not base64".to_owned()))?;
    let text = String::from_utf8(bytes).map_err(|_| Rejected::Malformed("not UTF-8".to_owned()))?;
    let parsed: Value =
        serde_json::from_str(&text).map_err(|e| Rejected::Malformed(e.to_string()))?;
    if !parsed.is_object() {
        return Err(Rejected::Malformed("payload is not an object".to_owned()));
    }
    Ok(parsed)
}

/// Reads a facilitator's `/verify` answer.
///
/// Separated from the HTTP call so the rule that matters — anything other than
/// an explicit `true` is a refusal — is testable without a server.
///
/// # Errors
///
/// Returns [`Rejected`] for any answer that is not an explicit approval.
pub fn read_verify(body: &str) -> Result<(), Rejected> {
    let parsed: Value =
        serde_json::from_str(body).map_err(|e| Rejected::Unreadable(e.to_string()))?;

    match parsed.get("isValid").and_then(Value::as_bool) {
        Some(true) => Ok(()),
        Some(false) => Err(Rejected::Invalid(
            parsed
                .get("invalidReason")
                .and_then(Value::as_str)
                .unwrap_or("no reason given")
                .to_owned(),
        )),
        // A missing or non-boolean field is not a "no" — it is a facilitator
        // speaking a protocol this does not understand, and reading that as
        // approval would be reading silence as consent.
        None => Err(Rejected::Unreadable(
            "answer has no boolean `isValid`".to_owned(),
        )),
    }
}

/// Reads a facilitator's `/settle` answer.
///
/// # Errors
///
/// Returns [`Rejected::NotSettled`] unless settlement explicitly succeeded.
pub fn read_settle(body: &str) -> Result<Settled, Rejected> {
    let parsed: Value =
        serde_json::from_str(body).map_err(|e| Rejected::Unreadable(e.to_string()))?;

    if parsed.get("success").and_then(Value::as_bool) != Some(true) {
        return Err(Rejected::NotSettled(
            parsed
                .get("errorReason")
                .or_else(|| parsed.get("error"))
                .and_then(Value::as_str)
                .unwrap_or("settlement did not report success")
                .to_owned(),
        ));
    }

    Ok(Settled {
        transaction: parsed
            .get("transaction")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        payer: parsed
            .get("payer")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
    })
}

/// A payment the facilitator has approved but not yet settled.
///
/// Carries the exact body that was verified, so settlement asks about the same
/// payment against the same requirements. Rebuilding it would be a second chance
/// to build it differently.
#[derive(Debug, Clone)]
pub struct Verified {
    body: Value,
}

/// Asks the facilitator whether this payment is good, blocking.
///
/// Called before the instrument runs, so an unpaid caller cannot learn anything
/// about whether the call would have succeeded.
///
/// Blocking: the HTTP client here is synchronous, and occupying an async executor
/// thread for a network round trip would stall every other request this process
/// is serving.
///
/// # Errors
///
/// Returns [`Rejected`] for anything short of an explicit approval.
pub fn verify(
    config: &Config,
    header: &str,
    resource: &str,
    description: &str,
    price: u64,
) -> Result<Verified, Rejected> {
    let body = json!({
        "x402Version": crate::x402::X402_VERSION,
        "paymentPayload": decode_payload(header)?,
        "paymentRequirements": requirements(config, resource, description, price),
    });

    read_verify(&post(&agent(), &endpoint(config, "verify"), &body)?)?;
    Ok(Verified { body })
}

/// Settles a verified payment, blocking.
///
/// Takes a [`Verified`] rather than a header, so there is no way to settle a
/// payment that was never verified — the type is the proof.
///
/// # Errors
///
/// Returns [`Rejected::NotSettled`] unless settlement explicitly succeeded.
pub fn settle(config: &Config, verified: &Verified) -> Result<Settled, Rejected> {
    read_settle(&post(
        &agent(),
        &endpoint(config, "settle"),
        &verified.body,
    )?)
}

/// The HTTP client used for both calls.
fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .build()
        .into()
}

/// The facilitator's URL for one operation.
fn endpoint(config: &Config, op: &str) -> String {
    format!("{}/{op}", config.facilitator.trim_end_matches('/'))
}

/// One JSON POST, with every failure mode collapsed into a refusal.
fn post(agent: &ureq::Agent, url: &str, body: &Value) -> Result<String, Rejected> {
    let mut response = agent
        .post(url)
        .content_type("application/json")
        .send(body.to_string())
        .map_err(|e| Rejected::Unreachable(e.to_string()))?;
    response
        .body_mut()
        .read_to_string()
        .map_err(|e| Rejected::Unreachable(e.to_string()))
}

#[cfg(test)]
mod tests {
    use crate::x402::Network;

    use super::*;

    fn config() -> Config {
        Config {
            pay_to: "RadarTreasury1111111111111111111111111111111".to_owned(),
            facilitator: "https://facilitator.example/".to_owned(),
            network: Network::solana_usdc(),
            margin_percent: 50,
        }
    }

    #[test]
    fn only_an_explicit_true_is_approval() {
        assert_eq!(read_verify(r#"{"isValid":true}"#), Ok(()));
    }

    #[test]
    fn an_explicit_false_is_a_rejection_carrying_its_reason() {
        assert_eq!(
            read_verify(r#"{"isValid":false,"invalidReason":"insufficient_funds"}"#),
            Err(Rejected::Invalid("insufficient_funds".to_owned()))
        );
    }

    #[test]
    fn a_missing_field_is_not_approval() {
        // The failure mode that matters most: a facilitator speaking a protocol
        // Radar does not understand must not be read as consent.
        assert!(matches!(read_verify("{}"), Err(Rejected::Unreadable(_))));
        assert!(matches!(
            read_verify(r#"{"valid":true}"#),
            Err(Rejected::Unreadable(_))
        ));
        // A string is not a boolean.
        assert!(matches!(
            read_verify(r#"{"isValid":"true"}"#),
            Err(Rejected::Unreadable(_))
        ));
        assert!(matches!(
            read_verify(r#"{"isValid":1}"#),
            Err(Rejected::Unreadable(_))
        ));
    }

    #[test]
    fn garbage_is_not_approval() {
        assert!(read_verify("").is_err());
        assert!(read_verify("<html>502 Bad Gateway</html>").is_err());
        assert!(read_verify("null").is_err());
        assert!(read_verify("true").is_err());
    }

    #[test]
    fn settlement_needs_an_explicit_success() {
        assert!(read_settle(r#"{"success":false}"#).is_err());
        assert!(read_settle("{}").is_err());
        let settled =
            read_settle(r#"{"success":true,"transaction":"5xyz","payer":"Abc"}"#).expect("settled");
        assert_eq!(settled.transaction, "5xyz");
        assert_eq!(settled.payer, "Abc");
    }

    #[test]
    fn a_failed_settlement_carries_its_reason() {
        assert_eq!(
            read_settle(r#"{"success":false,"errorReason":"expired"}"#),
            Err(Rejected::NotSettled("expired".to_owned()))
        );
    }

    #[test]
    fn requirements_come_from_radars_price_not_the_payload() {
        // A facilitator asked to check a payment against the payer's idea of the
        // price would approve every payment ever made.
        let r = requirements(&config(), "/x402/v1/instruments/x", "x", 1_500);
        assert_eq!(r["maxAmountRequired"], "1500");
        assert_eq!(r["payTo"], config().pay_to);
        assert_eq!(r["asset"], Network::solana_usdc().asset);
        assert_eq!(r["scheme"], "exact");
    }

    #[test]
    fn a_base64_payload_decodes() {
        let payload = r#"{"scheme":"exact","payload":{"signature":"abc"}}"#;
        let encoded = radar_types::b64::encode(payload.as_bytes());
        assert_eq!(
            decode_payload(&encoded).expect("decodes")["scheme"],
            "exact"
        );
    }

    #[test]
    fn a_bare_json_payload_also_decodes() {
        // Refusing a well-formed payment over an encoding preference is a lost
        // sale, not a security property — the facilitator validates either way.
        let direct = r#"{"scheme":"exact"}"#;
        assert_eq!(decode_payload(direct).expect("decodes")["scheme"], "exact");
    }

    #[test]
    fn a_header_that_is_not_a_payment_is_refused_before_anyone_else_is_asked() {
        assert!(matches!(
            decode_payload("!!!not base64!!!"),
            Err(Rejected::Malformed(_))
        ));
        assert!(matches!(decode_payload(""), Err(Rejected::Malformed(_))));
        // Valid base64 of something that is not JSON.
        assert!(matches!(
            decode_payload(&radar_types::b64::encode(b"hello")),
            Err(Rejected::Malformed(_))
        ));
        // Valid JSON that is not an object.
        assert!(matches!(
            decode_payload(&radar_types::b64::encode(b"[1,2,3]")),
            Err(Rejected::Malformed(_))
        ));
    }

    #[test]
    fn an_unreachable_facilitator_refuses_rather_than_serving() {
        // The rule the whole module rests on. Nothing reaches a success path
        // without an explicit approval.
        let mut c = config();
        c.facilitator = "http://127.0.0.1:1".to_owned();
        let err = verify(
            &c,
            &radar_types::b64::encode(br#"{"scheme":"exact"}"#),
            "/x402/v1/instruments/x",
            "x",
            1_000,
        )
        .expect_err("nothing is listening");
        assert!(matches!(err, Rejected::Unreachable(_)), "{err:?}");
        assert!(err.is_retryable(), "a transport failure is worth retrying");
    }

    #[test]
    fn a_rejected_payment_is_not_worth_retrying() {
        // Telling a caller to retry an identically-rejected payment wastes their
        // money as well as their time.
        assert!(!Rejected::Invalid("nope".to_owned()).is_retryable());
        assert!(!Rejected::NotSettled("nope".to_owned()).is_retryable());
        assert!(!Rejected::Malformed("nope".to_owned()).is_retryable());
        assert!(!Rejected::Unreadable("nope".to_owned()).is_retryable());
    }

    #[test]
    fn the_facilitator_url_survives_a_trailing_slash() {
        assert_eq!(
            endpoint(&config(), "verify"),
            "https://facilitator.example/verify"
        );
    }
}
