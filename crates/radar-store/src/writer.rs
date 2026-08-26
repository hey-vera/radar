// SPDX-License-Identifier: Apache-2.0
//! Append-only Parquet writer, partitioned by slot range.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow::array::{
    ArrayRef, BooleanBuilder, ListBuilder, StringBuilder, UInt32Builder, UInt64Builder,
};
use arrow::record_batch::RecordBatch;
use parquet::arrow::ArrowWriter;
use parquet::basic::{Compression, ZstdLevel};
use parquet::file::properties::WriterProperties;
use radar_types::Slot;

use crate::decision::{Conclusion, Decision, KernelOutcome};
use crate::error::StoreError;
use crate::event::{Event, Table};
use crate::outcome::Outcome;
use crate::schema::schema_for;

/// How many slots go in one file.
///
/// At roughly 2.5 slots a second this is about ninety minutes of chain. Small
/// enough that a query for a narrow slot range reads few files, large enough that
/// a day is sixteen files rather than thousands — directory listings dominate on
/// the kind of modest disk this runs on.
pub const SLOTS_PER_PARTITION: u64 = 12_800;

/// The slot-range partition a slot belongs to.
#[must_use]
pub const fn partition_of(slot: Slot) -> u64 {
    slot.get() / SLOTS_PER_PARTITION
}

/// Buffers events and writes them as Parquet.
///
/// Append-only by construction: a partition file is written once, and a second
/// write to the same partition creates a new generation rather than replacing it.
/// Nothing in this crate deletes or rewrites. The recorder's value is that it
/// never loses anything, and a store that can overwrite is one bug away from
/// losing a day.
pub struct Writer {
    root: PathBuf,
    /// Buffered events, keyed by table and partition.
    pending: BTreeMap<(Table, u64), Vec<Event>>,
    /// Buffered outcome measurements, keyed by partition.
    ///
    /// Separate because an outcome is not a chain event: it has no signature and
    /// no transaction position, and putting it through the same buffer would
    /// mean inventing both.
    pending_outcomes: BTreeMap<u64, Vec<Outcome>>,
    /// Buffered decisions, by partition.
    ///
    /// Separate for the same reason outcomes are: a decision has no signature
    /// and no transaction position, because nothing happened on chain.
    pending_decisions: BTreeMap<u64, Vec<Decision>>,
    buffered: usize,
    flush_at: usize,
    written_rows: u64,
    written_files: u64,
    highest_slot: Option<Slot>,
}

impl Writer {
    /// Opens a store rooted at `root`, creating the directory tree if needed.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] if the directories cannot be created.
    pub fn open(root: impl Into<PathBuf>, flush_at: usize) -> Result<Self, StoreError> {
        let root = root.into();
        for t in Table::ALL {
            fs::create_dir_all(root.join(t.dir()))?;
        }
        Ok(Self {
            root,
            pending: BTreeMap::new(),
            pending_outcomes: BTreeMap::new(),
            pending_decisions: BTreeMap::new(),
            buffered: 0,
            flush_at: flush_at.max(1),
            written_rows: 0,
            written_files: 0,
            highest_slot: None,
        })
    }

    /// Buffers an event, flushing if the buffer is full.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] if a flush fails.
    pub fn append(&mut self, event: Event) -> Result<(), StoreError> {
        let slot = event.slot();
        self.highest_slot = Some(self.highest_slot.map_or(slot, |h| h.max(slot)));
        self.pending
            .entry((event.table(), partition_of(slot)))
            .or_default()
            .push(event);
        self.buffered += 1;
        if self.buffered >= self.flush_at {
            self.flush()?;
        }
        Ok(())
    }

