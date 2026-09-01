// SPDX-License-Identifier: Apache-2.0
//! That a restart does not hand a customer their allowance back.
//!
//! The first version of the share meter kept its counts in memory. Deploys are
//! routine and `Restart=always` is configured, so in practice the allowance came
//! back several times a day — and under a crash loop, per crash. That is
//! [`LEARNINGS`] entries 1 and 9 in a new costume, and it is the exact failure
//! `RADAR_STATE_DIR` was made mandatory to stop for the global budget.
//!
//! Asserted by building a second meter over the same directory, because that is
//! what a restart is. A test that reached into the first meter's memory would
//! prove the counter increments, which was never in doubt.
//!
//! [`LEARNINGS`]: https://github.com/hey-vera/radar/blob/main/LEARNINGS.md

use radar_serve::ledger::Store;
use radar_serve::share::{Allowance, Refused, Shares};

const SALT: &[u8] = &[7u8; 32];
const DID: &str = "did:privy:first";
const OTHER: &str = "did:privy:second";

/// A fresh state directory, so runs cannot see each other's counts.
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("radar-share-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a writable scratch directory");
    dir
}

#[test]
fn a_spent_allowance_is_still_spent_after_a_restart() {
    let dir = scratch("restart");
    let today = 20_260_901;

    let before = Shares::restored(
        Allowance::per_day(2),
        Store::at(&dir).expect("a store"),
        today,
    );
    assert_eq!(before.charge(DID, SALT, today), Ok(1));
    assert_eq!(before.charge(DID, SALT, today), Ok(2));
    assert!(before.charge(DID, SALT, today).is_err(), "spent");

    // The restart.
    drop(before);
    let after = Shares::restored(
        Allowance::per_day(2),
        Store::at(&dir).expect("a store"),
        today,
    );

    assert_eq!(
        after.charge(DID, SALT, today),
        Err(Refused::Spent { allowance: 2 }),
        "a restart must not hand the allowance back"
    );

    // And it is still per customer, not a global latch.
    assert_eq!(after.charge(OTHER, SALT, today), Ok(1));
}

#[test]
fn yesterdays_counts_do_not_refuse_todays_questions() {
    // The mirror-image bug. Restoring a record from an earlier day would refuse
    // everyone until midnight, which is the same defect pointing the other way.
    let dir = scratch("newday");

    let yesterday = Shares::restored(
        Allowance::per_day(1),
        Store::at(&dir).expect("a store"),
        20_260_901,
    );
    assert_eq!(yesterday.charge(DID, SALT, 20_260_901), Ok(1));
    assert!(yesterday.charge(DID, SALT, 20_260_901).is_err());
    drop(yesterday);

    let today = Shares::restored(
        Allowance::per_day(1),
        Store::at(&dir).expect("a store"),
        20_260_902,
    );
    assert_eq!(
        today.charge(DID, SALT, 20_260_902),
        Ok(1),
        "tomorrow is a new day"
    );
}

#[test]
fn an_unreadable_record_starts_empty_rather_than_locking_everyone_out() {
    // The one place here where the safe direction is permissive, and it is
    // deliberate: a corrupt file must not lock a paying customer out of the
    // product while the global budget still bounds what can be spent. Losing a
    // day's counts costs fairness for a day; refusing costs the product.
    let dir = scratch("corrupt");
    std::fs::write(dir.join("chat-shares.json"), b"{ not json").expect("writes");

    let shares = Shares::restored(
        Allowance::per_day(1),
        Store::at(&dir).expect("a store"),
        20_260_901,
    );
    assert_eq!(shares.charge(DID, SALT, 20_260_901), Ok(1));
}

#[test]
fn the_written_record_holds_hashes_rather_than_identifiers() {
    // The file outlives the request by years and will be copied. A copy that
    // holds counts cannot be joined against anything; one holding DIDs can.
    let dir = scratch("hashed");
    let shares = Shares::restored(
        Allowance::per_day(5),
        Store::at(&dir).expect("a store"),
        20_260_901,
    );
    shares.charge(DID, SALT, 20_260_901).expect("charges");

    let written = std::fs::read_to_string(dir.join("chat-shares.json"))
        .or_else(|_| std::fs::read_to_string(dir.join("chat-shares")))
        .expect("the record was written at all");
    assert!(
        !written.contains(DID),
        "the durable record must not carry the identifier: {written}"
    );
    assert!(written.contains("counts"), "and it must carry the counts");
}
