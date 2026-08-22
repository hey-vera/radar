// SPDX-License-Identifier: Apache-2.0
//! The x402 paywall.
//!
//! Radar's public surface is priced per call from the instrument registry's cost
//! model, so the price a caller is quoted and the cost Radar actually incurs come
//! from the same declaration and cannot drift apart.
//!
//! **The paid surface is off unless it is configured.** Without a receiving
//! address and a facilitator, the public routes do not exist — they are not
//! served unpaid, and they are not served on trust. Accepting a payment that
//! cannot be verified is worse than refusing the request, and a paywall that
//! fails open is not a paywall.
//!
//! Verification is delegated to a facilitator rather than implemented here.
//! Settling a Solana payment means submitting and confirming a transaction, and
//! that is a different job from serving a request — see ADR 0002's note on the
//! same boundary for data.

use radar_types::MicroUsd;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The x402 protocol revision this speaks.
pub const X402_VERSION: u32 = 2;

/// Where payments settle.
///
/// Solana is the default because Radar already holds SOL for execution and
/// because settlement there is fast enough that a paid call is a request rather
/// than a wait.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Network {
    /// CAIP-2 identifier, e.g. `solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp`.
    pub caip2: String,
    /// The token contract or mint accepted.
    pub asset: String,
    /// Human-readable asset name, for the challenge body.
    pub asset_symbol: String,
    /// Decimals the asset uses, so an amount can be rendered exactly.
    pub asset_decimals: u8,
}

impl Network {
    /// Solana mainnet, USDC.
    #[must_use]
    pub fn solana_usdc() -> Self {
        Self {
            caip2: "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp".to_owned(),
            asset: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_owned(),
            asset_symbol: "USDC".to_owned(),
            asset_decimals: 6,
        }
    }
}

/// What the paid surface needs before it will serve anything.
#[derive(Clone, Debug)]
pub struct Config {
    /// Where payment is received.
    pub pay_to: String,
    /// The facilitator that verifies and settles.
    pub facilitator: String,
    /// Which network and asset.
    pub network: Network,
    /// Margin over cost, as a percentage.
    pub margin_percent: u64,
}

impl Config {
    /// Reads configuration from the process environment.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        Self::from_vars(|k| std::env::var(k).ok())
    }

    /// Reads configuration from an arbitrary lookup, returning `None` when the
    /// paid surface is not configured.
    ///
    /// `None` disables the public routes entirely rather than serving them free.
    /// A paywall that falls back to open is a paywall in name only.
    ///
    /// Takes a lookup rather than reading the environment directly so the rules
    /// can be tested without mutating process state — which in this edition is
    /// `unsafe`, and which the workspace forbids outright.
    #[must_use]
    pub fn from_vars(get: impl Fn(&str) -> Option<String>) -> Option<Self> {
        let pay_to = get("RADAR_X402_PAY_TO")?;
        let facilitator = get("RADAR_X402_FACILITATOR")?;
        if pay_to.trim().is_empty() || facilitator.trim().is_empty() {
            return None;
        }
        Some(Self {
            pay_to,
            facilitator,
            network: Network::solana_usdc(),
            margin_percent: get("RADAR_X402_MARGIN_PERCENT")
                .and_then(|v| v.parse().ok())
                .unwrap_or(radar_instruments::DEFAULT_MARGIN_PERCENT),
        })
    }

    /// The `402 Payment Required` body for a call that costs `price`.
    ///
    /// The amount is in the asset's base units, which for USDC is millionths —
    /// the same scale [`MicroUsd`] uses, so the conversion is exact rather than
    /// a rounded float.
    #[must_use]
    pub fn challenge(&self, resource: &str, description: &str, price: MicroUsd) -> Value {
        json!({
            "x402Version": X402_VERSION,
            "accepts": [{
                "scheme": "exact",
                "network": self.network.caip2,
                "asset": self.network.asset,
                "payTo": self.pay_to,
                "maxAmountRequired": price.get().to_string(),
                "resource": resource,
                "description": description,
                "mimeType": "application/json",
                // Long enough for a client to sign and submit, short enough that
                // a quote cannot be held while the price changes underneath it.
                "maxTimeoutSeconds": 60,
            }],
            "error": "payment required",
        })
    }
}

