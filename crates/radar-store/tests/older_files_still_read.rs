// SPDX-License-Identifier: Apache-2.0
//! A file written before a column existed must still be readable.
//!
//! Every schema change here is a change to files that already exist on disk and
//! cannot be rewritten. `authority_prevalence` was added to the decisions table
//! on 2026-08-30, after roughly nine hundred rows had already been recorded in
//! production, and the first version of that change read the column with the
//! *erroring* accessor.
//!
//! That would have made the entire recorded history unreadable — `radar
//! selection`, the decisions check in `radar brief`, `/v1/funnel` and the whole
//! interface, all failing at once on a store that was fine. `optional_u64_col`'s
//! doc comment describes exactly this failure, and the change made it anyway.
//!
//! So this test writes a decisions file with the *old* schema — by hand, since
//! the writer can only produce the new one — and reads it with the current
//! reader. It is the only kind of test that can catch the next instance.

use std::sync::Arc;

use arrow::array::{ArrayRef, ListBuilder, StringBuilder, UInt64Builder};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use parquet::arrow::ArrowWriter;
use radar_asof::AsOf;
use radar_store::{Reader, Table};
use radar_types::Slot;

/// The decisions schema as it stood *before* `authority_prevalence`.
///
/// Copied deliberately rather than derived from the live schema. Deriving it
/// would make the test track whatever the schema currently is, which is the one
/// thing it must not do — the point is to pin a shape that no longer exists.
fn schema_before_prevalence() -> Arc<Schema> {
    let reason_item = Arc::new(Field::new("item", DataType::Utf8, false));
    Arc::new(Schema::new(vec![
        Field::new("mint", DataType::Utf8, false),
        Field::new("creator", DataType::Utf8, false),
        Field::new("decided_at", DataType::UInt64, false),
        Field::new("launch_slot", DataType::UInt64, false),
        Field::new("strategy", DataType::Utf8, false),
        Field::new("strategy_version", DataType::Utf8, false),
        Field::new("conclusion", DataType::Utf8, false),
        Field::new("reasons", DataType::List(Arc::clone(&reason_item)), false),
        Field::new("notional_micro_usd", DataType::UInt64, true),
        Field::new("exit_capacity_micro_usd", DataType::UInt64, true),
        Field::new("assumed_round_trip_bps", DataType::UInt64, false),
        Field::new("coordination", DataType::Utf8, true),
        Field::new("kernel_outcome", DataType::Utf8, true),
        Field::new("kernel_reasons", DataType::List(reason_item), false),
        Field::new("entry_price", DataType::UInt64, true),
        Field::new("inputs_digest", DataType::Utf8, false),
    ]))
}

/// One row in that shape, written to a real Parquet file.
fn write_old_decision_file(dir: &std::path::Path) {
    let mut mint = StringBuilder::new();
    let mut creator = StringBuilder::new();
    let mut decided = UInt64Builder::new();
    let mut launch = UInt64Builder::new();
    let mut strategy = StringBuilder::new();
    let mut version = StringBuilder::new();
    let mut conclusion = StringBuilder::new();
    let mut reasons = ListBuilder::new(StringBuilder::new()).with_field(Arc::new(Field::new(
        "item",
        DataType::Utf8,
        false,
    )));
    let mut notional = UInt64Builder::new();
    let mut capacity = UInt64Builder::new();
    let mut cost = UInt64Builder::new();
    let mut coordination = StringBuilder::new();
    let mut kernel = StringBuilder::new();
    let mut kernel_reasons = ListBuilder::new(StringBuilder::new())
        .with_field(Arc::new(Field::new("item", DataType::Utf8, false)));
    let mut entry = UInt64Builder::new();
    let mut digest = StringBuilder::new();

    mint.append_value("So11111111111111111111111111111111111111112");
    creator.append_value("9BR3EaHtvyCbUqPWJHKgL3rEEJKvQTVWNQ3aJmXvVjkT");
    decided.append_value(10_000);
    launch.append_value(4_000);
    strategy.append_value("creator_edge");
    version.append_value("0.1.0");
    conclusion.append_value("proposed");
    reasons.append(true);
    notional.append_value(6_300_000);
    capacity.append_option(None);
    cost.append_value(850);
    coordination.append_value("unremarkable");
    kernel.append_value("refused");
    kernel_reasons.values().append_value("NoAutonomy");
    kernel_reasons.append(true);
    entry.append_option(None);
    digest.append_value("abc");

    let columns: Vec<ArrayRef> = vec![
        Arc::new(mint.finish()),
        Arc::new(creator.finish()),
        Arc::new(decided.finish()),
        Arc::new(launch.finish()),
        Arc::new(strategy.finish()),
        Arc::new(version.finish()),
        Arc::new(conclusion.finish()),
        Arc::new(reasons.finish()),
        Arc::new(notional.finish()),
        Arc::new(capacity.finish()),
        Arc::new(cost.finish()),
        Arc::new(coordination.finish()),
        Arc::new(kernel.finish()),
        Arc::new(kernel_reasons.finish()),
        Arc::new(entry.finish()),
        Arc::new(digest.finish()),
    ];
    let batch =
        RecordBatch::try_new(schema_before_prevalence(), columns).expect("the old shape is valid");

    let table = dir.join(Table::Decisions.dir());
    std::fs::create_dir_all(&table).expect("mkdir");
    let file =
        std::fs::File::create(table.join("slot_000000000000_g0001.parquet")).expect("create");
    let mut writer = ArrowWriter::try_new(file, batch.schema(), None).expect("writer");
    writer.write(&batch).expect("write");
    writer.close().expect("close");
}

