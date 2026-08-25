// SPDX-License-Identifier: Apache-2.0
//! `radar study` — does the creator signal predict anything?
//!
//! Prints the event study. The output is deliberately shaped so the honest
//! answer and the flattering one look different: every group rate sits beside
//! the population rate it has to beat, and a group too small to speak for prints
//! its counts with no rate at all.

use radar_asof::AsOf;
use radar_research::study::{self, Group, MIN_GROUP, MIN_PRIOR_LAUNCHES};
use radar_store::Reader;
use radar_types::Slot;

/// Where the pivot goes when the caller does not choose one.
///
/// Halfway between the **first outcome measurement** and the head — not halfway
/// through the recorded slot range, which is what this did first and which was
/// wrong in a way that produced a confident empty answer.
///
/// The store holds launches from well before its first measurement, so the
/// midpoint of the slot range landed before anything had been measured. Every
/// creator then had an empty prior, and the table read as "creator history
/// predicts nothing" when it actually said "nothing had been measured yet".
/// A pivot is only useful where knowledge existed.
fn default_pivot(first_measurement: Slot, head: Slot) -> Slot {
    Slot(first_measurement.get() + (head.get().saturating_sub(first_measurement.get())) / 2)
}

/// Runs the study and prints it.
///
/// # Errors
///
/// Returns a message if the store cannot be read or holds nothing.
pub fn run(reader: &Reader, pivot: Option<u64>) -> Result<(), String> {
    let head = reader
        .watermark()
        .map_err(|e| format!("cannot read the store: {e}"))?
        .ok_or("the store has recorded nothing yet")?;
    let earliest = reader
        .earliest()
        .map_err(|e| format!("cannot read the store: {e}"))?
        .unwrap_or(head);
    let first_measurement = reader
        .read_outcomes(AsOf::at(head))
        .map_err(|e| format!("cannot read outcomes: {e}"))?
        .iter()
        .map(|o| o.measured_at)
        .min();

    let pivot = match (pivot, first_measurement) {
        (Some(p), _) => Slot(p),
        (None, Some(first)) => default_pivot(first, head),
        (None, None) => {
            return Err("the store holds no outcome measurements, so there is nothing a                         prior could be built from"
                .to_owned());
        }
    };
    let study = study::run(reader, pivot, head).map_err(|e| format!("cannot study: {e}"))?;

    println!("store spans   : slot {earliest} .. {head}");
    println!("pivot         : slot {pivot}");
    println!(
        "creators      : {} with >= {MIN_PRIOR_LAUNCHES} launches before the pivot and at \
         least one after",
        study.creators
    );
    println!(
        "prior coverage: {} of {} pre-pivot launches had been measured by then",
        study.prior_measured, study.prior_launches
    );
    println!(
        "later launches: {} of which {} graduated organically",
        study.later_launches, study.later_organic
    );

    // Refused before the table is printed, not after. A table showing every
    // creator in "no organic graduation known" reads as a finding about
    // creators, and it is not one when nothing had been measured.
    if !study.prior_is_informative() {
        println!(
            "
Nothing had been measured about these creators by slot {pivot}, so every
             prior is empty and the grouping below would be an artefact. Choose a pivot
             inside the measured window with --pivot, or wait for the outcome pass to
             cover more of the record.

             This is the difference between 'creator history predicts nothing' and 'we
             had not looked yet', and they are not distinguishable from the table alone."
        );
        return Ok(());
    }

    let Some(base) = study.base_rate_bps() else {
        println!(
            "\nNo later launches from any creator with a prior record. The study has\n\
             nothing to compare, which is a fact about how much chain has been\n\
             recorded rather than about the signal."
        );
        return Ok(());
    };
    println!("base rate     : {}", bps(base));

    println!("\nWhat creators did next, by what was known about them at the pivot:\n");
    println!(
        "  {:<30}  {:>8}  {:>8}  {:>8}  RATE",
        "KNOWN AT PIVOT", "CREATORS", "LAUNCHES", "ORGANIC"
    );
    for group in &study.groups {
        println!(
            "  {:<30}  {:>8}  {:>8}  {:>8}  {}",
            group.label,
            group.creators,
            group.later_launches,
            group.later_organic,
            match (group.later_organic_bps(), group.later_organic_ci_bps()) {
                (Some(r), Some((lo, hi))) =>
                    format!("{} [{} – {}]{}", bps(r), bps(lo), bps(hi), lift(r, base)),
                _ => format!("(under {MIN_GROUP} creators; no rate stated)"),
            }
        );
    }

    verdict(&study.groups);
    frequency_control(&study.strata);
    curve(&study.frequency_curve);
    thresholds(&study.cuts, study.pivot, earliest);
    Ok(())
}

