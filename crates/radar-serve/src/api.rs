// SPDX-License-Identifier: Apache-2.0
//! The JSON surface the operator interface reads.
//!
//! Separate from the instrument registry on purpose. An instrument answers a
//! question about a *token* and is priced per call; these routes describe what
//! **Radar itself** has done, which is not something anyone should be charged
//! for and not something the x402 lane should serve.
//!
//! Every function here is a pure shaping of rows the store already holds, so
//! the shape can be tested without a server and without a network. The axum
//! handlers in [`crate`] are three lines each and contain no logic.
//!
//! # Nothing here computes a number the store does not have
//!
//! The free tier of `radar consider` refuses tens of thousands of candidates a
//! day and records none of them, because a free-tier refusal is a pure function
//! of the store and can be re-derived. So the funnel below starts at what was
//! *examined*, not at what was considered, and says so. Reporting a
//! considered-count from a stale run would be inventing the widest number on the
//! page.

use radar_asof::AsOf;
use radar_store::{Conclusion, Decision, Reader, Table};
use serde::Serialize;
use std::collections::BTreeMap;

/// One stage of the recorded funnel.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct Stage {
    /// Short identifier, stable across releases.
    pub name: &'static str,
    /// How many reached this stage.
    pub count: usize,
    /// What the stage means, in a sentence a novice can read.
    pub detail: &'static str,
}

/// How often a reason was given.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct ReasonCount {
    /// The reason, as the strategy names it.
    pub reason: String,
    /// How many decisions carried it.
    pub count: usize,
}

/// What Radar has decided, and where it stopped.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct Funnel {
    /// The watermark this was read at.
    pub as_of: u64,
    /// The stages, widest first.
    pub stages: Vec<Stage>,
    /// Why candidates were passed over, most common first.
    pub reasons: Vec<ReasonCount>,
    /// Whether the policy this build **decides** with could authorise anything.
    ///
    /// The single most important fact on the page, and the one a raw refusal
    /// list buries under six other reasons.
    ///
    /// # What it does not cover, stated because the field reads stronger than
    /// # it is
    ///
    /// This is `radar_risk::Policy::SHIPPED` — the constant `radar consider`
    /// judges against. It is **not** a guarantee that nothing can be signed.
    /// [ADR 0008](https://github.com/hey-vera/radar/blob/main/docs/adr/0008-the-signer-holds-its-own-policy.md)
    /// put a second policy in `radar-signer`, loaded from `RADAR_SIGNER_POLICY`
    /// at start, and this process cannot read it. The signer clamps against its
    /// own copy unconditionally, so it can refuse what this policy permits —
    /// never the reverse.
    ///
    /// So `true` means *the decider authorises nothing*, which is the honest
    /// claim. Rendering it as "nothing can be authorised, ever, anywhere" is a
    /// claim about a file in another process, and LEARNINGS 23 records what
    /// happens when a boundary is described as covering more than it does.
    pub policy_closed: bool,
}

/// Builds the funnel from recorded decisions.
///
/// `launches` is passed rather than read so the caller can decide how expensive
/// a read to do; the funnel does not need every launch, only the count.
#[must_use]
pub fn funnel(decisions: &[Decision], launches: usize, as_of: u64, policy_closed: bool) -> Funnel {
    let examined = decisions.len();
    let proposed = decisions.iter().filter(|d| d.proposed()).count();
    let authorised = decisions.iter().filter(|d| d.would_have_traded()).count();

    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for reason in decisions.iter().flat_map(|d| d.reasons.iter()) {
        *counts.entry(reason.as_str()).or_default() += 1;
    }
    let mut reasons: Vec<ReasonCount> = counts
        .into_iter()
        .map(|(reason, count)| ReasonCount {
            reason: reason.to_owned(),
            count,
        })
        .collect();
    // Most common first, then by name so the order is total and a client
    // rendering it twice gets the same list.
    reasons.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.reason.cmp(&b.reason)));

    Funnel {
        as_of,
        stages: vec![
            Stage {
                name: "recorded",
                count: launches,
                detail: "token launches the recorder has seen",
            },
            Stage {
                name: "examined",
                count: examined,
                detail: "candidates a paid look was spent on; the cheaper filters \
                         refuse far more and are not recorded because they can be \
                         re-derived",
            },
            Stage {
                name: "proposed",
                count: proposed,
                detail: "candidates the strategy proposed acting on",
            },
            Stage {
                name: "authorised",
                count: authorised,
                detail: "proposals the risk kernel authorised",
            },
        ],
        reasons,
        policy_closed,
    }
}

