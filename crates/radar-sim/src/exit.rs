// SPDX-License-Identifier: Apache-2.0
//! Exit analysis: can this position actually be sold, and at what size.
//!
//! The question Radar exists to answer before any other. Most losses in this
//! market are not bad entries, they are positions that could be opened and not
//! closed — so the exit is analysed before the entry, and a position larger than
//! its measured exit is refused by the risk kernel rather than sized down.
//!
//! Two halves, and both are necessary:
//!
//! - **Structure** ([`crate::mint`]) catches what no price can show. A quote will
//!   happily price a token with a transfer hook that reverts on sell.
//! - **The curve** catches what structure cannot. A token can be perfectly
//!   transferable and still have ten dollars of depth.
//!
//! Liquidity is never reported as one number. The useful question is not "how
//! much liquidity is there" but "how much can leave at a price I would accept",
//! and those have different answers at every size.

use std::collections::BTreeMap;

use radar_types::Address;
use serde::{Deserialize, Serialize};

use crate::mint::{Extension, MintStructure};

/// One point on the exit curve: what selling this much actually returns.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct QuotePoint {
    /// Tokens offered, in base units.
    pub size_tokens: u64,
    /// Lamports the route returns.
    pub out_lamports: u64,
    /// Price impact in basis points, as the router reports it.
    pub impact_bps: u32,
}

/// Why a quote could not be obtained.
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum QuoteError {
    /// No route exists at this size.
    ///
    /// Not an error in the usual sense — it is the answer, and one of the more
    /// important ones. A token with no sell route at any size cannot be exited.
    #[error("no route to sell {size_tokens} base units")]
    NoRoute {
        /// The size asked for.
        size_tokens: u64,
    },
    /// The router could not be reached or did not answer usefully.
    #[error("quote failed: {0}")]
    Unavailable(String),
}

/// Something that can price a sell.
///
/// A trait so the curve logic is testable without a network. Everything that
/// decides whether a position is allowed has to be exercisable offline.
pub trait Quoter {
    /// Quotes selling `size_tokens` base units of `mint` for lamports.
    ///
    /// # Errors
    ///
    /// Returns [`QuoteError::NoRoute`] when nothing will buy at that size, and
    /// [`QuoteError::Unavailable`] when the router itself could not answer.
    fn quote_sell(&self, mint: &Address, size_tokens: u64) -> Result<QuotePoint, QuoteError>;
}

/// How much of the report rests on measurement rather than assumption.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    /// Structure read and the curve measured at every size asked for.
    Measured,
    /// Structure read, but some sizes had no route or no answer.
    Partial,
    /// Structure could not be read, or nothing could be quoted.
    ///
    /// The risk kernel treats this as no exit at all, which is the point.
    Unknown,
}

/// What is known about getting out of a position.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct ExitReport {
    /// The token.
    pub mint: Address,
    /// What the mint account says, if it could be read.
    pub structure: Option<MintStructure>,
    /// Quotes at each size attempted, in ascending order.
    pub curve: Vec<QuotePoint>,
    /// Sizes that had no route.
    pub no_route_at: Vec<u64>,
    /// Extensions or authorities that could stop or tax an exit.
    pub structural_threats: Vec<Extension>,
    /// Whether someone can stop a holder selling.
    pub can_be_stopped: bool,
    /// How much of this was measured.
    pub confidence: Confidence,
}

impl ExitReport {
    /// The most lamports that can be taken out without exceeding `max_impact_bps`.
    ///
    /// `None` when nothing measured qualifies — including when the curve is
    /// empty. Callers must treat that as "cannot exit", never as "no limit
    /// found": an unmeasured exit and an unlimited one are opposite facts.
    #[must_use]
    pub fn capacity_lamports(&self, max_impact_bps: u32) -> Option<u64> {
        self.curve
            .iter()
            .filter(|p| p.impact_bps <= max_impact_bps)
            .map(|p| p.out_lamports)
            .max()
    }

    /// Whether this token should be considered exitable at all.
    ///
    /// Structure is decisive on its own. A token that can be frozen, hooked or
    /// seized is one whose quote describes a sale someone else can cancel, and
    /// a good price on an exit that can be revoked is not a good price.
    #[must_use]
    pub fn is_exitable(&self) -> bool {
        !self.can_be_stopped && self.confidence != Confidence::Unknown && !self.curve.is_empty()
    }

