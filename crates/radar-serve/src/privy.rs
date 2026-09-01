// SPDX-License-Identifier: Apache-2.0
//! Reading a customer's wallet from Privy.
//!
//! Reading only. **There is no signing here and there must never be**: the key
//! that authorises Privy to sign lives in `radar-signer`, which is
//! [ADR 0007](https://github.com/hey-vera/radar/blob/main/docs/adr/0007-the-privy-authorization-key-lives-in-the-signer-process.md).
//! What this module holds is the application credential, which authenticates
//! Radar as an application and authorises nothing.
//!
//! # Why the address is fetched rather than stored
//!
//! [ADR 0006](https://github.com/hey-vera/radar/blob/main/docs/adr/0006-radar-records-only-what-it-cannot-recover.md).
//! Privy is authoritative for it, and a cached address that has gone stale is an
//! address Radar might show a customer — or send funds to — that they no longer
//! control. It may be cached with a lifetime; it is never recorded as fact.

use serde::{Deserialize, Serialize};

/// Why a wallet could not be read.
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum Unavailable {
    /// No application credential is configured.
    ///
    /// Rule 8. A customer whose wallet cannot be looked up is not a customer
    /// with no wallet — the two are indistinguishable from here and only one of
    /// them is safe to act on.
    #[error(
        "no Privy application credential: set RADAR_PRIVY_APP_ID and \
         RADAR_PRIVY_APP_SECRET. A wallet that cannot be looked up is not a \
         wallet that is absent."
    )]
    NotConfigured,
    /// Privy could not be reached, or answered something unreadable.
    #[error("Privy did not answer usefully: {0}")]
    Unreachable(String),
    /// Privy answered, and this customer has no Solana embedded wallet.
    ///
    /// A real answer rather than a failure: a customer who has just signed up
    /// legitimately has none yet.
    #[error("this customer has no Solana embedded wallet")]
    NoWallet,
}

/// The application credential.
///
/// Authenticates Radar as an application. It authorises no signature — that
/// needs the authorization key, which is deliberately in another process.
#[derive(Clone, PartialEq, Eq)]
pub struct Credentials {
    app_id: String,
    secret: String,
}

impl std::fmt::Debug for Credentials {
    /// Prints the application id and never the secret.
    ///
    /// A `Debug` derive here would put the secret into every error message that
    /// formatted a struct containing it, and those reach logs. The id is safe
    /// and is the half an operator needs to identify which application this is.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("app_id", &self.app_id)
            .field("secret", &"<redacted>")
            .finish()
    }
}

impl Credentials {
    /// Reads the credential from the environment.
    ///
    /// Both halves or neither. A configuration with an id and no secret is a
    /// half-finished deployment, and treating it as "unconfigured" would hide
    /// the mistake behind a message about a missing feature.
    ///
    /// # Errors
    ///
    /// [`Unavailable::NotConfigured`] when either half is missing.
    pub fn from_vars(get: &impl Fn(&str) -> Option<String>) -> Result<Self, Unavailable> {
        let read = |key: &str| {
            get(key)
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
        };
        match (read("RADAR_PRIVY_APP_ID"), read("RADAR_PRIVY_APP_SECRET")) {
            (Some(app_id), Some(secret)) => Ok(Self { app_id, secret }),
            _ => Err(Unavailable::NotConfigured),
        }
    }

    /// Builds one directly, for tests and for a caller that already has both.
    #[must_use]
    pub fn new(app_id: impl Into<String>, secret: impl Into<String>) -> Self {
        Self {
            app_id: app_id.into(),
            secret: secret.into(),
        }
    }

    /// The application id, which is not secret.
    #[must_use]
    pub fn app_id(&self) -> &str {
        &self.app_id
    }

    /// The `Authorization` header value: Basic, app id as user, secret as
    /// password.
    #[must_use]
    pub fn basic(&self) -> String {
        format!(
            "Basic {}",
            radar_types::b64::encode(format!("{}:{}", self.app_id, self.secret).as_bytes())
        )
    }
}

/// A customer's Solana embedded wallet, as Privy reports it.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Wallet {
    /// The Solana address, base58.
    pub address: String,
    /// Privy's identifier for it, which the signing endpoint is keyed by.
    pub id: String,
    /// Whether the customer has granted Radar a signer on this wallet.
    ///
    /// # Rule 9 lives here
    ///
    /// Absent is **not** permission. Privy omits this field in some responses,
    /// and a missing value means Radar has not been shown a grant — which is the
    /// same thing as not having one, as far as anything that spends money is
    /// concerned.
    ///
    /// So this is `false` when the field is missing, and a caller that wants to
    /// act must check it. Reading a missing grant as permission is the shape of
    /// mistake that ends with somebody's wallet being traded without consent.
    #[serde(default)]
    pub delegated: bool,
}

