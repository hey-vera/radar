<!-- SPDX-License-Identifier: Apache-2.0 -->
# 0017 — A control that could have been traded

**Date:** 2026-08-30
**Store:** the live guardian VPS recorder, 1,092 proposals and 121,810 untouched
tokens priced
**Status:** measured. **No edge found.** Two of four comparable strata are
uninformative for a reason this note names and does not solve.

> **Correction, 2026-08-30.** The limitation this note names below — that
> 64–91% of short holds return exactly zero despite the pairing requiring a
> trade — **has been explained, and it was a defect.** `Outcome::fills` is folded
> with `saturating_add` across price windows overlapping by five of their six
> hours, so it grows on every hourly pass whether or not anything trades. The
> gate keyed on it establishes nothing. See
> [`LEARNINGS`](../../LEARNINGS.md) entry 19.
>
> The gate now uses `last_transfer_slot`, a maximum that cannot be inflated by
> re-reading. **Every figure below was produced with the broken gate and should
> be re-measured** with `radar control` before being cited.

## What this replaces

[`0014`](0014-the-control-was-entirely-tokens-nobody-could-sell.md) compared
Radar's proposals against its own refusals and found the comparison unusable:
all 606 scoreable refusals were `CapacityBelowFloor`, so the control was
composed entirely of tokens Radar had measured and found it could not sell. It
concluded that a real control needs the strategy to refuse something *after* the
exit probe, which it does not do.

**That conclusion was too pessimistic**, and
[`0016`](0016-the-entry-was-a-bid-and-the-exit-was-a-mid.md) is why. 0016 found
that the selection's entry is a **sell quote** and its exit a **realised fill** —
two instruments, worth at least 128 bps of spurious return. The fix for that also
removes the obstacle here.

**Price both cohorts from realised fills, and a population control becomes
available.** A token Radar never examined has no quote, which is exactly why the
quote-based measurement could not include one. It does have outcome
measurements — and so does every token Radar decided on.

## Method

Both cohorts priced **outcome to outcome**: the same instrument on each side and
on both ends.

Matched on the two confounders already known to dominate this data:

- **Token age at entry.** `creator_edge` acts around forty minutes after launch.
  A population token priced from its first checkpoint is being measured at a
  different point in its life.
- **Holding period.** [`0011`](0011-graduation-predicts-volatility-not-profit.md)
  states it outright — `last_price` depends on which checkpoint a token was last
  measured at, and the population median moves from −1,340 to −863 bps on that
  alone.

Only strata where **both** cohorts clear twenty rows contribute. A stratum only
the selection reaches would compare it against itself.

Tokens Radar **refused** are excluded from the control as well as from the
selection. A token its rules touched is not an untouched token, and admitting
refusals is precisely how 0014's comparison went wrong.

## The result

```
age    hold      sel n    ctl n   sel p25   sel med  sel p75   ctl med      edge
<90m   <6h         367    40930         0         0        0         0         0
<90m   <24h         94     3156     -2260     -1272     -198      -475      -797
90m+   <6h         341    72622         0         0        0         0         0
90m+   <24h        289     4954     -2062      -466      764      -575       109
```

**Median edge across the four comparable strata: 0 bps. One of four favours the
selection.**

## The two strata that can discriminate disagree

The `<6h` strata are uninformative, and the diagnostic says why:

```
Share of each cohort that returned exactly zero:
  <90m   <6h    selected  6430 bps   control  8386 bps
  <90m   <24h   selected     0 bps   control   671 bps
  90m+   <6h    selected  7448 bps   control  9057 bps
  90m+   <24h   selected    34 bps   control     8 bps
```

Sixty-four to ninety-one per cent of the short-hold strata return **exactly
zero**. A median over that is a report about the point mass, not about the
market, and it is why a bare median of 0 must not be read as "the two cohorts
performed identically".

That leaves two strata with real distributions, and **they point opposite ways**:

| stratum | selected | control | edge |
|---|---|---|---|
| `<90m` age, `<24h` hold | −1,272 | −475 | **−797** |
| `90m+` age, `<24h` hold | −466 | −575 | **+109** |

One says the selection did meaningfully worse than an untouched token; the other
says it did marginally better. On 94 and 289 proposals respectively, in one
regime, that is not a finding in either direction. **It is a null result reached
with a control that could actually have been traded**, which is more than 0014
was able to say.

Both cohorts are deeply negative in absolute terms, before the measured 850 bps
round trip.

## The limitation this note does not solve

**Why do 64–91% of short holds return exactly zero, when the pairing already
requires a trade between the two observations?**

The measurement admits a pair only when the exit observation has strictly more
fills than the entry — so something changed hands. A price that did not move
across a recorded trade is not obviously impossible on a bonding curve at very
low volume, but at a 10^18 scale it is surprising at this rate, and **it is not
understood**.

Two readings, wanting opposite responses:

- The venue really does transact at an unchanged quantised price for most short
  intervals, in which case these strata are a genuine feature and the comparison
  should simply lean on the longer holds.
- Something upstream carries a stale `last_price` forward while `fills`
  advances, in which case the short-hold strata are contaminated and so is any
  figure drawn from them.

This note does not distinguish them, and says so rather than picking the
convenient one. **It is the next measurement worth taking**, and until it is
taken the `<24h` strata carry the whole result.

## What this does not establish

- **Not causal.** Radar chose its cohort on creator history; the population did
  not choose itself. A difference is evidence about the selection *rule* under
  the conditions it ran in.
- **The control is untouched, not equivalent.** Matching on age and hold does
  not match on liquidity, creator, or launch shape. A token Radar never examined
  may be unexaminable for reasons that correlate with return.
- **One instance, one venue, one regime**, and 1,092 priced proposals.
- **Gross.** The measured round trip is 850 bps and nothing here is net of it.

## Reproducing

```bash
radar control --store <dir>
```

## Two false starts, both recorded

Worth carrying because each produced a plausible number that meant nothing.

**The first run reported a median of exactly 0 bps in every stratum, on both
sides, over 201,465 control tokens.** An `Outcome` reports what has happened *so
far*, so a token that stopped trading repeats the same `last_price` at every
later checkpoint — and on this venue most tokens die quickly. Pairing on time
alone made the majority of both cohorts a flat zero. 0011 already shows the
shape: its p75 for never-graduated tokens is exactly 0.

**The fix for that was also wrong, and its own test caught it.** Requiring the
exit to have more fills than the entry is right; taking the *last* such
observation is not, because that happily includes checkpoints recorded long after
trading stopped — pricing the token at a stale figure while stretching the hold,
biasing the very strata the design exists to separate. The exit is now the
**earliest** observation reaching the highest fill count, which is the last
moment a trade actually happened.

Both are the shape [`LEARNINGS`](../../LEARNINGS.md) records repeatedly: a
broken instrument does not produce less data, it produces a selected sample, and
a selected sample supports confident conclusions about the selection.
