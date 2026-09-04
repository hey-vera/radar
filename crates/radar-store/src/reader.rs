// SPDX-License-Identifier: Apache-2.0
//! Reading the store back, and the watermark that gates it.

use std::fs;
use std::path::{Path, PathBuf};

use arrow::array::{Array, BooleanArray, Int64Array, StringArray, UInt32Array, UInt64Array};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use radar_asof::{AsOf, PointInTime};
use radar_types::{Address, Signature, Slot};

use crate::decision::{Conclusion, Decision, KernelOutcome};
use crate::error::StoreError;
use crate::event::{Envelope, Event, Graduation, Launch, Origin, Side, Table, Trade};
use crate::outcome::Outcome;
use crate::writer::SLOTS_PER_PARTITION;

/// Reads events written by [`crate::Writer`].
pub struct Reader {
    root: PathBuf,
}

impl Reader {
    /// Opens a store for reading.
    #[must_use]
    pub fn open(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Every partition file for a table, in slot order.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] if the directory cannot be listed.
    pub fn files(&self, table: Table) -> Result<Vec<PathBuf>, StoreError> {
        let dir = self.root.join(table.dir());
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut out: Vec<PathBuf> = fs::read_dir(&dir)?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "parquet"))
            .collect();
        // Names embed a zero-padded start slot, so lexical order is slot order.
        out.sort();
        Ok(out)
    }

    /// The highest slot the store holds.
    ///
    /// `None` for an empty store, which is different from a store whose highest
    /// slot is zero: the first means "cannot answer anything", the second means
    /// "answered up to genesis".
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] if the store cannot be read.
    pub fn watermark(&self) -> Result<Option<Slot>, StoreError> {
        let mut highest: Option<Slot> = None;
        for table in Table::ALL {
            // Not every table calls it `slot`: a measurement or a decision is
            // stamped with when it was *taken*, not when it happened.
            let column = table.slot_column();
            for path in extremal_partition(&self.files(*table)?, Extreme::Newest) {
                for slot in slots_in(&path, column)? {
                    highest = Some(highest.map_or(slot, |h| h.max(slot)));
                }
            }
        }
        Ok(highest)
    }

    /// The lowest slot the store holds.
    ///
    /// The other end of [`watermark`](Self::watermark), and it answers a
    /// different question: how much chain is behind the data, which is what
    /// decides whether a study has room to split the record at all.
    ///
    /// `None` for an empty store, for the same reason as `watermark`.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] if the store cannot be read.
    pub fn earliest(&self) -> Result<Option<Slot>, StoreError> {
        let mut lowest: Option<Slot> = None;
        for table in Table::ALL {
            let column = table.slot_column();
            for path in extremal_partition(&self.files(*table)?, Extreme::Oldest) {
                for slot in slots_in(&path, column)? {
                    lowest = Some(lowest.map_or(slot, |l: Slot| l.min(slot)));
                }
            }
        }
        Ok(lowest)
    }

    /// Reads every event in a table whose slot is at or before `as_of`.
    ///
    /// The watermark is applied here rather than left to the caller. A reader
    /// that hands back rows past the watermark and trusts callers to filter is a
    /// reader that will eventually leak the future into a replay.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] if a file cannot be read or a row is malformed.
    pub fn read(&self, table: Table, as_of: AsOf) -> Result<Vec<Event>, StoreError> {
        // Fail with the reason rather than with "no `slot` column", which is
        // what the caller actually sees otherwise and which says nothing about
        // what to do instead.
        if !table.holds_events() {
            return Err(StoreError::NotAnEventTable { table: table.dir() });
        }
        let mut out = Vec::new();
        for path in self.files(table)? {
            // Files are slot-ranged, so whole files past the watermark are
            // skipped without being opened.
            if start_slot_of(&path).is_some_and(|start| start > as_of.slot().get()) {
                continue;
            }
            for event in read_file(&path, table)? {
                if as_of.admits(event.slot()) {
                    out.push(event);
                }
            }
        }
        out.sort_by_key(|e| {
            let env = e.envelope();
            (env.slot.get(), env.tx_index, env.instruction_index)
        });
        Ok(out)
    }

    /// How many rows a table holds at or before the watermark.
    ///
    /// Counting does not need the data. Parquet records the row count in each
    /// file's footer, so a file that lies **entirely** at or before the
    /// watermark can be counted without decoding a single value — and because
    /// the watermark is usually the store's own top, at most one partition
    /// straddles it and needs reading.
    ///
    /// Against the live store this is the difference between 5.5 seconds and
    /// milliseconds for the funnel, which reads this to say how many launches
    /// have been recorded. A count is the cheapest question anyone asks and it
    /// was the most expensive one to answer.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] if a file cannot be read.
    pub fn count(&self, table: Table, as_of: AsOf) -> Result<usize, StoreError> {
        let top = as_of.slot().get();
        let mut total = 0usize;
        for path in self.files(table)? {
            let Some(start) = start_slot_of(&path) else {
                // An unparseable name: fall back to reading, so a naming change
                // makes this slow rather than wrong.
                total += admitted_rows(&path, table, as_of)?;
                continue;
            };
            if start > top {
                // Wholly after the watermark. Nothing in it is admissible.
                continue;
            }
            // A partition covers `[start, start + SLOTS_PER_PARTITION)`, so this
            // is the highest slot it could possibly contain.
            let last_possible = start.saturating_add(SLOTS_PER_PARTITION - 1);
            if last_possible <= top {
                total += rows_in(&path)?;
            } else {
                total += admitted_rows(&path, table, as_of)?;
            }
        }
        Ok(total)
    }

    /// Reads outcome measurements taken at or before `as_of`.
    ///
    /// A mint can appear more than once: each row is a measurement at a
    /// different slot, and a later one does not replace an earlier one. Callers
    /// wanting "the latest as of N" take the last by `measured_at`.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] if a file cannot be read or a row is malformed.
    pub fn read_outcomes(&self, as_of: AsOf) -> Result<Vec<Outcome>, StoreError> {
        let mut out = Vec::new();
        for path in self.files(Table::Outcomes)? {
            if start_slot_of(&path).is_some_and(|start| start > as_of.slot().get()) {
                continue;
            }
            let file = fs::File::open(&path)?;
            let reader = ParquetRecordBatchReaderBuilder::try_new(file)?.build()?;
            for batch in reader {
                let batch = batch?;
                let mint = str_col(&batch, "mint")?;
                let measured = u64_col(&batch, "measured_at")?;
                let launch = u64_col(&batch, "launch_slot")?;
                let first = u64_col(&batch, "first_transfer_slot")?;
                let last = u64_col(&batch, "last_transfer_slot")?;
                let transfers = u64_col(&batch, "transfers")?;
                let senders = u64_col(&batch, "unique_senders")?;
                let receivers = u64_col(&batch, "unique_receivers")?;
                let graduated_at = u64_col(&batch, "graduated_at")?;
                // Optional by column, not just by row. A file written before
                // prices existed simply has no such column, and that must read
                // as "not measured" rather than fail the whole read -- the live
                // store held 29 outcome files, 142,826 measurements, when these
                // were added.
                let first_price = optional_u64_col(&batch, "first_price");
                let last_price = optional_u64_col(&batch, "last_price");
                let peak_price = optional_u64_col(&batch, "peak_price");
                let trough_price = optional_u64_col(&batch, "trough_price");
                // Optional by column, not just by row: these were added on
                // 2026-08-31 and no earlier file has them. The erroring
                // accessor here would make the whole recorded history
                // unreadable, which is LEARNINGS 17 exactly.
                let window_peak = optional_u64_col(&batch, "window_peak_price");
                let window_trough = optional_u64_col(&batch, "window_trough_price");
                let vwap = optional_u64_col(&batch, "vwap");
                let fills = optional_u64_col(&batch, "fills");

                for i in 0..batch.num_rows() {
                    let measured_at = Slot(measured.value(i));
                    if !as_of.admits(measured_at) {
                        continue;
                    }
                    out.push(Outcome {
                        mint: parse(mint.value(i), "mint")?,
                        measured_at,
                        launch_slot: Slot(launch.value(i)),
                        first_transfer_slot: first.is_valid(i).then(|| Slot(first.value(i))),
                        last_transfer_slot: last.is_valid(i).then(|| Slot(last.value(i))),
                        transfers: transfers.value(i),
                        unique_senders: senders.value(i),
                        unique_receivers: receivers.value(i),
                        graduated_at: graduated_at
                            .is_valid(i)
                            .then(|| Slot(graduated_at.value(i))),
                        first_price: cell(first_price, i),
                        last_price: cell(last_price, i),
                        peak_price: cell(peak_price, i),
                        trough_price: cell(trough_price, i),
                        window_peak_price: cell(window_peak, i),
                        window_trough_price: cell(window_trough, i),
                        vwap: cell(vwap, i),
                        fills: cell(fills, i).unwrap_or(0),
                    });
                }
            }
        }
        out.sort_by_key(|o| (o.measured_at.get(), o.mint));
        Ok(out)
    }
    /// Every position row recorded at or before the watermark.
    ///
    /// Rows, not positions: opening writes one and closing writes another with
    /// the same `(mint, opened_at)`. Folding them is
    /// [`crate::fold_positions`]'s job, and it is separate so that the read
    /// stays a read — a reader that quietly resolved supersession would make
    /// "what did Radar hold on Tuesday" unanswerable without re-reading.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] if a file cannot be read.
    pub fn read_positions(&self, as_of: AsOf) -> Result<Vec<crate::Position>, StoreError> {
        let mut out = Vec::new();
        for path in self.files(Table::Positions)? {
            if start_slot_of(&path).is_some_and(|start| start > as_of.slot().get()) {
                continue;
            }
            let file = fs::File::open(&path)?;
            let reader = ParquetRecordBatchReaderBuilder::try_new(file)?.build()?;
            for batch in reader {
                let batch = batch?;
                let mint = str_col(&batch, "mint")?;
                let creator = str_col(&batch, "creator")?;
                let opened = u64_col(&batch, "opened_at")?;
                let notional = u64_col(&batch, "notional_micro_usd")?;
                let entry = optional_u64_col(&batch, "entry_price");
                let closed = u64_col(&batch, "closed_at")?;
                let exit = optional_u64_col(&batch, "exit_price");
                let realised = i64_col(&batch, "realised_micro_usd")?;

                for i in 0..batch.num_rows() {
                    let opened_at = Slot(opened.value(i));
                    // The watermark applies to the row, the same way it does in
                    // every other read here. A position opened after the
                    // watermark did not exist as of it.
                    if opened_at > as_of.slot() {
                        continue;
                    }
                    out.push(crate::Position {
                        mint: parse(mint.value(i), "mint")?,
                        creator: parse(creator.value(i), "creator")?,
                        opened_at,
                        notional_micro_usd: notional.value(i),
                        entry_price: cell(entry, i),
                        closed_at: closed.is_valid(i).then(|| Slot(closed.value(i))),
                        exit_price: cell(exit, i),
                        realised_micro_usd: realised.is_valid(i).then(|| realised.value(i)),
                    });
                }
            }
        }
        out.sort_by_key(|p| (p.opened_at, p.mint));
        Ok(out)
    }

    /// Every decision recorded at or before the watermark.
    ///
    /// Read separately from events for the same reason outcomes are: a decision
    /// has no signature and no transaction position, because nothing happened
    /// on chain. Iterating [`Table::ALL`] and calling [`read`](Self::read) on
    /// each would compile and fail here at runtime, which is why
    /// [`Table::EVENT_TABLES`] exists.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] if a file cannot be read or a row is malformed.
    pub fn read_decisions(&self, as_of: AsOf) -> Result<Vec<Decision>, StoreError> {
        let mut out = Vec::new();
        for path in self.files(Table::Decisions)? {
            if start_slot_of(&path).is_some_and(|start| start > as_of.slot().get()) {
                continue;
            }
            let file = fs::File::open(&path)?;
            let reader = ParquetRecordBatchReaderBuilder::try_new(file)?.build()?;
            for batch in reader {
                let batch = batch?;
                let mint = str_col(&batch, "mint")?;
                let creator = str_col(&batch, "creator")?;
                let decided = u64_col(&batch, "decided_at")?;
                let launch = u64_col(&batch, "launch_slot")?;
                let strategy = str_col(&batch, "strategy")?;
                let version = str_col(&batch, "strategy_version")?;
                let conclusion = str_col(&batch, "conclusion")?;
                let notional = u64_col(&batch, "notional_micro_usd")?;
                let capacity = u64_col(&batch, "exit_capacity_micro_usd")?;
                let cost_bps = u64_col(&batch, "assumed_round_trip_bps")?;
                let coordination = str_col(&batch, "coordination")?;
                // Optional, not erroring. This column was added on 2026-08-30
                // and every decision file written before then lacks it —
                // reading it with the erroring form would make the entire
                // recorded history unreadable, which is the failure
                // `optional_u64_col`'s doc comment describes and which this
                // very change would otherwise have caused in production.
                let prevalence = optional_str_col(&batch, "authority_prevalence");
                // Added 2026-09-04, so optional for the same reason the two
                // above are: every decision recorded before this change has no
                // such column, and those are the rows a re-derived threshold
                // would be fitted against.
                let recipients = optional_u32_col(&batch, "launch_recipients");
                let block_txs = optional_u32_col(&batch, "launch_transactions");
                let kernel = str_col(&batch, "kernel_outcome")?;
                let digest = str_col(&batch, "inputs_digest")?;
                // Optional by column, not just by row: a file written before
                // entry prices existed has no such column, and that must read
                // as "not measured" rather than fail the whole read.
                let entry_price = optional_u64_col(&batch, "entry_price");
                let reasons = string_list_col(&batch, "reasons")?;
                let kernel_reasons = string_list_col(&batch, "kernel_reasons")?;

                for i in 0..batch.num_rows() {
                    let decided_at = Slot(decided.value(i));
                    if !as_of.admits(decided_at) {
                        continue;
                    }
                    out.push(Decision {
                        mint: parse(mint.value(i), "mint")?,
                        creator: parse(creator.value(i), "creator")?,
                        decided_at,
                        launch_slot: Slot(launch.value(i)),
                        strategy: strategy.value(i).to_owned(),
                        strategy_version: version.value(i).to_owned(),
                        // An unrecognised conclusion reads as passed, never as
                        // proposed: this table outlives the code, and the
                        // direction that invents a trade is the expensive one.
                        conclusion: match conclusion.value(i) {
                            "proposed" => Conclusion::Proposed,
                            _ => Conclusion::Passed,
                        },
                        reasons: reasons.get(i).cloned().unwrap_or_default(),
                        notional_micro_usd: notional.is_valid(i).then(|| notional.value(i)),
                        exit_capacity_micro_usd: capacity.is_valid(i).then(|| capacity.value(i)),
                        assumed_round_trip_bps: cost_bps.value(i),
                        coordination: coordination
                            .is_valid(i)
                            .then(|| coordination.value(i).to_owned()),
                        launch_recipients: cell_u32(recipients, i),
                        launch_transactions: cell_u32(block_txs, i),
                        authority_prevalence: prevalence
                            .filter(|c| c.is_valid(i))
                            .map(|c| c.value(i).to_owned()),
                        // Same asymmetry: anything unrecognised is a refusal.
                        kernel_outcome: kernel.is_valid(i).then(|| match kernel.value(i) {
                            "authorised" => KernelOutcome::Authorised,
                            _ => KernelOutcome::Refused,
                        }),
                        kernel_reasons: kernel_reasons.get(i).cloned().unwrap_or_default(),
                        entry_price: cell(entry_price, i),
                        inputs_digest: digest.value(i).to_owned(),
                    });
                }
            }
        }
        out.sort_by_key(|d| (d.decided_at.get(), d.mint));
        Ok(out)
    }
}

