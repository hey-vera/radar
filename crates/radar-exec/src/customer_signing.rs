// SPDX-License-Identifier: Apache-2.0
//! Signing on a customer's wallet, through Privy.
//!
//! A [`Signing`] implementation, so it drops into the existing pipeline without
//! the executor learning anything about custody. The stage order is unchanged:
//! route, gate, sign, submit.
//!
//! # The three-party shape, and why it is this way
//!
//! Three processes are involved and each holds exactly one thing:
//!
//! | who | holds | can |
//! |---|---|---|
//! | this process | the application credential | ask; authorise nothing |
//! | `radar-signer` | the P-256 authorization key | authorise one checked request |
//! | Privy | the wallet key | sign what a valid authorisation asks for |
//!
//! No single compromise signs a transaction the kernel did not approve, which is
//! [ADR 0007](https://github.com/hey-vera/radar/blob/main/docs/adr/0007-the-privy-authorization-key-lives-in-the-signer-process.md)
//! and [ADR 0008](https://github.com/hey-vera/radar/blob/main/docs/adr/0008-the-signer-holds-its-own-policy.md)
//! working together.
//!
//! # Why Privy signs but does not send
//!
//! Privy offers `signAndSendTransaction`. This uses `signTransaction` and sends
//! through Radar's own RPC, which is rule 7: the send path stays direct. It also
//! keeps the shape identical to the local lane, where the signer returns a
//! submittable transaction and [`Sending`](crate::pipeline::Sending) is a
//! separate stage that can be swapped, retried and measured.
//!
//! # What cannot go wrong quietly
//!
//! The request sent to Privy must be **byte-identical** to the one the signer
//! authorised, because the signature covers a canonicalisation of it. A caller
//! that altered the body after signing produces a signature Privy rejects. That
//! is a failure, and it is the safe kind: it fails closed, loudly, on the
//! vendor's side, rather than signing something nobody checked.

use radar_customer::Meter;
use radar_risk::Authorization;
use serde_json::json;

use crate::pipeline::Signing;

/// Why a customer signature could not be obtained.
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum NotSigned {
    /// The local signer refused, for these reasons.
    #[error("{}", .0.join("; "))]
    SignerRefused(Vec<String>),
    /// The customer's daily signature allowance is spent.
    ///
    /// A refusal rather than a delay. The meter exists because an unbounded
    /// signer is what invariant 1 prevents, and "just this once" is how a
    /// ceiling stops being one.
    #[error("this customer's daily signature allowance is spent")]
    AllowanceSpent,
    /// Privy could not be reached, or answered unusably.
    #[error("Privy did not sign: {0}")]
    PrivyRefused(String),
}

/// How a signing request reaches Privy.
///
/// A seam, so the ordering can be exercised without a network or a customer.
pub trait PrivyTransport: Send + Sync {
    /// `POST`s a body with an authorization signature, returning the response
    /// body.
    ///
    /// The body is passed as the exact string that was signed. Handing over a
    /// structured value and re-serialising here would be the one change that
    /// silently breaks every signature, because the signature covers the bytes.
    ///
    /// # Errors
    ///
    /// A message suitable for [`NotSigned::PrivyRefused`].
    fn post(&self, url: &str, body: &str, authorization_signature: &str) -> Result<String, String>;
}

/// How an authorization signature is obtained from the signer process.
///
/// Separate from [`Signing`] because it answers a different question: not "sign
/// this transaction" but "authorise this request". The signer's Privy mode
/// returns a header value, not a transaction, and collapsing the two would let a
/// caller treat an unsent authorisation as a completed signature.
pub trait Authorising: Send + Sync {
    /// Asks the signer to authorise a Privy request.
    ///
    /// # Errors
    ///
    /// The signer's refusal reasons, verbatim.
    fn authorise(
        &self,
        authorization: &Authorization,
        request: &serde_json::Value,
        wallet: &str,
    ) -> Result<String, Vec<String>>;
}

