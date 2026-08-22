<!-- SPDX-License-Identifier: Apache-2.0 -->
# ADR 0001 — Decode transactions locally; never buy parsed ones

**Status:** accepted, 2026-08-22. Measured against mainnet the same day; see
*Measured outcome* below, which confirms the decision and adds a constraint the
original did not anticipate.

## Context

Radar buys its data per call. The vendor catalogue that shapes everything:

| Call | Price |
|---|---|
| Standard RPC (`getBlock`, `getTransaction`, `getAccountInfo`) | **$0.001** |
| DAS / asset | $0.005 |
| **Enhanced / parsed transactions** | **$0.05** |
| SolScan Pro (holders, metadata) | $0.01 |

Analysing a token launch means understanding the create transaction, the dev buy,
and the buys that landed alongside them — 50 to 200 transactions.

Bought parsed, that is **$2.50–$10.00 per token**. At a thousand launches a day
it is $2,500–10,000 a day. There is no position size that justifies it.

But `getBlock` returns **every transaction in a slot for one $0.001 call**, and
the launch slot is precisely where the interesting transactions are: the create,
the dev buy, and every same-slot coordinated buy sit in the same block, in
order, with their transaction indices intact.

## Decision

**Radar fetches raw blocks and transactions at standard-RPC prices and decodes
them itself, in Rust. It never buys parsed or enhanced transaction data.**

Decoding lives in `radar-decode`, which owns program-specific decoders for
pump.fun, PumpSwap, Raydium, Meteora, Orca, the Token program and Token-2022.
Where Carbon (`sevenlabs-hq/carbon`) ships a decoder that fits, use it; generate
from the Anchor IDL where it does not.

## Consequences

**The bill drops by roughly 100–300×** on the analysis path. This single decision
is the difference between a system that can afford to look at every launch and
one that can afford to look at a handful.

**It is the justification for the Rust layer.** The original brief argued for Rust
on performance grounds. Performance is real but secondary — a Python decoder
would also be fast enough at these volumes. The actual argument is economic:
decoding is the step where a vendor charges fifty times what the raw material
costs, so it is the step worth owning.

**Same-slot ordering comes free.** Coordination detection needs to know which
transactions landed together and in what order, plus whether any of them paid a
known Jito tip account. All of that is in the block we already fetched. Buying
transactions one at a time would discard exactly the ordering the analysis needs
and charge fifty times more for the privilege.

**Free cache invalidation comes free too.** Because the discovery heartbeat is
already decoding the transactions that touch watched programs, Radar knows
whether a pool has been touched since slot N without asking anyone. That turns
the `Fast` mutability class from "revalidate on a timer" into "revalidate when
something actually happened" — which is only possible because the decode is ours.

**The cost is correctness risk we now own.** A vendor's parser is battle-tested
against programs we have never seen; ours is not. Mitigations:

- Decoders are pure functions over bytes, so they are exhaustively testable
  against recorded fixtures with no network.
- Every decoder carries fixtures captured from real mainnet transactions.
- An unknown instruction discriminator is recorded as unknown, never guessed. A
  decoder that silently mis-parses is worse than one that admits it cannot read
  something, because the first produces confident wrong analysis.
- Sampled cross-checks against a parsed source are cheap at $0.05 occasional and
  are worth running as a conformance test rather than a hot path.

**Program upgrades break decoders.** pump.fun and the AMMs ship changes. The
heartbeat must alarm on a rising unknown-discriminator rate rather than quietly
dropping events, because a decoder that has stopped understanding a program looks
exactly like a program that has gone quiet.

## Measured outcome

Probed against mainnet slots 440972500–440972965 (`scripts/probe/`):

| | |
|---|---|
| Mean transactions per block | 1,513 |
| Mean block size | **6.88 MiB** |
| pump.fun transactions in 45 slots | 4,173 |
| Cost as `getBlock` | $0.045 |
| Cost as parsed transactions | $208.65 |
| **Measured ratio** | **4,637×** |

The estimate above was 100–300×. The measured figure is an order of magnitude
better, because a busy slot carries far more pump.fun transactions than assumed.

**But the price is not the only cost, and this is the correction.** At 6.88 MiB
per block and ~2.5 slots per second, polling every slot would pull **~1,451
GiB/day**. Nothing in the original decision accounted for bandwidth, and on a
shared VPS that alone would sink the design.

So the decision is amended: **a full block fetch is for qualified candidates,
never for discovery.**

1. Discover Creates from a cheap WebSocket `logsSubscribe` — log lines, not blocks.
2. Tier-0 filter on the create transaction alone (`getTransaction`, ~20 KiB).
3. Fetch the whole block **only for candidates that survive**, for the same-slot
   coordination picture.

At the measured 0.24 launches per slot and a 90% Tier-0 kill rate that is roughly
5,000 block fetches a day, about 35 GiB — carryable. Without the filter it is 1.4 TB
and it is not.

It also raises the value of clawapis request #3 (a program-filtered slot-range
endpoint): returning 50–200 KiB instead of 6.88 MiB is a ~40× bandwidth reduction
on the path that dominates, and it is cheaper for the vendor to serve than shipping
whole blocks we discard 99% of.

The same probe found pump.fun running at least four buy variants, a
`BondingCurveV3`, per-user volume accumulators and a cashback mechanism — none of
which appear in public references. The "decoders are built from the chain, not from
prose" consequence below is therefore not a precaution; it is already true.

## Alternatives considered

**Buy parsed transactions.** Rejected on price: $2.50–$10.00 per token analysed.

**Buy parsed only for tokens that pass earlier filters.** Better, and still
rejected. Tier-0 filtering removes 90%+ of candidates, which brings the bill to
$250–1,000/day — still one to two orders of magnitude above decoding, for data
we would then have to reconcile against our own model anyway.

**Buy a historical archive and decode only live data.** Not an alternative but a
complement, and adopted separately: a one-time purchase of pump.fun history
supplies outcome labels and creator history that would otherwise take weeks to
accumulate. It does not remove the need to decode live.
