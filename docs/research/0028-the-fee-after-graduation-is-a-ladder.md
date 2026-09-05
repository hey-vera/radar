<!-- SPDX-License-Identifier: Apache-2.0 -->
# 0028 — The fee after graduation is a ladder, and the chain agrees with the help centre

**Date:** 2026-09-05
**Chain:** the fee program's PumpSwap schedule and PumpSwap's global config, read
at slots 444,505,805 and 444,505,829; the pump.fun schedule re-read at
444,505,437; six live PumpSwap swaps at slots 444,505,736 and 444,506,543–544
**Capture:** [`pumpswap_fees.json`](../../crates/radar-pumpfun/tests/fixtures/pumpswap_fees.json),
asserted by [`the_fee_after_graduation_is_a_ladder.rs`](../../crates/radar-pumpfun/tests/the_fee_after_graduation_is_a_ladder.rs)
**Status:** measured. Design 0009 §1 said the fee after graduation was
unmeasured and could be wrong by three times in either direction. It is now
read off the account that sets it: **30 bps to the creator below 420 SOL of
market cap, 95 bps from 420 to 1,470 SOL, then down a 23-row ladder to 5 bps
above 98,240 SOL.** Four of six live swaps paid exactly the row their pool's
market cap selects. Two paid a row further down the ladder than their cap
selects, and that is the open question at the end.

## Why this was looked at

[ADR 0013](../adr/0013-a-community-token-exists-and-radar-holds-none-of-it.md)
and the pool page say the prize is 30 bps of volume. [Research 0023](0023-the-fee-is-a-schedule-and-the-published-interface-is-incomplete.md)
measured that on the bonding curve. Nothing had measured it after graduation,
and the venue's help centre described a schedule keyed to market cap that would
make the prize three times larger per unit of volume for a coin that graduates
and keeps going. [Design 0009](../design/0009-three-loops-and-no-formula.md)
§1 put the measurement first (M6) and [plan 0006](../plans/0006-design-0009-to-done.md)
made it item 3: capture a swap, see whether it passes the fee program, re-read
the schedule, and correct the page only if the chain disagrees with 30 bps.

## Finding 1 — the fee program keeps one schedule per program, and PumpSwap's has 25 rows

The fee program (`pfeeUxB6…`) derives a `fee_config` account per program it
serves: seeds `["fee_config", program_id]`. 0023 read the one seeded with the
pump.fun program and found a single tier. Seeding with the PumpSwap program
(`pAMMBay6…`) gives `5PHirr8joyTMp9JMm6nW7hNDVyEYdkzDqazxPD7RaTjx`, and that
account parses with [`fees.rs`](../../crates/radar-pumpfun/src/fees.rs)
unchanged — same discriminator, same layout, 1,069 bytes used of 4,073.

| row | market cap from (SOL) | lp | protocol | creator | total |
|---|---|---|---|---|---|
| 0 | 0 | 2 | 93 | **30** | 125 |
| 1 | 420 | 20 | 5 | **95** | 120 |
| 2 | 1,470 | 20 | 5 | 90 | 115 |
| 3 | 2,460 | 20 | 5 | 85 | 110 |
| 4 | 3,440 | 20 | 5 | 80 | 105 |
| 5 | 4,420 | 20 | 5 | 75 | 100 |
| 6 | 9,820 | 20 | 5 | 70 | 95 |
| 7–13 | 14,740 … 44,210 | 20 | 5 | 65 … 35, five a row | 90 … 60 |
| 14 | 49,120 | 20 | 5 | 30 | 55 |
| 15–23 | 54,030 … 93,330 | 20 | 5 | 28 … 8 | 53 … 33 |
| 24 | 98,240 | 20 | 5 | **5** | 30 |

Every row is in the fixture and the test walks all 25: thresholds ascend,
the creator's share descends after the first step, liquidity providers get 20
bps on every row but the first, the protocol 5.

The pump.fun schedule, re-read the same hour, is what 0023 found: one row,
0 / 95 / 30, flat 0 / 95 / 30. **Nothing on the curve changed.**

