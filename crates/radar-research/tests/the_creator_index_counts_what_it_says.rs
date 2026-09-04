// SPDX-License-Identifier: Apache-2.0
//! What the creator index counts, against a real store.
//!
//! # Why this file exists
//!
//! The builder shipped with **no tests at all** and CI said so: every mutant
//! survived — the failed-launch filter deleted, every counter turned into a
//! subtraction, the latest-measurement comparison reversed. All of it passed,
//! because nothing ever called `build`.
//!
//! That is worse here than in most places. These counts are published in a
//! reply about somebody's project: "forty-seven launches, none of which ever
//! filled its curve" is a specific, checkable, damaging claim, and it has to be
//! right. A counter that decremented would produce a flattering number, which
//! is the direction nobody notices.

use radar_asof::AsOf;
use radar_research::creator_index;
use radar_store::{Envelope, Event, Launch, Origin, Outcome, Reader, Writer};
use radar_types::{Address, Signature, Slot};

fn address(n: u8) -> Address {
    Address::new([n; 32])
}

fn pumpfun() -> Address {
    "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P"
        .parse()
        .expect("program id")
}

fn launch(mint: u8, creator: u8, slot: u64, succeeded: bool) -> Event {
    Event::Launch(Box::new(Launch {
        envelope: Envelope {
            slot: Slot(slot),
            signature: Signature::new([(slot % 251) as u8; 64]),
            tx_index: 0,
            instruction_index: 1,
            parent_index: None,
            succeeded,
        },
        origin: Origin::known(pumpfun(), "create_v2"),
        mint: address(mint),
        creator: address(creator),
        name: "a token".to_owned(),
        symbol: "TKN".to_owned(),
        uri: "https://example.test".to_owned(),
        dev_buy_lamports: None,
    }))
}

/// An outcome, with graduation expressed as slots after launch.
///
/// `graduated_after` of `None` never graduated; `Some(0..=3)` is instant and
/// anything larger is organic — the store's own `INSTANT_WITHIN_SLOTS`, which
/// this test deliberately does not restate as a number.
fn outcome(
    mint: u8,
    measured: u64,
    launch_slot: u64,
    graduated_after: Option<u64>,
    transfers: u64,
) -> Outcome {
    // Survived four hundred slots, which is past the stillborn window. A test
    // that wants a stillborn token uses `stillborn` below rather than editing
    // this, so the two cases stay visibly different.
    Outcome {
        mint: address(mint),
        measured_at: Slot(measured),
        launch_slot: Slot(launch_slot),
        first_transfer_slot: Some(Slot(launch_slot)),
        last_transfer_slot: Some(Slot(launch_slot + 400)),
        transfers,
        unique_senders: 7,
        unique_receivers: 5,
        graduated_at: graduated_after.map(|a| Slot(launch_slot + a)),
        first_price: None,
        last_price: None,
        peak_price: None,
        trough_price: None,
        window_peak_price: None,
        window_trough_price: None,
        vwap: None,
        fills: 0,
    }
}

/// A token that showed almost no life: few transfers, over quickly.
///
/// The store's rule is five or fewer transfers inside three hundred slots, and
/// this deliberately does not restate those numbers as its own -- it builds a
/// token that is obviously inside them on both axes.
fn stillborn(mint: u8, measured: u64, launch_slot: u64) -> Outcome {
    let mut o = outcome(mint, measured, launch_slot, None, 2);
    o.last_transfer_slot = Some(Slot(launch_slot + 10));
    o
}

/// Builds a store and returns the index over it.
fn index_over(events: Vec<Event>, outcomes: Vec<Outcome>) -> radar_roast::CreatorIndex {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut writer = Writer::open(dir.path(), 1_000).expect("open");
    for event in events {
        writer.append(event).expect("append");
    }
    for outcome in outcomes {
        writer.append_outcome(outcome).expect("append outcome");
    }
    writer.flush().expect("flush");

    let reader = Reader::open(dir.path());
    let watermark = reader
        .watermark()
        .expect("watermark readable")
        .expect("a store with rows has one");
    creator_index::build(&reader, AsOf::at(watermark), 1_788_000_000).expect("built")
}