impl PointInTime for Reader {
    type Error = StoreError;

    fn watermark(&self) -> Result<Slot, Self::Error> {
        Self::watermark(self)?.ok_or(StoreError::Empty)
    }
}

/// Reads a `List<Utf8>` column into one owned vector per row.
///
/// Materialised up front rather than per row because the offsets have to be
/// walked anyway, and a per-row accessor would walk them again for every row.
fn string_list_col(
    batch: &arrow::record_batch::RecordBatch,
    name: &'static str,
) -> Result<Vec<Vec<String>>, StoreError> {
    let Some(column) = batch.column_by_name(name) else {
        // Absent by column, not just by row -- a file written before this
        // column existed must read as "no reasons recorded" rather than fail
        // the whole read, which is how the price columns were added.
        return Ok(Vec::new());
    };
    let list = column
        .as_any()
        .downcast_ref::<arrow::array::ListArray>()
        .ok_or(StoreError::WrongColumnType { name })?;

    let mut out = Vec::with_capacity(list.len());
    for i in 0..list.len() {
        let values = list.value(i);
        let strings = values
            .as_any()
            .downcast_ref::<arrow::array::StringArray>()
            .ok_or(StoreError::WrongColumnType { name })?;
        out.push(
            (0..strings.len())
                .map(|j| strings.value(j).to_owned())
                .collect(),
        );
    }
    Ok(out)
}

