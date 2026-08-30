<!-- SPDX-License-Identifier: Apache-2.0 -->
# 0015 — The second considerations document, classified

**Date:** 2026-08-30
**Source:** [`vendor/chatgpt-radar-considerations2.md`](vendor/chatgpt-radar-considerations2.md),
3,970 lines, authored by ChatGPT, opening with Josh's own note that it "should
not be taken as best-in-class yet".
**Method:** the same as [`0010`](0010-the-considerations-document-classified.md) —
source kept unedited under `vendor/`, verdict as a separate numbered note that can
be cited and superseded on its own.

## The headline, and it corrects an expectation

**This document does not repeat the first one's blind spot.** 0010 found that the
first document was written for a latency sniper and never asked whether the system
it advised competes on speed — which reorganised its whole middle section and made
thirty sections cargo-culting.

The second is written for a research and adversarial system, and it is explicit
where its predecessor was silent:

- **§39** treats latency as a variable to *measure*, and warns that measuring it
  may reveal a strategy "profitable conceptually but impossible because your
  pipeline is 800ms slower than competitors". That is the conclusion 0010 had to
  reach on the first document's behalf.
- **§58** says plainly that "having access to Rust/Solana tooling is not an edge".
- Part 2 **§12** ("simulate your own exit before entering"), **§13** (an
  adversarial liquidity model) and **§21** (a tradeability score *separate from*
  liquidity) are Radar's exit-first thesis, arrived at independently.

So 0010's dismissal method should not be applied to this document wholesale. It is
a materially better source than its predecessor, and it earns a closer reading.

## What it gets right, including where Radar does not

- **§94 "separate intelligence from authority."** This is
  [`AGENTS.md`](../../AGENTS.md) rule 1, and Radar's version is **stronger**. The
  document grants the model "very limited authority"; Radar grants it none — no
  action tools at all, output never parsed into an action, and a signer that
  re-decodes the transaction rather than trusting a description.
- **§6 "never give the AI unlimited tool access."** Same direction, same gap:
  the document proposes a read/analysis tool split, and Radar has no tools to
  split. If there is nothing to inject into, prompt injection stops being a threat
  that needs defending.
- **§96's "do not build" list** is worth checking against, and Radar clears every
  item on it — including "a system where the strategy can override risk controls"
  and "a backtest without realistic execution", the second of which Radar clears
  only in the sense that it has not backtested execution at all.

**§85–§88 are the sections Radar should sit with, because the document is right
and Radar is not.** §87: *"A strategy can work with $500 but fail at $500,000
because the market can't absorb it. The system should know its capacity."* §88
asks each strategy for a minimum, optimal and maximum capital and a liquidity
ceiling.

Radar does not know its capacity in the sense §87 means. It measures exit capacity
**once**, at decision time, in
[`Decision::exit_capacity_micro_usd`](../../crates/radar-store/src/decision.rs),
and never again — there is no capacity field on
[`Outcome`](../../crates/radar-store/src/outcome.rs) at all. A token's sellability
is recorded at one instant and assumed forever after, which is the assumption every
return figure in this repository rests on.

Measured on the same day as this note, across 2,365 recorded proposals: exit
capacity has a p10 of $26.90, a median of $31.03 and a p90 of $34.59, with 28 of
2,365 above $60. Median proposed notional is **$6.21**. Eighty per cent of
proposals sit in a ±13% band, because every pre-graduation pump.fun token rides the
same bonding curve with the same supply — so the number is close to a property of
the venue rather than of the token.

That is §87's question asked from the other end, and the document does not ask it
from that end. Its framing throughout §85–§88 is the **ceiling** — do not give a
strategy more capital than the market absorbs. Radar's binding problem is the
**floor**: the largest position the venue offers is $6.21, and an 850 bps round
trip needs +8.5% just to break even. A strategy whose maximum viable position sits
below its own cost floor is not a strategy, and no section says so.

## What is worth doing, and where it already sits in the plan

- **Part 2 §21, tradeability separate from liquidity.** The sharpest unbuilt idea
  in the document, and the finding above is why. Radar measures depth; it does not
  measure whether that depth is *there again later*. Recording capacity at each
  outcome checkpoint is Phase 1.2 of the current plan.
- **Part 2 §12, simulating the exit under stress** ("what if liquidity fell 25%?").
  Radar simulates the exit at one moment under no stress.
- **§89 strategy crowding** — "ten strategies can secretly be one strategy". Not
  actionable while there is one strategy, and exactly the right thing to have
  written down before there are several.
- **§95 autonomous strategy discovery** is the document's most ambitious section
  and Josh's own first vision item. The shape is right — observe, hypothesise,
  backtest, out-of-sample, paper, tiny capital, scale or retire. The hazard is
  arithmetic: Radar has 2,591 scoreable decisions from one venue, one regime and
  four days, and they are not independent. A search over that is an overfitting
  machine, and [`LEARNINGS`](../../LEARNINGS.md) entries 7, 10 and 11 are all
  cases of this project drawing confident conclusions from a selected sample. The
  prior question is that the one strategy which exists has never been shown to
  beat a matched control.

## What is wrong for this system

- **§23 / §107-shaped redundancy arguments — "build redundant data sources."** Same
  objection 0010 raised and it has not weakened: Radar has one historical source,
  chosen deliberately in
  [ADR 0002](../adr/0002-historical-data-comes-from-cryptohouse-not-a-vendor-archive.md).
  Redundancy is a cost with no benefit until there is a second source worth having.
- **Part 2 §3–§7, the wallet-clustering and entity-graph stack.** Real work,
  genuinely interesting, and it does not touch the binding constraint. Part 2 §4
  is also blocked by the same structural fact
  [`0012`](0012-recipient-sets-cannot-recur-authorities-can.md) measured: a
  `destination` in `solana.token_transfers` is a token account, so recipient sets
  cannot recur across mints however coordinated the launches are. Owner resolution
  is a join Radar does not have, and the document assumes it throughout.

## The document's real limitation

Not a blind spot this time — a category.

**Every one of its ~210 items is a thing to do, and none is a thing that is true.**
It is a coverage checklist. Where it carries numbers they are illustrative — "a
strategy generating 10% using 95% of capital", "$500", "$500,000" — chosen to make
a point rather than measured from anything.

For Radar the binding facts are all measurements, and none of them is derivable
from a checklist: capacity ≈ $31, round trip ≈ 850 bps, gross median ≈ +21 bps
([`0014`](0014-the-control-was-entirely-tokens-nobody-could-sell.md)). "The system
should know its capacity" is good advice that cost nothing to write; knowing the
capacity is $31 is the finding, and it took a query.

So the document's value is as an **audit against Radar's coverage**, not as a
source of direction — which is the same conclusion 0010 reached about its
predecessor in different words, arrived at for a different reason. It is worth
re-reading when a new subsystem is designed, and it is not worth mining for the
next thing to build.

## What this note does not do

- **It does not rank the items and does not become a backlog.** The sections judged
  worth doing are worth doing for reasons already in the plan before the document
  was read.
- **It does not classify all ~210 sections.** The four buckets above name the ones
  that change a decision. A section not mentioned here was read and found to be
  either already covered by 0010's verdict on the first document, or a restatement
  of something Radar already does.
- **It does not verify the document's factual claims about Solana or Jupiter.**
  §0's remark that Jupiter's Ultra is superseded by Swap V2 is repeated here as the
  document's claim, not as Radar's — Radar's own integration is
  [`radar-sim`](../../crates/radar-sim/src/jupiter.rs) and its behaviour is pinned
  by measurement in `LEARNINGS` entry 11 rather than by a vendor's changelog.
