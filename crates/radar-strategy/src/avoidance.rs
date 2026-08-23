// SPDX-License-Identifier: Apache-2.0
//! The shared disqualification gate.
//!
//! Radar's first claimed edge is *not buying the traps*. It is claimed first
//! because it is the only one that costs nothing to apply: every check here runs
//! on data already held, in microseconds, with no paid call.
//!
//! Every strategy runs this before its own rules. Not by convention — by
//! construction, because [`disqualify`] is the only way to build the reason list
//! a [`crate::Decision::Pass`] carries, and a strategy that skipped it would
//! have to fabricate one.
//!
//! The checks are deliberately blunt. A rule that rejects some good tokens along
//! with the traps is acceptable here; the population is ~35,000 launches a day
//! and there is no shortage of candidates. A rule that admits traps is not.

use crate::Candidate;

/// Why a candidate was passed over.
///
/// Ordered so that the structural reasons — the ones that say the token itself
/// is unsafe — sort before the ones that say the *evidence* is thin. A sorted
/// reason list reads worst-first.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PassReason {
    /// Someone can freeze, hook, seize or pause this token.
    ///
    /// Decisive on its own. A good quote on an exit that a third party can
    /// cancel is not a good price, it is a story about one.
    ExitCanBeStopped,
    /// No route at any size probed.
    NoRoute,
    /// An exit analysis was run and could not measure anything.
    ExitUnmeasurable,
    /// No exit analysis was run at all.
    ///
    /// Distinct from [`Self::ExitUnmeasurable`]: one means the token failed the
    /// test, the other means the test was never given. Collapsing them would
    /// hide a gap in the pipeline as a property of the token.
    NoExitSimulated,
    /// The creator has too few measured launches to say anything about them.
    CreatorUnproven,
    /// The creator's measured launches mostly died.
    CreatorMostlyStillborn,
    /// The creator has never graduated a token.
    CreatorNeverGraduated,
    /// The creator graduates tokens, but below the threshold in force.
    ///
    /// Separate from [`Self::CreatorNeverGraduated`] because they are different
    /// findings: one is a fact about the creator, the other is a fact about
    /// where a threshold currently sits. Research moves the second, never the
    /// first, and collapsing them would hide which is which.
    CreatorGraduatesTooRarely,
    /// No SOL price, so exit capacity cannot be turned into a notional.
    NoPrice,
    /// Capacity is real but too small to be worth a round trip's costs.
    CapacityBelowFloor,
    /// The candidate rests on an input older than the strategy accepts.
    InputsTooStale,
}

impl PassReason {
    /// Whether this reason is about the token rather than the evidence.
    ///
    /// Structural reasons do not improve with more data — no amount of waiting
    /// makes a freezable token unfreezable. Evidence reasons might, which makes
    /// them worth re-examining and structural ones worth caching against the
    /// mint forever.
    #[must_use]
    pub const fn is_structural(self) -> bool {
        matches!(
            self,
            Self::ExitCanBeStopped | Self::NoRoute | Self::ExitUnmeasurable
        )
    }
}

/// The disqualifications that apply to every strategy.
///
/// Returns an empty list when nothing here objects — which is permission to
/// consider the candidate, never a recommendation to buy it.
///
/// Note that all checks run: the first failure does not short-circuit. A
/// candidate that fails four ways is more informative than one that fails once,
/// and the research store wants the whole list.
#[must_use]
pub fn disqualify(candidate: &Candidate) -> Vec<PassReason> {
    let mut reasons = Vec::new();

    match &candidate.exit {
        None => reasons.push(PassReason::NoExitSimulated),
        Some(exit) => {
            if exit.can_be_stopped {
                reasons.push(PassReason::ExitCanBeStopped);
            }
            if exit.curve.is_empty() {
                reasons.push(PassReason::NoRoute);
            } else if !exit.is_exitable() {
                // A curve exists but the report still says no: unknown
                // confidence, or a threat the curve cannot express.
                reasons.push(PassReason::ExitUnmeasurable);
            }
        }
    }

    if candidate.sol_price_micro_usd.is_none() {
        reasons.push(PassReason::NoPrice);
    }

    reasons.sort_unstable();
    reasons.dedup();
    reasons
}

