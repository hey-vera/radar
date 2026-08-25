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

/// Bounds on a capacity search.
///
/// Every field is an integer, for the same reason [`radar_graph`] keeps its lift
/// in hundredths: a threshold compared as a float compares differently on a
/// replay, and a replay that disagrees with its recording is indistinguishable
/// from a leak.
///
/// [`radar_graph`]: https://github.com/hey-vera/radar
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Search {
    /// Impact budget capacity is measured at, in basis points.
    pub max_impact_bps: u32,
    /// Most quotes to spend on one token.
    ///
    /// A quote is an HTTP call against a shared rate limit, so this is a real
    /// budget rather than a safety valve. The search converges in three or four
    /// on typical tokens; the cap is what stops a pathological curve costing
    /// twenty.
    pub max_quotes: u8,
    /// Stop once a qualifying size returns at least this many lamports.
    ///
    /// Capacity far above any position Radar would take is not worth paying to
    /// measure precisely. Knowing it is "at least this much" is the whole answer.
    pub enough_lamports: u64,
}

impl Search {
    /// The starting bounds.
    pub const DEFAULT: Self = Self {
        // 1%. The same budget `creator_edge` sizes against.
        max_impact_bps: 100,
        max_quotes: 8,
        // 50 SOL. Two orders of magnitude above any position the current policy
        // could authorise, so reaching it means the token is not the constraint.
        enough_lamports: 50_000_000_000,
    };
}

/// Share of supply the first quote asks for: one ten-millionth.
///
/// Scale-free on purpose. The size that matters is a share of the token's own
/// supply, not a number of base units — a fixed base-unit size is 0.00005% of one
/// token and 5% of another, which is how the previous version of this search came
/// to measure dust on every candidate it saw (LEARNINGS entry 10).
const REFERENCE_DIVISOR: u64 = 10_000_000;

/// The search stops once the bracket is this tight, in hundredths.
///
/// 125 means "the largest qualifying size is within 25% of the smallest failing
/// one". Position sizing takes a fifth of capacity and rounds down, so resolution
/// beyond this buys nothing and costs a quote.
const BRACKET_TIGHT_X100: u64 = 125;

/// Finds the largest size that can be sold within an impact budget.
///
/// This answers a different question from [`probe`], and the difference is the
/// whole point. `probe` asks *"can I get out of this specific size?"*, which is
/// what an operator investigating one token wants. This asks *"how much can get
/// out at all?"*, which is what a strategy sizing a position needs — and it is
/// the question that cannot be answered by picking a size in advance, because
/// the size is the answer.
///
/// The search calibrates on a dust quote, projects the size at which impact would
/// reach the budget, then brackets it. Impact is close to linear in size for
/// small trades, so the projection lands near enough that two or three
/// confirmations close it.
///
/// Requires structure, because supply is what makes the first rung meaningful.
/// Without it the report is empty and therefore [`Confidence::Unknown`] — which
/// the risk kernel treats as no exit at all. That is deliberate: a search that
/// guessed a size when it could not read the supply would be the bug this
/// function replaces.
#[must_use]
pub fn discover_capacity<Q: Quoter + ?Sized>(
    quoter: &Q,
    mint: &Address,
    structure: Option<MintStructure>,
    search: Search,
) -> ExitReport {
    let Some(supply) = structure.as_ref().map(|s| s.supply).filter(|s| *s > 0) else {
        return ExitReport::build(*mint, structure, Vec::new());
    };

    let mut results: Vec<(u64, Result<QuotePoint, QuoteError>)> = Vec::new();
    let mut size = (supply / REFERENCE_DIVISOR).max(1);
    // The bracket: the largest size known to fit the budget, and the smallest
    // known not to. A `NoRoute` counts as "not to" -- it is a ceiling on what can
    // leave, which is exactly what the budget is about.
    let mut fits: Option<u64> = None;
    let mut fails: Option<u64> = None;

    for _ in 0..search.max_quotes {
        let outcome = quoter.quote_sell(mint, size);
        results.push((size, outcome.clone()));

        match outcome {
            // The router itself is broken. Continuing would spend the budget
            // re-asking a question nothing can answer.
            Err(QuoteError::Unavailable(_)) => break,
            Ok(point) if point.impact_bps <= search.max_impact_bps => {
                fits = Some(max_opt(fits, size));
                if point.out_lamports >= search.enough_lamports {
                    break;
                }
            }
            // Everything else is a ceiling on what can leave: a quote past the
            // impact budget and a size with no route at all are the same fact
            // about this size, and the search treats them the same way.
            //
            // Worth recording that the no-route half is **not distinguished by
            // any test** — a mutant deleting it survives, because `next_size`
            // halves on any error and converges regardless. It is kept because it
            // records the bound rather than reacting to it. Said plainly rather
            // than left to be discovered, since a surviving mutant nobody wrote
            // down is a claim nobody checked.
            Err(QuoteError::NoRoute { .. }) | Ok(_) => fails = Some(min_opt(fails, size)),
        }

        let Some(next) = next_size(size, &outcome, fits, fails, search, supply) else {
            break;
        };
        size = next;
    }

    ExitReport::build(*mint, structure, results)
}

