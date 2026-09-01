// SPDX-License-Identifier: Apache-2.0
//! Building the transaction.
//!
//! Jupiter's `/swap` endpoint returns a complete transaction, which is the
//! practical route — but by default it returns a **versioned transaction using
//! address lookup tables**, and [`radar_signer`] refuses those on purpose: a
//! lookup table names accounts the signer cannot see in the bytes it signs.
//!
//! Two ways out, and Radar takes both:
//!
//! - `asLegacyTransaction=true`, which Jupiter supports. Every account is named
//!   inline, so the signer can read the whole transaction. The cost is that
//!   routes needing more accounts than a legacy message holds become
//!   unavailable. That is a real restriction and an acceptable one: a route the
//!   signer cannot read is a route nothing can check, and the tokens Radar
//!   trades are early ones whose routes are short.
//! - For pre-graduation pump.fun tokens, go direct to the bonding curve, where
//!   the account set is fixed and known and no router is involved at all.
//!
//! See ADR 0003.

use radar_types::{Address, MicroUsd};
use serde::Deserialize;

/// Jupiter's swap-building endpoint.
pub const SWAP_API: &str = "https://lite-api.jup.ag/swap/v1/swap";

/// Jupiter's quote endpoint.
pub const QUOTE_API: &str = "https://lite-api.jup.ag/swap/v1/quote";

/// Wrapped SOL.
pub const WSOL: &str = "So11111111111111111111111111111111111111112";

/// Why a route could not be built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RouteError {
    /// Nothing will trade this token at this size.
    #[error("no route for {mint} at {size_lamports} lamports")]
    NoRoute {
        /// The token.
        mint: String,
        /// The size attempted.
        size_lamports: u64,
    },
    /// The router could not be reached, or answered with an error.
    #[error("router unavailable: {0}")]
    Unavailable(String),
    /// The router's answer did not have the shape expected.
    #[error("unreadable router response: {0}")]
    Malformed(String),
    /// The router returned a transaction the signer cannot verify.
    ///
    /// Not a transport failure — a refusal. Submitting it would mean signing
    /// bytes nothing checked.
    #[error("router returned a transaction the signer cannot read: {0}")]
    Unverifiable(String),
}

/// A transaction ready to be sent to the signer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    /// The unsigned transaction, base64.
    pub transaction: String,
    /// What the router expects out, in the output mint's base units.
    pub expected_out: u64,
    /// The price impact the router reported, in basis points.
    pub impact_bps: u32,
    /// The venues the route passes through, for the audit record.
    pub venues: Vec<String>,
}

/// What Jupiter's quote endpoint returns, in the parts we use.
#[derive(Debug, Deserialize)]
struct Quote {
    #[serde(rename = "outAmount")]
    out_amount: String,
    #[serde(rename = "priceImpactPct")]
    price_impact_pct: Option<String>,
    #[serde(rename = "routePlan", default)]
    route_plan: Vec<RouteStep>,
}

#[derive(Debug, Deserialize)]
struct RouteStep {
    #[serde(rename = "swapInfo")]
    swap_info: SwapInfo,
}

