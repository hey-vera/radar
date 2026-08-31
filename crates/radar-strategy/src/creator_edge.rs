// SPDX-License-Identifier: Apache-2.0
//! The first strategy: creator track record, sized off the measured exit.
//!
//! It is first because creator identity is the one attribution in this market
//! that is **structurally verifiable on-chain**. Wallet reputation is inference;
//! the create instruction names the creator. That makes creator history the only
//! signal here whose *inputs* need no trust, whatever turns out to be true of its
//! predictive value.
//!
//! Whether it has predictive value is an open question, and this crate is how it
//! gets answered rather than assumed. Every decision records the thresholds that
//! produced it, so the research store can replay the same candidates at other
//! thresholds and report what the rule actually bought.
//!
//! # Sizing
//!
//! Notional is a fraction of *measured exit capacity*, never a fixed figure and
//! never a fraction of the portfolio. This is the plan's exit-first principle
//! made mechanical: the position a token can support is a property of the token,
//! not of how much capital happens to be available. The risk kernel then applies
//! the portfolio limits on top, and the smaller of the two wins — as it must,
//! since the kernel is the only thing with authority.

use radar_risk::{Action, Proposal};
use radar_types::MicroUsd;

use crate::avoidance::{PassReason, disqualify};
use crate::{Candidate, Decision, Strategy, lamports_to_micro_usd};