/// Everything recorded about one token.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct TokenEvidence {
    /// The mint.
    pub mint: String,
    /// Every decision taken about it, oldest first.
    pub decisions: Vec<Decision>,
    /// Every price measurement, oldest first.
    pub measurements: Vec<Measurement>,
}

/// One outcome measurement, flattened for a client.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct Measurement {
    /// When it was taken.
    pub measured_at: u64,
    /// Price reads the figures below were computed from.
    ///
    /// **Not a fill count, and it over-counts.** The price windows it is folded
    /// across overlap by five of their six hours and the fold is
    /// `saturating_add`, so a fill inside the window is counted again on every
    /// hourly pass: it grows while nothing trades, and two measurements of the
    /// same token are not comparable
    /// ([LEARNINGS](https://github.com/hey-vera/radar/blob/main/LEARNINGS.md)
    /// entry 19, which invalidated the first runs of research 0017 and 0018).
    ///
    /// Renamed on the wire from `fills` so a client cannot render it under a
    /// label the number does not support. Use
    /// [`last_transfer_slot`](Self::last_transfer_slot) for "did anything
    /// change hands".
    #[serde(rename = "price_reads")]
    pub fills: u64,
    /// The last slot a transfer was observed, or `None` if none ever was.
    ///
    /// A `max`, so unlike [`fills`](Self::fills) it cannot be inflated by
    /// re-reading the same window. This is the field that answers whether the
    /// token still trades.
    pub last_transfer_slot: Option<u64>,
    /// Prices, scaled by `PRICE_SCALE`. Null where not measured, never zero.
    pub first_price: Option<u64>,
    /// The last price observed.
    pub last_price: Option<u64>,
    /// The highest.
    pub peak_price: Option<u64>,
    /// The lowest.
    pub trough_price: Option<u64>,
    /// Whether the token graduated, and when.
    pub graduated_at: Option<u64>,
    /// Return from first to last, in basis points, where both are known.
    pub held_to_end_bps: Option<i64>,
}

/// Reads everything recorded about one mint.
///
/// # Errors
///
/// Returns [`radar_store::StoreError`] if the store cannot be read.
pub fn token_evidence(
    reader: &Reader,
    mint: &str,
    as_of: AsOf,
) -> Result<TokenEvidence, radar_store::StoreError> {
    let decisions = reader
        .read_decisions(as_of)?
        .into_iter()
        .filter(|d| d.mint.to_string() == mint)
        .collect();
    let measurements = reader
        .read_outcomes(as_of)?
        .into_iter()
        .filter(|o| o.mint.to_string() == mint)
        .map(|o| Measurement {
            measured_at: o.measured_at.get(),
            fills: o.fills,
            last_transfer_slot: o.last_transfer_slot.map(radar_types::Slot::get),
            first_price: o.first_price,
            last_price: o.last_price,
            peak_price: o.peak_price,
            trough_price: o.trough_price,
            graduated_at: o.graduated_at.map(radar_types::Slot::get),
            held_to_end_bps: o.held_to_end_gain_bps(),
        })
        .collect();

    Ok(TokenEvidence {
        mint: mint.to_owned(),
        decisions,
        measurements,
    })
}

