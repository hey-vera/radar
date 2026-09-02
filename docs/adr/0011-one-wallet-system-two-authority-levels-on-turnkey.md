<!-- SPDX-License-Identifier: Apache-2.0 -->
# 0011 — One wallet system, two authority levels, on Turnkey

**Date:** 2026-09-02
**Status:** decided, and **conditional on a spike that has not run yet**. See
"What must be proved before any integration code" — if the spike fails, this
decision reverts to [ADR 0005](0005-customers-keep-custody-and-grant-radar-a-bounded-signer.md)'s
Privy and this file says so.

Supersedes the vendor choice in ADR 0005 and the key placement in
[ADR 0007](0007-the-privy-authorization-key-lives-in-the-signer-process.md).
It does **not** supersede their reasoning, which transfers intact — the custody
model, the bounded signer, and the rule that a connected wallet is
authentication rather than authority are all unchanged. Only the vendor moves.

## What is decided

**One wallet system serving both product modes, with the mode expressed as an
authority level rather than as a different wallet.**

| | Wallet | Deposit / withdraw / export | Trading authority |
|---|---|---|---|
| **Signals mode** | embedded, or the customer's own | identical | none — the customer signs each trade |
| **AI mode** | embedded, or the customer's own | identical | a **scoped, revocable** delegation |

A customer never changes wallets to change modes. The delegation is granted or
revoked against the wallet they already have.

**And the provider is Turnkey rather than Privy.**

## Why the modes share one wallet

An earlier version of this plan had the two modes on different wallet models —
bring-your-own for signals, embedded for AI. That is wrong, and the reason is not
technical.

Moving a customer between wallet models later means telling them their wallet is
being replaced. It is the single most trust-destroying message a product holding
funds can send, and it is entirely avoidable by not building the split in the
first place. Whatever else changes, the address a customer funded stays the
address a customer funded.

## Why the customer must sign every trade, and what follows

Verified against our own mainnet captures, not assumed: **pump.fun requires the
`user` account to be a signer on `buy`, `buy_exact_sol_in` and `sell`** — index 6
on all three, in
[`pumpfun_accounts.json`](../../crates/radar-decode/tests/fixtures/pumpfun_accounts.json).

That single fact settles the architecture:

- A **connection session** is not signing authority. Connecting a browser wallet
  keeps a site authorised to *request* signatures; the wallet still prompts each
  time. Solana's own guidance is that full auto-signing is rare and usually a bad
  idea.
- **Session keys** need the *program* to accept an ephemeral signer. pump.fun
  does not.
- **SPL token delegation** cannot help: it delegates token transfers, and a buy
  spends SOL through an instruction that names the user as signer regardless.

So unattended trading requires a key that can sign while the customer is asleep.
There is no clever middle path, and every comparable product has reached the same
conclusion — Photon, BullX and Axiom all generate an in-app wallet funded by
deposit, and all offer key export. The market converged because the venue left no
alternative.

## Why Turnkey over Privy

**The deciding reason: Turnkey evaluates policy inside the enclave, before a
signature exists. Privy's equivalent controls are off-chain.**

Radar's whole architecture is one invariant — nothing signs without passing a
deterministic check it cannot bypass. Turnkey's Solana Policy Engine expresses
that directly: allow and deny over the programs invoked, the accounts included,
amounts, and instruction call data, with **DENY overriding ALLOW**. That is a
second, independent enforcement of the rule `radar-signer` already enforces, held
by a party that is neither Radar nor the customer.

Defence in depth on the one thing that can lose a customer's money is worth more
here than anything else on the comparison.

Four supporting reasons:

1. **It makes ADR 0005's first precondition satisfiable.** That precondition is a
   policy-engine spike *verified by making it refuse*, and it is what has blocked
   delegation. Turnkey's DENY policies make it a real test with a real negative
   case.
2. **Turnkey states our custody model as its architecture**: the end user owns
   the wallet and sets the rules, the agent operates within them, the agent never
   touches the private key, and the user revokes.
3. **One SDK covers both modes.** Email, Google, Apple, X, Discord, passkeys, and
   external Solana wallet connect. The dual model is one integration.
