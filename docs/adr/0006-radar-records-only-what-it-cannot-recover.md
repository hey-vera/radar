<!-- SPDX-License-Identifier: Apache-2.0 -->
# ADR 0006 — Radar records only the customer state it cannot recover

**Date:** 2026-08-31
**Status:** accepted
**Decides:** who owns customer account state, and therefore what the customer
backend persists.

## Context

[ADR 0005](0005-customers-keep-custody-and-grant-radar-a-bounded-signer.md)
settles custody: the customer keeps it, and grants Radar a bounded signer whose
policy derives from the risk kernel's `Policy`. It does not say **where a
customer's account state lives**, and that question has to be answered before the
first line of the customer backend is written, because the answer is load-bearing
for what gets built and is expensive to reverse afterwards.

This ADR exists because the decision was about to be made by implication. The
first move on the customer backend was to create a crate with an `Account` type
in it — which silently answers "Radar owns customer state, in its own store",
without the alternative ever having been stated. That is the shape of mistake
this repository keeps recording: a structure that is coherent, and never argued.

### What a customer account actually consists of

Enumerated rather than assumed, because the enumeration is what decides it.

| state | where it already lives | recoverable? |
|---|---|---|
| identity (the Privy DID) | the access token's `sub`, on every request | **yes** — it arrives with the request |
| wallet address | Privy, `GET /v1/users/{did}` | **yes** — one call, authoritative |
| the grant and its bounds | Privy's policy engine, which enforces it | **yes** — and Privy's copy is the one that binds |
| deposits and withdrawals | the Solana chain | **yes** — and Radar already records chain events |
| positions and fills | the Solana chain, plus Radar's own `Positions` table | **yes** |
| **signatures made on the customer's behalf** | nowhere durable | **no** |

Five of six are already held by something that is more authoritative than a
Radar-side copy would be. A local mirror of any of them is a second source of
truth that can diverge from the first, and when it diverges the local one is
wrong by construction — Privy's policy is what actually refuses a transaction,
and the chain is what actually holds the balance.

The sixth is different, and [ADR 0005](0005-customers-keep-custody-and-grant-radar-a-bounded-signer.md)'s
precondition 5 already says why: the per-customer signature count decides whether
Privy's pricing (50,000 included, $0.01 each above) stays acceptable, and it
**cannot be taken retroactively**. Privy reports an aggregate on a dashboard; it
does not hand back a per-customer history for a month nobody was counting.

## Decision

**Radar persists exactly one thing about a customer: the signature meter. Every
other piece of account state is read from its owner at the point of use.**

Concretely:

- **The DID is not stored.** It arrives in the verified token on every request,
  which is the only moment it is trustworthy anyway. A stored DID is a stored
  identifier with no question it can answer that the token cannot.
- **The wallet address is fetched, not mirrored.** Privy is authoritative; a
  cached address that is stale is an address Radar might send funds to that the
  customer no longer controls. It may be *cached* with a lifetime, never
  *recorded* as fact.
- **The grant is Privy's.** Radar derives the bounds from the kernel's `Policy`
  and hands them over; Privy enforces them. Radar keeps the derivation, which is
  code, not state.
- **The signature meter is recorded**, keyed by a salted hash of the DID rather
  than the DID itself, and it holds a count and a day. Nothing else.

### Why the hash, given that Radar sees the DID anyway

Not privacy theatre, and worth stating precisely because it would otherwise look
like it. Radar *does* see the DID on every request — the hash does not hide it
from the running system. What it does is keep the DID out of the **durable**
artefact, so the file that outlives the request cannot be joined against anything
else, and so a store that is copied for research carries a count rather than a
customer list. The threat is not the live path; it is every copy of the store
that will exist later.

The salt is per-instance and comes from configuration. With no salt configured
the meter refuses to write, per rule 8 — a meter that silently falls back to
recording raw identifiers is worse than one that stops.

## What this costs, and the case against

**It costs a round trip to Privy on paths that need the wallet address**, where a
local mirror would cost nothing. That is a real latency and availability cost:
when Privy is down, Radar cannot look up a wallet, and a mirror would have kept
working.

That case is weaker than it looks. When Privy is down, Radar also cannot *sign* —
the wallet is Privy's, and knowing its address without being able to use it is
not a working system. A mirror would buy the ability to display an address during
an outage in which nothing can be done with it. Availability is not improved by
caching the one field that does not gate anything.

