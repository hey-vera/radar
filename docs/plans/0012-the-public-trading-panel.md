<!-- SPDX-License-Identifier: Apache-2.0 -->
# Plan 0012 — The free public trading panel

**Status:** planned, not started. No implementation has run. Every acceptance
case below is a requirement for a future session, **not the name of a test that
exists**.
**Date:** 2026-09-11.
**Branch:** none yet; each step lands on `main` as its own pull request.
**Inspected base:** `cbd5029` (`Say who runs this, what it collects, and how to
reach him (#222)`), read on 2026-09-11. Everything asserted about the tree came
from reading it at that commit. Everything asserted about CryptoHouse came from
queries run against it on 2026-09-11 and is quoted with its numbers. Everything
estimated is labelled.
**Planned for:** Josh's implementation handoff.
**Decided by:** the owner, 2026-09-11, recorded not argued — Radar is a public
product, market data is free to a stranger, each wallet's own history and signals
are free but private to that wallet, Radar's own decision record stays the
owner's, and only the 24/7 autonomous trader and its chat box are ever paid.

## Objective

`radar.heyvera.org` lands a stranger in a working trading panel with no login.
They pick a coin, move a chart, and read a tape of real trades that is seconds
behind the chain. When they connect a Solana wallet they get a second, private
half of the same screen — *their* trades, *their* positions, *their* watchlist —
and **no wallet can ever see another wallet's anything**. They choose between
approving each buy by hand and signing once for a session.

This plan builds the foundation. It does not build bundle detection, it does not
build the Radar intelligence signals, and it does not build trade execution.

## The four tiers

Every decision below answers "which tier is this row in", so the tiers come first.

| Tier | Who sees it | Needs a wallet | Exists today |
|---|---|---|---|
| **1. Public market data** — coins, launches, graduations, prices, candles, trade tape, holders | everyone | no | partly; see the survey |
| **2. Per-wallet private data** — that visitor's own trades, positions, watchlist, and later their own signals and bot decisions | only that wallet | yes | **no, in any form** |
| **3. Radar's own global decision record** — the `decisions` table, the funnel, the scoreboard, the evidence pages | the owner | operator | yes, and **currently one configuration change away from leaking** |
| **4. The 24/7 autonomous trader and its chat box** | paying customers, later | yes | no; [plan 0011](0011-private-autonomous-trader.md)'s business |

Tier 3 is the edge, not the giveaway. Nothing here builds a public page over it,
and P0 exists so that opening the product does not hand it away by accident.

## Not in scope

- **Trade execution.** No `Proposal`, no `Authorization`, no signer call, no
  route that constructs a transaction. AGENTS §4 rule 1 and
  [ADR 0005](../adr/0005-customers-keep-custody-and-grant-radar-a-bounded-signer.md)
  are untouched; a connected wallet stays authentication, never authority.
- **`Policy::CLOSED`.** It stays shut.
- **Plan 0011.** Not modified, not folded in, not scheduled here.
- **Billing.** No price, no subscription, no credit meter. P7 draws the boundary
  the paid product will sit behind and stops.
- **Bundle detection and the intelligence signals.** Deferred by the owner, free
  when they land. Tier 2 is where they will go.
- **Buying data.** No key acquired, no vendor called, no plan subscribed to. P8
  is a contingency, not a purchase order.

---

## What was measured, before any of it was planned

Read at `cbd5029` on 2026-09-11 unless another date is given. CryptoHouse
figures are from queries run that day; the endpoint is live and they decay.

### Opening the product to any wallet exposes tier 3, today

The most important thing found, and it is not hypothetical.

[`Admission`](../../crates/radar-serve/src/admission.rs) is the gate: unset means
`Closed`, `allowlist:…` admits named identities, `open` admits anyone whose token
verifies. The owner has authorised moving to `open`. The moment that happens,
every route classified `Audience::Customer` in
[`access`](../../crates/radar-serve/src/access.rs) is readable by **any stranger
with a wallet** — and that list is `/v1/funnel`, `/v1/scoreboard`,
`/v1/decisions`, `/v1/evidence/…`, `/v1/tokens/…`, `/v1/customer/wallet`,
`/v1/customer/events`, `/v1/chat`.

Four are tier 3. `/v1/decisions` *is* the 4.8 MB decision record.
`/v1/tokens/{mint}` is worse, because it is **mixed**: `TokenEvidence` in
[`api`](../../crates/radar-serve/src/api.rs) returns `decisions` (tier 3) and
`measurements` (tier 1) in one body, so no classification of the whole route is
correct. And the client paths `/decisions` and `/evidence` are already
`Audience::Public` for the shell — the HTML is public and only the data behind it
is not, which is the arrangement that makes this easy to miss.

**So `open` is not a configuration change. It is a reclassification, and the
reclassification comes first.** That is P0, and nothing else may merge before it.

### CryptoHouse is a live feed, three seconds behind the chain

Measured 2026-09-11 against `https://crypto-clickhouse.clickhouse.com/`, user
`crypto`, no password, no account, no key — the endpoint
[`cryptohouse`](../../crates/radar-backfill/src/cryptohouse.rs) already uses and
that [ADR 0002](../adr/0002-historical-data-comes-from-cryptohouse-not-a-vendor-archive.md)
already chose. The repository made this choice; **how fresh it is was recorded
nowhere**, and the whole shape of this plan turned on finding out.

| Measurement | Result |
|---|---|
| freshness | `max(block_timestamp)` = `2026-09-11 17:16:16` against `now()` = `17:16:19` — **3 seconds** |
| that query | 7,195,987 rows scanned in **2.66 s**, over a five-minute bound |
| pump.fun volume | **11,550 instruction rows across 8,143 transactions in one minute** |
| that query | 1,684,063 rows, 242 MB, **8.33 s** |

**Radar's own five-minute delay is self-imposed.** `FOLLOW_LAG_SECONDS` = 300 in
[`radar-backfill`](../../crates/radar-backfill/src/main.rs) is a Radar constant,
not a property of the source. A near-live tape is reachable on infrastructure the
repository already depends on, for nothing, and the free public Solana RPC has
nothing to do with it.