/// Rows in one file that the watermark admits, by reading its slot column.
fn admitted_rows(path: &Path, table: Table, as_of: AsOf) -> Result<usize, StoreError> {
    Ok(slots_in(path, table.slot_column())?
        .into_iter()
        .filter(|slot| as_of.admits(*slot))
        .count())
}

/// The row count from a Parquet footer, without decoding any values.
fn rows_in(path: &Path) -> Result<usize, StoreError> {
    let file = fs::File::open(path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let rows = builder.metadata().file_metadata().num_rows();
    Ok(usize::try_from(rows).unwrap_or(0))
}

/// Which end of the store a caller wants.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Extreme {
    /// The partition holding the highest slots.
    Newest,
    /// The partition holding the lowest.
    Oldest,
}

/// The files that could possibly hold the extreme slot.
///
/// **A partition file covers a bounded slot range, and its filename names the
/// start of that range.** So the highest slot in a table is inside a file whose
/// partition start is the highest, and nothing in any earlier partition can beat
/// it. Reading the rest is work whose answer is known in advance.
///
/// Every generation of that partition is returned, not just one: a flush writes
/// beside an existing file rather than replacing it, so `g0000` and `g0001` of
/// the same partition both hold rows and either may carry the extreme.
///
/// This is the difference between opening five files and opening seven
/// thousand. Measured against the live store, `watermark` took **3.4 seconds**
/// walking 6,998 files, and the count grows by roughly 4,700 a day — a serving
/// surface whose latency is a function of how long the recorder has been
/// running is one that gets worse forever.
fn extremal_partition(files: &[PathBuf], which: Extreme) -> Vec<PathBuf> {
    let starts = files.iter().filter_map(|p| start_slot_of(p));
    let Some(target) = (match which {
        Extreme::Newest => starts.max(),
        Extreme::Oldest => starts.min(),
    }) else {
        // No filename parsed. Rather than conclude the store is empty, hand back
        // everything and let the caller read it -- a naming change should make
        // this slow, never wrong.
        return files.to_vec();
    };
    files
        .iter()
        .filter(|p| start_slot_of(p) == Some(target))
        .cloned()
        .collect()
}

