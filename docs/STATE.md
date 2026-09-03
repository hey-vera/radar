<!-- SPDX-License-Identifier: Apache-2.0 -->
# STATE.md - what has actually been measured and built

Split out of [AGENTS.md](../AGENTS.md) so that file can be an operating
policy rather than a reference. Everything here is a **claim about the
world**, and claims decay: when this file and the repository disagree, the
repository is right and this file is a bug worth fixing in the same change.

The four most likely to be stale: the measured edge, what the trading lane
can reach, which venues are recorded, and whether anything has traded.

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
[`0014`](../docs/research/0014-the-control-was-entirely-tokens-nobody-could-sell.md)
measured Radar's proposals at a gross median of +21 bps and called that noise
around zero. It is not noise: that figure compares a **sell quote** against a
**mid**, and [`0016`](../docs/research/0016-the-entry-was-a-bid-and-the-exit-was-a-mid.md)
measures the gap between those two instruments at **at least +128 bps** — six
times the signal it was hiding. Corrected, the gross median is **at most −107
bps**, and **−957** after the measured 850 bps round trip.

The comparison against refusals that appeared to make it worse is unusable:
every scoreable refusal is `CapacityBelowFloor`, so the control is composed
entirely of tokens Radar measured and found it could not sell.

[`0017`](../docs/research/0017-a-control-that-could-have-been-traded.md) builds the
control that comparison lacked, against 121,810 tokens Radar never decided on,
priced the same way on both sides and matched on token age and holding period.
It finds **no edge** — a median edge of 0 bps across four matched strata. Both
that note and [`0018`](../docs/research/0018-the-deep-tail-points-the-wrong-way.md)
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
[`crates/radar-exec/tests/lane_composes.rs`](../crates/radar-exec/tests/lane_composes.rs):
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
[`the_pipeline_has_real_implementations.rs`](../crates/radar-exec/tests/the_pipeline_has_real_implementations.rs)
checks the trait methods delegate to the real ones rather than being present and
inert.

**There is still no production caller** for the *trading* path. Nothing invokes
the pipeline, for the local wallet or a customer's. Writing one is opening the
trading path, and it is a decision about money rather than a wiring task.

`radar route` is the first thing that runs any of it. It builds a swap
transaction and describes it, holding no key and no RPC endpoint, so it cannot
sign and cannot send. Every cost and failure rate in that composition is supplied
by the test rather than measured.

**[`0021`](../docs/research/0021-the-signer-cannot-read-the-only-venue-that-lists-them.md)
found that Radar could not build a transaction for any token it selects, and
never could** — Jupiter routes pre-graduation pump.fun liquidity only as a
versioned transaction, the signer accepts only legacy ones (ADR 0003), and Radar
selects only pre-graduation pump.fun tokens. Eight of eight candidates confirmed
it. LEARNINGS 24.

**That is fixed as of 2026-09-02, and it is worth knowing what the fix does and
does not give you.** [ADR 0009](../docs/adr/0009-radar-builds-its-own-pump-fun-swaps.md)
builds the swap directly rather than asking an aggregator for one, in
[`radar-pumpfun`](../crates/radar-pumpfun) — a pure crate with no network and no key.
A buy rebuilt from a mainnet capture **simulates against mainnet with no error**,
and `radar-signer`'s real `verify::check` reads a transaction that crate built
([`the_signer_reads_what_this_crate_builds.rs`](../crates/radar-pumpfun/tests/the_signer_reads_what_this_crate_builds.rs)),
including refusing one whose authorisation names a different mint. The venue was
never the obstacle: every capture behind that crate is a **legacy** transaction.

What it does not give you: nothing has been signed by a wallet, sent, or filled.
A simulation with `sigVerify: false` proves the instruction is well formed and the
accounts resolve. It does not prove a signed transaction lands, or at what price.

Two things that came out of building it and change other numbers.
[`0023`](../docs/research/0023-the-fee-is-a-schedule-and-the-published-interface-is-incomplete.md)
reads the venue fee off the chain at **125 bps a side** — 250 for a round trip,
which is what 0022 assumed without checking — and finds that the program's own
published IDL declares **sixteen** accounts for a buy where mainnet passes
**eighteen**. A builder written against the most authoritative available reference
would have been two accounts short. LEARNINGS 25.

The shipped policy is `Policy::CLOSED`, which refuses every proposal. But the lane
is shut a long way upstream of that too: on 2026-08-25 a live run over 41,254
candidates raised **zero proposals**, and the cause was a hardcoded exit-probe size
that made a proposal arithmetically impossible rather than a market that offered
nothing. `Policy::CLOSED` has never refused a real proposal, because it has never
been handed one. See [LEARNINGS](../LEARNINGS.md) entry 10.

If you are changing `Policy::CLOSED`, you are making a decision about money — make
it deliberately, and not as a side effect of something else. Note that opening it
before the funnel has been exercised with a real proposal would be opening a path
nothing has ever tested.

## Where to start

- [`docs/research/`](../docs/research/) — what was investigated and what it found,
  including the data-sourcing landscape and the freshness/caching design.
- [`docs/adr/`](../docs/adr/) — decisions, with what each one costs.
- `crates/radar-provider` — the metered, cached, health-aware data plane. Pure
  policy: no HTTP, no clock, no async.

  **Read it as a design, not as the running system.** Nothing depends on this
  crate. The economics that actually run are a separate, static cost model in
  [`radar-instruments`](../crates/radar-instruments/src/spec.rs), where each
  instrument *declares* its cost by hand ("a promise, not a measurement") and the
  x402 price is derived from that declaration. So the price Radar charges is not
  connected to what Radar spends, and nothing notices if the two diverge. This is
  the second time a documented-as-central economics layer has turned out to be
  unreachable; see [LEARNINGS](../LEARNINGS.md) entries 1 and 9 for the pattern.