### What that endpoint will and will not permit

Three limits, and **one of them is recorded nowhere in the repository**.

1. **A sixty-second query timeout.** Documented in
   [`cryptohouse`](../../crates/radar-backfill/src/cryptohouse.rs).
2. **A thousand-row result cap.** Same place. `readonly=1`, unraisable.
3. **`max_rows_to_read` = 10 billion.** Found by hitting it. An unbounded
   `max(block_timestamp)` over `solana.instructions` (1.34 trillion rows) is
   refused outright with `TOO_MANY_ROWS`. So is folding one mint's whole transfer
   history — 16.42 billion rows — because `solana.token_transfers` is ordered by
   time and **a `mint` filter alone scans the entire table**. Every query this
   product makes must carry a time bound. This is new and it belongs in the code.

Against limit 2, one minute of pump.fun instructions is 11.5× over, so **a live
tape window is measured in seconds, not minutes.** `MIN_WINDOW_SECONDS` = 4 and
`FOLLOW_MIN_STEP_SECONDS` = 15 are already in the right neighbourhood. The
binding constraint is more likely limit 1 than limit 2: the real extraction query
in [`extract`](../../crates/radar-backfill/src/extract.rs) is three CTEs, two of
which scan `solana.token_transfers` and `solana.transactions` across the window
with no program filter, and a bare *count* over the program alone already spent
8.33 s of the 60.

**`Scope::Trades`'s discouraging doc comment is about the wrong problem.** It
says trades are "viable only over narrow windows, for investigating specific
periods rather than backfilling history", and its own arithmetic explains why:
over a million trades a day is "a thousand rows per twenty seconds of chain and
some 780,000 queries" for six months. That is a verdict on **backfilling six
months**. A live tape needs a handful of queries a minute, and narrow windows are
not its obstacle — they are its access pattern. The caveat does not constrain
this use and the comment should say so when the work lands.

**Visitors never touch CryptoHouse.** One follower process does; every visitor
reads Radar's own store. Concurrency therefore does not multiply upstream load at
all, and the rate-limit question is about **one sustained poller**, not about
traffic. That is the single most important thing about the risk in decision 3.

### Holders are derivable, and the derivation is not what a holder list usually means

`solana.token_transfers` columns, read 2026-09-11: `authority`, `block_hash`,
`block_slot`, `block_timestamp`, `decimals`, `destination`, `fee`,
`fee_decimals`, `memo`, `mint`, `mint_authority`, `source`, `transfer_type`,
`tx_signature`, `value`, `id`.

`transfer_type` over one two-minute sample: `TransferChecked` 530,702,
`Transfer` 188,433, `SplTransfer` 87,944, `Burn` 2,693, `MintTo` 2,332,
`BurnChecked` 76, `TransferCheckedWithFee` 61, `MintToChecked` 8. Mints and burns
are present, so a fold can conserve supply rather than drift.

Three things that fold **cannot** claim, each of which the panel must say:

1. **It is a fold of observed transfers, not a read of current state.** A missed
   row is a wrong balance, silently. An account-state read is a different fact
   and the honest label for each is different.
2. **`source` and `destination` are token accounts, not wallet owners.** A sample
   destination read today was `KFSUyzz5qJCBGHAnKAXU7DgdVNzMdJACkgiVxh4Vo7v`.
   Reporting token accounts as "holders" overstates the holder count wherever one
   owner holds several accounts. Mapping to owners needs `solana.accounts` and is
   a second query, not a rename.
3. **It works for young coins and not for old ones.** The fold must run from the
   mint's first transfer, and limit 3 refuses an unbounded mint filter. A coin
   launched today is a narrow bound; a coin from last year is not expressible on
   this endpoint. Since the panel's subject is new launches, that is a fit — but
   it is a *capability boundary* and the seam must report it rather than
   returning a short list.

### Four things stand between the live lane and a trade tape, all in the tree

1. `Scope::Trades` exists in `extract` and **nothing is scheduled to run it.**
   The `trades` table is empty.
2. Its discriminators come from `pumpfun::KNOWN` only. The decoder handles
   PumpSwap ([`radar-decode`](../../crates/radar-decode/src/pumpswap.rs)) but the
   query does not ask for it, so **a graduated coin's tape is empty even after
   the tape is on.** Free to fix; named so it is not later found as a bug.
3. The follow cursor is **one file per store**:
   [`cursor`](../../crates/radar-store/src/cursor.rs) has a single
   `CURSOR_FILE = ".follow-cursor"`, and
   [`brief`](../../crates/radar-cli/src/brief.rs) reads its age as the answer to
   *is ingestion alive*. Two followers against one store would take turns
   advancing one cursor and each would silently skip the other's windows.
4. [`Reader::read`](../../crates/radar-store/src/reader.rs) reads a whole table
   into memory; `read_range` bounds the disk read by slot and is the only
   function a tape may use.

### The free public Solana RPC still binds live account reads

Unchanged, and kept from the original brief.
[`radar-onchain`](../../crates/radar-onchain/src/rpc.rs) falls back to
`api.mainnet-beta.solana.com` with no key configured, which Solana's own
documentation says is not intended for production and caps at 100 requests per
10 s and 100 MB per 30 s. **That still constrains anything needing an instant
on-chain read rather than a recent event** — pool reserves, quotes, current token
balances. Events come from CryptoHouse; account state comes from RPC; only the
second is rate-bound in a way this product feels.

### Identity exists in the server and nothing downstream uses it

[`session`](../../crates/radar-customer/src/session.rs) issues a bearer token
carrying a 32-byte wallet address and an expiry, authenticated with a blake3
keyed tag over a fixed-width 40-byte payload, `LIFETIME_SECONDS` = 43,200, **not
refreshed on use**. The guard in
[`radar-serve`](../../crates/radar-serve/src/lib.rs) verifies it and inserts a
`customer::Customer` into the request extensions.

