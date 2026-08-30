// SPDX-License-Identifier: Apache-2.0
//! Arrow schemas for the stored tables.
//!
//! Addresses and signatures are stored as base58 strings rather than fixed-width
//! bytes. That costs space, and buys something worth more: a DuckDB query against
//! these files returns values a human can paste into an explorer, without a
//! decoding step that would have to be reimplemented in every ad-hoc query. The
//! files are dictionary-encoded and compressed, so repeated mints and programs —
//! which is most of the volume — cost far less than the raw string length
//! suggests.
//!
//! Nullability is meaningful throughout: a null realised amount means "not
//! recoverable", never zero.

use std::sync::Arc;

use arrow::datatypes::{DataType, Field, Schema};

use crate::event::Table;

/// Fields every table carries, describing where the event sits in the chain.
fn envelope_fields() -> Vec<Field> {
    vec![
        Field::new("slot", DataType::UInt64, false),
        Field::new("signature", DataType::Utf8, false),
        Field::new("tx_index", DataType::UInt32, false),
        Field::new("instruction_index", DataType::UInt32, false),
        // Null means top-level rather than a cross-program invocation.
        Field::new("parent_index", DataType::UInt32, true),
        Field::new("succeeded", DataType::Boolean, false),
        Field::new("program", DataType::Utf8, false),
        Field::new("instruction", DataType::Utf8, false),
        Field::new("known", DataType::Boolean, false),
    ]
}

/// The Arrow schema for a table.
#[must_use]
pub fn schema_for(table: Table) -> Arc<Schema> {
    // Two of the tables are not chain events: no signature, no transaction
    // position, and their own slot column. Forcing them through the envelope
    // would mean inventing a signature.
    if table.holds_events() {
        event_schema(table)
    } else {
        recorded_schema(table)
    }
}

/// The schema for a table of chain events.
fn event_schema(table: Table) -> Arc<Schema> {
    let mut fields = envelope_fields();
    match table {
        Table::Launches => fields.extend([
            Field::new("mint", DataType::Utf8, false),
            Field::new("creator", DataType::Utf8, false),
            Field::new("name", DataType::Utf8, false),
            Field::new("symbol", DataType::Utf8, false),
            Field::new("uri", DataType::Utf8, false),
            // Null means not recoverable; zero would mean the creator bought none.
            Field::new("dev_buy_lamports", DataType::UInt64, true),
        ]),
        Table::Trades => fields.extend([
            Field::new("mint", DataType::Utf8, false),
            Field::new("trader", DataType::Utf8, false),
            Field::new("side", DataType::Utf8, false),
            Field::new("realised_lamports", DataType::UInt64, true),
            Field::new("realised_tokens", DataType::UInt64, true),
            Field::new("requested_amount", DataType::UInt64, false),
            Field::new("requested_is_lamports", DataType::Boolean, false),
            Field::new("limit_amount", DataType::UInt64, false),
            Field::new("accepted_any_price", DataType::Boolean, false),
        ]),
        Table::Graduations => fields.push(Field::new("mint", DataType::Utf8, false)),
        Table::Outcomes | Table::Decisions => {
            unreachable!("not chain events; handled by recorded_schema")
        }
    }
    Arc::new(Schema::new(fields))
}

/// A list of non-null strings, for the two reason columns.
fn string_list(name: &str) -> Field {
    Field::new(
        name,
        DataType::List(Arc::new(Field::new("item", DataType::Utf8, false))),
        false,
    )
}

