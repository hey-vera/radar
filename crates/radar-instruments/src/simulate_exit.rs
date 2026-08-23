// SPDX-License-Identifier: Apache-2.0
//! `simulate_exit` — can this token actually be sold, and at what size.
//!
//! The highest-value instrument in the system, and the one the plan says to build
//! first, because "can I get out" is the question that determines P&L. It is also
//! the one most worth selling: every rug-checker on the market answers the
//! structural half, and almost none of them quote the route.
//!
//! # Why this one is `Live` and not `Pure`
//!
//! Every other instrument so far reads the recorded store, so replaying it at a
//! historical watermark reproduces its answer exactly. This one asks a router
//! what it would pay *right now*. There is no historical order book to replay
//! against, so the honest declaration is [`Determinism::Live`] — and the registry
//! then knows not to expect a replay to match, rather than reporting a market
//! that moved as a leak.
//!
//! Recording it is still worth everything: the recorded quotes *become* the
//! historical series that a future version can replay against.

use radar_sim::exit::{Confidence, Quoter};
use radar_sim::{ExitReport, JupiterQuoter, RpcClient};
use radar_types::Address;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use radar_types::Mutability;

use crate::registry::{Context, Instrument, InstrumentError};
use crate::spec::{Cost, Determinism, Latency, Spec, Version};

/// Which token, and at what size.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct Input {
    /// The mint address, base58.
    pub mint: String,
    /// Size to probe from, in the token's base units.
    ///
    /// Optional because a caller who does not know the decimals cannot pick a
    /// meaningful number, and a wrong guess produces a confident answer about
    /// the wrong size.
    #[serde(default)]
    pub size: Option<u64>,
}

/// What would happen if you tried to sell.
#[derive(Debug, Clone, Serialize, JsonSchema, PartialEq)]
pub struct Output {
    /// The token asked about.
    pub mint: String,
    /// Whether a position in this could be closed at all.
    ///
    /// The single field most callers want, and the one the risk kernel refuses a
    /// position without.
    pub exitable: bool,
    /// Whether a third party can freeze, hook, tax or seize the token.
    ///
    /// Decisive on its own: a good quote on an exit somebody else can cancel is
    /// a story about a good price, not a good price.
    pub can_be_stopped: bool,
    /// Extensions or authorities that could stop or tax an exit.
    pub structural_threats: Vec<String>,
    /// Lamports out at each size probed, ascending.
    pub curve: Vec<Point>,
    /// Sizes that had no route at all.
    pub no_route_at: Vec<u64>,
    /// The largest exit, in lamports, within each impact budget.
    ///
    /// `null` means nothing measured qualifies — which is "cannot exit at this
    /// budget", never "no limit found". The two are opposite facts and the JSON
    /// must not blur them.
    pub capacity_lamports: Vec<Capacity>,
    /// How much of this was measured rather than inferred.
    pub confidence: &'static str,
    /// Whether the mint account could be read.
    ///
    /// When false, every structural claim above is absent rather than negative.
    pub structure_read: bool,
}

/// One point on the sell curve.
#[derive(Debug, Clone, Serialize, JsonSchema, PartialEq, Eq)]
pub struct Point {
    /// Size offered, in base units.
    pub size_tokens: u64,
    /// Lamports the router would pay.
    pub out_lamports: u64,
    /// Price impact in basis points, or `null` if the router did not say.
    ///
    /// Null rather than zero. Unknown impact must never read as "no impact" to
    /// something sizing a position off it.
    pub impact_bps: Option<u32>,
}

/// The largest exit within one impact budget.
#[derive(Debug, Clone, Serialize, JsonSchema, PartialEq, Eq)]
pub struct Capacity {
    /// The budget, in basis points.
    pub within_bps: u32,
    /// Lamports, or `null` if nothing measured fits.
    pub lamports: Option<u64>,
}

/// The default probe size: a thousand tokens at six decimals.
const DEFAULT_SIZE: u64 = 1_000_000_000;

/// Answers whether a token can be sold.
pub struct SimulateExit {
    quoter: Box<dyn Quoter + Send + Sync>,
    rpc: Option<RpcClient>,
}

