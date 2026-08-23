// SPDX-License-Identifier: Apache-2.0
//! A [`Quoter`] backed by Jupiter.
//!
//! Jupiter's free tier routes pump.fun bonding-curve liquidity as well as the
//! AMMs, which is what makes it usable for tokens minutes old — the population
//! Radar cares about and the one most routers cannot see.
//!
//! Deliberately thin. Everything that *decides* anything lives in [`crate::exit`]
//! behind the [`Quoter`] trait, so the rules that gate capital are exercisable
//! without a network. This file only turns HTTP into a [`QuotePoint`].

use std::time::Duration;

use radar_types::Address;
use serde::Deserialize;

use crate::exit::{QuoteError, QuotePoint, Quoter};

/// Jupiter's free quote endpoint.
pub const LITE_API: &str = "https://lite-api.jup.ag/swap/v1/quote";

/// Wrapped SOL, the mint an exit is quoted into.
pub const WSOL: &str = "So11111111111111111111111111111111111111112";

/// What Jupiter returns.
#[derive(Debug, Deserialize)]
struct QuoteResponse {
    #[serde(rename = "outAmount")]
    out_amount: String,
    #[serde(rename = "priceImpactPct")]
    price_impact_pct: Option<String>,
}

/// Quotes sells through Jupiter.
pub struct JupiterQuoter {
    endpoint: String,
    agent: ureq::Agent,
    slippage_bps: u32,
}

impl Default for JupiterQuoter {
    fn default() -> Self {
        Self::new(LITE_API)
    }
}

impl JupiterQuoter {
    /// A quoter against the given endpoint.
    #[must_use]
    pub fn new(endpoint: impl Into<String>) -> Self {
        let config = ureq::Agent::config_builder()
            // Short. A quote is a statement about a market that is moving, and
            // one that took thirty seconds is describing a market that has gone.
            .timeout_global(Some(Duration::from_secs(15)))
            .build();
        Self {
            endpoint: endpoint.into(),
            agent: config.into(),
            // Quoting is not trading. This is the slippage the *quote* assumes,
            // and a wide setting here would flatter the reported output.
            slippage_bps: 100,
        }
    }
}

/// Converts Jupiter's percentage string into basis points.
///
/// Jupiter reports impact as a decimal fraction string, sometimes with far more
/// precision than is meaningful and sometimes as exactly `"0"`. A parse failure
/// becomes the maximum rather than zero: an unreadable impact is unknown, and
/// unknown must not read as "no impact" to something sizing a position.
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

impl Quoter for JupiterQuoter {
    fn quote_sell(&self, mint: &Address, size_tokens: u64) -> Result<QuotePoint, QuoteError> {
        if size_tokens == 0 {
            return Err(QuoteError::NoRoute { size_tokens });
        }

        let response = self
            .agent
            .get(&self.endpoint)
            .query("inputMint", mint.to_string())
            .query("outputMint", WSOL)
            .query("amount", size_tokens.to_string())
            .query("slippageBps", self.slippage_bps.to_string())
            .call();

        let mut response = match response {
            Ok(r) => r,
            // Jupiter answers "nothing will buy this" with a 4xx. That is the
            // finding, not a failure -- and one of the more important findings
            // an exit analysis can produce.
            Err(ureq::Error::StatusCode(code)) if (400..500).contains(&code) => {
                return Err(QuoteError::NoRoute { size_tokens });
            }
            Err(e) => return Err(QuoteError::Unavailable(e.to_string())),
        };

        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|e| QuoteError::Unavailable(e.to_string()))?;

        let quote: QuoteResponse = serde_json::from_str(&body)
            .map_err(|e| QuoteError::Unavailable(format!("unreadable quote: {e}")))?;

        let out_lamports = quote
            .out_amount
            .parse::<u64>()
            .map_err(|_| QuoteError::Unavailable(format!("bad outAmount: {}", quote.out_amount)))?;

        // A route that returns nothing is not a route.
        if out_lamports == 0 {
            return Err(QuoteError::NoRoute { size_tokens });
        }

        Ok(QuotePoint {
            size_tokens,
            out_lamports,
            impact_bps: impact_to_bps(quote.price_impact_pct.as_deref()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impact_parses_the_shapes_jupiter_actually_returns() {
        // All three observed on the live endpoint.
        assert_eq!(impact_to_bps(Some("0")), 0);
        assert_eq!(impact_to_bps(Some("0.0000618213655794423150542903")), 1);
        assert_eq!(impact_to_bps(Some("0.05")), 500);
    }

    #[test]
    fn an_unreadable_impact_is_the_maximum_rather_than_zero() {
        // Unknown impact must not read as "no impact" to something sizing a
        // position off it. Zero would make the worst case look like the best.
        assert_eq!(impact_to_bps(None), u32::MAX);
        assert_eq!(impact_to_bps(Some("")), u32::MAX);
        assert_eq!(impact_to_bps(Some("not a number")), u32::MAX);
        assert_eq!(impact_to_bps(Some("NaN")), u32::MAX);
    }

    #[test]
    fn a_negative_impact_is_taken_by_magnitude() {
        // Routers occasionally report a favourable impact as negative. Its size
        // is what matters for a capacity budget, not its sign.
        assert_eq!(impact_to_bps(Some("-0.01")), 100);
    }

    #[test]
    fn a_zero_size_quote_is_refused_without_a_request() {
        let q = JupiterQuoter::new("http://127.0.0.1:1/never-reached");
        assert_eq!(
            q.quote_sell(&Address::new([1u8; 32]), 0),
            Err(QuoteError::NoRoute { size_tokens: 0 })
        );
    }
}