#[derive(Debug, Deserialize)]
struct SwapInfo {
    label: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SwapResponse {
    #[serde(rename = "swapTransaction")]
    swap_transaction: String,
}

/// Builds swap transactions through Jupiter.
pub struct Router {
    quote_endpoint: String,
    swap_endpoint: String,
    agent: ureq::Agent,
    slippage_bps: u32,
}

impl Default for Router {
    fn default() -> Self {
        Self::new(QUOTE_API, SWAP_API)
    }
}

impl Router {
    /// A router against the given endpoints.
    #[must_use]
    pub fn new(quote_endpoint: impl Into<String>, swap_endpoint: impl Into<String>) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(15)))
            .build();
        Self {
            quote_endpoint: quote_endpoint.into(),
            swap_endpoint: swap_endpoint.into(),
            agent: config.into(),
            // Tight. A wide setting is what makes a swap worth sandwiching, and
            // memecoin swaps are still being sandwiched several times a minute.
            slippage_bps: 100,
        }
    }

    /// Slippage tolerance, in basis points.
    #[must_use]
    pub const fn with_slippage_bps(mut self, bps: u32) -> Self {
        self.slippage_bps = bps;
        self
    }

    /// Builds a buy: SOL in, `mint` out.
    ///
    /// # Errors
    ///
    /// Returns [`RouteError`] if no route exists, the router is unreachable, or
    /// the transaction it returns cannot be verified.
    pub fn build_buy(
        &self,
        mint: &Address,
        wallet: &Address,
        size_lamports: u64,
    ) -> Result<Route, RouteError> {
        self.build(WSOL, &mint.to_string(), wallet, size_lamports)
    }

    /// Builds a sell: `mint` in, SOL out.
    ///
    /// # Errors
    ///
    /// Returns [`RouteError`] as for [`Self::build_buy`].
    pub fn build_sell(
        &self,
        mint: &Address,
        wallet: &Address,
        size_tokens: u64,
    ) -> Result<Route, RouteError> {
        self.build(&mint.to_string(), WSOL, wallet, size_tokens)
    }

    fn build(
        &self,
        input_mint: &str,
        output_mint: &str,
        wallet: &Address,
        amount: u64,
    ) -> Result<Route, RouteError> {
        let (quote, raw) = self.quote(input_mint, output_mint, amount)?;
        let expected_out = quote
            .out_amount
            .parse::<u64>()
            .map_err(|_| RouteError::Malformed(format!("bad outAmount: {}", quote.out_amount)))?;
        if expected_out == 0 {
            return Err(RouteError::NoRoute {
                mint: output_mint.to_owned(),
                size_lamports: amount,
            });
        }

        let venues = quote
            .route_plan
            .iter()
            .filter_map(|s| s.swap_info.label.clone())
            .collect();
        let impact_bps = impact_to_bps(quote.price_impact_pct.as_deref());

        let transaction = self.swap(&raw, wallet)?;
        verify_shape(&transaction)?;

        Ok(Route {
            transaction,
            expected_out,
            impact_bps,
            venues,
        })
    }

    /// Fetches a quote, returning both the parsed form and the raw JSON.
    ///
    /// Both, because Jupiter's `/swap` needs the quote echoed back *verbatim* —
    /// including fields this crate does not model. Re-serialising a parsed quote
    /// would silently drop them, and the transaction Jupiter then built would be
    /// for a different route from the one that was priced.
    fn quote(
        &self,
        input: &str,
        output: &str,
        amount: u64,
    ) -> Result<(Quote, serde_json::Value), RouteError> {
        let response = self
            .agent
            .get(&self.quote_endpoint)
            .query("inputMint", input)
            .query("outputMint", output)
            .query("amount", amount.to_string())
            .query("slippageBps", self.slippage_bps.to_string())
            // The whole reason this crate can hand a transaction to the signer.
            .query("asLegacyTransaction", "true")
            .call();

        let mut response = match response {
            Ok(r) => r,
            Err(ureq::Error::StatusCode(code)) if (400..500).contains(&code) => {
                return Err(RouteError::NoRoute {
                    mint: output.to_owned(),
                    size_lamports: amount,
                });
            }
            Err(e) => return Err(RouteError::Unavailable(e.to_string())),
        };

        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|e| RouteError::Unavailable(e.to_string()))?;
        let parsed: Quote =
            serde_json::from_str(&body).map_err(|e| RouteError::Malformed(e.to_string()))?;
        let raw: serde_json::Value =
            serde_json::from_str(&body).map_err(|e| RouteError::Malformed(e.to_string()))?;
        Ok((parsed, raw))
    }

    fn swap(&self, quote: &serde_json::Value, wallet: &Address) -> Result<String, RouteError> {
        let body = serde_json::json!({
            "quoteResponse": quote,
            "userPublicKey": wallet.to_string(),
            "asLegacyTransaction": true,
            "wrapAndUnwrapSol": true,
        });

        let mut response = self
            .agent
            .post(&self.swap_endpoint)
            .content_type("application/json")
            .send(body.to_string())
            .map_err(|e| RouteError::Unavailable(e.to_string()))?;

        let text = response
            .body_mut()
            .read_to_string()
            .map_err(|e| RouteError::Unavailable(e.to_string()))?;
        let parsed: SwapResponse =
            serde_json::from_str(&text).map_err(|e| RouteError::Malformed(e.to_string()))?;
        Ok(parsed.swap_transaction)
    }
}

/// Converts a percentage string to basis points, treating unreadable as maximal.
#[must_use]
pub fn impact_to_bps(pct: Option<&str>) -> u32 {
    pct.map_or(u32::MAX, |raw| {
        raw.parse::<f64>().map_or(u32::MAX, |fraction| {
            let bps = (fraction.abs() * 10_000.0).round();
            if bps.is_finite() && bps <= f64::from(u32::MAX) {
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "bounded and non-negative by the checks above"
                )]
                let bps = bps as u32;
                bps
            } else {
                u32::MAX
            }
        })
    })
}

