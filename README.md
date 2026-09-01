<!-- SPDX-License-Identifier: Apache-2.0 -->
# Radar

Solana research and trading infrastructure, and an intelligence layer other
agents can buy over x402.

**Status: recording, and refusing to trade.** The pipeline runs end to end on a
live instance — recording launches, measuring what became of them, answering
questions about both over HTTP and MCP, and running the full decision lane over
what it has recorded.

The trading lane is **built and shut**. Strategy, risk kernel, signer and
executor all exist and are tested end to end; the policy that ships is
`Policy::CLOSED`, which refuses every proposal. That is the distinction that
matters: nothing is missing, and nothing is armed. Turning it on is a deliberate
change to one value, made by a person, once recorded data shows an edge exists.

`radar consider` prints exactly what the system would have done and why it did
not do it. On the first live run it considered 3,924 recorded launches, spent
nothing on paid analysis because no paid call could have changed an answer, and
proposed nothing — see
[docs/research/0005](docs/research/0005-first-end-to-end-decision-pass.md).

## What this is for

Radar is built to answer one question honestly: **does a measurable edge exist in
this market, and if so, where?**

That framing is deliberate, because the base rate is unforgiving. In April 2026 —
one of the strongest months in two years — roughly 96% of pump.fun wallets either
lost money or made under $500, and only 5.4% cleared $1,000. Meanwhile the
lowest-latency public data path shut down, and the firms still winning on speed
are buying private fiber. Latency is not an edge available here.

What is available is unglamorous and measurable:

- **Avoidance** — not buying the traps. Cheap, deterministic, compounding.
- **Exit-first sizing** — most losses are inability to exit, not bad entries.
- **Cost discipline** — fees, tips, slippage and failed transactions are a large
  share of retail P&L.
- **Composition** — creator history × funding graph × social/on-chain divergence
  is hard to compute, which is why it may not be arbitraged away yet.

So the first thing Radar builds is not a strategy. It is a **recorder with a
point-in-time guarantee**, because every signal has to be validated against
outcomes, and the dataset only accumulates forward.

## The idea that shapes the architecture

Radar buys its data per call. That turns every design decision into arithmetic,
and two answers fall straight out:

**Decode locally.** Parsed transactions cost $0.05 each; a launch analysis touches
50–200 of them. But `getBlock` returns *every* transaction in a slot for $0.001 —
including the create, the dev buy, and every same-slot coordinated buy. Decoding
in Rust rather than buying it parsed is worth roughly **100–300×** on the bill,
and it is the real justification for the Rust layer.
See [ADR 0001](docs/adr/0001-decode-locally-never-buy-parsed-transactions.md).

**Never pay twice for the same fact.** Every fact carries a *mutability class*.
A token's structural facts are fixed within seconds of launch and consulted on
every re-evaluation, so they are fetched once per mint and never again. Authority
revocations are one-way latches: watched until they close, then never asked about
again. This needs nothing from any vendor and it is the largest saving available.

## Layout

| Crate | What it does |
|---|---|
| `radar-types` | Domain vocabulary. Slots as the only clock, integer money, mutability classes, provenance and trust tiers. |
| `radar-asof` | Point-in-time correctness. A watermark that makes look-ahead bias a compile-time concern rather than a discipline. |
| `radar-provider` | The metered, cached, health-aware data plane. Pure policy — no HTTP, no clock, no async — so spend control is exhaustively testable without a network. |
| `radar-agent` | The boundary a reasoning layer sits behind. Pure policy: the model gets read-only tools, observed text is fenced as data, and its answer is text nobody parses into an action. |
| `radar-model` | Reaching a model provider: the vendor CLI as a subprocess that owns its own credential, or a metered API key. The impure edge `radar-agent` deliberately does not have. |
| `radar-decode` | Solana program decoders. Matches Anchor discriminator bytes, never logged instruction names; an unrecognised discriminator is a recorded value, never a guess. |
| `radar-store` | Append-only, slot-partitioned Parquet event log. Nulls mean "not recoverable", never zero; the watermark reaches onto disk. |
| `radar-backfill` | Bulk historical extraction from CryptoHouse, decoded by the same decoder the live recorder uses. |
| `radar-instruments` | The instrument registry. One declaration; internal, HTTP, x402 and MCP surfaces derived from it, and every invocation recorded. |
| `radar-serve` | Ops page, JSON API, stateless MCP (2026-07-28), and the x402-priced public surface. |
| `radar-sim` | Exit analysis. Structural disqualification from the mint account, then a measured sell curve — never a single liquidity number. |
| `radar-risk` | The risk kernel. A pure function from a proposal to a verdict — the only thing that can authorise capital. |
| `radar-strategy` | Deterministic strategies. They emit proposals, which are inert data, and assemble candidates in one place so look-ahead is prevented once rather than per strategy. |
| `radar-graph` | Coordination detection. Scores what a launch block contained, and refuses on the shape that 68% of instantly-graduating launches share and 5% of ordinary ones do. |
| `radar-research` | Replay. Re-runs a recorded decision at its original watermark and separates a store that gained history from a strategy that is not a pure function of its inputs. |
| `radar-signer` | A separate process holding the key. Re-decodes every transaction and trusts nothing the caller said about it. |
| `radar-exec` | Route, gate, sign, submit, reconcile. The last stage, and the one holding the least authority. |
| `radar-pumpfun` | Builds pump.fun instructions and prices them off the bonding curve. Pure: no clock, no network, no key. The curve is `x·y=k` over published reserves, so a fill is computed rather than quoted — see ADR 0009. |
| `radar-customer` | The customer model. Pure: a bounded grant derived from the kernel's authorisation, and a signature meter. No account table — see ADR 0006. |
| `radar-cli` | The operator command line. Reads live state and computes nothing it does not have. |