/// How many rows each table holds, for the health screen.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Default)]
pub struct StoreCounts {
    /// Launches recorded.
    pub launches: usize,
    /// Graduations recorded.
    pub graduations: usize,
    /// Outcome measurements.
    pub outcomes: usize,
    /// Decisions recorded.
    pub decisions: usize,
}

/// Counts the store's contents at a watermark.
///
/// # Errors
///
/// Returns [`radar_store::StoreError`] if the store cannot be read.
pub fn store_counts(reader: &Reader, as_of: AsOf) -> Result<StoreCounts, radar_store::StoreError> {
    // `count` rather than `read(..).len()`. Decoding 167,987 launch events to
    // learn that there are 167,987 of them took ten seconds against the live
    // store; the row count is in each file's Parquet footer.
    Ok(StoreCounts {
        launches: reader.count(Table::Launches, as_of)?,
        graduations: reader.count(Table::Graduations, as_of)?,
        outcomes: reader.count(Table::Outcomes, as_of)?,
        decisions: reader.count(Table::Decisions, as_of)?,
    })
}

/// Whether a decision proposed acting.
///
/// Exposed because a client rendering the funnel wants the same predicate the
/// funnel counted with, rather than reimplementing it and drifting.
#[must_use]
pub const fn is_proposal(decision: &Decision) -> bool {
    matches!(decision.conclusion, Conclusion::Proposed)
}

/// The round-trip cost the scoreboard assumes.
///
/// 850 bps, measured. Research 0009 found fewer than one token in ten ever
/// finishes above it, which is the single most important number on the page and
/// the reason the scoreboard exists at all.
pub const ASSUMED_COST_BPS: u64 = 850;

