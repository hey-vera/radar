<!-- SPDX-License-Identifier: Apache-2.0 -->
# ADR 0012 — Record the launch-block count; do not retune the threshold

**Date:** 2026-09-03
**Status:** accepted
**Decides:** what to do about `radar-graph`'s coordination thresholds now that
[research 0024](../research/0024-the-spike-became-a-hump-and-the-signal-moved.md)
has superseded the measurement they were derived from.

## Context

`radar-graph` refuses on the shape of a launch block. Its thresholds come from
[`0008`](../research/0008-the-launch-block-gives-the-bundle-away.md), which
measured eighty launches per population over two days and found that 68% of
instantly-graduating launches had **exactly six** recipients against 5% of
launches that never graduated.

0024 re-measured that on 17,497 launches. The direction survived and the
magnitude did not:

| | 0008 | 0024 |
|---|---|---|
| instant launches with exactly six | 68% | **25.1%** |
| six recipients, vs base rate for instant graduation | 11.7× | **4.4×** |
| strongest band | six | **ten to thirteen**, at 10.1× |

So the rule in the decision lane today fires on a band that is no longer the
strongest one, using a magnitude that is wrong by a factor of 2.7.

**0008 predicted this exactly**, which is the part that decides this ADR:

> "**Six is a tool's default, not a law.** The number will move when whoever is
> running this changes their configuration, and the detector will go quiet
> without saying so. What should survive is the *shape* of the argument — a
> spike with holes either side — rather than the constant. Re-running this is
> how that gets noticed."

Nothing was built to notice. The drift was found nine days later, by hand,
because somebody happened to re-run the measurement.

## The options

**A. Retune the threshold to 0024's numbers.** Move the trigger to the
ten-to-thirteen band and update the magnitudes.

**B. Record the recipient count in the store, and derive thresholds from it.**

**C. Do nothing.**

## Decision

**B, and explicitly not A.**

**A is the wrong fix because it is the same mistake with fresher numbers.** The
defect 0024 exposes is not that six is the wrong constant. It is that a refusal
rule hard-codes a number *that nobody can re-derive without a twenty-five-minute
chain scan*, so the next drift will also be found late and by accident. Picking a
new constant buys correctness until the launchers change their configuration
again, and there is no mechanism that would tell us when.

**The root cause is that the store keeps the verdict and throws away the
number.** `Decision.coordination` persists the label; the count that produced it
is not a stored field. This is the same fact that forced `radar-onchain` to exist
at all — the count is answerable from the store for 7,543 mints out of 483,629,
and essentially never for the one being asked about.

That was a correct decision when it was made.
[`launch_block.rs`](../../crates/radar-backfill/src/launch_block.rs) rejected
recording it on cost, and the reason was not small: the recorder's extraction
groups token transfers by *transaction*, while the shape needs them grouped by
`(mint, slot)`, which for the whole window means grouping every token transfer on
Solana. On 2026-08-24 a two-minute spam burst pushed the existing query past the
endpoint's limits and took the recorder down for thirteen hours.

**Phase 1 changes that trade.** `radar-onchain` reads a launch block from RPC in
five or six calls, in about 1.5 seconds, for one mint. The recorder sees roughly
1,300 launches an hour. That is a bounded, per-mint cost against a direct RPC
endpoint rather than a heavier query against the shared CryptoHouse one — a
different operation from the one that was rejected, and it does not touch the
recorder's existing query at all.

Once the count is recorded, re-deriving a threshold is a **query**, drift is
**visible**, and the alarm 0024 asks for becomes possible to build.

## Why it is safe to leave the threshold wrong meanwhile

Stated plainly, because "we know it is wrong and we are not fixing it yet" needs
a reason rather than a shrug.

- **The trading lane is frozen and `Policy::CLOSED` ships.** Nothing has ever
  traded. A coordination verdict cannot currently cause or prevent a trade.
- **`radar-graph` refuses; it does not select.** Its verdict is a
  disqualification, so being wrong in the "fires too rarely" direction loses a
  refusal rather than authorising a purchase.
- **The public analyst does not use it.** `radar-roast` reads the
  [published snapshot](../research/data/0024-base-rates.json), which carries
  0024's corrected figures and a `measured_on` date, and it places a mint in a
  band at reply time rather than consulting `radar-graph`'s constant.

So the cost of the delay is **a mislabelled field in a table**, not a bad trade
and not a wrong public claim. That is what makes this a sequencing decision
rather than an urgent one. **If `Policy::CLOSED` is ever opened, this becomes
urgent in the same change** — a lane that can trade must not be gated on a
threshold measuring a superseded number.

## What this commits to

1. `LaunchBlockShape`'s `recipients` and `transactions` become **recorded
   fields** on the decision, alongside the label rather than instead of it.
2. `radar-graph`'s thresholds are then derived from the recorded distribution and
   carry the date they were derived, in the shape
   `docs/research/data/0024-base-rates.json` already uses.
3. A scheduled re-run alarms when the mode moves, building in the three lessons
   0024 names: LEARNINGS 21 (a monitor slower than its failure reports history)
   and LEARNINGS 5 and 26 (a check must fail differently when it did not run than
   when it found nothing).

Until 1 exists, **no threshold change is made**, and `radar-graph`'s constants
are documented as deriving from a superseded measurement — in
[`README.md`](../../README.md)'s crate table and in 0024's *What should follow*.

## What this does not decide

- **Whether the recorder should do the RPC read itself**, or whether it belongs
  in a separate pass. That is a wiring question with a real cost attached, and it
  should be answered against a measurement of the recorder's actual headroom
  rather than here.
- **What the new thresholds should be.** Deriving them is the point of recording
  the count; guessing them now would be option A wearing a different hat.
