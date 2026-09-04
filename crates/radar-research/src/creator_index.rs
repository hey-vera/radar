// SPDX-License-Identifier: Apache-2.0
//! Building the creator index the public analyst reads.
//!
//! The **writing** half. The type and the lookup live in
//! [`radar_roast::creator`], because the analyst reads it and must not depend on
//! the store to do so; this reads both tables in full, which is precisely the
//! operation the index exists to stop happening per mention.
//!
//! Run on a timer, the way the base rates are.
//!
//! # It measures the population in the same pass
//!
//! `docs/research/data/0024-base-rates.json` carries two figures of this shape —
//! the share of launches that graduate at all, and the share that graduate
//! instantly — and both came from **outside**: a public RPC walking 45 slots
//! (`scripts/probe/probe_base_rates.py`) and a SQL endpoint that truncates
//! silently at a thousand rows (`docs/research/queries/0024-…​.sql`). They are
//! samples of a window, and the note says plainly that its predecessor was wrong
//! by 2.7× nine days later.
//!
//! This pass has already visited every succeeded launch and every outcome at the
//! watermark, so counting them is five additions per row, and the result is the
//! **population** rather than a sample of it. On those two figures the store is
//! the better instrument, so it emits them, and the snapshot keeps the recipient
//! distribution — which needs the launch block, and which the store did not
//! record until ADR 0012.

use radar_asof::AsOf;
use radar_roast::creator::{CreatorIndex, Population, Record};
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
    // Accumulated in the same pass rather than summed from `creators`
    // afterwards. Summing would be the same arithmetic done twice and would
    // silently agree with a per-creator bug; counting here and asserting the two
    // agree is what `the_population_is_the_sum_of_its_parts` checks.
    let mut population = Population::default();
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
            population.launches += 1;
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
        population.measured += 1;
        if outcome.appears_stillborn() {
            record.stillborn += 1;
            population.stillborn += 1;
        }
        // Split, and by the store's own rule rather than a second copy of
        // it: `graduation_mode` is where `INSTANT_WITHIN_SLOTS` lives, and a
        // threshold restated here would drift from the one every other
        // consumer uses.
        match outcome.graduation_mode() {
            Some(radar_store::GraduationMode::Organic) => {
                record.organic += 1;
                population.organic += 1;
            }
            Some(radar_store::GraduationMode::Instant) => {
                record.instant += 1;
                population.instant += 1;
            }
            None => {}
        }
    }

    Ok(CreatorIndex {
        watermark_slot: as_of.slot().get(),
        built_at,
        // Always `Some` from a build. `None` is reserved for an index written
        // before the field existed, where it means "not measured" -- so emitting
        // `None` from here would make an empty store indistinguishable from an
        // old file, and rule 9 says those are different.
        population: Some(population),
        creators,
    })
}
