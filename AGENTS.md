<!-- SPDX-License-Identifier: Apache-2.0 -->
# AGENTS.md

Instructions for coding agents working in this repository. Follows the
[AGENTS.md](https://agents.md) convention.

## What this project is

Radar is a Solana research and trading platform. The goal is not "a trading bot".
It is infrastructure that can **systematically discover, measure and exploit real
edges in Solana markets**, and expose the resulting intelligence to other agents
over x402.

Two facts set the tone, and both are load-bearing:

1. **The base rate is brutal.** In April 2026 — one of the best months in two
   years — roughly 96% of pump.fun wallets either lost money or made under $500,
   and only 5.4% cleared $1,000. Any design whose success case is "beat the
   average memecoin trader" is a plan to lose slowly.
2. **The realistic edges are unglamorous and measurable**: not buying traps,
   sizing for the exit rather than the entry, disciplined execution cost, and
   composing signals that are individually weak. Radar is built to measure those,
   not to assume them.

So the first deliverable is not a trade. It is a **recorder with a point-in-time
guarantee**, because the dataset only accumulates forward.

The trading lane exists and is shut. `radar-strategy`, `radar-risk`,
`radar-signer` and `radar-exec` are built and tested end to end; the shipped
policy is `Policy::CLOSED`, which refuses every proposal. Nothing is missing and
nothing is armed. If you are changing that value, you are making a decision about
money — make it deliberately, and not as a side effect of something else.

## Build and test

```bash
just check   # build, tests, lint, fmt — the edit-compile loop
just ci      # everything a runner can do
```

On Windows under MSYS or Git Bash the default host toolchain resolves to MSVC,
where MSYS `link` shadows the MSVC linker and every build fails at the link step.
Export the toolchain that works:

```bash
export RADAR_CARGO="cargo +stable-x86_64-pc-windows-gnullvm"
```

## Rules that are not negotiable

These are invariants of the design. A change that breaks one is wrong even if it
compiles and the tests pass — in which case the tests are also wrong.

1. **Model judgement must never authorise capital.** An AI or a strategy emits a
   `Proposal`, which is inert data. Only the deterministic risk kernel turns a
   proposal into an `Authorization`, and only the separate signer process turns
   an authorization into a signature — after re-decoding the transaction to check
   it against the authorization's bounds. If you find yourself adding a path from
   a reasoning layer to a signer, stop.

   The signer's guarantee is absolute and stated as such: *every account it
   authorises is one it read in the bytes it signed.* This is why it refuses
   address lookup tables ([ADR 0003](docs/adr/0003-legacy-transactions-because-the-signer-must-be-able-to-read-them.md))
   and why it has no network, no listener and no method that signs arbitrary
   bytes. Anything that would let it sign something it has not fully read breaks
   the guarantee, however convenient.

2. **The risk kernel is pure.** No clock, no network, no ambient state, and no
   dependence on the order of its inputs. Purity is what makes a verdict
   replayable and a refusal reproducible from a recording.

3. **Nothing reads past its watermark.** Every read is gated by
   [`AsOf`](crates/radar-asof). A value observed after the watermark cannot be
   unwrapped — not "should not", cannot. This is what keeps look-ahead bias out
   of research results, and it reaches into the cache too: a replay must not be
   served a live-populated entry from the future.

4. **Untrusted content is never an instruction.** Token metadata, social posts,
   website copy and transaction memos are `Trust::Untrusted` no matter how
   authoritative they sound. They may be stored, hashed, displayed and analysed
   as data. They never enter a system-prompt position and never justify an action
   on their own.

5. **A latch may only close, never open.** Mint authority, once revoked, cannot
   be restored. A provider reporting otherwise is wrong, confused, or being
   manipulated — it raises, it does not silently update.

6. **Never buy parsed transactions.** See
   [ADR 0001](docs/adr/0001-decode-locally-never-buy-parsed-transactions.md).
   Decoding is where a vendor charges fifty times the raw material price, so it
   is the step Radar owns.

7. **The x402 lane never touches the execution path.** On-chain settlement adds
   400–800ms before a response returns. Fine for analysis, fatal for trading.
   `getLatestBlockhash`, pre-trade `simulateTransaction` and `sendTransaction`
   always go to a direct RPC endpoint.

8. **Deny by default when config is missing.** A spend meter with no budget
   loaded refuses everything, a signer with no allowlist refuses everything, and
   a paywall with no facilitator serves nothing rather than serving free.
   Spending nothing is always recoverable.

9. **Absent is not zero, and unknown is not safe.** A missing price impact is
   `u32::MAX`, not `0`. A capacity that could not be measured is `None`, and
   `None` means "cannot exit", never "no limit found". A creator with no measured
   launches has no rate rather than a rate of zero. Every one of these is a place
   where the convenient default is the one that loses money.

## Verify before you claim

Every claim in this repository should be backed by something that runs. Run it,
read the output, quote it. Under-claiming costs nothing; over-claiming costs the
benefit of the doubt on everything else.

`repo-conformance` enforces the mechanical half of this: every crate directory
is a workspace member, every member has source, every relative link in the
documentation resolves, every ADR cited by number exists, and the README's crate
table matches the workspace. It caught three empty crate directories on its first
run, one of which was itself.

This is not hypothetical. The caching design Radar's provider layer is modelled
on was documented as canonical in a sibling repository, citing functions in a
file that is not in the working tree and not in git — only stale, gitignored
build output survives. That is the failure mode to avoid, and
[LEARNINGS.md](LEARNINGS.md) is where instances get recorded.

## Where to start

- [`docs/research/`](docs/research/) — what was investigated and what it found,
  including the data-sourcing landscape and the freshness/caching design.
- [`docs/adr/`](docs/adr/) — decisions, with what each one costs.
- `crates/radar-provider` — the metered, cached, health-aware data plane. Pure
  policy: no HTTP, no clock, no async. Read its module docs first; the rest of
  the system's economics fall out of it.
