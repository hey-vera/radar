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

**The recorder has now produced its first verdict on the selection, and it is
negative.** Over 4,374 decisions,
[`0014`](docs/research/0014-the-control-was-entirely-tokens-nobody-could-sell.md)
measured Radar's proposals at a gross median of +21 bps and called that noise
around zero. It is not noise: that figure compares a **sell quote** against a
**mid**, and [`0016`](docs/research/0016-the-entry-was-a-bid-and-the-exit-was-a-mid.md)
measures the gap between those two instruments at **at least +128 bps** — six
times the signal it was hiding. Corrected, the gross median is **at most −107
bps**, and **−957** after the measured 850 bps round trip.

The comparison against refusals that appeared to make it worse is unusable:
every scoreable refusal is `CapacityBelowFloor`, so the control is composed
entirely of tokens Radar measured and found it could not sell.

[`0017`](docs/research/0017-a-control-that-could-have-been-traded.md) builds the
control that comparison lacked, against 121,810 tokens Radar never decided on,
priced the same way on both sides and matched on token age and holding period.
It finds **no edge** — a median edge of 0 bps across four matched strata. Both
that note and [`0018`](docs/research/0018-the-deep-tail-points-the-wrong-way.md)
were re-measured after LEARNINGS 19 corrected the pairing gate, and both
conclusions survived it.

0018 is the one to read next. Radar sizes every position as a share of measured
exit capacity, and 80% of proposals sit in a ±13% band around $31 because every
pre-graduation pump.fun token rides the same curve — so the median position is
**$6.21**, needing a +8.5% move to clear the round trip. The one band where a
real position fits, $60+, is **down 68% on 25 rows**. The binding constraint is
capacity, not signal, and the deep tail does not look like an escape from it.

Read that before adding a filter. The gross median says the selection is not
finding an edge, and the 850 bps says the round trip is currently larger than
anything the filter has found.

The trading lane exists and is shut, and it is worth being exact about *where*
it is shut, because an earlier version of this paragraph was not.

`radar-strategy`, `radar-risk`, `radar-signer` and `radar-exec` are each built and
tested, and **as of 2026-08-31 the lane is composed end to end** by
[`crates/radar-exec/tests/lane_composes.rs`](crates/radar-exec/tests/lane_composes.rs):
one real candidate runs strategy → kernel → executor, and the executor's
`Routing`, `Signing` and `Sending` traits are stubbed so the ordering can be
exercised without a network or a key.

Be exact about what that does and does not establish, because the previous
version of this paragraph was exact and it is worth staying that way.

**What it establishes.** The stages agree about what a fundable proposal looks
like. The signer is handed the kernel's authorisation verbatim rather than a
reconstruction. A signer refusal ends the attempt without sending. A trade that
does not pay for itself never reaches the process holding the key. And
`Policy::CLOSED` refuses the same candidate a permissive policy authorises —
which is what makes the other tests statements about the lane rather than about
the fixture.

**What it does not.** No production crate depends on `radar-exec`; the
composition reaches it through a dev-dependency, so the shipped dependency graph
is unchanged. Nothing has been signed, sent, or filled.

**As of 2026-09-01 the pipeline's traits have real implementations**, which they
did not before: `Routing` and `Sending` were satisfied only by stubs inside
`pipeline.rs`'s own test module, while `route::Router` and `submit::Submitter` —
which talk to Jupiter and to an RPC node — sat beside them unconnected. So the
executor could be composed only against a fixture, which is LEARNINGS 10's shape
exactly. `Router` and `Submitter` now implement the traits, and
[`the_pipeline_has_real_implementations.rs`](crates/radar-exec/tests/the_pipeline_has_real_implementations.rs)
checks the trait methods delegate to the real ones rather than being present and
inert.

**There is still no production caller.** Nothing invokes the pipeline, for the
local wallet or a customer's. Writing one is opening the trading path, and it is
a decision about money rather than a wiring task. Every cost and failure
rate in it is supplied by the test rather than measured, and the real signer —
a separate process that re-decodes what this side built — is not in the loop.

