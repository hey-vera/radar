// SPDX-License-Identifier: Apache-2.0
//! What can go wrong reading or writing the store.

/// A store operation failed.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The filesystem said no.
    #[error("store io: {0}")]
    Io(#[from] std::io::Error),
    /// Parquet could not be read or written.
    #[error("parquet: {0}")]
    Parquet(#[from] parquet::errors::ParquetError),
    /// A record batch could not be assembled or read.
    #[error("arrow: {0}")]
    Arrow(#[from] arrow::error::ArrowError),
    /// A stored column is missing.
    #[error("stored file has no `{name}` column")]
    MissingColumn {
        /// The column expected.
        name: &'static str,
    },
    /// A stored column has an unexpected type.
    #[error("stored `{name}` column has an unexpected type")]
    WrongColumnType {
        /// The column.
        name: &'static str,
    },
    /// A stored value could not be parsed back.
    ///
    /// Surfaced rather than skipped: a row that cannot be read is a gap, and a
    /// gap that looks like a quiet market is the failure this store exists to
    /// prevent.
    #[error("stored `{field}` is not readable: {value}")]
    Malformed {
        /// The field.
        field: &'static str,
        /// What was stored.
        value: String,
    },
    /// A measurement table was read as if it held chain events.
    #[error("`{table}` holds measurements, not events; read it with read_outcomes")]
    NotAnEventTable {
        /// The table asked for.
        table: &'static str,
    },
    /// The store holds nothing, so it cannot answer as of any slot.
    #[error("store is empty and cannot answer as of any slot")]
    Empty,
}