**One handler consumes that identity, and it is the precedent this plan
generalises.** `customer_wallet` takes the DID from the extension and its comment
says why in one sentence: *a DID taken from a path or a query would let any
caller read any customer's wallet, which is the whole of the authorisation on
this route.* Every other handler ignores identity, because every other handler
serves one global store.

**The store has no owner column anywhere.** `decisions` and `positions` are one
global table each, keyed by mint. `Trade.trader` is `Option<Address>` in
[`event`](../../crates/radar-store/src/event.rs) and is **`None` for every
backfilled row** — the CryptoHouse query does not fetch account keys, which
`a_backfilled_trade_has_no_trader_rather_than_a_placeholder_one` in `extract`
pins. So **per-wallet trade history cannot be read out of the store**, now or
after P2. That decides the storage shape in decision 2.

### The rest of the survey

**Store.** `/home/guardian/radar/data/store`, 539 MB: `launches` live,
`graduations` live, `outcomes` hourly by cron, `decisions` hourly by cron,
`trades` **empty**, `positions` **empty**. Outcome measurements are taken at three
fixed ages — `CHECKPOINTS` in
[`checkpoints`](../../crates/radar-backfill/src/checkpoints.rs) is roughly 1h, 6h
and 24h.

**Frontend.** React 19, wouter, Tailwind 4, Vite, vitest; 6,646 lines across 37
files in `web/src`; `mx-auto max-w-6xl` in [`App`](../../web/src/App.tsx) — a
narrow centered document, not a terminal. Two runtime dependencies. The build is
embedded into `radar-serve` by `rust-embed`. [`siws`](../../web/src/siws.ts)
states a **120 kB gzipped entry-bundle budget**; measured on the checked-in build
artefact on 2026-09-11 the shipped JS is 258,092 bytes raw and **78,194
gzipped**, plus 4,420 gzipped of CSS. **Nothing enforces that budget.**

**Lightweight Charts, checked 2026-09-11** against `registry.npmjs.org` and the
bundlephobia size API: version **5.2.1**, licence **Apache-2.0** with a NOTICE
attribution requirement that TradingView be named and `tradingview.com` linked,
**194,247 bytes minified / 61,584 gzipped**, one dependency (`fancy-canvas`,
~18.9 kB). 78 + 62 is 140, so the entry chunk cannot hold it; a lazy route chunk
can.

**Delivery.** `/v1/events` and `/v1/customer/events` are both SSE over one
`changes` helper that runs **one `unfold` per connection**, each polling the store
on its own timer. Store reads scale linearly with concurrent visitors.

---

## The eight decisions this plan makes

### 1. Where wallet identity enters, and how an unscoped read becomes impossible

The load-bearing decision. It is a data-isolation boundary and gets the weight
this repository gives security boundaries.

**Identity enters in exactly one place: the request extension the guard already
sets.** Never a path segment, never a query parameter, never a body, never a
client-controlled header. That is not new policy — it is what `customer_wallet`
already does and why. What is new is that it stops being one handler's discipline
and becomes a type.

**An unscoped per-wallet read must be impossible to express, not merely
discouraged.** AGENTS §5 ranks the enforcement levels and the top one is *make it
impossible — a type, a private field, an absent API*. So:

- A `Tenant` wraps a verified `Address`. Its only constructor takes the guard's
  `Customer` extension. No public field, no `from_str`, no `Deserialize`. A
  handler cannot manufacture one from anything a caller sent.
- A `TenantStore` is constructed **only** from a `Tenant`. It exposes the
  per-wallet reads and nothing else — no `Reader`, no root path, and **no method
  taking an address**, so "read wallet B's rows" is not a sentence this API can
  say.
- Per-wallet handlers take `TenantStore` and never the shared store. A route that
  forgets to scope does not compile, because there is nothing for it to read.

**How it fails closed.** A per-wallet route reached without a verified session has
no `Customer` extension, so no `Tenant`, so no `TenantStore`, so a 403 before any
read. Same shape as `audience_of`'s fallback to the most sensitive audience: the
missing case is the closed case.

**What this resists, and what it does not.** Stated exactly, because an earlier
document here said "absolute" and was read as more than it was.

- **Resisted:** a caller naming another wallet in any request field; a handler
  author forgetting to scope; an unauthenticated caller reaching a per-wallet
  route; an expired session (`verify` returns `Expired`, the guard never inserts
  the extension); a payload edited to swap the address (`BadTag`).
- **Not resisted:** a stolen bearer token. Anything that can read the page's
  storage can use it — which [`Wallet`](../../web/src/Wallet.tsx) already says of
  itself, and which is as true of the wallet extension beside it. Expiry is the
  only mitigation, twelve hours, not refreshed on use.
- **Not resisted, and worth naming:** the session tag carries **no issuer and no
  audience**. Two instances sharing a `customer_salt` would accept each other's
  tokens. Theoretical with one instance; real the first time a staging box is
  given production's secret. The mitigation is operational and belongs in the
  runbook, not in a type.

### 2. What per-wallet data is stored, and what is derived

**Almost nothing is stored, and that is
[ADR 0006](../adr/0006-radar-records-only-what-it-cannot-recover.md) rather than
laziness.** Radar records only what it cannot recover.

- **Derived, never stored:** a wallet's trade history and current positions. Both
  are on the chain, which is more authoritative than a Radar copy, and a mirror
  that diverges from an authority is wrong by construction. History comes from
  CryptoHouse filtered on the wallet **under a time bound** (limit 3 forbids an
  unbounded one); current balances come from RPC, which is where the 100 req/10 s
  cap bites. Cached at the watermark like every other read — AGENTS §4 rule 3: a
  cached value is a read whose watermark is the one it was **stored** at.

  This is also forced: `Trade.trader` is `None` on every backfilled row, so the
  store cannot answer "what did this wallet trade" and will not after P2.

- **Stored, because it cannot be recovered:** the watchlist, the authorisation
  mode the wallet chose, and — later, not here — that wallet's own signals and
  its bot's decisions.

