<!-- SPDX-License-Identifier: Apache-2.0 -->
# 0021 — The signer cannot read the only venue that lists them

**Date:** 2026-09-01
**Status:** measured
**Supersedes nothing. Blocks execution.**

## The finding

Radar cannot build a transaction for any token it selects.

Not "cannot profitably". Cannot build one at all. Jupiter returns **no route**
for every pre-graduation pump.fun mint Radar proposes, and it does so because of
a parameter Radar sends deliberately.

```
                              versioned   legacy
136xVadg3Y…pump                   ✓          ✗
13HWP6zA4h…pump                   ✓          ✗
1zZfyyb1Ki…pump                   ✓          ✗
24c9uNNSgV…pump                   ✓          ✗
25DSUF7NJm…pump                   ✓          ✗
25RMa4yy38…pump                   ✓          ✗
25UmZgzGMh…pump                   ✓          ✗
25UoBv9GuP…pump                   ✓          ✗
```

Eight of eight, drawn from the most recent decisions on the live instance. The
same query without `asLegacyTransaction=true` routes every one of them, through
`Pump.fun` as the venue label.

```
$ curl '…/quote?inputMint=SOL&outputMint=ut7bF5Vi…pump&amount=30000000'
{"outAmount":"1057436667569", … "label":"Pump.fun" …}

$ curl '…&asLegacyTransaction=true'
{"error":"No routes found","errorCode":"NO_ROUTES_FOUND"}   HTTP 400
```

## Why Radar sends that parameter

Because [ADR 0003](../adr/0003-legacy-transactions-because-the-signer-must-be-able-to-read-them.md)
requires it, and ADR 0003 is right.

A versioned transaction can name accounts through an **address lookup table** —
a pointer into on-chain state rather than bytes in the transaction. The signer's
guarantee is *every account it authorises is one it read in the bytes it signed.*
It cannot honour that for an account it can only resolve by making a network
call, and it deliberately has no network.

So the constraint chain is:

1. The signer must read every account → legacy transactions only.
2. Jupiter can only route pump.fun pre-graduation liquidity as versioned.
3. Radar selects pre-graduation pump.fun tokens exclusively.

**Any two of those are fine. All three cannot hold at once.**

## What this is not

It is not a market condition, a liquidity problem, or a transient Jupiter issue.
The tokens are routable right now; the venue is Pump.fun's own AMM; the
measurement above was taken twice, minutes apart, on live mints.

It is also not specific to small size. The same refusal appears at 0.03 SOL,
which is five times the median proposal.

And it is not universal: **BONK routes legacy fine**, through Whirlpool. Venues
that predate pump.fun's AMM are reachable. It is pump.fun's curve, which is the
only venue that lists a pre-graduation token, that is not.

## How it was found, which is the part worth keeping

`radar-exec` had **no production caller**. Its pipeline traits were satisfied
only by stubs inside their own test module, so the executor had only ever been
composed against fixtures. Everything from the transaction forward — Jupiter's
`/swap`, the lookup-table avoidance, the shape check — had never run.

`radar route` was written to run exactly that, and nothing else: no key, no RPC
endpoint, no flag that could be talked into signing. It found this on its second
invocation.

The first invocation found something smaller and in the same family: the command
had its own hardcoded Jupiter endpoint, `quote-api.jup.ag/v6`, which no longer
resolves — while `radar-exec`'s own constants had been pointing at the current
host all along. A diagnostic whose endpoint can drift from the thing it is
diagnosing reports on a system nobody runs.

Both are [LEARNINGS](../../LEARNINGS.md) 10's shape. That entry is about a live
run over 41,254 candidates raising zero proposals because a fixture had never
resembled a real candidate. The lesson recorded there was not "write more tests".
It was **run it against reality before trusting it** — and until today, the half
of the lane that touches reality never had been.

## What it costs, stated before the options

Every return figure Radar has is paper, and this makes a specific part of that
worse than it looked. `0017` and `0018` measure what the selection would have
returned *if it could be traded*. On the current execution design it could not
have been traded at all, at any size, for any of the tokens measured.

That does not change the conclusions — no edge, and capacity-bound — because
those are about the selection rather than the plumbing. It does mean the round
trip in them has never been available, not merely never taken.

## The options, and what each gives up

**A. Build pump.fun swaps directly, without Jupiter.** Radar already decodes
pump.fun's program; buying and selling on the bonding curve is a known
instruction set, and an instruction Radar builds itself names its accounts
explicitly and is legacy by construction. Keeps the signer's guarantee whole,
removes a vendor from the execution path, and is the only option that does not
weaken something. It costs an instruction-builder Radar must maintain against a
program it does not control.

**B. Resolve lookup tables in the signer.** Give the signer the ability to fetch
and expand a lookup table so it can read every account after all. This
contradicts ADR 0003 directly and gives the signer a network, which is the thing
its whole design is organised around not having.

**C. Trade only post-graduation tokens.** They route legacy through Raydium and
Whirlpool. But `0008` measured that the fast money is committed before the token
exists, and graduation is exactly when Radar's measured signals stop being
distinctive. It abandons the cohort the research is about.

**D. Accept versioned transactions with the lookup tables expanded by the
caller,** and have the signer verify the expansion against the table's on-chain
contents — which again needs a network call, or a trusted snapshot, which is the
same problem wearing a hat.

## Recommendation

**A.** It is the only one that resolves the conflict rather than relocating it,
and it is the one that makes Radar's execution independent of a vendor's routing
decisions — which is the same argument
[ADR 0001](../adr/0001-decode-locally-never-buy-parsed-transactions.md) already
made about decoding.

It is a decision about what Radar builds and maintains, so it belongs in an ADR
and to the owner, not to whoever noticed the problem.

## What would change this note

A Jupiter change that routes pump.fun liquidity as legacy. Worth re-checking
before committing to A, because it would make A unnecessary — and cheap to
re-check, since `radar route` now exists and answers it in one invocation.
