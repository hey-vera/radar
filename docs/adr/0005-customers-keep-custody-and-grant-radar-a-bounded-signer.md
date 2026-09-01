<!-- SPDX-License-Identifier: Apache-2.0 -->
# ADR 0005 — Customers keep custody, and grant Radar a bounded signer

**Date:** 2026-08-31
**Status:** accepted
**Decides:** how a customer's capital is held and how Radar is permitted to move
it, for the public product.

## Context

Radar is built to trade **on a person's behalf**. That is the product, and it
sets the constraint: the system must be able to sign a transaction while the user
is not present. A design where the user approves each trade in a browser wallet
is not this product — it is a signal feed with extra steps.

Three things bound the choice.

**Custody is a legal fork, not a technical one.** Holding customers' funds makes
Radar a money transmitter in most jurisdictions. That is a licensing and
compliance burden out of all proportion to the product, and it is not undone by
being careful.

**Autonomy and non-custody are in genuine tension on Solana.** There is no native
account abstraction. The available shapes are: the user signs every transaction
(kills the product); SPL `approve`, which natively delegates *transfer authority*
on a token account up to an amount — real, and it covers **selling** but not
buying with SOL; an on-chain program with a delegated PDA, which is the most
control and the largest scope; or an embedded wallet whose provider signs under
policy.

**Invariant 1 must survive.** *"Model judgement must never authorise capital.
Only the deterministic risk kernel turns a proposal into an `Authorization`, and
only the separate signer process turns an authorization into a signature — after
re-decoding the transaction to check it against the authorization's bounds."* A
customer's wallet is a **new signer**. The rule applies to whose capital
regardless.

## Decision

**The customer's funds live in an embedded wallet the customer controls. The
customer explicitly grants Radar's server a signer key, bounded by a policy
derived from the same `Policy` the risk kernel judged against. Radar never holds
the customer's key, and the customer can revoke.**

Provider: **Privy**, and the reason is architectural fit rather than features.

### Why the fit is exact

Radar's [`Authorization`](../../crates/radar-risk/src/kernel.rs) already carries
precisely the fields a delegated-signing policy needs:

| Radar's `Authorization` | what the policy must enforce |
|---|---|
| `max_notional` | the most that may be committed |
| `expires_after` | the slot after which it is void |
| `mint`, `action` | what may be traded, and which way |

Privy's model is: the server generates a key pair and registers the public key;
the **user** adds that key as a signer on their own wallet; the server signs
through an authorization header; and an optional **policy** constrains what that
signer may do — with time-based expiry, transaction amount limits, and
program/contract restrictions. Its policy engine understands Solana specifically,
evaluates amounts in lamports without conversion, and interprets SPL transfers.

So **Radar already produces the object the policy engine wants**. This is not an
integration that has to be bent into shape.

### Why that preserves invariant 1 rather than weakening it

The local signer's guarantee is that a separate process re-decodes the
transaction and checks it against the kernel's bounds. In the customer lane, the
policy engine plays that part — and it is **not Radar's code**. Radar's kernel
authorises; a system Radar does not control independently re-checks before
signing.

The rule that makes this hold, and it must be written down before the feature
exists: **the Privy policy is derived from the kernel's `Policy`, never from
anything a model, a strategy, or a customer's assertion says.** A connected
wallet is authentication, not authority. It may never soften a refusal — the same
boundary the per-token trust feature needs, for the same reason.

## What this costs

**A trust assumption Radar does not get to make on the customer's behalf.** Privy
combines key splitting with trusted execution environments, which means the
provider can in principle reconstitute a key. That is a real assumption and it
must be **disclosed to the user in those terms**, not buried. "Non-custodial"
here means Radar is not the custodian; it does not mean nobody is.

**A published threat model we do not have.** Privy's documentation describes the
mechanism but does not publish a formal threat model, cryptographic
specification, or audit claims — checked, and stated here rather than assumed
absent or assumed present. Radar's own signer has an absolute stated guarantee:
*every account it authorises is one it read in the bytes it signed*
([ADR 0003](0003-legacy-transactions-because-the-signer-must-be-able-to-read-them.md)).
The customer lane does not inherit that.

