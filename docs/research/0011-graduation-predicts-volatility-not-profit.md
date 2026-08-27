<!-- SPDX-License-Identifier: Apache-2.0 -->
# 0011 — Graduation predicts volatility, not profit

**Date:** 2026-08-26
**Store:** the live guardian VPS recorder, **59,647 priced mints**
**Status:** well powered for the first time; one regime still

Every signal in this repository is fitted against **graduation**.
[0007](0007-does-creator-history-predict-anything.md) asks whether creator
history predicts one, [0008](0008-the-launch-block-gives-the-bundle-away.md)
whether a launch block's shape does.
[0009](0009-what-a-token-actually-does-to-your-money.md) measured what a token
does to a holder but could not connect the two.

This note asks the question those three leave open: **is graduation worth
anything to the person holding the token?**

## What it says

Held from the first fill to the last observed price, in basis points:

| cohort | n | p25 | median | p75 | % up | % clearing 850 bps | median MFE | median MAE |
|---|---|---|---|---|---|---|---|---|
| never graduated | 57,335 | −3,602 | **−853** | 0 | 11.2% | 8.2% | 533 | −1,509 |
| organic graduation | 1,614 | −9,100 | **−3,228** | 0 | 18.2% | **15.0%** | 6,520 | −6,200 |
| instant graduation | 698 | −9,773 | **−5,981** | 75 | 25.6% | **18.1%** | 21,384 | −8,905 |
| **all** | **59,647** | −3,756 | **−863** | 0 | 11.6% | 8.5% | 598 | −1,603 |

Graduation makes the **median outcome worse** — a token that graduates organically
ends down 32.3% against 8.5% for one that never does, and an instantly graduating
one ends down 59.8%.

It also **roughly doubles the chance of clearing costs**: 15.0% and 18.1%
against 8.2%.

Those are not in tension. Graduated tokens are more volatile in both directions.
Their median MFE is +65.2% and +213.8% against +5.3%, and their median MAE is
−62.0% and −89.1% against −15.1%. They go much further up, and much further
down, and end lower.

**So graduation is a volatility signal that has been used as a profit proxy**,
and every threshold in this repository fitted against it inherits that.

## What this does not license

**It is not an entry.** The excursion figures are measured from each token's
*first fill*, and for an instant graduation the first fill belongs to whoever
arranged it — 0008 measured the shape of that arrangement and found 68% of
instant graduations carry exactly six recipients in the launch block.
`creator_edge` acts around forty minutes later. The +213.8% is not available to
Radar; it is what Radar would be buying *into*.

That is 0008's refusal thesis with a number attached, and it is the second time
this project has measured it from an independent direction.

**The organic cohort is the one worth more thought.** 15.0% clearing costs
against 8.2% is a real doubling on a cohort of 1,614, and unlike the instant
cohort it is not structurally spoken for. Whether any of it survives entry forty
minutes late, at Radar's prices, after 850 bps of friction, is exactly the
question [`radar selection`](../../crates/radar-research/src/selection.rs) exists
to answer — and it will not be answerable until enough decisions carry an entry
price and a later observation.

**One regime.** Every caveat 0007, 0008 and 0009 carry applies. The cohort is
large now; it is still one stretch of one market.

## Two corrections to 0009

**0009's excursion table was computed with a contaminated query and should not
be read.** It reported a median MFE of 2,367 bps and a 90th percentile of
2,306,382 bps. The price aggregate admitted dust transactions — a transfer of
one base unit beside unrelated SOL — and because `peak_price` is `max(lam/tok)`
the aggregate selected precisely those rows. Recorded as
[LEARNINGS](../../LEARNINGS.md) entry 14.

Corrected, over 59,647 mints: **median MFE 598 bps, p90 19,404 bps**. The p90
fell by a factor of 119.

**0009's headline median of −13.4% is not contradicted by the −8.6% here, and
the difference is not an error in either.** `last_price` is the last fill within
a measurement window, and a token measured at its one-hour checkpoint has had
less time to fall than one measured at twenty-four hours. The mix of checkpoint
ages changes as the store grows, so the population median moves with it. Over
4,199 priced mints it was −1,340 bps; over 59,647 it is −863.

The consequence is that **there is no single population constant to compare a
selection against**, which is why `radar selection` compares against the
decisions Radar refused — priced the same way, in the same passes — rather than
against a number from this note.

## Method

`Outcome` rows carrying a price, latest measurement per mint, from the store's
own hourly outcome pass. Prices are lamports per base unit from
`solana.transactions.balance_changes`, filtered to transfers between accounts
and to transactions moving at least a tenth of the mint's own median trade on
both sides.

Instant graduation is `graduated_at − launch_slot ≤ 3`, the same threshold
`radar-store` uses. The priced cohort's graduation rate is 3.88%, against the
store-wide 3.2% measured in 0006 — a mild upward bias, because a token that
never traded has no price and also never graduates.