    /// Buffers an outcome measurement.
    ///
    /// Partitioned by the slot it was *measured* at, not the launch slot: a
    /// measurement is a fact about the moment it was taken, and a replay asking
    /// what was known at slot N wants the measurements that existed by then.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] if a flush fails.
    pub fn append_outcome(&mut self, outcome: Outcome) -> Result<(), StoreError> {
        let slot = outcome.measured_at;
        self.highest_slot = Some(self.highest_slot.map_or(slot, |h| h.max(slot)));
        self.pending_outcomes
            .entry(partition_of(slot))
            .or_default()
            .push(outcome);
        self.buffered += 1;
        if self.buffered >= self.flush_at {
            self.flush()?;
        }
        Ok(())
    }

    /// Buffers a decision, flushing if the buffer is full.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] if a flush fails.
    pub fn append_decision(&mut self, decision: Decision) -> Result<(), StoreError> {
        let slot = decision.decided_at;
        self.highest_slot = Some(self.highest_slot.map_or(slot, |h| h.max(slot)));
        self.pending_decisions
            .entry(partition_of(slot))
            .or_default()
            .push(decision);
        self.buffered += 1;
        if self.buffered >= self.flush_at {
            self.flush()?;
        }
        Ok(())
    }

    /// Events buffered but not yet on disk.
    #[must_use]
    pub const fn buffered(&self) -> usize {
        self.buffered
    }

    /// Rows written to disk so far.
    #[must_use]
    pub const fn written_rows(&self) -> u64 {
        self.written_rows
    }

    /// Files written so far.
    #[must_use]
    pub const fn written_files(&self) -> u64 {
        self.written_files
    }

    /// The highest slot seen, buffered or written.
    #[must_use]
    pub const fn highest_slot(&self) -> Option<Slot> {
        self.highest_slot
    }

    /// Writes everything buffered to disk.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] if a file cannot be written or a batch cannot be
    /// built.
    pub fn flush(&mut self) -> Result<(), StoreError> {
        for (partition, outcomes) in std::mem::take(&mut self.pending_outcomes) {
            if outcomes.is_empty() {
                continue;
            }
            let rows = outcomes.len() as u64;
            let batch = build_outcome_batch(&outcomes)?;
            let path = self.next_path(Table::Outcomes, partition);
            write_parquet(&path, &batch)?;
            self.written_rows += rows;
            self.written_files += 1;
        }

        for (partition, decisions) in std::mem::take(&mut self.pending_decisions) {
            if decisions.is_empty() {
                continue;
            }
            let rows = decisions.len() as u64;
            let batch = build_decision_batch(&decisions)?;
            let path = self.next_path(Table::Decisions, partition);
            write_parquet(&path, &batch)?;
            self.written_rows += rows;
            self.written_files += 1;
        }

        let pending = std::mem::take(&mut self.pending);
        for ((table, partition), events) in pending {
            if events.is_empty() {
                continue;
            }
            let rows = events.len() as u64;
            let batch = build_batch(table, &events)?;
            let path = self.next_path(table, partition);
            write_parquet(&path, &batch)?;
            self.written_rows += rows;
            self.written_files += 1;
        }
        self.buffered = 0;
        Ok(())
    }

    /// A path that does not yet exist, so a flush never overwrites.
    fn next_path(&self, table: Table, partition: u64) -> PathBuf {
        let dir = self.root.join(table.dir());
        let start = partition * SLOTS_PER_PARTITION;
        for generation in 0u32.. {
            let name = format!("slot_{start:012}_g{generation:04}.parquet");
            let path = dir.join(name);
            if !path.exists() {
                return path;
            }
        }
        unreachable!("u32 generations exhausted for one partition")
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        // Losing buffered events on shutdown would leave a gap that looks exactly
        // like a quiet market. Best effort: a failure here cannot be reported, so
        // callers that care must call flush explicitly.
        let _ = self.flush();
    }
}

