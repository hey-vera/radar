// SPDX-License-Identifier: Apache-2.0
//! Sending a signed transaction, and finding out what happened to it.
//!
//! **Direct RPC only.** `getLatestBlockhash`, pre-trade `simulateTransaction`
//! and `sendTransaction` never go through the x402 lane: settlement adds
//! 400–800ms before a response returns, which is fine for analysis and fatal
//! here. That is rule 7 in `AGENTS.md`, and it is enforced by this module taking
//! an endpoint URL rather than a provider handle — there is no way to pass it
//! the metered lane.

use std::time::Duration;

use radar_types::{Signature, Slot};
use serde::Deserialize;

/// A public Solana RPC. Replace with a paid one for execution.
pub const DEFAULT_RPC: &str = "https://api.mainnet-beta.solana.com";

/// Why a submission failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SubmitError {
    /// The endpoint could not be reached.
    #[error("rpc unreachable: {0}")]
    Transport(String),
    /// The node rejected the transaction.
    ///
    /// Distinct from a transport failure because it is terminal: retrying an
    /// identical transaction the node already refused will be refused again,
    /// and a retry loop over a rejection is how a fee gets paid repeatedly for
    /// nothing.
    #[error("node rejected: {0}")]
    Rejected(String),
    /// The response could not be read.
    #[error("unreadable response: {0}")]
    Malformed(String),
}

/// How settled a transaction is.
///
/// An abstraction rather than a slot count on purpose. Alpenglow lands around
/// October 2026 and takes finality from ~12.8 seconds to ~150 milliseconds, so
/// anything that hard-codes 32 slots will be wrong shortly after it is written.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Finality {
    /// Not seen yet.
    Unknown,
    /// In a block, not yet voted on.
    Processed,
    /// Voted on by a supermajority.
    Confirmed,
    /// Rooted.
    Finalized,
    /// Landed, and failed on chain.
    ///
    /// A landed failure still costs its fee, which is why the economics gate
    /// carries a failure term at all.
    Failed {
        /// What the runtime said.
        error: String,
    },
}

impl Finality {
    /// Whether a position may be considered open on the strength of this.
    ///
    /// `Processed` deliberately does not qualify. A processed transaction can
    /// still be dropped on a fork, and a position believed open that is not is
    /// the state from which a strategy sells something it does not hold.
    #[must_use]
    pub const fn is_settled(&self) -> bool {
        matches!(self, Self::Confirmed | Self::Finalized)
    }

    /// Whether waiting longer could change this.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(self, Self::Finalized | Self::Failed { .. })
    }
}

/// Sends transactions to a direct RPC.
pub struct Submitter {
    endpoint: String,
    agent: ureq::Agent,
}

impl Default for Submitter {
    fn default() -> Self {
        Self::new(DEFAULT_RPC)
    }
}

impl Submitter {
    /// A submitter against the given endpoint.
    ///
    /// Takes a URL, not a provider handle. There is deliberately no constructor
    /// that accepts the metered x402 lane.
    #[must_use]
    pub fn new(endpoint: impl Into<String>) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(10)))
            .build();
        Self {
            endpoint: endpoint.into(),
            agent: config.into(),
        }
    }

    /// Sends a signed transaction.
    ///
    /// `skip_preflight` is on: preflight simulation costs a round trip against a
    /// state that has already moved, and the pre-trade simulation happened
    /// before the decision. Its usual benefit — catching a malformed
    /// transaction — is already covered by the signer refusing to sign one.
    ///
    /// # Errors
    ///
    /// Returns [`SubmitError`] if the node is unreachable, rejects the
    /// transaction, or answers unreadably.
    pub fn send(&self, transaction_base64: &str) -> Result<Signature, SubmitError> {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "sendTransaction",
            "params": [transaction_base64, {
                "encoding": "base64",
                "skipPreflight": true,
                "maxRetries": 0,
            }],
        });

        let text = self.post(&body)?;
        let envelope: SendEnvelope =
            serde_json::from_str(&text).map_err(|e| SubmitError::Malformed(e.to_string()))?;

        if let Some(error) = envelope.error {
            return Err(SubmitError::Rejected(error.message));
        }
        let raw = envelope
            .result
            .ok_or_else(|| SubmitError::Malformed("no signature returned".to_owned()))?;
        raw.parse::<Signature>()
            .map_err(|_| SubmitError::Malformed(format!("unreadable signature: {raw}")))
    }

    /// Asks what happened to a transaction.
    ///
    /// # Errors
    ///
    /// Returns [`SubmitError`] if the node is unreachable or answers unreadably.
    pub fn status(&self, signature: &Signature) -> Result<(Finality, Option<Slot>), SubmitError> {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getSignatureStatuses",
            "params": [[signature.to_string()], { "searchTransactionHistory": true }],
        });

        let text = self.post(&body)?;
        let envelope: StatusEnvelope =
            serde_json::from_str(&text).map_err(|e| SubmitError::Malformed(e.to_string()))?;

        let Some(status) = envelope
            .result
            .and_then(|r| r.value.into_iter().next().flatten())
        else {
            return Ok((Finality::Unknown, None));
        };

        if let Some(error) = status.err {
            return Ok((
                Finality::Failed {
                    error: error.to_string(),
                },
                Some(Slot(status.slot)),
            ));
        }

        let finality = match status.confirmation_status.as_deref() {
            Some("finalized") => Finality::Finalized,
            Some("confirmed") => Finality::Confirmed,
            Some("processed") => Finality::Processed,
            _ => Finality::Unknown,
        };
        Ok((finality, Some(Slot(status.slot))))
    }

    fn post(&self, body: &serde_json::Value) -> Result<String, SubmitError> {
        let mut response = self
            .agent
            .post(&self.endpoint)
            .content_type("application/json")
            .send(body.to_string())
            .map_err(|e| SubmitError::Transport(e.to_string()))?;
        response
            .body_mut()
            .read_to_string()
            .map_err(|e| SubmitError::Transport(e.to_string()))
    }
}