/// Signs on one customer's wallet.
pub struct CustomerSigner<'a> {
    /// Privy's identifier for the wallet, which keys the RPC endpoint.
    wallet_id: String,
    /// The wallet's address, which the signer checks the fee payer against.
    wallet_address: String,
    /// The application id, which is part of the signed payload.
    app_id: String,
    signer: &'a dyn Authorising,
    privy: &'a dyn PrivyTransport,
    /// The customer's signature meter.
    ///
    /// Behind a mutex because charging it mutates, and a pipeline that signed
    /// two transactions at once must not let both see the same remaining
    /// allowance.
    meter: &'a std::sync::Mutex<Meter>,
}

impl<'a> CustomerSigner<'a> {
    /// Builds one for a customer's wallet.
    #[must_use]
    pub fn new(
        wallet_id: impl Into<String>,
        wallet_address: impl Into<String>,
        app_id: impl Into<String>,
        signer: &'a dyn Authorising,
        privy: &'a dyn PrivyTransport,
        meter: &'a std::sync::Mutex<Meter>,
    ) -> Self {
        Self {
            wallet_id: wallet_id.into(),
            wallet_address: wallet_address.into(),
            app_id: app_id.into(),
            signer,
            privy,
            meter,
        }
    }

    /// The endpoint for this wallet.
    fn url(&self) -> String {
        format!("https://api.privy.io/v1/wallets/{}/rpc", self.wallet_id)
    }

    /// The request, built once and used for both signing and sending.
    ///
    /// Built once on purpose. Two constructions of "the same" request are two
    /// chances to differ, and the difference would be invisible here and fatal
    /// at Privy.
    fn request(&self, transaction: &str) -> serde_json::Value {
        json!({
            "method": "POST",
            "url": self.url(),
            "body": {
                "method": "signTransaction",
                "params": {"transaction": transaction, "encoding": "base64"},
            },
            "headers": {"privy-app-id": self.app_id},
        })
    }
}

impl Signing for CustomerSigner<'_> {
    fn sign(
        &self,
        authorization: &Authorization,
        transaction: &str,
    ) -> Result<String, Vec<String>> {
        self.sign_through_privy(authorization, transaction)
            .map_err(|why| vec![why.to_string()])
    }
}

impl CustomerSigner<'_> {
    /// The real body, with a typed error the trait then flattens.
    fn sign_through_privy(
        &self,
        authorization: &Authorization,
        transaction: &str,
    ) -> Result<String, NotSigned> {
        // Charged before the signature is asked for, not after.
        //
        // A process that dies mid-call cannot know whether Privy signed, and
        // counting afterwards undercounts exactly the calls that went wrong --
        // which is what a runaway loop is made of. The conservative direction is
        // to have paid for a signature that never happened.
        self.meter
            .lock()
            .map_err(|_| NotSigned::PrivyRefused("the signature meter is poisoned".to_owned()))?
            .charge()
            .map_err(|_| NotSigned::AllowanceSpent)?;

        let request = self.request(transaction);
        let signature = self
            .signer
            .authorise(authorization, &request, &self.wallet_address)
            .map_err(NotSigned::SignerRefused)?;

        // The exact bytes the signer canonicalised. Re-serialising the body here
        // would produce a different string with the same meaning, and Privy
        // verifies the bytes.
        let body = serde_json::to_string(&request["body"])
            .map_err(|e| NotSigned::PrivyRefused(e.to_string()))?;

        let response = self
            .privy
            .post(&self.url(), &body, &signature)
            .map_err(NotSigned::PrivyRefused)?;

        let parsed: serde_json::Value = serde_json::from_str(&response)
            .map_err(|e| NotSigned::PrivyRefused(format!("unreadable answer: {e}")))?;

        parsed
            .get("data")
            .and_then(|d| d.get("signed_transaction"))
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned)
            .ok_or_else(|| {
                // An answer with no signed transaction is a failure, never a
                // pass. Privy reports errors in the same envelope as successes.
                NotSigned::PrivyRefused(format!(
                    "no signed transaction in the answer: {}",
                    parsed.get("error").unwrap_or(&parsed)
                ))
            })
    }
}