Further crates (graph, sim, strategy, exec, signer, research) are planned; see
the architecture plan.

## The safety invariant

Borrowed from GitLocus and applied to money: **model judgement never authorises
capital.**

```
strategy or model  --emits-->  Proposal       inert data, zero authority
risk kernel        --emits-->  Authorization  pure fn; nonce, expiry, hard bounds
signer process     --emits-->  Signature      re-derives the tx, trusts nothing
```

`radar_risk::evaluate` has no clock, no network and no ambient state — the
current slot is an argument and so is the portfolio. So every past decision can
be replayed, every refusal is reproducible from a recording, and any decision can
be re-judged under a tighter policy without ever having run it that way. The
default policy refuses everything.

## What it does today

Recording continuously from the chain, measuring each token at roughly one hour,
six hours and a day after launch, and serving the result:

```
$ radar creators --store ./data/store -n 3
distinct creators: 1148  (418 launched more than once)

LAUNCHES  CREATOR
     88  6LdWMVxj6R7683M9ioAcaFNRfUhcr9v9K2xNjYd9Fnbx
     44  EH7aHeLEz8wd9wqeMuT364ntXM93j6Mo5cbmjQFhKY9S
     41  bwamJzztZsepfkteWRChggmXuiiCQvpLqPietdNfSXa
```

That first address, through `creator_track_record`: 88 launches, 39 of them old
enough to have been measured, **100% stillborn**, median survival **0 slots**.
The population rate across all measured tokens is ~35%.

The instrument reports `measured=39` rather than 88 because the other 49 have not
yet reached their first checkpoint. It says what it knows and no more, which is
the same reason it will not state a rate below five measured launches.

Whether creator history *predicts returns* is a different and larger question,
and nobody has measured it yet. What exists is the machinery to ask: signals
computed at a watermark, outcomes measured later, and a replay that must
reproduce both.

## Trying it

```bash
cargo run -p radar-backfill -- --from '2026-08-21 06:00:00' --to '2026-08-21 06:30:00' --store ./data/store
cargo run -p radar-cli -- creators --store ./data/store
RADAR_STORE=./data/store cargo run -p radar-serve
```

The last one serves an ops page at `/`, the instrument catalogue at
`/v1/instruments`, and a stateless MCP endpoint at `/mcp`. The x402-priced public
surface appears only when `RADAR_X402_PAY_TO` and `RADAR_X402_FACILITATOR` are
set — it is never served free as a fallback.

## Measured, not assumed

`scripts/probe/` holds the scripts that produced every number in
[docs/research/](docs/research/), because a figure without the thing that
produced it is a claim rather than a measurement. Running them before writing
decoders overturned three assumptions while they were still cheap to change —
see [0004-measured-launch-base-rates.md](docs/research/0004-measured-launch-base-rates.md).

## Building

```bash
just check
```

Requires the stable Rust toolchain and [`just`](https://just.systems). On Windows
under Git Bash, `export RADAR_CARGO="cargo +stable-x86_64-pc-windows-gnullvm"`
first — [AGENTS.md](AGENTS.md) says why.

## Documentation

- [AGENTS.md](AGENTS.md) — how to work here, and the invariants that are not
  negotiable.
- [docs/adr/](docs/adr/) — decisions, and what each one costs.
- [docs/research/](docs/research/) — the data-sourcing landscape, the
  freshness/caching design, and the vendor feature requests that would change
  the architecture if granted.

## Licence

Apache-2.0. See [LICENSE](LICENSE).
