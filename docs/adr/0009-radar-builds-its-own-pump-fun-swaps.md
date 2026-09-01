<!-- SPDX-License-Identifier: Apache-2.0 -->
# ADR 0009 — Radar builds its own pump.fun swaps

**Date:** 2026-09-01
**Status:** accepted
**Decides:** how Radar constructs the transaction it asks a signer to sign.

## Context

[`0021`](../research/0021-the-signer-cannot-read-the-only-venue-that-lists-them.md)
found that Radar cannot build a transaction for any token it selects. Not
"cannot profitably" — cannot at all. Three constraints, each correct on its own,
cannot all hold:

1. The signer's guarantee is *every account it authorises is one it read in the
   bytes it signed*, so it accepts **legacy transactions only**
   ([ADR 0003](0003-legacy-transactions-because-the-signer-must-be-able-to-read-them.md)).
2. Jupiter routes pump.fun **pre-graduation** liquidity only as **versioned**.
3. Radar selects pre-graduation pump.fun tokens exclusively.

**Re-verified 2026-09-01, on six mints the live instance decided on that day**,
because 0021 says the cheapest thing that could make this decision unnecessary is
a Jupiter change and that it should be re-checked before committing:

```
mint             versioned   legacy
3AhQ9iHjvtr7…       200        400
5RQgJVq3JPkd…       200        400
71rZF8o3qyfR…       200        400
GHBjkwQiuky8…       200        400
H2EdnMRoyWnu…       200        400
Hnnnv28Dbn5T…       200        400
```

The refusal is `{"error":"No routes found","errorCode":"NO_ROUTES_FOUND"}`. When
it does route, the venue label is `Pump.fun`. The control holds too: **BONK
routes legacy fine, through Whirlpool** — so this is pump.fun's curve
specifically and not a Jupiter legacy outage.

Six of six, months after the original eight of eight. Nothing has changed.

## Decision

**Radar builds pump.fun `buy` and `sell` instructions itself, and stops asking an
aggregator to build them.**

This is 0021's option A, and 0021 already argues it is the only option that
resolves the conflict rather than relocating it. That argument is not repeated
here. What follows is what 0021 did not say, and it is the reason this is not
merely plumbing.

### This is the cost lever, and cost is the only lever left

Take the three measurements together:

- [`0017`](../research/0017-a-control-that-could-have-been-traded.md) — **no
  edge**, 0 bps against a control that could actually have been traded. So
  improving the *filter* is not the move.
- [`0018`](../research/0018-the-deep-tail-points-the-wrong-way.md) — capacity is
  a **venue constant**: 80% of proposals sit in a ±13% band around $31, and the
  one band where a real position fits is down 68%. So selecting for *capacity* is
  not the move, and may be selecting for the trap.
- [`0019`](../research/0019-the-round-trip-is-not-one-number.md) — the round trip
  is **850 bps**, and at the median position of $6.21 it needs a +8.5% move to
  break even.

Two of the three doors are shut. The third is the round trip, and it is the one
nobody has tried.

**Going direct to the curve is a cost intervention, not only an execution fix.**
An aggregator route carries the aggregator's own accounts, its hop, and whatever
it takes; an instruction Radar builds against the bonding curve carries the
curve's accounts and nothing else. 0019 already measured that cost here has a
large *fixed* component — the lamport column is flat across the two smallest
buckets — and a fixed component is exactly what fewer accounts and fewer hops
reduce.

So the honest framing is: **this is the only remaining intervention that attacks
the binding constraint directly.** Not because it is certain to work, but because
the other two levers have been measured and are shut.

### What it does not claim

It does **not** claim the round trip will fall, and no number here says it will.
It claims the experiment becomes possible. `assumed_round_trip_bps` stays at 850
until something measures otherwise, in the direction
[`0019`](../research/0019-the-round-trip-is-not-one-number.md) insists on: a cost
estimate rounded down is the direction that launders a trade past the kernel.

It also does not claim an edge appears. 0017's null result is about the
selection, and nothing here touches the selection.

## What this costs

**An instruction builder maintained against a program Radar does not control.**
This is the real price and it is not small. pump.fun has already versioned its
instructions — `buy`/`buy_v2`, `sell`/`sell_v2`, `create`/`create_v2` — so the
churn is observed rather than hypothetical.

