// SPDX-License-Identifier: Apache-2.0
//! `radar graduations` — the rarest event Radar records, and who caused it.
//!
//! Graduation is the only unambiguously good outcome in the dataset: the token
//! reached an AMM with liquidity that someone else put there. Everything else
//! Radar measures — transfers, survival, stillbirth — is a proxy for it.
//!
//! So this is the numerator of the question the creator signal is gated on:
//! *does a creator having graduated a token before predict them doing it again?*
//! The plan says to build that measurement before trusting the signal, and this
//! is the smallest honest version of it.
//!
//! It also reports the population rate, because a signal's usefulness is
//! relative to the base rate and the base rate here turned out to be far lower
//! than the plan assumed.

use std::collections::BTreeMap;

use radar_asof::AsOf;
use radar_store::{Event, GraduationMode, INSTANT_WITHIN_SLOTS, Reader, Table};
use radar_types::{Address, Slot};

/// Slots per hour, at roughly 2.5 a second.
///
/// Nominal, and it stays nominal on purpose: the measured rate is not a
/// constant. Two whole days off `solana.blocks` gave **2.4059** slots a second
/// on 2026-08-20 and **2.7349** on 2026-08-23, a 14% spread. So any slot-to-hour
/// figure approximates a moving quantity, and is printed with a `~` rather than
/// given more digits than it has.
///
/// Durations that need to be exact are reported in slots, which is what the
/// store actually holds.
const SLOTS_PER_HOUR: u64 = 9_000;

/// What is known about one graduation.
#[derive(Clone)]
struct Graduated {
    mint: Address,
    creator: Option<Address>,
    launch_slot: Option<Slot>,
    graduation_slot: Slot,
}

impl Graduated {
    /// How long the token took to reach an AMM, in slots.
    ///
    /// `None` when the launch was not recorded — which happens for a token that
    /// launched before this store started. Reporting zero there would put a
    /// fabricated "graduated instantly" into the fastest bucket.
    fn slots_to_graduate(&self) -> Option<u64> {
        self.launch_slot
            .map(|launch| self.graduation_slot.get().saturating_sub(launch.get()))
    }

    /// Whether the curve filled over time or was bought out in a block.
    ///
    /// `None` when the duration is unknown, never a guess. A graduation whose
    /// launch this store never saw could be either.
    fn mode(&self) -> Option<GraduationMode> {
        self.slots_to_graduate().map(|slots| {
            if slots <= INSTANT_WITHIN_SLOTS {
                GraduationMode::Instant
            } else {
                GraduationMode::Organic
            }
        })
    }
}

/// Prints every graduation the store holds, and the population rate.
///
/// # Errors
///
/// Returns a message if the store cannot be read.
pub fn run(reader: &Reader, limit: usize) -> Result<(), String> {
    let watermark = reader
        .watermark()
        .map_err(|e| format!("cannot read the store: {e}"))?
        .ok_or("the store has recorded nothing yet")?;
    let as_of = AsOf::at(watermark);

    let mut launches: BTreeMap<Address, (Address, Slot)> = BTreeMap::new();
    let mut per_creator: BTreeMap<Address, u64> = BTreeMap::new();
    for event in reader
        .read(Table::Launches, as_of)
        .map_err(|e| format!("cannot read launches: {e}"))?
    {
        if let Event::Launch(l) = event {
            launches.insert(l.mint, (l.creator, l.envelope.slot));
            *per_creator.entry(l.creator).or_default() += 1;
        }
    }

    let events = reader
        .read(Table::Graduations, as_of)
        .map_err(|e| format!("cannot read graduations: {e}"))?;
    let graduations = distinct_graduations(&events, &launches);

    report(&graduations, &launches, &per_creator, limit);
    Ok(())
}

/// The tokens that graduated, one entry each, earliest event winning.
///
/// Two corrections live here, and both were found by reading rows rather than by
/// expecting them:
///
/// **A failed `migrate` moved nothing**, so it is not a graduation. About a third
/// of migration attempts in a sampled hour failed — 35 of 97.
///
/// **A token can carry more than one successful migration instruction.** The same
/// hour held 62 successful rows across only 50 distinct mints, several as a
/// `migrate` followed a slot later by a `migrate_v2`. The table is right to keep
/// every event; a token graduates once, so the count and the population rate are
/// over mints. Counting rows instead overstated the rate by 24%.
fn distinct_graduations(
    events: &[Event],
    launches: &BTreeMap<Address, (Address, Slot)>,
) -> Vec<Graduated> {
    let mut by_mint: BTreeMap<Address, Graduated> = BTreeMap::new();
    for event in events.iter().filter(|e| e.envelope().succeeded) {
        let mint = event.mint();
        let known = launches.get(&mint);
        let this = Graduated {
            mint,
            creator: known.map(|(c, _)| *c),
            launch_slot: known.map(|(_, s)| *s),
            graduation_slot: event.envelope().slot,
        };
        by_mint
            .entry(mint)
            .and_modify(|held| {
                if this.graduation_slot < held.graduation_slot {
                    *held = this.clone();
                }
            })
            .or_insert(this);
    }
    by_mint.into_values().collect()
}