**So the two lanes stay separate, and that is the point.** Radar's **own** capital
keeps the local signer, whose guarantee is stronger and whose key is on a machine
Radar owns. Customers' capital uses Privy, because the alternative for customers
is Radar custodying — which is worse on every axis that matters. Different
capital, different threat models, different mechanisms. Neither replaces the
other.

**A dependency on a company.** If Privy disappears or changes terms, the
customer lane needs rebuilding. Mitigated only by the boundary: Radar's side is
"produce an `Authorization` and ask a signer", which is the same shape the local
signer already answers.

## Alternatives rejected

**Reown AppKit, or any connect-only wallet.** The user's Phantom or Solflare
would have to approve every trade. That is the correct choice for a research
product and the wrong one for this product, and it is not a close call — it
rules itself out on the requirement rather than on a comparison.

**Dynamic.** A comparable embedded-wallet product with real Solana support. It
was **not evaluated to the same depth**, so this ADR does not claim Privy is
better — only that Privy's policy primitives were confirmed to match Radar's
`Authorization` shape. If Dynamic's do too, the choice is closer than this
document makes it look, and the spike below is where that would surface.

**Turnkey, or key infrastructure directly.** More control and a smaller trust
surface, at the cost of building the policy and onboarding layers Privy
supplies. Worth revisiting if the disclosed trust assumption proves unacceptable.

**An on-chain program with a delegated PDA.** The most control and the least
counterparty risk. Also a Solana program Radar does not have, with an audit
requirement it cannot currently meet. Not now; the right answer eventually if the
product works.

**SPL `approve` alone.** Natively delegates transfer authority up to an amount,
which covers the **exit** and not the entry. Worth noting rather than dismissing:
Radar's entire thesis is exit-first, and the one thing Solana delegates natively
is the sell. It does not make a product on its own, and it is a real fallback for
the half that matters most.

**Radar holding customer keys.** Off the table on the legal fork above.

## What has to be true before this ships

1. **A spike confirming the policy engine enforces what this ADR assumes**: a
   lamport-denominated amount ceiling and a time bound, on a Solana wallet,
   refused in the direction that fails. Verified by making it refuse, not by
   reading that it can — the standard `LEARNINGS` 16 sets.
2. **The route model.** `is_public()` is a one-line stand-in
   (`/health`, `/x402/`). Cloudflare Access currently gates the whole site to one
   operator, and a public product cannot sit behind it. Access protects the ops
   surface; customer auth protects the customer surface; they are different route
   sets and the seam does not exist yet.
3. **The disclosure**, in the product, in the terms above.
4. **The kernel still in the path.** No customer transaction may be built from
   anything but an `Authorization` the kernel issued.

## Where this is going, and what keeps the move cheap

Written down now because the beta will otherwise shape the code toward one
vendor's API and nobody will have recorded where it was supposed to end up.

### The two are layers, not alternatives

An earlier reading of this treated the on-chain program as a *replacement* for
the hosted wallet. It is not, and the layering is the point:

| | who signs | how often | who pays |
|---|---|---|---|
| **identity and user actions** — sign up, deposit, delegate, withdraw | the customer, from their embedded wallet | about three times, ever | the provider's meter |
| **trading** — every buy and sell | **Radar, with one key it already owns** | thousands | nobody |

So the endgame is not "replace the provider". It is **keep the provider for
identity and move the trading authority on-chain**, which takes the signature
bill from thousands per customer to roughly three. The embedded wallet is
unavoidable regardless if customers are to be onboarded without already owning
one, which the deposit model above assumes.

### Why not now

**An audit, before a customer exists.** A program holding delegated authority
over customer funds is exactly the thing that gets drained, so it cannot ship
unaudited — and that is a large sum spent on a product whose edge is currently
measured as *negative*
([`0017`](../research/0017-a-control-that-could-have-been-traded.md)).

**Upgrade authority is itself a custody question, and it has no clean answer.**
Hold it, and Radar can change what the program does — which is arguably custody,
having paid for an audit to arrive back at the fork this ADR exists to avoid.
Burn it, and a bug in code holding customer money can never be fixed. This is a
hard design problem rather than a detail, and it is the single thing to solve
first when the time comes.

