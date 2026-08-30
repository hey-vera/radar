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

    // And everything the old file *did* carry comes back intact, which is the
    // other half: a reader that tolerated the missing column by returning an
    // empty row would pass the assertion above and be useless.
    assert_eq!(decision.strategy, "creator_edge");
    assert_eq!(decision.assumed_round_trip_bps, 850);
    assert_eq!(decision.coordination.as_deref(), Some("unremarkable"));
    assert_eq!(decision.kernel_reasons, vec!["NoAutonomy".to_owned()]);
    assert_eq!(decision.decided_at, Slot(10_000));
}
