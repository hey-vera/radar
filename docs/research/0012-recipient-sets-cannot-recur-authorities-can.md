# 0012 — Recipient sets cannot recur; authorities can

**Date:** 2026-08-29
**Status:** measured, on one window. The distribution is reproducible; the band
boundaries are provisional.

## What this was meant to be

The plan's Phase E named the next coordination feature as *"repeat co-occurrence
of the same recipient set across different creators"* — the idea being that one
team running many launches would distribute to the same wallets each time, and
that seeing the same set twice is much stronger evidence than seeing six
recipients once.

**That feature is not computable, and the reason is structural rather than a
matter of effort.**

## Why it is not computable

`destination` in `solana.token_transfers` is a **token account**, and a token
account is a `(owner, mint)` pair. Two different mints cannot share one. So a
"recipient set" is mint-specific by construction and can never recur across
launches, however coordinated those launches are.

Measured rather than reasoned, over ten minutes of transfers:

```
SELECT destination, uniqExact(mint) AS mints, count() AS rows
FROM solana.token_transfers
WHERE block_timestamp >= now() - INTERVAL 10 MINUTE AND destination != ''
GROUP BY destination ORDER BY mints DESC LIMIT 5
```

| destination | mints | rows |
|---|---|---|
| 4pKBRyY2nx479h6Jp1hnxLNwbBGGVQgDTTsYsv8wr9aW | 2 | 4 |
| 5AnVbZDrvDSgAyPbYzGU7qsqA4DhevxCEE4UmMKcVWZq | 2 | 2 |
| BHmtam2KizqScC4CsGiXU2W9DMoBBCQQFPRd9BE5BnDQ | 2 | 2 |

The **maximum** across the whole window is two. There is no tail. Resolving token
accounts to owners would fix this and is a join Radar does not have — which is
exactly what [`0008`](0008-the-launch-block-gives-the-bundle-away.md) already
said when it named the field `recipients` rather than `buyers`, *"keeping the
claim the size of the evidence."* That caution turns out to have been load-bearing.

## What is computable instead

The same table carries `authority`: the wallet that signed for the transfer. It
is a wallet address, not a token account, so it recurs across mints by
construction.

It does, heavily:

```
SELECT authority, uniqExact(mint) AS mints, count() AS transfers
FROM solana.token_transfers
WHERE block_timestamp >= now() - INTERVAL 20 MINUTE AND authority != ''
GROUP BY authority ORDER BY mints DESC LIMIT 3
```

| authority | mints | transfers |
|---|---|---|
| Ej8Yw4ky2DB2gvPEXsi6KFoHScPyDsimLXdwYB9c9uvu | 758 | 758 |
| ARu4n5mFdZogZAravu7CcizaojWnS6oqka37gdLT5SZn | 728 | 18,879 |
| D5YqVMoSxnqeZAKAUUE1Dm3bmjtdxQ5DCF356ozqN9cM | 491 | 13,618 |

## The measurement

Restricted to **launch blocks** — a mint's earliest slot — because that is where
0008's signal lives and because it is the only part of a token's life that is
over before anyone can react to it.

Window: 90 minutes of `solana.token_transfers`, with launches restricted to
mints whose first transfer is at least ten minutes inside the window, so a
long-lived token's earliest *in-window* slot is not mistaken for its launch.

```
launches seen              17,032
distinct authorities        8,707
transfers in launch blocks 25,409
```

Appearances per authority:

| launch blocks | authorities | share of authorities | launch appearances |
|---|---|---|---|
| 1 | 7,793 | 89.5% | 7,793 |
| 2 | 410 | 4.7% | 820 |
| 3–5 | 317 | 3.6% | 1,164 |
| 6–20 | 133 | 1.5% | 1,229 |
| 21–100 | 43 | 0.5% | 1,638 |
| **100+** | **13** | **0.15%** | **7,180** |

## The finding

**Thirteen addresses appear in the launch blocks of 42% of all launches.**

That is the whole reason a naive co-occurrence signal would have been worthless.
Any two randomly chosen launches share an authority with high probability,
because both were touched by the same handful of routers, and a signal that
fires on nearly everything is not a signal — it is a restatement of how the venue
works.

The usable structure is the **middle band**. An authority in 3–100 launch blocks
in ninety minutes is neither a one-off nor infrastructure: 493 addresses, 0.6% of
the population, accounting for 4,031 launch appearances. That is the shape of a
repeat launcher — somebody running a factory.

Note the head is separable by inspection as well as by count. `Ej8Yw4ky…` appears
in 4,428 launch blocks with **one** distinct destination across all of them: a
single sink, which is a fee or treasury account rather than a participant.

## What this does not establish

- **One window.** Ninety minutes on one day. 0008's caveat applies here with more
  force, because that note had three populations and this has one.
- **No outcome link.** This measures *who recurs*, not whether recurrence
  predicts anything about money. The obvious next question — do launches
  containing a mid-band authority graduate, or lose, differently — needs a join
  against Radar's own outcomes and is not attempted here. **Until that is
  measured, this is a description of the venue and not a signal.**
- **`min(block_slot)` is a proxy for the launch slot.** It is correct for tokens
  that launched inside the window, which the ten-minute inset is meant to
  ensure, and wrong for any that did not.
- **The band boundaries are provisional.** 3 and 100 are read off one histogram.
  They are the kind of number [`0008`](0008-the-launch-block-gives-the-bundle-away.md)
  called *"a tool with a default setting"*, and they will move.

## What was built on it

[`radar_graph::prevalence`](../../crates/radar-graph/src/prevalence.rs) — a pure
scorer over an authority's launch-block appearance count, with the head excluded
by a measured threshold rather than by a hand-written denylist.

Nothing refuses on it. It is recorded beside the decision so that the outcome
link above can be measured later, which is the same order
[`0008`](0008-the-launch-block-gives-the-bundle-away.md) and the decisions table
were built in: record first, act only once the record says something.
