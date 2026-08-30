<!-- SPDX-License-Identifier: Apache-2.0 -->
# 0018 — The deep tail points the wrong way

**Date:** 2026-08-30
**Store:** the live guardian VPS recorder, 1,100 proposals banded by measured
exit capacity
**Status:** measured, and **the band that matters is below the cohort floor at
n=16**. Directional, not settled. Recorded because the direction is the opposite
of the one the plan was hoping for.

## Why this question comes before any filter work

Radar sizes every position as a share of measured exit capacity, so capacity
decides how much money can move. Measured across 2,365 recorded proposals:

```
exit capacity  p10 $26.90   p50 $31.03   p90 $34.59    >$60: 28 (1.2%)   max $618.40
notional       p10  $5.38   p50  $6.21                              max $123.68
```

**Eighty per cent of proposals sit in a ±13% band around $31**, because every
pre-graduation pump.fun token rides the same bonding curve with the same 1e15
supply. Capacity is closer to a property of the venue than of the token, and the
median position it produces is **$6.21**.

At $6.21 an 850 bps round trip needs a **+8.5% move to break even**. No
improvement to the *filter* changes that, which is why
[`0017`](0017-a-control-that-could-have-been-traded.md) finding no edge does not
make "improve the selection" the next move. The next move is to ask whether
there is any depth here at all — and Radar has **never selected for capacity**.
It selects on creator history and sizes off whatever depth happens to be
present.

## The result

Priced realised-to-realised, the same way 0017 prices both its cohorts, because
[`0016`](0016-the-entry-was-a-bid-and-the-exit-was-a-mid.md) showed a
quote-to-fill comparison carries an artefact larger than anything it measures.

```
capacity        n     median   zero share
<$25          108       -448          555
$25-30        387          0         4108
$30-35        489          0         6298
$35-60        100       -541         1900
$60+           16      -6813            0
```

**The deep band is the only one where a position of any size is possible, and its
median is −6,813 bps — down 68%.**

It is also the only band with **no** flat returns at all. The middle bands, where
876 of 1,100 proposals sit, are 41% and 63% exactly zero — the point mass 0017
identified, tokens that barely traded. The deep tokens traded, and they fell.

## Why this is directional and not a finding

**Sixteen rows.** Below the twenty-row floor this repository uses, and far below
anything that should move a decision.
[`LEARNINGS`](../../LEARNINGS.md) entries 7, 10 and 11 are all cases of a
confident conclusion drawn from a small selected sample, and the $60+ band is
selected by exactly the mechanism under study.

What makes it worth recording anyway is that it is **the opposite of the
convenient answer**. The plan's hope was that deeper tokens might behave
differently in a way that permits size. They do behave differently. They behave
worse.

## The mechanism this is consistent with

A token with unusual depth forty minutes after launch has had unusual volume, and
this repository has measured twice what unusual early volume means:

- [`0011`](0011-graduation-predicts-volatility-not-profit.md) — graduation
  predicts **volatility, not profit**; graduated tokens go further up, further
  down, and end lower.
- [`0008`](0008-the-launch-block-gives-the-bundle-away.md) — the fast money is
  committed before the token exists, and the remaining role for a later buyer is
  to be who they sell to.

Depth at forty minutes may simply be another reading of the same thing. If so,
selecting for capacity would be selecting for the trap, and the whole idea of
"find deeper tokens so a real position fits" is buying the wrong end of 0008's
finding.

**That is a hypothesis, not a result.** It is consistent with the sixteen rows
and with two prior measurements, and it has not been tested.

## What this means for the venue question

The plan asked Phase 1 to decide whether pump.fun pre-graduation can host
$10k–$100k. On this evidence it looks unlikely from both directions at once:

- **Capacity is a venue constant.** 80% of tokens offer ~$31, so a $10k position
  is three hundred times what the median token supports.
- **The tokens that offer more appear to be worse**, at n=16.

Neither settles it. Together they say the burden of proof has moved: the case for
staying on this venue now needs evidence, where before it only needed no evidence
against.

## What this does not establish

- **n=16 in the band that carries the argument.** Everything above turns on it.
- **Capacity is a Radar measurement**, not a fact about the token — 0014's caveat
  applies unchanged. A different probe or budget moves which band a token lands
  in.
- **No causal claim.** Deep tokens are not deep at random.
- **One venue, one regime, four days.**
- **Gross.** The measured round trip is 850 bps and nothing here is net of it.

## What would settle it

1. **Wait for the band to fill.** At ~1.2% of proposals above $60 and roughly 960
   decisions a day, twenty rows is about two days and a hundred is under a
   fortnight. This is the cheapest decisive measurement available and it needs no
   new code.
2. **Record depth over time** (plan Phase 1.2). Capacity is measured once, at
   decision time, and never again — so "the deep tokens fell" cannot currently be
   separated from "the deep tokens stopped being deep".
3. **Test the mechanism** against 0008's launch-block signal: if depth at forty
   minutes is a proxy for coordination, the two should agree on the same tokens.

## Reproducing

```bash
radar control --store <dir>
```
