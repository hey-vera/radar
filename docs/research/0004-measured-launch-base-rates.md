<!-- SPDX-License-Identifier: Apache-2.0 -->
# Measured: block economics and launch base rates

Measured 2026-08-22 against Solana mainnet via the free public RPC, slots
440972500–440972965. Reproduce with `scripts/probe/`. Sample is small (45 blocks,
11 launches) and one window on one day — treat the percentages as calibration,
not as findings. The block economics are robust; the base rates need a bigger
sample before anything is built on them.

The point of running this before writing decoders was to find out which
assumptions in the plan were wrong while they were still cheap to change. Three
were.

## Block economics — ADR 0001 confirmed, with a caveat that changes the design

| Measured over 45 blocks | |
|---|---|
| Mean transactions per block | **1,513** |
| Mean block size | **6.88 MiB** |
| pump.fun transactions seen | 4,173 |
| Cost as `getBlock` | **$0.045** |
| Cost as parsed transactions | **$208.65** |
| **Measured ratio** | **4,637×** |

ADR 0001 estimated 100–300×. The measured figure on this window is **4,637×**,
because a busy slot carries far more pump.fun transactions than the 50–200 the
ADR assumed. The decision is confirmed and then some.

**The caveat is bandwidth, and it is not small.** At 6.88 MiB per block and ~2.5
slots per second, polling every slot would pull **~1,451 GiB/day**. The price is
$0.001 a call; the bandwidth is not. Nothing in the plan accounted for this.

**What it changes.** A full block fetch is for *qualified candidates*, never for
discovery:

1. Discover Creates cheaply — a WebSocket `logsSubscribe` on the pump.fun program
   carries log lines, not blocks.
2. Tier-0 filter on the create transaction alone (`getTransaction`, ~20 KiB).
3. Fetch the **full block only for candidates that survive**, for the same-slot
   coordination picture.

At the measured 0.24 launches per slot and a 90% Tier-0 kill rate, that is
roughly 5,000 block fetches a day — about 35 GiB, which a VPS can carry. Without
the filter it is 1.4 TB and the design does not work.

**It also makes clawapis request #3 much more valuable than it looked.** A
program-filtered slot-range endpoint would return perhaps 50–200 KiB instead of
6.88 MiB. That is a ~40× bandwidth reduction on the path that dominates, and it
is cheaper for the vendor to serve than shipping whole blocks we discard 99% of.

## Launch base rates — the coordination heuristics need rethinking

Eleven launches. Small, so these are direction not truth.

| Signal | Rate |
|---|---|
| Dev buy inside the create transaction | **8/11 (73%)** |
| Create transaction pays a Jito tip | **1/11 (9%)** |
| Any other pump.fun buy in the same slot | **10/11 (91%)** |
| A buy within 30 transaction indices | **4/11 (36%)** |

Plan section 6 proposed as Tier-A direct evidence: *"same-slot contiguous
transaction indices plus a transfer to a known Jito tip account."* Against these
numbers that heuristic is in trouble from both ends.

- **Same-slot buys occur in 91% of launches.** A signal that fires on nearly
  every launch discriminates nothing. On its own it is close to useless.
- **Jito tips at launch occur in 9%.** Requiring one would discard the
  overwhelming majority of genuinely coordinated launches.

Combining a signal that fires almost always with one that fires almost never does
not produce a good detector; it produces one that is either saturated or silent
depending on which term dominates.

**What survives.** The dev buy inside the create transaction is real and common
(73%) — it is genuine Tier-A evidence in the sense of being directly observed
rather than inferred, but at that base rate it is *descriptive, not
discriminating*. It says "this is a normal pump.fun launch", not "this is
coordinated".

So the discriminating quantity cannot be the presence of these things. It has to
be their **magnitude and structure**: what fraction of supply the dev buy takes,
how many *distinct* wallets buy early, and — the part no single-slot heuristic
reaches — whether those wallets share a funding ancestor. That pushes the weight
of coordination detection onto the funding graph, where the plan already said the
labelled-exchange-address work was the highest-leverage correctness investment.

**Plan section 6 needs revising accordingly**, and the revision should wait for a
larger sample. Recording the direction now so the detector is not built on the
version that these numbers contradict.

## Instruction mix — decoders must be built from the chain, not from documentation

Distinct pump.fun instructions observed in 45 slots:

```
   246  GetFees                      36  ClaimCashbackV2      14  BuyV2
   214  TransferChecked              29  SellV2               12  BondingCurveV3
   111  Sell                         27  BuyExactSolIn         9  CreateV2
    81  Buy                          26  CloseAccount          9  Initialize
    81  InitializeAccount3           24  CloseUserVolumeAccumulator
   106  InitializeAccount3           21  ClaimCashback         9  UpdateAuthority
    81  GetAccountDataSize           20  InitUserVolumeAccumulator
                                     15  BuyExactQuoteInV2
```

Also seen: `CreateConsume`, `CreatePool`, `InitializeMayhemState`.

The program has moved a long way from what public references describe. There are
now at least four buy variants (`Buy`, `BuyV2`, `BuyExactSolIn`,
`BuyExactQuoteInV2`), a `BondingCurveV3`, per-user volume accumulators, and a
cashback mechanism. A decoder written from 2024-era documentation would handle a
minority of live traffic and would silently mis-classify the rest.

This is precisely the risk ADR 0001 flagged, now measured. Two consequences for
`radar-decode`:

- Build from the live IDL and from captured mainnet fixtures, never from prose.
- **An unrecognised discriminator is recorded as unknown, never guessed**, and a
  rising unknown rate alarms. A decoder that has silently stopped understanding a
  program looks exactly like a program that has gone quiet.

## The bug this probe caught in itself

Probe 2 checked for the dev buy with `"Buy" in instruction_names` and reported
`False` on a launch whose instruction list plainly contained `BuyV2`. Exact-match
on an instruction name that had been versioned produced a confident wrong answer
about the single most load-bearing piece of coordination evidence.

Caught only because the raw instruction list was printed next to the verdict. If
the probe had printed just the verdict, the number would have been wrong and
believed. Recorded in [LEARNINGS.md](../../LEARNINGS.md).

---

## Addendum, same day: the first real store, and a constraint that reshaped the backfill

Thirty minutes of chain extracted from CryptoHouse into Radar's own Parquet store
(`radar-backfill`, 27.7s, 87% yield, 376 events).

**The public endpoint caps a result at 1,000 rows** — `max_result_rows = 1000`,
`max_execution_time = 60`, `readonly = 1`, and a readonly user cannot raise any
of them. Aggregates are unaffected because they return few rows, which is why
every earlier research query worked and only bulk extraction hit the wall.

That decides what can be backfilled, by arithmetic rather than preference:

| | Rate | Chain per 1,000 rows | Queries for 6 months |
|---|---|---|---|
| Launches + graduations | ~24k/day | ~1 hour | ~4,400 — **a few hours** |
| Every trade | >1.2M/day | ~20 seconds | ~780,000 — **not a plan** |

So `Scope::Lifecycle` extracts in full and is the default; raw trades are for
investigating a specific window, and outcome labels will come from per-mint
aggregates — which is the granularity they need anyway, and which fits both the
row cap and the 48 GB free on the VPS.

### What thirty minutes already shows

```
distinct creators: 178  (59 launched more than once)

LAUNCHES  CREATOR
     42  AeZpHiVXZ62G8qY1g84qVrq2z877RoiUDRq4QsTp2RJj
     13  HaA8W4e2Y8cqQiH8QiEqkjfRsjQQqKMA1Q4gBLr9TS88
     10  AZry31LgByHrZ37sdPdH3kPPGM5HzkjhBWcerZxMgX4a
```

One creator launched **42 tokens in half an hour** — a rate of about 2,000 a day
from a single address. A third of creators in the window launched more than once.

Two launches landed in the *same slot* with identical names and symbols and
different mints:

```
440628931  F3zWWerz2PDesSo82b8hp315k2zsZMMon1vajPnvpump  CHIPS  REALLL CHECK DISCORDDDD
440628931  796Fqos48pKyadvLoj1nq7V78TiPrrb7kKDWyEimpump  CHIPS  REALLL CHECK DISCORDDDD
```

Creator launch rate is cheap to compute, needs no paid data, and is already
discriminating — which is what section 6.1 argued creator intelligence would be,
now with numbers behind it. Worth measuring against outcomes before it informs
sizing, like everything else.

**Also learned:** not every pump.fun mint carries the `pump` vanity suffix
(`2kfy88Bh72LDqkKgdMwFg68RGhf5xHkSp4mPMLRgycRq` in this window did not). Anything
filtering on that suffix would silently miss real launches.

**Skips were honest**: 36 with no resolvable mint and 20 with more than one
candidate, refused rather than guessed. An event attributed to the wrong token is
worse than a missing one.

