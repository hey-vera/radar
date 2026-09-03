<!-- SPDX-License-Identifier: Apache-2.0 -->
# 0024 — The spike became a hump, and the signal moved off six

**Date:** 2026-09-03
**Store:** the live guardian VPS recorder, 483,629 launches and 14,336 graduations
**Chain:** 17,497 pump.fun launches, 2026-08-25 00:00–12:00 UTC
**Query:** [`queries/0024-launch-block-recipients.sql`](queries/0024-launch-block-recipients.sql)
**Status:** measured. **[`0008`](0008-the-launch-block-gives-the-bundle-away.md)'s
headline does not survive.** The enrichment at six is real and about five times
smaller than reported; the spike-with-holes shape is gone; and the strongest band
is no longer six.

> 0008 wrote its own warning, and it has come true:
>
> > "**Six is a tool's default, not a law.** The number will move when whoever is
> > running this changes their configuration, and the detector will go quiet
> > without saying so. What should survive is the *shape* of the argument — a
> > spike with holes either side — rather than the constant. Re-running this is
> > how that gets noticed."
>
> This is that re-run. The constant held and **the shape did not**, which is the
> opposite of what that paragraph expected to lose.

> **Corrected, 2026-09-03, the same day.** The first run of this note counted
> **failed** launch transactions as launches. CryptoHouse's `token_transfers`
> carries rows for failed transactions, and the query did not join
> `solana.transactions` to filter on `status`. That is the same mistake
> [`0006`](0006-the-graduation-table-was-empty-for-a-structural-reason.md)
> records, where 35 of 97 migration instructions in an hour were in failed
> transactions and counting them overstated a label by more than a third.
>
> It was found by `radar dossier` disagreeing with this query about a specific
> mint: the query reported a launch, and the dossier reported that the mint
> account did not exist and every transaction touching it had failed. Measured
> over one hour, **218 of 2,607** launch transactions failed — 8.4%.
>
> Re-measured with the filter, **every conclusion below holds** and the figures
> move by a few tenths of a point: 18,268 launches become **17,497**, six
> recipients among instant graduations goes from 25.6% to **25.1%**, and the
> population graduation rate goes from 2.874% to **3.001%** — which is *closer*
> to the store's independently computed 2.964%. Everything below is the
> corrected run, and the carried query carries the fix.

## Why this was re-run now

0008 and [`0007`](0007-does-creator-history-predict-anything.md) are the two
signals this project has measured as real, and both were measured on a two-day
window when the store held 72,193 launches. It now holds 483,629. Anything that
publishes these numbers — a research note, the risk kernel, a public account
answering a stranger — is otherwise repeating a measurement taken at a seventh of
the present evidence.

## The method changed, and that is the first finding

**0008's launch block was a first-seen heuristic, and it was wrong.** It took a
mint's launch block to be the slot of its first observed token transfer inside
the query window. A token launched days earlier, whose first transfer *in the
window* lands in some later slot, is then counted as a launch — and an ordinary
trading block is read as its launch block.

Measured over one hour, 2026-09-02 00:30–01:30 UTC:

```
first-transfer heuristic, any mint             11,019
first-transfer heuristic, mints ending 'pump'   5,285
create / create_v2 discriminator join           1,928
```

[`0006`](0006-the-graduation-table-was-empty-for-a-structural-reason.md) measured
the store's recorded launch rate at **1,283/hour** against a chain rate of
**1,328/hour**. Only the discriminator join is in that neighbourhood; the
heuristic overstates the population by roughly four times.

So this note identifies a launch the way the recorder does — a transaction
carrying the pump.fun `create` (`181ec828051c0777`) or `create_v2`
(`d6904cec5f8b31b4`) **discriminator bytes**, never a logged instruction name
([LEARNINGS](../../LEARNINGS.md) 3), and both rather than one, because
`radar_decode::pumpfun::Instruction::is_launch` matches both and checking for
`create_v2` alone silently drops the other.

**The cross-check that the population is right:** 525 of the 17,497 launches
found on chain graduated, a rate of **3.001%**, against the store's own
independently computed **2.964%** over 483,629 launches. Two different
instruments over different populations landing within a tenth of a point is what
makes the rest of this readable.

**Three populations, and no sampling.** Every launch in the window is measured,
and the graduation label comes from Radar's own store — CryptoHouse cannot
distinguish an instant graduation from an organic one without re-implementing the
resolver 0006 fixed. 0008 sampled eighty per population; this is 17,497.

## The result