    /// Builds a report from structure and a set of quotes.
    #[must_use]
    pub fn build(
        mint: Address,
        structure: Option<MintStructure>,
        results: Vec<(u64, Result<QuotePoint, QuoteError>)>,
    ) -> Self {
        let mut curve = Vec::new();
        let mut no_route_at = Vec::new();
        let mut unavailable = 0usize;

        for (size, result) in results {
            match result {
                Ok(point) => curve.push(point),
                Err(QuoteError::NoRoute { .. }) => no_route_at.push(size),
                Err(QuoteError::Unavailable(_)) => unavailable += 1,
            }
        }
        curve.sort_by_key(|p| p.size_tokens);

        let structural_threats = structure
            .as_ref()
            .map(MintStructure::exit_threats)
            .unwrap_or_default();
        // No structure read means nothing is known about whether a sale can be
        // stopped, and unknown is not the same as safe.
        let can_be_stopped = structure.as_ref().is_none_or(MintStructure::can_be_stopped);

        let confidence = if structure.is_none() || curve.is_empty() {
            Confidence::Unknown
        } else if !no_route_at.is_empty() || unavailable > 0 {
            Confidence::Partial
        } else {
            Confidence::Measured
        };

        Self {
            mint,
            structure,
            curve,
            no_route_at,
            structural_threats,
            can_be_stopped,
            confidence,
        }
    }
}

/// The sizes an exit is probed at, as multiples of the intended position.
///
/// Probing at the intended size alone answers "can I get out of exactly this",
/// which is the wrong question — by the time a holder wants out, so does
/// everyone else. The multiples above one are what show whether depth survives
/// company.
pub const PROBE_MULTIPLES: &[u64] = &[1, 2, 5];

/// Quotes an exit across the probe multiples.
///
/// Errors are collected rather than propagated: a size with no route is a
/// finding about the token, not a failure of the analysis, and a report that
/// gave up at the first missing route would hide the sizes that *did* work.
#[must_use]
pub fn probe<Q: Quoter + ?Sized>(
    quoter: &Q,
    mint: &Address,
    structure: Option<MintStructure>,
    intended_tokens: u64,
) -> ExitReport {
    let results = PROBE_MULTIPLES
        .iter()
        .map(|m| {
            let size = intended_tokens.saturating_mul(*m);
            (size, quoter.quote_sell(mint, size))
        })
        .collect();
    ExitReport::build(*mint, structure, results)
}

/// Impact thresholds a caller is likely to ask about, in basis points.
pub const IMPACT_THRESHOLDS: &[u32] = &[50, 100, 300];

