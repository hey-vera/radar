<!-- SPDX-License-Identifier: Apache-2.0 -->
# ADR 0002 — Historical data comes from CryptoHouse, not a vendor archive

**Status:** accepted, 2026-08-22

## Context

Radar needs history. Every signal has to be validated against outcomes, and the
recorder only accumulates forward — so without history, Phase 2 waits weeks
before it can say anything, and can only ever speak about one market regime.

Reconstructing history from RPC is not an option. Six months is ~39M slots; at
$0.001 per `getBlock` that is **~$38,900** and ~267 TB of transfer. Per-transaction
is worse: ~400M pump.fun transactions at $0.001 each is **~$400,000**.

The obvious answer was to buy it. NoLimitNodes sells a parsed pump.fun archive —
every create, trade and graduation since the program deployed, seven Parquet
tables, $200/month rolling with one-time pulls quoted separately.

Two things argued against paying before checking alternatives.

**The attractive tables are the unusable ones.** `token_lifecycle` carries peak
market cap and `creator_aggregates` carries graduation rates — both computed over
all time. They are fine as *labels* and radioactive as *features*: using either at
slot N leaks information from after slot N, which is the exact failure
[`radar-asof`](../../crates/radar-asof) exists to make impossible. They would have
to be recomputed point-in-time from the raw event tables anyway.

**A purchased parse is a second source of truth.** Radar already decodes
pump.fun (ADR 0001). Ingesting someone else's parse means history and live data
travel different code paths, so the replay test — run a recorded decision at its
original `as_of` and require an identical result — would be comparing two
different decoders rather than checking one.

## What was found instead

**CryptoHouse** (ClickHouse + Goldsky, `crypto-clickhouse.clickhouse.com`) is a
free public ClickHouse holding the whole chain:

| `solana` table | Rows | Size |
|---|---|---|
| `instructions` | **1.31 trillion** | 120.78 TiB |
| `transactions` | 574.11 billion | 163.12 TiB |
| `token_transfers` | 211.61 billion | 11.74 TiB |
| `blocks` | 418.88 million | 32.08 GiB |
| `tokens` | 30.15 million | 4.53 GiB |

`solana.instructions` carries raw bytes rather than someone else's parse:

```
block_slot Int64      -- the point-in-time key
block_timestamp       tx_signature String
index Int64           -- ordering, for same-slot clustering
parent_index Int64    -- the CPI tree
data String           -- RAW instruction bytes, base58
program_id String
accounts Array(String)-- present in the schema, empty in practice; see below
```

Verified end to end. One hour of pump.fun instructions, grouped by the first
eight bytes of `base58Decode(data)`, returns a histogram whose top entries are
exactly `radar-decode`'s table:

```
38fc74089edfcd5f  563,591  BuyExactSolIn      d6904cec5f8b31b4  1,714  CreateV2
33e685a4017f83ad   53,131  Sell               5df6823ce7e940b2 23,576  SellV2
66063d1201daebea   49,386  Buy                b817ee6167c5d33d  4,962  BuyV2
```

1,714 launches in one hour is **~41,000/day**, consistent with the independent
mainnet probe. Data is current to today.

### What `solana.instructions` does not carry

Its `accounts` array is **empty for every pump.fun row**, and `parsed`/`params`
are empty too — CryptoHouse only parses programs it has decoders for. So that
table alone answers "which instruction, with what arguments" and cannot answer
"which mint".

`solana.transactions` closes the gap, joined on signature:

```
accounts            Array(Tuple(pubkey String, signer Bool, writable Bool))
balance_changes     Array(Tuple(account String, before Decimal, after Decimal))
pre_token_balances  Array(Tuple(account_index, mint, owner, amount, decimals))
post_token_balances Array(Tuple(account_index, mint, owner, amount, decimals))
index               Int64     -- position in the block
fee, err, status, compute_units_consumed, log_messages
```

That is more than `getBlock` gives cheaply, and it changes what the recorder can
know. `balance_changes` and the token-balance pair carry **realised** amounts,
where the instruction arguments carry only what the trader asked for — a
`buy(tokens, max_sol_cost)` says nothing about what was actually spent. Fills,
slippage and effective price come from the deltas; without them the fill model
would have to be inferred.

