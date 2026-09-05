<!-- SPDX-License-Identifier: Apache-2.0 -->
# Radar

Solana research and trading infrastructure, and an intelligence layer other
agents can buy over x402.

**Status: recording, and refusing to trade.** The pipeline runs end to end on a
live instance — recording launches, measuring what became of them, answering
questions about both over HTTP and MCP, and running the full decision lane over
what it has recorded. 479,564 launches, 1,326,862 outcome measurements and 7,543
replayable decisions as of 2026-09-03.

**Nothing has ever traded, and no edge has been found.** The measured selection
edge is **0 bps** ([0017](docs/research/0017-a-control-that-could-have-been-traded.md))
against a bar of roughly **456 bps** — the expected edge a strategy must clear
before a single trade is worth making
([0022](docs/research/0022-capacity-was-a-budget-not-a-ceiling.md)). The shipped
`Policy::CLOSED` refuses every proposal, and on that arithmetic it is the correct
position rather than a placeholder.

The trading lane is **composed and shut**, and an earlier version of this section
said it was *"built and tested end to end"* with *"nothing missing"*. That
sentence is exactly what [LEARNINGS](LEARNINGS.md) 10 retracts, so here is the
distinction it was hiding:

- **Composed** — `radar-exec`'s composition tests run strategy → kernel →
  signer → executor, and `Policy::CLOSED` refuses what a permissive policy
  authorises. [docs/STATE.md](docs/STATE.md) owns the account of what those
  tests do and do not establish, for the local lane and the customer one, and
  names them.
- **Not exercised** — nothing has been signed by a wallet, sent, or filled.
  One production crate depends on `radar-exec`: `radar-cli`, which reaches
  `radar_exec::route` for `radar route` and prints unsigned bytes. Nothing in
  production reaches `pipeline::execute`, the signer client or the submitter, so
  **there is no production caller for the trading path**, and writing one is a
  decision about money rather than a wiring task. `repo-conformance`'s
  `the_documented_dependency_claims_are_true` pins all three statements; the
  earlier claim that *nothing* depended on `radar-exec` was false from
  2026-08-31 and went uncaught for three days (LEARNINGS 29).
- **`Policy::CLOSED` has never refused a real proposal, because it has never been
  handed one.** A live run over 41,254 candidates raised zero proposals — the
  cause was a hardcoded probe size that made a proposal arithmetically
  impossible, not a market offering nothing (LEARNINGS 10).