/// Capacity at each standard threshold.
#[must_use]
pub fn capacity_table(report: &ExitReport) -> BTreeMap<u32, Option<u64>> {
    IMPACT_THRESHOLDS
        .iter()
        .map(|bps| (*bps, report.capacity_lamports(*bps)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mint::{TOKEN_2022_PROGRAM, TOKEN_PROGRAM};

    fn mint() -> Address {
        Address::new([5u8; 32])
    }

    fn clean_structure() -> MintStructure {
        let mut data = vec![0u8; 82];
        data[44] = 6;
        data[45] = 1;
        MintStructure::parse(&data, TOKEN_PROGRAM).expect("parses")
    }

    fn freezable_structure() -> MintStructure {
        let mut data = vec![0u8; 82];
        data[44] = 6;
        data[45] = 1;
        data[46..50].copy_from_slice(&1u32.to_le_bytes());
        data[50..82].copy_from_slice(&[9u8; 32]);
        MintStructure::parse(&data, TOKEN_PROGRAM).expect("parses")
    }

    fn hooked_structure() -> MintStructure {
        let mut data = vec![0u8; 300];
        data[44] = 6;
        data[45] = 1;
        data[165] = 1;
        data[166..168].copy_from_slice(&14u16.to_le_bytes());
        data[168..170].copy_from_slice(&32u16.to_le_bytes());
        MintStructure::parse(&data, TOKEN_2022_PROGRAM).expect("parses")
    }

    /// A quoter with a fixed depth: impact grows with size, and past a ceiling
    /// there is simply no route.
    struct Pool {
        depth_tokens: u64,
    }

    impl Quoter for Pool {
        fn quote_sell(&self, _mint: &Address, size_tokens: u64) -> Result<QuotePoint, QuoteError> {
            if size_tokens > self.depth_tokens {
                return Err(QuoteError::NoRoute { size_tokens });
            }
            let impact_bps =
                u32::try_from(size_tokens * 10_000 / self.depth_tokens.max(1)).unwrap_or(u32::MAX);
            Ok(QuotePoint {
                size_tokens,
                out_lamports: size_tokens / 1_000,
                impact_bps,
            })
        }
    }

    #[test]
    fn a_clean_token_with_depth_is_exitable() {
        let report = probe(
            &Pool {
                depth_tokens: 10_000_000,
            },
            &mint(),
            Some(clean_structure()),
            1_000_000,
        );
        assert_eq!(report.confidence, Confidence::Measured);
        assert!(report.is_exitable());
        assert_eq!(report.curve.len(), PROBE_MULTIPLES.len());
        assert!(report.no_route_at.is_empty());
    }

    #[test]
    fn a_freezable_token_is_not_exitable_however_good_the_quote() {
        // A good price on an exit someone else can cancel is not a good price.
        let report = probe(
            &Pool {
                depth_tokens: 10_000_000_000,
            },
            &mint(),
            Some(freezable_structure()),
            1_000,
        );
        assert!(!report.curve.is_empty(), "the quotes are fine");
        assert!(report.can_be_stopped);
        assert!(!report.is_exitable());
    }

    #[test]
    fn a_transfer_hook_is_reported_as_a_structural_threat() {
        // No price can show this. A quote will happily price a token with a hook
        // that reverts on sell.
        let report = probe(
            &Pool {
                depth_tokens: 10_000_000,
            },
            &mint(),
            Some(hooked_structure()),
            1_000,
        );
        assert!(report.structural_threats.contains(&Extension::TransferHook));
        assert!(!report.is_exitable());
    }

    #[test]
    fn depth_that_runs_out_partway_is_partial_rather_than_a_failure() {
        // A size with no route is a finding about the token, not a failure of
        // the analysis, and the sizes that did work still matter.
        let report = probe(
            &Pool {
                depth_tokens: 1_500_000,
            },
            &mint(),
            Some(clean_structure()),
            1_000_000,
        );
        assert_eq!(report.confidence, Confidence::Partial);
        assert_eq!(report.curve.len(), 1, "only 1x fit");
        assert_eq!(report.no_route_at, vec![2_000_000, 5_000_000]);
    }

    #[test]
    fn a_token_nobody_will_buy_at_any_size_is_unknown_not_empty() {
        let report = probe(
            &Pool { depth_tokens: 0 },
            &mint(),
            Some(clean_structure()),
            1_000,
        );
        assert_eq!(report.confidence, Confidence::Unknown);
        assert!(!report.is_exitable());
        assert_eq!(report.capacity_lamports(10_000), None);
    }

    #[test]
    fn unreadable_structure_is_never_treated_as_safe() {
        // Unknown and safe are opposite facts, and the cautious one is the only
        // acceptable default when capital is downstream.
        let report = probe(
            &Pool {
                depth_tokens: 10_000_000,
            },
            &mint(),
            None,
            1_000,
        );
        assert!(
            report.can_be_stopped,
            "not knowing must not read as nothing can stop it"
        );
        assert_eq!(report.confidence, Confidence::Unknown);
        assert!(!report.is_exitable());
    }

    #[test]
    fn capacity_is_the_largest_size_within_the_impact_budget() {
        let report = probe(
            &Pool {
                depth_tokens: 10_000_000,
            },
            &mint(),
            Some(clean_structure()),
            1_000_000,
        );
        // 1x is 1000bps, 2x is 2000bps, 5x is 5000bps against this depth.
        assert_eq!(report.capacity_lamports(1_000), Some(1_000));
        assert_eq!(report.capacity_lamports(2_500), Some(2_000));
        assert_eq!(
            report.capacity_lamports(100),
            None,
            "nothing fits a 1% budget"
        );
    }

    #[test]
    fn the_probe_asks_beyond_the_intended_size() {
        // By the time a holder wants out, so does everyone else. Probing only at
        // the intended size answers the wrong question.
        assert!(PROBE_MULTIPLES.contains(&1));
        assert!(
            PROBE_MULTIPLES.iter().any(|m| *m > 1),
            "must probe past the position"
        );
    }

    #[test]
    fn the_capacity_table_covers_every_standard_threshold() {
        let report = probe(
            &Pool {
                depth_tokens: 100_000_000,
            },
            &mint(),
            Some(clean_structure()),
            1_000,
        );
        let table = capacity_table(&report);
        assert_eq!(table.len(), IMPACT_THRESHOLDS.len());
        for bps in IMPACT_THRESHOLDS {
            assert!(table.contains_key(bps));
        }
    }
}
