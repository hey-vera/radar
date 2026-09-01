// SPDX-License-Identifier: Apache-2.0
//! `radar consider` — the whole decision lane, on recorded data.
//!
//! Runs everything Radar knows how to do, in order, against tokens actually
//! recorded on this instance: assemble candidates at the watermark, apply the
//! strategy, pay for exit analysis only where it could change an answer, then
//! put whatever survives through the risk kernel.
//!
//! It commits nothing. Under [`Policy::CLOSED`] — the default, and what ships —
//! the kernel refuses everything, so the output is a complete account of what
//! the system *would* do and why it would not do it. That is the point: the
//! evidence Josh gated deploying capital on is exactly this report, run over
//! enough days to mean something.
//!
//! # The tiering is real, not described
//!
//! The strategy runs twice. The first pass costs nothing: no exit report, so
//! every candidate fails on at least `NoExitSimulated`, and the ones failing on
//! *only* that are the ones where a paid look could change the answer. The
//! second pass spends a network call on those alone.
//!
//! At ~35,000 launches a day this is the difference between a few calls and tens
//! of thousands. It also makes the tier falsifiable: the report says how many
//! candidates the paid tier was spent on and how many it changed.

use std::collections::BTreeMap;

use radar_asof::AsOf;
use radar_backfill::launch_block::CryptoHouseBlocks;
use radar_graph::LaunchBlockSource;
use radar_risk::{Policy, PortfolioState, Proposal, Verdict, evaluate};
use radar_sim::{JupiterQuoter, RpcClient};
use radar_store::Reader;
use radar_strategy::{Candidate, CreatorEdge, Decision, PassReason, Strategy, Universe, universe};
use radar_types::{Address, MicroUsd};

/// How many candidates the paid tier will be spent on in one pass.
///
/// A cap rather than a budget in dollars, because the calls here are free —
/// Jupiter's lite tier and a public RPC. It exists to bound *time*, and to stop
/// a first run against a large store from making thousands of requests to
/// somebody's free endpoint.
const PAID_TIER_CAP: usize = 25;

/// Runs the lane.
///
/// # Errors
///
/// Returns a message if the store cannot be read or has recorded nothing.
pub fn run(
    reader: &Reader,
    window: u64,
    cap: usize,
    record_to: Option<&str>,
) -> Result<(), String> {
    let watermark = reader
        .watermark()
        .map_err(|e| format!("cannot read the store: {e}"))?
        .ok_or("the store has recorded nothing yet")?;
    let as_of = AsOf::at(watermark);

    let universe = universe(reader, as_of).map_err(|e| format!("cannot read the store: {e}"))?;
    let recent = universe.recent(window);

    println!("watermark    : slot {watermark}");
    println!("launches     : {} recorded", universe.launches.len());
    println!("creators     : {}", universe.creators.len());
    println!(
        "considering  : {} launched within {window} slots\n",
        recent.len()
    );

    if recent.is_empty() {
        println!("Nothing recent enough to consider. Widen --window, or let the recorder run.");
        return Ok(());
    }

    // Tier 0 and 1: free. No exit report, so every candidate fails at least on
    // NoExitSimulated, and the interesting ones fail on nothing else.
    let strategy = CreatorEdge::default();
    let mut tally: BTreeMap<PassReason, usize> = BTreeMap::new();
    let mut worth_paying_for = Vec::new();

    for mint in &recent {
        let Some(candidate) = universe.candidate(mint, None, None) else {
            continue;
        };
        let reasons = strategy.consider(&candidate).reasons().to_vec();
        for reason in &reasons {
            *tally.entry(*reason).or_default() += 1;
        }
        // NoExitSimulated and NoPrice are the two that a paid look removes.
        // Anything else failing means the answer cannot change, so the call
        // would buy nothing.
        if reasons
            .iter()
            .all(|r| matches!(r, PassReason::NoExitSimulated | PassReason::NoPrice))
        {
            worth_paying_for.push(*mint);
        }
    }

    println!(
        "free tier — why {} candidates were passed over:",
        recent.len()
    );
    for (reason, count) in &tally {
        println!("  {count:>6}  {reason:?}");
    }

    println!(
        "\n{} candidate(s) fail on nothing a paid look cannot resolve.",
        worth_paying_for.len()
    );
    if worth_paying_for.is_empty() {
        println!("Spending on exit analysis would change no answer, so nothing is spent.");
        return Ok(());
    }

    let budget = worth_paying_for.len().min(cap);
    if worth_paying_for.len() > budget {
        println!("Examining the first {budget} of them this pass.");
    }

    let quoter = JupiterQuoter::default();
    let Some(sol_price) = radar_sim::sol_price_micro_usd(&quoter) else {
        // A wrong SOL price silently rescales every position in the system, so
        // an absent one stops the pass rather than defaulting.
        println!("\nSOL price unavailable — refusing to size anything without it.");
        return Ok(());
    };
    println!("SOL price    : ${:.2}\n", price_dollars(sol_price));

    let mut examined: Vec<(radar_store::Decision, Address)> = Vec::new();
    let proposals = paid_tier(
        &universe,
        &strategy,
        worth_paying_for.iter().take(budget),
        &quoter,
        sol_price,
        watermark,
        &mut examined,
    );

    let verdicts_by_mint = verdicts(&proposals, watermark, reader);

    if let Some(dir) = record_to {
        // The kernel's verdict is folded in only now, because a decision is not
        // complete until the thing with the authority has seen it.
        for (record, mint) in &mut examined {
            if let Some(v) = verdicts_by_mint.get(mint) {
                record.kernel_outcome = Some(match v {
                    Verdict::Authorised(_) => radar_store::KernelOutcome::Authorised,
                    Verdict::Refused { .. } => radar_store::KernelOutcome::Refused,
                });
                if let Verdict::Refused { reasons } = v {
                    record.kernel_reasons = reasons.iter().map(|r| format!("{r:?}")).collect();
                }
            }
        }
        write_decisions(dir, &examined)?;
    }
    Ok(())
}

