<!-- SPDX-License-Identifier: Apache-2.0 -->
# 0008 — The launch block gives the bundle away

**Date:** 2026-08-25
**Store:** the live guardian VPS recorder, slots 441,040,080 – 441,520,140 (~2 days)
**Source:** CryptoHouse `solana.token_transfers`, one query per batch of forty mints
**Status:** **superseded in its headline by
[`0024`](0024-the-spike-became-a-hump-and-the-signal-moved.md)** (2026-09-03),
which re-ran it on 17,497 launches instead of 80 per population. The
coordination signal is real; the magnitude and the shape were not. The
qualifications are in the block below, and this note's own warning is what
caught them.

> **Superseded on 2026-09-03 by
> [`0024`](0024-the-spike-became-a-hump-and-the-signal-moved.md), which re-ran
> this on 17,497 launches instead of 80 per population.** Two things did not
> survive, and this note's own warning is what caught them.
>
> - ~~68% of instantly-graduating launches have exactly six recipients~~ —
>   **25.1%** [19.7–31.4]. The enrichment over the 5.5% control is real; the
>   magnitude was wrong by 2.7×.
> - ~~"a spike at six and holes on both sides"~~ — **the upper holes were
>   sampling, not structure.** At n=207 the instant column carries 11.1%, 9.7%
>   and 6.3% at seven, eight and nine.
>
> The real holes are at the *bottom*, and they are stronger than anything here:
> **one to three recipients covers 70.5% of launches and graduates instantly in
> 0.02% of them.** The 11.7× predictive figure below has also moved off six to
> the **ten-to-thirteen** band. The method changed too — this note's launch block
> was a first-seen heuristic that overstates the launch population by about four
> times, and 0024 joins on the `create`/`create_v2` discriminators instead.
>
> Read this note for the argument and 0024 for the numbers.

[0006](0006-the-graduation-table-was-empty-for-a-structural-reason.md) found that
graduations are bimodal: 27–39% complete within three slots of launch, which
means the whole bonding curve was bought by capital committed before the token
existed. That is a bundle, and it was visible only *after* the fact.

This asks whether it is visible **at launch time**, in the block itself.

## Method

Three populations, eighty launches each, from the live store:

- **instant** — graduated within three slots of launch (734 available)
- **organic** — graduated later (1,173 available)
- **control** — never graduated at all (67,366 available, randomly sampled)

For each, one measurement: how many distinct accounts received the token inside
its own launch block. Not "buyers" — these are token accounts, and resolving them
to owners is a join this did not do.

The control is the part that makes the rest mean anything. Without it, "68% of
instant graduations have six recipients" is compatible with 68% of *all* launches
having six recipients, which would make it worthless.

## The result

| recipients in launch block | never graduated | organic | instant |
|---|---|---|---|
| exactly six | 5% | 16% | **68%** |
| five to seven | 12% | 30% | **88%** |

The full distributions are the more convincing form:

```
buyers           instant           organic            control
     1                 -                 -            16 (20%)
     2             0 (0%)            3 (4%)           32 (40%)
     3             0 (0%)          15 (19%)           14 (18%)
     4             0 (0%)           8 (10%)            4 (5%)
     5           16 (20%)           8 (10%)            3 (4%)
     6           54 (68%)          13 (16%)            4 (5%)
     7             0 (0%)            3 (4%)            3 (4%)
     8             0 (0%)            3 (4%)            1 (1%)
     9             0 (0%)            3 (4%)            3 (4%)
   10+            10 (12%)         24 (30%)            4 (5%)
```

An ordinary launch has one to three recipients — 78% of the control. An instantly
graduating one has **six**, with **no observed cases at two, three, four, seven,
eight or nine**. A distribution with a spike at six and holes on both sides of it
is not a market. It is a tool with a default setting.

## What it is worth as a predictor

Against the store's own base rates — 72,193 launches, 734 instant graduations,
1,173 organic:

| signal | fires on | P(graduates) | P(graduates instantly) |
|---|---|---|---|
| exactly six recipients | 5.8% of launches | 16.3% — **6.2× base** | 11.9% — **11.7× base** |
| five to seven | 13.1% | 10.6% — 4.0× | 6.8% — 6.7× |

Base rates for comparison: 2.64% of launches graduate at all, 1.02% instantly.

## This is a reason to stay away, not a reason to buy

A signal that makes graduation six times likelier reads like an entry. It is the
opposite, and getting this backwards would be expensive.

The same observation predicts *instant* graduation at **11.7×**. An instant
graduation means the curve was bought out by people who were ready before the
token existed; they hold the supply, and the remaining role for a later buyer is
to be who they sell to. The graduation is real and the opportunity is already
spoken for.

This is the plan's thesis with a number attached: the realistic edge is **not
buying traps**, not finding rockets. `radar-graph` is built to refuse, and its
`Coordination::Likely` verdict is a disqualification.

## What this does not show

**It does not say what the six accounts are.** Six is the count of distinct
recipients, and no attempt was made to resolve them to owners or to check whether
they share a funder. The funding graph — the expensive half of coordination
detection, needing the labelled exchange-address set that does not exist yet —
would say whether these are six wallets or one person's six wallets. The signal
measures without it, which is why it was worth building first.

**Six is a tool's default, not a law.** The number will move when whoever is
running this changes their configuration, and the detector will go quiet without
saying so. What should survive is the *shape* of the argument — a spike with
holes either side — rather than the constant. Re-running this is how that gets
noticed.

**Eighty per population, one two-day window.** Same limits as
[0007](0007-does-creator-history-predict-anything.md), and the same remedy:
re-run as the store grows.

**The control is of launches that had not graduated by the time it was taken.**
Some will graduate later. That contaminates the control in the direction of
*understating* the difference, so it does not threaten the finding — but it is
not a clean "never graduates" population, and calling it one would overclaim.