4. **Solana is first-class.** Privy's depth is EVM and Solana is secondary — a
   real mismatch for a Solana-only product, and the risk that ruled it out.

**Cost is not a constraint.** A free tier for development, then roughly
$99/month for unlimited signatures across up to 2,000 wallets.

### What Privy was better at, stated because it is true

Privy was acquired by Stripe in June 2025, which substantially retires the
"dependency on a company" cost ADR 0005 recorded — and Radar already has a Stripe
relationship for billing ([ADR 0010](0010-entitlement-is-read-from-stripe-never-recorded.md)),
so staying would have meant one commercial counterparty rather than two.

Privy also correctly rebuts one thing ADR 0005 implied and this file corrects:
**a customer keeps access if Radar disappears.** Privy's documentation is
explicit that keys can be exported using the user's access token against Privy's
API even when the application does not implement an export flow. That is a real
guarantee and it was better than ADR 0005 gave it credit for. **Turnkey must be
held to the same standard** — see the preconditions.

Turnkey is also, in its own framing, "a building block, not a product." There is
more integration work than Privy would have needed. The Embedded Wallet Kit
narrows that gap; it does not close it.

## Why now, and why the cost is acceptable

Switching costs about **1,333 lines across 17 files** —
[`radar-signer/src/privy.rs`](../../crates/radar-signer/src/privy.rs),
[`radar-serve/src/privy.rs`](../../crates/radar-serve/src/privy.rs), and the
larger part of [`customer.rs`](../../crates/radar-serve/src/customer.rs).

It is acceptable for one reason: **the Privy application has zero registered
users.** Switching today costs code. Switching later costs customers, and costs
them in exactly the currency this ADR opens by refusing to spend — a message
telling people their wallet is being moved. This is the only cheap moment there
will ever be.

The design transfers rather than being redesigned. ADR 0007 put the Privy
authorization key inside the signer process and kept it out of `radar-serve` —
the process with a listener, a model provider, an HTTP client and an embedded
frontend. A Turnkey API key is the same category of object: a credential that
causes customer funds to move. It goes in the same place, for the same reason,
and ADR 0007's argument is reused verbatim with the noun changed.

## What must be proved before any integration code

**Precondition 1 — the policy engine refuses.** Turnkey's Solana
`SIGN_TRANSACTION` policy parsing is recent, and new code in a security boundary
deserves suspicion rather than trust. The spike must show a policy **denying** a
transaction it should deny — a different mint, an amount above the cap, a program
outside the allowlist — and it is not passed by showing an allow working. ADR
0005's precondition, unchanged except that it is now answerable.

**Precondition 2 — the customer's exit survives Radar.** Turnkey must be shown to
let a customer export their key or move their funds **without Radar's
cooperation, uptime, or good behaviour**, in the way Privy documents. If it
cannot, that is a regression against the vendor being replaced and this decision
does not hold.

**Precondition 3 — the delegation cannot move funds to an arbitrary address.**
The policy must be provably scoped to trading. A delegation that can transfer to
any address is custody, whatever it is called, and a compromised `radar-serve`
would drain every wallet at once.

**Withdrawals are signed by the customer, never by Radar's delegation.** Deposit
is an address. Withdrawal is the customer moving their own money with their own
authenticated session, to any address they choose. Radar's server-side credential
is never what authorises an outbound transfer — that is what precondition 3
protects, and it is why withdrawal can ship before delegation does.

## What this does not decide

**Whether autonomous trading opens.** `Policy::CLOSED` still ships, the measured
edge is still 0 bps
([`0017`](../research/0017-a-control-that-could-have-been-traded.md)), and
[`0022`](../research/0022-capacity-was-a-budget-not-a-ceiling.md) puts the bar at
roughly 456 bps of expected edge before a trade is worth making. This decision
makes the wallet layer ready. It does not make the case for using it.

**That Turnkey is right at scale.** It is right at this scale, for this
architecture, with zero customers to migrate. Revisit if custody grows to where
an institutional posture matters, or if the Solana policy engine proves
unreliable.

## Reversal

If the spike fails precondition 1, 2 or 3, stay on Privy. The Privy code is not
deleted until the spike passes, so reverting is abandoning a branch rather than
rebuilding a lane.