impl Default for SimulateExit {
    fn default() -> Self {
        Self {
            quoter: Box::new(JupiterQuoter::default()),
            rpc: Some(RpcClient::default()),
        }
    }
}

impl SimulateExit {
    /// Builds one against a given quoter, for tests and for a paid router.
    ///
    /// `rpc` is optional so the structural half can be skipped when a caller has
    /// already read the mint — and so tests can run with no network at all.
    #[must_use]
    pub fn with(quoter: Box<dyn Quoter + Send + Sync>, rpc: Option<RpcClient>) -> Self {
        Self { quoter, rpc }
    }
}

impl Instrument for SimulateExit {
    type Input = Input;
    type Output = Output;

    fn spec(&self) -> Spec {
        Spec {
            name: "simulate_exit",
            version: Version::new(1, 0),
            summary: "Whether a position in this token could actually be closed, at what \
                      size, and what could stop it: mint and freeze authorities, Token-2022 \
                      extensions, and a measured sell curve rather than a liquidity number.",
            // Cold: it makes network calls, so it must never sit between a
            // decision and a submission.
            latency: Latency::Cold,
            cost: Cost::FREE,
            // A route quote describes a market that is moving. Nothing here may
            // be cached for longer than it takes to be wrong.
            freshness: Mutability::Realtime,
            // See the module docs: there is no historical order book to replay
            // against, and declaring purity would make a moved market look like
            // a leak.
            determinism: Determinism::Live,
        }
    }

    fn run(&self, input: Input, _ctx: &Context<'_>) -> Result<Output, InstrumentError> {
        let mint: Address = input
            .mint
            .parse()
            .map_err(|_| InstrumentError::BadArguments {
                instrument: "simulate_exit",
                detail: format!("`{}` is not a base58 address", input.mint),
            })?;
        let size = input.size.unwrap_or(DEFAULT_SIZE);
        if size == 0 {
            return Err(InstrumentError::BadArguments {
                instrument: "simulate_exit",
                detail: "size must be greater than zero".to_owned(),
            });
        }

        let structure = self.rpc.as_ref().and_then(|r| r.mint_structure(&mint).ok());
        let structure_read = structure.is_some();
        let report = radar_sim::probe(self.quoter.as_ref(), &mint, structure, size);

        Ok(render(&report, structure_read))
    }
}

/// Turns an [`ExitReport`] into the wire shape.
///
/// Separate so it can be tested without a quoter, and so the JSON contract lives
/// in one readable place rather than inside a network call.
#[must_use]
pub fn render(report: &ExitReport, structure_read: bool) -> Output {
    Output {
        mint: report.mint.to_string(),
        exitable: report.is_exitable(),
        can_be_stopped: report.can_be_stopped,
        structural_threats: report
            .structural_threats
            .iter()
            .map(|e| format!("{e:?}"))
            .collect(),
        curve: report
            .curve
            .iter()
            .map(|p| Point {
                size_tokens: p.size_tokens,
                out_lamports: p.out_lamports,
                // The sentinel becomes null at the boundary. Serving u32::MAX
                // to an external caller would hand them a number to do
                // arithmetic on when the truth is that nobody knows.
                impact_bps: (p.impact_bps != u32::MAX).then_some(p.impact_bps),
            })
            .collect(),
        no_route_at: report.no_route_at.clone(),
        capacity_lamports: radar_sim::capacity_table(report)
            .into_iter()
            .map(|(within_bps, lamports)| Capacity {
                within_bps,
                lamports,
            })
            .collect(),
        confidence: match report.confidence {
            Confidence::Measured => "measured",
            Confidence::Partial => "partial",
            Confidence::Unknown => "unknown",
        },
        structure_read,
    }
}

#[cfg(test)]
mod tests {
    use radar_asof::AsOf;
    use radar_sim::exit::{QuoteError, QuotePoint};
    use radar_types::Slot;

    use super::*;

    /// A quoter with a fixed answer, so the instrument runs with no network.
    struct Fixed(Vec<QuotePoint>);

    impl Quoter for Fixed {
        fn quote_sell(&self, _: &Address, size_tokens: u64) -> Result<QuotePoint, QuoteError> {
            self.0
                .iter()
                .find(|p| p.size_tokens == size_tokens)
                .copied()
                .ok_or(QuoteError::NoRoute { size_tokens })
        }
    }

