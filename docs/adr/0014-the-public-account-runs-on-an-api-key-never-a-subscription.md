<!-- SPDX-License-Identifier: Apache-2.0 -->
# ADR 0014 — The public account runs on an API key, never a subscription

**Status:** accepted, 2026-09-07.

**Decision:** `@thecabalhunter`'s reply layer is authenticated with a metered
API key. The `RADAR_MODEL_CODEX` subscription path stays in the code for local
and private use, and **must never be configured on the box that runs the public
account.**

**Context:** Josh asked directly whether the Codex/ChatGPT subscription pathway
could carry the bot's LLM layer — 24/7, reliably, and legally.

---

## Why this is its own ADR and not a line in 0004

[ADR 0004](0004-radar-spawns-the-vendor-cli-rather-than-holding-an-oauth-token.md)
already says the subscription path is private-use-only, and it gives one
reason: *"a personal subscription credential is not a foundation for a sold
product."*

Radar was a sold product when that was written. **The public account is not
sold**, so the question is genuinely open again rather than already answered,
and it deserves an answer rather than a pointer at a sentence written about
something else. The answer turns out to be the same, for two reasons neither of
which is about selling.

## The two reasons

**1. It is not the supported pathway for this, and the vendor says so.** The
documentation splits the two sign-in methods by use, not by scale: plan sign-in
for interactive work in the CLI, the web app and the IDE extension; **API keys
for CI, automation and scripted runs**. A bot that answers strangers on a
24/7 loop is the second, whatever its billing looks like. Building on the first
is building against a documented boundary and hoping.

**2. The usage is not personal any more, and that is the line the terms
draw.** The subscription is one person's, and once a system makes calls with it
while that person is not in the loop, the usage has stopped being personal. A
public account that answers whoever mentions it is exactly that. Selling is not
required for it to be true — Josh being asleep is enough.

## Reliability, which is the part that would bite first

Even setting the terms aside, this is the wrong shape for an unattended service.

- **A shared five-hour rolling window.** A plan's included usage is shared
  across the CLI, the web app and the IDE extension. Josh using ChatGPT in a
  browser would take budget from the bot, and the bot would take it from him.
  Two consumers of one quota, neither aware of the other.
- **The credential is single-writer** (ADR 0004). It has to be re-authorised by
  hand if it lapses, and there is no way to automate that which is not the
  option ADR 0004 rejected.
- **A rate limit reached is silence, not a refusal Radar can meter.** The spend
  meter would show a call it never made; `provider_notice` would say a provider
  is configured; the log would fill with `fellback`. The template is a good
  fallback for an outage and a bad one for a permanent state.

## What this costs, said plainly

It costs money that the subscription would not. That is the trade, and it is
small: the account makes at most a few hundred calls a day at the admission
gate's caps, on a model chosen for cost, and `radar model-prices` prints the
rates beside the configured ones.

## What we do instead

Nothing changes. The OpenAI API key is already the configured path and the
account already runs on it. This ADR exists so that the next person who
notices `RADAR_MODEL_CODEX` in the source and thinks it would save a few
dollars finds the reason it is not set, rather than the variable.

## The one thing that would reopen this

A **business or enterprise** agreement, which is a different contract with
different terms and is not a personal subscription with more quota. If Radar
ever has one, this ADR is reconsidered on its actual text — not on the
resemblance of the two products' names.

## Related

- [ADR 0004](0004-radar-spawns-the-vendor-cli-rather-than-holding-an-oauth-token.md)
  — why the subscription path spawns the vendor CLI rather than holding a token
- [Design 0014](../design/0014-the-four-asks-answered.md) — the model provider
  as the answer to "smarter replies", and X's own approval clause for AI reply
  bots, which is a separate open item