This is the help centre's ladder, to the row. The one difference is the top
threshold: the venue writes "above 98,000 SOL" and the chain says 98,240.
Design 0009 §1 called the article a claim awaiting a capture; the capture
agrees, which is LEARNINGS 25 going the other way for once and worth
recording as such.

## Finding 2 — live swaps pass the fee program and pay the rows

Two swaps at slot 444,505,736, read with `getTransaction` and the fee
transfers inside the PumpSwap instruction counted. Both pass the fee program
and `5PHirr…` in their account lists (positions 21–22 of 26 on the buy, 19–20
of 24 on the sell) and CPI into the fee program's `GetFees` before moving
anything.

| swap | signature | quote (lamports) | lp | protocol | creator | creator bps |
|---|---|---|---|---|---|---|
| buy, pool `A6Zpvj47…pump` | `5HxVtAB7…eTxZ8WG` | 1,903,467 in | 3,807 (stays in the pool) | 952, as 476 + 476 to two recipients | 952 | **5** |
| sell, pool `DUHuzn4j…pump` | `27Pcbk7h…59EdMhu` | 311,088,811 out | 622,178 | 155,545, as 77,772 + 77,773 | 311,089 | **10** |

The arithmetic closes to the lamport: on the sell the user received
309,999,999, which is the quote out less the four fees. The protocol's 5 bps
is paid in two equal halves to two recipients, the second of which is a
`buyback-vault` PDA under the fee program — the account 0023 said had no known
index rule; it has a purpose now.

The swap events carry the row's creator basis points as a field, so the six
pools below are read from the program's own statement of what it charged,
not inferred from transfers. Market cap is the pool's pre-trade quote reserve
times the mint's supply over its base reserve, all three read off the chain.

| pool (base mint) | market cap (SOL) | row the cap selects | creator bps paid | agree |
|---|---|---|---|---|
| `GB91ML7Z…` | 119.6 | 30 | 30 | yes |
| `DJ9Nx77b…` | 972 | 95 | 95 | yes |
| `NSnFLVVi…` | 96,632 | 8 | 8 | yes |
| `A6Zpvj47…pump` | 1,427,780 | 5 | 5 | yes |
| `4XNpog6q…` | 1,818 | 90 | **75** | no — the 4,420 row |
| `DUHuzn4j…pump` | 87,524 | 13 | **10** | no — the 88,400 row |

Four agree. Two paid a row that a *higher* market cap selects — 4,420 SOL
against a current 1,818; 88,400 against 87,524 — and in both the pool is
below a threshold it could plausibly have crossed and fallen back from. That
shape fits "the ladder is climbed and not descended", a high-water mark, and
it fits nothing else tried here: post-trade reserves make the cap lower, not
higher; the mint supplies were fetched, not assumed; the rounding is exact.
It is a hypothesis with two data points and it is left as one.

## Finding 3 — three accounts, three answers, again

0023 found the pump.fun global account and the fee schedule disagreeing.
PumpSwap has the same disease with a third voice:

| account | says the creator gets |
|---|---|
| PumpSwap global config `ADyA8hde…`, offset 313 | 5 bps |
| the schedule's flat entry | 0 bps (lp 25, protocol 5) |
| the schedule's rows | 5 to 95 bps by market cap |

The rows are what swaps pay. The other two are asserted to still disagree,
so the day they are reconciled something fails and somebody reads it.

## What this changes

**The pool page and the prize arithmetic.** "30 bps of volume" is right on
the curve and right after graduation while the market cap stays under 420
SOL. On the venue's published initial virtual reserves — 30 SOL against
1,073,000,000 tokens with 793,100,000 for sale, a claim this repository has
not captured — a curve completes at about 411 SOL of market cap, so a fresh
graduate sits nine SOL below the first step. A coin that goes on to 420 SOL
pays the prize **95 bps** of its volume, more than three times the curve's
rate, and the rate then steps down as the coin grows. The page now says the
fee is 30 bps on the curve and a published ladder after it; the worked
example at $10,000 a week stays at $30 because it is a curve-stage figure
and says so.

