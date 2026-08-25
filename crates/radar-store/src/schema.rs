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
    // Outcomes are measurements rather than chain events: no signature, no
    // transaction position, and `measured_at` in place of the envelope's slot.
    // Forcing them through the envelope would mean inventing a signature.
    if table == Table::Outcomes {
        return Arc::new(Schema::new(vec![
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
        ]));
    }

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
        Table::Outcomes => unreachable!("handled above"),
    }
    Arc::new(Schema::new(fields))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_table_has_a_schema_starting_with_the_envelope() {
        for t in Table::ALL.iter().filter(|t| **t != Table::Outcomes) {
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
            let column = if *t == Table::Outcomes {
                "measured_at"
            } else {
                "slot"
            };
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
