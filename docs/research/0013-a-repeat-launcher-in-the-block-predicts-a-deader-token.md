# 0013 — A repeat launcher in the block predicts a deader token

**Date:** 2026-08-30
**Status:** measured on two disjoint windows. The direction replicates; the
magnitudes move, and the outcome is **activity, not money**.

## The question 0012 left open

[`0012`](0012-recipient-sets-cannot-recur-authorities-can.md) measured *who*
recurs in launch blocks and said plainly what it had not measured:

> This measures who recurs, not whether recurrence predicts anything about
> money. **Until that is measured, this is a description of the venue and not a
> signal.**

This is the first half of that measurement. It is not the money half.

## What was measured

Launches were classified by the strongest authority prevalence in their own
launch block, using 0012's bands — `repeat` (3–99 launch blocks) outranking
`ordinary` (1–2), which outranks `infrastructure` (100+). Then, for each launch,
what happened **after** its launch block: transfers, distinct participants, and
whether anything happened at all.

Launches are restricted to those with at least an hour of follow-up inside the
query window, so a token launched at the window's edge is not counted as dead for
having had no time to live.

## The result, on two disjoint windows

Window A ends now; window B ends three hours earlier. No launch is in both.

| cohort | launches A / B | **% with no transfer after the launch block** | % reaching 10+ participants | median later transfers |
|---|---|---|---|---|
| ordinary | 7,487 / 8,031 | **33.3% / 25.2%** | 13.3% / 19.6% | 2 / 4 |
| repeat | 3,378 / 3,475 | **62.6% / 55.0%** | 5.8% / 10.4% | 0 / 0 |
| infrastructure | 4,150 / 7,703 | **87.3% / 70.0%** | 2.2% / 2.0% | 0 / 0 |

**A launch whose block contains a repeat launcher is about twice as likely to be
dead immediately** — 1.88× in A, 2.18× in B — **and about half as likely to reach
ten participants** — 0.44× in A, 0.53× in B.

The ordering is identical in both windows and the ratios are close. The
magnitudes are not: the absolute death rate moves eight to seventeen points
between two windows three hours apart, which is the usual reminder that these are
regime-dependent numbers.

## The mean is not a finding

Mean later transfers came out at **204.5** for the repeat cohort in window A and
**72.9** in window B — a flip large enough to reverse the ranking against
`ordinary` (62.7 / 106.1).

That instability is the point, and it is the same shape
[`0011`](0011-graduation-predicts-volatility-not-profit.md) recorded: a mean over
a heavy tail is a search for the tail, and it finds a different one each time.
The medians and the rates are the stable part, so they are the finding and the
mean is not.

## Why this does not make the signal actionable

The outcome here is **activity**, and activity is not money.

[`0011`](0011-graduation-predicts-volatility-not-profit.md) is the cautionary
case, in this repository, about exactly this substitution: graduation looked like
a success measure, was used as a profit proxy, and turned out to predict
*volatility* — "a volatility signal that has been used as a profit proxy, and
every threshold fitted against it inherits that." A token that keeps trading is
not a token that made anyone money, and a token that died is not necessarily one
that lost anyone anything if nobody could have bought it.

So `Prevalence::is_actionable` stays false. What would change that is the
measurement 0012 named and this one still does not make: **prices**, joined
against Radar's own recorded outcomes, on a cohort large enough to have a median.

## What this does change

0012's crate documentation claimed the reading had *"no measured link to any
outcome"*. That was true when it was written and is now false — there is a
measured link, to a weak outcome, replicated once. The wording is corrected
rather than left to age, which is the failure
[`LEARNINGS` 13](../../LEARNINGS.md) records twice in one day.

The evidence tier stays `Weak`. [`0008`](0008-the-launch-block-gives-the-bundle-away.md)
earns `Strong` by comparing three populations against a graduation outcome; this
has two windows and an activity proxy.

## What this does not establish

- **Not money.** Said three times above because it is the whole caveat.
- **Two windows, one afternoon.** Both are same-day. A week apart, or a different
  regime, is untested.
- **`min(block_slot)` is a proxy for the launch slot**, correct for tokens that
  launched inside the window and wrong for any that did not. The hour of inset is
  meant to ensure the former.
- **The `infrastructure` cohort is a strange population** and its numbers should
  not be read as a finding. It is the set of launches where *every* authority in
  the block was above the infrastructure floor — no ordinary participant at all
  — which is a router touching a token nobody else did. That it is the deadest
  cohort is unsurprising and not interesting.
- **No causal claim.** A repeat launcher in the block and a token nobody trades
  are both consequences of the same thing — somebody minting in bulk — rather
  than one causing the other.

## Queries

[`queries/0013-activity-by-authority-band.sql`](queries/0013-activity-by-authority-band.sql),
carried rather than described — a note whose query exists only in prose is one
nobody can re-run, and re-running is the only way a number in a note gets
checked. Window B is the same statement with every interval shifted back by 180
minutes.

Each ran in under nine seconds against the public endpoint.
