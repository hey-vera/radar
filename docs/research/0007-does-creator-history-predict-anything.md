<!-- SPDX-License-Identifier: Apache-2.0 -->
# 0007 — Does creator history predict anything?

**Date:** 2026-08-25
**Command:** `radar study --store <store>`
**Store:** the live guardian VPS recorder, slots 441,040,080 – 441,520,140 (~2 days)
**Status:** measured, and the first evidence the creator signal is real —
prior organic graduation predicts a materially higher future rate, on intervals
that do not overlap. **One regime.** The confound the note names itself is time:
both halves of every band sit inside the same two days. Re-run weekly, and watch
the prior-coverage line.

This is the question the whole system exists to answer. `creator_edge` gates on
organic graduation history because that is the *plausible* rule, not because
anyone had measured that it works. Research
[0005](0005-first-end-to-end-decision-pass.md) said so plainly: *"Nothing about
whether `creator_edge` is a good strategy. It has not proposed a single trade."*

It is now measurable, because [0006](0006-the-graduation-table-was-empty-for-a-structural-reason.md)
fixed the capture bug that had left the store with four graduations instead of
roughly fifteen hundred.

## Method, and the trap it avoids

The easy version of this study answers itself: compute each creator's graduation
rate over all their launches, then check whether creators with high rates have
launches that graduated. That correlates perfectly and means nothing, because
the same events sit on both sides.

So the record is split at a **pivot slot**, and the two halves never touch:

- The **prior** is what was knowable at the pivot — launches at or before it,
  scored *only* by outcomes measured at or before it. A launch that had happened
  but had not yet been measured contributes nothing, because at the pivot nobody
  knew.
- The **outcome** is what those same creators' *later* launches went on to do.

A creator needs at least five launches before the pivot and at least one after.
That excludes almost everyone, and it is the price of asking honestly.

## The result

```
store spans   : slot 441040080 .. 441520140
pivot         : slot 441412045
creators      : 638 with >= 5 launches before the pivot and at least one after
prior coverage: 12553 of 16423 pre-pivot launches had been measured by then
later launches: 9730 of which 100 graduated organically
base rate     : 1.02%

  KNOWN AT PIVOT                  CREATORS  LAUNCHES   ORGANIC  RATE
  no organic graduation known          552      7328        60  0.81% [0.64% – 1.05%]  (0.79x)
  exactly one                           64      1123        18  1.60% [1.02% – 2.52%]  (1.57x)
  two or more                           22      1279        22  1.72% [1.14% – 2.59%]  (1.69x)
```

**The direction is the one `creator_edge` assumes, it is monotonic across three
bands, and the intervals of the top and bottom groups do not overlap** —
`[1.14%, 2.59%]` against `[0.64%, 1.05%]`. A creator who had already graduated a
token organically went on to do it again at roughly **twice** the rate of one who
had not.

Intervals are Wilson score at 95%, chosen because these are rare events over
modest samples — 22 graduations in 1,279 launches — where the textbook normal
interval misbehaves and will happily return a negative lower bound.

## Controlling for launch frequency

The obvious confound: a creator with four hundred launches is likelier to have
graduated *something* than a creator with five, purely by having more attempts.
So "has a prior organic graduation" partly encodes "launches a lot", and if
launch frequency itself predicts the later rate, the headline could be that in
disguise.

Holding frequency roughly fixed and comparing inside each band:

```
  PRIOR LAUNCHES      WITHOUT PRIOR GRADUATION       WITH PRIOR GRADUATION
  5-9 launches       1.43% [0.91%-2.25%] n=228   9.09% [3.95%-19.58%] n=15  separated
  10-29 launches     1.20% [0.85%-1.72%] n=216    2.31% [1.22%-4.34%] n=30
  30+ launches       0.33% [0.19%-0.58%] n=108    1.32% [0.91%-1.94%] n=41  separated
```

**The direction holds in all three bands, and two of the three separate at 95%.**
The middle band leads without separating, which is a statement about how many
creators are in it rather than a contrary result.

Consistency across bands is evidence in its own right. Each band is an
independent comparison, and three landing the same way is unlikely if there is
nothing there — more informative here than any single band's interval, given how
few creators each one holds.

## The confound runs the other way, which is a finding of its own

Read down the **without prior graduation** column. These are creators about whom
nothing good was known, separated only by how much they launch:

| Prior launches | Later organic rate |
|---|---|
| 5–9 | 1.43% |
| 10–29 | 1.20% |
| 30+ | **0.33%** |

**Creators who launch more graduate less, per launch** — more than four times
worse at the top band than the bottom. Launch volume is a *negative* signal.

That settles the confound in the opposite direction from the one feared. Prolific
creators have more chances to have graduated something, and are worse per
attempt, so their over-representation in the "has graduated" group was
**suppressing** the headline result rather than manufacturing it. Controlling for
frequency strengthens the finding instead of dissolving it.

