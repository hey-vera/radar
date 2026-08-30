// SPDX-License-Identifier: Apache-2.0
//! What the decision lane concluded, recorded at the watermark it concluded it.
//!
//! Every signal in this repository has been validated against *graduation*, and
//! [research 0009](../../docs/research/0009-what-a-token-actually-does-to-your-money.md)
//! measured what a token does to a holder: a median of **−13.4%** before costs,
//! with **8.9%** clearing the 850 bps round trip. Both halves exist. What does
//! not exist is the join between them — *what did Radar decide about this mint,
//! and what did that mint then do* — because `radar consider` printed its
//! decisions and exited.
//!
//! Until that join exists, no claim that Radar's selection beats the base rate
//! can be made at all. This is the table that makes it possible.
//!
//! # Why the refusals are recorded, not only the proposals
//!
//! Radar's thesis is that the edge is in **not buying traps**, so the refusals
//! are the product rather than the leftovers. A store holding only proposals
//! could say what the seven tokens Radar liked went on to do, and nothing about
//! whether refusing the other 41,714 was worth doing. The counterfactual is the
//! measurement.
//!
//! It also makes the funnel checkable after the fact rather than only while the
//! command is running, which is what a reader of the numbers needs.
//!
//! # Why this is not a chain event
//!
//! It has no signature and no transaction position, because nothing happened on
//! chain. It is stamped `decided_at` rather than `slot` for the same reason
//! [`Outcome`](crate::Outcome) is stamped `measured_at`: a decision was taken
//! *as of* a watermark, and conflating that with the slot something happened at
//! is how look-ahead gets in.

use radar_types::{Address, Slot};
use serde::{Deserialize, Serialize};

/// What the strategy concluded about one candidate.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Conclusion {
    /// Passed over. The reasons are in [`Decision::reasons`].
    Passed,
    /// A proposal was raised and handed to the risk kernel.
    Proposed,
}

/// What the risk kernel did with a proposal.
///
/// Absent when no proposal was raised — the kernel never saw one, which is a
/// different state from having refused it. AGENTS.md rule 9.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KernelOutcome {
    /// Authorised, under bounds.
    Authorised,
    /// Refused. The reasons are in [`Decision::kernel_reasons`].
    Refused,
}

/// One recorded decision.
///
/// Deliberately flat and deliberately small. Every field is either something a
/// later join needs (`mint`, `decided_at`, `conclusion`, `notional_micro_usd`)
/// or something that says which *rule* produced it, so a decision taken under
/// thresholds that have since moved is not silently compared with one taken
/// under today's.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Decision {
    /// The token.
    pub mint: Address,
    /// Its creator, so exposure and outcomes can be grouped without a lookup.
    pub creator: Address,
    /// The watermark the decision was taken as of.
    ///
    /// Not the slot the token launched at, and not the time the command ran.
    /// This is the point in the store's history the decision is reproducible
    /// from, which is what a replay needs.
    pub decided_at: Slot,
    /// When the token launched, so a return can be measured from the right
    /// origin without a second read.
    pub launch_slot: Slot,
    /// Which strategy decided.
    pub strategy: String,
    /// Its version. A decision that changed under a changed version is
    /// expected; under the same version it is a bug.
    pub strategy_version: String,
    /// Proposed or passed over.
    pub conclusion: Conclusion,
    /// Why it was passed over, worst-first, as
    /// [`PassReason`](radar_strategy::PassReason) renders them.
    ///
    /// Stored as strings rather than as an enum, because this table outlives
    /// the code: a reason retired from the strategy must still read correctly
    /// from a file written a month ago. An enum would either refuse to
    /// deserialise or silently remap.
    pub reasons: Vec<String>,
    /// What the strategy proposed committing, if anything.
    pub notional_micro_usd: Option<u64>,
    /// The exit capacity the sizing was derived from.
    ///
    /// The number [`notional_micro_usd`](Self::notional_micro_usd) is a
    /// fraction of, kept because a proposal that looks small is a different
    /// finding depending on whether the market was thin or the share was
    /// conservative.
    pub exit_capacity_micro_usd: Option<u64>,
    /// What the round trip was assumed to cost, per ten thousand.
    ///
    /// Recorded because it moved by a factor of four on 2026-08-25 and every
    /// decision either side of that is judged against a different bar.
    pub assumed_round_trip_bps: u64,
    /// The coordination verdict, if a launch block was read.
    ///
    /// `None` means the source could not answer, never that the launch looked
    /// clean — the distinction the whole gate rests on.
    pub coordination: Option<String>,
    /// How widely the launch block's signing wallets appear in other launch
    /// blocks, as [`radar_graph::prevalence::Prevalence`] labels it.
    ///
    /// `None` means the prevalence table could not be read — including the case
    /// where it was truncated, which is a table that cannot be trusted rather
    /// than one that found nothing. Never that the wallets looked ordinary.
    ///
    /// Recorded and not acted on. Research 0012 measured *who* recurs and not
    /// whether recurrence predicts anything about money, so this exists to make
    /// that second question answerable later — the same order the coordination
    /// gate and the decisions table were built in.
    pub authority_prevalence: Option<String>,
    /// What the risk kernel did, if it was handed a proposal.
    pub kernel_outcome: Option<KernelOutcome>,
    /// Why the kernel refused, if it did.
    pub kernel_reasons: Vec<String>,
    /// What one base unit was worth when the decision was taken, scaled by
    /// [`PRICE_SCALE`](crate::PRICE_SCALE).
    ///
    /// **This is the field that makes a decision joinable to money**, and it
    /// exists because [`Outcome::first_price`](crate::Outcome::first_price) is
    /// the token's *first fill ever*. `creator_edge` acts around forty minutes
    /// after launch, by which point the token has usually already moved a long
    /// way — research 0009 says so in as many words: *"entry at the first fill
    /// is not Radar's entry"*. Measuring a selected cohort from the first fill
    /// would credit Radar with a move it was not present for, in the direction
    /// that flatters it.
    ///
    /// Taken from the smallest rung of the exit probe's realised price ladder,
    /// which is the price the decision itself was sized against — so it costs
    /// nothing extra to record and cannot disagree with the sizing.
    ///
    /// `None` when no exit was probed, which is every refusal that never
    /// reached the paid tier's quote.
    pub entry_price: Option<u64>,
    /// Digest of the candidate the decision was made from.
    ///
    /// The same digest [`radar_research`](https://github.com/hey-vera/radar)
    /// compares on replay. Stored so a recorded decision can be checked against
    /// the store later without a separate recording file.
    pub inputs_digest: String,
}