/// Tests every candidate threshold and says which ones the data supports.
///
/// A rule needs a number. Choosing it by looking at a curve picks whichever
/// boundary flatters this sample; this asks the same question of every candidate
/// and reports the answer, so the choice can be re-run when there is more data.
fn thresholds(cuts: &[study::Cut], pivot: Slot, earliest: Slot) {
    // The count is only meaningful against the window it was counted over, so
    // the per-day equivalent travels with it. A threshold of twenty launches
    // means something different over two days than over two months, and a rule
    // that stores the bare count silently changes meaning as the store grows.
    let span_slots = pivot.get().saturating_sub(earliest.get());
    #[expect(clippy::cast_precision_loss, reason = "display of a slot count")]
    let days = (span_slots as f64 / SLOTS_PER_DAY).max(0.01);

    println!(
        "

Candidate thresholds on prior launch count
"
    );
    println!(
        "  measured over {days:.2} days of chain before the pivot
"
    );
    println!(
        "  {:<8}  {:>7}  {:>24}  {:>24}",
        "CUT", "PER DAY", "BELOW (quieter)", "AT OR ABOVE (busier)"
    );
    for c in cuts {
        #[expect(clippy::cast_precision_loss, reason = "display of a small integer")]
        let per_day = c.at as f64 / days;
        println!(
            "  {:<8}  {:>7.1}  {:>24}  {:>24}{}",
            c.at,
            per_day,
            cell_rate(&c.below),
            cell_rate(&c.above),
            if c.separates() { "  separates" } else { "" }
        );
    }

    let supported: Vec<&study::Cut> = cuts.iter().filter(|c| c.separates()).collect();
    println!();
    if supported.is_empty() {
        println!("No candidate threshold separates. Launch count leans the right way but");
        println!("this sample cannot support a refusal rule built on it, and shipping one");
        println!("anyway would be a preference wearing a measurement's clothes.");
        return;
    }
    println!(
        "{} of {} candidate thresholds separate at 95%. The lowest that does is {}",
        supported.len(),
        cuts.len(),
        supported[0].at
    );
    println!("launches, which is the least aggressive rule the data actually supports.");
}

/// Slots in a day, at the nominal 2.5 a second. The measured rate varies by 14%
/// across days, so this is a scale for reading, never for arithmetic that matters.
const SLOTS_PER_DAY: f64 = 216_000.0;

/// Prints later organic rate against prior launch count, over every creator.
///
/// The shape a threshold has to be read off. Split cells are too small to see
/// where a gradient turns; these are not split, so they are.
fn curve(bands: &[study::Group]) {
    println!(
        "

Later organic rate by prior launch count, all creators
"
    );
    println!(
        "  {:<10}  {:>9}  {:>9}  {:>8}  RATE",
        "LAUNCHES", "CREATORS", "LAUNCHES", "ORGANIC"
    );
    for b in bands {
        println!(
            "  {:<10}  {:>9}  {:>9}  {:>8}  {}",
            b.label,
            b.creators,
            b.later_launches,
            b.later_organic,
            cell_rate(b)
        );
    }
}

/// A rate with its interval, or why there is none.
fn cell_rate(g: &study::Group) -> String {
    match (g.later_organic_bps(), g.later_organic_ci_bps()) {
        (Some(r), Some((lo, hi))) => format!("{} [{} - {}]", bps(r), bps(lo), bps(hi)),
        _ => "(too few creators)".to_owned(),
    }
}

/// Prints the same comparison with launch frequency held roughly fixed.
///
/// The headline table cannot distinguish "this creator is good" from "this
/// creator launches constantly and has more chances". This can, to the extent
/// the sample allows — and where it does not, it says so rather than reporting
/// agreement it has not earned.
fn frequency_control(strata: &[study::Stratum]) {
    println!(
        "

Controlling for launch frequency
"
    );
    println!(
        "  {:<16}  {:>26}  {:>26}",
        "PRIOR LAUNCHES", "WITHOUT PRIOR GRADUATION", "WITH PRIOR GRADUATION"
    );

    for s in strata {
        println!(
            "  {:<16}  {:>26}  {:>26}{}",
            s.label,
            cell(&s.without_prior),
            cell(&s.with_prior),
            if s.separates() { "  separated" } else { "" }
        );
    }

    let comparable: Vec<&study::Stratum> = strata.iter().filter(|s| s.can_compare()).collect();
    let separated = comparable.iter().filter(|s| s.separates()).count();

    println!();
    if comparable.is_empty() {
        println!("No band has enough creators on both sides to compare, so frequency has");
        println!("not been controlled for. The headline result stands unqualified and");
        println!("therefore unconfirmed: it may be entirely explained by prolific creators");
        println!("having more chances to graduate something.");
        return;
    }

    let leading = comparable.iter().filter(|s| s.direction_holds()).count();
    println!(
        "{} of {} band(s) could be compared; the direction holds in {leading}, and {separated}",
        comparable.len(),
        strata.len()
    );
    println!("separate at 95%.");

    frequency_alone(&comparable);

    if leading == comparable.len() {
        println!();
        println!("The direction holds at every frequency it could be tested at, and the gap");
        println!("is not an artefact of prolific creators having more chances. Consistency");
        println!("across independent bands is evidence in its own right: each band is a");
        println!("separate comparison, and all of them landing the same way is unlikely if");
        println!("there is nothing there.");
        if separated < comparable.len() {
            println!();
            println!("Not every band separates at 95%, which is a sample-size statement");
            println!("rather than a contrary result -- the bands that do not separate still");
            println!("lean the same way.");
        }
    } else if separated > 0 || leading > 0 {
        println!();
        println!("The effect holds in some bands and reverses in others. That is what a real");
        println!("but weak signal looks like on a small sample, and also what a confound");
        println!("looks like partway through being uncovered. It does not settle anything.");
    } else {
        println!();
        println!("The effect does not survive in any band where it could be tested. The");
        println!("headline separation is then most likely launch frequency wearing a");
        println!("creator-quality costume, and `creator_edge` would be selecting for");
        println!("creators who launch constantly.");
    }
}

