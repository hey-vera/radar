// SPDX-License-Identifier: Apache-2.0
//! What the feature table may and may not know, against a real store.
//!
//! # Why this file exists
//!
//! Design 0010 §6.2: a leaked feature does not fail on the second fold, it wins
//! every fold. So no amount of care in the walk-forward protocol can catch one,
//! and the only place it can be caught is here — at the moment a number enters
//! a row. The unit tests in `features` prove the guard refuses a value stamped
//! after T. These prove the harder half: that the values handed to the guard
//! are stamped honestly, so the guard has something true to check.
//!
//! The load-bearing case is a creator's record. A creator whose earlier token
//! graduated has a different record depending on **when somebody measured it**,
//! and the measurement usually arrives hours after the fact. Counting the
//! measurement rather than the moment it was taken is the leak that would make
//! `creator_edge` look prophetic in a backtest and ordinary in production.

use radar_asof::AsOf;
use radar_research::features::{self, FeatureTable};
use radar_store::{Envelope, Event, Launch, Origin, Outcome, Reader, Trade, Writer};
use radar_types::{Address, Signature, Slot};

/// The offset from a launch to T, taken from the module rather than restated.
const T: u64 = features::ENTRY_OFFSET_SLOTS;

fn address(n: u8) -> Address {
    Address::new([n; 32])
}

fn pumpfun() -> Address {
    "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P"
        .parse()
        .expect("program id")
}

fn launch(mint: u8, creator: u8, slot: u64) -> Event {
    launch_named(mint, creator, slot, "a token", "TKN")
}

fn launch_named(mint: u8, creator: u8, slot: u64, name: &str, symbol: &str) -> Event {
    Event::Launch(Box::new(Launch {
        envelope: Envelope {
            slot: Slot(slot),
            signature: Signature::new([(slot % 251) as u8; 64]),
            tx_index: 0,
            instruction_index: 1,
            parent_index: None,
            succeeded: true,
        },
        origin: Origin::known(pumpfun(), "create_v2"),
        mint: address(mint),
        creator: address(creator),
        name: name.to_owned(),
        symbol: symbol.to_owned(),
        uri: "https://metadata.test/one".to_owned(),
        dev_buy_lamports: Some(500_000_000),
    }))
}

fn failed_launch(mint: u8, creator: u8, slot: u64) -> Event {
    let Event::Launch(mut l) = launch(mint, creator, slot) else {
        unreachable!("launch builds a launch")
    };
    l.envelope.succeeded = false;
    Event::Launch(l)
}

/// A buy of `sol` SOL by `trader`, at `slot`, in transaction position `tx`.
fn buy(mint: u8, trader: u8, slot: u64, tx: u32, sol: u64) -> Event {
    Event::Trade(Box::new(Trade {
        envelope: Envelope {
            slot: Slot(slot),
            signature: Signature::new([(slot % 251) as u8; 64]),
            tx_index: tx,
            instruction_index: 0,
            parent_index: None,
            succeeded: true,
        },
        origin: Origin::known(pumpfun(), "buy"),
        mint: address(mint),
        trader: address(trader),
        side: radar_store::Side::Buy,
        realised_lamports: Some(sol * 1_000_000_000),
        realised_tokens: Some(1),
        requested_amount: sol * 1_000_000_000,
        requested_is_lamports: true,
        limit_amount: 0,
        accepted_any_price: false,
    }))
}

/// A measurement. `graduated_after` of `None` never graduated; a value larger
/// than the store's own `INSTANT_WITHIN_SLOTS` is organic.
fn outcome(
    mint: u8,
    measured: u64,
    launch_slot: u64,
    graduated_after: Option<u64>,
    price: Option<u64>,
) -> Outcome {
    Outcome {
        mint: address(mint),
        measured_at: Slot(measured),
        launch_slot: Slot(launch_slot),
        first_transfer_slot: Some(Slot(launch_slot)),
        last_transfer_slot: Some(Slot(launch_slot + 400)),
        transfers: 40,
        unique_senders: 7,
        unique_receivers: 5,
        graduated_at: graduated_after.map(|a| Slot(launch_slot + a)),
        first_price: price,
        last_price: price,
        peak_price: price,
        trough_price: price,
        window_peak_price: None,
        window_trough_price: None,
        vwap: price,
        fills: 3,
    }
}

/// Builds a store from the fixtures and returns the table over all of it.
fn table_over(events: Vec<Event>, outcomes: Vec<Outcome>) -> FeatureTable {
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
    features::build(&reader, AsOf::at(watermark), Slot(0), Slot(u64::MAX)).expect("built")
}

/// One row's feature, by name.
fn value(table: &FeatureTable, mint: u8, feature: &str) -> Option<f64> {
    let index = features::feature_index(feature).expect("a known feature");
    table
        .rows
        .iter()
        .find(|r| r.mint == address(mint))
        .unwrap_or_else(|| panic!("no row for mint {mint}"))
        .value(index)
}

