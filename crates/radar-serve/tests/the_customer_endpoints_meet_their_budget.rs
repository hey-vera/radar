// SPDX-License-Identifier: Apache-2.0
//! How long a customer-facing read actually takes, against a real store.
//!
//! # Why this is a test and not a benchmark
//!
//! The frontend plan sets a budget — **p95 under 500ms for every customer-facing
//! endpoint** — and a budget nobody measured is an aspiration. It was written as
//! one, because there was no local store to measure against, and "measure, do not
//! assume" is the whole point of having written it down.
//!
//! So this runs the real `api::` functions over a real copy of the production
//! store. It is a test rather than a `#[bench]` so it can *fail* when an endpoint
//! outgrows its budget, which is the behaviour that matters: a benchmark reports
//! a regression to whoever reads the output, and nobody reads the output.
//!
//! # Why it skips when the store is absent
//!
//! It needs data nobody should commit — 80MB of parquet — so it reads
//! `RADAR_BUDGET_STORE` and **skips loudly** when that is unset. A skip prints
//! what it would have measured and how to run it.
//!
//! Skipping is not free and the risk is named rather than hidden: a test that
//! usually skips is a test nobody notices has stopped working. It is here anyway
//! because the alternative is a budget nobody ever checks, and a check that runs
//! when someone points it at a store beats a number in a document.
//!
//! ```text
//! RADAR_BUDGET_STORE=/path/to/store cargo test -p radar-serve --test the_customer_endpoints_meet_their_budget -- --nocapture
//! ```

use std::time::{Duration, Instant};

use radar_asof::AsOf;
use radar_serve::api;
use radar_store::Reader;

/// What the frontend plan promises a customer-facing endpoint will do.
const BUDGET: Duration = Duration::from_millis(500);

/// Where a real store is, when one has been pointed at.
fn store() -> Option<Reader> {
    let dir = std::env::var("RADAR_BUDGET_STORE").ok()?;
    let path = std::path::PathBuf::from(&dir);
    if !path.join("outcomes").is_dir() {
        eprintln!("RADAR_BUDGET_STORE={dir} has no outcomes/ — not a store");
        return None;
    }
    Some(Reader::open(path))
}

/// Runs `f` a few times and returns the slowest.
///
/// The slowest rather than the mean, deliberately. A budget is about the bad
/// case, and averaging is how a p95 problem is reported as a p50 success.
fn slowest<T>(runs: u32, mut f: impl FnMut() -> T) -> Duration {
    (0..runs)
        .map(|_| {
            let started = Instant::now();
            let _ = f();
            started.elapsed()
        })
        .max()
        .unwrap_or_default()
}

#[test]
fn every_customer_facing_read_fits_in_its_budget() {
    let Some(reader) = store() else {
        eprintln!(
            "SKIPPED: set RADAR_BUDGET_STORE to a real store to measure the \
             p95 budget the frontend plan promises. Nothing was measured."
        );
        return;
    };

    let watermark = Reader::watermark(&reader)
        .expect("a readable store")
        .expect("a store with something in it");
    let as_of = AsOf::at(watermark);

    // Read once outside the timing so the figures describe the endpoint rather
    // than the first page fault. A cold cache is a different measurement and it
    // is not the one the budget is about -- the process is long-lived.
    let decisions = reader.read_decisions(as_of).expect("decisions");
    let outcomes = reader.read_outcomes(as_of).expect("outcomes");
    eprintln!(
        "store: {} decisions, {} outcomes, watermark {}",
        decisions.len(),
        outcomes.len(),
        watermark.get()
    );

    // A mint that is really in the store, so the filter does real work. Picking
    // an absent one measures the scan and none of the matching, which is the
    // cheaper half.
    let mint = decisions
        .first()
        .map(|d| d.mint.to_string())
        .expect("a store with a decision in it");

    let mut over: Vec<String> = Vec::new();
    let mut report = |name: &str, took: Duration| {
        eprintln!("  {name:<28} {:>8.1} ms", took.as_secs_f64() * 1000.0);
        if took > BUDGET {
            over.push(format!(
                "{name} took {:.0}ms against a {}ms budget",
                took.as_secs_f64() * 1000.0,
                BUDGET.as_millis()
            ));
        }
    };

    report(
        "/v1/tokens/{mint}",
        slowest(3, || api::token_evidence(&reader, &mint, as_of)),
    );
    report(
        "/v1/decisions",
        slowest(3, || {
            api::page(
                reader.read_decisions(as_of).expect("decisions"),
                &api::Query {
                    limit: api::DEFAULT_LIMIT,
                    ..api::Query::default()
                },
                watermark.get(),
            )
        }),
    );
    report(
        "/v1/evidence/capacity",
        slowest(3, || api::capacity(&decisions, watermark.get(), 850)),
    );
    report(
        "/v1/evidence/returns",
        slowest(3, || {
            api::returns(&decisions, &outcomes, watermark.get(), 850)
        }),
    );
    report(
        "/v1/evidence/activity",
        slowest(3, || api::activity(&decisions, watermark.get(), 14)),
    );
    report(
        "/v1/scoreboard",
        slowest(3, || api::scoreboard(&reader, as_of, 850)),
    );

    assert!(
        over.is_empty(),
        "customer-facing endpoints over budget: {over:#?}"
    );
}
