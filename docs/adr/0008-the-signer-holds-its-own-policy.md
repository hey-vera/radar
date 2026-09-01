<!-- SPDX-License-Identifier: Apache-2.0 -->
# ADR 0008 — The signer holds its own policy, and clamps against it unconditionally

**Date:** 2026-08-31
**Status:** accepted
**Decides:** what the signer trusts about the authorisation it is handed, and
therefore what it protects against.

## Context

[LEARNINGS](../../LEARNINGS.md) 23 records the finding this ADR answers. The
signer does not verify that the `Authorization` it receives came from the kernel:
there is no MAC on it, and its `nonce` — a content hash of the proposal and the
state it was judged against — is never checked against anything.

So its guarantee is *the transaction matches the authorisation the caller
supplied.* Against an executor **bug**, that is complete, and it is what the
check was built for. Against a **compromised caller**, it is not: such a caller
writes its own authorisation, with its own mint, its own ceiling and its own
expiry, and the signer checks the transaction faithfully against those.

That gap became load-bearing when
[ADR 0007](0007-the-privy-authorization-key-lives-in-the-signer-process.md) moved
the Privy authorization key into this process specifically to survive a
compromise of `radar-serve`.

### The obvious fix is worse than it looks

ADR 0007's amendment suggested the signer could hold the `Policy` and **run the
kernel itself**, taking a proposal rather than a finished authorisation.
`radar-risk` is already one of its dependencies, so it is cheap.

On inspection it is the wrong shape, and it is worth writing down why, because it
is the intuitive answer.

`evaluate(proposal, state, policy)` is a function of three things, and the signer
would hold only one of them. The caller would still supply the **portfolio
state** — how much is already deployed, what today's realised loss is, what is
held per creator. Every portfolio-scoped limit in the policy is evaluated against
that state, so a caller that understates it gets those limits to pass. The
kernel's answer is exactly as trustworthy as the state it was given.

The result would *look* far stronger — the signer runs the risk kernel! — while
protecting against very little more than today. That is the failure mode this
repository keeps recording, and building it would be a new instance of it.

## Decision

**The signer loads a `Policy` of its own and clamps every authorisation against
it, unconditionally.** No caller-supplied value can widen anything.

Unconditional is the operative word. A clamp does not depend on state the caller
supplies, so unlike a kernel re-run it cannot be defeated by lying about
anything. It answers a narrower question than the kernel does, and it answers it
without trusting the questioner.

Four checks, each on a value the caller currently controls outright:

1. **Autonomy.** If the signer's policy cannot self-authorise
   (`Observe`, `Alert`, `Approve`), nothing is signed. This is what makes
   `Policy::CLOSED` *closed at the signer* rather than closed only in the process
   that decides. Today `Policy::CLOSED` is enforced in one place, and that place
   is upstream of the key.
2. **Notional.** An authorisation may not exceed `policy.max_position`. Refused
   rather than silently clamped: a caller asking for more than the operator's
   policy allows is either a bug or an attack, and quietly serving it a smaller
   number hides both.
3. **Canary.** Under `Autonomy::Canary`, `policy.max_canary` is the ceiling
   instead. The dust round trip is the one thing that level exists to permit, and
   it must not inherit the larger bound.
4. **Lifetime.** An authorisation's window may not exceed
   `policy.max_input_staleness` slots from now. Expiry is the only thing making a
   grant temporary, and a caller currently chooses it.

The policy is **loaded once, at start**, from a file — the same rule the
allowlist already follows, for the same reason stated there: *a signer that
re-reads its rules per request is a signer whose rules can be changed by whoever
can write that file while it runs.*

Rule 8 applies: **no policy file means no signing.** Not a default policy, and
not the caller's judgement.

## What this does not fix, stated plainly

**A compromised caller can still repeat.** Each authorisation is bounded by
`max_position`, and nothing here bounds how many of them arrive. The
portfolio-scoped limits — `max_deployed`, `max_daily_loss`, `max_per_creator` —
remain unenforceable in this process, for the same reason the kernel re-run was
rejected: they are functions of state the signer does not hold.

The fix for that is for the signer to hold its **own** accounting: it sees every
authorisation it grants, so it can accumulate them and enforce a daily ceiling on
its own numbers rather than on the caller's. That is a real design and it is
deliberately not in this ADR, because it needs durable state that survives a
restart, and adding both at once produces a change too large to review carefully
in the one process whose virtue is that it is small enough to read completely.

**It is a precondition for customer capital, and it is written down as one here**
so that it is a scheduled piece of work rather than a paragraph someone remembers
later.

Until it exists, the bound on a compromised caller is: `max_position` per
transaction, times however many transactions it can get signed, with
**Privy's policy engine as the independent backstop** — which
[ADR 0005](0005-customers-keep-custody-and-grant-radar-a-bounded-signer.md)
precondition 1 still requires be verified by making it refuse.

## Consequences

- `radar-signer` gains `RADAR_SIGNER_POLICY`, and refuses to sign without it.
  Absent is a refusal, not a permissive default.
- The deployment gains a third file for the signer. Its contents are an operator
  decision about money, which is the point of it being a file rather than a
  constant.
- `radar-exec` and `radar-serve` are unaffected: the protocol does not change.
  The clamp is invisible to a caller that was already inside the policy, and a
  refusal to one that was not — which is the correct signal in both directions.
- A test suite that asserts the clamp must include the case where the caller's
  authorisation is *wider* than the policy, since that is the only case the
  change is about.

## What would reverse this

Nothing foreseeable reverses holding a policy locally. The specific ceilings are
expected to move, and moving them is an operator action against a file rather
than a code change, which is the shape this ADR is choosing.