**The storage shape is a per-wallet directory whose path is derived from the
verified address**, under a `customers/` root beside the existing tables, reusing
`radar-store`'s writer and reader unchanged. Chosen on one criterion — *what
happens when a developer forgets*:

| Shape | Forgetting to scope yields |
|---|---|
| an `owner` column on the shared tables | **every other tenant's rows** |
| a separate database | a connection with no tenant filter; same failure, one layer down |
| **a per-wallet directory** | **a path that does not exist** — an empty read |

The third fails closed by construction and is the only one that does. It also
keeps tier 2 physically apart from tier 3, so no query can join a customer's
watchlist to Radar's decision record.

The path is built from `Address`'s canonical base58 rendering of 32 verified
bytes, so it cannot contain a separator or a traversal sequence — and the
construction is still one function with a test feeding it hostile strings,
because "it cannot happen" is the sentence that precedes it happening.

Cost, stated: no cross-tenant query is possible. That is the point, and any
future "how many wallets watch this coin" becomes deliberate work rather than a
`GROUP BY`.

### 3. The market-data seam, and why it exists

**One trait, `MarketData`, in a new `market` module inside `radar-serve`, next to
the handlers that are its only caller.** Not a new crate: AGENTS §5 is explicit
that a crate nothing depends on is not a design, and this repository has produced
three. The argument for a separate crate is "a future Geyser implementation needs
somewhere to live", and it does not — **an owned Geyser recorder writes into
`radar-store` and the store-backed implementation picks it up with no new
implementation of this trait.** Only a vendor needs a second implementation, and
a vendor client is HTTP in a module.

**The seam is not about affordability. It is about continuity.** CryptoHouse is
free, live and already depended on — and it is a public service with **no
contract, no SLA, and no rate-limit documentation I could find**. Radar is a guest
there, which `cryptohouse`'s own module comment says in as many words. If the
product's entire live path runs through it and it throttles or disappears, the
product stops. The seam exists so a paid firehose can be slotted in **on the day
that happens**, rather than being designed during the outage. Named prices from
the brief, unauthorised and recorded only so the decision is not researched under
pressure: Chainstack Growth $49/mo plus a Yellowstone gRPC add-on $49/mo, or
Shyft $199/mo.

Four methods, each named for the region it feeds:

- `capability()` — what this implementation can answer at all, its lag, its venue
  scope, and for holders **which kind of fact it is returning**. Read by the panel
  before it asks anything.
- `tape(mint, from_slot, limit)`, `candles(mint, interval, from, to)`,
  `holders(mint)`.

Every one returns `Answer<T>`:

```
enum Answer<T> {
    Observed { value: T, source: Source, as_of: Slot, lag_seconds: u32, scope: Scope },
    Nothing  { source: Source, as_of: Slot, scope: Scope },
    Unavailable(Unavailable),
}
```

`Nothing` and `Unavailable` **must never merge**. `Nothing` is a source that
answered and found no rows. `Unavailable` is a source that cannot be asked — *this
coin is too old to fold holders from transfers on this endpoint*. Collapsing them
prints "no holders" over a capability boundary, which is AGENTS §4 rule 9 broken
at the one seam this plan adds. `Unavailable` is the existing `{ fact, why }`
already carried per-fact by
[`Dossier`](../../crates/radar-onchain/src/dossier.rs), reused not reinvented.

| | `StoreMarket` (P1–P4, free) | `VendorMarket` (P8, contingency) | Owned Geyser (contingency) |
|---|---|---|---|
| tape | pump.fun curve, seconds behind | vendor scope | owned, sub-second |
| candles | folded from stored trades | vendor OHLCV | folded from stored trades |
| holders | transfer fold, young coins, token accounts | vendor top holders, owner-level | needs a state lane regardless |
| cost | $0 | unauthorised | unauthorised |

`VendorMarket` is **not constructed when no key is set**, and the seam then
reports `Unavailable`, never an empty list — AGENTS §4 rule 8.

**The whole seam is tier 1.** It takes no identity, has no `Tenant` parameter, and
must never gain one. Market facts belong to nobody.

### 4. The screen

**`/` becomes the trading panel, in two halves.** Left and centre are tier 1 and
render for a stranger with no wallet. The right column is tier 2 and renders an
invitation until a wallet is connected. Tier 3 does not appear on it; `/decisions`
and `/evidence` become operator screens.

| Region | Tier | Source | Free | What it says when its source cannot answer |
|---|---|---|---|---|
| **Left rail** — coin list, newest first | 1 | `launches` | **yes, live** | "Radar has recorded no launches yet" — true only of a fresh instance |
| **Centre, chart** | 1 | `candles` | **yes, after P2** | for a graduated coin: "Radar records pump.fun bonding-curve trades. This coin has graduated and its AMM trades are not recorded yet." |
| **Centre, tape** | 1 | `tape` | **yes, after P2** | as above, and separately "No fills in this window" when the source answered and found none |
| **Right, price path** | 1 | outcome measurements | **yes**, 1h/6h/24h | "Too new to have been measured. First measurement at about one hour old." |
| **Right, holders** | 1 | transfer fold | **yes, after P4** | for an old coin: "Radar folds holders from this coin's transfers since it launched, and this coin is older than that fold can reach." Always captioned with **what kind of fact it is** — see below |
| **Right, your trades** | 2 | chain, scoped | after P3 | no wallet: "Connect a wallet to see your own trades. Radar reads them from the chain; it does not keep a copy." fresh wallet: **"You have no trades in this coin yet"** |
| **Right, your positions** | 2 | RPC, scoped | after P3 | as above |
| **Right, watchlist** | 2 | per-wallet store | after P3 | "Your watchlist is empty" |
| **Header** | 1/2 | [`Wallet`](../../web/src/Wallet.tsx) | **yes** | its five existing states |

**The holder caption is not decoration.** It reads, in substance: *folded from
every transfer of this coin since it launched, counting token accounts rather
than wallets, as of slot N.* Three qualifications, all load-bearing, and a panel
that drops them is claiming a current-state owner list it does not have.