/// The honest scoreboard: what Radar's selection returned against its own
/// refusals.
///
/// The comparison is the **matched control**, not a constant. An earlier version
/// of this compared Radar's cohort to research 0009's population median, and
/// those are different quantities: 0009 enters at the token's first fill and
/// Radar enters forty minutes later, and the constant itself moved from −1,340
/// to −863 bps as the cohort grew. Refusals are priced in the same passes, the
/// same way, over the same universe — which is what makes a difference between
/// them attributable to the selection rather than to the measurement.
///
/// # Errors
///
/// Returns [`radar_store::StoreError`] if the store cannot be read.
pub fn scoreboard(
    reader: &Reader,
    as_of: AsOf,
    cost_bps: u64,
) -> Result<radar_research::selection::Report, radar_store::StoreError> {
    let decisions = reader.read_decisions(as_of)?;
    let outcomes = reader.read_outcomes(as_of)?;
    Ok(radar_research::selection::evaluate(
        &decisions, &outcomes, cost_bps,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use radar_types::{Address, Slot};

    fn decision(mint: u8, proposed: bool, reasons: &[&str], authorised: bool) -> Decision {
        Decision {
            mint: Address::new([mint; 32]),
            creator: Address::new([9u8; 32]),
            decided_at: Slot(10_000),
            launch_slot: Slot(4_000),
            strategy: "creator_edge".to_owned(),
            strategy_version: "0.1.0".to_owned(),
            conclusion: if proposed {
                Conclusion::Proposed
            } else {
                Conclusion::Passed
            },
            reasons: reasons.iter().map(|r| (*r).to_owned()).collect(),
            notional_micro_usd: proposed.then_some(6_300_000),
            exit_capacity_micro_usd: None,
            assumed_round_trip_bps: 850,
            coordination: None,
            authority_prevalence: None,
            kernel_outcome: proposed.then_some(if authorised {
                radar_store::KernelOutcome::Authorised
            } else {
                radar_store::KernelOutcome::Refused
            }),
            kernel_reasons: Vec::new(),
            entry_price: None,
            inputs_digest: "d".to_owned(),
        }
    }

    #[test]
    fn the_funnel_narrows_and_never_widens() {
        // A stage that reported more than the one before it would be a bug in
        // the counting, and it is the kind a reader would believe.
        let decisions = vec![
            decision(1, true, &[], false),
            decision(2, false, &["NoRoute"], false),
            decision(3, false, &["CapacityBelowFloor"], false),
        ];
        let f = funnel(&decisions, 1_000, 500, true);

        let counts: Vec<usize> = f.stages.iter().map(|s| s.count).collect();
        assert_eq!(counts, vec![1_000, 3, 1, 0]);
        for pair in counts.windows(2) {
            assert!(pair[1] <= pair[0], "the funnel widened: {counts:?}");
        }
    }

    #[test]
    fn a_proposal_the_kernel_refused_is_not_counted_as_authorised() {
        // The distinction that keeps the last stage honest. Every proposal in
        // production is refused by Policy::CLOSED, so a funnel counting
        // proposals as authorised would report activity that never happened.
        let refused = vec![decision(1, true, &[], false)];
        let authorised = vec![decision(1, true, &[], true)];

        assert_eq!(funnel(&refused, 10, 1, true).stages[3].count, 0);
        assert_eq!(funnel(&authorised, 10, 1, false).stages[3].count, 1);
    }

    #[test]
    fn reasons_are_ranked_by_frequency_with_a_total_order() {
        // A client rendering this twice must get the same list. Sorting by count
        // alone leaves ties in whatever order the map yielded.
        let decisions = vec![
            decision(1, false, &["NoRoute"], false),
            decision(2, false, &["NoRoute"], false),
            decision(3, false, &["CapacityBelowFloor"], false),
            decision(4, false, &["AaaSameCount"], false),
        ];
        let f = funnel(&decisions, 10, 1, true);
        assert_eq!(f.reasons[0].reason, "NoRoute");
        assert_eq!(f.reasons[0].count, 2);
        assert_eq!(
            f.reasons[1].reason, "AaaSameCount",
            "ties break by name, so the order is total"
        );
        assert_eq!(f.reasons[2].reason, "CapacityBelowFloor");
    }

    #[test]
    fn a_decision_carrying_several_reasons_counts_towards_each() {
        // A candidate that failed four ways is more informative than one that
        // failed once, and the funnel should say so rather than picking one.
        let decisions = vec![decision(
            1,
            false,
            &["NoRoute", "CapacityBelowFloor"],
            false,
        )];
        let f = funnel(&decisions, 10, 1, true);
        assert_eq!(f.reasons.len(), 2);
        assert!(f.reasons.iter().all(|r| r.count == 1));
    }

    #[test]
    fn an_empty_store_reports_zeroes_rather_than_refusing() {
        // A fresh instance has recorded nothing, and that is a state the screen
        // has to render. It is distinguishable from a broken one by the stage
        // details, which are present either way.
        let f = funnel(&[], 0, 0, true);
        assert!(f.stages.iter().all(|s| s.count == 0));
        assert!(f.reasons.is_empty());
        assert_eq!(f.stages.len(), 4, "the stages are always described");
    }

    #[test]
    fn the_examined_stage_says_it_is_not_the_whole_funnel() {
        // The free tier refuses tens of thousands a day and records none of
        // them. A reader seeing "examined: 900" under "recorded: 170,618" would
        // otherwise conclude the other 169,718 were never looked at.
        let f = funnel(&[], 170_618, 1, true);
        let examined = &f.stages[1];
        assert!(
            examined.detail.contains("re-derived"),
            "the stage must explain what is missing: {}",
            examined.detail
        );
    }

    #[test]
    fn the_closed_policy_is_reported_as_one_fact() {
        // The single most important thing on the page. A raw refusal list buries
        // it under six other reasons that are the same fact restated.
        assert!(funnel(&[], 0, 0, true).policy_closed);
        assert!(!funnel(&[], 0, 0, false).policy_closed);
    }
}
