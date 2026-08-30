// SPDX-License-Identifier: Apache-2.0
//! The append-only event log.
//!
//! Radar's first deliverable is a recorder with a point-in-time guarantee,
//! because every signal has to be validated against outcomes and the dataset
//! only accumulates forward. This crate is that recorder's disk.
//!
//! Three properties are load-bearing:
//!
//! **Append-only.** A partition file is written once. A second write to the same
//! slot range creates a new generation beside it rather than replacing it, and
//! nothing here deletes or rewrites. A store that can overwrite is one bug away
//! from losing a day that cannot be bought back.
//!
//! **Slot-partitioned.** Files are named by the slot range they cover, so a query
//! for a narrow range skips whole files without opening them, and the
//! point-in-time reader can discard a file from its name alone.
//!
//! **One schema for history and live.** The backfill from CryptoHouse and the
//! live recorder write the same rows through the same decoder. That is what makes
//! the replay test meaningful: it checks one pipeline rather than comparing two.
//!
//! Files are Parquet, so DuckDB can query the store directly with no exporter and
//! no service running.

#![forbid(unsafe_code)]

pub mod cursor;
mod decision;
mod error;
pub mod event;
mod outcome;
mod position;
mod reader;
mod schema;
mod writer;

pub use cursor::{CURSOR_FILE, from_epoch, now_epoch, read_cursor, to_epoch, write_cursor};
pub use decision::{Conclusion, Decision, KernelOutcome};
pub use error::StoreError;
pub use event::{Envelope, Event, Graduation, Launch, Origin, Side, Table, Trade};
pub use outcome::{GraduationMode, INSTANT_WITHIN_SLOTS, Outcome, PRICE_SCALE};
pub use position::{Position, fold_positions};
pub use reader::Reader;
pub use schema::schema_for;
pub use writer::{SLOTS_PER_PARTITION, Writer, partition_of};