/// Appends the examined candidates to the store.
///
/// # Errors
///
/// Returns a message if the store cannot be opened or written. A recording
/// failure is loud rather than swallowed: the whole point of the pass is the
/// record, so a run that printed its findings and failed to keep them has not
/// done the job.
fn write_decisions(dir: &str, examined: &[(radar_store::Decision, Address)]) -> Result<(), String> {
    let mut writer = radar_store::Writer::open(dir, 512)
        .map_err(|e| format!("cannot open the store for writing: {e}"))?;
    for (record, _) in examined {
        writer
            .append_decision(record.clone())
            .map_err(|e| format!("cannot record a decision: {e}"))?;
    }
    writer
        .flush()
        .map_err(|e| format!("cannot flush decisions: {e}"))?;
    println!(
        "
recorded {} decision(s) to {dir}/decisions",
        examined.len()
    );
    Ok(())
}

/// The kernel's view of the portfolio, from the positions on record.
///
/// A read that fails is treated as *no positions*, and that is safe only
/// because it is also true: nothing writes a position yet, so an empty store
/// and an unreadable one hold the same thing. When something does trade this
/// must become a refusal — a kernel handed an empty portfolio because the read
/// failed would size against capital it cannot see.
fn portfolio_state(reader: &Reader, watermark: radar_types::Slot) -> PortfolioState {
    let rows = reader
        .read_positions(AsOf::at(watermark))
        .inspect_err(|e| eprintln!("  positions unreadable, treating as none: {e}"))
        .unwrap_or_default();

    radar_strategy::state_from(
        &radar_store::fold_positions(rows),
        watermark,
        radar_strategy::Operator {
            halted: false,
            consecutive_failures: 0,
        },
    )
}

/// The paid tier: the two calls that cost money, in the order that spends least.
///
/// Split out because it is the part with a budget attached, and because reading
/// the free tier's tally should not mean scrolling past the spending.
fn paid_tier<'a>(
    universe: &Universe,
    strategy: &CreatorEdge,
    mints: impl Iterator<Item = &'a Address>,
    quoter: &JupiterQuoter,
    sol_price: MicroUsd,
    watermark: radar_types::Slot,
    examined: &mut Vec<(radar_store::Decision, Address)>,
) -> Vec<Proposal> {
    let rpc = RpcClient::default();
    // The clock stays at the edge. The launch-block window is a fixed width
    // back from *now* rather than from a calendar date, so its cost does not
    // grow every day -- see `launch_block::LOOKBACK_HOURS`.
    let blocks = CryptoHouseBlocks::new(
        radar_backfill::cryptohouse::Client::default(),
        &radar_store::from_epoch(radar_store::now_epoch()),
    );
    let mut proposals = Vec::new();
    let mut refused_on_shape = 0usize;
    // Counted apart from "looked and found clean". A fetch that fails leaves the
    // verdict absent, the strategy correctly declines to refuse on an absence,
    // and the gate is then silently off. Without this a broken source and a
    // clean population produce identical output.
    //
    // The shapes are kept rather than counted, because the count alone had the
    // same defect one level up: `read: 25, unreadable: 0` is what a detector
    // whose constant has moved prints too. See [`render_shapes`].
    let mut shapes = radar_graph::Distribution::new();
    let mut look_failed = 0usize;

    let prevalence_table = prevalence_table_of(&blocks);

    for mint in mints {
        // The launch-block look runs first because it is the cheaper of the two
        // paid calls and it can end the question. Probing the exit of a token
        // whose curve was already bought out by whoever arranged it is money
        // spent to be told something the block said for less.
        let coordination = match universe.launches.get(mint) {
            Some(facts) => match blocks.shape_at(mint, facts.slot) {
                Ok(shape) => {
                    shapes.observe(shape);
                    Some(radar_graph::assess(shape).coordination)
                }
                Err(e) => {
                    look_failed += 1;
                    eprintln!("  {mint}  launch block unreadable: {e}");
                    None
                }
            },
            None => None,
        };

        // Only asked when the table can answer. A second cheap single-block
        // read, skipped entirely when the answer would be discarded.
        let prevalence = prevalence_table.as_ref().and_then(|table| {
            let facts = universe.launches.get(mint)?;
            let authorities = blocks.authorities_at(mint, facts.slot).ok()?;
            table.strongest_of(&authorities)
        });

        if coordination.is_some_and(radar_graph::Coordination::is_actionable) {
            refused_on_shape += 1;
            println!("  {mint}  launch block looks arranged — not probing the exit");
            // Recorded before the `continue`, because this is a decision and it
            // is the strongest one Radar makes. Skipping it made the decisions
            // table structurally incapable of holding a `Likely` verdict, so a
            // monitor counting them read 0 of 779 and reported a working
            // detector as one that had gone quiet -- a filter selecting the
            // sample, and the sample then supporting a confident conclusion
            // about the selection. LEARNINGS 7 and 10, a third time.
            if let Some(base) = universe.candidate(mint, None, Some(sol_price)) {
                let candidate = match coordination {
                    Some(verdict) => base.with_coordination(verdict),
                    None => base,
                };
                let decision = strategy.consider(&candidate);
                examined.push((
                    record_of(&candidate, &decision, strategy, None, watermark, prevalence),
                    candidate.mint,
                ));
            }
            continue;
        }

        let structure = rpc.mint_structure(mint).ok();
        // Discovered, not assumed. This used to quote a hardcoded 1_000_000_000
        // base units for every token — roughly 0.00005% of a pump.fun supply —
        // so the "capacity" it measured was worth a fraction of a cent and every
        // candidate was refused as CapacityBelowFloor. Zero proposals read as a
        // fact about the market and was a fact about the probe. LEARNINGS 10.
        let exit = radar_sim::discover_capacity(
            quoter,
            mint,
            structure,
            radar_sim::Search {
                max_impact_bps: strategy.thresholds.capacity_impact_bps,
                ..radar_sim::Search::DEFAULT
            },
        );
        let Some(candidate) = universe.candidate(mint, Some(exit), Some(sol_price)) else {
            continue;
        };
        let candidate = match coordination {
            Some(c) => candidate.with_coordination(c),
            // Carried as absent rather than as clean. The strategy will not
            // refuse on it, and it will not treat it as a pass either.
            None => candidate,
        };
        report_one(
            strategy,
            &candidate,
            &mut proposals,
            watermark,
            examined,
            prevalence,
        );
    }

    print!("{}", render_shapes(&shapes, look_failed));
    if refused_on_shape > 0 {
        println!(
            "{refused_on_shape} candidate(s) refused on shape before any exit probe was paid for."
        );
    }
    if look_failed > 0 {
        println!(
            "  A candidate whose launch block could not be read carries no verdict,
               and the strategy will not refuse on an absence — so those passed this
               gate without being examined rather than by being clean."
        );
    }
    proposals
}