/// Where to quote next, or `None` when the answer is as tight as it needs to be.
fn next_size(
    last: u64,
    outcome: &Result<QuotePoint, QuoteError>,
    fits: Option<u64>,
    fails: Option<u64>,
    search: Search,
    supply: u64,
) -> Option<u64> {
    // Bracketed on both sides: bisect, and stop once the gap stops mattering.
    if let (Some(lo), Some(hi)) = (fits, fails) {
        if hi.saturating_mul(100) <= lo.saturating_mul(BRACKET_TIGHT_X100) {
            return None;
        }
        let mid = lo + (hi - lo) / 2;
        return (mid > lo && mid < hi).then_some(mid);
    }

    let projected = match outcome {
        // Impact is roughly linear in size at these fractions of supply, so the
        // size at which it would reach the budget is a straight scaling. Clamped
        // hard because a 0 bps reading -- common on the first dust quote -- would
        // otherwise project to infinity.
        Ok(point) => {
            let ratio = u64::from(search.max_impact_bps).saturating_mul(100)
                / u64::from(point.impact_bps).max(1);
            let ratio = ratio.clamp(if fits.is_some() { 200 } else { 50 }, 6_400);
            last.saturating_mul(ratio) / 100
        }
        // No route at this size and nothing smaller tried yet: halve.
        Err(_) => last / 2,
    };

    // Never ask for more than the whole supply, and never repeat a size.
    let capped = projected.min(supply);
    (capped != last && capped > 0).then_some(capped)
}

fn min_opt(current: Option<u64>, candidate: u64) -> u64 {
    current.map_or(candidate, |c| c.min(candidate))
}