#[test]
fn a_failed_launch_is_not_a_launch() {
    // The recorder keeps failed transactions because a spam burst is real
    // information about the market. But a creator credited with them is a
    // creator ranked on somebody else's failed transactions -- and on
    // 2026-08-24 a two-minute burst produced 7,233 of them from one source.
    let index = index_over(
        vec![
            launch(1, 100, 1_000, true),
            launch(2, 100, 1_001, false),
            launch(3, 100, 1_002, false),
        ],
        Vec::new(),
    );

    let record = index.get(&address(100).to_string()).expect("the creator");
    assert_eq!(
        record.launches, 1,
        "two of the three never happened: {record:?}"
    );
}

#[test]
fn each_creator_is_counted_separately() {
    let index = index_over(
        vec![
            launch(1, 100, 1_000, true),
            launch(2, 100, 1_001, true),
            launch(3, 200, 1_002, true),
        ],
        Vec::new(),
    );

    assert_eq!(index.len(), 2, "two creators");
    assert!(!index.is_empty());
    assert_eq!(index.get(&address(100).to_string()).expect("a").launches, 2);
    assert_eq!(index.get(&address(200).to_string()).expect("b").launches, 1);
}

#[test]
fn graduation_is_split_by_how_the_curve_filled() {
    // The distinction the whole signal rests on. A curve bought out within
    // three slots was bought by capital committed before the token existed --
    // evidence of coordination rather than demand -- so a creator ranked on the
    // undifferentiated count is ranked partly on how well they bundle.
    let index = index_over(
        vec![
            launch(1, 100, 1_000, true),
            launch(2, 100, 1_001, true),
            launch(3, 100, 1_002, true),
        ],
        vec![
            // Filled over time.
            outcome(1, 2_000, 1_000, Some(5_000), 900),
            // Filled inside its own launch block.
            outcome(2, 2_000, 1_001, Some(0), 900),
            // Never graduated at all.
            outcome(3, 2_000, 1_002, None, 900),
        ],
    );

    let record = index.get(&address(100).to_string()).expect("the creator");
    assert_eq!(record.launches, 3);
    assert_eq!(record.measured, 3);
    assert_eq!(record.organic, 1, "one filled over time: {record:?}");
    assert_eq!(record.instant, 1, "one filled in a block: {record:?}");
}

#[test]
fn the_latest_measurement_of_a_mint_is_the_one_that_counts() {
    // A mint is measured repeatedly as it ages, and what is known *now* is the
    // last measurement taken. An earlier one saying "never graduated" must not
    // override a later one saying it did.
    //
    // The comparison that decides this had three surviving mutants -- `>` as
    // `<`, `>=` and `==` -- and reversing it makes a graduated token read as
    // one that never was, which is the flattering direction.
    let index = index_over(
        vec![launch(1, 100, 1_000, true)],
        vec![
            outcome(1, 1_500, 1_000, None, 10),
            outcome(1, 9_000, 1_000, Some(5_000), 900),
            outcome(1, 3_000, 1_000, None, 100),
        ],
    );

    let record = index.get(&address(100).to_string()).expect("the creator");
    assert_eq!(record.measured, 1, "three measurements, one mint");
    assert_eq!(
        record.organic, 1,
        "the latest measurement says it graduated: {record:?}"
    );
}

#[test]
fn a_launch_with_no_measurement_is_counted_but_not_measured() {
    // The gap between the two is the denominator a reply must quote. Publishing
    // "0 graduated" from an unmeasured population would be a measurement of
    // Radar's own lag presented as a fact about the creator.
    let index = index_over(
        vec![launch(1, 100, 1_000, true), launch(2, 100, 1_001, true)],
        vec![outcome(1, 2_000, 1_000, None, 900)],
    );

    let record = index.get(&address(100).to_string()).expect("the creator");
    assert_eq!(record.launches, 2);
    assert_eq!(record.measured, 1, "only one has been looked at");
}

#[test]
fn an_outcome_for_a_mint_no_launch_was_recorded_for_is_ignored() {
    // The store holds outcomes for tokens whose launch predates the recorder.
    // Attributing them would need a creator nobody recorded, and inventing one
    // is worse than counting nothing.
    let index = index_over(
        vec![launch(1, 100, 1_000, true)],
        vec![
            outcome(1, 2_000, 1_000, None, 900),
            outcome(99, 2_000, 900, Some(5_000), 900),
        ],
    );

    let record = index.get(&address(100).to_string()).expect("the creator");
    assert_eq!(record.measured, 1);
    assert_eq!(record.organic, 0, "the orphan must not be credited here");
    assert_eq!(index.len(), 1, "and it must not invent a creator");
}