**Design 0009 §1 and §10.** The paragraph that said this was unmeasured now
points here; weakness 3 is closed and replaced by the open question below.

**Nothing in `fees.rs`.** The parser read the second schedule without
change, and [`fees_at_market_cap`](../../crates/radar-pumpfun/src/fees.rs)
is the rule four of six pools followed. The two that did not are not a reason
to change the function before the rule is known; a guess coded in would be
LEARNINGS 9's shape.

## What this does not establish

- **How the row is chosen when the cap has fallen.** Two swaps paid a lower
  row than the current cap selects. The pool accounts were read for a stored
  high-water mark and none was recognised in the 58 bytes after
  `coin_creator`; the field there is not identified.
- **What the venue does above 98,240 SOL.** One pool, one row. The ladder
  ends and the test pins that it ends at 5.
- **The prize in dollars after graduation.** It depends on which row a coin
  sits in and for how long, which is a distribution nobody has measured
  because no coin of ours exists.
- **Any swap Radar signs.** As 0023: nothing was sent, everything was read.

## What is still open

The tier lookup, above. The cheap next measurement is one pool watched across
a threshold in both directions: a buy that takes it over 420 SOL, a sell that
brings it back, and the creator basis points on each event. Sixty signatures
at one slot found six pools, so a few hundred over a day would find a pool
at a step. Not urgent: it moves the prize between two published rows, and
both are on the page.

## Reproducing

Read-only, against the public mainnet endpoint, no key. Every figure above
came from one of these calls; the account captures are in the fixture as hex.

```bash
# The two accounts. Addresses derived offline: find_program_address(["fee_config", pAMM], pfee)
# and (["global_config"], pAMM); pda.rs asserts the same derivation for the curve's.
curl -s https://api.mainnet-beta.solana.com -H 'content-type: application/json' -d \
  '{"jsonrpc":"2.0","id":1,"method":"getAccountInfo","params":["5PHirr8joyTMp9JMm6nW7hNDVyEYdkzDqazxPD7RaTjx",{"encoding":"base64"}]}'
curl -s https://api.mainnet-beta.solana.com -H 'content-type: application/json' -d \
  '{"jsonrpc":"2.0","id":1,"method":"getAccountInfo","params":["ADyA8hdefvWN2dbGGWFotbzWxrAvLW83WG6QCVXvJKqw",{"encoding":"base64"}]}'
# The two swaps in finding 2.
curl -s https://api.mainnet-beta.solana.com -H 'content-type: application/json' -d \
  '{"jsonrpc":"2.0","id":1,"method":"getTransaction","params":["5HxVtAB7MnwhUajo3sGs1Raw413FkzMpC4tUzDQNaAbChhwpBAVJPzH5bgYar42Eo8nuxbBZLaeQRBEL6eTxZ8WG",{"encoding":"jsonParsed","maxSupportedTransactionVersion":0}]}'
curl -s https://api.mainnet-beta.solana.com -H 'content-type: application/json' -d \
  '{"jsonrpc":"2.0","id":1,"method":"getTransaction","params":["27Pcbk7hF6XDXZPWU2Fren6KRMEUMsVJGmMvJ51Evpzm8YP1FAj6REx8eTtCzcowCfv2mhKvCjnJn9QQQ59EdMhu",{"encoding":"jsonParsed","maxSupportedTransactionVersion":0}]}'
```

The other four pools came from `getSignaturesForAddress` on the fee program
with a limit of 60 at slots 444,506,543–544, keeping the transactions whose
top-level instruction is PumpSwap's and whose event discriminator is the buy's
(`67f4521f2cf57777`) or the sell's (`3e2f370aa503dc2a`). In the event, the
pool's base and quote reserves are the `u64`s at offsets 48 and 56 and the
creator basis points is the `u64` at 344; the mint's supply is `getTokenSupply`.
Three further transactions in the sixty were swaps on pools whose base side is
wrapped SOL, paid no creator fee, and are excluded as not the kind of pool a
graduated coin trades on.
