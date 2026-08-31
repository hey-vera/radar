<!-- SPDX-License-Identifier: Apache-2.0 -->
# 0019 — The round trip is not one number

**Date:** 2026-08-30
**Source:** 183,647 pump.fun trade legs, 2026-08-25 04:00–05:00 UTC
**Status:** measured. **The 850 bps constant is not changed on this evidence**,
and the reason is a population difference this note does not close.

## What could not be checked before

`creator_edge::Thresholds::assumed_round_trip_bps` is **850**. It is what turns
[`0014`](0014-the-control-was-entirely-tokens-nobody-could-sell.md)'s gross
median of +21 bps into −829, and it is the figure every net number in this
repository rests on.

Its doc comment describes the measurement in detail — 26,691 fills touching 200
pump.fun tokens launched in one hour, a median of 423 bps a leg, a mean of 845, a
90th percentile of 2,280, a breakdown by transaction shape. **The query that
produced it is not in this repository.** No note, and `docs/research/queries/`
did not hold it. [`LEARNINGS`](../../LEARNINGS.md) entry 1's shape, on the most
load-bearing constant in the system.

The guard beside it does not help: `the_assumed_cost_is_not_below_what_a_round
_trip_was_measured_to_cost` compares 850 against `MEASURED_MEDIAN_ROUND_TRIP_BPS`
and both are hand-entered in the same file. It guards a transcription.

## The question the constant cannot answer

850 bps is applied as a **pure proportion**. The method it came from explicitly
captures "rent and any second hop", and rent is **fixed** — an associated token
account costs about 0.00204 SOL whatever the trade is worth.

A cost with a fixed component, measured on other people's trade sizes and then
applied in basis points to a $6.21 position, is arithmetic rather than a cost
model. So this measures cost **as a function of notional**.

## The result

```
notional >=      fills   median bps  median lamports
  1,000,000      29996         1521           115,000     $0.20 – $2
 10,000,000      79061          125           231,971     $2 – $20
100,000,000      56777          228         7,257,585     $20 – $200
1,000,000,000    17723          225        31,163,295     $200 – $2,000
10,000,000,000      90          130       265,302,520     $2,000+
```

**Cost is strongly size-dependent, and the dependence is at the bottom.** A leg
under $2 costs **1,521 bps**; above $20 it settles near **225 bps**. That is the
fixed-cost signature: the lamport column is roughly flat across the two smallest
buckets (115,000 against 231,971 for a tenfold jump in notional) and then grows
proportionally.

## What this changes today, and what it does not

**It does not change `assumed_round_trip_bps`.** Two reasons, and the second is
the one that matters.

The population is different. The original measured *trades on 200 tokens
launched in that hour*; this measures *all pump.fun trades in that hour* —
183,647 legs against 26,691, roughly sevenfold. Fresh launches are exactly the
cohort where costs should be highest: a new token means a new associated token
account for most buyers, which is rent, and early curve positions carry more
slippage. **So the two numbers are not in contradiction; they are answers to
different questions**, and the original's question is the one Radar's own trades
belong to.

And the direction is the dangerous one. `creator_edge`'s own documentation says
a cost estimate "rounded down is the direction that launders a trade past the
kernel". Lowering 850 on a measurement of a broader, cheaper population would be
exactly that, and no result here justifies it.

**What it does change is a threshold that was never examined.**
`min_notional` is `MicroUsd::DOLLAR` — **$1.00**, about 5,000,000 lamports at a
SOL near $200, which lands squarely in the **1,521 bps** band. A position at
Radar's own floor faces a round trip of roughly **30%**.

Radar's *median* proposed notional, $6.21, is about 31,000,000 lamports and sits
in the $2–$20 band. So the floor and the median live in bands whose costs differ
by an order of magnitude, and nothing in the system knows that.

## What should follow

1. **Raise `min_notional` above the fixed-cost cliff.** Anything under about
   10,000,000 lamports is in the expensive band. This is a threshold change with
   a measurement behind it, in the direction that refuses more trades — the safe
   direction — and it is the first concrete change this measurement licenses.
2. **Re-run restricted to fresh launches**, which closes the population gap and
   is the only comparison that can honestly move the 850. It needs the store's
   own launch list joined into the query, which `radar cost` does not do yet.
3. **Make the cost a function rather than a constant.** The kernel currently
   multiplies one bps figure by the notional. A two-term model — a fixed lamport
   cost plus a proportional rate — is what the lamport column above describes,
   and it would make the $1 floor refuse itself.

## What this does not establish

- **The bands are not monotonic**, and the $2–$20 band at 125 bps being *cheaper*
  than $20–$200 at 228 is not explained here. The largest-outflow/largest-inflow
  heuristic attributes one account pair to the trade, and larger trades move more
  accounts — an aggregator hop or a separate fee account could be read as cost.
  Until that is understood, the 125 should not be leaned on; the finding that
  survives it is the **1,521 at the bottom**, which is an order of magnitude
  clear of everything else.
- **One hour, one venue, one regime.** The same caveat every note here carries.
- **A leg, not a round trip.** Double for a round trip, as the original did.
- **`err = ''` excludes failed transactions**, which burn a fee against no
  notional. Including them would drag every median toward infinity, and
  LEARNINGS 7 records 35 of 97 migrations in a sampled hour being failures — but
  a trader who pays for a failed transaction really has lost that money, so the
  true cost of *attempting* to trade is above this.

## Reproducing

```bash
radar cost --from "2026-08-25 04:00:00" --to "2026-08-25 05:00:00"
```

The statement it issues is carried at
[`queries/0019-round-trip-cost-by-notional.sql`](queries/0019-round-trip-cost-by-notional.sql).