The shipped policy is `Policy::CLOSED`, which refuses every proposal. But the lane
is shut a long way upstream of that too: on 2026-08-25 a live run over 41,254
candidates raised **zero proposals**, and the cause was a hardcoded exit-probe size
that made a proposal arithmetically impossible rather than a market that offered
nothing. `Policy::CLOSED` has never refused a real proposal, because it has never
been handed one. See [LEARNINGS](LEARNINGS.md) entry 10.

If you are changing `Policy::CLOSED`, you are making a decision about money — make
it deliberately, and not as a side effect of something else. Note that opening it
before the funnel has been exercised with a real proposal would be opening a path
nothing has ever tested.

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

   **This holds for a customer's capital too, and a connected wallet does not
   change it.** [ADR 0005](docs/adr/0005-customers-keep-custody-and-grant-radar-a-bounded-signer.md)
   settles the custody model: the customer keeps custody and grants Radar a
   bounded signer, whose policy is derived from the same `Policy` the kernel
   judged against — never from a model, a strategy, or anything the customer
   asserts. A connected wallet is **authentication, not authority**, and it may
   never soften a refusal.

   The signer's guarantee is *every account it authorises is one it read in the
   bytes it signed* — and it is worth stating precisely what that is a guarantee
   **against**, because a previous version of this paragraph said "absolute" and
   was read as more than it is.

   The signer does not verify that the `Authorization` it receives came from the
   kernel. There is no MAC on it and its `nonce` is never checked. So the
   property is *the transaction matches the authorisation the caller supplied*:
   a complete defence against an executor **bug**, which is what it was built
   for, and not one against a **compromised caller**, which writes its own
   authorisation. See [LEARNINGS](LEARNINGS.md) 23, and
   [ADR 0007](docs/adr/0007-the-privy-authorization-key-lives-in-the-signer-process.md)'s
   amendment for what that means for customer capital. This is why it refuses
   address lookup tables ([ADR 0003](docs/adr/0003-legacy-transactions-because-the-signer-must-be-able-to-read-them.md))
   and why it has no network, no listener and no method that signs arbitrary
   bytes. Anything that would let it sign something it has not fully read breaks
   the guarantee, however convenient.

   **The customer lane composes end to end as of 2026-09-01**, in
   [`the_customer_lane_composes.rs`](crates/radar-exec/tests/the_customer_lane_composes.rs),
   and its shape is different from the local one in the way that matters: **no
   process in Radar can produce a signature on its own.** Three parties hold one
   thing each — the executor an application credential that authorises nothing,
   `radar-signer` a P-256 key that authorises one request it has checked, and
   Privy the wallet key.

   What that test establishes: an authorised trade reaches Privy signed; a trade
   for another token stops **inside Radar** and never reaches the network; a
   `Policy::CLOSED` in the signer's own file refuses the identical request a
   permissive one allows; the body Privy receives is the body the signer
   authorised; and a spent signature allowance refuses before anything is
   signed. The signer in it is the real `verify::check`, not a stub.

   What it does not: nothing has been signed by Privy, sent, or filled, and no
   customer has ever existed. `Policy::CLOSED` is still shipped.

   **The customer path holds the same line, and it took an ADR to keep it.**
   Privy's API requires a `privy-authorization-signature` — an ECDSA P-256
   signature Radar makes with a key whose public half is registered as a signer
   on the customer's wallet. That key causes customer funds to move, which makes
   it the same category of object as the wallet key, so
   [ADR 0007](docs/adr/0007-the-privy-authorization-key-lives-in-the-signer-process.md)
   puts it in the signer and keeps it out of `radar-serve` — the process with a
   listener, a model provider, an HTTP client, an embedded frontend and a
   paywall.

   [`privy::authorise`](crates/radar-signer/src/privy.rs) takes a typed request
   and an `Authorization`, never bytes. It reads the transaction out of the body
   that will be sent and runs it through the same `verify::check` the local path
   uses, so the bytes checked are provably the bytes the signature causes to be
   signed. A caller able to hand over one transaction for checking and another
   for signing would make the check decorative, which is what a byte-signing
   method would allow.