/// Prints the findings.
fn report(
    graduations: &[Graduated],
    launches: &BTreeMap<Address, (Address, Slot)>,
    per_creator: &BTreeMap<Address, u64>,
    limit: usize,
) {
    println!("launches recorded : {}", launches.len());
    println!("graduations       : {}", graduations.len());
    println!(
        "population rate   : {}",
        rate(graduations.len(), launches.len())
    );

    if graduations.is_empty() {
        println!(
            "\nNo graduations recorded yet. Until there are, `creator_edge` cannot\n\
             propose anything — a creator who has never graduated a token is refused,\n\
             and with no graduations that is every creator."
        );
        return;
    }

    split(graduations);

    let (mint, took) = ("MINT", "TOOK");
    println!("\n{mint:<46}  {took:>16}  MODE     CREATOR (their launches)");
    for g in graduations.iter().take(limit) {
        // Slots as well as hours. Every graduation measured so far completed in
        // under an hour, and an hours column alone renders all of them as "0.0"
        // — hiding the one number that separates "fast" from "same block".
        let hours = g.slots_to_graduate().map_or_else(
            || "  launch unknown".to_owned(),
            |slots| format!("{slots:>7} sl {:>5.1}h", as_hours(slots)),
        );
        let creator = g.creator.map_or_else(
            // Not "unknown creator" — a creator we did not record the launch
            // for. The distinction matters when reading how complete the
            // dataset is.
            || "launch not in this store".to_owned(),
            |c| format!("{c} ({})", per_creator.get(&c).copied().unwrap_or(0)),
        );
        let mode = match g.mode() {
            Some(GraduationMode::Instant) => "instant",
            Some(GraduationMode::Organic) => "organic",
            None => "unknown",
        };
        println!("{:<46}  {hours}  {mode:<7}  {creator}", g.mint.to_string());
    }

    repeats(graduations, per_creator);
}

/// Reports how the graduations divide between the two modes.
///
/// The split is the point of the command. A graduation that completed inside its
/// launch block is a bonding curve bought out by capital committed before the
/// token existed — a bundle, not demand — and counting it beside a curve that
/// filled over hours produces a "this creator has graduated a token" signal that
/// selects for bundlers. Measured over 44 graduations with a recoverable
/// subject: 27% completed in zero slots, and the median took 828.
fn split(graduations: &[Graduated]) {
    let count = |want: GraduationMode| {
        graduations
            .iter()
            .filter(|g| g.mode() == Some(want))
            .count()
    };
    let (instant, organic) = (
        count(GraduationMode::Instant),
        count(GraduationMode::Organic),
    );
    // Counted apart rather than folded into either bucket. A graduation whose
    // launch predates this store has no measurable duration, and giving it a
    // mode would invent the number the split exists to report.
    let unknown = graduations.len() - instant - organic;

    println!("  of which instant  : {instant}  (<= {INSTANT_WITHIN_SLOTS} slots from launch)");
    println!("  of which organic  : {organic}");
    if unknown > 0 {
        println!("  duration unknown  : {unknown}  (launched before this store started)");
    }
    if organic == 0 && instant > 0 {
        println!(
            "\nEvery graduation recorded was instant. `creator_edge` gates on the\n\
             organic rate, so no creator can qualify on this data — which is the\n\
             intended refusal rather than a bug: a curve bought out in its own\n\
             launch block is evidence of coordination, not of demand."
        );
    }
}

/// Reports whether any creator graduated more than once.
///
/// The whole question, in one number. If no creator has ever done it twice, a
/// creator's past graduation predicts nothing yet — not because the signal is
/// weak, but because there is no sample.
fn repeats(graduations: &[Graduated], per_creator: &BTreeMap<Address, u64>) {
    let mut by_creator: BTreeMap<Address, u64> = BTreeMap::new();
    for g in graduations {
        if let Some(creator) = g.creator {
            *by_creator.entry(creator).or_default() += 1;
        }
    }

    let repeaters: Vec<(&Address, &u64)> = by_creator.iter().filter(|(_, n)| **n > 1).collect();
    println!(
        "\ndistinct creators who graduated a token: {}",
        by_creator.len()
    );
    if repeaters.is_empty() {
        println!(
            "none of them did it twice, so \"has graduated before\" has no sample to\n\
             predict from yet. That is a fact about the dataset's age, not about the signal."
        );
        return;
    }
    println!("creators who did it more than once:");
    for (creator, count) in repeaters {
        println!(
            "  {creator}  {count} graduations from {} launches",
            per_creator.get(creator).copied().unwrap_or(0)
        );
    }
}

