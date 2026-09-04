// SPDX-License-Identifier: Apache-2.0
//! Building the creator index the public analyst reads.
//!
//! The **writing** half. The type and the lookup live in
//! [`radar_roast::creator`], because the analyst reads it and must not depend on
//! the store to do so; this reads both tables in full, which is precisely the
//! operation the index exists to stop happening per mention.
//!
//! Run on a timer, the way the base rates are.

use radar_asof::AsOf;
use radar_roast::creator::{CreatorIndex, Record};
use radar_store::{Event, Outcome, Reader, StoreError, Table};
use radar_types::Address;

use std::collections::BTreeMap;

/// Builds the index from a store.
///
/// # Errors
///
/// [`StoreError`] when either table cannot be read.
pub fn build(reader: &Reader, as_of: AsOf, built_at: u64) -> Result<CreatorIndex, StoreError> {
    let launches = reader.read(Table::Launches, as_of)?;
    let outcomes = reader.read_outcomes(as_of)?;

    // Which creator launched which mint. A mint has exactly one launch, so
    // the first wins -- a duplicate would be the same launch recorded twice
    // and taking the later one would say the same thing.
    let mut creator_of: BTreeMap<Address, Address> = BTreeMap::new();
    let mut creators: BTreeMap<String, Record> = BTreeMap::new();
    for event in &launches {
        let Event::Launch(launch) = event else {
            continue;
        };
        // Failed launches are not launches. The recorder keeps them because
        // a spam burst is real information about the market, but a creator
        // credited with a thousand launches that never happened is a
        // creator ranked on somebody else's failed transactions.
        if !launch.envelope.succeeded {
            continue;
        }
        if creator_of.insert(launch.mint, launch.creator).is_none() {
            creators
                .entry(launch.creator.to_string())
                .or_default()
                .launches += 1;
        }
    }

    // The latest measurement at or before the watermark, per mint. A mint is
    // measured repeatedly as it ages, and what was known *then* is the last
    // one taken by then.
    let mut latest: BTreeMap<Address, &Outcome> = BTreeMap::new();
    for outcome in &outcomes {
        if !creator_of.contains_key(&outcome.mint) {
            continue;
        }
        latest
            .entry(outcome.mint)
            .and_modify(|held| {
                if outcome.measured_at > held.measured_at {
                    *held = outcome;
                }
            })
            .or_insert(outcome);
    }

    for (mint, outcome) in latest {
        let Some(creator) = creator_of.get(&mint) else {
            continue;
        };
        let record = creators.entry(creator.to_string()).or_default();
        record.measured += 1;
        if outcome.appears_stillborn() {
            record.stillborn += 1;
        }
        // Split, and by the store's own rule rather than a second copy of
        // it: `graduation_mode` is where `INSTANT_WITHIN_SLOTS` lives, and a
        // threshold restated here would drift from the one every other
        // consumer uses.
        match outcome.graduation_mode() {
            Some(radar_store::GraduationMode::Organic) => record.organic += 1,
            Some(radar_store::GraduationMode::Instant) => record.instant += 1,
            None => {}
        }
    }

    Ok(CreatorIndex {
        watermark_slot: as_of.slot().get(),
        built_at,
        creators,
    })
}
