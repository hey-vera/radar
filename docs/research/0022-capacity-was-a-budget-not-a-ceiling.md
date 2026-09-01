<!-- SPDX-License-Identifier: Apache-2.0 -->
# 0022 — Capacity was a budget, not a ceiling

**Date:** 2026-09-01
**Source:** four pump.fun bonding-curve accounts read from mainnet, three of them
tokens the live instance decided on that day
**Status:** measured. **`0018`'s capacity figure is a policy setting, not a
property of the venue** — and the distinction reopens a question 0018 closed.

## What 0018 concluded

> Eighty per cent of proposals sit in a ±13% band around $31, because every
> pre-graduation pump.fun token rides the same bonding curve with the same 1e15
> supply. Capacity is closer to a property of the venue than of the token.

and from that:

> **Capacity is a venue constant.** 80% of tokens offer ~$31, so a $10k position
> is three hundred times what the median token supports.

The first sentence is right about the *mechanism* and the second overstates what
follows from it.

## What "capacity" actually measures

[`Search::DEFAULT`](../../crates/radar-sim/src/exit.rs) is the search the exit
probe runs:

```rust
max_impact_bps: 100,          // 1%
max_quotes: 8,
enough_lamports: 50_000_000_000,
```

So capacity is **the largest size whose price impact stays under 1%**. It is not
the largest size the venue can absorb, and nothing measured it as such.

That also explains the ±13% band, and explains it better than "same supply" does.
A pump.fun curve is constant product, so for small sizes impact is very close to
`sol_in / virtual_sol_reserves`. Every standard launch starts at the same virtual
reserves, so **1% of a near-constant is a near-constant**. The tightness of that
band is a fact about the budget, not about the market.

## The arithmetic, which on this venue is exact

A bonding curve is `x·y = k` over reserves published in an account anyone can
read. Given reserves and a size, the fill price is *computed*, not estimated —
which is unusual and worth saying plainly, because on an order book the same
question needs a model.

Read from mainnet, 2026-09-01:

```
curve                              virtual SOL   1% impact        8.5% impact
fresh launch (decided on today)         30.13    0.301 SOL ~$60   2.56 SOL ~$512
mid-curve token                         36.19    0.362 SOL ~$72   3.08 SOL ~$615
```

8.5% is chosen because it is the measured round trip
([`0019`](0019-the-round-trip-is-not-one-number.md)) — the point at which impact
costs the same as the friction Radar already accepts.

**So a standard fresh launch supports roughly $500 at an impact equal to the
round trip, against the ~$31 the 1% budget reports.** That is a factor of about
sixteen, and it is the difference between a setting and a wall.

## What this does and does not change

**It does not make $10k reachable.** $10k is still around twenty times what the
curve absorbs at 8.5%, so 0018's conclusion about the *stated objective* stands.
What changes is the size of the gap and what closes it: twenty times is a
different problem from three hundred, and the first number came from comparing
against a budget rather than a limit.

**It does not say taking more size is better.** Impact is a real cost, paid in
full, and a position taken at 8.5% impact has spent the round trip before the
market moves at all. The claim here is only that the trade-off is now
*computable* rather than fixed — which is what an impact budget is supposed to
be.

**It does not vindicate the selection.**
[`0017`](0017-a-control-that-could-have-been-traded.md)'s null result is about
which tokens are chosen and is untouched by any of this.

**What it does change** is that `max_impact_bps` becomes a decision with a number
behind it. It is currently 1%, it was never argued for in the file that sets it,
and it is the single input that determined the figure 0018 built its case on.

## The population is not uniform, which 0018's framing understates

Three curves for mints the live instance decided on today:

| token | virtual SOL | state |
|---|---|---|
| `2GS2W11v…` | 30.13 | fresh, essentially at launch defaults |
| `2EZReDjo…` | **0.02** | real reserves 0.003 SOL — genuinely thin |
| `Bai2nLiN…` | 0.00 | `complete = 1` — **already graduated** |

