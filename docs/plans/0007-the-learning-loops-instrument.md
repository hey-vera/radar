<!-- SPDX-License-Identifier: Apache-2.0 -->
# Plan 0007 — The learning loop's instrument

**Status:** in progress
**Branch:** `docs/0010-close-the-remainder-then-raise-the-ceiling` for item 0,
merged as #155; `feat/the-learning-loops-instrument` from `main` for items 1–4
**Base:** `464e742`, `main` with the whole stack merged
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

- [x] 0. Design 0010 and this plan in the repository
      done: `cargo test -p repo-conformance` 33 passed at `85e7ad8`; merged as
      #155, and #147-#154 and #146 with it, so `main` is `464e742`
- [x] 1. **E1 — `radar features`.** `new:crates/radar-research/src/features.rs`
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
      done: `crates/radar-research/src/features.rs` and
      `crates/radar-cli/src/features.rs`; 24 tests green
      (`cargo test -p radar-research`, `-p radar-cli`), clippy clean under the
      workspace's pedantic lints. The planted leak is
      `a_value_observed_after_t_is_refused_and_never_becomes_a_number`, and the
      end-to-end half is
      `a_creators_record_counts_only_what_had_been_measured_by_t` in
      `crates/radar-research/tests/the_feature_table_cannot_see_the_future.rs`:
      a sibling token's graduation measured after T is not counted, and
      `the_same_record_measured_before_t_does_count` is the same store with one
      number moved, so a function returning zero fails it. Determinism is
      `the_same_table_writes_the_same_bytes`; the round trip is
      `the_file_says_what_the_table_said`, which also holds that an absent value
      survives as absent.
      not done in this item, and why: the launch-slot trader count against
      `Decision.launch_recipients` is a comparison of two instruments over the
      **production** store, and no store exists on this workstation. It belongs
      to research 0026 (item 3), which runs on the box, and it is listed there
- [x] 2. **E3 — `radar edge`.** `new:crates/radar-research/src/edge.rs` and
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
      done: `crates/radar-research/src/edge.rs` and
      `crates/radar-cli/src/edge.rs`; the whole workspace green and clippy clean
      at this commit. `noise_is_never_found_across_ten_seeds` is the planted
      test and it passes on all ten;
      `an_engineered_edge_is_found` is its opposite number and matters more,
      because a harness that never says `Found` passes the first test and is
      worthless — it failed twice and forced the two changes in Q3 below.
      `the_bar_and_the_round_trip_come_from_the_snapshot` holds design 0010
      §6.1's rule that neither is a constant here, and
      `a_band_the_snapshot_does_not_name_is_refused_rather_than_substituted`
      holds that an absent band is a refusal. `the_table_a_store_produces_feeds_the_protocol`
      is the join: a store on disk, through the builder, out to a file, back in,
      and into the protocol.
      not done in this item: the deterministic rule's fit-free result against
      0017's interval is a **measurement**, not a unit test — it needs the
      production store and belongs to item 3
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
- Q2 (2026-09-05), **answered, and not Josh's to answer**: the plan said the
  label is net of the band's round trip *and* that `Found` needs the median net
  at or above 456 bps. That double-charges, and `docs/STATE.md`'s reconciliation
  of the three cost figures says why in one sentence: **"250 and 456 are the
  same measurement read in two different bands."** 456 **is**
  `by_notional["$20-$200"]`. There is no second bar; the bar *is* the round
  trip, and clearing it is exactly "the net pays for itself".

  The same paragraph settled which round trip to charge, and the answer was not
  the one the design gave. `by_notional` is 0019's table over **all** pump.fun
  trades in an hour. These rows are **fresh launches**, which 0019 measured
  separately at **850 bps** — a new associated token account is rent, and early
  curve positions carry more slippage — and it **declined to lower the
  constant** on that evidence, because the population is wrong and a cost
  rounded down launders a trade past the gate that should have refused it.

  So the harness charges 850 by default, `--cost-band` selects a `by_notional`
  row for a sensitivity run, and acceptance is three conditions rather than
  four: at least a hundred rows, the net **measurably** above zero — by more
  than the standard error of its own median, since a median that only looks
  positive inside its own noise has not been shown to be — and the Wilson lower
  bound of the share that paid above a half.

  Net effect: **stricter than what the double charge produced.** It required a
  gross median of 456; this requires 850 plus a margin, and it requires it for
  the right reason. Design 0010 §6.1 is superseded on this point and is not
  edited — it decays by its own rule, and STATE.md carries the correction.

- Q3 (2026-09-05), **two protocol changes item 2's tests forced**, both
  deviations from design 0010 §6.2 and both recorded here rather than made
  quietly:
  1. **The fitting-period row floor is scaled from the smallest test fold**, not
     flat at a hundred. A stratum holding a hundred fitting rows out of sixteen
     hundred holds about twenty-five in a four-hundred-row test fold and can
     never be accepted; fitting on it displaces one that could have been. The
     floor is now the share that would leave 120 rows — the acceptance floor
     plus two standard deviations of a count that size — in the smallest test
     fold.
  2. **The fit-fold winner is not simply the highest median.** Taken literally
     that preferred a three-term refinement whose rows' noise ran high, which
     then failed the test folds and took a planted 3,000 bps edge with it,
     because only the winner is tested. A candidate now has to beat the held one
     by more than the larger of their two median standard errors; inside that
     band the second acceptance condition decides — the Wilson lower bound on
     the share of rows that actually paid, which already prefers more rows at an
     equal share — and a dead-level tie goes to fewer terms.

  Neither weakens the acceptance test, which is unchanged. Both change **what
  gets tested**, and without them the harness could not see an edge it was
  looking straight at.

## Handback

Stopped at: items 1 and 2 done and green on
`feat/the-learning-loops-instrument`. The instrument exists end to end —
`radar features` writes the table, `radar edge` runs the protocol over it — and
**it has never been run against the production store**, so there is still no
measured result and STATE.md says so.

Next action, and it is the whole of items 3 and 4: run the pass on the box.
That needs a Linux `radar` binary there, which this workstation cannot build.
The route, in order:

1. Merge this PR. `release-linux` builds on pushes to `main`.
2. `gh run download <run> -n <artifact>` and `scp` the binary to
   `guardian-vps-tail:~/bin/radar-next`, installing by rename — never over a
   running binary.
3. Run it **windowed and niced**, not over all of history at once:
   `nice -n 19 ~/bin/radar-next features --store ~/radar/data/store --from <slot> --to <slot> --out ~/radar/data/research/<name>.parquet`.
   The trades table is the large read and `radar-follow` is on the same box;
   design 0010 §11 item 5 is the warning, and the recorder dying quietly is a
   failure this repository has already had.
4. `radar-next edge --features <that file>`, then write research 0026 from what
   it says — including the deterministic rule's fit-free result against 0017's
   interval, which is the two-instruments comparison item 2 could not make here.
5. Item 4's STATE.md sentence then gains the measured result and its date. The
   instrument half of that sentence is already written.

Do not: touch `Policy::CLOSED`, the thresholds, the bot or the box's units; add
a crate; add a Python dependency; run the pass unwindowed on the box; or write
research 0026 without a run behind it — a note with no measurement is the thing
this repository refuses.