`index` gives position in the block, which is the input to same-slot clustering.
`err` and `status` mean failed transactions are visible — and a failed buy is
real information about a token, not an absence of one.

## Decision

**Radar takes its history from CryptoHouse and buys no vendor archive.**

The extraction is a one-time bulk pull into Radar's own Parquet store: raw
pump.fun instructions from `solana.instructions`, joined to `solana.transactions`
on signature for the mint, the realised amounts and the block position, and
decoded by `radar-decode` — the same decoder the live recorder uses. History and
live data become one code path over one schema.

### Two limits that decide what a query may look like

Both were found the hard way and neither is documented anywhere obvious.

**`max_result_rows = 1000`, `readonly = 1`.** Raising it is refused. Aggregates
are unaffected because they return one row per group however much they scan,
which is why every research query worked and only bulk extraction hit it — and
why outcomes are extracted as per-mint aggregates while raw trades are not.

**`max_rows_to_read = 10 billion`.** `token_transfers` is partitioned by
`block_timestamp`, so that is the only bound that prunes. A `block_slot >= N`
filter reads perfectly naturally, prunes nothing, and fails at the ceiling *even
for a single mint*. With a timestamp bound the same query over 376 mints returns
in 8.5 seconds.

The failure gives no hint of this: the server answers a bad column with a bare
`404` and an over-large scan with a `500`, and both arrive with the explanation
in the response body that a client is likely to discard. Radar's client now
reports the body and the first hundred characters of the offending query,
because "HTTP 404" on its own cost a debugging round trip.

## Consequences

**Cost goes from $200/month, or an unquoted lump sum, to zero.**

**Correctness improves rather than being traded away.** Raw bytes plus
`block_slot` means the archive is point-in-time correct by construction, and the
replay test genuinely checks one decoder instead of comparing two.

**We get more than the vendor sold.** `index` and `parent_index` give same-slot
ordering and the CPI tree — the inputs to coordination analysis — and the
transaction join gives realised amounts, failed transactions and the funding
graph. The vendor's pre-computed aggregates were the parts that could not be used
as features anyway.

**It found instructions we were missing.** The same query surfaced six
discriminators absent from `radar-decode`; three resolved to `extend_account`,
`create` (the pre-V2 launch path) and `migrate_v2`, and `e445a52e51cb9a1d` is
Anchor's event-CPI marker, which carries emitted trade details. That last one is
worth real money later.

**No runtime dependency.** CryptoHouse is a free public service with no SLA and
no support commitment. It is used for **bulk extraction into our own store,
once** — never on a hot path, never as a live provider lane. After extraction we
own the data and its disappearance costs us nothing.

**Rate limits and courtesy.** Queries over a 1.31-trillion-row table take 20–50s.
Extraction runs in slot-ranged batches at a modest rate. The credentials are
shipped in the public web client, so this is a public read endpoint, but that is
an implicit invitation rather than an explicit one: keep the load light and stop
if asked.

**Validation is still required, and for the same reason it would have been for a
paid archive.** Before any of it informs a signal, sample rows are checked
against `getBlock` at the same slot. A third party's reconstruction is still a
reconstruction. That harness is needed to validate our own decoder regardless, so
it costs nothing extra.

## Alternatives considered

**Buy the NoLimitNodes archive.** Rejected: $200/month for data available free,
in a shape that mixes labels with forward-looking aggregates and introduces a
second decoder.

**Record forward only.** Rejected: weeks before Phase 2 can speak, and no regime
diversity ever. Cheap to avoid now that history is free.

**Reconstruct from RPC.** Rejected on arithmetic: $38,900 and 267 TB for six
months.

**Dune.** Community-curated Spellbook SQL, good for one-off charts, capped CSV
export. Not a bulk extraction path.

**Deep history (12+ months).** Deliberately not the default. pump.fun now runs
`CreateV2`, `BondingCurveV3`, cashback and per-user volume accumulators — a
launch from a year ago used different instructions under different economics, so
old data is less transferable than its row count suggests. Start with 3–6 months
inside the current program era and extend only if a signal needs it.