/// A rate as a percentage string, or a note when there is nothing to divide by.
fn rate(numerator: usize, denominator: usize) -> String {
    if denominator == 0 {
        // Not "0%". A rate over nothing is not a small number, it is not a
        // number, and printing 0% would be a claim the data does not support.
        return "no launches recorded".to_owned();
    }
    // Integer arithmetic to parts per million, then rendered. The rate is small
    // enough that two decimal places of a percent is the resolution that matters.
    let ppm = (numerator as u128 * 1_000_000) / denominator as u128;
    format!(
        "{}.{:04}%  ({numerator} of {denominator})",
        ppm / 10_000,
        ppm % 10_000
    )
}

/// Slots as hours, for display.
#[expect(
    clippy::cast_precision_loss,
    reason = "display only; slot counts here are far inside f64's exact range"
)]
fn as_hours(slots: u64) -> f64 {
    slots as f64 / SLOTS_PER_HOUR as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A graduation event for `mint` at `slot`, succeeded or not.
    fn grad_event(mint: u8, slot: u64, succeeded: bool) -> Event {
        Event::Graduation(Box::new(radar_store::Graduation {
            envelope: radar_store::Envelope {
                slot: Slot(slot),
                signature: radar_types::Signature::new([mint; 64]),
                tx_index: 3,
                instruction_index: 2,
                parent_index: None,
                succeeded,
            },
            origin: radar_store::Origin::known(Address::new([5u8; 32]), "migrate_v2"),
            mint: Address::new([mint; 32]),
        }))
    }

    #[test]
    fn a_failed_migration_is_not_a_graduation() {
        // It moved nothing. 35 of 97 migration rows in a sampled hour had failed,
        // so counting them would overstate the rarest label by more than a third.
        let events = [grad_event(1, 500, false)];
        assert!(distinct_graduations(&events, &BTreeMap::new()).is_empty());
    }

    #[test]
    fn a_token_with_two_successful_migrations_graduated_once() {
        // Real shape, from mainnet: a `migrate` at one slot and a `migrate_v2` a
        // slot later, same mint. 62 successful rows covered 50 mints that hour.
        let events = [
            grad_event(1, 441_251_537, true),
            grad_event(1, 441_251_536, true),
            grad_event(2, 441_251_600, true),
        ];
        let out = distinct_graduations(&events, &BTreeMap::new());
        assert_eq!(out.len(), 2, "a token graduates once");
    }

    #[test]
    fn the_earliest_successful_migration_is_the_graduation() {
        // Which one is kept decides the measured duration, and the duration is
        // what separates a bundled launch from a real one. The later row would
        // make an instant graduation look organic.
        let mut launches = BTreeMap::new();
        launches.insert(
            Address::new([1u8; 32]),
            (Address::new([9u8; 32]), Slot(441_251_535)),
        );
        let events = [
            grad_event(1, 441_252_500, true),
            grad_event(1, 441_251_536, true),
        ];
        let out = distinct_graduations(&events, &launches);
        assert_eq!(out[0].slots_to_graduate(), Some(1));
        assert_eq!(out[0].mode(), Some(GraduationMode::Instant));
    }

    #[test]
    fn a_rate_over_no_launches_is_not_zero_percent() {
        // Printing 0% would be a claim the data does not support.
        assert_eq!(rate(0, 0), "no launches recorded");
    }

    #[test]
    fn the_rate_renders_at_the_resolution_the_number_needs() {
        // The measured rate is ~0.017%, so a whole-percent renderer would print
        // "0%" for the finding that matters most.
        assert_eq!(rate(3, 17_869), "0.0167%  (3 of 17869)");
        assert_eq!(rate(1, 100), "1.0000%  (1 of 100)");
    }

    #[test]
    fn a_graduation_whose_launch_predates_the_store_reports_no_duration() {
        // Reporting zero would put a fabricated "graduated instantly" into the
        // fastest bucket, which is exactly where a reader looks first.
        let g = Graduated {
            mint: Address::new([1u8; 32]),
            creator: None,
            launch_slot: None,
            graduation_slot: Slot(500),
        };
        assert_eq!(g.slots_to_graduate(), None);
    }

    #[test]
    fn duration_is_measured_from_launch() {
        let g = Graduated {
            mint: Address::new([1u8; 32]),
            creator: Some(Address::new([2u8; 32])),
            launch_slot: Some(Slot(1_000)),
            graduation_slot: Slot(10_000),
        };
        assert_eq!(g.slots_to_graduate(), Some(9_000));
        assert!((as_hours(9_000) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_graduation_recorded_before_its_launch_is_zero_rather_than_wrapping() {
        // Partition replay can put a graduation ahead of the launch it belongs
        // to. A wrapped duration would be an astronomically large number in a
        // column a reader scans for outliers.
        let g = Graduated {
            mint: Address::new([1u8; 32]),
            creator: Some(Address::new([2u8; 32])),
            launch_slot: Some(Slot(10_000)),
            graduation_slot: Slot(1_000),
        };
        assert_eq!(g.slots_to_graduate(), Some(0));
    }
}