/// Checks the router's transaction is one the signer will accept.
///
/// Done here rather than left to the signer so a route that cannot be signed is
/// discarded before a decision is built on it. The signer still checks — this is
/// an early exit, never a substitute.
///
/// # Errors
///
/// Returns [`RouteError::Unverifiable`] if the bytes do not decode, or use
/// address lookup tables.
pub fn verify_shape(transaction_base64: &str) -> Result<(), RouteError> {
    let bytes = radar_types::b64::decode(transaction_base64)
        .ok_or_else(|| RouteError::Unverifiable("not base64".to_owned()))?;
    radar_signer::decode(&bytes).map_err(|e| RouteError::Unverifiable(e.to_string()))?;
    Ok(())
}

/// The lamports a notional is worth at a given SOL price.
#[must_use]
pub fn notional_to_lamports(notional: MicroUsd, sol_price: MicroUsd) -> u64 {
    if sol_price.get() == 0 {
        return 0;
    }
    let product = u128::from(notional.get()) * 1_000_000_000u128;
    u64::try_from(product / u128::from(sol_price.get())).unwrap_or(u64::MAX)
}

/// The router, as the pipeline sees it.
///
/// An adapter and nothing more — the signature already matches. It exists
/// because until 2026-09-01 `Routing` had **no implementation outside a test
/// stub**, so the pipeline could only ever be exercised against a fixture.
///
/// That is the gap [LEARNINGS](https://github.com/hey-vera/radar/blob/main/LEARNINGS.md)
/// 10 is about: a lane whose every stage is tested against something the real
/// one would never produce. `Router` builds a transaction from Jupiter; this
/// makes the executor able to ask it for one.
impl crate::pipeline::Routing for Router {
    fn build_buy(
        &self,
        mint: &Address,
        wallet: &Address,
        size_lamports: u64,
    ) -> Result<Route, RouteError> {
        Self::build_buy(self, mint, wallet, size_lamports)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impact_parses_the_shapes_jupiter_returns() {
        assert_eq!(impact_to_bps(Some("0")), 0);
        assert_eq!(impact_to_bps(Some("0.05")), 500);
        assert_eq!(impact_to_bps(Some("-0.01")), 100);
    }

    #[test]
    fn an_unreadable_impact_is_maximal_rather_than_zero() {
        assert_eq!(impact_to_bps(None), u32::MAX);
        assert_eq!(impact_to_bps(Some("nonsense")), u32::MAX);
    }

    #[test]
    fn a_legacy_transaction_passes_the_shape_check() {
        let mut bytes = vec![1u8];
        bytes.extend_from_slice(&[0u8; 64]);
        bytes.extend_from_slice(&[1, 0, 0, 2]);
        bytes.extend_from_slice(&[0u8; 32]);
        bytes.extend_from_slice(&[1u8; 32]);
        bytes.extend_from_slice(&[0xAA; 32]);
        bytes.push(0);
        assert_eq!(verify_shape(&radar_types::b64::encode(&bytes)), Ok(()));
    }

    #[test]
    fn a_transaction_with_lookup_tables_is_rejected_before_a_decision_rests_on_it() {
        // The reason this crate asks for asLegacyTransaction at all. Discovering
        // it at the signer would mean a decision was already built on a route
        // that can never be executed.
        let mut bytes = vec![0u8, 0x80, 1, 0, 0, 2];
        bytes.extend_from_slice(&[0u8; 32]);
        bytes.extend_from_slice(&[1u8; 32]);
        bytes.extend_from_slice(&[0xAA; 32]);
        bytes.push(0);
        bytes.push(1);
        let err = verify_shape(&radar_types::b64::encode(&bytes)).expect_err("must refuse");
        assert!(
            matches!(err, RouteError::Unverifiable(ref m) if m.contains("lookup")),
            "got {err}"
        );
    }

    #[test]
    fn garbage_from_the_router_is_a_refusal_not_a_panic() {
        assert!(verify_shape("!!!!").is_err());
        assert!(verify_shape("QUJD").is_err());
        assert!(verify_shape("").is_err());
    }

    #[test]
    fn notional_converts_to_lamports_in_integers() {
        let sol = MicroUsd::from_dollars(200.0);
        assert_eq!(
            notional_to_lamports(MicroUsd::from_dollars(200.0), sol),
            1_000_000_000
        );
        assert_eq!(
            notional_to_lamports(MicroUsd::from_dollars(2.0), sol),
            10_000_000
        );
    }

    #[test]
    fn an_unknown_price_sizes_at_nothing_rather_than_at_everything() {
        // Dividing by an absent price is the arithmetic that turns a missing
        // input into an unbounded position.
        assert_eq!(
            notional_to_lamports(MicroUsd::from_dollars(100.0), MicroUsd::ZERO),
            0
        );
    }
}