/// The widest bar drawn, in characters.
const BAR_WIDTH: usize = 28;

/// Renders what the sampled launch blocks looked like.
///
/// **Every row of the band is printed, at zero if that is what was observed.**
/// Their absence is the thing worth noticing: [`radar_graph::BUNDLE_CENTRE`] is
/// a bundler tool's default setting, and when that default moves the detector
/// goes quiet without saying so. A histogram that omitted empty rows would
/// report a moved constant exactly the way it reports a clean population, which
/// is the failure LEARNINGS 5 names — a check that reports absence the same way
/// it reports success is not a check.
///
/// The two rates at the foot are the comparison that makes decay visible at all.
fn render_shapes(dist: &radar_graph::Distribution, unreadable: usize) -> String {
    use core::fmt::Write as _;

    let mut out = String::new();
    let _ = write!(
        out,
        "\nlaunch blocks read: {}, unreadable: {unreadable}\n",
        dist.total()
    );

    if dist.is_empty() {
        // Never "0 at the centre". Nothing was looked at, so nothing is known,
        // and saying otherwise is the exact confusion this function exists to
        // prevent.
        out.push_str(
            "  No launch block was read, so the coordination gate did not run.\n\
             \x20 That is not the same as finding nothing.\n",
        );
        return out;
    }

    // The band always appears; observed values are merged in.
    let mut rows: std::collections::BTreeMap<u64, usize> = radar_graph::BUNDLE_BAND
        .clone()
        .map(|r| (r, dist.count(r)))
        .collect();
    for (recipients, count) in dist.iter() {
        rows.insert(recipients, count);
    }

    let peak = rows.values().copied().max().unwrap_or(0).max(1);
    out.push_str("  recipients  observed\n");
    for (recipients, count) in rows {
        let filled = count * BAR_WIDTH / peak;
        let note = if recipients == radar_graph::BUNDLE_CENTRE {
            "  <- centre, refused"
        } else if radar_graph::BUNDLE_BAND.contains(&recipients) {
            "  <- band"
        } else {
            ""
        };
        let _ = writeln!(
            out,
            "  {recipients:>10}  {:<width$} {count:>4}{note}",
            "#".repeat(filled),
            width = BAR_WIDTH
        );
    }

    let v = dist.verdicts();
    let centre = dist.centre_rate_bps().unwrap_or(0);
    let band = dist.band_rate_bps().unwrap_or(0);
    let _ = write!(
        out,
        "
  at the centre: {} of {} ({centre} bps)
  in the band  : {} of {} ({band} bps)
",
        v.likely,
        dist.total(),
        v.likely + v.suspected,
        dist.total(),
    );
    // Stated as a different population rather than as a target, because it is
    // one. 0008 measured across *all* launches; everything here has already
    // survived the creator filters, so the shapes should differ and a reader
    // comparing the two numbers directly would be drawing a conclusion about
    // the selection -- the trap LEARNINGS 7, 10 and 11 each record. Only a
    // sustained collapse is evidence about the detector.
    let _ = write!(
        out,
        "  0008 measured {} bps at the centre and {} bps in the band, over all
           launches. These have already survived the creator filters, so a
           different shape is expected; a sustained zero is what would suggest
           the bundler default has moved off {}.
",
        radar_graph::MEASURED_CENTRE_RATE_BPS,
        radar_graph::MEASURED_BAND_RATE_BPS,
        radar_graph::BUNDLE_CENTRE,
    );

    out.push_str(&render_calibration(dist));
    out
}