/// Parses the start slot out of a partition filename.
fn start_slot_of(path: &Path) -> Option<u64> {
    let name = path.file_name()?.to_str()?;
    name.strip_prefix("slot_")?.split('_').next()?.parse().ok()
}

/// Every slot present in a file, read from the slot column alone.
fn slots_in(path: &Path, column: &'static str) -> Result<Vec<Slot>, StoreError> {
    let file = fs::File::open(path)?;
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)?.build()?;
    let mut out = Vec::new();
    for batch in reader {
        let batch = batch?;
        let col = u64_col(&batch, column)?;
        for i in 0..col.len() {
            out.push(Slot(col.value(i)));
        }
    }
    Ok(out)
}

fn read_file(path: &Path, table: Table) -> Result<Vec<Event>, StoreError> {
    let file = fs::File::open(path)?;
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)?.build()?;
    let mut out = Vec::new();

    for batch in reader {
        let batch = batch?;
        let slot = u64_col(&batch, "slot")?;
        let signature = str_col(&batch, "signature")?;
        let tx_index = u32_col(&batch, "tx_index")?;
        let ix_index = u32_col(&batch, "instruction_index")?;
        let parent = u32_col(&batch, "parent_index")?;
        let succeeded = bool_col(&batch, "succeeded")?;
        let program = str_col(&batch, "program")?;
        let instruction = str_col(&batch, "instruction")?;
        let known = bool_col(&batch, "known")?;
        let mint = str_col(&batch, "mint")?;

        for i in 0..batch.num_rows() {
            let envelope = Envelope {
                slot: Slot(slot.value(i)),
                signature: parse(signature.value(i), "signature")?,
                tx_index: tx_index.value(i),
                instruction_index: ix_index.value(i),
                parent_index: parent.is_valid(i).then(|| parent.value(i)),
                succeeded: succeeded.value(i),
            };
            let origin = Origin {
                program: parse(program.value(i), "program")?,
                instruction: instruction.value(i).to_owned(),
                known: known.value(i),
            };
            let mint = parse(mint.value(i), "mint")?;

            out.push(match table {
                Table::Launches => {
                    let dev = u64_col(&batch, "dev_buy_lamports")?;
                    Event::Launch(Box::new(Launch {
                        envelope,
                        origin,
                        mint,
                        creator: parse(str_col(&batch, "creator")?.value(i), "creator")?,
                        name: str_col(&batch, "name")?.value(i).to_owned(),
                        symbol: str_col(&batch, "symbol")?.value(i).to_owned(),
                        uri: str_col(&batch, "uri")?.value(i).to_owned(),
                        dev_buy_lamports: dev.is_valid(i).then(|| dev.value(i)),
                    }))
                }
                Table::Trades => {
                    let rl = u64_col(&batch, "realised_lamports")?;
                    let rt = u64_col(&batch, "realised_tokens")?;
                    let side = str_col(&batch, "side")?.value(i);
                    Event::Trade(Box::new(Trade {
                        envelope,
                        origin,
                        mint,
                        trader: parse(str_col(&batch, "trader")?.value(i), "trader")?,
                        side: match side {
                            "buy" => Side::Buy,
                            "sell" => Side::Sell,
                            other => {
                                return Err(StoreError::Malformed {
                                    field: "side",
                                    value: other.to_owned(),
                                });
                            }
                        },
                        realised_lamports: rl.is_valid(i).then(|| rl.value(i)),
                        realised_tokens: rt.is_valid(i).then(|| rt.value(i)),
                        requested_amount: u64_col(&batch, "requested_amount")?.value(i),
                        requested_is_lamports: bool_col(&batch, "requested_is_lamports")?.value(i),
                        limit_amount: u64_col(&batch, "limit_amount")?.value(i),
                        accepted_any_price: bool_col(&batch, "accepted_any_price")?.value(i),
                    }))
                }
                Table::Graduations => Event::Graduation(Box::new(Graduation {
                    envelope,
                    origin,
                    mint,
                })),
                Table::Outcomes | Table::Decisions | Table::Positions => {
                    unreachable!(
                        "not events; read by read_outcomes / read_decisions / read_positions"
                    )
                }
            });
        }
    }
    Ok(out)
}