/// Thresholds, held as data so they can be varied in research.
///
/// Every field is an integer. A threshold expressed as a float is a threshold
/// that compares differently on a replay, and a replay that disagrees with the
/// recording is indistinguishable from a leak.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Thresholds {
    /// Measured launches below which a creator says nothing.
    ///
    /// One graduation out of one launch is not a 100% rate, it is one event.
    pub min_measured_launches: u64,
    /// Graduations per ten thousand measured launches, at minimum.
    pub min_graduation_bps: u64,
    /// Stillbirths per ten thousand above which a creator is refused.
    pub max_stillborn_bps: u64,
    /// Impact budget the exit capacity is measured at.
    pub capacity_impact_bps: u32,
    /// Share of measured capacity to take, per ten thousand.
    ///
    /// Well under half. Capacity is what the book showed at one moment, and the
    /// book is the first thing to leave.
    pub capacity_share_bps: u64,
    /// Notional below which a round trip is not worth its costs.
    ///
    /// **$1.00 sits in the most expensive cost band there is, and nothing knew
    /// that.** Research 0019 measures a leg under $2 at **1,521 bps** against
    /// about 225 above $20 -- so a position at this floor faces a round trip near
    /// 30%, while Radar's median proposal of $6.21 faces a tenth of that. The
    /// floor and the median live in bands an order of magnitude apart.
    ///
    /// Raising it above the cliff -- roughly 10,000,000 lamports -- is a change
    /// with a measurement behind it, in the direction that refuses more trades.
    /// It is left as a plan item rather than made here, because a threshold that
    /// decides which trades exist is a decision about money.
    pub min_notional: MicroUsd,
    /// Slots beyond which a token's own reading is too old to act on.
    ///
    /// Fast-moving: liquidity and activity change by the minute.
    pub max_token_age: u64,
    /// Slots beyond which a creator's record is too old to act on.
    ///
    /// Slow-moving, and far more generous. A creator's history is a count over
    /// months. Holding it to the token budget refuses candidates for a reason
    /// that is about Radar's measurement cadence rather than about the creator —
    /// which is what a single shared budget did to 88% of live candidates.
    pub max_creator_age: u64,
    /// Launches per day above which a creator is refused.
    ///
    /// The one threshold here that was set from a measurement rather than from
    /// first principles. `docs/research/0007` split 638 creators at candidate
    /// cuts of 10, 15, 20, 30, 50 and 100 prior launches; the first four
    /// separated the quieter and busier populations at 95% and the last two did
    /// not. Over the 1.72 days measured those cuts are roughly 5.8, 8.7, 11.6
    /// and 17.4 launches a day.
    ///
    /// Ten sits inside that range without being any of the tested points, which
    /// is deliberate: every cut across the range separated, so the rule does not
    /// depend on picking the one that flattered the sample. That insensitivity is
    /// the argument for the number, not the fit.
    pub max_launches_per_day: u64,
    /// Round-trip cost assumed when proposing, per ten thousand of notional.
    ///
    /// **This was 200 bps, described as "deliberately pessimistic". It was
    /// optimistic by roughly a factor of four, and the sentence claiming
    /// otherwise was the only thing standing between that and the kernel.**
    ///
    /// Measured on 2026-08-25 over 26,691 fills touching 200 pump.fun tokens
    /// launched in one hour, as the gap between the largest outflow and the
    /// largest inflow of each transaction — the protocol fee, the priority fee,
    /// account rent and slippage together, which is what a trader actually
    /// gives up:
    ///
    /// | | per leg, bps |
    /// |---|---|
    /// | median | 423 |
    /// | mean | 845 |
    /// | 90th percentile | 2,280 |
    ///
    /// Broken down by how many accounts the transaction moved, the two most
    /// common shapes — 14,312 of the fills — sit near 950–990 bps per leg, while
    /// larger shapes sit near 260–300. The spread is wide and the method is
    /// approximate: it attributes the largest inflow/outflow pair to the trade,
    /// so it also captures rent and any second hop.
    ///
    /// So this is 850 — twice the overall median of 423, rounded up rather than
    /// down, which is above every measured subgroup's 25th percentile and below
    /// the mean. It is not precise and is not claimed to be. It is four times
    /// better than a number that was wrong in the direction that costs money,
    /// and it is now wrong in the direction that refuses trades.
    ///
    /// The first draft of this change used 800, because it is a rounder number.
    /// `the_assumed_cost_is_not_below_what_a_round_trip_was_measured_to_cost`
    /// rejected it, which is what that test is for.
    ///
    /// Refining it is [`radar_exec`]'s job once the execution lane records what
    /// a fill actually cost, rather than what a cohort of other people's fills
    /// cost.
    ///
    /// **The query behind the table above was lost, and has been rebuilt.** See
    /// research 0019 and `radar cost`. That measurement does **not** move this
    /// constant -- it covers all pump.fun trades where the original covered
    /// trades on 200 fresh launches, which is a broader and cheaper population,
    /// and lowering a cost estimate is the direction that launders a trade past
    /// the kernel.
    ///
    /// What it does establish is that **this number should not be one number**.
    /// Measured by notional, a leg under $2 costs 1,521 bps and a leg above $20
    /// costs about 225 -- a fixed component, exactly as the "rent and any second
    /// hop" in the method above implies. Applying a single rate to a $1 position
    /// and a $600 one is arithmetic rather than a cost model.
    ///
    /// [`radar_exec`]: https://github.com/hey-vera/radar
    pub assumed_round_trip_bps: u64,
}

impl Thresholds {
    /// The starting values.
    ///
    /// Not tuned. Tuning them against data that has not been collected yet is
    /// how a backtest gets fitted to noise; these are placed where they are
    /// defensible from first principles and left there until the research store
    /// can argue otherwise.
    pub const DEFAULT: Self = Self {
        // Five is the same floor the creator_track_record instrument uses.
        min_measured_launches: 5,
        // 5%. The population base rate is roughly 1%, so this asks for several
        // times better than average rather than for excellence.
        min_graduation_bps: 500,
        // 90%. Almost every creator is above this; it catches the spammers.
        max_stillborn_bps: 9_000,
        // 1%. Tighter than the exit report's widest probe.
        capacity_impact_bps: 100,
        // A fifth of what the book showed.
        capacity_share_bps: 2_000,
        // Measured, not assumed. See the field's documentation.
        max_launches_per_day: 10,
        min_notional: MicroUsd::DOLLAR,
        // ~40 minutes at 2.5 slots a second.
        max_token_age: 6_000,
        // ~24 hours. Deliberately wider than the outcome pass's largest
        // checkpoint gap, so a creator's record is never stale merely because
        // the next scheduled measurement has not come round yet.
        max_creator_age: 216_000,
        // 8.5%. Measured, not assumed -- see the field's documentation. Two legs
        // at the median of 26,691 observed fills, rounded *up*: a cost estimate
        // rounded down is the direction that launders a trade past the kernel.
        assumed_round_trip_bps: 850,
    };
}

