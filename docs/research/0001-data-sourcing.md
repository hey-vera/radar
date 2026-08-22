<!-- SPDX-License-Identifier: Apache-2.0 -->
# Data sourcing: the problem set and what exists to solve it

Researched 2026-08-22. Every price and status here needs re-checking before it is relied on;
this market moves monthly. Prices are list prices, not negotiated.

Four problems stand between radar and a working recorder. This document is what exists to
solve each one.

---

## Problem 1 — the discovery heartbeat

**The problem.** Something must notice new launches continuously. Priced per call this is
brutal: `getBlock` every slot is ~216k calls/day (**~$6,480/mo** at $0.001), and
`getSignaturesForAddress` at 1 Hz is ~86k calls/day (**~$2,600/mo**). Neither is defensible
for what amounts to a few MB of interesting data per day.

**What exists.** Flat-rate free and near-free tiers are far better than expected:

| Provider | Free tier | First paid tier |
|---|---|---|
| dRPC | 210M CU/mo (public nodes) | — |
| Alchemy | 30M CU/mo | — |
| QuickNode | 10M credits/mo | — |
| Syndica | 10M req/mo | — |
| Chainstack | 3M req/mo, 25 RPS | **$5/mo for 20M req** |
| Helius | 1M credits, 10 RPS | $49/mo for 10M credits |
| Subglow | — | $99/mo flat, Yellowstone gRPC, 2 streams, no credits |

**Recommendation.** **Chainstack Growth at $5/mo for 20M requests** — 666k calls/day, ~7.7/s
sustained, which covers a 1 Hz poll with an order of magnitude spare. This is the cheapest
credible answer and it makes the Lane A / Lane B split cost essentially nothing.

Subglow at $99/mo flat is the upgrade path if we ever want a true Yellowstone stream; it is
5× cheaper than Helius's $499 LaserStream floor and speaks the open Dragon's Mouth protocol
rather than a proprietary one, so it stays swappable.

**Open ask.** Feature request #3 to clawapis (a program-filtered slot-range endpoint) would
remove the need for any flat account. See `0002-clawapis-feature-requests.md`.

---

## Problem 2 — cold start

**The problem.** Creator history, outcome labels and signal validation all need *past* data.
Recording forward means waiting weeks before Phase 2 can say anything, and the original plan
treated that delay as unavoidable.

**It is not.** Historical pump.fun data is a commodity:

**NoLimitNodes historic pump.fun archive** — every create, trade and graduation since the
program deployed. ~14M tokens, ~1.8B trades, ~38k graduations. Seven Parquet tables:

| Table | Why it matters to radar |
|---|---|
| `pumpfun_creates` | mint, creator wallet, virtual reserves, metadata URI |
| `pumpfun_trades` | side, SOL/token amounts, USD, fees, post-trade reserves |
| `pumpfun_graduations` | graduation slot, final reserves, target AMM |
| `pumpfun_priority_fees` | CU price, priority fees, **Jito tips per transaction** |
| `pumpfun_creator_aggregates` | tokens launched, graduation rate, time-to-graduation |
| `pumpfun_token_lifecycle` | create slot, **peak market cap**, graduation flag |
| `pumpfun_post_graduation_trades` | PumpSwap and Raydium activity after graduation |

Parquet + CSV, decimal-normalised, USD-stamped, monthly bundles at 6–15 GB/month compressed.
**$200/mo** rolling; one-time historical pulls quoted separately.

Three of those tables are worth more than their price. `token_lifecycle` with peak market cap
is **outcome labels for free** — the single hardest thing to construct correctly and the thing
every signal has to be validated against. `creator_aggregates` is Phase 1's `creator_history`
instrument, pre-computed. `priority_fees` carries per-transaction Jito tips, which is
simultaneously the Tier-A coordination evidence and the historical execution cost model.

**Recommendation.** Buy a **one-time historical pull** of the last 6–12 months, not the
rolling subscription. Radar's own recorder handles forward data at ~1 GB/month. This collapses
the Phase 2 wait from weeks to zero for one payment instead of $200/mo indefinitely, and it
keeps the guardian VPS off a 6–15 GB/month growth curve.

Also note **PumpArchive** (GitHub, free) — archives token metadata, websites, screenshots and
social links rather than trades. Complementary rather than competing: it is the only source
seen for *off-chain* launch artefacts, which is where creator-linkage evidence lives. Caveat:
two commits, no licence stated, no bulk download — API only. Treat as unproven.

**Bias warning.** A purchased archive is a point-in-time reconstruction by someone else. Before
any of it informs a signal, spot-check a sample of rows against `getBlock` at the same slot.
If their reconstruction has look-ahead in it, ours inherits it.

---

## Problem 3 — per-call latency

**The problem.** x402's `exact` scheme settles on-chain before responding: **400–800 ms**. That
is fine for analysis and fatal for execution, which is why the plan forbids x402 on the
execution path.

**What exists.** Solana x402 **payment channels**: a signed Ed25519 claim verified in **<10 ms**
at **$0 per payment**, two on-chain transactions total (open + close) — a 40–80× latency
improvement and ~99.7% settlement cost reduction. SDKs exist today
(`@solana-payment-channel/client`, `@x402-solana/core`). The x402 spec's `batch-settlement`
scheme does the same thing via escrow plus off-chain vouchers but is **EVM-only**; an escrow
scheme is in flight upstream.

**Recommendation.** Ask clawapis for it (request #1). If it lands, the hot-path exclusion in
the plan can be relaxed and the flat-rate account disappears. Until then the exclusion stands
and is enforced by type.

---

## Problem 4 — the cost of parsed data

**The problem.** Parsed/enhanced transactions cost $0.05 each. A launch analysis touches
50–200 transactions, so buying them parsed is $2.50–$10.00 **per token**.

**What exists.** `getBlock` returns every transaction in a slot for one $0.001 call, and the
launch slot contains the create, the dev buy and every same-slot coordinated buy. Decoding is
a solved problem in Rust: **Carbon** (sevenlabs-hq) ships ~40 pre-built decoder crates and
generates decoders from Anchor IDLs, and it is built for exactly this.

**Recommendation.** Own the decoders. Use Carbon's decoder crates where they fit and generate
from IDL where they do not, but keep the decode step in our process. This is ADR 0001 and it
is worth ~100–300× on the data bill.

**Open question** (request #2): whether a JSON-RPC batch of 50 calls bills as 1 or 50, and
whether `getMultipleAccounts` (100 accounts per call) bills as one request. If it bills per
request, the account-read path gets 10–100× cheaper for free.

---

## Sources

Helius pricing and historical data; Chainstack, Alchemy, QuickNode, Syndica, dRPC free tiers;
Subglow gRPC pricing; NoLimitNodes historic pump.fun; PumpArchive; x402 docs and the Solana
x402 payment-channel implementation; Carbon indexing framework; Solana JSON-RPC
`getMultipleAccounts` limits.