**The trading lane is frozen** for the duration of the current work. It unfreezes
on one condition: a measured edge at or above that 456 bps bar, in a stratum
Radar can size into, on data that was not used to find it.

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
| `radar-asof` | Point-in-time correctness. A watermark applied at every boundary function that reads the store — four `admits` call sites plus one hand-rolled gate, not a compile-time guarantee. LEARNINGS 9 retracts the stronger claim this row used to make. |
| `radar-provider` | Spend metering, and now only that. Pure policy — no HTTP, no clock, no async. `Meter`/`Ledger`/`Budget`/`Commitment` run: `radar-agent` reserves before every model call and `radar-serve` persists the ledger across restarts. Its cache, breaker and planner had no caller anywhere and were **deleted on 2026-09-04**, taking the crate from 1,897 lines to 484. |
| `radar-agent` | The boundary a reasoning layer sits behind. Pure policy: the model gets read-only tools, observed text is fenced as data, and its answer is text nobody parses into an action. |
| `radar-model` | Reaching a model provider: the vendor CLI as a subprocess that owns its own credential, or a metered API key. The impure edge `radar-agent` deliberately does not have. |
| `radar-decode` | Solana program decoders. Matches Anchor discriminator bytes, never logged instruction names; an unrecognised discriminator is a recorded value, never a guess. |
| `radar-store` | Append-only, slot-partitioned Parquet event log. Nulls mean "not recoverable", never zero; the watermark reaches onto disk. |
| `radar-backfill` | Bulk historical extraction from CryptoHouse, decoded by the same decoder the live recorder uses. |
| `radar-instruments` | The instrument registry. One declaration; internal, HTTP, x402 and MCP surfaces derived from it, and every invocation recorded. |
| `radar-serve` | Ops page, JSON API, stateless MCP (2026-07-28), the x402-priced public surface, and the public site's three documents — stats, leaderboard, pool — served by exact path from published files, never the store. |
| `radar-sim` | Exit analysis. Structural disqualification from the mint account, then a measured sell curve — never a single liquidity number. |
| `radar-risk` | The risk kernel. A pure function from a proposal to a verdict — the only thing that can authorise capital. |
| `radar-strategy` | Deterministic strategies. They emit proposals, which are inert data, and assemble candidates in one place so look-ahead is prevented once rather than per strategy. |
| `radar-graph` | Coordination detection. Scores what a launch block contained. Its thresholds derive from [0008](docs/research/0008-the-launch-block-gives-the-bundle-away.md), which [0024](docs/research/0024-the-spike-became-a-hump-and-the-signal-moved.md) superseded on 17,497 launches: exactly six recipients is **25.1%** of instant graduations, not 68%, and the strongest band has moved to ten-to-thirteen. The rule has not been re-derived — but **the count is now recorded** alongside the verdict (`Decision.launch_recipients`), which is [ADR 0012](docs/adr/0012-the-launch-block-count-is-recorded-not-the-threshold-retuned.md)'s first commitment and what makes the next re-derivation a query over the store rather than another chain scan. |
| `radar-research` | Replay. Re-runs a recorded decision at its original watermark and separates a store that gained history from a strategy that is not a pure function of its inputs. |
| `radar-signer` | A separate process holding the key. Re-decodes every transaction and trusts nothing the caller said about it. |
| `radar-exec` | Route, gate, sign, submit, reconcile. The last stage, and the one holding the least authority. |
| `radar-pumpfun` | Builds pump.fun instructions and prices them off the bonding curve. Pure: no clock, no network, no key. The curve is `x·y=k` over published reserves, so a fill is computed rather than quoted — see ADR 0009. |
| `radar-customer` | The customer model. Pure: a bounded grant derived from the kernel's authorisation, and a signature meter. No account table — see ADR 0006. |
| `radar-onchain` | The on-demand chain read behind `radar dossier`. Rebuilds a token's launch block and prices its curve from RPC in a bounded number of calls, for a mint that may be forty seconds old — which the store cannot answer, being minutes behind by design and holding the launch-block verdict rather than the number. Read-only: no key, no signer in its tree. |
| `radar-roast` | The public analyst's reply. A typed fact sheet built from the dossier and the published base rates, a deterministic verdict, a voice pass that picks the headline and the tone, and two checks after generation: every numeral must appear on the sheet, and a list of claims may never be published. Either check failing ships the deterministic template instead. Prints; it holds no credential and cannot post. |
| `radar-analyst` | The summoned-reply loop, and the daemon that runs it. Only a base58 mint or a `$TICKER` survives parsing a mention, so there is no field an instruction can travel in; an admission gate caps chain reads per summoner and replies per day and per mint; a spend meter reserves before every call and refuses everything when unfunded; every reply is recorded **before** it is said and again after, so a published statement always has a record. The X client exists and posts nothing without `RADAR_X_BEARER` and `RADAR_X_USER_ID` — with neither set the publisher is a dry run, which is what ships. |
| `radar-contest` | The weekly contest, pure. Weeks that open Monday 00:00 UTC, the published score over the bot's **own** replies, every exclusion returned with its reason rather than dropped, a winner with a cooldown, the seven-day claim, one JSON record per week, and the three-line payout policy `radar-payout` must ask before it signs. No clock, no network, no key. Its callers — the public leaderboard, the week-close job, the payout — land in plan 0006, in that order. |
| `radar-cli` | The operator command line. Reads live state and computes nothing it does not have. |

Every crate in that table exists and is a workspace member; `repo-conformance`
fails the build if the table and the workspace disagree. (This paragraph
previously listed graph, sim, strategy, exec, signer and research as *planned*,
five lines after the table listed them as built.)

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

Whether creator history *predicts returns* has since been measured, and the
answer is not the encouraging one. A creator with a prior organic graduation
graduates again at about **1.69× the base rate**
([0007](docs/research/0007-does-creator-history-predict-anything.md)) — but
graduation predicts **volatility, not profit**: organic graduations end at a
median **−3,228 bps** against −853 for tokens that never graduated
([0011](docs/research/0011-graduation-predicts-volatility-not-profit.md)). A
creator's graduation history is not a good sign, and nothing built on this data
may imply that it is.

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