/// Proposes on creators with a measured record, sized off the measured exit.
#[derive(Clone, Copy, Debug)]
pub struct CreatorEdge {
    /// The thresholds in force.
    pub thresholds: Thresholds,
}

impl Default for CreatorEdge {
    fn default() -> Self {
        Self {
            thresholds: Thresholds::DEFAULT,
        }
    }
}

impl Strategy for CreatorEdge {
    fn name(&self) -> &'static str {
        "creator_edge"
    }

    fn version(&self) -> &'static str {
        "0.1.0"
    }

    fn consider(&self, candidate: &Candidate) -> Decision {
        let t = &self.thresholds;
        let mut reasons = disqualify(candidate);

        let record = &candidate.creator_record;
        if record.measured < t.min_measured_launches {
            reasons.push(PassReason::CreatorUnproven);
        } else {
            // Only meaningful above the sample floor. Below it these rates are
            // arithmetic on noise, and reporting them as findings would give a
            // creator with one launch a verdict.
            // Organic graduations only. Gating on the undifferentiated rate
            // selects for creators whose curves complete inside the launch
            // block, which is the bundling signature rather than evidence that
            // anyone wanted the token.
            match record.organic_graduation_bps() {
                // Distinguished, because they are different findings. Never is
                // a fact about the creator; rarely is a fact about where this
                // threshold happens to sit, and research will want to move it.
                Some(0) => reasons.push(PassReason::CreatorNeverGraduated),
                Some(bps) if bps < t.min_graduation_bps => {
                    reasons.push(PassReason::CreatorGraduatesTooRarely);
                }
                _ => {}
            }
            if record
                .stillborn_bps()
                .is_some_and(|bps| bps > t.max_stillborn_bps)
            {
                reasons.push(PassReason::CreatorMostlyStillborn);
            }
        }

        // Outside the sample-floor branch on purpose: a launch rate needs no
        // measured outcomes, only launches, so it is knowable for creators whose
        // record is otherwise too thin to say anything about.
        if record.launches_too_fast(t.max_launches_per_day) {
            reasons.push(PassReason::CreatorLaunchesTooFast);
        }

        // Only an actionable verdict refuses. `None` is "not looked at" and must
        // not read as clean, but it also cannot refuse on its own -- a candidate
        // that was never examined has no evidence against it, and inventing some
        // would refuse the entire population the moment the fetch broke.
        if candidate
            .coordination
            .is_some_and(radar_graph::Coordination::is_actionable)
        {
            reasons.push(PassReason::LaunchLooksCoordinated);
        }

        let age_of = |observed| candidate.as_of.slot().saturating_since(observed).get();
        if age_of(candidate.token_observed_at) > t.max_token_age {
            reasons.push(PassReason::TokenReadingTooOld);
        }
        if age_of(candidate.creator_observed_at) > t.max_creator_age {
            reasons.push(PassReason::CreatorRecordTooOld);
        }

        // Sizing runs even when reasons exist, because a candidate that fails
        // only on capacity should say so rather than be hidden behind an
        // earlier failure. The research store wants the complete list.
        let notional = size(candidate, t);
        match notional {
            None => {
                // Absent only when the inputs sizing needs are missing, and
                // whichever one is missing already pushed its own reason.
                debug_assert!(
                    !reasons.is_empty(),
                    "sizing failed with nothing to explain it"
                );
            }
            Some(n) if n < t.min_notional => reasons.push(PassReason::CapacityBelowFloor),
            Some(_) => {}
        }

        if !reasons.is_empty() {
            return Decision::pass(reasons);
        }

        let Some(notional) = notional else {
            return Decision::pass(vec![PassReason::CapacityBelowFloor]);
        };

        Decision::Propose(Box::new(Proposal {
            mint: candidate.mint,
            creator: candidate.creator,
            action: Action::Buy,
            notional,
            estimated_round_trip_cost: MicroUsd(
                notional.get().saturating_mul(t.assumed_round_trip_bps) / 10_000,
            ),
            oldest_input_slot: candidate.oldest_input_slot(),
            simulated_exit_capacity: capacity(candidate, t),
        }))
    }
}

