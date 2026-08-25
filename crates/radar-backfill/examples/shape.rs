// SPDX-License-Identifier: Apache-2.0
//! Reads one launch block's shape and scores it.
//!
//! Verifies the fetch path against real chain rather than trusting that the SQL
//! is right. The unit tests check the query's shape; only this checks that the
//! endpoint agrees.
use radar_backfill::launch_block::CryptoHouseBlocks;
use radar_graph::{LaunchBlockSource, assess};
use radar_types::{Address, Slot};

fn main() {
    let mut args = std::env::args().skip(1);
    let mint: Address = args.next().expect("mint").parse().expect("base58");
    let slot = Slot(args.next().expect("slot").parse().expect("number"));
    let shape = CryptoHouseBlocks::default()
        .shape_at(&mint, slot)
        .expect("query");
    let a = assess(shape);
    println!(
        "{mint} slot {slot}: recipients={} txs={} -> {:?} (lift {})",
        shape.recipients, shape.transactions, a.coordination, a.instant_lift_x100
    );
}
