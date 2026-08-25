<!-- SPDX-License-Identifier: Apache-2.0 -->
# Freshness and caching: not paying twice for the same fact

When every call costs money, the cheapest call is the one you do not make. This document
is the discipline for that, and it is worth more to radar than any single data source
negotiation.

## The insight from ClawNet

ClawNet's internal design (`claw-net:internal/active/cache-layers-distinction.md`,
`claw-net:internal/active/soma-check-header-spec.md`) separates two layers that are usually conflated:

| | ClawNet cache (L1/L2) | Soma Check (ETag) |
|---|---|---|
| Origin touched on hit | **No** — served from Redis | **Yes** — origin validates hash, returns 304 |
| Key | `endpointId + normalised(params)` | content hash of the body |
| Latency | ~2 ms | ~50–500 ms |
| Answers | "have I asked this recently?" | "has the answer changed?" |

That second column is the one radar needs, and it is the one most caches do not have. Radar's
dominant access pattern is **not** "the same query twice in five minutes." It is *"I looked at
this token at slot N; it is now slot N+5000; what changed?"* — and for most tokens most of the
time, the honest answer is nothing.

A conditional request that returns `304 Not Modified` at a fraction of the full price is
exactly the right primitive for that pattern.

## Implementation status in claw-net (checked 2026-08-22)

Stating this precisely, because it is worse than simple doc drift and it is the reason radar
rebuilds this layer rather than calling into it.

**Implemented and present** — `src/cache/index.ts`: L1 in-memory map plus L2 Redis, keyed by
`sha256(endpointId + sorted params)`, single global `CACHE_TTL_SECONDS` (default 300). Sound,
working, and tracked in git.

**Implemented, then lost.** The billing layer was written and compiled, and only its build
output survives:

- `dist/core/credits.js` (dated 11 Apr) and `dist/core/credits.d.ts` (16 Mar) contain
  `creditCostForEndpoint`, `x402SurchargeCredits`, `cacheCreditCost`, `CREDITS_PER_USD` and
  `COST_MARKUP_FACTOR` — so the logic `claw-net:docs/reference/billing.md` documents did exist.
- `src/core/credits.ts` is **not in the working tree**, and `git ls-files` does not list it
  under any path. It was never committed, or was untracked before deletion.
- `dist/` is in `.gitignore`, so the only surviving copy is untracked build output on one
  machine. A clean clone rebuilds a service without it.
- The two surviving artifacts already disagree: `credits.js` says
  `COST_MARKUP_FACTOR = 1000` (1:1 raw cost) while `credits.d.ts` says `1500` (1.5×). They
  were compiled a month apart from different sources.

`src/routes/soma-check.ts` and `soma-check-billing.ts` are absent from both `src/` and `dist/`,
so Soma Check appears never to have been built — only specified, in
`claw-net:internal/active/soma-check-header-spec.md`.

**The correction that matters:** an earlier reading of this said the cache economics were
"designed but not built". That was wrong. They were built, and the source is gone. Worth raising
with claw-net independently of radar — recovering it means decompiling `dist/` or rewriting from
the docs.

For radar the conclusion is the same either way, and firmer: **do not depend on it.** Rebuild
the layer standalone in Rust, in `radar-provider`, where it is tracked, tested and typed.

## What radar does about it

### 1. Mutability classes — the biggest saving, available with zero upstream cooperation

Every field radar reads gets a declared mutability class, and the class decides the caching
rule. This is a property of the *data*, not of the provider, so it works no matter who serves it.

| Class | Examples | Rule |
|---|---|---|
| `immutable` | create slot, creator address, launch-slot transaction set, decimals, token program | **Fetch once, ever.** Keyed by mint. Never revalidated. |
| `latched` | mint authority revoked, freeze authority revoked, LP burned | Fetch until the latch closes, then never again. One-way transitions only. |
| `slow` | creator's prior launches, holder distribution tail | Revalidate on a slot budget (e.g. every 50k slots) |
| `fast` | reserves, price, top-holder balances | Revalidate on a short slot budget, or on an event we already saw for free |
| `realtime` | route quote, exit simulation | Never cached; always live at decision time |

The `immutable` and `latched` classes are where the money is. A token's structural facts are
fixed within seconds of launch, and `token_structure` is a Tier-0 input consulted on every
re-evaluation. Fetching it once per mint instead of once per evaluation is a large multiple on
the enrichment bill and needs nothing from any vendor.

**A latch may only close, never open.** Mint authority, once revoked, cannot be restored. Model
these as one-way and assert it: if a revalidation ever shows a latch reopening, that is either
a provider bug or an attack, and it must raise rather than silently update.

### 2. Content-hash everything, key by `(instrument, args, as_of)`

Radar already records every instrument invocation with its point-in-time watermark (plan §3.1).
Adding the response hash to that record gives change detection for free, and makes
`If-None-Match` usable the moment any upstream supports it.

### 3. Free revalidation from data we already have

The discovery heartbeat is already streaming us the transactions that touch watched programs.
If no transaction has touched a mint's pool since slot N, its reserves have not changed and
**no paid call is needed to know that.** The heartbeat doubles as a free invalidation signal
for the `fast` class. This is the single highest-leverage interaction between the two lanes,
and it only works because we decode locally (ADR 0001) rather than buying parsed data.

### 4. Rebuild standalone, do not call into claw-net

Radar implements this layer itself in `radar-provider`. The reasons compound: the billing source
is lost, Soma Check was never built, claw-net has a crash-loop history on the shared VPS, and it
is a Node service radar would otherwise have to keep alive to make a trade.

What radar takes from it is the **design** — the two-layer split, the mutability thinking, and
the insight that a revalidation is worth a fraction of a fetch. What radar does differently:
the classes are types rather than config, the latch invariant is enforced by the compiler, costs
are integers rather than floats, and single-flight coalescing and stale-while-revalidate are
built rather than documented.

If claw-net later exposes a working, tracked cache-pricing API, it can be added as one lane
behind the same health scoring and circuit breaker as any other provider. A lane, never a
dependency.

### 5. Ask upstream for conditional requests

See request #11 in `0002-clawapis-feature-requests.md`. For Solana specifically there is a
primitive better than generic ETag: responses already carry `context.slot`, so a
*"return this only if it changed since slot N, else 304"* is exact, cheap to implement, and
cheaper for the provider to serve than a full body.