It also corroborates [0004](0004-measured-launch-base-rates.md) from the other
end. That note found a creator launching 42 tokens in half an hour with 98% of
them stillborn, and argued creator launch rate was "cheap to compute and already
discriminating". This is the same fact measured against outcomes across 638
creators rather than three.

## Turning the gradient into a threshold

The banded table shows launch volume is a negative signal but not where to draw a
line. Read finely, over every creator rather than split in two:

```
  LAUNCHES     CREATORS   LAUNCHES   ORGANIC  RATE
  5-9               243       1313        23  1.75% [1.17% - 2.61%]
  10-19             165       1809        25  1.38% [0.94% - 2.03%]
  20-49             155       2431        20  0.82% [0.53% - 1.27%]
  50-99              44       1562        14  0.89% [0.53% - 1.50%]
  100-249            27       1530        12  0.78% [0.45% - 1.37%]
  250+                4       1085         6  (too few creators)
```

It **falls and then flattens**: roughly halving between 5–9 and 20–49, and flat
from there. The extremes of this finer split do not separate at 95% — which is
worth stating, because the earlier 0.33% figure was the *conjunction* of prolific
**and** never-graduated, not launch volume alone. Launch volume alone is weaker
than that number implied.

Choosing a threshold by looking at a curve picks whichever boundary flatters the
sample. Asking the same question of every candidate instead:

```
  CUT       PER DAY           BELOW (quieter)      AT OR ABOVE (busier)
  10            5.8     1.75% [1.17% - 2.61%]     0.91% [0.73% - 1.14%]  separates
  15            8.7     1.53% [1.12% - 2.10%]     0.85% [0.67% - 1.09%]  separates
  20           11.6     1.53% [1.16% - 2.03%]     0.78% [0.60% - 1.03%]  separates
  30           17.4     1.48% [1.16% - 1.89%]     0.68% [0.50% - 0.94%]  separates
  50           29.0     1.22% [0.97% - 1.55%]     0.76% [0.54% - 1.08%]
  100          58.1     1.15% [0.93% - 1.43%]     0.68% [0.44% - 1.09%]
```

**Four of six separate at 95%, spanning roughly 6 to 17 launches a day.** That
range is the finding, not any single cut in it. `creator_edge` uses **10 a day**,
which is inside the supported range and is deliberately *not* one of the tested
points — every cut across the range separated, so the rule does not depend on
picking the one that fitted best. The insensitivity is the argument for the
number.

The rate is computed over each creator's own observed span, and is `None` below
six hours of activity: under that, one busy minute reads as a thousand launches a
day and the number says more about when the creator was first seen than how they
behave. Absent rather than zero, because zero would read as the quietest possible
creator and pass a threshold it was never tested against.

### What it does to the decision lane

Against 1,372 recent candidates, the new refusal fires on **500** of them, and
candidates reaching the paid tier fall from 16 to 8. A third of recent launches
coming from creators above the threshold is not surprising: prolific creators are
a small minority of *creators* and a large share of *launches*, so any per-launch
filter meets them disproportionately.

## What this is not

**One window, of a few days.** Two days of chain, one pivot. The next month
could look different, and a rule validated on a single window is a rule fitted to
one market regime.

**Small where it matters.** The strongest band is 22 creators. The whole study
rests on 100 organic graduations.

**Not a claim about returns.** Graduation is the only unambiguously good outcome
the store records, and it is still a proxy. Nothing here says a token that
graduates makes money for someone who bought it.

## Why the first run of this was wrong, and what fixed it

The first version chose its pivot at the midpoint of the recorded slot range. The
store holds launches from well before its first outcome measurement — every
measurement is at slot 441,303,950 or later, because 0006's repair moved the old,
wrong-schema outcome files aside — so the pivot landed **before anything had been
measured at all**. Every creator had an empty prior, and the table read:

```
  no organic graduation known          613     13016       132  1.01% vs 1.01% base  (1.00x)
```

Which looks exactly like the finding *"creator history predicts nothing"*, and is
not that finding. It is an empty column.

Two things now make that unreachable. The default pivot is taken from the
**outcome coverage** rather than the slot range, and the study reports how many
pre-pivot launches had actually been measured — refusing to print the grouping at
all when the answer is none:

> *This is the difference between 'creator history predicts nothing' and 'we had
> not looked yet', and they are not distinguishable from the table alone.*

That near-miss is the same shape as [LEARNINGS](../../LEARNINGS.md) entry 7: a
gap in measurement wearing the costume of a result.

## What to watch

Re-run this weekly. The numbers that matter are the **prior coverage** line — the
study is only as good as the fraction of pre-pivot launches that had been
measured — and whether the direction keeps holding in all three frequency bands
as they fill out. If bands start reversing as the sample grows, the effect was
noise. If the middle band separates too, the case is much stronger.

The remaining confound nobody has touched: **time**. Both halves of every band
are measured over the same two days, so a market-wide swing affects them equally
— but nothing here shows the effect persists across regimes, and two days is one
regime. That needs weeks, not a better method.