**Three distinctions the copy must hold, and they are not one distinction.**
*Radar cannot see this* is about the instrument. *You have none of this* is about
the visitor. *Nobody traded this* is about the coin. Three sentences, never one,
and a test per region asserting which renders.

The panel is **full-width**. `max-w-6xl` stays on the operator documents.

### 5. Live delivery

**SSE, reusing the existing pattern, carrying only a watermark; the panel
re-fetches when it moves.** No websocket, no per-mint stream. A websocket buys a
client→server channel this needs none of, and costs a dependency and a Cloudflare
proxy concern.

The tick rate now follows the tape rather than a five-minute window, so the screen
has something new every few seconds. That makes the fan-out question real rather
than theoretical: `changes` polls the store **once per connection**. P5 fixes it,
and its rubric is a number.

**Two streams, not one.** The tier 1 stream is identical for every visitor, so it
is computed once and broadcast. A tier 2 stream would have to be per-wallet, and a
broadcast channel carrying per-wallet payloads is the exact shape of a
cross-tenant leak. So **tier 2 does not stream**: the private half refreshes on
the tier 1 tick or when the visitor asks. That removes the failure mode rather
than defending against it.

### 6. The wallet and the two authorisation modes

Both modes are **identity plus a declared scope**. Neither is execution.

- **Approve each buy.** The default. Connect, browse, and when a buy is eventually
  built the wallet pops up for every one.
- **Sign once for a session.** The existing [`siws`](../../web/src/siws.ts) flow.
  What changes: **the mode and the expiry go into the challenge text the server
  composes**, so the customer signs what they agreed to rather than setting a
  client-side preference with nothing behind it. The panel shows the mode and the
  time left, and one control ends it.

**The boundary.** This plan builds the choice, its presence in the signed message,
its expiry, its display, its revocation. It builds no code constructing a Solana
transaction, no route reaching `radar-exec` or `radar-signer`, and no path from a
session token to a signature. A session token authenticates a reader and scopes a
read. Authority over capital comes from the deterministic kernel and the separate
signer — ADR 0005, unchanged.

### 7. Chart tooling

**Adopt `lightweight-charts` 5.2.1, lazily imported into its own chunk.**

Apache-2.0 is this repository's own licence, so the only friction is the NOTICE
attribution, a real obligation landing in the same pull request as a visible
credit and link. The dependency posture in
[`PricePath`](../../web/src/PricePath.tsx) — *every dependency is one more thing
that can be compromised into a process that will eventually hold a signing key* —
gets an answer rather than a waiver: this runs in the visitor's browser tab, not
in the signer's process tree, and the alternative is hand-writing candles,
crosshair, pan, zoom, timeframes, drawings and indicators.

**`PricePath` survives, unchanged.** Its module comment explains why outcome
measurements **cannot** be drawn as candles: `peak_price` and `trough_price` are
folded from launch, so rendering them as a candle's high and low claims every
interval reached those levels. The new chart draws actual fills — a different
source — and folding the two into one series would be exactly that lie.

Indicators are computed in the browser from the candle series. One whose lookback
exceeds the available bars is **absent**, never truncated, never zero-seeded.

### 8. The paid trader's boundary

Only tier 4. **One directional dependency and one path prefix:**

- The paid product is reachable only under `/v1/trader/…` and `/trader`. Neither
  appears in `audience_of`'s public or customer expressions, so both fall through
  to `Audience::Operator` — **closed by default, today, with no code written**.
- The trader may read the market seam and a `TenantStore`. **Neither may know the
  trader exists.**

Checkable mechanically; P7 checks it. Billing is not designed here.

---

## Dependency sequence

**P0 gates everything.** P1 and P2 are the tracer bullet for tier 1. P3 is tier 2
and the highest-risk step. P4–P7 are independent of each other. P8 is a
contingency that runs only if P2's measurement says the free lane is unsafe.

Task ids are `M-D-NNNN`. **Independent** marks a task runnable at the same time as
any other so marked. **Reviewer** marks a task touching a shared contract, a
default, an on-disk format, auth, or a public surface.

### P0 — Make opening the product safe, before opening it

**9-11-0001 — reclassify tier 3 away from `Customer`.** Implementer. Blocks on:
nothing; blocks everything else. Moves `/v1/funnel`, `/v1/scoreboard`,
`/v1/decisions` and `/v1/evidence/…` to `Audience::Operator`, and moves the client
paths `/decisions` and `/evidence` out of the public shell list. Allowed:
`crates/radar-serve/src/access.rs`, `web/src/routes.ts`, their tests, `docs/adr/`.
Verification: `cargo test -p radar-serve`, `just web`, plus an ADR in the same
commit because this amends
[ADR 0018](../adr/0018-the-shell-is-public-and-the-gate-moves-to-the-data.md).
Rubric: with `RADAR_CUSTOMER_ACCESS=open` and a valid wallet session, each of the
four returns 403; the wrong behaviour is re-applied — one route put back — and a
named test fails. **Reviewer**: public surface, and the change that decides
whether the owner's edge leaks. Stop and ask if any of the four has a legitimate
deployed public consumer; that is a different plan.

**9-11-0002 — split the mixed token route.** Implementer. Blocks on 9-11-0001.
A public route returning `measurements` only, an operator route returning
`decisions`. Verification: `just check`, `just web`. Rubric: the public response
body contains no decision, no reason code and no strategy name — asserted by
serialising a fixture **with** decisions present and inspecting the rendered JSON,
not by reading the struct. **Reviewer**: public surface.

**9-11-0003 — open admission.** Implementer. Blocks on 9-11-0001 and 9-11-0002.
`RADAR_CUSTOMER_ACCESS=open`. **Production is not the implementer's to restart**
(AGENTS §7): documented, applied by a human. Verification: a fresh wallet signing
in and reaching tier 1, and the four tier 3 routes refusing it, both quoted.
**Reviewer**: it is the act of going public.

