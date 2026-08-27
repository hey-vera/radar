<!-- SPDX-License-Identifier: Apache-2.0 -->
# 0009 — What a token actually does to your money

**Date:** 2026-08-25
**Status:** first measurement, one hour of launches, one regime

Every signal in this repository has been validated against **graduation**. Research
[0007](0007-does-creator-history-predict-anything.md) measured whether creator
history predicts a later organic graduation; [0008](0008-the-launch-block-gives-the-bundle-away.md)
measured whether a launch block's shape predicts one. Neither asked whether a
graduation is worth anything, because until now the store held no price and the
question could not be put.

0008 is also the reason the question cannot be skipped. A launch whose curve
completes inside its own block graduates — and the supply is already held by
whoever arranged it, so the graduation is real and the opportunity belongs to
somebody else. Graduation is a proxy for profit, and on that cohort it is an
inverted one.

This note is the first measurement of the thing itself.

## Method

200 pump.fun tokens minted between 04:00 and 05:00 UTC on 2026-08-25 — every
launch in the hour, not a selected set. For each, the realised price path over
the following six hours, computed from `solana.transactions.balance_changes`
against the token amount moved: what the SOL side of each fill actually was,
divided by what the token side actually was.

177 of the 200 had at least one fill and so a usable entry price. The other 23
never traded at all, which is itself the most common outcome in this market and
is excluded here only because a token that never traded has no return.

Three figures per token, all in basis points against the **first fill**:

- **MFE** — maximum favourable excursion, the best it ever got.
- **MAE** — maximum adverse excursion, the worst it ever got.
- **HELD** — where it was at the last observed fill.

> **Correction, 2026-08-26.** The excursion table below was computed with a
> contaminated price query and **should not be read**. The aggregate admitted
> dust transactions — a transfer of one base unit beside unrelated SOL — and
> because `peak_price` is `max(lam/tok)` it selected precisely those rows. See
> [LEARNINGS](../../LEARNINGS.md) entry 14. Corrected figures over 59,647 mints
> are in [0011](0011-graduation-predicts-volatility-not-profit.md): median MFE
> **598 bps**, p90 **19,404 bps**, against the 2,367 and 2,306,382 reported here.
>
> **The held-to-end median of −13.4% stands.** `first` and `last` are chosen by
> timestamp rather than by price, so they were very nearly unaffected — which is
> why this note's headline survived while the table beside it did not. Over a
> larger cohort the population median moves to −863 bps, because `last_price`
> depends on which checkpoint a token was last measured at; 0011 explains why
> that is a property of the measurement rather than a disagreement.

## What it says

```
                          p10       p25    median       p75       p90
MFE  (best gain)            0         0      2367     23434   2306382
MAE  (worst drawdown)     141       751      2048      8395      9987
HELD (entry -> last)    -9485     -4315     -1339      -237       753
```

**The median token, bought at its first fill and held, returns −13.4%.** A
quarter are down 43% or worse. Only **12% (22 of 177)** are up at all, and 23%
are down more than half.

That is the base rate `AGENTS.md` opens with, arriving from an independent
direction. It was quoted there from an external source; this is the same claim
measured in Radar's own data, on Radar's own definition of a fill.

The median MFE of +23.7% against a median MAE of −20.5% is the shape worth
sitting with: **the typical token does go up before it goes down.** There is
something for an exit rule to capture. What there is not, is anything for a naive
one to capture.

## A take-profit does not rescue an unselected entry

Entering every token at its first fill and exiting at a fixed gain, or at the
last observed price if that gain never arrives. Costs ignored, which flatters
every row:

```
take-profit    hit rate    mean bps
     500bps         63%        -643
    1000bps         57%        -500
    2000bps         52%         -91
```

Hit rates look encouraging and the means are all negative. The reason is in the
distribution rather than the rule: the 37–48% that miss the target do not miss it
by a little. Their `HELD` is the p10 of −94.9%, and no achievable take-profit on
the winners pays for that.

**So the exit rule is not where the edge is.** It cannot be — this is the
unselected population, and Radar's entire thesis is that the edge is in *not
buying traps*. This note is the control that thesis has to beat, and the number
to beat is −13.4% before costs.

## What this does not establish

- **One hour, one regime, 200 launches.** Same caveat as 0007 and 0008 and it has
  not weakened with repetition.
- **Entry at the first fill is not Radar's entry.** `creator_edge` refuses on
  creator history, launch rate and launch-block shape, and its token-age budget
  means it acts around 40 minutes after launch, not at the first trade. The
  cohort here is deliberately *unfiltered*: it is the population, not the
  selection.
- **Costs are ignored throughout.** `assumed_round_trip_bps` was 200 when this
  was written. Measured over 26,691 fills it is **850**, which moves every mean
  above further down.
- **Survivorship runs the other way for once.** The 23 tokens that never traded
  are excluded, and they are the worst outcomes in the sample. Their exclusion
  makes these figures *better* than the truth, not worse.

## The bug this note found

The first run of this analysis reported a median MFE of **7,054%** and a 90th
percentile of **1,081,162%**. Those numbers are not a market and were not
questioned by anything in the pipeline that produced them.

`token_transfers` carries a `MintTo` row at launch holding the **entire supply** —
1e15 base units, 44× the average real fill on the same token. It is also the
earliest row, so it became `first_price`, and every excursion was measured
against the supply mint instead of against a trade.

The price query now counts only `Transfer` and `TransferChecked`. Recorded as
[LEARNINGS](../../LEARNINGS.md) entry 12, because the failure shape is the one
this project keeps meeting: a wrong number looks like a number, where a missing
one looks like a gap.

## Next

The measurement that matters is the same one restricted to candidates that
survive `creator_edge`. That is a strictly harder query — it needs decisions
recorded alongside prices — and it is the point of the paper lane. Until then,
**−13.4% is the number any selection has to beat**, and it is now measured rather
than assumed.