fn write_parquet(path: &Path, batch: &RecordBatch) -> Result<(), StoreError> {
    let props = WriterProperties::builder()
        // zstd(3) is roughly twice the ratio of snappy on this data at a write
        // cost that does not matter for a batch flush, and the store is expected
        // to outlive many rereads.
        .set_compression(Compression::ZSTD(ZstdLevel::try_new(3).unwrap_or_default()))
        .set_dictionary_enabled(true)
        .build();

    let file = File::create(path)?;
    let mut writer = ArrowWriter::try_new(file, batch.schema(), Some(props))?;
    writer.write(batch)?;
    writer.close()?;
    Ok(())
}

/// The item field of a reason list, matching the schema's non-null declaration.
fn reason_item() -> Arc<arrow::datatypes::Field> {
    Arc::new(arrow::datatypes::Field::new(
        "item",
        arrow::datatypes::DataType::Utf8,
        false,
    ))
}

/// Builds the Parquet batch for a run of decisions.
fn build_decision_batch(decisions: &[Decision]) -> Result<RecordBatch, StoreError> {
    let (mut mint, mut creator) = (StringBuilder::new(), StringBuilder::new());
    let (mut decided, mut launch) = (UInt64Builder::new(), UInt64Builder::new());
    let (mut strategy, mut version) = (StringBuilder::new(), StringBuilder::new());
    let mut conclusion = StringBuilder::new();
    // `with_field` rather than the default: a bare ListBuilder declares its
    // items nullable, and the schema says they are not. A reason is always a
    // string or absent from the list entirely -- there is no such thing as a
    // null reason -- so the schema is right and the builder has to be told.
    let mut reasons = ListBuilder::new(StringBuilder::new()).with_field(reason_item());
    let (mut notional, mut capacity) = (UInt64Builder::new(), UInt64Builder::new());
    let mut cost_bps = UInt64Builder::new();
    let (mut coordination, mut kernel) = (StringBuilder::new(), StringBuilder::new());
    let mut kernel_reasons = ListBuilder::new(StringBuilder::new()).with_field(reason_item());
    let mut entry_price = UInt64Builder::new();
    let mut digest = StringBuilder::new();

    for d in decisions {
        mint.append_value(d.mint.to_string());
        creator.append_value(d.creator.to_string());
        decided.append_value(d.decided_at.get());
        launch.append_value(d.launch_slot.get());
        strategy.append_value(&d.strategy);
        version.append_value(&d.strategy_version);
        conclusion.append_value(match d.conclusion {
            Conclusion::Passed => "passed",
            Conclusion::Proposed => "proposed",
        });
        for r in &d.reasons {
            reasons.values().append_value(r);
        }
        reasons.append(true);
        notional.append_option(d.notional_micro_usd);
        capacity.append_option(d.exit_capacity_micro_usd);
        cost_bps.append_value(d.assumed_round_trip_bps);
        coordination.append_option(d.coordination.as_deref());
        kernel.append_option(d.kernel_outcome.map(|k| match k {
            KernelOutcome::Authorised => "authorised",
            KernelOutcome::Refused => "refused",
        }));
        for r in &d.kernel_reasons {
            kernel_reasons.values().append_value(r);
        }
        kernel_reasons.append(true);
        entry_price.append_option(d.entry_price);
        digest.append_value(&d.inputs_digest);
    }

    RecordBatch::try_new(
        schema_for(Table::Decisions),
        vec![
            Arc::new(mint.finish()) as ArrayRef,
            Arc::new(creator.finish()),
            Arc::new(decided.finish()),
            Arc::new(launch.finish()),
            Arc::new(strategy.finish()),
            Arc::new(version.finish()),
            Arc::new(conclusion.finish()),
            Arc::new(reasons.finish()),
            Arc::new(notional.finish()),
            Arc::new(capacity.finish()),
            Arc::new(cost_bps.finish()),
            Arc::new(coordination.finish()),
            Arc::new(kernel.finish()),
            Arc::new(kernel_reasons.finish()),
            Arc::new(entry_price.finish()),
            Arc::new(digest.finish()),
        ],
    )
    .map_err(StoreError::from)
}