```
recipients          never graduated          organic          instant
     1            2337 ( 13.8%)         0 (  0.0%)       0 (  0.0%)
     2            5920 ( 34.9%)        20 (  6.3%)       2 (  1.0%)
     3            3906 ( 23.0%)       153 ( 48.1%)       0 (  0.0%)
     4            1040 (  6.1%)        15 (  4.7%)       6 (  2.9%)
     5            1299 (  7.7%)        25 (  7.9%)      39 ( 18.8%)
     6             930 (  5.5%)        24 (  7.5%)      52 ( 25.1%)
     7             561 (  3.3%)        22 (  6.9%)      23 ( 11.1%)
     8             348 (  2.1%)        10 (  3.1%)      20 (  9.7%)
     9             200 (  1.2%)         8 (  2.5%)      13 (  6.3%)
    10             120 (  0.7%)        13 (  4.1%)      19 (  9.2%)
    11              87 (  0.5%)         6 (  1.9%)      12 (  5.8%)
    12              62 (  0.4%)         5 (  1.6%)      10 (  4.8%)
    13              37 (  0.2%)         0 (  0.0%)       4 (  1.9%)
   14+             125 (  0.7%)        17 (  5.3%)       7 (  3.4%)
                 ------                -----            -----
       n          16,972                  318              207
    mode               2 (34.9%)            3 (48.1%)       6 (25.1%)
```

## What survived, and what did not

| | 0008 | 0024 |
|---|---|---|
| instant launches with exactly six | **68%** | **25.1%** [19.7–31.4] |
| never-graduated with exactly six | 5% | 5.5% [5.1–5.8] |
| organic with exactly six | 16% | 7.5% [5.1–11.0] |
| instant with five to seven | 88% | 55.1% [48.3–61.7] |
| holes at 7, 8, 9 in the instant column | yes | **no** — 11.1%, 9.7%, 6.3% |
| instant mode | 6 | 6 |

**The direction survives and the magnitude does not.** Six recipients is still
about five times commoner among instant graduations than among launches that
never graduated, and the two intervals are nowhere near touching. But the 68% is
a 25.1%, and a claim built on 68% is wrong by a factor of 2.7.

**The spike is a hump.** 0008's most persuasive sentence was structural rather
than statistical — *"a distribution with a spike at six and holes on both sides
of it is not a market. It is a tool with a default setting."* At n=80 the instant
column had literal zeros at 2, 3, 4, 7, 8 and 9. At n=207 the upper holes are
gone: 7, 8 and 9 carry 11.1%, 9.7% and 6.3%. **Those zeros were sampling, not
structure**, and the argument that rested on them cannot be made any more.

## The holes that are real are at the bottom, and they are the strongest thing here

The instant column still has genuine zeros — at **one and three** — and unlike
the upper ones these are not a small-sample artefact, because the band they sit
in is the largest in the data.

```
band              fires on   P(graduates)          P(instant)              vs base
one to three         70.5%      1.42%  (0.5x)    0.02% [0.00–0.06]     0.0x
exactly six           5.7%      7.55%  (2.5x)    5.17% [3.96–6.72]     4.4x
five to seven        17.0%      6.22%  (2.1x)    3.83% [3.20–4.58]     3.2x
ten to thirteen       2.1%     18.40%  (6.1x)   12.00% [9.09–15.68]   10.1x
```

Base rates over the window: **3.001%** graduate, **1.183%** graduate instantly.

**A launch block with one to three recipients covers 70.5% of all launches and
graduates instantly in 0.02% of cases** — two launches out of 12,338, with a 95%
upper bound of 0.06%. That is a stronger, broader and more stable statement than
anything 0008 made, and it points the same way the project already points:
**the reliable signal is the refusal.**

## The signal moved off six

0008's headline predictive figure was six recipients at **11.7×** base for
instant graduation. That figure still exists in this data — it has moved to a
different band.

**Ten to thirteen recipients is now 10.1× base**, where six is 4.4×. It fires on
2.1% of launches and graduates instantly 12.00% of the time.

This is exactly the failure mode 0008 named: the count is a configuration, and
whoever runs these tools has changed it. **Anything that hard-codes six is
measuring a setting that has already moved once.** What generalises is *"an
unusually large number of distinct recipients inside the launch block"*, and the
threshold is a measurement with a date on it rather than a constant.

## What this does not show

- **Recipients are token accounts, not people.** `destination` is an
  `(owner, mint)` pair. No attempt is made to resolve them to owners or to check
  whether they share a funder.
  [`0012`](0012-recipient-sets-cannot-recur-authorities-can.md) shows why the
  obvious follow-up is unavailable: two mints cannot share a destination, so
  recipient sets cannot recur across launches. **Nothing built on this number may
  imply a cabal identity it cannot see.**
- **"Never graduated" means "not in Radar's graduation table".** It conflates a
  token that never graduated with one that graduated and was not recorded. That
  contaminates in the direction of *understating* the difference, so it does not
  threaten the finding — but it is not a clean population, and calling it one
  would overclaim. Same caveat 0008 carried.