2. **The risk kernel is pure.** No clock, no network, no ambient state, and no
   dependence on the order of its inputs. Purity is what makes a verdict
   replayable and a refusal reproducible from a recording.

3. **Nothing reads past its watermark.** Every read is gated by
   [`AsOf`](crates/radar-asof), and this is what keeps look-ahead bias out of
   research results. It reaches into the cache too: a replay must not be served a
   live-populated entry from the future.

   The enforcement is **at the boundary functions, not in the type system**, and
   it is worth being exact about that because an earlier version of this rule was
   not. There are two mechanisms and they are for different situations:

   - **Scans filter.** `Reader::read` and `Reader::read_outcomes` drop rows past
     the watermark as they go, because a partition file legitimately contains
     slots on both sides of it. Erroring there would make a normal read fail.
   - **Single observations are refused.** `AsOf::accept` takes an `Observed<T>`
     and returns `LookAhead` rather than a value. That is the right shape for one
     value arriving from outside the store, and nothing inside the store needs
     it today.

   So the guarantee is "every path out of the store applies the gate", which is a
   property of four call sites rather than something the compiler proves.
   [`crates/radar-store/tests/watermark_holds.rs`](crates/radar-store/tests/watermark_holds.rs)
   is what holds it up: it reads across a file that straddles the watermark,
   sweeps every boundary, and each of its cases was checked by deleting the
   filter and confirming the test fails.

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
   loaded refuses everything, a signer with no allowlist refuses everything, a
   paywall with no facilitator serves nothing rather than serving free, and
   `radar brief` with no serving endpoint configured reports that it cannot see
   rather than that nothing is wrong. Spending nothing is always recoverable.

   **The spend-meter half is wired for one component and not for the rest, and
   saying exactly which is the point.** `radar-provider` implements the budget,
   the commitment, the refusal and a [`Ledger`](crates/radar-provider/src/cost.rs)
   that can survive a restart.

   Until 2026-08-31 this paragraph said it *did* survive one, and it did not.
   `Agent::restore` had exactly one caller and it was a unit test; the startup
   path called `Agent::new`, so every restart began the day again at zero and a
   crash loop under `Restart=always` would have handed out a fresh allowance per
   crash. It had cost nothing when it was found — the service had no unplanned
   restarts — which is why it was worth finding then. Third occurrence of the
   pattern [LEARNINGS](LEARNINGS.md) entries 1 and 9 record.

   It is wired now, through [`ledger`](crates/radar-serve/src/ledger.rs), and
   [`the_budget_survives_a_restart.rs`](crates/radar-serve/tests/the_budget_survives_a_restart.rs)
   holds it up — checked by putting `Agent::new` back and confirming the test
   fails. `RADAR_STATE_DIR` is now required for the agent to run at all: a meter
   that cannot record what it spent cannot enforce a ceiling across a restart.

   As of 2026-08-27 the reading assistant goes through it: `radar-model`'s
   `budget_from_vars` has no default, so an instance with no
   `RADAR_MODEL_DAILY_USD` gets no agent at all rather than an unmetered one, and
   every model call reserves before it spends and releases when it fails. That is
   the first component in the running system that spends through a meter.

   Every component that spends money on *data* still does not. `radar-backfill`
   on CryptoHouse, `radar-sim` on Jupiter and RPC, and `radar-serve` on the
   facilitator each hold their own HTTP agent and pass through no meter. There is
   no daily ceiling on any of them. So this rule is enforced for the signer, the
   paywall and the agent, and **not** for data spend.

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
  policy: no HTTP, no clock, no async.

  **Read it as a design, not as the running system.** Nothing depends on this
  crate. The economics that actually run are a separate, static cost model in
  [`radar-instruments`](crates/radar-instruments/src/spec.rs), where each
  instrument *declares* its cost by hand ("a promise, not a measurement") and the
  x402 price is derived from that declaration. So the price Radar charges is not
  connected to what Radar spends, and nothing notices if the two diverge. This is
  the second time a documented-as-central economics layer has turned out to be
  unreachable; see [LEARNINGS](LEARNINGS.md) entries 1 and 9 for the pattern.