/// What one base unit was worth when the decision was taken, scaled by
/// [`radar_store::PRICE_SCALE`].
///
/// Read off the **smallest rung** of the realised price ladder the exit probe
/// already built, so it is the price the sizing was derived from and cannot
/// disagree with it. The smallest rung because impact grows with size: the
/// largest rung is what a full exit would realise, and the smallest is the
/// closest thing the ladder holds to an untouched mid.
///
/// Scaled to match [`radar_store::Outcome`]'s price columns exactly, because
/// the only thing this number is for is being compared with them.
fn entry_price_of(exit: &radar_sim::ExitReport) -> Option<u64> {
    let rung = exit
        .curve
        .iter()
        .filter(|q| q.size_tokens > 0 && q.out_lamports > 0)
        .min_by_key(|q| q.size_tokens)?;
    // u128 throughout: lamports times a 1e18 scale leaves u64 immediately, and a
    // wrapped entry price would make every return computed from it nonsense in
    // a way that still looks like a number.
    let scaled = u128::from(rung.out_lamports)
        .checked_mul(radar_store::PRICE_SCALE)?
        .checked_div(u128::from(rung.size_tokens))?;
    u64::try_from(scaled).ok()
}

/// Turns one examined candidate into the row that outlives the run.
///
/// # What is recorded, and what deliberately is not
///
/// Only candidates that reached the **paid tier**. The line is not cost, it is
/// **reproducibility**: a free-tier refusal is a pure function of data already
/// in the store, so it can be re-derived at any time by replaying `disqualify`
/// and the creator record at the same watermark. Recording 41,721 rows an hour
/// to store an answer that is already implied would be storing a derivation.
///
/// A paid-tier decision cannot be re-derived. It rests on a live Jupiter price
/// ladder and a CryptoHouse launch block, neither of which is recorded anywhere
/// and neither of which answers the same way tomorrow. If it is not written
/// down as it happens it is gone — which is the same argument
/// [`radar_research`] makes for digesting inputs rather than copying them, run
/// the other way.
///
/// [`radar_research`]: https://github.com/hey-vera/radar
/// The pass's prevalence table, or `None` if it cannot be trusted.
///
/// One query for the whole pass. Asked per candidate this took 32 seconds
/// against the real endpoint ([research 0012](../../docs/research/0012-recipient-sets-cannot-recur-authorities-can.md)),
/// which at forty candidates an hour is twenty minutes of query time per hour on
/// an endpoint Radar is a guest on.
///
/// A truncated table is `None`, not a short one. Every authority the row cap cut
/// would otherwise read as `Ordinary` — the least alarming answer available —
/// and a decision would record that as though it had been measured. Rule 9.
fn prevalence_table_of<B>(blocks: &B) -> Option<radar_graph::prevalence::Table>
where
    B: LaunchBlockSource,
    B::Error: std::fmt::Display,
{
    match blocks.prevalence_table() {
        Ok(table) if table.is_complete() => {
            println!(
                "launch-block authorities at or above the repeat floor: {}",
                table.len()
            );
            Some(table)
        }
        Ok(_) => {
            eprintln!(
                "  prevalence table hit the row cap and cannot be trusted — recorded as absent"
            );
            None
        }
        Err(e) => {
            eprintln!("  prevalence table unreadable: {e}");
            None
        }
    }
}

fn record_of(
    candidate: &Candidate,
    decision: &Decision,
    strategy: &CreatorEdge,
    verdict: Option<&Verdict>,
    decided_at: radar_types::Slot,
    prevalence: Option<radar_graph::prevalence::Prevalence>,
) -> radar_store::Decision {
    let proposal = match decision {
        Decision::Propose(p) => Some(p),
        Decision::Pass(_) => None,
    };
    radar_store::Decision {
        mint: candidate.mint,
        creator: candidate.creator,
        decided_at,
        launch_slot: candidate.launch_slot,
        strategy: strategy.name().to_owned(),
        strategy_version: strategy.version().to_owned(),
        conclusion: if proposal.is_some() {
            radar_store::Conclusion::Proposed
        } else {
            radar_store::Conclusion::Passed
        },
        reasons: decision
            .reasons()
            .iter()
            .map(|r| format!("{r:?}"))
            .collect(),
        notional_micro_usd: proposal.map(|p| p.notional.get()),
        exit_capacity_micro_usd: proposal
            .and_then(|p| p.simulated_exit_capacity)
            .map(MicroUsd::get),
        assumed_round_trip_bps: strategy.thresholds.assumed_round_trip_bps,
        // Absent because the source could not answer, never because the launch
        // looked clean. Collapsing those would quietly clear a bundle.
        coordination: candidate.coordination.map(|c| format!("{c:?}")),
        // Recorded, never acted on. 0012 measured who recurs and not whether
        // recurrence predicts anything about money; this is what makes that
        // second question answerable later.
        authority_prevalence: prevalence.map(|p| p.label().to_owned()),
        entry_price: candidate.exit.as_ref().and_then(entry_price_of),
        kernel_outcome: verdict.map(|v| match v {
            Verdict::Authorised(_) => radar_store::KernelOutcome::Authorised,
            Verdict::Refused { .. } => radar_store::KernelOutcome::Refused,
        }),
        kernel_reasons: match verdict {
            Some(Verdict::Refused { reasons }) => {
                reasons.iter().map(|r| format!("{r:?}")).collect()
            }
            _ => Vec::new(),
        },
        // The same digest a replay compares on, so a recorded decision can be
        // checked against the store later without a separate recording file.
        inputs_digest: radar_research::Digest::of(candidate)
            .map_or_else(|_| String::new(), |d| d.0),
    }
}

