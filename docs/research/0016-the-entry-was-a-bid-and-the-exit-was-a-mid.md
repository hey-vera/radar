<!-- SPDX-License-Identifier: Apache-2.0 -->
# 0016 — The entry was a bid and the exit was a mid

**Date:** 2026-08-30
**Store:** the live guardian VPS recorder, 4,534 decisions, 2,577 paired
**Status:** measured. The correction is a **floor**, not an estimate, and the
premise that makes it one is measured beside it.

## What this corrects

[`0014`](0014-the-control-was-entirely-tokens-nobody-could-sell.md) reports
Radar's selection at a **gross median of +21 bps** and calls it "noise around
zero". That figure is the move from a decision's `entry_price` to a later
`Outcome::last_price`, and those two numbers are produced by different
instruments:

| | source | what it is |
|---|---|---|
| entry | smallest rung of `quote_sell` ([`consider.rs`](../../crates/radar-cli/src/consider.rs)) | a **sell quote** — a bid, net of the router's fee and pump.fun's |
| exit | `argMax(lam / tok, (ts, sig))` ([`prices.rs`](../../crates/radar-backfill/src/prices.rs)) | realised fills, **buys and sells pooled** — near the mid |

A bid measured against a mid is positive **before the market has moved at all**.

[`selection.rs`](../../crates/radar-research/src/selection.rs)'s module
documentation states *"Both prices come from the sell side"*. That is true of the
entry and false of the exit, which makes it a claim documented more strongly than
its enforcement — [`LEARNINGS`](../../LEARNINGS.md) entry 9's shape, landing on
the one number the project exists to produce.

## Method

`radar basis` pairs each decision carrying a quote with the realised price
observed **nearest to it in time**, in either direction, and buckets by that gap.

Nearest rather than latest is the whole method: `selection` asks what *followed* a
decision, this asks what was *true at the same moment*. Symmetric rather than
forward-only because taking later observations alone would fold this market's
downward drift into the answer, which is the contamination being measured.

The two components cannot be separated inside one bucket. They can be separated
**across** buckets, because they behave differently with time: real movement
grows with the gap, and an instrument difference does not.

## The result

```
gap         pairs        p25     median        p75
<=10m           0          -          -          -
<=15m          23       -265        -20        723
<=20m        1756       -110        128        514
<=30m           2      -1391        140        140
<=1h          470       -340         72        325
<=3h          100       -658         46        268
>3h           226      -1238        -84        282
```

Across the four buckets that clear the thirty-pair floor the median falls
monotonically:

| gap | pairs | median basis |
|---|---|---|
| **≤20m** | **1,756** | **+128** |
| ≤1h | 470 | +72 |
| ≤3h | 100 | +46 |
| >3h | 226 | −84 |

**A pure instrument difference would be flat across these buckets, and a pure
market effect would grow with the gap.** This does neither: it is a positive
constant with something negative added that grows with time. The negative term is
this market's drift, which [`0011`](0011-graduation-predicts-volatility-not-profit.md)
measures at a population median of −863 bps held to last observation.

## Why +128 bps is a floor and not an estimate

The drift subtracts. So the basis measured at *any* positive gap is **less than**
the instrument difference alone, and the tightest measurable bucket gives a lower
bound rather than a point estimate.

The premise — that the drift is negative in this sample — is not assumed from
0011. It is computed in the same data as the conclusion and carried beside it as
`drifts_down`, so a reader can refuse it. Where the basis does not fall with the
gap, the floor claim is withheld rather than made anyway.

**An independent check lands on the same number.** pump.fun charges roughly 1% a
leg. A bid sits below a mid by about the fee plus half the spread, which puts the
expected gap a little above 100 bps. The measurement says at least 128. Two routes
to one number, neither derived from the other.

## What this does to the headline

0014's gross median is **+21 bps**. The correction is **at least 128 bps** and it
is subtracted.

```
                              0014      corrected (upper bound)
gross median                   +21                        <= -107
net of the 850 bps round trip -829                        <=  -957
```

**The selection's gross median is not "noise around zero". It is negative, and
the artefact was six times the size of the signal it was hiding.**

This does not make the selection *harmful* — that would need the control 0014
showed is unusable, and building one is still owed. It removes the reading that
the selection is roughly break-even before costs.

## What this does not establish

- **The tightest bucket with any data at all reads −20 bps, and it is below the
  floor.** Twenty-three pairs, with a p25 of −265 and a p75 of +723 — a spread
  that admits almost anything. It is excluded by the cohort rule rather than by
  choice, and **its selection mechanism is not understood**: the cadence puts
  nearly everything at fifteen to twenty minutes, so whatever put 23 pairs closer
  is unexplained. A cohort of 23 selected by an unknown mechanism is exactly the
  shape LEARNINGS 7, 10 and 11 record. If that bucket fills in and holds near
  −20, this note's finding reverses, and that is the measurement most worth
  taking next.
- **The floor is not a point estimate.** The true instrument difference is at
  least 128 bps and this method cannot say how much more.
- **It does not decompose fee from spread.** A realised fill already has the
  protocol fee inside it. The figure is the whole gap between two instruments,
  which is what `selection` needs subtracted, not an account of why.
- **The cadence sets the resolution.** The outcome pass runs at `:17` and `radar
  consider` at `:37`, so no pair can be closer than about twenty minutes. Moving
  them nearer together would tighten this bound directly, and is the cheapest
  available improvement to it.
- **One instance, one venue, one regime.** The caveat every note here carries.

## What should follow

1. **Stop comparing across instruments.** Price the exit sell-side, or record a
   contemporaneous mid at decision time, so `selection` compares like with like
   rather than subtracting a correction after the fact.
2. **Move the two passes closer together.** A cron change tightens the floor.
3. **The control 0014 still owes.** An age- and calendar-matched population
   cohort, which is a join against data already recorded.

## Reproducing

```bash
radar basis --store <dir>
```

Carried as a command rather than described, for the reason
[`0013`](0013-a-repeat-launcher-in-the-block-predicts-a-deader-token.md) gives:
a note whose query exists only in prose is one nobody can re-run, and re-running
is the only way a number in a note gets checked.