#[test]
fn a_creators_record_counts_only_what_had_been_measured_by_t() {
    // The leak, planted. Mint 1 graduated organically at slot 1,100 — but
    // nobody measured it until well after mint 2's T. At mint 2's T the honest
    // answer is that this creator has one prior launch and no known
    // graduation, and a table that says otherwise is reading a measurement
    // that had not been taken.
    let second_launch = 20_000;
    let after_t = second_launch + T + 1_000;

    let leaky = table_over(
        vec![launch(1, 100, 1_000), launch(2, 100, second_launch)],
        vec![outcome(1, after_t, 1_000, Some(100), None)],
    );

    assert_eq!(
        value(&leaky, 2, "creator_prior_launches"),
        Some(1.0),
        "the earlier launch is a fact at its own slot and is counted"
    );
    assert_eq!(
        value(&leaky, 2, "creator_prior_organic"),
        Some(0.0),
        "the graduation was not measured until after T, so at T it was not known"
    );
}

#[test]
fn the_same_record_measured_before_t_does_count() {
    // The other side, and the reason the test above is not satisfied by a
    // function that returns zero. Same store, one number moved: the
    // measurement now lands before T.
    let second_launch = 20_000;
    let before_t = second_launch + T - 1_000;

    let known = table_over(
        vec![launch(1, 100, 1_000), launch(2, 100, second_launch)],
        vec![outcome(1, before_t, 1_000, Some(100), None)],
    );

    assert_eq!(
        value(&known, 2, "creator_prior_organic"),
        Some(1.0),
        "measured before T, so it was known at T"
    );
}

#[test]
fn the_launch_block_is_counted_from_what_the_store_recorded() {
    // Contiguity is the bundle's shape: three transactions in a row, then a
    // gap, then one more. Four transactions, three of them consecutive, and
    // three distinct accounts.
    let at = 1_000;
    let table = table_over(
        vec![
            launch(1, 100, at),
            buy(1, 10, at, 3, 1),
            buy(1, 11, at, 4, 1),
            buy(1, 12, at, 5, 1),
            buy(1, 10, at, 9, 1),
        ],
        vec![],
    );

    assert_eq!(value(&table, 1, "launch_transactions"), Some(4.0));
    assert_eq!(value(&table, 1, "launch_traders"), Some(3.0));
    assert_eq!(
        value(&table, 1, "launch_contiguity"),
        Some(3.0),
        "positions 3, 4 and 5 are one run; 9 is on its own"
    );
}

#[test]
fn liquidity_velocity_counts_the_buys_that_moved_the_curve() {
    // Four buys of ten SOL each. Ten is reached on the first, twenty on the
    // second, thirty on the third — and the fourth does not change any of them.
    let at = 1_000;
    let table = table_over(
        vec![
            launch(1, 100, at),
            buy(1, 10, at + 1, 0, 10),
            buy(1, 11, at + 2, 0, 10),
            buy(1, 12, at + 3, 0, 10),
            buy(1, 13, at + 4, 0, 10),
        ],
        vec![],
    );

    assert_eq!(value(&table, 1, "trades_to_10_sol"), Some(1.0));
    assert_eq!(value(&table, 1, "trades_to_20_sol"), Some(2.0));
    assert_eq!(value(&table, 1, "trades_to_30_sol"), Some(3.0));
}

#[test]
fn a_depth_never_reached_is_absent_rather_than_a_large_number() {
    // Rule 9. A curve that took two SOL and stopped did not take thirty in some
    // very large number of trades; nobody measured how many it would have
    // taken. A stratum on this feature must not include the row at all.
    let at = 1_000;
    let table = table_over(vec![launch(1, 100, at), buy(1, 10, at + 1, 0, 2)], vec![]);

    assert_eq!(value(&table, 1, "trades_to_10_sol"), None);
    assert_eq!(value(&table, 1, "trades_to_30_sol"), None);
}

#[test]
fn a_failed_launch_gets_no_row() {
    // The same rule the creator index keeps, for the same reason: a burst of
    // failed transactions is information about the market and not a launch.
    let table = table_over(
        vec![launch(1, 100, 1_000), failed_launch(2, 100, 1_001)],
        vec![],
    );

    assert_eq!(table.rows.len(), 1);
    assert_eq!(table.rows[0].mint, address(1));
}

#[test]
fn the_label_is_the_return_between_two_checkpoints_after_t() {
    // Labels are read from the future on purpose. Entry is the first
    // measurement at or after T; the six-hour exit is the last one by then.
    let at = 1_000;
    let entry_at = at + T;
    let six_hours = at + 6 * 9_000;

    let table = table_over(
        vec![launch(1, 100, at)],
        vec![
            outcome(1, entry_at, at, None, Some(100)),
            outcome(1, six_hours, at, None, Some(150)),
        ],
    );

    let row = &table.rows[0];
    assert_eq!(
        row.gross_6h_bps,
        Some(5_000.0),
        "100 to 150 is fifty per cent, which is 5,000 bps"
    );
}

#[test]
fn a_missing_price_leaves_the_label_absent_rather_than_zero() {
    // A return of zero is a claim that nothing moved. "Nobody recorded a price"
    // is a different fact, and a fold that treated them alike would fill its
    // point mass at zero with tokens nobody measured.
    let at = 1_000;
    let table = table_over(
        vec![launch(1, 100, at)],
        vec![outcome(1, at + T, at, None, None)],
    );

    assert_eq!(table.rows[0].gross_6h_bps, None);
    assert_eq!(table.rows[0].gross_24h_bps, None);
}