#[cfg(test)]
mod tests {
    use radar_asof::AsOf;
    use radar_sim::exit::{Confidence, QuotePoint};
    use radar_sim::{ExitReport, Extension};
    use radar_types::{Address, MicroUsd, Slot};

    use super::*;
    use crate::CreatorRecord;

    fn exitable() -> ExitReport {
        ExitReport {
            mint: Address::new([7u8; 32]),
            structure: None,
            curve: vec![QuotePoint {
                size_tokens: 1_000_000,
                out_lamports: 50_000,
                impact_bps: 20,
            }],
            no_route_at: Vec::new(),
            structural_threats: Vec::new(),
            can_be_stopped: false,
            confidence: Confidence::Measured,
        }
    }

    fn candidate(exit: Option<ExitReport>) -> Candidate {
        Candidate {
            mint: Address::new([7u8; 32]),
            creator: Address::new([8u8; 32]),
            launch_slot: Slot(1_000),
            as_of: AsOf::at(Slot(10_000)),
            exit,
            creator_record: CreatorRecord::default(),
            sol_price_micro_usd: Some(MicroUsd::from_dollars(200.0)),
            oldest_input_slot: Slot(9_900),
        }
    }

    #[test]
    fn a_clean_candidate_is_not_disqualified() {
        assert_eq!(disqualify(&candidate(Some(exitable()))), Vec::new());
    }

    #[test]
    fn no_exit_analysis_is_its_own_reason() {
        // Never conflate "failed the test" with "was never tested". One is a
        // fact about the token, the other is a gap in the pipeline.
        assert_eq!(
            disqualify(&candidate(None)),
            vec![PassReason::NoExitSimulated]
        );
    }

    #[test]
    fn a_freezable_token_is_refused_however_good_the_quote() {
        let mut exit = exitable();
        exit.can_be_stopped = true;
        exit.structural_threats = vec![Extension::PermanentDelegate];
        assert!(disqualify(&candidate(Some(exit))).contains(&PassReason::ExitCanBeStopped));
    }

    #[test]
    fn an_empty_curve_reads_as_no_route_not_as_unmeasurable() {
        let mut exit = exitable();
        exit.curve.clear();
        exit.confidence = Confidence::Unknown;
        let reasons = disqualify(&candidate(Some(exit)));
        assert!(reasons.contains(&PassReason::NoRoute));
        assert!(
            !reasons.contains(&PassReason::ExitUnmeasurable),
            "no route is the specific finding; the general one would bury it"
        );
    }

    #[test]
    fn a_quoted_but_unconfident_exit_is_refused() {
        // Jupiter answered, so there is a curve, but nothing corroborated it.
        let mut exit = exitable();
        exit.confidence = Confidence::Unknown;
        assert!(disqualify(&candidate(Some(exit))).contains(&PassReason::ExitUnmeasurable));
    }

    #[test]
    fn every_failure_is_reported_not_just_the_first() {
        // A candidate failing four ways is more informative than one failing
        // once, and the research store wants the whole list.
        let mut exit = exitable();
        exit.can_be_stopped = true;
        exit.curve.clear();
        let mut c = candidate(Some(exit));
        c.sol_price_micro_usd = None;
        assert_eq!(disqualify(&c).len(), 3);
    }

    #[test]
    fn structural_reasons_sort_before_evidence_reasons() {
        // So a reason list reads worst-first, and so the two kinds can be
        // cached differently: structure never improves, evidence might.
        assert!(PassReason::ExitCanBeStopped < PassReason::CreatorUnproven);
        assert!(PassReason::ExitCanBeStopped.is_structural());
        assert!(!PassReason::CreatorUnproven.is_structural());
    }
}
