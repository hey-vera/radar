<!-- SPDX-License-Identifier: Apache-2.0 -->
# 0005 — The first end-to-end decision pass

**Date:** 2026-08-23
**Command:** `radar consider --store <store>`
**Store:** the live guardian VPS recorder, copied at slot 441068845
**Status:** a run, not a measurement. It establishes that the decision lane
executes end to end. It establishes nothing about whether `creator_edge` works —
see its own "What it does not establish" — and that question was answered later
by [`0007`](0007-does-creator-history-predict-anything.md).

## What was run

The whole decision lane, against tokens this instance actually recorded:
assemble candidates at the watermark, apply `creator_edge`, spend on exit
analysis only where a paid look could change an answer, put survivors through
the risk kernel.

## What it found

```
watermark    : slot 441068845
launches     : 3924 recorded
creators     : 1518
considering  : 3924 launched within 216000 slots

free tier — why 3924 candidates were passed over:
    3924  NoExitSimulated
    2354  CreatorUnproven
     289  CreatorMostlyStillborn
    1570  CreatorNeverGraduated
    3924  NoPrice
    2014  InputsTooStale

0 candidate(s) fail on nothing a paid look cannot resolve.
Spending on exit analysis would change no answer, so nothing is spent.
```

Narrowing to the freshest cohort (`--window 6000`, roughly the last 40 minutes
of chain) changes the shape but not the conclusion: 594 candidates, 490 with a
creator below the sample floor, 104 whose creator has never graduated a token,
2 spammers. Nothing survives.

## Three things this establishes

**1. The tiering is real, and it is falsifiable.** Zero paid calls were made,
because no paid call could have changed an answer. That is the tiered
investigation from the plan working as designed rather than as documented — and
the report says so in a form that could have said otherwise.

**2. Not one creator in the sample has graduated a token.** 1,518 creators, 104
of them above the five-measurement sample floor, zero graduations. This is
consistent with the ~1% population base rate over a three-hour window, and it is
the first measured confirmation of it from Radar's own data rather than from
someone else's report.

**3. `CreatorUnproven` dominating is the sample age, not a threshold problem.**
Outcomes are measured at checkpoints — one hour, six hours, a day after launch.
A store holding three hours of chain has measured almost nothing at the second
checkpoint and nothing at all at the third, so most creators cannot yet have a
record. This number should fall steadily as the recorder runs, and if it does
not, something is wrong with the outcome pass rather than with the creators.

## What it does not establish

Nothing about whether `creator_edge` is a *good* strategy. It has not proposed a
single trade, so it has no track record to judge — which is the correct state
for a strategy whose inputs do not exist yet, and exactly why the recorder
shipped first.

The honest reading is: the machine works, the data is three hours old, and the
question the machine exists to answer needs weeks of it.

## The number to watch

`CreatorUnproven` in the `--window 6000` cohort. Today it is 490 of 594. When it
falls below roughly half, creator records have accumulated enough that the rest
of the strategy is being exercised rather than short-circuited, and the first
real question — does creator history predict anything — becomes answerable.
