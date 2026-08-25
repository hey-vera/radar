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
    // and the gate is then silently off. Without this a broken source and a
    // clean population produce identical output.
    //
    // The shapes are kept rather than counted, because the count alone had the
    // same defect one level up: `read: 25, unreadable: 0` is what a detector
    // whose constant has moved prints too. See [`render_shapes`].
    let mut shapes = radar_graph::Distribution::new();
    let mut look_failed = 0usize;

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
    out
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