#[test]
fn a_decision_file_written_before_authority_prevalence_still_reads() {
    // The regression. Read with the erroring accessor this returns an error and
    // every caller that touches decisions fails at once, on a store that is
    // perfectly fine.
    let dir = tempfile::tempdir().expect("tempdir");
    write_old_decision_file(dir.path());

    let decisions = Reader::open(dir.path())
        .read_decisions(AsOf::at(Slot(20_000)))
        .expect("a file written before the column existed is still a valid file");

    assert_eq!(decisions.len(), 1);
    let decision = &decisions[0];

    // The column is absent, so the value is absent -- not empty, and not a
    // guess. `None` here means "this decision predates the measurement", which
    // is exactly what it should mean.
    assert_eq!(decision.authority_prevalence, None);

    // Added 2026-09-04, and absent from this file for the same reason. The two
    // together are the point: **every decision recorded before today has no
    // launch-block count**, and those are exactly the rows a re-derived
    // threshold would be fitted against. A reader that failed on them would
    // make the history unreadable in the same change that started recording the
    // number -- so the count is `None`, meaning "this decision predates the
    // measurement", and never zero, which would mean "the block held nobody".
    assert_eq!(decision.launch_recipients, None);
    assert_eq!(decision.launch_transactions, None);

    // And everything the old file *did* carry comes back intact, which is the
    // other half: a reader that tolerated the missing column by returning an
    // empty row would pass the assertion above and be useless.
    assert_eq!(decision.strategy, "creator_edge");
    assert_eq!(decision.assumed_round_trip_bps, 850);
    assert_eq!(decision.coordination.as_deref(), Some("unremarkable"));
    assert_eq!(decision.kernel_reasons, vec!["NoAutonomy".to_owned()]);
    assert_eq!(decision.decided_at, Slot(10_000));
}

/// The outcomes schema as it stood *before* the window extremes.
///
/// Copied deliberately rather than derived from `schema_for`, for the reason the
/// decisions half of this file gives: deriving it would make the test track
/// whatever the schema currently is, which is the one thing it must not do.
fn outcomes_schema_before_window_extremes() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("mint", DataType::Utf8, false),
        Field::new("measured_at", DataType::UInt64, false),
        Field::new("launch_slot", DataType::UInt64, false),
        Field::new("first_transfer_slot", DataType::UInt64, true),
        Field::new("last_transfer_slot", DataType::UInt64, true),
        Field::new("transfers", DataType::UInt64, false),
        Field::new("unique_senders", DataType::UInt64, false),
        Field::new("unique_receivers", DataType::UInt64, false),
        Field::new("graduated_at", DataType::UInt64, true),
        Field::new("first_price", DataType::UInt64, true),
        Field::new("last_price", DataType::UInt64, true),
        Field::new("peak_price", DataType::UInt64, true),
        Field::new("trough_price", DataType::UInt64, true),
        // `window_peak_price` and `window_trough_price` belong here and are
        // absent on purpose. That absence is the whole test.
        Field::new("vwap", DataType::UInt64, true),
        Field::new("fills", DataType::UInt64, false),
    ]))
}