/// The verdict on whether the detector is still calibrated.
///
/// Rendered separately from the histogram because a reader should not have to
/// derive it. The histogram says what was seen; this says whether what was seen
/// is consistent with the measurement the threshold rests on.
fn render_calibration(dist: &radar_graph::Distribution) -> String {
    use core::fmt::Write as _;
    let mut out = String::new();
    let _ = match radar_graph::calibration(dist) {
        radar_graph::Calibration::NotEnoughData { observed, needed } => write!(
            out,
            "
  CALIBRATION: {observed} block(s) is too few to say; {needed} more.
               Not a clean bill of health -- a detector nobody has sampled and one
               that works look the same from here.
"
        ),
        radar_graph::Calibration::Consistent { centre_rate_bps } => write!(
            out,
            "
  CALIBRATION: consistent — {centre_rate_bps} bps at the centre.
"
        ),
        radar_graph::Calibration::Silent {
            centre_rate_bps,
            expected_bps,
            observed,
        } => write!(
            out,
            "
  CALIBRATION: SILENT — {centre_rate_bps} bps at the centre over {observed}
               block(s), against {expected_bps} measured. The band has gone quiet, which
               is the direction that fails permissive: a moved bundler default makes
               `is_actionable` stop firing, and nothing raises an error.
"
        ),
        radar_graph::Calibration::Elevated {
            centre_rate_bps,
            expected_bps,
            observed,
        } => write!(
            out,
            "
  CALIBRATION: ELEVATED — {centre_rate_bps} bps at the centre over {observed}
               block(s), against {expected_bps} measured. Either the market moved or this
               sample is not what it is believed to be; both invalidate the threshold.
"
        ),
    };
    out
}

/// Prints what the strategy made of one paid-for candidate.
fn report_one(
    strategy: &CreatorEdge,
    candidate: &Candidate,
    proposals: &mut Vec<radar_risk::Proposal>,
    watermark: radar_types::Slot,
    examined: &mut Vec<(radar_store::Decision, Address)>,
    prevalence: Option<radar_graph::prevalence::Prevalence>,
) {
    let decision = strategy.consider(candidate);
    // Recorded before the kernel runs, and updated with its verdict afterwards.
    // A proposal the kernel never saw is a different state from one it refused.
    examined.push((
        record_of(candidate, &decision, strategy, None, watermark, prevalence),
        candidate.mint,
    ));
    match decision {
        Decision::Pass(ref reasons) => {
            println!("  {}  passed: {reasons:?}", candidate.mint);
        }
        Decision::Propose(ref proposal) => {
            println!(
                "  {}  PROPOSED ${:.2} (exit capacity ${:.2})",
                candidate.mint,
                price_dollars(proposal.notional),
                proposal.simulated_exit_capacity.map_or(0.0, price_dollars)
            );
            proposals.push((**proposal).clone());
        }
    }
}

/// Puts every proposal through the kernel under the shipped policy.
fn verdicts(
    proposals: &[radar_risk::Proposal],
    watermark: radar_types::Slot,
    reader: &Reader,
) -> BTreeMap<Address, Verdict> {
    let mut by_mint = BTreeMap::new();
    println!(
        "
{} proposal(s) raised.",
        proposals.len()
    );
    if proposals.is_empty() {
        return by_mint;
    }

    // The shipped policy. Building the trading lane deploys no capital; only
    // changing this does, and changing it is a decision with an owner.
    //
    // `Policy::SHIPPED` rather than `Policy::CLOSED`, and the difference is not
    // cosmetic: `radar-serve`'s funnel reports the same constant, so opening the
    // policy here cannot leave the interface telling a customer that nothing can
    // trade. It used to read `CLOSED` on its own.
    let policy = Policy::SHIPPED;
    // Rebuilt from what was recorded rather than assumed empty. It *is* empty
    // today, because nothing has ever traded -- but `flat()` would keep saying
    // so on the day something does, and a position limit measured against a
    // portfolio that is always empty is not a limit.
    //
    // `halted` and `consecutive_failures` are supplied rather than defaulted:
    // positions cannot know either, and the permissive answer arriving silently
    // from a component that does not know is the shape rule 9 warns about.
    // Zero failures is honest while nothing executes; there is no execution
    // record yet to read them from.
    let state = portfolio_state(reader, watermark);

    println!("\nrisk kernel, under the policy this instance actually holds:");
    for proposal in proposals {
        let verdict = evaluate(proposal, &state, &policy);
        match &verdict {
            Verdict::Authorised(auth) => {
                println!(
                    "  {}  AUTHORISED up to ${:.2}",
                    proposal.mint,
                    price_dollars(auth.max_notional)
                );
            }
            Verdict::Refused { reasons } => {
                // Split rather than dumped. Under the shipped policy all seven
                // reasons are artifacts of a policy of zeros, and printing them
                // as a flat list tells a reader there are seven problems when
                // there is one.
                let (policy_bound, about_this) = radar_risk::partition_refusals(reasons, &policy);
                if about_this.is_empty() {
                    println!(
                        "  {}  refused by policy ({} limit(s)); nothing about the token",
                        proposal.mint,
                        policy_bound.len()
                    );
                } else {
                    println!("  {}  refused: {about_this:?}", proposal.mint);
                }
            }
        }
        by_mint.insert(proposal.mint, verdict);
    }
    // Derived from the policy this run actually judged against. It printed
    // "Radar ships with Policy::CLOSED, which refuses everything" as a literal,
    // so it said so whatever the policy was -- the fourth place that could not
    // stop reassuring, and the one a `repo-conformance` check found rather than
    // a reader.
    if policy.is_closed() {
        println!(
            "
Radar ships with a policy that refuses everything. Nothing above was acted
         on, and nothing will be until that policy is changed deliberately."
        );
    } else {
        println!(
            "
CAPITAL IS ARMED: autonomy {:?}, max position {}. What is above was judged
         against a policy that can authorise. The signer holds its own policy
         (ADR 0008) and clamps against it unconditionally; it is not readable
         from here, so this is not a statement that anything was signed.",
            policy.autonomy, policy.max_position
        );
    }
    by_mint
}

