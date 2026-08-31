// SPDX-License-Identifier: Apache-2.0
//! A daily budget that a restart resets is not a daily budget.
//!
//! Rule 8 claims this property in `AGENTS.md`, and until now nothing implemented
//! it. `radar-provider` has a [`Ledger`] type, `radar-agent` has an
//! `Agent::restore` that consumes one — and `restore`'s only caller was a unit
//! test. `radar-serve`'s startup path called `Agent::new`, so every restart
//! began the day again at zero.
//!
//! `radar-serve` runs under `Restart=always`. A crash loop would have handed out
//! a fresh day's allowance per crash, and the budget exists precisely so a
//! runaway cannot spend without a ceiling.
//!
//! This exercises the composition rather than either half: what the ledger
//! writes is what the agent restores, and the restored agent's ceiling is still
//! the one the first agent was spending against.

use radar_agent::{Agent, Allowlist, Budget, Config, Ledger};
use radar_serve::ledger::Store;
use radar_types::MicroUsd;

const RECORD: &str = "model-ledger";
const TODAY: u64 = 20_331;

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("radar-restart-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn config() -> Config {
    let mut allowlist = Allowlist::new();
    allowlist.allow("creator_history");
    Config {
        budget: Budget {
            per_call_max: MicroUsd(10_000),
            daily_max: MicroUsd(30_000),
        },
        allowlist,
    }
}

#[test]
fn a_restart_does_not_hand_out_a_fresh_days_allowance() {
    let store = Store::at(&scratch("carries-forward")).expect("a writable scratch directory");

    // Three calls at the per-call ceiling exhausts the day exactly.
    let mut before = Agent::new(config(), TODAY);
    for i in 1..=3 {
        let commitment = before
            .begin(MicroUsd(10_000), TODAY)
            .unwrap_or_else(|why| panic!("call {i} is inside the budget: {why}"));
        before.settle(commitment, MicroUsd(10_000));
        store.write(RECORD, &before.ledger()).expect("records");
    }
    assert!(
        before.begin(MicroUsd(10_000), TODAY).is_err(),
        "the day is spent before the restart"
    );

    // The process dies here.
    drop(before);

    let saved: Ledger = store.read(RECORD).expect("the ledger was written");
    let mut after = Agent::restore(config(), &saved, TODAY);

    assert!(
        after.begin(MicroUsd(10_000), TODAY).is_err(),
        "the restarted agent got a fresh allowance -- this is the bug, and it is \
         what `Agent::new` at startup did"
    );
    assert_eq!(
        after.ledger().spent,
        30_000,
        "the restored agent must carry the day's spend, not merely refuse"
    );
}

#[test]
fn a_restart_on_a_new_day_starts_the_new_day_clean() {
    // The other direction, and it has to hold or the fix is worse than the bug:
    // restoring yesterday's spend as though it were today's would refuse
    // everything until midnight. `Meter::restore` drops a stale day, and this
    // asserts the composition inherits that rather than re-implementing it.
    let store = Store::at(&scratch("new-day")).expect("a writable scratch directory");

    let mut yesterday = Agent::new(config(), TODAY);
    let commitment = yesterday
        .begin(MicroUsd(10_000), TODAY)
        .expect("inside the budget");
    yesterday.settle(commitment, MicroUsd(10_000));
    store.write(RECORD, &yesterday.ledger()).expect("records");

    let saved: Ledger = store.read(RECORD).expect("the ledger was written");
    let mut today = Agent::restore(config(), &saved, TODAY + 1);

    assert_eq!(
        today.ledger().spent,
        0,
        "yesterday's spend has no claim on today's allowance"
    );
    assert!(
        today.begin(MicroUsd(10_000), TODAY + 1).is_ok(),
        "a new day must be spendable"
    );
}

#[test]
fn an_in_flight_commitment_is_recorded_before_the_call_goes_out() {
    // The reason the ledger is written at `begin` and not only at `settle`.
    //
    // A process that dies mid-call cannot know whether the call happened.
    // Recording only on settlement loses exactly the calls that crashed in
    // flight, which is what a runaway loop consists of -- so a crash between
    // reserving and settling must still cost the day its reservation.
    let store = Store::at(&scratch("in-flight")).expect("a writable scratch directory");

    let mut before = Agent::new(config(), TODAY);
    let _never_settled = before
        .begin(MicroUsd(10_000), TODAY)
        .expect("inside the budget");
    store.write(RECORD, &before.ledger()).expect("records");
    drop(before);

    let saved: Ledger = store.read(RECORD).expect("the ledger was written");
    assert_eq!(
        saved.spent, 10_000,
        "an in-flight commitment must reach the durable record"
    );

    let after = Agent::restore(config(), &saved, TODAY);
    assert_eq!(
        after.ledger().spent,
        10_000,
        "and must survive the restart, because assuming the call did not happen \
         risks paying for it twice"
    );
}

#[test]
fn nothing_written_yet_is_a_clean_start_rather_than_a_failure() {
    // The first boot on a fresh machine. The absent case here means "no spend
    // recorded", which is true, and it is the one absent case in this system
    // that is genuinely safe -- unlike an absent *budget*, which must refuse.
    let store = Store::at(&scratch("first-boot")).expect("a writable scratch directory");
    assert_eq!(store.read::<Ledger>(RECORD), None);

    let mut fresh = Agent::new(config(), TODAY);
    assert!(fresh.begin(MicroUsd(10_000), TODAY).is_ok());
}
