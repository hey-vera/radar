<!-- SPDX-License-Identifier: Apache-2.0 -->
# ADR 0010 — Entitlement is read from Stripe, never recorded

**Date:** 2026-09-01
**Status:** accepted
**Decides:** how Radar knows whether a customer has paid, and what it keeps
about that.

## Context

[ADR 0009](0009-radar-builds-its-own-pump-fun-swaps.md) is what makes the
product able to act. This one is about who may ask it to.

The requirement is a monthly subscription: a person signs in, pays, and from
then on may use the product; if they stop paying, they may not. Radar therefore
needs to answer one question on a request — **is this identity entitled?** — and
the whole of this decision is *where that answer comes from*.

[ADR 0006](0006-radar-records-only-what-it-cannot-recover.md) already settles
the shape of the answer, and its amendment of the same date sharpens the test:
Radar records customer state it **cannot recover**, and nothing else. So the
question is whether entitlement is recoverable, and from whom.

It is. **Stripe is the authority on whether a subscription is active** — it is
the thing that took the money, the thing that will fail a renewal, and the thing
a customer disputes with. A Radar-side copy would be a second source of truth
that can disagree with the first, and when it disagrees the local one is wrong
by construction.

## Decision

**Entitlement is read from Stripe at the point of use, cached with a short
lifetime, and never recorded as fact.** This is exactly the treatment
[ADR 0006](0006-radar-records-only-what-it-cannot-recover.md) gives the wallet
address, for the same reason.

Three consequences follow, and each is a thing deliberately not built.

### No local subscription table

There is no `subscriptions` table, no `customers` table, and no `entitled`
column. A request carries a verified Privy DID; Radar asks Stripe what that
identity's subscription status is; the answer decides the request.

### No webhooks

This is the part most likely to be argued with, so the argument is written down.

The conventional design is: Stripe pushes `customer.subscription.*` events to an
endpoint, Radar verifies the signature and updates its own copy. That copy is
then the thing requests are served from.

Radar declines that, and not to save work — it is *more* work to poll:

- A webhook is **the only notice you get**, which makes the local copy
  load-bearing. A missed delivery, a deploy during a retry window, or a
  signature check that fails for an hour all leave Radar serving a stale
  entitlement, and nothing in the system knows. Polling has no such state to be
  stale.
- It requires a **public unauthenticated endpoint** on `radar-serve`, which is
  already described in [ADR 0007](0007-the-privy-authorization-key-lives-in-the-signer-process.md)
  as the largest attack surface in the system, plus a shared secret to verify
  with.
- It reintroduces exactly the durable customer state ADR 0006 exists to refuse.

**What declining costs:** a round trip to Stripe on entitlement-gated requests,
and an availability dependency — when Stripe is down, Radar cannot tell who has
paid. That is the same trade ADR 0006 already took on the wallet address, and
the same answer applies: rule 8, deny by default. A customer whose entitlement
cannot be checked is not an entitled customer.

That is a real cost to a paying person during someone else's outage, and it is
mitigated by the cache below rather than by guessing.

### The link is metadata on Stripe's side, not a mapping on Radar's

Radar needs Privy DID → Stripe customer. Storing that mapping is storing customer
state, so it is stored on **Stripe**: the customer object carries
`metadata.privy_did`, set when Checkout is created, and Radar finds it with
Stripe's search API.

The DID is also passed as `client_reference_id` on the Checkout session, which is
what ties a completed payment back to the identity that started it.

## The cache, and what it may and may not do

A short-lived cache, keyed by DID, holding the answer and when it was taken.

**It may serve a fresh positive.** Sixty seconds is enough to stop a page that
makes six requests making six Stripe calls, and short enough that a cancellation
takes effect while the customer is still looking at the screen.

**It may not extend a positive through an outage.** When Stripe cannot be
reached and the entry is stale, the answer is "cannot tell", which is a refusal.
A cache that falls back to its last good answer is a cache that grants access on
the strength of a payment that may have failed a week ago — and the failure mode
is silent and open-ended, which is the direction rule 8 exists to forbid.

**A negative is not cached at all.** Someone who has just paid must not wait out
a TTL to use what they bought, and the volume of refused requests is not a load
problem.

## What this does not decide

**Price, plan shape, or trial.** Those are product decisions and they live in
Stripe's dashboard, which is the point of using it — none of them should require
a deploy.

**What an unentitled visitor may see.** That is the frontend's tiering question,
and it is settled separately: the honest public record — the decision funnel, the
capacity wall, the null result — is *better* marketing than a paywall over it,
because those figures are the argument that Radar can be trusted with money.
What entitlement gates is the reading assistant, the wallet, and trading, all of
which cost Radar money or move a customer's.

**Whether the product is worth a subscription.** It is not, yet: 0017 finds no
edge, 0018 finds the venue cannot host a real position, and
[`0021`](../research/0021-the-signer-cannot-read-the-only-venue-that-lists-them.md)
finds Radar cannot currently execute at all. This ADR settles how billing works
so that it is not designed in a hurry later. **It is not a statement that billing
should be switched on**, and the ordering matters: ADR 0009 before this one.

## What has to be true before this ships

1. **A restricted Stripe key**, not a secret key. Read on customers and
   subscriptions, write on Checkout sessions, and nothing else. It lives in
   `radar-serve`'s environment, which is where the Privy *application* credential
   already lives — it authenticates the application and authorises no movement of
   a customer's funds, which is the distinction
   [ADR 0007](0007-the-privy-authorization-key-lives-in-the-signer-process.md)
   draws.
2. **Checkout and the customer portal are Stripe's hosted pages.** Radar never
   sees a card number, and there is no code path in which it could.
3. **Entitlement is checked where money is spent, not at the router.** A paywall
   in the guard is a paywall that gets forgotten on the next route; the checks
   belong on `/v1/chat`, on the wallet routes and on anything that trades — the
   places that already know they are expensive.
4. **A refusal says which of the two it is.** "You are not subscribed" and "we
   cannot reach Stripe" want different actions from a customer, and collapsing
   them is the failure this repository keeps recording.

## What would reverse this

A measured Stripe rate limit or latency that makes per-request lookup
impractical, in which case the answer is a longer cache before it is a webhook.
A webhook only becomes right if Radar needs to *act* on a cancellation rather
than merely refuse the next request — closing a position, say — which is a
different product than this one.