fn write_old_outcome_file(dir: &std::path::Path) {
    let outcomes = dir.join("outcomes");
    std::fs::create_dir_all(&outcomes).expect("a directory");

    let schema = outcomes_schema_before_window_extremes();
    let mut mint = StringBuilder::new();
    mint.append_value("So11111111111111111111111111111111111111112");

    let u64_col = |v: Option<u64>| -> ArrayRef {
        let mut b = UInt64Builder::new();
        b.append_option(v);
        Arc::new(b.finish())
    };

    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(mint.finish()),
            u64_col(Some(9_000)),
            u64_col(Some(1_000)),
            u64_col(None),
            u64_col(None),
            u64_col(Some(12)),
            u64_col(Some(4)),
            u64_col(Some(5)),
            u64_col(None),
            u64_col(Some(1_000)),
            u64_col(Some(1_500)),
            u64_col(Some(2_000)),
            u64_col(Some(900)),
            u64_col(Some(1_400)),
            u64_col(Some(12)),
        ],
    )
    .expect("a batch in the old shape");

    let path = outcomes.join("slot_000000009000_g0000.parquet");
    let file = std::fs::File::create(path).expect("a file");
    let mut writer = ArrowWriter::try_new(file, schema, None).expect("a writer");
    writer.write(&batch).expect("writes");
    writer.close().expect("closes");
}

#[test]
fn an_outcome_file_written_before_the_window_extremes_still_reads() {
    // The same failure as the decisions half, one table over. Production holds
    // roughly a million outcome measurements written before these two columns
    // existed, and reading them with the erroring accessor would fail every
    // research command, the interface, and `radar brief` at once — on a store
    // that is perfectly fine.
    let dir = tempfile::tempdir().expect("a temporary directory");
    write_old_outcome_file(dir.path());

    let reader = Reader::open(dir.path());
    let outcomes = reader
        .read_outcomes(AsOf::at(Slot(10_000)))
        .expect("a file missing a later column still reads");

    assert_eq!(outcomes.len(), 1, "the row is there");
    let outcome = &outcomes[0];

    // The columns that existed are intact.
    assert_eq!(outcome.peak_price, Some(2_000));
    assert_eq!(outcome.trough_price, Some(900));
    assert_eq!(outcome.fills, 12);

    // And the ones that did not read as "not measured" rather than as zero.
    // Zero would be a claim: that the window's high was nothing, which would
    // make every exit-rule simulation over this row nonsense.
    assert_eq!(outcome.window_peak_price, None);
    assert_eq!(outcome.window_trough_price, None);
}

/// Files written by a **previous version of the parquet library** still read.
///
/// The tests above all write with the same `parquet` this test binary links, so
/// none of them can see a change in the library's own output. That is the gap
/// this closes, and it is not hypothetical: the `arrow`/`parquet` 56 to 59 bump
/// landed on a store holding hundreds of megabytes written by 56, and the two
/// crates had to move together — bumped apart, the tree holds two `RecordBatch`
/// types and does not compile, which is why Dependabot's separate pull requests
/// both failed while the combined change is clean.
///
/// The fixtures are **real production files**, copied off the recorder's store,
/// written by `parquet` 56.2.1. Real rather than synthesised on purpose: a file
/// this test wrote would encode whatever the writing library chose to do, and
/// the question is what the *previous* one actually did.
///
/// Small on purpose — the two smallest partitions in the store, 3.5KB and 5.2KB
/// — because a fixture is checked in forever.
///
/// If this ever fails after a dependency bump, the store on disk has become
/// unreadable and no amount of green elsewhere makes that safe to ship.
#[test]
fn files_written_by_an_older_parquet_are_still_readable() {
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");

    // The store's layout is a directory per table, so the fixtures are laid out
    // as one rather than read as loose files: this exercises the same path
    // production uses, not a private entry point.
    let store = tempfile::tempdir().expect("tempdir");
    for (table, fixture) in [
        ("graduations", "graduations_written_by_parquet_56.parquet"),
        ("launches", "launches_written_by_parquet_56.parquet"),
    ] {
        let dir = store.path().join(table);
        std::fs::create_dir_all(&dir).expect("create table dir");
        std::fs::copy(fixtures.join(fixture), dir.join(fixture)).expect("copy fixture");
    }

    let reader = Reader::open(store.path());

    // The watermark comes from the row data, so a non-zero answer proves the
    // file was decoded rather than merely opened.
    let watermark = reader
        .watermark()
        .expect("watermark readable")
        .expect("a store with two partitions has a watermark");
    assert!(
        watermark > Slot(440_000_000),
        "watermark {watermark:?} should be a real recorded slot"
    );

    let as_of = AsOf::at(watermark);
    let launches = reader.read(Table::Launches, as_of).expect("launches read");
    let graduations = reader
        .read(Table::Graduations, as_of)
        .expect("graduations read");

    assert!(
        !launches.is_empty(),
        "the launches fixture decoded to zero rows, which is what an unreadable \
         file looks like from here"
    );
    assert!(
        !graduations.is_empty(),
        "the graduations fixture decoded to zero rows"
    );
}