#[test]
fn prior_name_and_symbol_count_only_earlier_launches() {
    // The shape of a factory, and an off-by-one here would tell every launch
    // that one earlier launch shared its name — including the first.
    let table = table_over(
        vec![
            launch_named(1, 100, 1_000, "same", "AAA"),
            launch_named(2, 101, 2_000, "same", "BBB"),
            launch_named(3, 102, 3_000, "other", "AAA"),
        ],
        vec![],
    );

    assert_eq!(value(&table, 1, "prior_same_name"), Some(0.0));
    assert_eq!(value(&table, 2, "prior_same_name"), Some(1.0));
    assert_eq!(value(&table, 3, "prior_same_name"), Some(0.0));
    assert_eq!(value(&table, 3, "prior_same_symbol"), Some(1.0));
    assert_eq!(
        value(&table, 1, "prior_same_uri_host"),
        Some(0.0),
        "the host is shared by all three, and the first has nobody before it"
    );
    assert_eq!(value(&table, 3, "prior_same_uri_host"), Some(2.0));
}

#[test]
fn the_table_is_the_same_twice_and_ordered_by_launch_slot() {
    // The replay standard, applied to the harness. A table that depends on
    // iteration order cannot be compared across runs, and every later verdict
    // is computed from this one.
    let events = || {
        vec![
            launch(3, 102, 3_000),
            launch(1, 100, 1_000),
            launch(2, 101, 2_000),
            buy(1, 10, 1_000, 1, 5),
            buy(2, 11, 2_000, 2, 5),
        ]
    };
    let outcomes = || {
        vec![
            outcome(1, 1_000 + T, 1_000, Some(100), Some(10)),
            outcome(2, 2_000 + T, 2_000, None, Some(20)),
        ]
    };

    let first = table_over(events(), outcomes());
    let second = table_over(events(), outcomes());

    assert_eq!(first, second, "two builds of the same store must agree");
    let slots: Vec<u64> = first.rows.iter().map(|r| r.launch_slot.get()).collect();
    assert_eq!(slots, vec![1_000, 2_000, 3_000]);
}

#[test]
fn the_file_says_what_the_table_said() {
    // The writer would otherwise be a layer with no caller, and a format
    // nothing reads is a format nobody notices is wrong. Absence has to survive
    // the round trip too: a column of nulls read back as zeros would turn every
    // unmeasured depth into a curve that filled in no trades.
    let table = table_over(
        vec![
            launch(1, 100, 1_000),
            buy(1, 10, 1_000, 3, 40),
            launch(2, 101, 2_000),
        ],
        vec![
            outcome(1, 1_000 + T, 1_000, Some(100), Some(10)),
            outcome(1, 1_000 + 6 * 9_000, 1_000, Some(100), Some(25)),
        ],
    );

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join(features::file_name(table.watermark));
    features::write(&table, &path).expect("written");
    let read_back = features::read(&path).expect("read");

    assert_eq!(read_back, table);
    assert!(
        read_back.rows.iter().any(|r| r
            .value(features::feature_index("trades_to_30_sol").expect("known"))
            .is_none()),
        "a depth nobody reached must still be absent after a round trip"
    );
}

#[test]
fn the_same_table_writes_the_same_bytes() {
    // `radar features` run twice produces identical bytes — the gate plan 0007
    // sets for item 1, and what makes a research note's sha256 mean anything.
    let table = table_over(
        vec![launch(1, 100, 1_000), buy(1, 10, 1_000, 1, 5)],
        vec![outcome(1, 1_000 + T, 1_000, None, Some(10))],
    );

    let dir = tempfile::tempdir().expect("tempdir");
    let first = dir.path().join("first.parquet");
    let second = dir.path().join("second.parquet");
    features::write(&table, &first).expect("first");
    features::write(&table, &second).expect("second");

    assert_eq!(
        std::fs::read(&first).expect("read first"),
        std::fs::read(&second).expect("read second"),
        "two writes of one table must be byte-identical"
    );
}

#[test]
fn an_empty_table_still_carries_its_watermark() {
    // An empty fold is a legitimate result, and a table that lost its watermark
    // would have its fold boundaries computed against slot zero — which
    // silently includes everything.
    let table = table_over(vec![launch(1, 100, 1_000)], vec![]);
    let empty = radar_research::features::FeatureTable {
        watermark: table.watermark,
        entry_offset: table.entry_offset,
        rows: Vec::new(),
    };

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("empty.parquet");
    features::write(&empty, &path).expect("written");

    assert_eq!(features::read(&path).expect("read"), empty);
}

#[test]
fn a_file_that_is_not_a_feature_table_is_refused() {
    // Reading something else as a table of zeros is the failure this replaces.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("not-parquet.parquet");
    std::fs::write(&path, b"this is not parquet").expect("write");

    assert!(features::read(&path).is_err());
}
