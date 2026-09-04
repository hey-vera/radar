<!-- SPDX-License-Identifier: Apache-2.0 -->
# Feature requests for clawapis

Ordered by leverage to radar. Each item states what it costs us today, what the change
would save, and why it is plausibly worth it to clawapis as well — these are not favours,
most of them reduce their own costs or widen their market.

**Status:** written, **not sent.** Eleven asks; nothing has been requested of
clawapis and no item here has been agreed by anyone. Items 1 and 3 are the two
that would let Radar buy all of its data pay-per-call and drop the flat-rate
heartbeat account.

Context: radar is a Solana research/trading platform that wants to buy **all** its data
pay-per-call rather than hold flat-rate subscriptions. Today three things stop that from
being possible. Items 1–4 are those three things.

---

## 1. Solana payment channels (or a prepaid escrow balance) — highest value

**Today.** The `exact` scheme settles on-chain before the response returns: **400–800 ms**
added latency and ~$0.0005 of settlement cost on every call.

**With channels.** Published Solana x402 channel implementations verify a signed Ed25519
claim in **<10 ms** at **$0 per payment**, with two on-chain transactions total (open and
close) instead of one per request. That is a 40–80× latency improvement and a ~99.7%
reduction in settlement cost. SDKs already exist (`@solana-payment-channel/client`,
`@x402-solana/core`), and the x402 spec has a directly analogous `batch-settlement` scheme
— escrow plus off-chain vouchers — that is currently **EVM-only**.

**Why we care.** At 400–800 ms per call, x402 can only serve offline analysis; anything
latency-sensitive has to hold a separate flat-rate account. At <10 ms it can serve the whole
system, and we would drop every other subscription.

**Why you might care.** It removes a per-request on-chain cost from your own margin, and no
other x402 gateway we surveyed offers it on Solana. It is the clearest differentiator
available in the category right now.

---

## 2. Per-HTTP-request metering, and a price for batch JSON-RPC

**Question first:** is a JSON-RPC batch array of 50 calls billed as **1 request or 50**?

Solana's JSON-RPC accepts batch arrays, and `getMultipleAccounts` takes **up to 100 accounts
in a single call**. Our enrichment path reads 3–5 accounts per token and our graph work reads
far more.

**Ask.** Bill per HTTP request, or offer an explicit batch price — e.g. `$0.001` for the
request plus `$0.0002` per additional call in the batch. Either way, document it, because the
answer changes how we write every call site.

**Leverage:** 10–100× on the account-read path, and it costs you less upstream too.

---

## 3. A program-filtered slot-range endpoint

**Ask.** Something shaped like:

```
getBlocksFiltered(start_slot, end_slot, program_ids[]) -> [transactions]
```

returning only the transactions in that slot range that touch the given programs.

**Today** watching pump.fun means either polling `getSignaturesForAddress` at ~1 Hz
(~$2,600/mo at $0.001/call) or pulling whole blocks (~$6,480/mo). Both are absurd for what is
actually a small amount of data. The filtering is trivial server-side and **cheaper for you to
serve** than shipping us whole blocks we discard 99% of.

**This is the single request that would let us buy 100% of our data pay-per-call.** Right now
we have to hold a cheap flat-rate account purely for the discovery heartbeat, and we would
rather not.

---

## 4. Streaming passthrough priced per connection-hour

Alternative to #3 if slot-range filtering is harder than it looks: a `logsSubscribe` /
`programSubscribe` WebSocket passthrough billed **per connection-hour** rather than per
message. Even $0.02/hour is $14.40/mo for a persistent program subscription — cheaper than
any flat tier on the market and it stays inside the pay-per-use model.

---

## 5. Idempotency keys

If we pay for a call and the response is lost to a network drop, we currently pay again on
retry. **Ask:** an `Idempotency-Key` header where a retry within N minutes returns the cached
response without re-charging. Standard practice for metered APIs, and it removes a class of
disputes for you.

---

## 6. A usage/spend API

An endpoint returning spend-to-date and a per-endpoint breakdown. We enforce hard budget caps
in code and today we can only *estimate* our own spend; we would rather reconcile against your
numbers. Useful to any customer running an automated budget.

---

## 7. Cache-hit pricing

When many agents request the same token's holders inside a short TTL, you serve one upstream
call. A discounted cache-hit price (say $0.002 against $0.01) would align your margin with
ours and make you the cheapest option for exactly the popular queries that cost you least.

---

## 8. Confirm the LLM endpoint catalogue

Public docs and `install.sh` list only X + Helius + SolScan. We understand there are 252+
endpoints including OpenAI, Anthropic, Gemini, Groq and Grok. We need:

- the actual endpoint list and per-endpoint pricing,
- whether streaming (SSE) responses are supported for the LLM endpoints,
- whether request/response bodies are logged or retained.

The last one matters to us specifically — we would be sending trading-relevant context.

---

## 9. A stateless MCP surface

The **MCP 2026-07-28** spec is stateless-first: no sessions, no `initialize` handshake, and
required `Mcp-Method` / `Mcp-Name` headers on every POST. That last detail matters
commercially — **a paywall can price per tool from the header without parsing the JSON body**,
which makes x402-per-tool-call almost free to implement.

Exposing the clawapis catalogue as a stateless MCP server would let any agent integrate with
zero custom code. Cloudflare shipped a Monetization Gateway and AWS shipped an equivalent WAF
action in July 2026, and MCPay already does this generically — it is becoming table stakes
rather than a differentiator, which is the argument for doing it soon.

---

## 10. Jupiter / DEX quote passthrough

Not currently in the catalogue. Routing quotes are the one hot dependency we would otherwise
have to source elsewhere; having them in the same metered lane would consolidate our spend
with you rather than splitting it.

---

## 11. Conditional requests: charge a fraction for "nothing changed"

**This is the second-highest-value item after payment channels, and it may be the easiest.**

Our dominant access pattern is not "the same query twice in a minute." It is *"I read this at
slot N; it is now slot N+5000; what changed?"* For most tokens most of the time the answer is
nothing, and today we pay full price to learn that.

**Generic ask:** support `If-None-Match` / `ETag` (RFC 9111) and price a `304 Not Modified` at a
fraction of a full response — 10% would match what comparable gateways charge.

**Better, Solana-specific ask:** every RPC response already carries `context.slot`. A parameter
like `onlyIfChangedSince: <slot>` returning `304` when the account or program state has not been
written since that slot is semantically exact, trivial server-side, and **cheaper for you to
serve** than serialising a body we are going to discard.

Either version cuts our revalidation spend by roughly an order of magnitude and cuts your
upstream and bandwidth costs on the same calls. It is the rare ask where both sides save.

---

## Summary of what we would buy

If #1 and #3 land, radar buys **100% of its data from clawapis pay-per-call** and holds no
other data subscription. Without them we hold a $5–49/mo flat account for the heartbeat and
route only enrichment through x402 — perhaps a third of the volume.
