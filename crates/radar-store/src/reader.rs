// SPDX-License-Identifier: Apache-2.0
//! Reading the store back, and the watermark that gates it.

use std::fs;
use std::path::{Path, PathBuf};

use arrow::array::{Array, BooleanArray, StringArray, UInt32Array, UInt64Array};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use radar_asof::{AsOf, PointInTime};
use radar_types::{Address, Signature, Slot};

use crate::error::StoreError;
use crate::event::{Envelope, Event, Graduation, Launch, Origin, Side, Table, Trade};
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
            for path in self.files(*table)? {
                for slot in slots_in(&path)? {
                    highest = Some(highest.map_or(slot, |h| h.max(slot)));
                }
            }
        }
        Ok(highest)
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
}

impl PointInTime for Reader {
    type Error = StoreError;

    fn watermark(&self) -> Result<Slot, Self::Error> {
        Self::watermark(self)?.ok_or(StoreError::Empty)
    }
}

/// Parses the start slot out of a partition filename.
fn start_slot_of(path: &Path) -> Option<u64> {
    let name = path.file_name()?.to_str()?;
    name.strip_prefix("slot_")?.split('_').next()?.parse().ok()
}

/// Every slot present in a file, read from the slot column alone.
fn slots_in(path: &Path) -> Result<Vec<Slot>, StoreError> {
    let file = fs::File::open(path)?;
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)?.build()?;
    let mut out = Vec::new();
    for batch in reader {
        let batch = batch?;
        let col = u64_col(&batch, "slot")?;
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