    fn instrument(points: Vec<QuotePoint>) -> SimulateExit {
        SimulateExit::with(Box::new(Fixed(points)), None)
    }

    fn run(instrument: &SimulateExit, mint: &str) -> Result<Output, InstrumentError> {
        let reader = radar_store::Reader::open(".");
        let ctx = Context {
            as_of: AsOf::at(Slot(1_000)),
            store: &reader,
        };
        instrument.run(
            Input {
                mint: mint.to_owned(),
                size: Some(1_000_000),
            },
            &ctx,
        )
    }

    const MINT: &str = "So11111111111111111111111111111111111111112";

    #[test]
    fn a_quotable_token_reports_its_curve() {
        let i = instrument(vec![
            QuotePoint {
                size_tokens: 1_000_000,
                out_lamports: 500_000,
                impact_bps: 20,
            },
            QuotePoint {
                size_tokens: 2_000_000,
                out_lamports: 990_000,
                impact_bps: 60,
            },
        ]);
        let out = run(&i, MINT).expect("answers");
        assert_eq!(out.curve.len(), 2);
        assert_eq!(out.curve[0].impact_bps, Some(20));
        assert!(!out.structure_read, "no rpc was given");
    }

    #[test]
    fn an_unknown_impact_serialises_as_null_not_as_a_huge_number() {
        // Serving u32::MAX to an external caller hands them a number to do
        // arithmetic on when the truth is that nobody knows.
        let i = instrument(vec![QuotePoint {
            size_tokens: 1_000_000,
            out_lamports: 500_000,
            impact_bps: u32::MAX,
        }]);
        let out = run(&i, MINT).expect("answers");
        assert_eq!(out.curve[0].impact_bps, None);
        let json = serde_json::to_string(&out).expect("serialises");
        assert!(json.contains("\"impact_bps\":null"), "{json}");
        assert!(!json.contains("4294967295"), "{json}");
    }

    #[test]
    fn a_token_nothing_will_buy_is_not_exitable() {
        let out = run(&instrument(vec![]), MINT).expect("answers");
        assert!(!out.exitable);
        assert!(out.curve.is_empty());
        assert!(!out.no_route_at.is_empty());
    }

    #[test]
    fn a_bad_address_is_a_bad_argument_not_a_failure() {
        // The caller can fix one of those and not the other, and an x402 caller
        // is paying either way.
        let err = run(&instrument(vec![]), "not-an-address").expect_err("must refuse");
        assert!(matches!(err, InstrumentError::BadArguments { .. }), "{err}");
    }

    #[test]
    fn a_zero_size_is_refused_before_any_call_is_made() {
        let reader = radar_store::Reader::open(".");
        let ctx = Context {
            as_of: AsOf::at(Slot(1)),
            store: &reader,
        };
        let err = instrument(vec![])
            .run(
                Input {
                    mint: MINT.to_owned(),
                    size: Some(0),
                },
                &ctx,
            )
            .expect_err("must refuse");
        assert!(matches!(err, InstrumentError::BadArguments { .. }));
    }

    #[test]
    fn the_spec_keeps_this_off_the_execution_path() {
        // It makes network calls. On the x402 lane a paid call settles on-chain
        // before responding, and that must never sit between a decision and a
        // submission.
        assert!(!SimulateExit::default().spec().safe_on_execution_path());
    }

    #[test]
    fn the_spec_declares_itself_live_rather_than_pure() {
        // Declaring purity would make a market that moved between a recording
        // and its replay look like a leak, and the leak test would start crying
        // wolf until somebody switched it off.
        assert_eq!(
            SimulateExit::default().spec().determinism,
            Determinism::Live
        );
    }

    #[test]
    fn capacity_of_null_means_cannot_exit_rather_than_no_limit() {
        // Opposite facts. A JSON consumer reading a missing limit as "unlimited"
        // would size a position against a token nothing will buy.
        let out = run(&instrument(vec![]), MINT).expect("answers");
        assert!(out.capacity_lamports.iter().all(|c| c.lamports.is_none()));
        let json = serde_json::to_string(&out).expect("serialises");
        assert!(json.contains("\"lamports\":null"), "{json}");
    }
}