It is smaller than it first appears, and the reason is worth stating: Radar
**already** tracks those discriminators from mainnet, because
[`radar-decode`](../../crates/radar-decode/src/pumpfun.rs) has to decode them
whatever happens. Twenty-one instructions are named there with mainnet-observed
bytes and a test that recomputes `sha256("global:" + name)[..8]` against the
table. The marginal burden of this decision is therefore the **account lists**,
not the discriminators or the argument layouts, both of which exist.

**A second source of truth about the same program.** The decoder reads
instructions and the builder writes them, and they can disagree. That is a real
hazard and it has an obvious mitigation this ADR requires below: they must share
the discriminator table rather than each holding one.

**Losing the aggregator's price improvement.** Jupiter routes across venues; a
direct curve interaction takes the curve's price. For a pre-graduation pump.fun
token the curve *is* the only venue, so today this costs nothing — but it stops
being free the moment Radar trades anything with more than one market, and the
same is true of graduation, which happens to tokens Radar holds. **A position
that graduates while held must exit through a route, not through the curve.**
That is a consequence rather than an aside, and it is listed below.

## Alternatives rejected

0021 states these; they are recorded here with what settles each.

**B — resolve lookup tables in the signer.** Gives the signer a network, which is
the single thing its design is organised around not having. It does not weaken
the guarantee slightly; it removes the mechanism the guarantee rests on.

**C — trade only post-graduation tokens.** They route legacy fine. But
[`0008`](../research/0008-the-launch-block-gives-the-bundle-away.md) measured
that the fast money is committed before the token exists, and graduation is
exactly where Radar's measured signals stop being distinctive. It abandons the
cohort the research is about, which is a different product rather than a fix to
this one. **Worth revisiting only as the answer to the venue question, not as the
answer to this one** — and it is the venue question that
[`0018`](../research/0018-the-deep-tail-points-the-wrong-way.md) says now carries
the burden of proof.

**D — versioned transactions with caller-expanded tables.** The signer would
verify the expansion against the table's on-chain contents, which needs a network
call or a trusted snapshot. The same problem wearing a hat.

## What has to be true before this ships

1. **The account list comes from mainnet, not from a reference.** Every
   discriminator in `radar-decode` was captured from a real transaction and none
   copied from documentation, for the reason that file gives: public references
   describe a program with three instructions and the live one has twenty-one.
   The account list is held to the same standard.
2. **One discriminator table, shared.** The builder imports
   `radar_decode::pumpfun`, and a test asserts a built instruction decodes back
   to the instruction it claims to be. Two tables that can disagree about the
   same program is the failure this ADR would otherwise introduce.
3. **The signer must be able to read what the builder makes.** A test builds a
   real buy and runs it through `verify::check` against a matching
   `Authorization` — and, more importantly, against a **mismatched** one, to
   prove the check still refuses. A builder that produces something the signer
   cannot read has moved the problem rather than solved it.
4. **Simulated before it is believed.** `radar route`'s successor prints the
   transaction and stops. Whether it *works* cannot be established without
   sending one, and sending one is a decision about money — so the boundary
   stays where [`0021`](../research/0021-the-signer-cannot-read-the-only-venue-that-lists-them.md)
   put it: this delivers *a transaction Radar can build and the signer can read*,
   verified against a simulation, and the first real send is a separate
   deliberate act by a person.
5. **An exit path for a token that graduates while held.** The curve stops being
   the venue at graduation. Until this exists, a held position can become
   unsellable by the builder that bought it — which is
   [`0021`](../research/0021-the-signer-cannot-read-the-only-venue-that-lists-them.md)'s
   own failure in the opposite direction, and it would be worse, because it
   traps capital rather than refusing to deploy it.

## What would reverse this

Jupiter routing pump.fun pre-graduation liquidity as legacy. It is one HTTP call
to check and the check is now in this document; re-run it before doing the work,
and again if the builder ever becomes expensive to maintain.

## What this does not decide

**Whether Radar should be on this venue at all.** 0018 moved the burden of proof
onto *staying*, and this decision does not discharge it — it makes the venue
tradeable, which is a precondition for answering the question rather than an
answer to it. That decision is separate, it is about where the product lives, and
it should be taken on a measurement rather than on this one being finished.
