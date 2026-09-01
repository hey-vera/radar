<!-- SPDX-License-Identifier: Apache-2.0 -->
# 0023 — The fee is a schedule, and the published interface is incomplete

**Date:** 2026-09-01
**Source:** the pump.fun global and fee-config accounts, both on-chain Anchor
IDLs, and the three mainnet trades captured for
[ADR 0009](../adr/0009-radar-builds-its-own-pump-fun-swaps.md)
**Status:** measured. Two findings, one that **confirms** a number 0022 assumed
and one that **contradicts** the program's own published interface.

## Why this was looked at

[`0022`](0022-capacity-was-a-budget-not-a-ceiling.md) priced the round trip with
a fee component of 250 bps and was explicit that it had not checked it:

> The curve's own fee is not modelled here, and it is charged on top of impact.
> The figures above are therefore optimistic by that fee.

That number carries real weight. It is half of the ~456 bps bar 0022 derives for
*"any strategy has to clear this before a trade is worth making"*, and that bar is
what an AI strategy would be measured against. A bar built on an unchecked
constant is not a bar.

## Finding 1 — the fee is 125 bps a side, and two accounts disagree about it

Read from mainnet on 2026-09-01:

| account | field | value |
|---|---|---|
| `Global` (`4wTV1Ymi…`) | `fee_basis_points` | 95 |
| `Global` | `creator_fee_basis_points` | **5** |
| `FeeConfig` (`8Wf5TiAh…`) | tier 0, protocol | 95 |
| `FeeConfig` | tier 0, creator | **30** |

So the global account says 100 bps and the fee program's schedule says **125**.
Both are live. They agree on the protocol's share and differ only on the
creator's, which is what identifies the global field as the fossil rather than a
different fee.

**The schedule is the binding one.** Every `buy` and `sell` captured from mainnet
passes both the fee config account and the fee program in its account list, and
the fee program exposes a `get_fees(is_pump_pool, market_cap_lamports,
trade_size_lamports, …)` instruction. A program that consults a schedule at trade
time is not reading the constant.

**0022's assumption survives.** 125 bps a side is 250 for a round trip, which is
exactly the figure it used. The arithmetic in that note — including the ~456 bps
bar and the conclusion that optimal size is negative at a measured edge of 0 —
stands, and now stands on a number that was read rather than supplied.

**It is a schedule, not a constant, and that is the part worth carrying
forward.** `FeeConfig` holds a *vector of tiers keyed on market capitalisation*,
in an account whose admin can update it, and it has already moved once. Today
there is exactly one tier with a threshold of zero, so the schedule is flat — but
a cost model that hardcoded 125 would be right until the day it silently was not,
and the direction of that error is the one that makes a trade look cheaper than
it is. [`radar_pumpfun::fees`](../../crates/radar-pumpfun/src/fees.rs) therefore
parses the schedule, and
[`the_fee_is_what_mainnet_charges.rs`](../../crates/radar-pumpfun/tests/the_fee_is_what_mainnet_charges.rs)
asserts the disagreement rather than resolving it: if pump.fun ever reconciles
the two accounts, a test fails and someone reads it.

Incidentally confirmed in the same read: `initial_virtual_sol_reserves` is
exactly **30 SOL**, which is the figure 0022's fresh-curve row was built on.

## Finding 2 — mainnet passes more accounts than the program's own IDL declares

Both programs publish an Anchor IDL on chain. Fetched and decoded, pump.fun's
declares **16 accounts** for `buy`. Every `buy` captured from mainnet carries
**18**. `sell` declares 14 and carries 17.

The IDL closed three questions that a seed search had not:

| index | account | derivation |
|---|---|---|
| 13 | `user_volume_accumulator` | `["user_volume_accumulator", user]` under pump.fun |
| 14 | `fee_config` | `["fee_config", pump.fun program id]` under the fee program |
| 15 | `fee_program` | constant |

Each was verified against the captures rather than taken from the IDL — the
derivation is computed and compared to the captured address, and
`user_volume_accumulator` was confirmed a second way, by reading the account and
finding the transaction's own signer inside it.

The two beyond the declared list are **remaining accounts**, and they behave
differently from each other:

- **The last account is a `BuybackVault`**, `["buyback-vault", index]` under the
  fee program with a one-byte index. Confirmed by derivation against all three
  captures, at indices 6, 4 and 2 — so the index **rotates per trade**. Its
  208-byte layout matches the fee program's declared `BuybackVault` type, and the
  contents read as running totals with a recent timestamp.
- **The other is a function of the mint alone, and does not exist.** Six trades
  of the same token by six different signers pass the identical address, so it is
  keyed on the mint and nothing else. The account has never been created.

**That last one is still unidentified**, and it is recorded that way. Roughly
six hundred derivations were checked against all three captures simultaneously —
every seed string in both IDLs, every plausible variant, every one- and two-key
combination of the mint, curve, creator, user, fee recipient, creator vault and
volume accumulator, under the pump.fun, fee, AMM and associated-token programs,
plus ATA-shaped derivations. Named hypotheses ruled out include `sharing-config`,
`mayhem-state`, the AMM `pool` and `pool-authority`, and every ATA of every owner
in the account list.

## What this changes about ADR 0009

**The precondition it was written to enforce is doing its job.** ADR 0009's first
condition is that the account list comes from mainnet rather than from a
reference. Had the builder been written against the published IDL — the most
authoritative reference available, published by the program itself — it would
have built a 16-account instruction, and mainnet passes 18.

Whether those two are *required* is a separate question from what they are, and
it is one **simulation answers and inspection cannot**. An Anchor program ignores
remaining accounts it does not use; this one CPIs into a fee program that may
require them. So the next step is to build the sixteen declared accounts, simulate,
and let the runtime say which of the two it misses — rather than to keep guessing
seeds, which has now had its fair run.

**This is the second time a published reference has under-described this
program.** `radar-decode`'s discriminator table exists because public references
described three instructions where the live program has twenty-one. The IDL is a
better reference than those were and it is still not the program. Recorded in
[LEARNINGS](../../LEARNINGS.md).

## What this does not establish

- **Nothing has been simulated, built, signed or sent.** These are account reads
  and derivations.
- **The fee schedule was read once.** It has one tier today; a schedule with
  several would make the market-cap argument load-bearing in a way no capture
  here exercises.
- **The fee applies to the curve.** `get_fees` takes an `is_pump_pool` flag, and
  the tier chosen here is the pump-pool one. The flat fees, for other pools, are
  parsed but not used.
- **The buyback index rotation is three observations.** It rotates; the rule it
  rotates by is not known, and a builder cannot currently predict which index a
  given trade should carry.

## Reproducing

The IDL for a program is at `create_with_seed(find_program_address([], program),
"anchor:idl", program)`, zlib-compressed after an 8-byte discriminator, a 32-byte
authority and a 4-byte length. Both account captures are stored as hex in
[`pumpfun_fees.json`](../../crates/radar-pumpfun/tests/fixtures/pumpfun_fees.json)
with their addresses and owners, and the fee assertions run offline against them.