/// Whether a request carries a payment header at all.
///
/// Presence is not validity: this only decides whether to challenge or to hand
/// the header to the facilitator. Nothing here treats an unverified payment as
/// good.
#[must_use]
pub fn payment_header(headers: &axum::http::HeaderMap) -> Option<String> {
    // The 2026 revision renamed `X-Payment` to `PAYMENT-SIGNATURE`; accept both
    // so a client built against either spelling can pay.
    for name in ["payment-signature", "x-payment"] {
        if let Some(v) = headers.get(name)
            && let Ok(s) = v.to_str()
            && !s.trim().is_empty()
        {
            return Some(s.to_owned());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;

    fn config() -> Config {
        Config {
            pay_to: "RadarTreasury1111111111111111111111111111111".to_owned(),
            facilitator: "https://facilitator.example".to_owned(),
            network: Network::solana_usdc(),
            margin_percent: 50,
        }
    }

    #[test]
    fn a_challenge_quotes_the_price_in_base_units_exactly() {
        // USDC has six decimals and MicroUsd is millionths, so the conversion is
        // an identity rather than a rounded float. A price quoted a millionth off
        // is a payment that fails verification.
        let body = config().challenge("/x402/v1/creator_history", "history", MicroUsd(1_500));
        assert_eq!(body["accepts"][0]["maxAmountRequired"], "1500");
        assert_eq!(body["accepts"][0]["scheme"], "exact");
        assert_eq!(body["x402Version"], 2);
        assert_eq!(body["accepts"][0]["payTo"], config().pay_to);
    }

    /// A lookup over a fixed table, standing in for the environment.
    fn vars(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let owned: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |k| {
            owned
                .iter()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.clone())
        }
    }

    #[test]
    fn the_paid_surface_is_off_unless_both_halves_are_configured() {
        // A receiving address with no facilitator means payments arrive that
        // nothing verifies; a facilitator with no address means nowhere to pay.
        // Either way the answer is "do not serve it", never "serve it free".
        assert!(Config::from_vars(vars(&[])).is_none());
        assert!(
            Config::from_vars(vars(&[("RADAR_X402_PAY_TO", "addr")])).is_none(),
            "an address alone is not enough"
        );
        assert!(
            Config::from_vars(vars(&[("RADAR_X402_FACILITATOR", "https://f")])).is_none(),
            "a facilitator alone is not enough"
        );
        assert!(
            Config::from_vars(vars(&[
                ("RADAR_X402_PAY_TO", "addr"),
                ("RADAR_X402_FACILITATOR", "  "),
            ]))
            .is_none(),
            "blank is not configured"
        );
        assert!(
            Config::from_vars(vars(&[
                ("RADAR_X402_PAY_TO", "addr"),
                ("RADAR_X402_FACILITATOR", "https://f"),
            ]))
            .is_some(),
            "both halves present enables it"
        );
    }

    #[test]
    fn the_margin_defaults_rather_than_disabling_the_surface() {
        let c = Config::from_vars(vars(&[
            ("RADAR_X402_PAY_TO", "addr"),
            ("RADAR_X402_FACILITATOR", "https://f"),
            ("RADAR_X402_MARGIN_PERCENT", "not a number"),
        ]))
        .expect("configured");
        assert_eq!(c.margin_percent, radar_instruments::DEFAULT_MARGIN_PERCENT);
    }

    #[test]
    fn both_header_spellings_are_accepted() {
        // The 2026 revision renamed X-Payment to PAYMENT-SIGNATURE. Refusing the
        // older spelling would silently reject clients built against v1.
        let mut h = HeaderMap::new();
        assert_eq!(payment_header(&h), None);

        h.insert("x-payment", "abc".parse().expect("header"));
        assert_eq!(payment_header(&h).as_deref(), Some("abc"));

        h.insert("payment-signature", "def".parse().expect("header"));
        assert_eq!(
            payment_header(&h).as_deref(),
            Some("def"),
            "the current name wins"
        );
    }

    #[test]
    fn an_empty_payment_header_counts_as_no_payment() {
        let mut h = HeaderMap::new();
        h.insert("payment-signature", "   ".parse().expect("header"));
        assert_eq!(payment_header(&h), None);
    }
}