/// The measured exit capacity as a notional, or `None` if it cannot be computed.
fn capacity(candidate: &Candidate, t: &Thresholds) -> Option<MicroUsd> {
    let lamports = candidate
        .exit
        .as_ref()?
        .capacity_lamports(t.capacity_impact_bps)?;
    let price = candidate.sol_price_micro_usd?;
    Some(lamports_to_micro_usd(lamports, price))
}

/// The notional to propose: a share of measured capacity.
fn size(candidate: &Candidate, t: &Thresholds) -> Option<MicroUsd> {
    let capacity = capacity(candidate, t)?;
    Some(MicroUsd(
        capacity.get().saturating_mul(t.capacity_share_bps) / 10_000,
    ))
}

/// Median round-trip execution cost measured on 2026-08-25, in basis points.
///
/// 423 bps per leg over 26,691 fills touching 200 pump.fun tokens launched in one
/// hour, doubled. Kept as a named constant so the floor below reads as a
/// measurement rather than as a magic number.
#[cfg(test)]
const MEASURED_MEDIAN_ROUND_TRIP_BPS: u64 = 846;

/// The assumption may not sit below what a round trip was measured to cost.
///
/// Compile-time, because the whole risk is someone lowering it under pressure to
/// make something propose — and a check that only runs when the tests are run is
/// a check that can be skipped by not running them.
#[cfg(test)]
const _: () = assert!(
    Thresholds::DEFAULT.assumed_round_trip_bps >= MEASURED_MEDIAN_ROUND_TRIP_BPS,
    "assumed_round_trip_bps is below the measured median cost of a round trip"
);

#[cfg(test)]
mod tests {
    use radar_asof::AsOf;
    use radar_sim::ExitReport;
    use radar_sim::exit::{Confidence, QuotePoint};
    use radar_types::{Address, Slot};

    use super::*;
    use crate::CreatorRecord;

    /// A candidate that passes everything, for tests to spoil one field at a time.
    fn good() -> Candidate {
        Candidate {
            mint: Address::new([7u8; 32]),
            creator: Address::new([8u8; 32]),
            launch_slot: Slot(400_000),
            coordination: None,
            as_of: AsOf::at(Slot(500_000)),
            exit: Some(ExitReport {
                mint: Address::new([7u8; 32]),
                structure: None,
                curve: vec![
                    QuotePoint {
                        size_tokens: 1_000_000,
                        out_lamports: 500_000_000,
                        impact_bps: 30,
                    },
                    QuotePoint {
                        size_tokens: 5_000_000,
                        out_lamports: 2_400_000_000,
                        impact_bps: 80,
                    },
                    // Past the impact budget: must not be counted as capacity.
                    QuotePoint {
                        size_tokens: 20_000_000,
                        out_lamports: 8_000_000_000,
                        impact_bps: 900,
                    },
                ],
                no_route_at: Vec::new(),
                structural_threats: Vec::new(),
                can_be_stopped: false,
                confidence: Confidence::Measured,
            }),
            creator_record: CreatorRecord {
                launches: 20,
                measured: 20,
                stillborn: 10,
                graduated: 3,
                graduated_organic: 3,
                launches_per_day: None,
            },
            sol_price_micro_usd: Some(MicroUsd::from_dollars(200.0)),
            token_observed_at: Slot(499_000),
            creator_observed_at: Slot(499_000),
        }
    }

    #[test]
    fn a_qualified_candidate_produces_a_proposal() {
        let d = CreatorEdge::default().consider(&good());
        let Decision::Propose(p) = d else {
            panic!("expected a proposal, got {d:?}");
        };
        assert_eq!(p.action, Action::Buy);
        assert_eq!(p.mint, Address::new([7u8; 32]));
    }