- **Organic graduations are undercounted by construction.** The window is
  2026-08-25 and the labels were read on 2026-09-03, so an organic graduation
  taking longer than nine days is scored as "never". The window was chosen old
  for exactly this reason; it does not eliminate the effect.
- **n=207 instant and n=318 organic.** Better than 0008's 80, and the reason the
  upper holes closed. It is not enough to settle the shape of the tail beyond 13.
- **Failed launches were counted as launches in this note's first run.** The
  correction is at the top; 8.4% of launch transactions fail. It was found by
  `radar dossier` disagreeing with this query about one mint, which is the first
  time the two halves of this work have checked each other.
- **One venue, one twelve-hour window, one day.** The comparison against 0008 is
  therefore also a comparison between two different days, and some of the
  movement may be the venue rather than the tooling.
- **This is not a return.**
  [`0011`](0011-graduation-predicts-volatility-not-profit.md) is the standing
  correction: graduation predicts volatility, not profit, and organic graduations
  end at a median −3,228 bps. A signal that predicts graduation is a reason to
  stay away, not a reason to buy.

## 0007 re-measured on the same store

The other signal, on the same evidence, run as
`radar study --store <store> --pivot 442653662`. The pivot is pinned because two
unpinned runs minutes apart disagreed — the recorder appends while the study
reads, so the store span moves and the default pivot moves with it. A published
figure needs a fixed one.

```
store spans   : slot 441040080 .. 444006292
pivot         : slot 442653662
creators      : 2957 with >= 5 launches before the pivot and at least one after
later launches: 74871 of which 1366 graduated organically
base rate     : 1.82%

  KNOWN AT PIVOT                  CREATORS  LAUNCHES   ORGANIC  RATE
  no organic graduation known         2231     34846       302  0.86% [0.77% – 0.97%]
  exactly one                          350     10365       202  1.94% [1.70% – 2.23%]
  two or more                          376     29660       862  2.90% [2.72% – 3.10%]
```

**This one strengthens.** 0007 had 638 creators; this has 2,957. The separation
is wider — **3.37×** band-to-band, against 0007's 2.12× — and the intervals are
nowhere near touching.

**Say which ratio is meant.** Band-to-band is 3.37×; the best band against the
1.82% base rate is **1.59×**. 0007's headline "~2×" was the band-to-band figure
and is often repeated as though it were the lift over the population. The two
differ by more than a factor of two, and the smaller one is the one that answers
"how much better than not knowing".

**The confound runs the wrong way for the sceptic**, which is the part worth
keeping. Controlling for launch frequency, all three bands separate at 95%:

```
  PRIOR LAUNCHES      WITHOUT PRIOR GRADUATION       WITH PRIOR GRADUATION
  5-9 launches       1.66% [1.39%-2.00%] n=918  10.92% [9.13%-13.04%] n=126
  10-29 launches     0.79% [0.64%-0.98%] n=888   5.39% [4.53%-6.41%] n=195
  30+ launches       0.60% [0.50%-0.73%] n=420   2.26% [2.12%-2.43%] n=405
```

Creators who launch more graduate *less* per launch — 1.66%, 0.79%, 0.60% among
creators with nothing good known. So the obvious objection, that prolific
creators have more chances to have graduated something, runs against the result
rather than for it, and controlling for frequency strengthens it.

**And it still is not a reason to buy.**
[`0011`](0011-graduation-predicts-volatility-not-profit.md) applies here exactly
as it does above: a creator whose tokens graduate is a creator whose tokens end
at a median −3,228 bps. This measures that `creator_edge`'s rule is not
arbitrary. It does not measure that following it makes money — 
[`0017`](0017-a-control-that-could-have-been-traded.md) measures that, at 0 bps.

## The snapshot

Both re-measurements are emitted as
[`data/0024-base-rates.json`](data/0024-base-rates.json), which is what the Phase
2 fact-sheet builder reads rather than decoding 483,629 launches per question.
Every figure in it carries `measured_on`, because the whole finding above is that
these numbers move.

## What should follow

1. **Stop publishing 68% / 5%.** It is wrong by 2.7× on the numerator and it
   hides the organic cohort. Where the shape needs one sentence, the honest one
   is the refusal: *"70.5% of launches have one to three recipients in their
   launch block, and 0.02% of those graduate instantly."*
2. **Re-measure on a schedule, and alarm when the mode moves.** 0008 asked for
   this and nothing was built, so the drift was found nine days later by hand.
   [LEARNINGS](../../LEARNINGS.md) 21 — a monitor slower than its failure is
   reporting history — and LEARNINGS 5 and 26 — a check must fail differently
   when it did not run than when it found nothing.
3. **`radar-graph`'s thresholds are derived from 0008 and should be
   re-derived.** Not done here: this note measures, and changing a refusal rule
   is a separate change with its own tests.