fn parse<T: std::str::FromStr>(s: &str, field: &'static str) -> Result<T, StoreError> {
    s.parse().map_err(|_| StoreError::Malformed {
        field,
        value: s.to_owned(),
    })
}

macro_rules! typed_col {
    ($fn_name:ident, $arr:ty) => {
        fn $fn_name<'a>(
            batch: &'a arrow::record_batch::RecordBatch,
            name: &'static str,
        ) -> Result<&'a $arr, StoreError> {
            batch
                .column_by_name(name)
                .ok_or(StoreError::MissingColumn { name })?
                .as_any()
                .downcast_ref::<$arr>()
                .ok_or(StoreError::WrongColumnType { name })
        }
    };
}

typed_col!(u64_col, UInt64Array);

/// A string column if the file has one, or `None` if it does not.
///
/// The same distinction [`optional_u64_col`] draws, for the same reason: a
/// column added after some files were written is absent from them, and absent
/// must read as "not measured" rather than as a corrupt file.
fn optional_str_col<'a>(
    batch: &'a arrow::record_batch::RecordBatch,
    name: &str,
) -> Option<&'a StringArray> {
    batch
        .column_by_name(name)?
        .as_any()
        .downcast_ref::<StringArray>()
}

/// A `u64` column if the file has one, or `None` if it does not.
///
/// Distinct from [`u64_col`], which errors, and both are right for different
/// things: a column the writer has always emitted going missing is a corrupted
/// file and should fail loudly, while a column added later is simply absent from
/// older files and must read as "not measured". Using the erroring form for a new
/// column would make one schema change unreadable for every file written before
/// it.
fn optional_u64_col<'a>(
    batch: &'a arrow::record_batch::RecordBatch,
    name: &str,
) -> Option<&'a UInt64Array> {
    batch
        .column_by_name(name)?
        .as_any()
        .downcast_ref::<UInt64Array>()
}