    #[test]
    fn capacity_stops_at_the_impact_budget() {
        // 2.4 SOL at 80bps qualifies; 8 SOL at 900bps does not. At $200 that is
        // $480 of capacity, and the proposal takes a fifth.
        let Decision::Propose(p) = CreatorEdge::default().consider(&good()) else {
            panic!("expected a proposal");
        };
        assert_eq!(
            p.simulated_exit_capacity,
            Some(MicroUsd::from_dollars(480.0))
        );
        assert_eq!(p.notional, MicroUsd::from_dollars(96.0));
    }

    #[test]
    fn the_proposal_never_claims_the_whole_book() {
        // Capacity is what the book showed at one moment, and the book is the
        // first thing to leave. Sizing at capacity would assume it stays.
        let Decision::Propose(p) = CreatorEdge::default().consider(&good()) else {
            panic!("expected a proposal");
        };
        let capacity = p.simulated_exit_capacity.expect("qualified");
        assert!(p.notional.get() * 2 < capacity.get());
    }

    #[test]
    fn an_unproven_creator_is_passed_over() {
        let mut c = good();
        c.creator_record = CreatorRecord {
            launches: 3,
            measured: 3,
            stillborn: 0,
            graduated: 3,
            graduated_organic: 3,
            launches_per_day: None,
        };
        // A perfect record over three launches is three events, not a rate.
        let d = CreatorEdge::default().consider(&c);
        assert_eq!(d.reasons(), [PassReason::CreatorUnproven]);
    }

    #[test]
    fn a_creator_who_never_graduates_is_passed_over() {
        let mut c = good();
        c.creator_record = CreatorRecord {
            launches: 40,
            measured: 40,
            stillborn: 20,
            graduated: 0,
            graduated_organic: 0,
            launches_per_day: None,
        };
        assert!(
            CreatorEdge::default()
                .consider(&c)
                .reasons()
                .contains(&PassReason::CreatorNeverGraduated)
        );
    }

    #[test]
    fn graduating_rarely_is_a_different_finding_from_never_graduating() {
        // One is a fact about the creator, the other about the threshold. A
        // research pass that moves the threshold needs to tell them apart.
        let mut c = good();
        c.creator_record = CreatorRecord {
            launches: 60,
            measured: 60,
            stillborn: 30,
            graduated: 1,
            graduated_organic: 1,
            launches_per_day: None,
        };
        let reasons = CreatorEdge::default().consider(&c).reasons().to_vec();
        assert!(reasons.contains(&PassReason::CreatorGraduatesTooRarely));
    }

    #[test]
    fn a_launch_block_that_looks_arranged_is_refused() {
        // 68% of launches that graduated within three slots had exactly this
        // shape, against 5% of launches that never graduated. It is a refusal
        // and not an entry: the same observation makes an instant graduation
        // 11.7x likelier, and an instant graduation means the supply is already
        // held by whoever arranged it.
        let c = good().with_coordination(radar_graph::Coordination::Likely);
        let reasons = CreatorEdge::default().consider(&c).reasons().to_vec();
        assert!(
            reasons.contains(&PassReason::LaunchLooksCoordinated),
            "expected a coordination refusal, got {reasons:?}"
        );
    }

    #[test]
    fn a_suspected_launch_block_is_not_refused_on_its_own() {
        // `Suspected` fires on 13% of all launches at a four-fold enrichment.
        // Worth recording, too blunt to refuse on -- the failure research 0004
        // recorded for the same-slot-buy heuristic, which fired on 91%.
        let c = good().with_coordination(radar_graph::Coordination::Suspected);
        let reasons = CreatorEdge::default().consider(&c).reasons().to_vec();
        assert!(!reasons.contains(&PassReason::LaunchLooksCoordinated));
    }

    #[test]
    fn an_unremarkable_launch_block_is_not_refused() {
        let c = good().with_coordination(radar_graph::Coordination::Unremarkable);
        let reasons = CreatorEdge::default().consider(&c).reasons().to_vec();
        assert!(!reasons.contains(&PassReason::LaunchLooksCoordinated));
    }