**Acceptance evidence for P0.** One transcript, one wallet session: signs in,
reads a tier 1 route, is refused on each of the four tier 3 routes with the
refusal naming the audience, and is refused again after expiry with a *different*
message.

### P1 — The panel exists, and every region explains itself

**9-11-0004 — the market seam and its store implementation.** Implementer. Blocks
on 9-11-0001. `MarketData`, `Answer`, `Source`, `Scope`, and `StoreMarket` reading
through `Reader::read_range` with a slot bound. Allowed:
`crates/radar-serve/src/`, `crates/radar-serve/tests/`. Forbidden:
`crates/radar-store/`, `crates/radar-exec/`, `crates/radar-signer/`, `web/`.
Verification: `just check`. Rubric: with an empty trades table `tape` returns
`Nothing`, distinguishable in the response body from `Unavailable`; a test
re-applies the merge and a named test fails. Stop and ask if the seam needs more
than four methods to serve decision 4's regions.

**9-11-0005 — the capability route.** Implementer. Blocks on 9-11-0004.
`GET /v1/market/capability`, `Audience::Public`. Rubric: reachable with no session
at all, and returns no decision, no reason and no store count. **Reviewer**: the
first deliberately public data route this repository has.

**9-11-0006 — the panel and its tier 1 empty states.** Implementer. Blocks on
9-11-0005. `/` becomes the panel; the tier 2 column renders its invitation.
Verification: `just web` — and **raise `MIN_WEB_TESTS` in the same commit**,
because `vitest run` exits zero having found no files and the floor is what
catches it. Rubric: `routes.test.ts` passes with the new table, and every tier 1
region has a test asserting its exact sentence when its source cannot answer.

**Acceptance evidence for P1.** A screenshot with **no wallet connected**: a coin
list that moves, a price path where measured, chart and tape regions each stating
in one sentence why they are empty, a right column inviting a connection. No blank
box. No decision, no reason code, no funnel.

### P2 — The live tape, and the rate limit nobody has measured

**9-11-0007 — a follow cursor per scope.** Implementer. Blocks on nothing.
**Independent.** Per-scope cursor file, existing path kept readable as the
lifecycle cursor so no running deployment loses its place, `radar brief` reporting
each. Verification: `just check` plus
`cargo mutants -f crates/radar-store/src/cursor.rs`. Rubric: two followers each
resume at their own position; the regression re-applies the shared-cursor bug and
a named test fails by showing one follower skipping the other's window.
**Reviewer**: on-disk format read by a production monitor.

**9-11-0008 — measure sustained CryptoHouse behaviour.** Researcher. Blocks on
9-11-0007. **The task that decides P2's shape**, and it answers a question nobody
has: what does this endpoint permit under *sustained* polling, not one query.
Runs `radar-backfill --follow --scope trades` against a scratch store for a bounded
period and records: achieved lag; window size actually sustained; narrowing depth
distribution; how often each of the three limits fires; query latency percentiles;
and **any evidence of throttling, degradation, or refusal that a single query does
not show**. Allowed: a scratch store path, `docs/research/`. Forbidden: the
production store, any change under `crates/`. Verification: the numbers, quoted.
Rubric: a stated sustainable window, a stated achievable lag, and a yes/no to
*can one follower hold a live pump.fun trade tape against this endpoint
indefinitely*. **If no**, stop and take it to the owner: the options are a
degraded lag, a narrower venue scope, or P8, and choosing between them is a
spending decision. Do not widen the window and drop rows.

**9-11-0009 — schedule the trades follower and lower the lag.** Implementer.
Blocks on 9-11-0008 answering yes. A systemd unit at the measured window, and
`FOLLOW_LAG_SECONDS` reduced from 300 to what 9-11-0008 sustained — with the
constant's doc comment rewritten to say it is a Radar choice against a
three-second source, since today it reads as though 300 were forced. Also corrects
`Scope::Trades`'s doc comment, which discourages a use it does not actually
constrain. The unit lands in the repository and a human installs it.
Verification: `radar brief` showing both cursors advancing, quoted with
timestamps. Rubric: the trades cursor age stays under the measured target across a
one-hour observation. **Reviewer**: shared production infrastructure, and a
constant three other things read.

**9-11-0010 — the tape and candle routes, and the chart.** Implementer. Blocks on
9-11-0009. `GET /v1/market/tape/{mint}` and `/v1/market/candles/{mint}`, public,
over the seam. The chart swaps to `lightweight-charts`, dynamically imported, with
the NOTICE attribution visible. Adds a CI job asserting the **entry** chunk stays
under 120 kB gzipped. Verification: `just check`, `just web`, the size job.
Rubric: the entry chunk is under budget with the chart present, proved by the
job's output; a candle whose `realised_lamports` or `realised_tokens` is `None` is
**absent from the series**, never priced at zero, with a test re-applying the zero.

**Acceptance evidence for P2.** A recorded trade on the screen within the measured
lag of landing on chain, named by signature and checkable on an explorer. The
sustained-behaviour numbers quoted in full including every limit that fired. A
graduated coin showing the AMM sentence rather than an empty list.

### P3 — Per-wallet data, isolated

The hardest step. It builds decisions 1 and 2.

**9-11-0011 — `Tenant` and `TenantStore`.** Implementer. Blocks on 9-11-0003. The
types from decision 1 with the layout from decision 2, and the watchlist as their
first and only stored table. Verification: `just check` plus `cargo mutants -f` on
both new files. Rubric, a list because each item is a different leak:

1. A `Tenant` cannot be constructed from a string, path segment, query parameter
   or body — demonstrated by there being no such constructor and by a compile-fail
   test.
2. A `TenantStore` has no method taking an address.
3. No `Customer` extension → 403 naming *no session*, not *not found*.
4. An expired session → 403 naming **expiry**, distinguishably from (3).
5. An edited payload → 403 naming a bad tag, not naming which half was wrong.
6. The path builder refuses every non-base58 input, fed a hostile list including
   separators and traversal sequences.
7. Wallet A's watchlist write is invisible to wallet B, proved by writing as A and
   reading as B in one test.