**The program cannot be designed yet.** It has to encode which instructions and
venues Radar trades through, and
[`0018`](../research/0018-the-deep-tail-points-the-wrong-way.md) has moved the
burden of proof onto *staying* on pump.fun pre-graduation. On-chain bounds
written around a venue Radar may leave is building the wrong thing carefully.

### The constraint that keeps the move cheap

**Radar's side of the boundary stays "produce an `Authorization` and ask a
signer".** All three answers — the local signer, a hosted policy engine, an
on-chain program — consume exactly that object. The boundary is already this
thin, and the only way to lose the option is to let a vendor's API shape appear
anywhere above it.

Concretely, and this is the thing to refuse in review: **no provider type may
appear in `radar-risk`, `radar-strategy` or `radar-exec`.** The provider is an
implementation of `Signing` at the edge, which is where it already sits — the
lane composition test proves the executor talks to a trait rather than a client.

## What this does not decide

Whether Radar has an edge worth trading. That is measured separately and is
currently negative
([`0017`](../research/0017-a-control-that-could-have-been-traded.md),
[`0018`](../research/0018-the-deep-tail-points-the-wrong-way.md)). This decision
is about custody architecture, which is a long-lead item where retrofitting is
the expensive path — so it is settled now, deliberately, ahead of the thing it
serves.

---

## Amendment, 2026-08-31 — the deposit wallet, and the cost axis this missed

A second research round changed two things. The decision stands; the model is
better and the cost basis is new.

### The wallet is created at signup and the customer deposits into it

The original text described a customer's existing wallet with Radar added as a
delegated signer. **A wallet provisioned at account creation, which the customer
deposits into and withdraws from, is better** — and not only for onboarding.

**The blast radius becomes the deposit rather than the customer's whole wallet.**
That is a risk boundary, not a convenience: a bug, a compromised policy or a
runaway strategy can lose what was deposited and nothing else. The customer sets
their own exposure by choosing how much to move in, which is a limit no
`max_position` can express because it lives outside the system entirely.

It is also the category's convention, and it is Privy's flagship pattern —
embedded self-custody wallets generated for each user at signup.

**The custody line is unchanged and is what makes this legitimate:** the wallet is
provisioned *to the customer*. They can always revoke the signer and always
withdraw. If they cannot do both, it is custody wherever the key shards live, and
the legal fork in the Context above applies.

### The cost axis: Radar is signature-heavy and user-light

This ADR chose a provider on architectural fit and **did not price the
workload**. It should have.

Privy includes 50,000 signatures a month on every developer tier and charges
**$0.01 per signature** beyond it, above a $2,000 base. That is priced for
applications where a user signs occasionally. A trading system is the opposite: a
round trip is two signatures, so at *N* round trips per customer per day the bill
is 60·*N* signatures per customer per month.

| trades/customer/day | customers before overage | signature cost at 1,000 customers |
|---|---|---|
| 1 | ~833 | ~$100/mo |
| 10 | ~83 | ~$5,500/mo |
| 50 | ~16 | ~$29,500/mo |

**Radar's variable cost scales with how often it trades, not with how many
customers it has.** Nothing else in the system has that shape, and it is the
figure that should decide the provider.

Worth noting where it points: a design that trades *less* is cheaper on this axis
as well as on the 850 bps round trip
([`0019`](../research/0019-the-round-trip-is-not-one-number.md)). Radar's
exit-first thesis already argues for holding rather than churning. This is a
second, independent reason for the same discipline.

### What this does to the decision

**Still Privy, and start now** — the free tier below 500 monthly active users
covers an entire beta at no cost, so the cheapest way to settle the question is
to run on it and **measure signatures per customer per month**, then choose on
that number rather than on the estimate above.

Instrumenting that count is a precondition, added to the list below. Committing to
a provider on an unmeasured cost when the measurement is free is the mistake
[`LEARNINGS`](../../LEARNINGS.md) entry 1 keeps recording in a different costume.

### The alternatives moved

- **Dynamic was acquired by Fireblocks.** The original text named it as the
  comparable option; it has changed hands, as did Privy (Stripe, June 2025) and
  BVNK (Mastercard). **Consolidation here is the norm rather than an event**, and
  that argues for keeping Radar's side of the boundary thin — which it is: produce
  an `Authorization` and ask a signer.