/// The schema for a table recording something Radar measured or decided.
fn recorded_schema(table: Table) -> Arc<Schema> {
    match table {
        Table::Outcomes => Arc::new(Schema::new(vec![
            Field::new("mint", DataType::Utf8, false),
            Field::new("measured_at", DataType::UInt64, false),
            Field::new("launch_slot", DataType::UInt64, false),
            Field::new("first_transfer_slot", DataType::UInt64, true),
            Field::new("last_transfer_slot", DataType::UInt64, true),
            Field::new("transfers", DataType::UInt64, false),
            Field::new("unique_senders", DataType::UInt64, false),
            Field::new("unique_receivers", DataType::UInt64, false),
            // The slot, not a flag. Null means no graduation recorded; a value
            // lets "same block as launch" and "three days later" stay different
            // outcomes instead of collapsing into one boolean.
            Field::new("graduated_at", DataType::UInt64, true),
            // Prices: lamports per base unit scaled by `PRICE_SCALE`. All
            // nullable, because a token that never traded has no price and a
            // measurement taken before prices were recorded has none either —
            // and neither is a price of zero.
            Field::new("first_price", DataType::UInt64, true),
            Field::new("last_price", DataType::UInt64, true),
            Field::new("peak_price", DataType::UInt64, true),
            Field::new("trough_price", DataType::UInt64, true),
            Field::new("vwap", DataType::UInt64, true),
            // Not nullable: zero fills is a real, informative measurement.
            Field::new("fills", DataType::UInt64, false),
        ])),
        Table::Decisions => Arc::new(Schema::new(vec![
            Field::new("mint", DataType::Utf8, false),
            Field::new("creator", DataType::Utf8, false),
            // The watermark the decision was taken as of, not the slot anything
            // happened at. Conflating those is how look-ahead gets in.
            Field::new("decided_at", DataType::UInt64, false),
            Field::new("launch_slot", DataType::UInt64, false),
            Field::new("strategy", DataType::Utf8, false),
            Field::new("strategy_version", DataType::Utf8, false),
            Field::new("conclusion", DataType::Utf8, false),
            // Lists rather than joined strings: joining assumes no reason ever
            // contains the separator, which is true today and is the kind of
            // assumption that stops being true silently.
            string_list("reasons"),
            // Null means nothing was sized, never that zero was.
            Field::new("notional_micro_usd", DataType::UInt64, true),
            Field::new("exit_capacity_micro_usd", DataType::UInt64, true),
            Field::new("assumed_round_trip_bps", DataType::UInt64, false),
            // Null means the launch block could not be read — never that it
            // looked clean, which is the distinction the whole gate rests on.
            Field::new("coordination", DataType::Utf8, true),
            // Null means the prevalence table could not be read, including the
            // case where it was truncated — a table that cannot be trusted, not
            // one that found nothing. Never that the wallets looked ordinary.
            Field::new("authority_prevalence", DataType::Utf8, true),
            // Null means the kernel never saw a proposal, which is a gap in the
            // pipeline rather than a refusal.
            Field::new("kernel_outcome", DataType::Utf8, true),
            string_list("kernel_reasons"),
            // What one base unit was worth when the decision was taken, scaled
            // by PRICE_SCALE. Null when no exit was probed. This is the field a
            // return is measured from: the outcome table's first_price is the
            // token's first fill ever, which is not where Radar entered.
            Field::new("entry_price", DataType::UInt64, true),
            Field::new("inputs_digest", DataType::Utf8, false),
        ])),
        Table::Launches | Table::Trades | Table::Graduations => {
            unreachable!("chain events; handled by event_schema")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_table_has_a_schema_starting_with_the_envelope() {
        for t in Table::ALL.iter().filter(|t| t.holds_events()) {
            let s = schema_for(*t);
            let names: Vec<&str> = s
                .fields()
                .iter()
                .take(9)
                .map(|f| f.name().as_str())
                .collect();
            assert_eq!(names[0], "slot", "{t:?}");
            assert_eq!(names[1], "signature", "{t:?}");
            assert!(
                names.contains(&"known"),
                "{t:?} must carry the decoder flag"
            );
        }
    }

    #[test]
    fn slot_is_never_nullable_because_it_is_the_point_in_time_key() {
        // A row that cannot say when it was true cannot be admitted through a
        // watermark, so it must never be storable in the first place. Outcomes
        // carry `measured_at` instead, for the same reason.
        for t in Table::ALL {
            let schema = schema_for(*t);
            let column = t.slot_column();
            assert!(
                !schema.field_with_name(column).expect(column).is_nullable(),
                "{t:?}"
            );
        }
    }

    #[test]
    fn amounts_that_may_be_unrecoverable_are_nullable() {
        let s = schema_for(Table::Trades);
        for col in ["realised_lamports", "realised_tokens"] {
            assert!(s.field_with_name(col).expect(col).is_nullable(), "{col}");
        }
        // What the trader asked for is always known, because it is in the
        // instruction we decoded.
        assert!(
            !s.field_with_name("requested_amount")
                .expect("col")
                .is_nullable()
        );
    }

    #[test]
    fn every_table_carries_a_mint() {
        // An event with no token is not useful to anything downstream.
        for t in Table::ALL {
            assert!(schema_for(*t).field_with_name("mint").is_ok(), "{t:?}");
        }
    }
}
