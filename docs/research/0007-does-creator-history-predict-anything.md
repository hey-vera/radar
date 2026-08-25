<!-- SPDX-License-Identifier: Apache-2.0 -->
# 0007 — Does creator history predict anything?

**Date:** 2026-08-25
**Command:** `radar study --store <store>`
**Store:** the live guardian VPS recorder, slots 441,040,080 – 441,520,140 (~2 days)

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

## What this is not

**One window, of a few days.** Two days of chain, one pivot. The next month
could look different, and a rule validated on a single window is a rule fitted to
one market regime.

**Small where it matters.** The strongest band is 22 creators. The whole study
rests on 100 organic graduations.

**Uncontrolled for launch frequency.** This is the most likely confound and it is
not addressed. A creator with four hundred launches has four hundred chances, and
prolific creators are over-represented among those with any graduation at all.
The honest next step is to compare creators at similar launch rates, which the
current study does not do.

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
measured — and whether the separation survives as the top band grows past 22
creators. If the intervals begin to overlap as the sample grows, the effect was
noise. If they stay apart, the next question is the launch-frequency control.