    #[test]
    fn a_launch_block_that_was_never_looked_at_neither_passes_nor_refuses() {
        // The subtle one. `None` must not read as clean -- but it must not
        // refuse either, or a broken fetch would disqualify the whole population
        // and look like a market with nothing in it. The absence is carried, and
        // whatever consumes the decision can see that no look was paid for.
        let c = good();
        assert_eq!(c.coordination, None);
        let reasons = CreatorEdge::default().consider(&c).reasons().to_vec();
        assert!(
            !reasons.contains(&PassReason::LaunchLooksCoordinated),
            "an unexamined launch has no evidence against it"
        );
    }

    #[test]
    fn a_creator_who_launches_faster_than_the_measured_threshold_is_refused() {
        // Measured over 638 creators: tokens from creators who launch more
        // graduate less often per launch, and every cut between roughly six and
        // seventeen a day separated the populations at 95%.
        let mut c = good();
        c.creator_record.launches_per_day = Some(Thresholds::DEFAULT.max_launches_per_day + 1);
        let reasons = CreatorEdge::default().consider(&c).reasons().to_vec();
        assert!(
            reasons.contains(&PassReason::CreatorLaunchesTooFast),
            "expected a launch-rate refusal, got {reasons:?}"
        );
    }

    #[test]
    fn a_creator_exactly_at_the_threshold_is_not_refused_for_it() {
        // The boundary belongs to the permitted side. Refusing at the threshold
        // would make the documented number mean something other than it says.
        let mut c = good();
        c.creator_record.launches_per_day = Some(Thresholds::DEFAULT.max_launches_per_day);
        let reasons = CreatorEdge::default().consider(&c).reasons().to_vec();
        assert!(!reasons.contains(&PassReason::CreatorLaunchesTooFast));
    }

    #[test]
    fn an_unmeasurable_launch_rate_is_not_grounds_for_refusal() {
        // Absent is not zero, and it is also not guilt. A creator seen for too
        // little chain to divide by has no rate, and refusing on one that could
        // not be computed would be inventing evidence -- the sample floor already
        // handles creators nobody knows anything about.
        let mut c = good();
        c.creator_record.launches_per_day = None;
        let reasons = CreatorEdge::default().consider(&c).reasons().to_vec();
        assert!(!reasons.contains(&PassReason::CreatorLaunchesTooFast));
    }

    #[test]
    fn the_launch_rate_is_judged_without_waiting_for_measured_outcomes() {
        // It needs launches, not outcomes, so it is knowable for a creator whose
        // record is otherwise too thin to say anything about. Putting it inside
        // the sample-floor branch would have hidden the one signal that does not
        // need the floor.
        let mut c = good();
        c.creator_record = CreatorRecord {
            launches: 200,
            measured: 0,
            stillborn: 0,
            graduated: 0,
            graduated_organic: 0,
            launches_per_day: Some(120),
        };
        let reasons = CreatorEdge::default().consider(&c).reasons().to_vec();
        assert!(
            reasons.contains(&PassReason::CreatorUnproven),
            "still unproven"
        );
        assert!(
            reasons.contains(&PassReason::CreatorLaunchesTooFast),
            "and the rate is knowable anyway: {reasons:?}"
        );
    }

    #[test]
    fn a_creator_whose_tokens_only_graduate_instantly_is_refused() {
        // The finding this rule exists for. Every graduation in the live store
        // completed within three slots of launch -- a whole bonding curve bought
        // out in the launch block, which is a bundle rather than demand. Gating
        // on the undifferentiated rate would rank this creator top; gating on the
        // organic rate refuses them.
        let mut c = good();
        c.creator_record = CreatorRecord {
            launches: 40,
            measured: 40,
            stillborn: 0,
            graduated: 40,
            graduated_organic: 0,
            launches_per_day: None,
        };
        let record = c.creator_record;
        assert_eq!(record.graduation_bps(), Some(10_000), "flawless, on paper");
        assert_eq!(record.organic_graduation_bps(), Some(0));
        assert_eq!(record.graduated_instant(), 40);

        let reasons = CreatorEdge::default().consider(&c).reasons().to_vec();
        assert!(
            reasons.contains(&PassReason::CreatorNeverGraduated),
            "a 100% instant graduation rate must not read as a perfect record: {reasons:?}"
        );
    }

