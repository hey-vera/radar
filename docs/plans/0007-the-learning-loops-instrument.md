<!-- SPDX-License-Identifier: Apache-2.0 -->
# Plan 0007 — The learning loop's instrument

**Status:** in progress
**Branch:** `docs/0010-close-the-remainder-then-raise-the-ceiling` for item 0,
stacked on #154; `feat/the-learning-loops-instrument` from `main` for items
1–4, once the stack has merged
**Base:** `b0b8e05`, the tip of #147–#154
**Planned by:** Fable 5.1, plan mode, 2026-09-05
**Design:** [0010](../design/0010-close-the-remainder-then-raise-the-ceiling.md)
§4 A-1 and §6; design 0007 E1 and E3

## Objective

Radar can measure, out of sample and on rows it did not fit on, whether any
stratum of its own recorded launches clears the bar — and it refuses, at the
type level, a feature that peeks past a row's watermark. Afterwards
`radar features` writes a deterministic feature table from the store,
`radar edge` runs the walk-forward protocol over it, research 0026 records the
first result on the deterministic rule whatever that result is, and every
fact the bot later wants to state has the pass it must go through first.

## Not in scope

- `Policy::CLOSED`, the kernel, the signer, `radar-exec`. Nothing here reaches
  them. A `Found` is an ADR for Josh, not a change in this plan.
- `creator_edge`'s thresholds. Measured, not moved.
- The bot, the box, the shadow strategy (design 0010 V2), E4 and E5 (design
  0010 A-3 and A-2).
- A new crate. E1 and E3 live in `radar-research` and `radar-cli`, where
  their callers are.
- A Python dependency. The probes stay stdlib-only; the harness is Rust over
  the store's `Reader`.

## Items

- [ ] 0. Design 0010 and this plan in the repository
      gate: `cargo test -p repo-conformance` green; the PR open, stacked on
      #154
- [ ] 1. **E1 — `radar features`.** `new:crates/radar-research/src/features.rs`
      and `new:crates/radar-cli/src/features.rs`. One row per succeeded launch
      with an outcome measured at both the one-hour and the twenty-four-hour
      checkpoint. T is launch plus 6,000 slots — `Thresholds::DEFAULT`'s
      `max_token_age` — with `--at` for other offsets. **Every feature value is
      `Observed<f64>` at the slot it was read and is accepted against `AsOf(T)`
      in the row builder**, so a value from after T is a `LookAhead` error and
      never a number: `radar-asof`'s two idle types get their caller here.
      Features: the launch slot's distinct traders and transaction count from
      the trades table, and the longest run of consecutive `tx_index` among
      them (contiguity); `dev_buy_lamports`; the creator's prior launches,
      prior organic, prior instant, prior stillborn and launches per day,
      counted only from outcomes measured at or before T — the pivot
      discipline `study.rs` already keeps, applied per row; trades and
      distinct traders in the first 25, 300 and 6,000 slots; trades to reach
      10, 20 and 30 SOL of realised inflow; the count of prior launches
      sharing the name, the symbol and the URI's host; `launch_recipients`,
      `launch_transactions` and `authority_prevalence` from the decision row
      where one exists (2026-09-04 onward), absent otherwise — never zero.
      Labels: net return T→6h and T→24h, the checkpoint's `last_price` over the
      one-hour checkpoint's `last_price`, minus the round trip of the band a
      fifth-of-capacity position would sit in, read from the snapshot's
      `by_notional`; absent where either price is absent. Output: Parquet in a
      research directory beside the store, named by the store's watermark,
      never appended to the store (ADR 0006: derived and recoverable).
      gate: two runs → identical sha256; the planted leak — a feature
      observed at T + 1 — refused as `LookAhead` in a test, and the test
      fails with the `accept` removed; `cargo test -p radar-research`; clippy
      clean; the launch-slot trader count compared against
      `Decision.launch_recipients` on the mints that carry both — a bound,
      not an equality, since traders and token accounts are different things
- [ ] 2. **E3 — `radar edge`.** `new:crates/radar-research/src/edge.rs` and
      `new:crates/radar-cli/src/edge.rs`. Folds: five contiguous windows by
      launch slot, equal in rows, never shuffled. Purge and embargo: a fit
      fold's rows must have their twenty-four-hour checkpoint before the
      boundary, and the test fold begins 216,000 slots after it (López de
      Prado 2018, chapter 7). Strata: conjunctions of at most three feature
      thresholds at fit-fold deciles, enumerated; the fit-fold winner is the
      highest median net return with at least 100 rows; tested on the next
      two folds; `Found` only if the median net return is at or above the bar
      on both, each with at least 100 rows, and the Wilson 95% lower bound of
      the share with a net return above zero exceeds one half on both — a
      median over a point mass at zero is a report about the point mass
      (research 0017). The bar: 456 bps, with 850 reported beside it
      (research 0022). Fixed strata run without fitting through the same test
      folds: `creator_edge`'s thresholds, and the refusal signals' complements
      — the strongest band or above; a creator with measured launches and no
      organic graduation — so the refusal edge is measured too. The number of
      strata tried is printed beside every verdict. Seeded; no clock; no
      network.
      gate: a seeded uniform-noise feature added to the grammar is never
      `Found` across ten seeds, and when it wins a fit fold it fails both test
      folds; the deterministic rule's fit-free result lands inside 0017's
      interval on the overlapping window (two instruments); `cargo test`;
      clippy clean; mutants on the fold boundary and the two acceptance
      conditions, in CI
- [ ] 3. **Research 0026** —
      `new:docs/research/0026-the-walk-forward-protocol-and-what-it-found.md`:
      the protocol, the two planted tests and the commands that ran them, the
      per-fold tables for the deterministic rule, the refusal strata and the
      best fitted stratum, the number of strata tried, and the verdict — a
      null written as a result.
      gate: every figure carries its window and watermark; the commands
      reproduce it; `cargo test -p repo-conformance` green
- [ ] 4. **`docs/STATE.md`**: the learning-loop section gains the measured
      result with its date and the command; "Where to start" says
      `Observed<T>` and `LookAhead` have a caller — or the two types are
      deleted in this PR if item 1 found no honest use for them (design 0010
      §8.1 row 3), with the crate's own doc changed in the same commit.
      gate: conformance; the sentence names the command

## Open questions for Josh

- Q1 (2026-09-05): the bar for E3 is 456 bps with 850 reported beside it,
  per research 0022. Assumed unless changed.

## Handback

Stopped at: item 0 — the two documents written; nothing else started. Plan
mode; no code was run, built or changed.
Next action: merge #147–#154 in order, then #146, then this PR; branch
`feat/the-learning-loops-instrument` from `main`; item 1.
Do not: touch `Policy::CLOSED`, the thresholds, the bot or the box; add a
crate; add a Python dependency; delete `Observed<T>` before item 1 has tried
to use it — an unused type and a deleted invariant look identical in a
diffstat (plan 0003's handback).