/// Micro-USD as dollars, for display only.
#[expect(
    clippy::cast_precision_loss,
    reason = "display only; both halves are far inside f64's exact integer range,               and every calculation upstream of this is integer micro-USD"
)]
fn price_dollars(amount: MicroUsd) -> f64 {
    let whole = amount.get() / 1_000_000;
    let fraction = amount.get() % 1_000_000;
    whole as f64 + fraction as f64 / 1e6
}

/// The paid-tier cap, exposed for the CLI's flag handling.
#[must_use]
pub const fn default_cap() -> usize {
    PAID_TIER_CAP
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A launch-block source that answers however a test needs it to.
    struct StubBlocks(Result<radar_graph::prevalence::Table, String>);

    impl LaunchBlockSource for StubBlocks {
        type Error = String;

        fn shape_at(
            &self,
            _: &radar_types::Address,
            _: radar_types::Slot,
        ) -> Result<radar_graph::LaunchBlockShape, Self::Error> {
            Err("not used by these tests".to_owned())
        }

        fn authorities_at(
            &self,
            _: &radar_types::Address,
            _: radar_types::Slot,
        ) -> Result<Vec<String>, Self::Error> {
            Err("not used by these tests".to_owned())
        }

        fn prevalence_table(&self) -> Result<radar_graph::prevalence::Table, Self::Error> {
            self.0.clone()
        }
    }

    #[test]
    fn a_truncated_prevalence_table_is_refused_rather_than_used() {
        // The load-bearing guard. A table that hit the thousand-row cap is
        // missing the authorities the cut removed, and every one of them would
        // then read as `Ordinary` -- the least alarming answer available --
        // recorded on a decision as though it had been measured. Rule 9.
        let capped: Vec<(String, u64)> = (0..radar_graph::prevalence::ROW_CAP)
            .map(|i| (format!("authority-{i:04}"), 50))
            .collect();
        let truncated = radar_graph::prevalence::Table::new(capped);
        assert!(
            !truncated.is_complete(),
            "the fixture is actually truncated"
        );

        assert_eq!(
            prevalence_table_of(&StubBlocks(Ok(truncated))),
            None,
            "a truncated table must not be used"
        );
    }

    #[test]
    fn a_complete_prevalence_table_is_used() {
        // The other direction, and it is not decoration: a guard that refused
        // every table would disable the feature entirely while looking like a
        // safety measure, and nothing else in the pass would say so.
        let table = radar_graph::prevalence::Table::new([("factory".to_owned(), 8)]);
        assert!(table.is_complete());

        let kept = prevalence_table_of(&StubBlocks(Ok(table))).expect("a complete table is used");
        assert_eq!(
            kept.of("factory"),
            Some(radar_graph::prevalence::Prevalence::Repeat)
        );
        assert_eq!(
            kept.of("never-seen"),
            Some(radar_graph::prevalence::Prevalence::Ordinary),
            "below the floor, which is what the query means"
        );
    }

    #[test]
    fn an_unreadable_prevalence_table_records_absence_rather_than_failing_the_pass() {
        // A prevalence the pass could not fetch must not stop it deciding. The
        // decision is still worth recording; what it carries is an absent
        // prevalence, which is the honest value.
        assert_eq!(
            prevalence_table_of(&StubBlocks(Err("endpoint down".to_owned()))),
            None
        );
    }

    fn a_candidate() -> Candidate {
        Candidate {
            mint: radar_types::Address::new([7u8; 32]),
            creator: radar_types::Address::new([8u8; 32]),
            launch_slot: radar_types::Slot(1_000),
            as_of: radar_asof::AsOf::at(radar_types::Slot(10_000)),
            exit: None,
            creator_record: radar_strategy::CreatorRecord::default(),
            coordination: None,
            sol_price_micro_usd: None,
            token_observed_at: radar_types::Slot(9_900),
            creator_observed_at: radar_types::Slot(9_900),
        }
    }

    fn quote(size_tokens: u64, out_lamports: u64) -> radar_sim::exit::QuotePoint {
        radar_sim::exit::QuotePoint {
            size_tokens,
            out_lamports,
            impact_bps: 20,
        }
    }

    fn report(curve: Vec<radar_sim::exit::QuotePoint>) -> radar_sim::ExitReport {
        radar_sim::ExitReport {
            mint: radar_types::Address::new([7u8; 32]),
            structure: None,
            curve,
            no_route_at: Vec::new(),
            structural_threats: Vec::new(),
            can_be_stopped: false,
            confidence: radar_sim::exit::Confidence::Measured,
        }
    }

    #[test]
    fn the_entry_price_comes_from_the_smallest_rung() {
        // Impact grows with size, so the largest rung is what a full exit would
        // realise and the smallest is the closest the ladder holds to an
        // untouched price. Taking the wrong end would systematically understate
        // the entry and overstate every return measured from it.
        let exit = report(vec![
            quote(1_000_000_000_000, 20_000_000),
            quote(1_000_000_000, 30_000),
            quote(10_000_000_000_000, 150_000_000),
        ]);
        // 30_000 lamports for 1e9 base units = 3e-5 lamports each, times 1e18.
        assert_eq!(entry_price_of(&exit), Some(30_000_000_000_000));
    }

    #[test]
    fn a_rung_with_no_route_does_not_become_a_price_of_zero() {
        // A zero-output rung is "no route at this size", not "worthless". Taking
        // it would record an entry price of zero, which `return_bps` then
        // refuses -- so the decision would silently become unscoreable.
        let exit = report(vec![quote(1_000_000_000, 0), quote(2_000_000_000, 60_000)]);
        assert_eq!(entry_price_of(&exit), Some(30_000_000_000_000));
    }

    #[test]
    fn an_empty_curve_has_no_entry_price() {
        // Absent, not zero. A token with no route was never priced, and a
        // decision about it cannot be scored later.
        assert_eq!(entry_price_of(&report(Vec::new())), None);
        assert_eq!(entry_price_of(&report(vec![quote(0, 0)])), None);
    }

    #[test]
    fn the_entry_price_is_on_the_same_scale_as_a_recorded_outcome() {
        // The only purpose of this number is to be compared with the outcome
        // table's prices. A scale mismatch would make every return wrong by
        // eighteen orders of magnitude while still looking like a number --
        // which is the shape LEARNINGS 12 and 14 both record.
        //
        // One lamport per base unit must land exactly on PRICE_SCALE.
        let exit = report(vec![quote(1_000, 1_000)]);
        assert_eq!(
            u128::from(entry_price_of(&exit).expect("priced")),
            radar_store::PRICE_SCALE
        );
    }

    #[test]
    fn a_price_too_large_to_represent_is_refused_rather_than_wrapped() {
        // A wrapped entry price produces returns that are confidently wrong.
        let exit = report(vec![quote(1, u64::MAX)]);
        assert_eq!(entry_price_of(&exit), None);
    }

    #[test]
    fn a_refusal_records_the_reasons_the_kernel_gave() {
        // Deleting this arm leaves every refusal with an empty reason list,
        // which reads as "refused for no stated reason" -- and the reasons are
        // the entire point of recording a refusal. A mutant doing exactly that
        // survived the first version of these tests.
        let strategy = CreatorEdge::default();
        let candidate = a_candidate();
        let decision = strategy.consider(&candidate);
        let verdict = Verdict::Refused {
            reasons: vec![
                radar_risk::Refusal::NoAutonomy,
                radar_risk::Refusal::InputsTooStale,
            ],
        };

        let record = record_of(
            &candidate,
            &decision,
            &strategy,
            Some(&verdict),
            radar_types::Slot(10_000),
            None,
        );
        assert_eq!(
            record.kernel_reasons,
            vec!["NoAutonomy".to_owned(), "InputsTooStale".to_owned()]
        );
        assert_eq!(
            record.kernel_outcome,
            Some(radar_store::KernelOutcome::Refused)
        );
    }

    #[test]
    fn a_decision_the_kernel_never_saw_carries_no_verdict_and_no_reasons() {
        // Absent is not a refusal. A proposal that never reached the kernel is a
        // gap in the pipeline; recording it as refused would hide that.
        let strategy = CreatorEdge::default();
        let candidate = a_candidate();
        let decision = strategy.consider(&candidate);

        let record = record_of(
            &candidate,
            &decision,
            &strategy,
            None,
            radar_types::Slot(10_000),
            None,
        );
        assert_eq!(record.kernel_outcome, None);
        assert!(record.kernel_reasons.is_empty());
    }

    #[test]
    fn a_record_carries_the_watermark_and_the_cost_the_rule_assumed() {
        // Both are what makes a decision comparable later. The assumed cost
        // moved by a factor of four on 2026-08-25, and a decision either side of
        // that was judged against a different bar -- comparing them without
        // knowing which would be comparing two rules.
        let strategy = CreatorEdge::default();
        let candidate = a_candidate();
        let decision = strategy.consider(&candidate);
        let record = record_of(
            &candidate,
            &decision,
            &strategy,
            None,
            radar_types::Slot(441_734_987),
            None,
        );

        assert_eq!(record.decided_at, radar_types::Slot(441_734_987));
        assert_eq!(record.launch_slot, candidate.launch_slot);
        assert_eq!(
            record.assumed_round_trip_bps,
            strategy.thresholds.assumed_round_trip_bps
        );
        assert_eq!(record.strategy, "creator_edge");
        assert!(
            !record.inputs_digest.is_empty(),
            "the digest is what lets a recorded decision be checked against the store"
        );
    }

    #[test]
    fn an_unread_launch_block_records_as_absent_not_as_clean() {
        // The distinction the whole coordination gate rests on: a source that
        // could not answer must never look like a launch that looked fine.
        let strategy = CreatorEdge::default();
        let mut candidate = a_candidate();
        candidate.coordination = None;
        let unread = record_of(
            &candidate,
            &strategy.consider(&candidate),
            &strategy,
            None,
            radar_types::Slot(10_000),
            None,
        );
        assert_eq!(unread.coordination, None);

        let clean = candidate.with_coordination(radar_graph::Coordination::Unremarkable);
        let looked = record_of(
            &clean,
            &strategy.consider(&clean),
            &strategy,
            None,
            radar_types::Slot(10_000),
            None,
        );
        assert_eq!(looked.coordination, Some("Unremarkable".to_owned()));
        assert_ne!(unread.coordination, looked.coordination);
    }

    #[test]
    fn dollars_render_from_integers() {
        assert!((price_dollars(MicroUsd::from_dollars(12.34)) - 12.34).abs() < 1e-9);
        assert!((price_dollars(MicroUsd::ZERO) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn a_huge_amount_still_renders_rather_than_saturating_into_nonsense() {
        // Micro-USD exceeds f64's exact integer range, which is why the split
        // exists. The whole-dollar half stays exact well past any real balance.
        let large = MicroUsd(u64::MAX);
        assert!(price_dollars(large) > 1.0e12);
    }

    fn dist(recipients: &[u64]) -> radar_graph::Distribution {
        let mut d = radar_graph::Distribution::new();
        for r in recipients {
            d.observe(radar_graph::LaunchBlockShape {
                recipients: *r,
                transactions: 4,
            });
        }
        d
    }

    #[test]
    fn nothing_read_does_not_render_as_nothing_found() {
        // The whole reason this function exists. Before it, `read: 25,
        // unreadable: 0` was printed for both a healthy gate on a clean sample
        // and a gate whose constant had moved, and the two were byte-identical.
        let unread = render_shapes(&dist(&[]), 0);
        let clean = render_shapes(&dist(&[1, 2, 2, 3, 3, 3]), 0);

        assert_ne!(unread, clean);
        assert!(
            unread.contains("did not run"),
            "an unread sample must say so: {unread}"
        );
        assert!(
            !unread.contains("at the centre"),
            "no sample means no rate to report: {unread}"
        );
        assert!(
            clean.contains("at the centre: 0 of 6"),
            "a clean sample has a rate and it is zero: {clean}"
        );
    }

    #[test]
    fn the_band_is_printed_even_when_empty() {
        // A moved bundler default shows up as holes where the band used to be.
        // Omitting zero rows would hide exactly that.
        let out = render_shapes(&dist(&[1, 1, 2, 2, 3]), 0);
        for recipients in radar_graph::BUNDLE_BAND {
            assert!(
                out.lines()
                    .any(|l| l.trim_start().starts_with(&format!("{recipients} "))),
                "band row {recipients} missing from:\n{out}"
            );
        }
        assert!(out.contains("<- centre, refused"), "{out}");
        assert!(out.contains("<- band"), "{out}");
    }

    #[test]
    fn the_measured_baseline_is_shown_beside_the_observed_rate() {
        // The comparison is the decay check. A rate with nothing to compare it
        // against is a number nobody can act on.
        let out = render_shapes(&dist(&[6, 1, 1, 1, 1, 1, 1, 1, 1, 1]), 0);
        assert!(out.contains("1000 bps"), "observed rate missing: {out}");
        assert!(
            out.contains(&format!(
                "{} bps at the centre",
                radar_graph::MEASURED_CENTRE_RATE_BPS
            )),
            "measured baseline missing: {out}"
        );
        assert!(
            out.contains("survived the creator filters"),
            "the baseline is a different population and the output must say so,              or a reader compares a selected sample against an unselected one: {out}"
        );
    }

    #[test]
    fn an_unreadable_block_is_reported_separately_from_a_read_one() {
        // A source that failed leaves the verdict absent and the gate silently
        // off. It must never be folded into the read count.
        let out = render_shapes(&dist(&[1, 2, 3]), 7);
        assert!(out.contains("read: 3, unreadable: 7"), "{out}");
    }

    #[test]
    fn the_centre_marker_sits_on_the_centre_row_and_nowhere_else() {
        // Asserting only that both labels appear somewhere is not evidence: with
        // the comparison inverted, every ordinary row gets "centre" and the
        // centre row gets "band", and both strings are still present. A mutant
        // doing exactly that survived the first version of this test.
        let out = render_shapes(&dist(&[1, 5, 6, 7, 40]), 0);
        let mut rows_checked = 0;
        // Only the histogram block. Scanning the whole output for "a line
        // starting with a number" caught the footer, because `"0008".parse()`
        // is `Ok(8)` -- a reminder that a loose heuristic in a test is a way to
        // assert something other than what was meant.
        for line in out
            .lines()
            .skip_while(|l| !l.contains("recipients  observed"))
            .skip(1)
            .take_while(|l| !l.trim().is_empty())
        {
            let Some(first) = line.split_whitespace().next() else {
                continue;
            };
            let recipients: u64 = first
                .parse()
                .unwrap_or_else(|_| panic!("histogram row is not numeric: {line}"));
            rows_checked += 1;
            assert_eq!(
                line.contains("<- centre"),
                recipients == radar_graph::BUNDLE_CENTRE,
                "row {recipients}: {line}"
            );
            assert_eq!(
                line.contains("<- band"),
                radar_graph::BUNDLE_BAND.contains(&recipients)
                    && recipients != radar_graph::BUNDLE_CENTRE,
                "row {recipients}: {line}"
            );
        }
        assert_eq!(rows_checked, 5, "expected one row per distinct count");
    }

    #[test]
    fn the_bar_is_proportional_to_the_count() {
        // Not merely "within the width". A bar that is always empty, or that
        // shrinks as the count grows, also fits inside the width and tells the
        // reader nothing -- two arithmetic mutants survived a width-only
        // assertion.
        let out = render_shapes(&dist(&[1, 1, 1, 1, 2, 2, 3]), 0);
        let bar_of = |r: u64| -> usize {
            let want = r.to_string();
            out.lines()
                .find(|l| l.split_whitespace().next() == Some(want.as_str()))
                .map_or_else(
                    || panic!("no row for {r} in\n{out}"),
                    |l| l.matches('#').count(),
                )
        };

        assert_eq!(
            bar_of(1),
            BAR_WIDTH,
            "the most frequent count fills the width"
        );
        assert!(
            bar_of(1) > bar_of(2),
            "four observations must draw wider than two"
        );
        assert!(bar_of(2) > bar_of(3), "two must draw wider than one");
        assert_eq!(bar_of(5), 0, "an unobserved band row draws nothing");

        // And still never overflows, including where the peak is one.
        for sample in [vec![1], vec![1; 500], vec![1, 6], (0..40).collect()] {
            for line in render_shapes(&dist(&sample), 0).lines() {
                assert!(
                    line.matches('#').count() <= BAR_WIDTH,
                    "bar overflowed on {sample:?}: {line}"
                );
            }
        }
    }

    #[test]
    fn the_paid_tier_is_capped() {
        // A first run against a large store must not make tens of thousands of
        // requests to somebody's free endpoint.
        assert!(default_cap() > 0 && default_cap() <= 100);
    }
}