    #[test]
    fn the_same_creator_passes_the_gate_once_the_graduations_are_organic() {
        // The other half of the pair. Without this, the rule above would also be
        // satisfied by a gate that refuses everyone.
        let mut c = good();
        c.creator_record = CreatorRecord {
            launches: 40,
            measured: 40,
            stillborn: 0,
            graduated: 40,
            graduated_organic: 40,
            launches_per_day: None,
        };
        let reasons = CreatorEdge::default().consider(&c).reasons().to_vec();
        assert!(
            !reasons.contains(&PassReason::CreatorNeverGraduated)
                && !reasons.contains(&PassReason::CreatorGraduatesTooRarely),
            "organic graduations must clear the graduation gate: {reasons:?}"
        );
        assert!(!reasons.contains(&PassReason::CreatorNeverGraduated));
    }

    #[test]
    fn a_spammer_is_passed_over_even_with_graduations() {
        // 500 launches, 495 stillborn, 5 graduated: above the graduation floor
        // on the count but the record is a shotgun, not a skill.
        let mut c = good();
        c.creator_record = CreatorRecord {
            launches: 500,
            measured: 500,
            stillborn: 495,
            graduated: 5,
            graduated_organic: 5,
            launches_per_day: None,
        };
        let reasons = CreatorEdge::default().consider(&c).reasons().to_vec();
        assert!(reasons.contains(&PassReason::CreatorMostlyStillborn));
    }

    #[test]
    fn a_token_whose_reading_is_old_is_passed_over() {
        let mut c = good();
        c.token_observed_at = Slot(400_000);
        assert!(
            CreatorEdge::default()
                .consider(&c)
                .reasons()
                .contains(&PassReason::TokenReadingTooOld)
        );
    }

    #[test]
    fn a_creator_record_measured_hours_ago_is_still_good_enough() {
        // The bug this split fixes. Outcomes are measured at checkpoints one
        // hour, six hours and a day after launch, so between checkpoints every
        // creator record is hours old — and under one shared budget that read as
        // "too stale to act on" for 88% of live candidates. A creator's history
        // is a count over months; six hours does not move it.
        let mut c = good();
        c.creator_observed_at = Slot(c.as_of.slot().get() - 54_000);
        let reasons = CreatorEdge::default().consider(&c).reasons().to_vec();
        assert!(
            !reasons.contains(&PassReason::CreatorRecordTooOld),
            "got {reasons:?}"
        );
    }

    #[test]
    fn a_creator_record_from_last_week_is_too_old() {
        // The budget is generous, not absent.
        let mut c = good();
        c.creator_observed_at = Slot(1);
        assert!(
            CreatorEdge::default()
                .consider(&c)
                .reasons()
                .contains(&PassReason::CreatorRecordTooOld)
        );
    }

    #[test]
    fn the_proposal_carries_the_stalest_ingredient_not_the_freshest() {
        // The strategy budgets the two classes separately; the kernel checks the
        // decision as a whole, and a decision is only as current as its oldest
        // mutable input.
        let mut c = good();
        c.creator_observed_at = Slot(c.as_of.slot().get() - 50_000);
        let Decision::Propose(p) = CreatorEdge::default().consider(&c) else {
            panic!("expected a proposal");
        };
        assert_eq!(p.oldest_input_slot, c.creator_observed_at);
    }

    #[test]
    fn a_position_too_small_to_pay_its_costs_is_passed_over() {
        let mut c = good();
        // A curve that only supports dust.
        c.exit.as_mut().expect("has exit").curve = vec![QuotePoint {
            size_tokens: 1_000,
            out_lamports: 1_000,
            impact_bps: 10,
        }];
        assert!(
            CreatorEdge::default()
                .consider(&c)
                .reasons()
                .contains(&PassReason::CapacityBelowFloor)
        );
    }