/// Column builders shared by every table.
struct EnvelopeCols {
    slot: UInt64Builder,
    signature: StringBuilder,
    tx_index: UInt32Builder,
    instruction_index: UInt32Builder,
    parent_index: UInt32Builder,
    succeeded: BooleanBuilder,
    program: StringBuilder,
    instruction: StringBuilder,
    known: BooleanBuilder,
}

impl EnvelopeCols {
    fn new() -> Self {
        Self {
            slot: UInt64Builder::new(),
            signature: StringBuilder::new(),
            tx_index: UInt32Builder::new(),
            instruction_index: UInt32Builder::new(),
            parent_index: UInt32Builder::new(),
            succeeded: BooleanBuilder::new(),
            program: StringBuilder::new(),
            instruction: StringBuilder::new(),
            known: BooleanBuilder::new(),
        }
    }

    fn push(&mut self, e: &Event) {
        let env = e.envelope();
        self.slot.append_value(env.slot.get());
        self.signature.append_value(env.signature.to_string());
        self.tx_index.append_value(env.tx_index);
        self.instruction_index.append_value(env.instruction_index);
        self.parent_index.append_option(env.parent_index);
        self.succeeded.append_value(env.succeeded);
        let origin = match e {
            Event::Launch(l) => &l.origin,
            Event::Trade(t) => &t.origin,
            Event::Graduation(g) => &g.origin,
        };
        self.program.append_value(origin.program.to_string());
        self.instruction.append_value(&origin.instruction);
        self.known.append_value(origin.known);
    }

    fn finish(mut self) -> Vec<ArrayRef> {
        vec![
            Arc::new(self.slot.finish()),
            Arc::new(self.signature.finish()),
            Arc::new(self.tx_index.finish()),
            Arc::new(self.instruction_index.finish()),
            Arc::new(self.parent_index.finish()),
            Arc::new(self.succeeded.finish()),
            Arc::new(self.program.finish()),
            Arc::new(self.instruction.finish()),
            Arc::new(self.known.finish()),
        ]
    }
}

fn build_outcome_batch(outcomes: &[Outcome]) -> Result<RecordBatch, StoreError> {
    let mut mint = StringBuilder::new();
    let (mut measured, mut launch) = (UInt64Builder::new(), UInt64Builder::new());
    let (mut first, mut last) = (UInt64Builder::new(), UInt64Builder::new());
    let mut transfers = UInt64Builder::new();
    let (mut senders, mut receivers) = (UInt64Builder::new(), UInt64Builder::new());
    let mut graduated_at = UInt64Builder::new();
    let (mut first_price, mut last_price) = (UInt64Builder::new(), UInt64Builder::new());
    let (mut peak_price, mut trough_price) = (UInt64Builder::new(), UInt64Builder::new());
    let (mut vwap, mut fills) = (UInt64Builder::new(), UInt64Builder::new());

    for o in outcomes {
        mint.append_value(o.mint.to_string());
        measured.append_value(o.measured_at.get());
        launch.append_value(o.launch_slot.get());
        first.append_option(o.first_transfer_slot.map(radar_types::Slot::get));
        last.append_option(o.last_transfer_slot.map(radar_types::Slot::get));
        transfers.append_value(o.transfers);
        senders.append_value(o.unique_senders);
        receivers.append_value(o.unique_receivers);
        graduated_at.append_option(o.graduated_at.map(radar_types::Slot::get));
        first_price.append_option(o.first_price);
        last_price.append_option(o.last_price);
        peak_price.append_option(o.peak_price);
        trough_price.append_option(o.trough_price);
        vwap.append_option(o.vwap);
        fills.append_value(o.fills);
    }

    RecordBatch::try_new(
        schema_for(Table::Outcomes),
        vec![
            Arc::new(mint.finish()) as ArrayRef,
            Arc::new(measured.finish()),
            Arc::new(launch.finish()),
            Arc::new(first.finish()),
            Arc::new(last.finish()),
            Arc::new(transfers.finish()),
            Arc::new(senders.finish()),
            Arc::new(receivers.finish()),
            Arc::new(graduated_at.finish()),
            Arc::new(first_price.finish()),
            Arc::new(last_price.finish()),
            Arc::new(peak_price.finish()),
            Arc::new(trough_price.finish()),
            Arc::new(vwap.finish()),
            Arc::new(fills.finish()),
        ],
    )
    .map_err(StoreError::from)
}

