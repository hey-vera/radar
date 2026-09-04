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
control that comparison lacked, against **38,461** tokens Radar never decided on,
priced the same way on both sides and matched on token age and holding period.
It finds **no edge** — a median edge of 0 bps across four matched strata. Both
that note and [`0018`](../docs/research/0018-the-deep-tail-points-the-wrong-way.md)
were re-measured after LEARNINGS 19 corrected the pairing gate, and both
conclusions survived it.

The figures are the **corrected** run: 990 proposals against 38,461 control
tokens. An earlier version of this paragraph carried 121,810, which is 0017's
superseded pre-correction control and is ~3.2× the real one. 0017's own header
line still carried the superseded pair until 2026-09-03; both are now fixed.

0018 is the one to read next. Radar sizes every position as a share of measured
exit capacity, and 80% of proposals sit in a ±13% band around $31 because every
pre-graduation pump.fun token rides the same curve — so the median position is
**$6.21**, needing a +8.5% move to clear the round trip. The one band where a
real position fits, $60+, is **down 68% on 25 rows**.

**0018 concluded from that "the binding constraint is capacity, not signal", and
that conclusion is superseded.** This file said it too, and it was wrong for as
long as 0022 has existed.
[`0022`](../docs/research/0022-capacity-was-a-budget-not-a-ceiling.md) reads four
bonding curves off mainnet and finds the ~$31 is the output of Radar's own
`Search::DEFAULT { max_impact_bps: 100 }` — a **policy setting, not a venue
wall**. The same curve supports roughly **$500** at an impact equal to the round
trip, a factor of about sixteen. 0022 struck its own recommendation to raise the
budget and said why in one line:

> "**Capacity was never the reason this does not work.**"

**The binding constraint is the bar, not the depth.** A strategy has to clear
roughly **456 bps** of expected edge before a single trade is worth making, and
roughly **850 bps** before a position larger than about $59 makes sense.
[`0017`](../docs/research/0017-a-control-that-could-have-been-traded.md) measures
the edge at **0 bps**, so the arithmetically correct size is not small but
negative — which is what `Policy::CLOSED` already is. `max_impact_bps` should not
move until something measures an edge above that bar; raising it first would only
spend money faster in the direction 0017 says there is nothing in.

Read that before adding a filter. The gross median says the selection is not
finding an edge, and the round trip is currently larger than anything the filter
has found.

## The two measured signals, re-measured

Both were measured on a two-day window at 72,193 launches. The store now holds
483,629, and [`0024`](../docs/research/0024-the-spike-became-a-hump-and-the-signal-moved.md)
re-ran both on 2026-09-03.

**The launch-block signal did not survive intact.** `0008`'s headline — 68% of
instantly-graduating launches have exactly six recipients — is **25.1%** on
17,497 launches. The enrichment over the 5.5% control is real; the magnitude was
wrong by 2.7×, and the "spike with holes either side" that made the structural
argument was sampling rather than structure. The strongest band has moved off six
to **ten to thirteen** (10.1× base). `0008` predicted exactly this failure — *"six
is a tool's default, not a law"* — and nothing was built to notice it, so it was
found nine days later by hand.

What replaces it is stronger and points the same way the project already points:
**one to three recipients covers 70.5% of launches and graduates instantly in
0.02% of them.** The reliable signal is the refusal.

**The creator signal strengthened.** `0007`'s separation holds on 2,957 creators
against its original 638, at **3.37×** band-to-band (0007 had 2.12×) and **1.59×**
against the base rate — and it survives controlling for launch frequency in all
three bands. Neither is a reason to buy: `0011` still says graduation predicts
volatility rather than profit, at a median −3,228 bps.

The numbers a consumer should read are in
[`data/0024-base-rates.json`](../docs/research/data/0024-base-rates.json), each
carrying the date it was measured.

## The round trip is three numbers, and they are not in conflict

Three figures circulate in this repository and **no document reconciled them
until this one did**. They measure different things, and quoting the wrong one
understates cost by up to 3.4×. Anything that publishes a cost figure — the
kernel, a research note, the roaster — resolves it here.

| bps | what it is | measured in | scope |
|---|---|---|---|
| **250** | the pump.fun **venue fee**, round trip — 125 bps a side, read off the on-chain `FeeConfig` | [`0023`](../docs/research/0023-the-fee-is-a-schedule-and-the-published-interface-is-incomplete.md) | the fee alone. Excludes impact, rent and failed transactions. |
| **456** | the **bar**: expected edge a strategy must clear before one trade is worth making | [`0022`](../docs/research/0022-capacity-was-a-budget-not-a-ceiling.md) | all-in round trip for a position in the **$20–$200** band, used as the fee rate `a` in `s* = (r − a) / 2b` |
| **850** | the **measured all-in round trip** the kernel assumes, `creator_edge::Thresholds::assumed_round_trip_bps` | [`0019`](../docs/research/0019-the-round-trip-is-not-one-number.md) | fresh-launch pump.fun tokens — the cohort Radar's own trades belong to. Every net figure in this repository rests on it. |

**Why they differ, precisely.** 0019 measured 183,647 legs and found cost is
**size-dependent, and the dependence is at the bottom**:

```
notional band   median bps / leg   round trip
$0.20 – $2                 1,521       3,042
$2 – $20                     125         250
$20 – $200                   228         456
$200 – $2,000                225         450
$2,000+                      130         260
```