    #[test]
    fn a_candidate_with_no_exit_never_proposes() {
        // The invariant the risk kernel also enforces, held here too so a
        // proposal with no exit cannot even be constructed for it to refuse.
        let mut c = good();
        c.exit = None;
        assert!(!CreatorEdge::default().consider(&c).is_proposal());
    }

    #[test]
    fn every_proposal_carries_a_simulated_exit_capacity() {
        // The kernel refuses None. A strategy that emitted one would be
        // generating guaranteed refusals, which reads as a broken pipeline
        // rather than as a strategy declining to trade.
        let Decision::Propose(p) = CreatorEdge::default().consider(&good()) else {
            panic!("expected a proposal");
        };
        assert!(p.simulated_exit_capacity.is_some());
    }

    #[test]
    fn the_strategy_is_pure() {
        // Same candidate, same decision, however many times and in whatever
        // order. This is what makes a recorded decision replayable, and the
        // property that would silently rot if anything here read a clock.
        let c = good();
        let s = CreatorEdge::default();
        let first = s.consider(&c);
        for _ in 0..64 {
            assert_eq!(s.consider(&c), first);
        }
    }

    #[test]
    fn a_candidate_failing_several_ways_reports_all_of_them() {
        let mut c = good();
        c.creator_record = CreatorRecord::default();
        c.exit = None;
        c.token_observed_at = Slot(0);
        c.creator_observed_at = Slot(0);
        let reasons = CreatorEdge::default().consider(&c).reasons().to_vec();
        assert!(reasons.contains(&PassReason::NoExitSimulated));
        assert!(reasons.contains(&PassReason::CreatorUnproven));
        assert!(reasons.contains(&PassReason::TokenReadingTooOld));
        assert!(reasons.contains(&PassReason::CreatorRecordTooOld));
    }

    #[test]
    fn thresholds_are_data_so_research_can_vary_them() {
        // The reason the rule is falsifiable rather than a belief: the same
        // candidates can be re-run at other thresholds and compared.
        let mut c = good();
        c.creator_record = CreatorRecord {
            launches: 6,
            measured: 6,
            stillborn: 2,
            graduated: 1,
            graduated_organic: 1,
            launches_per_day: None,
        };
        let strict = CreatorEdge {
            thresholds: Thresholds {
                min_measured_launches: 50,
                ..Thresholds::DEFAULT
            },
        };
        assert!(!strict.consider(&c).is_proposal());
        assert!(CreatorEdge::default().consider(&c).is_proposal());
    }

    #[test]
    fn the_assumed_cost_is_pessimistic_enough_to_reach_the_kernel() {
        // The kernel refuses on round-trip cost as a share of notional, so an
        // optimistic estimate here launders a bad trade past that check. The
        // previous value was 200 bps and its documentation called that
        // pessimistic; 26,691 measured fills put the median at 423 bps *per
        // leg*, so it was optimistic by about four times and the claim was the
        // only guard against it.
        let Decision::Propose(p) = CreatorEdge::default().consider(&good()) else {
            panic!("expected a proposal");
        };
        assert!(p.estimated_round_trip_cost > MicroUsd::ZERO);
        assert_eq!(p.estimated_round_trip_cost, MicroUsd::from_dollars(8.16));
    }

    #[test]
    fn the_assumed_cost_is_not_below_what_a_round_trip_was_measured_to_cost() {
        // The property, rather than the number. A future edit that lowers this
        // to make something propose is making a decision about money, and this
        // is where it has to argue with a measurement instead of with a comment.
        //
        // Checked on the proposal the strategy actually emits, so it holds for
        // what reaches the kernel rather than only for the constant. The floor
        // itself is asserted at compile time beside the threshold.
        let Decision::Propose(p) = CreatorEdge::default().consider(&good()) else {
            panic!("expected a proposal");
        };
        let notional = p.notional.get();
        let implied_bps = p.estimated_round_trip_cost.get() * 10_000 / notional;
        assert!(
            implied_bps >= MEASURED_MEDIAN_ROUND_TRIP_BPS,
            "the proposal implies {implied_bps} bps against a measured {MEASURED_MEDIAN_ROUND_TRIP_BPS}"
        );
    }
}
