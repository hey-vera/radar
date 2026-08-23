# 0003 — Legacy transactions, because the signer must be able to read them

**Status:** accepted
**Date:** 2026-08-22

## Context

The signer's guarantee is stated as an absolute: *every account this process
authorises is one it read in the bytes it signed.* That is what makes the
separate process worth having. A compromised executor can build any transaction
and describe it any way it likes, and the signer's answer does not depend on any
of its claims.

Solana v0 transactions break the guarantee. A versioned message may name
accounts indirectly through an on-chain **address lookup table**: the message
carries a table address and an index, and the runtime resolves the real account
at execution time. A verifier reading only the transaction bytes cannot see which
accounts an instruction touches.

Jupiter's `/swap` endpoint returns v0 transactions with lookup tables by default,
because that is how a router fits a long multi-hop route inside the transaction
size limit.

So the two things Radar wants are in direct conflict: the best routes, and a
signer that can check them.

## Options

**1. Resolve the lookup tables in the signer.** The signer fetches each table and
expands the indices before checking. This restores visibility — at the cost of
giving the signer a network dependency and an RPC endpoint, which is one more
input that can lie to it, and one more thing that can be unavailable at the
moment a position needs closing. It also means a table's *contents* at fetch time
must match its contents at execution time, which is not guaranteed: lookup tables
can be extended.

**2. Trust the router.** Sign v0 transactions without expanding them, on the
grounds that Jupiter is reputable. This makes the signer's guarantee conditional
on a third party, at which point the separate process is decorative.

**3. Ask the router for legacy transactions.** Jupiter supports
`asLegacyTransaction=true` on both `/quote` and `/swap`. Every account is named
inline. The signer reads everything.

## Decision

**Option 3.** `radar-exec` passes `asLegacyTransaction=true`, and
`radar-signer` refuses any message carrying lookup tables — refuses, not
"resolves", so the two halves cannot drift into disagreeing.

`radar_exec::route::verify_shape` also runs the signer's decoder on the router's
answer before a decision rests on it. That is an early exit, never a substitute:
the signer still checks. Discovering an unsignable route at the signer would mean
a decision had already been built on a route that could never execute.

## What this costs

Routes needing more accounts than a legacy message can hold become unavailable.
In practice that means long multi-hop routes across several venues.

This is a real restriction and an acceptable one, for two reasons:

- The tokens Radar trades are early ones. Their liquidity is on the pump.fun
  bonding curve or on one AMM, and those routes are short. The long routes a v0
  transaction exists to carry are for established tokens with fragmented
  liquidity — not this population.
- A route the signer cannot read is a route nothing can check. Losing access to
  it is the cost of the guarantee, and the guarantee is the thing that makes
  running this system with real money defensible.

For pre-graduation pump.fun tokens the question does not arise: Radar goes
direct to the bonding-curve program, where the account set is fixed, known, and
built locally with no router in the path at all.

## Consequences

- `radar-exec` and `radar-signer` share one decoder — the signer's — so a
  transaction accepted by the pre-check is accepted by the signer for the same
  reasons.
- If a future router only offers v0, Radar builds instructions itself rather than
  relaxing this. That is more work, not a different principle.
- Alpenglow does not change any of this. It changes finality timing, not
  addressing.
