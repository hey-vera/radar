<!-- SPDX-License-Identifier: Apache-2.0 -->
# 0020 — The exit-rule question cannot be answered from this data

**Date:** 2026-08-30
**Store:** the live guardian VPS recorder, 206,539 mints with a usable price path
**Status:** measured. **No rule beats holding** — and the reason is that the data
can see movement on only 4% of paths, which is a weaker result than it looks.

## The claim being tested

[`0009`](0009-what-a-token-actually-does-to-your-money.md) concluded **"the exit
rule is not where the edge is"**, on the strength of a take-profit table. Its own
diagnosis of why those failed:

> the 37–48% that miss the target do not miss it by a little. Their `HELD` is the
> p10 of −94.9%, and no achievable take-profit on the winners pays for that.

**That diagnosis is the case for a stop, and 0009 never tested one.** A
take-profit truncates the right tail; only a stop truncates the left one, and the
left one is the stated problem. The conclusion could still be right — it did not
follow from the evidence given for it.

## The result

```
target/stop           n   pess p25   pess med    opt med   target     stop     held
—/—              206539          0          0          0        0        0   206539
—/-1000          206539          0          0          0        0     8365   198174
—/-2500          206539          0          0          0        0     6405   200134
—/-5000          206539          0          0          0        0     4302   202237
+1000/—          206539          0          0          0     2082        0   204457
+2500/—          206539          0          0          0     1813        0   204726
+5000/—          206539          0          0          0     1627        0   204912
+1000/-1000      206539          0          0          0     1528     8290   196721
+2500/-2500      206539          0          0          0     1372     6338   198829
+5000/-2500      206539          0          0          0     1241     6345   198953
```

**No rule beats the baseline**, on the pessimistic bound or the optimistic one.
0009's conclusion survives the test it never ran.

## Why that is weaker than it sounds

Look at the right-hand columns. Under the loosest rule, **8,365 of 206,539
positions are stopped and 2,082 reach a target** — 4.1% and 1.0%. Everything else
is held. **The measurement can see post-entry movement on about four per cent of
paths**, and on the other ninety-six it has nothing to compare.

Two things cause that, and only one is the market.

**The venue.** Most pump.fun tokens trade a handful of times and stop, which
[`0011`](0011-graduation-predicts-volatility-not-profit.md) already shows: its
p75 held-to-end for never-graduated tokens is exactly 0. A price that never moves
again offers an exit rule nothing to do.

**The instrument, and this is the part worth stating carefully.** `peak_price`
and `trough_price` are folded with `max` and `min` **from launch**, not from the
entry checkpoint. The first version of this measurement compared them against the
entry price directly, and reported that 69% of positions hit a +10% target — a
figure produced entirely by peaks that were set *before* the position existed.
Look-ahead wearing the shape of a fill.

The correction is to count a threshold as crossed only when a **new** extreme is
set after entry. That is right, and it costs the measurement most of its sight:
**a token that falls 50% and recovers, without exceeding its pre-entry range, is
invisible.** The 4% is what remains visible, not what happened.

## So the honest verdict

**This data cannot answer the exit-rule question**, and the previous answer to it
was not evidence either.

What can be said: on the movement this store can observe, no stop and no
take-profit beats holding, and stops do not rescue the losing tail the way 0009's
diagnosis implied they might. That is worth having — it is the first time the
stop has been tested at all — and it is not the same as knowing an exit rule
cannot help.

## What would answer it

**Intra-checkpoint price paths.** The store records running extremes at hourly
checkpoints; an exit rule is a question about the order of prices *within* those
gaps. Nothing folded from launch can recover it.

Concretely, one of:

1. **Record extremes since the previous checkpoint** as well as since launch — a
   new pair of columns, added optionally so files written before them stay
   readable ([`LEARNINGS`](../../LEARNINGS.md) entry 17). This makes each
   interval's movement visible without any new data source, and it is the cheap
   option.
2. **Record the fill series** for tokens under consideration, which is exact and
   expensive.

Option 1 is a schema addition to the outcome pass and would make this
measurement meaningful on the next window of data. It does not repair the
history.

## What this does not establish

- **Not a refutation of 0009, and not a confirmation.** It agrees with 0009's
  conclusion, on 4% of the population, by a route 0009 did not take.
- **Gross.** Nothing charges the round trip, and a rule that exits early pays it
  more often than the baseline does — which can only make the rules look worse.
- **The unselected population.** These are all priced mints, not Radar's
  proposals. An exit rule fitted on the selection is a different question, and a
  smaller cohort.
- **The tie-break is unresolved by construction.** Where a target and a stop are
  both crossed inside one interval the order is unrecoverable, so every figure is
  reported as a pair of bounds. Here they agree, because so little moves.

## Reproducing

```bash
radar exits --store <dir>
```