/// What launch frequency predicts on its own.
///
/// Read down the without-prior column: these are creators about whom nothing
/// good was known, separated only by how much they launch. Any gradient here is
/// a signal in itself, and it also says which way the confound runs — if
/// prolific creators do *worse*, then their extra chances were suppressing the
/// headline result rather than manufacturing it.
fn frequency_alone(comparable: &[&study::Stratum]) {
    let rates: Vec<(String, u64)> = comparable
        .iter()
        .filter_map(|s| {
            s.without_prior
                .later_organic_bps()
                .map(|r| (s.label.clone(), r))
        })
        .collect();
    if rates.len() < 2 {
        return;
    }

    println!();
    println!("Launch frequency on its own, among creators with nothing good known:");
    for (label, rate) in &rates {
        println!("  {:<16}  {}", label, bps(*rate));
    }

    let (first, last) = (rates[0].1, rates[rates.len() - 1].1);
    if last < first {
        println!();
        println!("Creators who launch more graduate less, per launch. So the confound runs");
        println!("against the headline result rather than for it: prolific creators have");
        println!("more chances to have graduated something, and are worse per attempt, which");
        println!("means controlling for frequency strengthens the finding instead of");
        println!("dissolving it.");
    } else if last > first {
        println!();
        println!("Creators who launch more graduate more, per launch -- so frequency is a");
        println!("live confound and the headline result is partly it. The banded comparison");
        println!("above is the one to trust, not the headline.");
    }
}

/// One cell of the control table.
fn cell(g: &study::Group) -> String {
    match (g.later_organic_bps(), g.later_organic_ci_bps()) {
        (Some(r), Some((lo, hi))) => {
            format!("{} [{}-{}] n={}", bps(r), bps(lo), bps(hi), g.creators)
        }
        _ => format!("(n={}, too few)", g.creators),
    }
}

/// Says what the table supports, and refuses to say more.
fn verdict(groups: &[Group]) {
    let Some(without) = groups
        .iter()
        .find(|g| g.label == "no organic graduation known")
    else {
        return;
    };
    let best = groups
        .iter()
        .filter(|g| g.label != "no organic graduation known")
        .filter(|g| g.later_organic_bps().is_some())
        .max_by_key(|g| g.later_organic_bps().unwrap_or(0));

    let Some(best) = best else {
        println!(
            "
No group of creators with a prior organic graduation is large enough to"
        );
        println!("state a rate for, so there is nothing to compare. Not evidence either way.");
        return;
    };

    if best.clearly_above(without) {
        println!(
            "
Creators with a prior organic graduation went on to graduate more often,"
        );
        println!("and the two intervals do not overlap — so this is a separation rather than a");
        println!("gap between two noisy midpoints. It is the direction `creator_edge` assumes.");
        println!();
        println!("What it still is not: this is one window of a few days, the better group is");
        println!(
            "{} creators, and nothing here controls for how often a creator launches — a",
            best.creators
        );
        println!("creator with four hundred launches has four hundred chances. Treat it as the");
        println!("first evidence the rule is not arbitrary, not as its validation.");
    } else {
        println!(
            "
The intervals overlap, so the difference between these groups has not been"
        );
        println!("shown. Their midpoints may differ and that is not the same thing. On this");
        println!("much data that is the expected answer, and it is not evidence against the");
        println!("rule either.");
    }
}

/// A rate in basis points, rendered as a percentage a person can read.
fn bps(v: u64) -> String {
    #[expect(clippy::cast_precision_loss, reason = "display of a small integer")]
    let pct = v as f64 / 100.0;
    format!("{pct:.2}%")
}

/// How a group compares with the base rate, when the comparison is meaningful.
fn lift(rate: u64, base: u64) -> String {
    if base == 0 {
        return String::new();
    }
    #[expect(clippy::cast_precision_loss, reason = "display of a small integer")]
    let ratio = rate as f64 / base as f64;
    format!("  ({ratio:.2}x)")
}