So 250 and 456 are the same measurement read in two different bands, and 0022's
"fees rt" column is this table doubled rather than a fee schedule. That 250
coincides with 0023's venue fee is a real finding rather than a clash: in the
$2–$20 band the measured all-in cost is **the venue fee and essentially nothing
else**.

**850 is none of these bands and is not superseded by them.** 0019 measured *all*
pump.fun trades in an hour; the 850 came from trades on 200 tokens launched in
that hour — the fresh-launch cohort, where a new associated token account is rent
and early curve positions carry more slippage. 0019 **explicitly declines to
lower the constant** on that evidence, because the population is wrong and the
error direction is the dangerous one: a cost estimate rounded down launders a
trade past the gate that should have refused it.

**The number nobody else publishes.** `min_notional` is `MicroUsd::DOLLAR` —
$1.00 — which lands in the 1,521 bps band. **A position at Radar's own floor
faces a round trip of roughly 30%.** That is measured, it is about the venue
rather than a prediction, and it is the strongest single fact this repository
holds.

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

**What it does not.** One production crate depends on `radar-exec`:
`radar-cli` reaches `radar_exec::route` for `radar route`, which prints unsigned
bytes. Nothing in production reaches `pipeline::execute`, the signer client or
the submitter — the composition reaches *those* through a dev-dependency, so the
shipped graph still cannot sign. Nothing has been signed, sent, or filled.
`repo-conformance`'s `the_documented_dependency_claims_are_true` pins it.

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

**The customer lane also composes, as of 2026-09-01**, in
[`the_customer_lane_composes.rs`](../crates/radar-exec/tests/the_customer_lane_composes.rs),
and its shape differs from the local one in the way that matters: **no process in
Radar can produce a signature on its own.** Three parties hold one thing each —
the executor an application credential that authorises nothing, `radar-signer` a
P-256 key that authorises one request it has checked, and Privy the wallet key.
Moved here from AGENTS.md rule 1 on 2026-09-03: it is status, and rule 1 is a
rule.

**What that test establishes.** An authorised trade reaches Privy signed. A trade
for another token stops **inside Radar** and never reaches the network. A
`Policy::CLOSED` in the signer's own file refuses the identical request a
permissive one allows. The body Privy receives is the body the signer authorised.
A spent signature allowance refuses before anything is signed. The signer in it
is the real `verify::check`, not a stub.

**What it does not.** Nothing has been signed by Privy, sent, or filled, and no
customer has ever existed. `Policy::CLOSED` is still what ships.

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

## The public analyst, as of 2026-09-04

**Code-complete and posting nothing.** `radar dossier`, `radar roast` and
`radar analyst --mentions <file>` run offline; `radar-analyst` is also a daemon
under `deploy/radar-analyst.service` that polls mentions, answers them, meters
what it spends and logs every reply beside the fact sheet it was built from.

**The only thing between it and a public account is a credential and a
decision.** `RADAR_X_BEARER` and `RADAR_X_USER_ID` is the switch; with either
absent the publisher is the dry run and nothing can be posted. The four prices
have **no defaults**, so an instance that has not been told what it is charged
answers nothing rather than spending an unmetered amount — which is what makes
the two unverified X billing figures a line in a config file rather than a
blocker.

Three properties are load-bearing and each is pinned by a test that was verified
by re-applying the bug it prevents:

- **A reply is recorded before it is said.** `publish` appends the intent, posts,
  then appends the outcome. It used to post first, and a failed log write would
  have left a public statement with no record of it.
- **The reply is sanitised before it is checked, never after.** The forbidden and
  fidelity checks read characters, and a zero-width space renders as nothing —
  `s<ZWSP>cam` is two tokens to a checker and one word to a reader. Cleaning
  afterwards would assemble exactly the statement the checks refused.
- **The gate refuses before the chain is read**, because the read is the
  expensive part.

Nothing here touches the store, the signer or `Policy::CLOSED`. It is read-only
against the chain and append-only against its own log.

## Where to start

- [`docs/research/`](../docs/research/) — what was investigated and what it found,
  including the data-sourcing landscape and the freshness/caching design.
- [`docs/adr/`](../docs/adr/) — decisions, with what each one costs.
- `crates/radar-provider` — the spend meter, and as of 2026-09-04 nothing else.

  What runs, and did before: `radar-agent` reserves against `Budget`, `Ledger`,
  `Meter` and `Commitment` for every model call, and
  [`radar-serve`'s ledger](../crates/radar-serve/src/ledger.rs) persists that
  across a restart. It is rule 8 enforced in the running system.

  What is gone: the cache, the breaker and the planner that composed them —
  about 1,300 lines against the 484 that run, with no caller outside the crate
  and none since it was written. This file called for them to be wired or
  deleted, three documents flagged them, and deleting is what happened
  (design 0007 J9). The crate's own module doc records why, and what to extend
  instead: `radar-serve` has a live cache that already carries the lesson
  LEARNINGS 27 paid for.

  Two consequences worth knowing. `radar-asof`'s `Observed<T>` and `LookAhead`
  now have **no caller** — the deleted cache was the only one — so `radar-asof`
  is `AsOf` plus two types nothing uses. And AGENTS.md rule 3 no longer claims
  a type-system half; it was describing that cache.

  The economics that actually run for *pricing* are a separate, static cost model in
  [`radar-instruments`](../crates/radar-instruments/src/spec.rs), where each
  instrument *declares* its cost by hand ("a promise, not a measurement") and the
  x402 price is derived from that declaration. So the price Radar charges is not
  connected to what Radar spends, and nothing notices if the two diverge.