Each verified by **re-applying the wrong behaviour** and watching a named test
fail. **Reviewer**: the data-isolation boundary; the most senior review in the
plan.

**9-11-0012 — the scoped reads.** Implementer. Blocks on 9-11-0011. A wallet's own
trades from CryptoHouse filtered on that wallet **under a time bound**, and
current balances from RPC under a `Budget`; both cached keyed on the tenant **and**
the watermark. Verification: `just check`. Rubric: a cache entry stored under
wallet A's key is unreachable from wallet B's request — re-apply the shared-key
bug and a named test fails; and no entry is served at a watermark later than the
one it was stored at. Stop and ask if the RPC cap makes on-demand balance reads
unusable at the page's load rate; the answer is then a shallower or cached-longer
position view, not a purchase.

**9-11-0013 — the private half of the screen.** Implementer. Blocks on 9-11-0012.
Three regions, three distinct empty sentences. Verification: `just web`,
`MIN_WEB_TESTS` raised. Rubric: a fresh wallet renders "You have no trades in this
coin yet" and a failed request renders something else; a test asserts the two
strings differ, because collapsing them tells a visitor they have no trades when
Radar could not look.

**Acceptance evidence for P3.** Two wallets side by side in one transcript: each
sees its own watchlist and neither sees the other's — by address guess, by id
guess, by unauthenticated request, and by expired session. Four refusals, four
distinct messages.

### P4 — Holders, folded honestly

**9-11-0014 — the transfer fold.** Implementer. Blocks on 9-11-0004.
**Independent.** `StoreMarket::holders` folding `solana.token_transfers` from the
coin's launch under a time bound, including `MintTo`, `Burn` and their checked
variants so supply is conserved. Verification: `just check`. Rubric: the fold's
signed sum reconciles against the mint's supply for a sampled young coin, and a
coin older than the fold's reach returns `Unavailable` naming that reason —
**never a short list**, which is the failure that looks like an answer. Re-apply
the truncation and a named test fails.

**9-11-0015 — say which fact it is.** Implementer. Blocks on 9-11-0014.
The region and its caption. Rubric: the rendered caption names all three
qualifications — folded from transfers since launch, token accounts rather than
wallets, as of a stated slot — asserted on the rendered output, not on a prop.
**Reviewer**: it is a public claim about data, and the class of claim this
repository exists to get right.

### P5 — Live delivery that survives more than one visitor

**9-11-0016 — one store read, many connections.** Implementer. Blocks on nothing.
**Independent.** A single shared ticker broadcast to every subscriber.
Verification: `cargo test -p radar-serve`. Rubric: **store reads per second stay
flat as connections rise from 1 to 50**, measured by a counting fake store, not
asserted in prose; the wrong behaviour is re-applied and the test fails. The
broadcast payload is tier 1 only and a test asserts its type has no address field,
because that is the shape a cross-tenant leak would take here.

**9-11-0017 — the panel re-fetches on tick.** Implementer. Blocks on 9-11-0016 and
9-11-0010. Rubric: a tick with no new fills produces **no visible change and no
invented row**; a tick with new fills appends in slot order without reordering.

### P6 — Chart tools

**9-11-0018 — timeframes and crosshair.** Implementer. Blocks on 9-11-0010.
Rubric: switching timeframe refolds from the same fills rather than asking the
server for a different truth; a timeframe with insufficient history renders the
bars that exist plus a stated gap, never an extrapolated bar.

**9-11-0019 — drawings and indicators.** Implementer. Blocks on 9-11-0018.
Rubric: an indicator whose lookback exceeds the available bars is absent, not
truncated and not zero-seeded; re-apply the zero-seeded version and a named test
fails. Drawings persist per mint **in the browser**, labelled as the visitor's own
— not in the tenant store, because a drawing is not something Radar cannot
recover, it is something Radar never had.

### P7 — The paid trader's boundary, drawn while it is cheap

**9-11-0020 — the seam, checked.** Implementer. Blocks on 9-11-0011.
**Independent.** A `repo-conformance` check that no source file in the `market`
module, the tenant module or `web/src` names the trader's paths, and that
`/v1/trader` is in neither the public nor the customer expression in
`audience_of`. Verification: `cargo test -p repo-conformance`. Rubric: the check
fires when a `/v1/trader` path is added to the customer list and does **not** fire
on any change a reasonable person would make to the panel — demonstrated both
ways, because AGENTS §5 says a check worth having is one whose false-positive rate
was tested for. Stop and ask if it needs a heuristic that guesses.

### P8 — The contingency, unauthorised and not to be started

**9-11-0021 — a vendor implementation, only if P2 said no.** Implementer. Blocks
on 9-11-0008 returning a negative **and on an explicit owner authorisation that
does not exist today.** A second `MarketData` implementation behind configuration,
absent when no key is set. Rubric: with no key the implementation is not
constructed and the seam reports `Unavailable`, never an empty list — tested by
re-applying the fail-open. **Stop and ask before writing a line**: it presumes a
purchase nobody has approved, and it exists in this document so the design is not
done under outage pressure.

---

## What the panel can show, and where the free lane stops

| Panel | Tier | On $0 | Bound by |
|---|---|---|---|
| Coin list | 1 | **full, live** | — |
| Trade tape, pump.fun curve | 1 | **full, seconds behind**, pending 9-11-0008 | CryptoHouse sustained rate, unmeasured |
| Candles | 1 | **full**, from the same fills | as the tape |
| Trade tape, post-graduation AMM | 1 | **empty** | PumpSwap discriminators absent from the query — a code change, not a purchase |
| Price path | 1 | **full**, 1h/6h/24h only | outcome pass frequency; free to increase |
| Holders | 1 | **full for young coins**, token-account level | `max_rows_to_read`; an old coin is not expressible |
| Holders, owner-level | 1 | **not built** | a second join against `solana.accounts` — free, deferred |
| Your trades | 2 | **full**, time-bounded | CryptoHouse |
| Your positions | 2 | **full** | free public RPC, 100 req/10 s |
| Your watchlist | 2 | **full** | — |
| Your own signals and bot decisions | 2 | **empty; not built** | deferred by the owner; the seam is left |