#[test]
fn an_empty_store_yields_an_empty_index_rather_than_a_failure() {
    let index = index_over(vec![launch(1, 100, 1_000, false)], Vec::new());
    assert!(index.is_empty(), "one failed launch is no launches");
    assert_eq!(index.len(), 0);
    assert_eq!(index.get(&address(100).to_string()), None);
}

#[test]
fn a_token_that_showed_almost_no_life_is_counted_as_such() {
    // The most common outcome on this venue, and the one a reply most often
    // has to report: 33 of a creator's 41 measured tokens going nowhere is the
    // sentence that tells somebody what they are buying into.
    //
    // Nothing tested it, so the counter could be decremented and every other
    // assertion still held.
    let index = index_over(
        vec![
            launch(1, 100, 1_000, true),
            launch(2, 100, 1_001, true),
            launch(3, 100, 1_002, true),
        ],
        vec![
            stillborn(1, 2_000, 1_000),
            stillborn(2, 2_000, 1_001),
            // Traded for a long time and never graduated: measured, not
            // stillborn, and not a graduation either.
            outcome(3, 2_000, 1_002, None, 900),
        ],
    );

    let record = index.get(&address(100).to_string()).expect("the creator");
    assert_eq!(record.measured, 3);
    assert_eq!(record.stillborn, 2, "two went nowhere: {record:?}");
    assert_eq!(record.organic, 0);
    assert_eq!(record.instant, 0);
}

#[test]
fn the_population_is_the_sum_of_its_parts() {
    // The population is accumulated in its own pass rather than summed from the
    // records at the end, which is the only way it can disagree with them --
    // and the only way it is worth having, because a total summed from the
    // parts would agree with a per-creator bug rather than catch it.
    //
    // Two creators, one of whom has every outcome shape, so no counter can be
    // attached to the wrong accumulator without this failing.
    let index = index_over(
        vec![
            launch(1, 100, 1_000, true),
            launch(2, 100, 1_001, true),
            launch(3, 100, 1_002, true),
            launch(4, 200, 1_003, true),
            // Never happened, so it is in neither total.
            launch(5, 200, 1_004, false),
        ],
        vec![
            outcome(1, 2_000, 1_000, Some(5_000), 900),
            outcome(2, 2_000, 1_001, Some(0), 900),
            stillborn(3, 2_000, 1_002),
            outcome(4, 2_000, 1_003, Some(5_000), 900),
        ],
    );

    let population = index.population.expect("a build always measures one");
    let summed = index
        .creators
        .values()
        .fold((0_u64, 0_u64, 0_u64, 0_u64, 0_u64), |mut acc, r| {
            acc.0 += u64::from(r.launches);
            acc.1 += u64::from(r.measured);
            acc.2 += u64::from(r.organic);
            acc.3 += u64::from(r.instant);
            acc.4 += u64::from(r.stillborn);
            acc
        });
    assert_eq!(
        (
            population.launches,
            population.measured,
            population.organic,
            population.instant,
            population.stillborn
        ),
        summed,
        "the total and the parts were counted separately and must agree: {population:?}"
    );
    // And the absolute values, so that both halves being wrong the same way is
    // still caught.
    assert_eq!(population.launches, 4, "the failed launch is not one");
    assert_eq!(population.measured, 4);
    assert_eq!(population.organic, 2);
    assert_eq!(population.instant, 1);
    assert_eq!(population.stillborn, 1);
}

#[test]
fn an_empty_store_measures_a_population_rather_than_declining_to() {
    // `None` means "written by a version that did not measure this". A build
    // that found nothing must say so as zeroes-with-no-denominator, because the
    // consumer's two branches are "not measured" and "measured, and empty" and
    // they produce different replies.
    let index = index_over(vec![launch(1, 100, 1_000, false)], Vec::new());
    let population = index.population.expect("a build always measures one");
    assert_eq!(population.launches, 0);
    assert_eq!(
        population.graduated_share(),
        None,
        "no denominator, so no share -- rule 9, and this is the flattering direction"
    );
}
