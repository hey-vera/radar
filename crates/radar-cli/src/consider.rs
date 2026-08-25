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
pub fn run(reader: &Reader, window: u64, cap: usize) -> Result<(), String> {
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

    let proposals = paid_tier(
        &universe,
        &strategy,
        worth_paying_for.iter().take(budget),
        &quoter,
        sol_price,
    );

    verdicts(&proposals, watermark);
    Ok(())
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
) -> Vec<Proposal> {
    let rpc = RpcClient::default();
    let blocks = CryptoHouseBlocks::default();
    let mut proposals = Vec::new();
    let mut refused_on_shape = 0usize;
    // Counted apart from "looked and found clean". A fetch that fails leaves the
    // verdict absent, the strategy correctly declines to refuse on an absence,
    // and the gate is then silently off. Without this line a broken source and a
    // clean population produce identical output.
    let (mut looked, mut look_failed) = (0usize, 0usize);

    for mint in mints {
        // The launch-block look runs first because it is the cheaper of the two
        // paid calls and it can end the question. Probing the exit of a token
        // whose curve was already bought out by whoever arranged it is money
        // spent to be told something the block said for less.
        let coordination = match universe.launches.get(mint) {
            Some(facts) => match blocks.shape_at(mint, facts.slot) {
                Ok(shape) => {
                    looked += 1;
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

        if coordination.is_some_and(radar_graph::Coordination::is_actionable) {
            refused_on_shape += 1;
            println!("  {mint}  launch block looks arranged — not probing the exit");
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
        report_one(strategy, &candidate, &mut proposals);
    }

    println!(
        "
launch blocks read: {looked}, unreadable: {look_failed}"
    );
    if refused_on_shape > 0 {
        println!(
            "{refused_on_shape} candidate(s) refused on shape before any exit probe was paid for."
        );
    }
    if look_failed > 0 {
        println!(
            "A candidate whose launch block could not be read carries no verdict, and the
             strategy will not refuse on an absence — so those passed this gate without
             being examined rather than by being clean."
        );
    }
    proposals
}

/// Prints what the strategy made of one paid-for candidate.
fn report_one(
    strategy: &CreatorEdge,
    candidate: &Candidate,
    proposals: &mut Vec<radar_risk::Proposal>,
) {
    match strategy.consider(candidate) {
        Decision::Pass(reasons) => {
            println!("  {}  passed: {reasons:?}", candidate.mint);
        }
        Decision::Propose(proposal) => {
            println!(
                "  {}  PROPOSED ${:.2} (exit capacity ${:.2})",
                candidate.mint,
                price_dollars(proposal.notional),
                proposal.simulated_exit_capacity.map_or(0.0, price_dollars)
            );
            proposals.push(*proposal);
        }
    }
}

/// Puts every proposal through the kernel under the shipped policy.
fn verdicts(proposals: &[radar_risk::Proposal], watermark: radar_types::Slot) {
    println!("\n{} proposal(s) raised.", proposals.len());
    if proposals.is_empty() {
        return;
    }

    // The shipped policy. Building the trading lane deploys no capital; only
    // changing this does, and changing it is a decision with an owner.
    let policy = Policy::CLOSED;
    let state = PortfolioState::flat(watermark);

    println!("\nrisk kernel, under the policy this instance actually holds:");
    for proposal in proposals {
        match evaluate(proposal, &state, &policy) {
            Verdict::Authorised(auth) => {
                println!(
                    "  {}  AUTHORISED up to ${:.2}",
                    proposal.mint,
                    price_dollars(auth.max_notional)
                );
            }
            Verdict::Refused { reasons } => {
                println!("  {}  refused: {reasons:?}", proposal.mint);
            }
        }
    }
    println!(
        "\nRadar ships with Policy::CLOSED, which refuses everything. Nothing above\n\
         was acted on, and nothing will be until that policy is changed deliberately."
    );
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

    #[test]
    fn the_paid_tier_is_capped() {
        // A first run against a large store must not make tens of thousands of
        // requests to somebody's free endpoint.
        assert!(default_cap() > 0 && default_cap() <= 100);
    }
}