fn build_batch(table: Table, events: &[Event]) -> Result<RecordBatch, StoreError> {
    let mut env = EnvelopeCols::new();
    for e in events {
        debug_assert_eq!(e.table(), table, "event routed to the wrong table");
        env.push(e);
    }
    let mut cols = env.finish();

    match table {
        Table::Launches => {
            let (mut mint, mut creator) = (StringBuilder::new(), StringBuilder::new());
            let (mut name, mut symbol, mut uri) = (
                StringBuilder::new(),
                StringBuilder::new(),
                StringBuilder::new(),
            );
            let mut dev_buy = UInt64Builder::new();
            for e in events {
                let Event::Launch(l) = e else { continue };
                mint.append_value(l.mint.to_string());
                creator.append_value(l.creator.to_string());
                name.append_value(&l.name);
                symbol.append_value(&l.symbol);
                uri.append_value(&l.uri);
                dev_buy.append_option(l.dev_buy_lamports);
            }
            cols.extend([
                Arc::new(mint.finish()) as ArrayRef,
                Arc::new(creator.finish()),
                Arc::new(name.finish()),
                Arc::new(symbol.finish()),
                Arc::new(uri.finish()),
                Arc::new(dev_buy.finish()),
            ]);
        }
        Table::Trades => {
            let (mut mint, mut trader, mut side) = (
                StringBuilder::new(),
                StringBuilder::new(),
                StringBuilder::new(),
            );
            let (mut rl, mut rt) = (UInt64Builder::new(), UInt64Builder::new());
            let mut req = UInt64Builder::new();
            let mut req_lam = BooleanBuilder::new();
            let mut limit = UInt64Builder::new();
            let mut any_price = BooleanBuilder::new();
            for e in events {
                let Event::Trade(t) = e else { continue };
                mint.append_value(t.mint.to_string());
                trader.append_value(t.trader.to_string());
                side.append_value(t.side.as_str());
                rl.append_option(t.realised_lamports);
                rt.append_option(t.realised_tokens);
                req.append_value(t.requested_amount);
                req_lam.append_value(t.requested_is_lamports);
                limit.append_value(t.limit_amount);
                any_price.append_value(t.accepted_any_price);
            }
            cols.extend([
                Arc::new(mint.finish()) as ArrayRef,
                Arc::new(trader.finish()),
                Arc::new(side.finish()),
                Arc::new(rl.finish()),
                Arc::new(rt.finish()),
                Arc::new(req.finish()),
                Arc::new(req_lam.finish()),
                Arc::new(limit.finish()),
                Arc::new(any_price.finish()),
            ]);
        }
        Table::Graduations => {
            let mut mint = StringBuilder::new();
            for e in events {
                let Event::Graduation(g) = e else { continue };
                mint.append_value(g.mint.to_string());
            }
            cols.push(Arc::new(mint.finish()));
        }
        Table::Outcomes | Table::Decisions => {
            unreachable!("not events; see build_outcome_batch / build_decision_batch")
        }
    }

    RecordBatch::try_new(schema_for(table), cols).map_err(StoreError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partitions_are_contiguous_slot_ranges() {
        assert_eq!(partition_of(Slot(0)), 0);
        assert_eq!(partition_of(Slot(SLOTS_PER_PARTITION - 1)), 0);
        assert_eq!(partition_of(Slot(SLOTS_PER_PARTITION)), 1);
        assert_eq!(partition_of(Slot(SLOTS_PER_PARTITION * 3 + 5)), 3);
    }
}