- **Turnkey is the serious alternative on this cost axis**, and more so than the
  original text allowed. Pure key infrastructure in TEEs, a policy engine at the
  signing layer, a Solana Developer Platform partner, and per-signature pricing
  that reportedly scales down at volume. It supplies less product — onboarding and
  session handling are yours to build — which is the trade.
- **Stripe owning Privy is a mild point in its favour**, since billing was going
  to be Stripe regardless and that is one vendor relationship rather than two.

### Honest framing of "non-custodial"

Every hosted option here — Coinbase, Crossmint, Privy, Turnkey — is key material
held in the provider's cloud inside trusted execution environments. **"Non-custodial"
means Radar is not the custodian. It does not mean nobody is**, and the
disclosure required above should say so in those words.

### Added preconditions

5. **Signature counting, from the first customer.** The provider decision is
   deferred to a measurement that costs nothing to take, and it cannot be taken
   retroactively.
6. **Withdrawal must work before deposit is offered.** A deposit path without a
   proven withdrawal path is custody with extra steps, whatever the architecture
   diagram says.

   **Resolved 2026-08-31, structurally, and better than this precondition asked
   for.** It assumed Radar would have to *build* a withdrawal path. It does not,
   and it should not, because Privy gives the customer two exits that do not
   involve Radar at all:

   - **Export.** The private key is assembled on an origin separate from the
     application's, so neither Radar nor Privy can ever see it. The customer is
     the only party who can. They can take the wallet to Phantom and leave.
   - **Revocation.** `removeSessionSigners` removes Radar's signer, after which
     Radar can take no further action on the wallet.

   Neither needs Radar's cooperation, or its uptime, or its good behaviour. That
   is a materially stronger answer than a withdrawal endpoint, which would have
   been a path Radar could break, throttle, or be compromised into refusing.

   So the precondition is met by *not building the thing it was worried about* —
   and the product's obligation becomes making both exits **visible**, not
   implementing them. A wallet the customer cannot find the exit from is
   custodial in effect regardless of who holds the key.

8. **The wallet must be user-owned, not app-owned.** Added 2026-08-31, and it is
   the piece that makes the rest of the story hold.

   Privy distinguishes an **owner** from a **signer**. The owner may update
   policies, change owners, add signers and export the key. A signer may do none
   of those — it may only sign, within the policy the owner set.

   [ADR 0008](0008-the-signer-holds-its-own-policy.md) names Privy's policy
   engine as the independent backstop against a compromised `radar-serve`, on the
   grounds that it holds bounds Radar does not supply per request. **That is only
   true if Radar is not the owner.** An app-owned wallet lets whoever holds the
   application credential rewrite the policy that was supposed to bound them,
   which turns the backstop into a formality and takes the rest of the design
   with it.

   So: the customer is the owner. Radar is a signer and nothing more. This is a
   configuration decision at wallet creation, it is invisible in normal
   operation, and it is the difference between a bounded signer and a custodian
   with extra steps.
7. **An ES256 verifier, which does not exist.** Found by checking rather than
   assuming, and it would otherwise have been discovered after the integration
   was wired up.

   Privy's access tokens are JWTs signed with **ES256** (ECDSA P-256).
   [`access::verify`](../../crates/radar-serve/src/access.rs) refuses anything
   that is not **RS256**, and that refusal is correct and deliberate — the
   comment above it says why: *"`alg: none` has no signature to check and `HS256`
   invites checking one with the wrong primitive; both are settled by refusing to
   proceed at all."*

   So the customer authenticator needs a second verification path, and the shape
   of the seam is set by the same reasoning that makes the current refusal right:
   **each authenticator pins exactly one algorithm.** Cloudflare Access pins
   RS256; a Privy authenticator pins ES256. Neither may accept a *set*, because
   accepting a set is the algorithm-confusion attack the existing code already
   refuses to be exposed to.

   It is deliberately not built ahead of a real token to test it against. An
   untested signature verifier that looks correct is worse than an absent one —
   it is the only kind of bug in this system that fails open.