**It also costs the ability to answer questions offline** — "how many customers
are there", "who signed up last week" — from the store alone. Those are real
product questions and they will be asked. The answer is that they are Privy's to
answer, it has an API for them, and duplicating its user table into an
append-only research store to avoid an API call is how a research store becomes
a customer database that nobody decided to build.

**The strongest counter-argument, stated fairly:** a system that owns none of its
customer state is a system that cannot be migrated off its provider without the
provider's cooperation. That is true, and 0005 already accepted a version of it
by choosing a hosted wallet. It is bounded by the same thing that bounds 0005:
the on-chain layer 0005 describes as the eventual destination moves custody to a
program Radar controls, and at that point the state moves with it. Recording a
mirror today does not make that migration easier, because the migration replaces
the schema rather than the contents.

## Consequences

- The customer backend has **no account table**, and `radar-store`'s schema does
  not change for customers. LEARNINGS 17's migration burden is not incurred.
- The signature meter is the only new durable artefact, and it is small enough to
  be verified completely.
- Every path that needs a wallet address needs a Privy client and an error path
  for Privy being unavailable. **Deny by default**: a wallet that cannot be
  looked up is not a wallet that can be traded on.
- The pricing measurement 0005 defers to becomes available from the first
  customer, which was the point of making it a precondition.

## What would reverse this

A measured latency cost on a customer-facing path that a cache cannot fix, or a
Privy rate limit that makes per-request lookup impractical. Both are measurable,
and neither is currently measured — so the honest position is that this is
decided on the structural argument and is open to being overturned by a number.

---

## Amendment, 2026-09-01 — the per-customer model spend is a second such thing

The decision stands unchanged. What changes is the count: **two** things pass
this ADR's test, not one.

### The reading that was wrong

A per-customer question meter for `/v1/chat` was first built **in memory**, on
the grounds that this ADR permits Radar exactly one durable customer artefact and
the signature meter was already it.

That read the arithmetic rather than the rule. The rule is in the title —
*records only the customer state it **cannot recover*** — and the table above has
one decisive column, `recoverable?`. Five rows answer yes and are not kept; one
answers no and is. **"Exactly one" was a count of what passed the test in August,
not a ceiling on what may.**

### Why the chat meter passes the same test

| state | where it already lives | recoverable? |
|---|---|---|
| **model spend on a customer's behalf** | nowhere durable | **no** |

Privy does not know it. Stripe will not know it — it bills for access, not for
consumption. The chain has never heard of it. Radar spent the money, and Radar is
the only party that could ever say how much of it went to whom.

That is the same row as the signature meter, for the same reason.

### What the in-memory version actually cost

Two things, and the second is the one that would have been discovered late.

**A restart handed the allowance back.** Deploys are routine and the unit is
configured `Restart=always`, so in practice a customer's daily ceiling was reset
several times a day — and under a crash loop, per crash. That is precisely the
failure [`LEARNINGS`](../../LEARNINGS.md) entries 1 and 9 record, and precisely
what `RADAR_STATE_DIR` was made mandatory to stop for the global budget. Building
a second meter beside it that forgot was repeating a fixed mistake in a new
costume.

**It was about to become a billing fact.** Once a subscription decides who may
ask, what a customer consumed is something Radar has to be able to stand behind
in a dispute. A figure that resets on deploy is not one, and this would have been
noticed by a customer rather than by a test.

### What this does not open

It is **not** a general licence to record customer state, and the test is
unchanged: *can this be read back from a more authoritative owner?* If yes, Radar
does not keep it. Identity, wallet, grant, deposits and positions all still
answer yes, and none of them may be mirrored on the strength of this amendment.

Two consequences follow, and both are already implemented:

- The record is keyed by the **salted hash** of the DID, exactly as the signature
  meter is, for the reason given above: the threat is every future copy of the
  store, not the live path.
- The daily rollover rule is the same. A record from an earlier day is not
  carried forward, because restoring one would refuse every customer until
  midnight — the same defect pointing the other way.

One deliberate asymmetry with rule 8, stated because it is a departure: an
**unreadable** record starts empty rather than refusing. A corrupt file must not
lock a paying customer out of a product while the global budget still bounds what
can be spent. Losing a day's counts costs fairness for a day; refusing on a
missing file costs the product.