One of three had effectively no depth at any budget, and one had left the curve
entirely. A median over this population describes none of them well, and "every
token rides the same curve" is true of the *formula* rather than of the reserves.

## What this does not establish

- **Four curves.** Two read in full, two sampled. This is arithmetic on a
  published formula rather than a survey.
- **The curve's own fee is not modelled here**, and it is charged on top of
  impact. The figures above are therefore optimistic by that fee.
- **Impact is not slippage-on-execution.** A fill lands after other transactions
  in the same block, so the reserves at execution are not the reserves at
  decision. The arithmetic is exact for a state, not for a future.
- **Nothing here is a realised fill.** It is what the curve says it would pay,
  which is the same instrument on both sides — an improvement on the quote ladder
  0016 warned about, and still not a trade.

## Addendum, same day — the budget is not the lever, and this note first implied it was

The section below originally opened by asking for `max_impact_bps` to be argued
or raised. Worked through, that is the wrong conclusion from the right finding,
and the arithmetic says so plainly.

**Round-trip cost rises monotonically with size**, because impact is linear in
size while the fee component is roughly flat in basis points above $20:

```
   size   impact rt   fees rt    total
     $3          10       250      260 bps
  $6.21          21       250      271 bps
    $20          67       456      523 bps
    $60         200       456      656 bps
   $200         667       450    1,117 bps
   $512       1,707       450    2,157 bps
```

So the venue *supporting* $512 is not an argument for taking $512. The current
sizing — a fifth of the 1% figure, about 20 bps of impact — is the **cheapest**
row in that table, and it was not a mistake.

**What size is right is a function of the edge, not of the venue.** Profit is
`s · (r − a − b·s)` for an edge rate `r`, fee rate `a` and impact slope `b`,
which is maximised at:

```
s* = (r − a) / 2b
```

With `a ≈ 456` bps and `b` from a 30 SOL curve:

| edge | optimal size |
|---|---|
| 0 bps | **do not trade** |
| 100 bps | **do not trade** |
| 300 bps | **do not trade** |
| 850 bps | $59 |
| 2,000 bps | $232 |

[`0017`](0017-a-control-that-could-have-been-traded.md) measures the edge at
**0 bps**. So the optimal size is not small — it is *negative*, and the shipped
`Policy::CLOSED` is the arithmetically correct position.

**The useful thing this produces is a bar rather than a setting.** Any strategy —
including a model-driven one — has to clear roughly **456 bps of expected edge
before a single trade is worth making**, and roughly 850 before a position of
even $59 is. That is a number to measure a strategy against, and it is a more
honest target than "be good at trading".

`max_impact_bps` should therefore be left at 1% and revisited **only** once
something measures an edge above that bar. Raising it first would spend money
faster in the direction 0017 says there is nothing to be gained in.

## What should follow

1. ~~**Argue `max_impact_bps`.**~~ Superseded by the addendum above: it is not the
   binding constraint and should not move until an edge is measured. Capacity was
   never the reason this does not work.
2. **Price the exit off the curve rather than off a quote ladder.**
   [ADR 0009](../adr/0009-radar-builds-its-own-pump-fun-swaps.md) has to read the
   bonding curve account to build an instruction; once it does, `radar-sim` can
   stop asking a vendor what a fill would cost and compute it.
3. **Re-run 0018's banding against curve-derived capacity.** Its bands were built
   from the 1% figure, so every row of that table is a row about the budget.

## Reproducing

The bonding curve address is `["bonding-curve", mint]` under the pump.fun
program, verified against mainnet in
[`the_account_layouts_are_what_mainnet_shows.rs`](../../crates/radar-decode/tests/the_account_layouts_are_what_mainnet_shows.rs).
Its account layout after the 8-byte discriminator is five `u64` reserves, a
`complete` flag, and the creator.