/// How a request reaches Privy.
///
/// A seam, so the parsing and the rule-9 handling can be tested without a
/// network — and so the tests exercise **real Privy response bodies** rather
/// than a mock of Radar's own understanding of them.
pub trait Transport: Send + Sync {
    /// Performs a `GET`, returning the body.
    ///
    /// # Errors
    ///
    /// A message suitable for [`Unavailable::Unreachable`].
    fn get(&self, url: &str, credentials: &Credentials) -> Result<String, String>;
}

/// The real transport.
#[derive(Debug, Default)]
pub struct Https;

/// How long a call to Privy may take before it is a failure.
///
/// Ten seconds, matching the key fetch. A customer-facing lookup that hangs is
/// worse than one that fails: the failure is visible and retryable.
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

impl Transport for Https {
    fn get(&self, url: &str, credentials: &Credentials) -> Result<String, String> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .build()
            .into();
        agent
            .get(url)
            .header("authorization", &credentials.basic())
            .header("privy-app-id", credentials.app_id())
            .call()
            .map_err(|e| e.to_string())?
            .body_mut()
            .read_to_string()
            .map_err(|e| e.to_string())
    }
}

/// Privy's API root.
const API: &str = "https://api.privy.io/v1";

/// Reads customers' wallets.
pub struct Client {
    credentials: Credentials,
    transport: Box<dyn Transport>,
}

impl Client {
    /// A client over the real network.
    #[must_use]
    pub fn new(credentials: Credentials) -> Self {
        Self {
            credentials,
            transport: Box::new(Https),
        }
    }

    /// A client over a supplied transport.
    #[must_use]
    pub fn with_transport(credentials: Credentials, transport: Box<dyn Transport>) -> Self {
        Self {
            credentials,
            transport,
        }
    }

    /// The Solana embedded wallet belonging to a Privy DID.
    ///
    /// The DID comes from a **verified** access token and nowhere else. A DID
    /// taken from a request body would let any caller read any customer's
    /// wallet.
    ///
    /// # Errors
    ///
    /// [`Unavailable`] when Privy cannot be reached, answers unreadably, or
    /// reports no Solana embedded wallet. Never an empty success.
    pub fn wallet_for(&self, did: &str) -> Result<Wallet, Unavailable> {
        let body = self
            .transport
            .get(&format!("{API}/users/{did}"), &self.credentials)
            .map_err(Unavailable::Unreachable)?;
        let user: serde_json::Value =
            serde_json::from_str(&body).map_err(|e| Unavailable::Unreachable(e.to_string()))?;
        solana_wallet(&user).ok_or(Unavailable::NoWallet)
    }
}

/// Picks the Solana embedded wallet out of a Privy user object.
///
/// Three conditions, all required. A customer may have linked an external
/// Phantom wallet, an Ethereum embedded wallet, or several of each, and Radar
/// may only act on one it can actually be a signer for:
///
/// - `type == "wallet"` — the account is a wallet at all,
/// - `chain_type == "solana"` — the right chain, and
/// - `connector_type == "embedded"` — Privy holds it, rather than it being an
///   external wallet the customer connected.
///
/// Anything less specific risks returning a **Phantom address the customer
/// connected for login**, which Radar can never sign for and must never be shown
/// as a deposit destination.
fn solana_wallet(user: &serde_json::Value) -> Option<Wallet> {
    user.get("linked_accounts")?
        .as_array()?
        .iter()
        .find(|account| {
            account.get("type").and_then(serde_json::Value::as_str) == Some("wallet")
                && account
                    .get("chain_type")
                    .and_then(serde_json::Value::as_str)
                    == Some("solana")
                && account
                    .get("connector_type")
                    .and_then(serde_json::Value::as_str)
                    == Some("embedded")
        })
        .and_then(|account| {
            Some(Wallet {
                address: account.get("address")?.as_str()?.to_owned(),
                id: account.get("id")?.as_str()?.to_owned(),
                // Rule 9. A field Privy did not send is not a grant.
                delegated: account
                    .get("delegated")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
            })
        })
}