impl Decision {
    /// Whether a proposal was raised.
    #[must_use]
    pub const fn proposed(&self) -> bool {
        matches!(self.conclusion, Conclusion::Proposed)
    }

    /// Whether capital would have moved under the policy in force.
    ///
    /// Both halves are required: the strategy proposing and the kernel
    /// authorising. Reading either alone overstates what the system would have
    /// done, and the shipped policy refuses everything.
    #[must_use]
    pub const fn would_have_traded(&self) -> bool {
        matches!(
            (self.conclusion, self.kernel_outcome),
            (Conclusion::Proposed, Some(KernelOutcome::Authorised))
        )
    }

    /// The return from the decision's own entry price to a later price, in
    /// basis points.
    ///
    /// Signed, and `None` unless both prices exist — a decision with no entry
    /// price cannot be scored, and reporting it as zero would fold "not
    /// measurable" into "broke even", which is the whole population's median
    /// dressed up as a result.
    #[must_use]
    pub fn return_bps(&self, later_price: u64) -> Option<i64> {
        let entry = self.entry_price?;
        if entry == 0 {
            return None;
        }
        let entry = i128::from(entry);
        let delta = i128::from(later_price) - entry;
        i64::try_from(delta.saturating_mul(10_000) / entry).ok()
    }

    /// How old the token was when the decision was taken.
    #[must_use]
    pub fn token_age_slots(&self) -> u64 {
        self.decided_at.saturating_since(self.launch_slot).0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passed(reasons: &[&str]) -> Decision {
        Decision {
            mint: Address::new([1u8; 32]),
            creator: Address::new([2u8; 32]),
            decided_at: Slot(10_000),
            launch_slot: Slot(4_000),
            strategy: "creator_edge".to_owned(),
            strategy_version: "0.1.0".to_owned(),
            conclusion: Conclusion::Passed,
            reasons: reasons.iter().map(|r| (*r).to_owned()).collect(),
            notional_micro_usd: None,
            exit_capacity_micro_usd: None,
            assumed_round_trip_bps: 850,
            coordination: None,
            authority_prevalence: None,
            kernel_outcome: None,
            kernel_reasons: Vec::new(),
            entry_price: None,
            inputs_digest: "abc123".to_owned(),
        }
    }

    #[test]
    fn a_refusal_is_not_a_trade_and_never_looks_like_one() {
        let d = passed(&["CreatorNeverGraduated"]);
        assert!(!d.proposed());
        assert!(!d.would_have_traded());
        assert_eq!(d.notional_micro_usd, None, "nothing was sized");
    }

    #[test]
    fn a_proposal_the_kernel_refused_did_not_trade() {
        // The distinction that keeps the funnel honest. `Policy::CLOSED` refuses
        // every proposal, so a store counting proposals as trades would report
        // activity that never happened -- and every proposal in production so
        // far is in exactly this state.
        let mut d = passed(&[]);
        d.conclusion = Conclusion::Proposed;
        d.notional_micro_usd = Some(6_300_000);
        d.kernel_outcome = Some(KernelOutcome::Refused);
        d.kernel_reasons = vec!["NoAutonomy".to_owned()];

        assert!(d.proposed());
        assert!(
            !d.would_have_traded(),
            "the strategy proposing is not the kernel authorising"
        );
    }

    #[test]
    fn a_proposal_no_kernel_ever_saw_is_not_a_refusal() {
        // Absent is not zero. A missing kernel outcome means the proposal never
        // reached it, which is a gap in the pipeline rather than a verdict about
        // the token -- the same distinction NoExitSimulated draws.
        let mut d = passed(&[]);
        d.conclusion = Conclusion::Proposed;
        d.kernel_outcome = None;
        assert!(!d.would_have_traded());
        assert!(
            d.kernel_reasons.is_empty(),
            "no verdict means no reasons, not an empty refusal"
        );
    }

    #[test]
    fn only_an_authorised_proposal_would_have_traded() {
        let mut d = passed(&[]);
        d.conclusion = Conclusion::Proposed;
        d.kernel_outcome = Some(KernelOutcome::Authorised);
        assert!(d.would_have_traded());
    }

    #[test]
    fn a_return_is_measured_from_the_price_the_decision_saw() {
        // Not from the token's first fill. Radar acts around forty minutes after
        // launch, and crediting it with the move before it arrived is the error
        // that would make a selected cohort look good for free.
        let mut d = passed(&[]);
        d.entry_price = Some(1_000);
        assert_eq!(d.return_bps(1_500), Some(5_000), "+50%");
        assert_eq!(d.return_bps(500), Some(-5_000), "-50%");
        assert_eq!(d.return_bps(1_000), Some(0), "flat");
    }

    #[test]
    fn a_decision_with_no_entry_price_cannot_be_scored() {
        // Absent is not zero. Returning 0 would fold "not measurable" into
        // "broke even" -- and break-even is far better than this population's
        // median, so the error would flatter every cohort it touched.
        let d = passed(&["NoRoute"]);
        assert_eq!(d.entry_price, None);
        assert_eq!(d.return_bps(9_999), None);
    }

    #[test]
    fn a_zero_entry_price_is_refused_rather_than_dividing() {
        let mut d = passed(&[]);
        d.entry_price = Some(0);
        assert_eq!(d.return_bps(1_000), None);
    }

    #[test]
    fn a_total_loss_reads_as_minus_ten_thousand_bps() {
        // The floor of the scale, and the most common real outcome in this
        // market. Worth pinning: an off-by-one here would rescale every loss.
        let mut d = passed(&[]);
        d.entry_price = Some(1_000);
        assert_eq!(d.return_bps(0), Some(-10_000));
    }

    #[test]
    fn an_absurd_gain_is_refused_rather_than_wrapped() {
        // Prices are scaled by 1e18, so the intermediate product leaves i64 long
        // before any real return does. The property is that it never *wraps*: a
        // wrapped return reads as a catastrophic loss on the best outcome in the
        // sample, which is the direction that would quietly bury a winner.
        //
        // The first version of this test asserted a specific figure the author
        // had worked out by hand, and the figure was wrong -- LEARNINGS 2, an
        // assertion on a number nobody verified. What is actually guaranteed is
        // the sign, and that an unrepresentable answer is refused.
        let mut d = passed(&[]);
        d.entry_price = Some(1);
        for later in [u64::MAX, u64::MAX / 10_000, 1 << 62] {
            match d.return_bps(later) {
                None => {}
                Some(bps) => assert!(bps > 0, "a gain must never report as a loss: {bps}"),
            }
        }

        // And a return that does fit is reported rather than refused, so the
        // guard above is not passing by refusing everything.
        d.entry_price = Some(1_000_000_000);
        assert_eq!(d.return_bps(2_000_000_000), Some(10_000), "+100%");
    }

    #[test]
    fn token_age_is_measured_from_launch_not_from_the_epoch() {
        assert_eq!(passed(&[]).token_age_slots(), 6_000);
    }

    #[test]
    fn a_decision_taken_before_its_token_launched_reports_no_age() {
        // Saturating rather than panicking or wrapping: a store being backfilled
        // can legitimately hold a launch whose slot is above a decision's
        // watermark, and a wrapped age would read as eighteen quintillion slots.
        let mut d = passed(&[]);
        d.decided_at = Slot(1_000);
        d.launch_slot = Slot(4_000);
        assert_eq!(d.token_age_slots(), 0);
    }

    #[test]
    fn reasons_round_trip_as_written_even_when_the_code_no_longer_knows_them() {
        // The table outlives the code. A reason retired from the strategy has to
        // keep reading correctly from a file written before it was retired,
        // which an enum would not do -- it would refuse the row or remap it.
        let d = passed(&["AReasonThatNoLongerExists", "NorThisOne"]);
        let json = serde_json::to_string(&d).expect("serialises");
        let back: Decision = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(back.reasons, d.reasons);
        assert_eq!(back, d);
    }
}