/// One cell of an optional column, absent if either the column or the value is.
fn cell(column: Option<&UInt64Array>, i: usize) -> Option<u64> {
    let column = column?;
    column.is_valid(i).then(|| column.value(i))
}

/// The `u32` form of [`optional_u64_col`], for the launch-block counts.
///
/// Same reasoning, and it is worth restating rather than pointing at: these two
/// columns were added on 2026-09-04 and every decision file written before then
/// lacks them. Read with the erroring form, one schema change would make the
/// whole recorded history unreadable.
fn optional_u32_col<'a>(
    batch: &'a arrow::record_batch::RecordBatch,
    name: &str,
) -> Option<&'a UInt32Array> {
    batch
        .column_by_name(name)?
        .as_any()
        .downcast_ref::<UInt32Array>()
}

/// One cell of an optional `u32` column.
fn cell_u32(column: Option<&UInt32Array>, i: usize) -> Option<u32> {
    let column = column?;
    column.is_valid(i).then(|| column.value(i))
}
typed_col!(i64_col, Int64Array);
typed_col!(u32_col, UInt32Array);
typed_col!(bool_col, BooleanArray);
typed_col!(str_col, StringArray);

/// Reads the address parse implementations the reader needs.
impl Reader {
    /// The slot range a partition file covers, from its name alone.
    #[must_use]
    pub fn partition_range(path: &Path) -> Option<(Slot, Slot)> {
        let start = start_slot_of(path)?;
        Some((Slot(start), Slot(start + SLOTS_PER_PARTITION - 1)))
    }
}

// Address and Signature both implement FromStr, which `parse` above relies on.
const _: fn() = || {
    fn assert_from_str<T: std::str::FromStr>() {}
    assert_from_str::<Address>();
    assert_from_str::<Signature>();
};
