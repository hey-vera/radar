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

## What this does not decide

Whether Radar has an edge worth trading. That is measured separately and is
currently negative
([`0017`](../research/0017-a-control-that-could-have-been-traded.md),
[`0018`](../research/0018-the-deep-tail-points-the-wrong-way.md)). This decision
is about custody architecture, which is a long-lead item where retrofitting is
the expensive path — so it is settled now, deliberately, ahead of the thing it
serves.