**Nothing on this screen requires spend.** Four rows are free work rather than
money — the AMM tape, owner-level holders, more frequent outcome passes, and the
lag reduction — and naming them stops "we need a vendor" being said about a
problem that is a query.

## Where this plan is weakest

**One free service with no contract carries the whole live product.** CryptoHouse
is three seconds fresh, costs nothing and is already depended on — and it has no
SLA, no published rate limits, and Radar is explicitly a guest. Every tier 1
region except the coin list and the price path stops if it throttles. The
mitigations are the seam in decision 3, the measurement in 9-11-0008 and the
contingency in P8, and **none of them is a substitute for the fact that this is a
single point of failure chosen on price.** The mitigating architecture is that
visitors never touch it: one follower does, so the exposure is one process's
sustained behaviour rather than traffic-dependent.

**9-11-0008 is measuring something a single query cannot show.** Every figure in
this document is from one-shot queries. Throttling, degradation under sustained
load, and any per-user quota would appear only over hours. The measurement is
scheduled first in P2 for that reason, and its negative branch is a spending
decision escalated to the owner rather than a workaround.

**Tier 2 is being designed from zero.** Nothing like it exists in the tree: no
owner column, no tenant, no per-wallet anything. Decisions 1 and 2 are proposals,
not modifications of something working, and the cost of getting it wrong is a
cross-tenant leak — the most expensive class of bug this product can have. The
mitigations are that isolation is a *type* rather than a discipline and that
forgetting yields an empty directory. What it does not have is a second pair of
eyes on a running system; P3's review is the only substitute.

**The bearer token is the floor of the isolation.** Everything in decision 1
assumes the token in the browser belongs to the wallet named in it. A stolen token
defeats all of it, expiry is twelve hours, and the tag has no issuer or audience.
Stated rather than solved, because solving it properly is different work.

**The holder fold is the claim most likely to be misread.** It is a fold of
observed transfers over token accounts, and every instinct — the reader's and the
implementer's — will be to call it a holder list. Two of the three qualifications
are easy to drop in a redesign. 9-11-0015's rubric asserts the caption on rendered
output rather than on a prop for exactly that reason.

**The screen is specified from the tree, not from a visitor.** Nobody has watched
anyone use this. Decision 4's table is a considered guess at what a pumpfun or
Axiom user expects, made by reading those products rather than by measuring a
session. If the first version is wrong it will be wrong in the layout, which is
the cheap half.

**The bundle budget has never been enforced and is being leaned on.** 78 kB is
from a checked-in artefact dated 2026-09-08, not from current `main`. The size job
lands in 9-11-0010 for that reason.

## Open questions for Josh

- **Q1 (2026-09-11): confirm that `/decisions`, `/evidence`, `/v1/funnel` and
  `/v1/scoreboard` become operator-only.** They are `Audience::Customer` today and
  `RADAR_CUSTOMER_ACCESS=open` hands all four to any stranger with a wallet.
  **Recommendation: yes, and before admission opens.** The cost is real: the
  honest-scoreboard work in [`Scoreboard`](../../web/src/Scoreboard.tsx) stops
  being visible to anyone, and it is some of the best public argument this project
  has. A *curated* public evidence page is a different artefact from an open
  `/v1/decisions` and can be built later without reopening the route.
  **Unanswered, and all of P0 turns on it.**
- **Q2 (2026-09-11): how low should `FOLLOW_LAG_SECONDS` go?** The source is three
  seconds fresh; 300 was chosen when nobody had measured that. **Recommendation:
  let 9-11-0008 pick it** — the lag should be whatever the sustained measurement
  supports with margin, not a round number. Note that lowering it changes the
  *launches* follower too, which is a live production behaviour change and wants
  its own observation window. **Unanswered.**
- **Q3 (2026-09-11): PumpSwap discriminators and owner-level holders — in scope
  here?** Both are free code changes and both visibly widen the panel.
  **Recommendation: no for this plan** — land the bonding-curve tape and the
  token-account fold first, and open each as its own item once 9-11-0008 has said
  what the endpoint sustains for the queries already being made. Adding query load
  before measuring the limit is the one move that could cost the free lane.
  **Unanswered.**

## Handback

**Stopped at:** planning only. Nothing built, no branch created, no build or test
run, no production setting touched. The tree was read at `cbd5029`. CryptoHouse
was queried read-only on 2026-09-11 and every figure is quoted with what it cost;
nothing was written anywhere but this file.

**Corrected in this document, plainly, and it is the reason the plan changed
shape twice.** An earlier draft from the same day (a) treated the panel as a single
shared public product and planned a public `/decisions` page, and (b) planned
around empty tape and chart panels as the expected steady state on $0, because it
carried the free-RPC rate limits over to a lane that does not use RPC. Both were
wrong. The record is tier 3 and the reverse of a giveaway; the event lane is
CryptoHouse at three seconds and costs nothing. The first mistake surfaced the P0
exposure. The second was a constraint imported from the wrong source and applied
without checking the one that was actually in use — the tree said `CryptoHouse` at
the top of the backfill crate the whole time.

**Next action:** answer **Q1**, then do P0. 9-11-0001 blocks every other task and
is small. 9-11-0007 (the per-scope cursor) is the only task genuinely independent
of P0 and can start in parallel on its own branch.

**Do not:** open `RADAR_CUSTOMER_ACCESS` before 9-11-0001 and 9-11-0002 have
merged; add query load to CryptoHouse before 9-11-0008 has measured what it
sustains; fold the paid trader into this work; open `Policy::CLOSED`; acquire a
data key; write a line of transaction construction; or modify
[plan 0011](0011-private-autonomous-trader.md) or design 0017. If 9-11-0008
reports the free lane cannot hold the tape, **stop and take it to the owner** —
do not widen the window, do not drop rows, and do not describe a partial capture
as a tape.