#[derive(Deserialize)]
struct SendEnvelope {
    result: Option<String>,
    error: Option<NodeError>,
}

#[derive(Deserialize)]
struct NodeError {
    message: String,
}

#[derive(Deserialize)]
struct StatusEnvelope {
    result: Option<StatusResult>,
}

#[derive(Deserialize)]
struct StatusResult {
    value: Vec<Option<SignatureStatus>>,
}

#[derive(Deserialize)]
struct SignatureStatus {
    slot: u64,
    err: Option<serde_json::Value>,
    #[serde(rename = "confirmationStatus")]
    confirmation_status: Option<String>,
}

/// The submitter, as the pipeline sees it.
///
/// The error is flattened to a string because [`Sending`](crate::pipeline::Sending)
/// carries whatever the node said, verbatim, rather than a type this crate has
/// pre-judged. A node's own words are what an operator needs at three in the
/// morning; a category this code chose for it is not.
impl crate::pipeline::Sending for Submitter {
    fn send(&self, transaction: &str) -> Result<Signature, String> {
        Self::send(self, transaction).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_processed_transaction_does_not_count_as_settled() {
        // A processed transaction can still be dropped on a fork. Believing a
        // position open that is not is the state from which a strategy sells
        // something it does not hold.
        assert!(!Finality::Processed.is_settled());
        assert!(Finality::Confirmed.is_settled());
        assert!(Finality::Finalized.is_settled());
    }

    #[test]
    fn a_landed_failure_is_not_settled_and_is_terminal() {
        // It cost its fee and it changed nothing. Both facts matter: one to the
        // cost model, the other to the position tracker.
        let failed = Finality::Failed {
            error: "InstructionError".to_owned(),
        };
        assert!(!failed.is_settled());
        assert!(failed.is_terminal());
    }

    #[test]
    fn unknown_and_processed_are_worth_waiting_on() {
        assert!(!Finality::Unknown.is_terminal());
        assert!(!Finality::Processed.is_terminal());
        assert!(!Finality::Confirmed.is_terminal());
    }

    #[test]
    fn an_unreachable_node_is_a_transport_failure_not_a_rejection() {
        // The distinction decides whether retrying is sane. Retrying a rejection
        // pays the fee again for the same answer.
        let submitter = Submitter::new("http://127.0.0.1:1/nothing-here");
        let err = submitter.send("QUJD").expect_err("nothing is listening");
        assert!(matches!(err, SubmitError::Transport(_)), "got {err}");
    }

    #[test]
    fn status_of_an_unreachable_node_is_an_error_not_an_unknown() {
        // Reporting Unknown here would let a network outage read as "the
        // transaction has not landed yet", and the caller would wait forever.
        let submitter = Submitter::new("http://127.0.0.1:1/nothing-here");
        assert!(submitter.status(&Signature::new([0u8; 64])).is_err());
    }
}