fn max_opt(current: Option<u64>, candidate: u64) -> u64 {
    current.map_or(candidate, |c| c.max(candidate))
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

    /// A mint with a realistic supply, unlike `clean_structure` whose supply
    /// field is zero — fine for the structural tests, useless for anything that
    /// scales against supply.
    fn supplied_structure(supply: u64) -> MintStructure {
        let mut data = vec![0u8; 82];
        data[36..44].copy_from_slice(&supply.to_le_bytes());
        data[44] = 6;
        data[45] = 1;
        MintStructure::parse(&data, TOKEN_PROGRAM).expect("parses")
    }

    /// The largest size in the curve that fits an impact budget.
    fn qualifying_size(report: &ExitReport, budget: u32) -> Option<u64> {
        report
            .curve
            .iter()
            .filter(|p| p.impact_bps <= budget)
            .map(|p| p.size_tokens)
            .max()
    }

    /// A pool whose depth is expressed relative to the token's own supply.
    struct RelativePool {
        depth_tokens: u64,
    }

    impl Quoter for RelativePool {
        fn quote_sell(&self, _mint: &Address, size_tokens: u64) -> Result<QuotePoint, QuoteError> {
            if size_tokens > self.depth_tokens {
                return Err(QuoteError::NoRoute { size_tokens });
            }
            let impact_bps = u32::try_from(
                u128::from(size_tokens) * 10_000 / u128::from(self.depth_tokens.max(1)),
            )
            .unwrap_or(u32::MAX);
            Ok(QuotePoint {
                size_tokens,
                out_lamports: size_tokens / 1_000,
                impact_bps,
            })
        }
    }

    const TEST_SUPPLY: u64 = 1_000_000_000_000_000;

    #[test]
    fn capacity_is_discovered_rather_than_assumed() {
        // LEARNINGS entry 10, as a regression test. The old path quoted a
        // hardcoded 1_000_000_000 base units for every token. Against this supply
        // that is one ten-thousandth of one percent, so the "capacity" it measured
        // was dust and every candidate was refused as CapacityBelowFloor — a fact
        // about the probe reported as a fact about the market.
        let report = discover_capacity(
            &RelativePool {
                depth_tokens: TEST_SUPPLY / 100,
            },
            &mint(),
            Some(supplied_structure(TEST_SUPPLY)),
            Search::DEFAULT,
        );

        let found = qualifying_size(&report, 100).expect("a token with depth has capacity");
        // The true 100 bps size is depth/100 = 1e11. The search must land near it.
        assert!(
            found > 50_000_000_000 && found <= 100_000_000_000,
            "expected capacity near 1e11, got {found}"
        );
        assert!(
            found > 10_000_000_000,
            "the old fixed probe would have reported 1e9 or less; got {found}"
        );
    }

    #[test]
    fn the_search_is_scale_free() {
        // The property the old code violated. Two tokens with identical relative
        // depth must yield the same capacity *as a share of supply*, whatever the
        // supply happens to be. A fixed base-unit probe is 0.0001% of one token
        // and 5% of another, which is how the bug survived review.
        let small = TEST_SUPPLY;
        let large = TEST_SUPPLY * 1_000;

        // The early stop is deliberately absolute — capacity past ~50 SOL is not
        // worth a quote whatever the supply — so it has to be lifted to isolate
        // the property under test. Leaving it in made this assertion fail at 94
        // against 56, which is the early stop doing its job, not the ladder
        // drifting.
        let run = |supply: u64| {
            discover_capacity(
                &RelativePool {
                    depth_tokens: supply / 100,
                },
                &mint(),
                Some(supplied_structure(supply)),
                Search {
                    enough_lamports: u64::MAX,
                    ..Search::DEFAULT
                },
            )
        };
        let share = |r: &ExitReport, supply: u64| {
            qualifying_size(r, 100)
                .map(|s| u128::from(s) * 1_000_000 / u128::from(supply))
                .expect("capacity")
        };

        assert_eq!(
            share(&run(small), small),
            share(&run(large), large),
            "same relative depth must give the same relative capacity"
        );
    }

    #[test]
    fn a_token_with_no_route_at_any_size_has_no_capacity() {
        struct Dead;
        impl Quoter for Dead {
            fn quote_sell(
                &self,
                _mint: &Address,
                size_tokens: u64,
            ) -> Result<QuotePoint, QuoteError> {
                Err(QuoteError::NoRoute { size_tokens })
            }
        }
        let report = discover_capacity(
            &Dead,
            &mint(),
            Some(supplied_structure(TEST_SUPPLY)),
            Search::DEFAULT,
        );
        assert_eq!(report.capacity_lamports(100), None);
        assert!(!report.no_route_at.is_empty());
    }

    #[test]
    fn a_size_with_no_route_bounds_the_search_from_above() {
        // Found by mutation: deleting the line that records a NoRoute as a
        // ceiling left every test passing. Nothing constrained it, so nothing
        // would have noticed the search climbing past a cliff forever.
        //
        // A pool with shallow impact and a hard ceiling is the case that
        // separates them: impact never refuses, so the route disappearing is the
        // *only* signal that the size is too big. Without it the search keeps
        // projecting upward, spends its whole budget above the cliff, and reports
        // whichever small size it happened to try first.
        struct Cliff {
            ceiling: u64,
        }
        impl Quoter for Cliff {
            fn quote_sell(
                &self,
                _mint: &Address,
                size_tokens: u64,
            ) -> Result<QuotePoint, QuoteError> {
                if size_tokens > self.ceiling {
                    return Err(QuoteError::NoRoute { size_tokens });
                }
                Ok(QuotePoint {
                    size_tokens,
                    out_lamports: size_tokens / 1_000,
                    // Always well inside any budget, so impact never refuses.
                    impact_bps: 10,
                })
            }
        }

        let ceiling = TEST_SUPPLY / 1_000;
        let report = discover_capacity(
            &Cliff { ceiling },
            &mint(),
            Some(supplied_structure(TEST_SUPPLY)),
            Search {
                enough_lamports: u64::MAX,
                max_quotes: 12,
                ..Search::DEFAULT
            },
        );

        let found = qualifying_size(&report, 100).expect("everything under the cliff fits");
        assert!(found <= ceiling, "{found} is past the cliff at {ceiling}");
        // Within a factor of two of the cliff: the search must have closed on it
        // rather than given up somewhere far below.
        assert!(
            found * 2 > ceiling,
            "search stopped at {found}, nowhere near the cliff at {ceiling}"
        );
    }

    #[test]
    fn a_search_without_structure_measures_nothing_rather_than_guessing() {
        // Rule 9. Supply is what makes a size meaningful; without it the only
        // options are to guess one or to report that nothing is known, and
        // guessing is the bug this function replaces.
        let report = discover_capacity(
            &RelativePool {
                depth_tokens: 1_000_000,
            },
            &mint(),
            None,
            Search::DEFAULT,
        );
        assert_eq!(report.confidence, Confidence::Unknown);
        assert!(
            report.curve.is_empty(),
            "no quote should have been paid for"
        );
        assert!(!report.is_exitable());
    }

    #[test]
    fn a_zero_supply_mint_measures_nothing() {
        // It must not scale to a size of zero and quote that instead.
        let report = discover_capacity(
            &RelativePool {
                depth_tokens: 1_000_000,
            },
            &mint(),
            Some(clean_structure()),
            Search::DEFAULT,
        );
        assert_eq!(report.confidence, Confidence::Unknown);
        assert!(report.curve.is_empty());
    }

    #[test]
    fn the_search_never_spends_more_quotes_than_its_budget() {
        // A quote is an HTTP call against a shared rate limit. This is a budget,
        // not a safety valve, and a curve that never brackets must still stop.
        for max_quotes in [1u8, 2, 3, 8] {
            let report = discover_capacity(
                &RelativePool {
                    depth_tokens: TEST_SUPPLY / 100,
                },
                &mint(),
                Some(supplied_structure(TEST_SUPPLY)),
                Search {
                    max_quotes,
                    ..Search::DEFAULT
                },
            );
            let asked = report.curve.len() + report.no_route_at.len();
            assert!(
                asked <= usize::from(max_quotes),
                "{max_quotes} allowed, {asked} spent"
            );
        }
    }

    #[test]
    fn a_deep_token_stops_once_it_is_deep_enough() {
        // Capacity far above any position that could be authorised is not worth
        // paying to measure precisely.
        let with_early_stop = discover_capacity(
            &RelativePool {
                depth_tokens: TEST_SUPPLY,
            },
            &mint(),
            Some(supplied_structure(TEST_SUPPLY)),
            Search {
                enough_lamports: 1_000,
                ..Search::DEFAULT
            },
        );
        let thorough = discover_capacity(
            &RelativePool {
                depth_tokens: TEST_SUPPLY,
            },
            &mint(),
            Some(supplied_structure(TEST_SUPPLY)),
            Search::DEFAULT,
        );
        assert!(
            with_early_stop.curve.len() < thorough.curve.len(),
            "an early stop must cost fewer quotes: {} vs {}",
            with_early_stop.curve.len(),
            thorough.curve.len()
        );
    }

    #[test]
    fn discovered_capacity_never_comes_from_a_point_past_the_budget() {
        // The number the strategy sizes against. A search that reported a size
        // past the budget would launder an unfillable position past the kernel.
        let report = discover_capacity(
            &RelativePool {
                depth_tokens: TEST_SUPPLY / 50,
            },
            &mint(),
            Some(supplied_structure(TEST_SUPPLY)),
            Search::DEFAULT,
        );
        let capacity = report.capacity_lamports(100).expect("has capacity");
        let source = report
            .curve
            .iter()
            .find(|p| p.out_lamports == capacity)
            .expect("capacity came from somewhere");
        assert!(
            source.impact_bps <= 100,
            "capacity came from a point at {} bps",
            source.impact_bps
        );
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
