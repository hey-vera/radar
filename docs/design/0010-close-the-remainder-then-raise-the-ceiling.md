<!-- SPDX-License-Identifier: Apache-2.0 -->
# Design 0010 — Close the remainder, then raise the ceiling

**Date:** 2026-09-05
**Status:** proposal, for Josh to accept, change or reject. Section 3 holds
the decisions that are his, each with a recommendation and the answer this
document assumes if he says nothing. Written in plan mode as
`workstation:~/.claude/plans/you-are-claude-fable-peaceful-sunrise.md` and
moved here in the first commit of
`docs/0010-close-the-remainder-then-raise-the-ceiling`, for the reason
design 0007 gives: a plan outside the repository did not happen. The unit
being executed is [plan 0007](../plans/0007-the-learning-loops-instrument.md).
Section 11 is where this is weakest, and it is the section to read twice.

**Base.** Read at `b0b8e05`, the tip of the stacked PRs #147 to #154, which
had not merged when this was written; Josh chose to plan against the tip
rather than wait. `main` was `a621453` (#145). Where a row below says a thing
exists, it exists on that tip. The box was read the same afternoon over SSH
with read-only commands; section 1 has what it said.

**How to read it.** Section 1 is the honest state, re-verified. Section 2 is
the shape: two parts, one order. Section 3 is every decision that is Josh's.
Section 4 is Part A, ordered, each item with a gate. Section 5 is the list of
things only Josh can do. Sections 6 to 8 are Part B: Radar smarter, the bot
judging better, the code at its highest honest tier. Section 9 is venues.
Section 10 is what stops. Section 11 is where it is weakest. Section 12 is
cost. Section 13 is verification. Section 14 is the documents this changes.

**Paths.** `new:` marks a file this document proposes and that does not exist;
`workstation:` and `memory:` name files outside the repository.
`repo-conformance` rejects a bare path with nothing tracked behind it, and a
design is exactly the document most likely to name one.

---

## 1. The honest state, checked 2026-09-05

### 1.1 The box, read at 17:39 UTC

`systemctl list-unit-files "radar*"`, `ls -la /etc/radar`, `ls ~/bin`,
`crontab -l`, `df -h /`, and `radar brief` run the way the timer runs it,
all over `ssh guardian-vps-tail`:

| what | state |
|---|---|
| units installed | `radar-follow`, `radar-serve` (enabled, active); `radar-brief.timer` and `radar-creator-index.timer` (enabled); `radar-hosted` (disabled, not Radar) |
| units **not** installed | `radar-analyst`, `radar-seven-days`, `radar-payout` — the three the stack shipped. Nothing under `/etc/systemd/system/` names them |
| `/etc/radar` | `radar.env` only. No `alert.env`, no `analyst.env`, no `payout.env`, no `payout.json` |
| `~/bin` | `radar`, `radar-backfill`, `radar-analyst`, all dated 2026-09-04 23:54 — **built before the stack**, so `population.json` is not written, the summoner cap is charged on sending, and the week-close, daily and Telegram code is not on the box |
| `~/radar/data/analyst` | empty — the analyst has never run |
| crons | outcomes at `:17`, `radar consider --cap 40 --record` at `:37`, both hourly |
| disk | 38% used, 48 GB free |

`radar brief`, 17:39:44 UTC:

| check | reading |
|---|---|
| ingestion | 6 minutes behind, watermark slot 444,577,375 |
| launches | 556,235 recorded, 33,069 of them failed on chain |
| graduations | 47,207 recorded, 26,070 of them failed on chain |
| outcomes | 1,565,086 measurements |
| decisions | 9,906 recorded, 3,820 proposed, 4,744 with an entry price |
| coordination | **WARN**: 2,916 bps of the last 600 launch blocks sat at the centre, against 580 bps calibrated — the drift plan 0003's handback recorded, still firing to a journal nobody reads |
| agent, analyst | off; never run |
| trading | autonomy Observe, max position $0 |

The creator index the bot will read, rebuilt on the box at 16:55 UTC at slot
444,568,924, holds the venue's own population. **These are today's figures
and every consumer should read the file, not this table:**

| figure | value | derived |
|---|---|---|
| launches, succeeded | 522,407 | |
| measured | 521,031 | the denominator of every share below |
| filled over time (organic) | 9,327 | 1.79% |
| filled inside the launch block (instant) | 5,584 | 1.07% |
| graduate at all | 14,911 | **2.86%** |
| almost no activity | 118,368 | **22.7%** |

The prompt for this document carried 2.81% / 1.03% / 23.0% from the same
file a day earlier. The population moved by a few tenths in a day, which is
the reason the file carries its date and nothing hard-codes it.

### 1.2 The remainder, re-checked

The table `/plan-the-vision` carries, held against the tip and the box. Only
the `verified` column is new.

| design | row | verified 2026-09-05 | blocked on |
|---|---|---|---|
| 0007 A | fix what bites | done except A3: `/etc/radar/alert.env` is absent (read on the box) | one root command |
| 0007 B | the bot goes public | code complete on the tip. **Not installed**: no unit, no env file, binary predates the stack. **B4 is priced and not built**: `Cost::PostRead` exists in `crates/radar-analyst/src/spend.rs`, and nothing in `crates/radar-analyst/src/x.rs` fetches a post by id — `mentions`, `metrics` and the two posting calls are the whole client. A mention that names no mint and is a reply to one that does is answered "nothing" today | the X credential; the five prices; a root session for the unit and the env file |
| 0007 C | the token and the contest | C1–C5, C7, C8 on the tip; nothing has touched a chain | the credential; J12; the wallet; the devnet week |
| 0007 D | trading for humans | nothing built: no `/v1/tokens/{mint}/pretrade`, no `build` route, `web/src/Token.tsx` renders recorded decisions and measurements only | an audience |
| 0007 E | the learning loop | E2 done. **E1, E3, E4, E5 not built and nothing blocks them**: `radar-research` holds six modules — basis, control, creator index, exits, selection, study — and no features pass, no walk-forward, no research 0026 or 0027 | — |
| 0007 F | repository and workflow | F1: secret scanning and push protection **on**, Dependabot security updates **off** (`gh api repos/hey-vera/radar/automated-security-fixes` → `enabled: false`). F2: `release-linux` runs on pushes to `main` only, zero tags, zero releases. F3: the `.claude` directory holds `launch.json` and nothing else — no reviewer agent. F4: no pull-request template exists. F5 withdrawn. F6 `just orient` runs and printed the three open plans. F8: workspace `0.0.1` | F1 one click; F2's timer one root install |
| 0008 | the public site | phase 0 merged; phase 1 (#148) and the pool reading (#153) on the tip. **The OG image is still missing and so is the directory it would live in**: there is no public asset directory under the site, while `site/index.html` references `og.png` | the credential; the token; the domain |
| 0009 | three loops | L1–L6 decided; M1–M6 on the tip. Open: the ladder row when a coin's cap has fallen back (0028) | the credential; the launch |
| plans | `docs/plans/` | four plans, each starting where the previous handback stopped; `just orient` prints the open ones | section 3, V6 |

### 1.3 Three things documents get wrong, said once

The pattern the prompt asks for — a mechanism described as enforced that no
crate contains, or a figure copied wrong across documents — turned up as
follows. Section 8.4 has the full list with what to do; these three matter.

1. **`GOAL.md` says the rolling block source is not written. It is.**
   `crates/radar-backfill/src/launch_block.rs` implements
   `LaunchBlockSource::bundle_slots` with a CryptoHouse query, and
   `crates/radar-cli/src/consider.rs` calls it and reads
   `radar_graph::ongoing::strongest` off the result. The table row "bundling
   detected after launch — partly; the rolling block source is not" describes
   less than exists, which is the safe direction and still wrong.
2. **The price count and the switch count disagree inside two documents.**
   `docs/STATE.md` says "the four prices have no defaults" and, sixty lines
   later, "the fifth required price"; `deploy/README.md` does the same. Five
   is right (`Cost` in `crates/radar-analyst/src/spend.rs` has five
   variants). STATE.md also still describes one switch — the credential
   alone — where #141 made it two (`RADAR_X_PUBLISH=on` speaks).
3. **`crates/radar-graph/src/lib.rs` carries "68% … against 5%" beside
   `BUNDLE_CENTRE = 6`**, and a test comment in
   `crates/radar-strategy/src/creator_edge.rs` repeats it. Research 0024
   measured 25.1% on 2026-09-03 and moved the strongest band; ADR 0012
   decided the constant stays until the count is recorded, and the count has
   been recorded since 2026-09-04. The comments are the pre-0024 sentence in
   the file that will be edited when the threshold is re-derived.

Also: the `_comment` in `docs/research/data/0024-base-rates.json` says
"nothing reads it"; `radar-roast` has read it since design 0007 phase 2.

### 1.4 Two corrections to the brief

**The external study.** The prompt cites "the share of curve trades that are
bot-shaped, which an external study found the strongest predictor of
graduation". Read on 2026-09-05, arXiv 2602.14860 names **liquidity
velocity** — the number of trades needed to reach a given bonding-curve level
— as its single strongest predictor, and uses the bot share as a partition of
the sample, not as the leading variable. Both are computable from the store's
trades table, so the correction changes which fact section 7 puts first, not
whether either is measurable.

**The venue example.** Josh asked, mid-session, whether Radar and the bot
should cover memecoins broadly, naming Robinhood's platform. Robinhood Chain
is an Arbitrum Orbit L2 on Ethereum, launched 2026-07-01 (external claim,
three trade-press sources read 2026-09-05, unverified), with four memecoin
launch routes. It is a **second chain**, not a second Solana venue: a
different transaction format, decoder, RPC and signer. Section 9 answers the
question with that distinction.

### 1.5 Figures this document uses

Every figure carries a date and a source, or is marked an assumption.

| figure | value | source |
|---|---|---|
| succeeded launches / measured / graduate at all / instantly / almost no activity | 522,407 / 521,031 / 2.86% / 1.07% / 22.7% | the creator index on the box, 2026-09-05 16:55 UTC |
| launch block 1–3 recipients | 70.5% of launches; 0.02% graduate instantly | research 0024, sampled 2026-08-25 |
| launch block 10–13 recipients | 2.1% of launches; 10.1× base | same, weaker because sampled |
| the drift | 2,916 bps at the centre over 600 blocks against 580 calibrated | `radar brief` on the box, 2026-09-05 |
| organic / instant graduations, held to the end | median −3,228 / −5,981 bps | research 0011 |
| measured selection edge | 0 bps | research 0017 |
| the bar | ~456 bps; ~850 before a position above about $59 | research 0022 |
| venue fee on the curve | 125 bps a side; creator 30 bps | research 0023; pump.fun's fee page agrees (read 2026-09-05) |
| the fee after graduation | 25-row ladder, creator 30 bps below 420 SOL, 95 to 1,470, down to 5 above 98,240 | research 0028; the fee page agrees to the row |
| creator fee sharing | up to ten wallets, from 2026-01-10; **one** post-launch change of fee settings, then locked; "Cashback Coins" (fees to traders, chosen at launch) from 2026-02-17 | trade press read 2026-09-05, external, unverified against the program |
| USDC-quoted pump.fun coins | reported available since 2026-05-21 | a search summary of the fee page; **the page fetched today did not show it**; treat as unverified |
| decisions recorded / with an entry price | 9,906 / 4,744 | `radar brief`, 2026-09-05 |
| X: summoned reply / top-level post | $0.010 / $0.015 | design 0007 §11, verified 2026-09-03. Not re-raised, per Josh |
| Helius free tier | 1M credits a month, 10 RPS; a dossier 35–150 credits | same |
| all-in before viral | under $50 a month | same |
| store read on `/v1/tokens/{mint}` | ~3.2 s, and 4.4–5.2 s on a cache miss, against 500 ms | `crates/radar-serve/src/cache.rs`, measured on the box |
| arXiv 2607.02823 | 832,941 launches, 2026-05-08 to 06-10; fast-regime graduation 0.198%; initial cap above the 30 SOL default HR 4.51; a Telegram channel 1.485% vs 0.166% (8.94×); a six-minute window | the abstract, read 2026-09-05; external |
| MELT (arXiv 2602.13480) | 41k launches, 200M transactions; Jito bundle traces; on average 36.5% of supply held by coordinated accounts | the abstract, read 2026-09-05; external |
| a Jito bundle | up to five transactions, sequential and atomic, inside one slot, no gap | vendor guides read 2026-09-05; external; **a capture disposes** |
| `getBlock` with `transactionDetails: "signatures"` | returns the block's signatures only, in wire order | Solana RPC docs read 2026-09-05; ordering caveats exist for other methods |

---

## 2. The shape: two parts, one order

```
 Part A — close the remainder (unblocked rows only; Josh's list beside it)
 ┌──────────────────────────────────────────────────────────────────────┐
 │ A-1 the instrument: E1 features + E3 walk-forward   (plan 0007)      │
 │ A-2 the launch-block calibration, re-derived and on a timer          │
 │ A-3 the organic cohort study                                         │
 │ A-4 ship by tag; PR template; reviewer agent                         │
 │ A-5 the bot's last code: B4, the OG image, USDC curves, the checklist│
 │ A-6 documents that drifted                                           │
 │ A-7 the box: what needs no root now; what needs one root session     │
 └──────────────────────────────────────────────────────────────────────┘
                              │ nothing in B needs A-2 … A-7; B-1 and B-2 need A-1
                              ▼
 Part B — raise the ceiling, each "better" a number
 ┌────────────────────┐ ┌──────────────────────────┐ ┌────────────────────┐
 │ B-1 Radar smarter  │ │ B-2 the bot judges better│ │ B-3 the code       │
 │ edge ≥ 456 bps,    │ │ more facts, each with a  │ │ every property on  │
 │ twice, out of      │ │ base rate and a date;    │ │ its cheapest rung; │
 │ sample; AI mode in │ │ the investigator; calls  │ │ callers named;     │
 │ shadow, scored the │ │ scored in public as      │ │ nothing described  │
 │ same way           │ │ counts                   │ │ stronger than it is│
 └────────────────────┘ └──────────────────────────┘ └────────────────────┘
```

**Why A-1 is first, and first inside Part A.** It is the only row in three
designs that can move the number the whole product is measured against, it
needs no credential, no root and no decision about money, and it is also the
instrument every new fact in B-2 must pass before the bot may state it. The
rest of Part A is either small enough to run beside it on its own branch
(A-4, A-5, A-6 — AGENTS.md §8's three conditions hold) or is waiting on Josh.

**Why the bot is not first.** It is code complete. Everything between it and a
public account is on Josh's list (section 5), and a design cannot shorten that
list by adding code to it.

**What "the vision" means in this document.** Josh's words: the best trading
bot, with the strongest tokenomics intelligence; a public bot that always
tells the truth, digs like an investigator, takes the time it needs, and gets
its judgement from Radar. Translated into things that can be checked:

- *Best trading bot* cannot mean "trades". Nothing may trade until the bar is
  cleared twice out of sample (decided; GOAL.md). It means the instrument
  that would find the edge exists and runs, the refusals are right and
  measured, and a human who trades manually sees the round trip at their
  size before they click. B-1 and design 0007 D.
- *Tokenomics intelligence* means facts about supply, fees, authorities,
  holders and the launch block, each with a base rate and a date. B-2.
- *Digs deeper and takes its time* means a bounded second pass — the
  investigator — that spends a stated budget of calls and minutes, decided
  by a rule, never by the model. B-2.
- *Always tells the truth* means what the sheet already enforces, plus its
  own calls scored in public. B-2's calibration.
- *Derived from Radar's intelligence* means the bot states nothing Radar has
  not measured against its own record. That rule is the whole of B-2's
  admission test.

---

## 3. Decisions that are Josh's

Numbered V1–V7 so they do not collide with 0007's J, 0008's K or 0009's L
rows. Each: the decision, the options, **my pick** and why, and what this
document assumes if he says nothing. Money, public surfaces and legal
exposure are not mine to decide.

| # | decision | options | my pick | assumed |
|---|---|---|---|---|
| V1 | **Part A's order**: the learning loop's instrument first (A-1), with A-4 to A-6 in parallel on their own branches | (a) as above; (b) hygiene first, then E; (c) wait for the bot to go live before any of it | **(a)**. A-1 is the only item that can change the ceiling and nothing blocks it; the hygiene items are hours and independent | (a) |
| V2 | **An AI strategy in shadow** (§6.3): a model proposes over the same candidates, recorded beside the deterministic rule, scored by E3, with zero authority | (a) not until E3 exists; (b) build now; (c) never | **(a)**. It is held to the same bar, and the bar has no instrument yet. Built after plan 0007 lands, funded by a line Josh sets (about $2 a day at the hourly cap — an assumption on model price) | (a) |
| V3 | **The investigator's second reply** (§7.3): when the first pass leaves a question a rule can name, the bot spends up to a stated budget and posts once more in the same thread | (a) yes, capped per day and priced as a reply; (b) fold the deeper read into a slower first reply; (c) no | **(a)**. The first reply lands inside twenty seconds so the thread is alive; the second is what "takes its time" means, and a cap keeps it from being the bill. (b) makes every reply wait for the slowest question | (a) |
| V4 | **The account states its own measured edge** weekly — "measured edge this week: 0 bps; nothing traded" — as GOAL.md's honesty made public | (a) yes, after J4's read; (b) keep it in the research notes only | **(a)**. It is the one sentence no competitor can post, it is measured, and it ages in public like everything else the account says. It is a public statement about Radar's own performance, so it goes to the lawyer with the rest (§7.5) | (a) after J4 |
| V5 | **The creator's funding source on the sheet** (§7.2, row f): an address-level fact, one hop, computed locally | (a) measure it in E1 and write the base-rate note; state it on the sheet only after J4 covers it; (b) state it as soon as measured; (c) refuse it | **(a)**. It is an address, not a person, and README lists the funding graph as one of Radar's four edges — but it is the fact closest to the identity line ADR 0013 draws, and the wording matters more than the number | (a) |
| V6 | **`docs/plans/`**: keep or delete on 2026-09-17 | (a) keep the directory, drop the date; (b) delete on the date as design 0002 set; (c) extend the date | **(a)**. The evidence: plan 0004 starts where 0003's handback points, plan 0006 closes 0005's open item 10 by name, `just orient` prints the open handbacks on every start, and this document was written from them. The honest test F6 asked for — a session that was not told — cannot be run, because the session protocol now tells every session. Keep the thing that is read; delete the date that measured the wrong question | (a) |
| V7 | **Venues** (§9): the bot's venue-agnostic tier now; the next Solana venue chosen by the bot's own refusal log; no second chain in this design; trading pump.fun-only until an edge exists | (a) as stated; (b) record a second Solana venue now, before the bot says which; (c) add Robinhood Chain | **(a)**. (b) is a decoder and a recorder change for a venue nobody has asked the bot about yet; (c) is a second chain and a different repository's worth of work. GOAL.md's line on trading stands and is Josh's already | (a) |

**Not reopened**, per the prompt and the documents: ADR 0013's six
constraints; 0009's L1–L6; the frozen trading lane; summoned-reply-only; no
social or identity scraping; the x402 lane out of the analyst path; rule 1;
pump.fun as the only recorded venue until an edge; the two X billing figures.

---

## 4. Part A — close the remainder

Every unblocked row of 0007, 0008 and 0009, in the order to build them. Each
item names its caller, its files and the gate that says it is done. Items
share a PR only where the table says so; everything else is one PR each, so
`mutants-shards` stays cheap.

### A-1 The instrument: E1 and E3 — [plan 0007](../plans/0007-the-learning-loops-instrument.md)

**Why.** The only thing that can move the 456 bps number, and the admission
test for every fact in section 7. Design 0007 E1 and E3, unchanged in aim;
changed in two ways, both said here: the protocol runs in Rust over the
store's own reader rather than in a Python script, because
`scripts/probe/README.md` promises stdlib-only probes and a pinned `pyarrow`
environment on the owner's workstation is a new toolchain for a first pass
that needs no model; and every feature value is an
`radar_asof::Observed<f64>` accepted against the row's watermark, which is
where the look-ahead guard moves from a test to the type — section 8.1, row 2.

**Caller.** `radar features` and `radar edge` in `radar-cli`; the weekly
re-run in A-2; research 0026.

**Gate.** `radar features` run twice produces identical bytes; a feature
observed one slot after the row's watermark is refused as `LookAhead` and
never a number; a seeded noise feature is never reported `Found` across ten
seeds; research 0026 is written with the result on the deterministic rule,
whatever it is. Plan 0007 has the items.

### A-2 The launch-block calibration, re-derived and on a timer — ADR 0012 c2, c3; 0007 E5

**Why.** `radar brief` on the box has been warning since at least 2026-09-04
that 29% of recent launch blocks sit at the centre count against 5.8%
calibrated. Either the venue moved or the sample is not what it is believed
to be; nothing tells which, and the constant it compares against came from
a measurement 0024 superseded two days ago. Since 2026-09-04 the count is
recorded on every decision (`Decision.launch_recipients`), so re-deriving a
threshold is a query.

**What.** `new:crates/radar-cli/src/snapshot.rs`: `radar snapshot` re-runs the
0024 query (`docs/research/queries/0024-launch-block-recipients.sql`) over the
last twelve hours against CryptoHouse, labels the launches from the store's
graduation table, and writes a dated snapshot in the shape
`docs/research/data/0024-base-rates.json` already has — beside the creator
index on the box, never committed. The bot reads the newest snapshot; the
`coordination` check compares the recorded count distribution over the last
600 decisions against the newest snapshot rather than against a constant,
and **fails, not warns**, when the strongest band's mode has moved between
two consecutive snapshots (LEARNINGS 21 and 26: a monitor slower than its
failure reports history; a check must fail differently when it did not run).
`radar-graph`'s thresholds are then derived from the recorded distribution
once it holds at least 5,000 decided launches with a count (an assumption:
about a week at the hourly cap of 40), carrying the date, and the two stale
comments in section 1.3 go in the same commit.

**Caller.** A second `ExecStart` on `deploy/radar-creator-index.service`
(root, once — section 5), or the unit's existing timer if the pass is folded
into `radar creator-index`. The brief. `radar-graph::assess` reads the
derived thresholds.

**Gate.** Two consecutive snapshots on the box, and the brief's
`coordination` line either green with a date or red with the band that
moved; `assess` re-derived from the store reproduces 0024's band on 0024's
window (two instruments compared).

### A-3 The organic cohort study — 0007 E4, research 0027

**Why.** 0011's one open cohort: organic graduations clear costs twice as
often and are not structurally spoken for. Whether any of it survives entry
forty minutes late, priced both sides the same way, is the one entry-side
question the store can answer without trading.

**What.** `new:docs/research/0027-the-organic-cohort-on-the-amm.md` with its
query under `docs/research/queries/`: the first 24 hours after graduation on
PumpSwap, priced outcome-to-outcome (0016's lesson), matched on age and
hold (0017's method), net of the ladder row the coin sat in (0028). If it
finds a stratum, it is a candidate for E3, not a strategy.

**Gate.** The note names its query and the query runs; every figure carries
its window; the null is written as a result.

### A-4 Ship by tag; the PR template; the reviewer agent — 0007 F2, F3, F4, F8

**What.** `release-linux` also runs on `v*` tags and attaches the three
binaries and `BUILD-INFO.txt` to a GitHub Release (`.github/workflows/release-linux.yml`);
`new:deploy/radar-deploy.sh`, `new:deploy/radar-deploy.service`,
`new:deploy/radar-deploy.timer` fetch the latest release every fifteen
minutes, verify sha256 against `BUILD-INFO.txt`, install `radar`,
`radar-backfill` and `radar-analyst` into `~/bin` by rename (no sudo), and
stage `radar-serve` under `/tmp` for the human step the runbook already
describes. `new:.github/pull_request_template.md`, five lines, the shape the
stack's PR bodies already use. `new:.claude/agents/reviewer.md`: a cheaper
model, given the plan file and `git diff origin/main`, asked one question.
The version stays `0.0.1` until the bot's launch, when F8 bumps it to
`0.1.0` and tags it; the release path is proved before that with a
pre-release tag.

**Caller.** The workflow; the box's timer; the next session.

**Gate.** `gh release view` on the pre-release lists the four files;
`sha256sum -c` passes on the box; the timer installs by rename and
`readlink /proc/<pid>/exe` shows the new file after a restart; the template
appears on the next PR.

### A-5 The bot's last code items

| item | what | proof |
|---|---|---|
| B4 | the parent read: when a mention names no mint and is a reply, one `GET /2/tweets?ids=<parent>` — the shape `metrics` already uses, with the default `text` field — through the same strict parser, priced as `Cost::PostRead`, capped at one per mention | a fixture page whose mention holds no mint and whose parent holds one produces a reply; one whose parent holds an instruction produces nothing; the meter is charged once |
| the OG image | `new:site/public/og.png`, 1200×630, the three numbers and the claim; `site/index.html` already references it | the site test asserts the file exists; a card validator once the domain is live |
| USDC-quoted curves | a capture of one pump.fun coin quoted in USDC, if the venue offers it (section 1.5: reported, unverified); until the layout is asserted from a capture, the dossier **refuses** a curve whose quote mint is not SOL rather than pricing it as SOL | the capture, or a documented refusal with the reason on the sheet |
| the ladder row | 0028's open question: one pool watched across the 420 SOL step in both directions, a few hundred signatures over a day | the capture asserted; `fees_at_market_cap` unchanged until the rule is known |
| the launch checklist | three lines in `deploy/README.md` step 4: do not opt into Cashback Coins; do not opt the creator fee into charity; **never use the one post-launch fee-settings change** — the vault `radar-payout` reads belongs to the launch wallet, and a redirected fee is a prize that silently vanishes rather than a refusal | a reader can check each against the venue's own page |

### A-6 Documents that drifted

In the same commit as the code they describe where a PR touches it, else one
docs PR: the three items in section 1.3; the snapshot's `_comment`; the
verification table in `deploy/README.md` (dated 2026-08-25, so not wrong;
incomplete — the creator-index units run and the table does not list them);
`docs/STATE.md`'s "Where to start" once A-1 gives `Observed<T>` a caller.
Every figure the fixes touch carries its date.

### A-7 The box

What needs no root, and can be done the day the stack merges: the three
`~/bin` binaries from the merged build, so `population.json` starts flowing on
the next creator-index run and `/v1/public/stats` stops answering 404. What
needs one root session, in one sitting, with the exact commands already in
`deploy/README.md`: `radar-analyst.service` and `/etc/radar/analyst.env`;
`radar-seven-days.timer`; `radar-payout.service` when the wallet exists;
`/etc/radar/alert.env`; the second `ExecStart` from A-2; the deploy timer from
A-4; the NOPASSWD line for the three restarts. Then the 24-hour dry run, which
needs the credential.

**Not in Part A**, and why: design 0007 D (needs an audience; gated on B and
C); the devnet week (needs the wallet); anything on Josh's list.

---

## 5. Only Josh can do these, and what each unblocks

Nothing else goes on this list.

| what | unblocks |
|---|---|
| merge #147 to #154 in order, then #146 and this PR | every "on the tip" row above being on `main` |
| one root session on the box: the analyst unit and `analyst.env`, the seven-days timer, the alert file, the NOPASSWD line for the three restarts; later the payout unit and the deploy timer | the 24-hour dry run; the alarm that says when the bot dies; A-2's timer; A-4's pull-by-tag |
| the X developer account, its credit, and the one live test post that settles the five prices | the live gate; C2's scoring reads; everything in 0009 M1–M4 with real data |
| a Telegram bot token (BotFather, five minutes) and a channel for the two posts | M5's live test; the free lane |
| Dependabot security updates — one click under Settings → Code security, or one `gh api -X PUT repos/hey-vera/radar/automated-security-fixes`, which the session's token has admin for; a settings change, so it is here and not done | nothing; do it today |
| the token's name and a fresh wallet, gated on J12, its keypair installed as the payout key | the devnet week; C7; the pool page's first real reading |
| the legal read (J4), with 0009 §8's documents plus this document's §7.5 | the first public post; V4; V5 |
| `cabalhunter.org` and its Cloudflare Pages project | the site going live |

---

## 6. Part B-1 — Radar smarter

### 6.1 The bar, stated once

A strategy has an edge when its expected return, **net of the round trip for
the band the position would sit in**, is at or above about **456 bps**,
measured on rows it was not fitted on, on two non-overlapping test folds, in a
stratum Radar can size into (about $60 at the 1% impact budget; research
0022). 0022 also says a position above about $59 needs about 850, so the
harness reports both thresholds and the plan calls a stratum found only on the
first. The round trip comes from the snapshot's `by_notional` table, never from
a constant in the harness (0019: cost is a function of size).

### 6.2 The protocol — E3, so that a null result is a result

Design 0007 E3, made exact, with two additions the literature insists on and
0007 did not name.

- **Rows.** E1's feature table: one row per succeeded launch with an outcome
  at the one-hour and the twenty-four-hour checkpoint, every feature observed
  at or before T = launch + 6,000 slots.
- **Folds.** Five contiguous windows by launch slot, equal in rows, never
  shuffled. Time order is the whole point.
- **Purge and embargo.** A label takes twenty-four hours to mature. A fit
  fold's rows must have their twenty-four-hour checkpoint before the fold
  boundary, and the test fold begins 216,000 slots after it, so no row's label
  overlaps a row it is tested against. This is purged cross-validation with an
  embargo (López de Prado, *Advances in Financial Machine Learning*, 2018,
  chapter 7), and it is the difference between a harness and a leak.
- **Strata.** A stratum is a conjunction of at most three feature thresholds,
  thresholds at fit-fold deciles, enumerated. The fit-fold winner is the
  stratum with the highest median net return and at least 100 rows.
- **The test.** The winner runs on the next two folds. `Found` only if the
  median net return is at or above the bar on both, each with at least 100
  rows, **and** the Wilson 95% lower bound of the share of rows with a net
  return above zero exceeds one half on both. Two conditions because a median
  over a point mass at zero is a report about the point mass, which is what
  0017 found in its short-hold strata.
- **How many were tried.** The count of strata enumerated is printed beside
  every verdict. A winner chosen from ten thousand candidates is expected to
  regress; the two test folds are the control, and the count is what lets a
  reader discount the result. A deflated-Sharpe correction is the refinement,
  not built now.
- **Fixed strata, no fitting.** `creator_edge`'s thresholds as one stratum —
  the first result, and research 0026's headline. And the refusal signals'
  complements — the strongest band or above; a creator with measured launches
  and no organic graduation — so the **refusal edge** is measured too: how
  much worse the refused set does than the rest, net. That is the product the
  bot sells today, and it has never had a number of its own.
- **Deterministic.** Seeded, no clock, no network; `radar replay`'s standard.

**Two planted tests, two different properties.** A leaked feature does not
die on fold two; it wins every fold, which is why a leak cannot be caught by
the fold design and must be made impossible earlier. E1's row builder accepts
every feature value as `Observed<f64>` against `AsOf(T)`, so a feature
observed after T is a `LookAhead` error and never a number — rung 1, and the
first real caller `radar-asof`'s two idle types have had since 2026-09-04
(section 8.1, row 3). Overfitting *does* die on fold two, so E3's planted test
is a seeded uniform-noise feature added to the grammar: across ten seeds it is
never `Found`, and when it wins a fit fold it fails both test folds.

### 6.3 What happens when it finds nothing

That is the expected outcome (0007 §12 item 5; still my prior). Then:

1. Research 0026 says so, with the tables, and GOAL.md's honest-state
   paragraph keeps "0 bps" with the new date and the new n.
2. Nothing unfreezes. `Policy::CLOSED` ships.
3. The harness runs weekly on the box beside the snapshot (A-2's timer), so
   "nothing" is re-measured on a schedule rather than believed.
4. V4 puts one measured sentence on the account each week. Nobody else can
   post it, and it ages in public like every other statement the bot makes.
5. The product stays "lose less", which is the one GOAL.md says can ship
   honestly today, and the refusal edge from 6.2 is its number.
6. **The loop that keeps it moving:** every new fact section 7 admits to the
   sheet is an E1 feature first, so every sheet fact is a new stratum
   candidate for the next run. Contiguity, trades-to-depth, holders, the
   funder — each arrives as a feature, is measured as a base rate for the bot,
   and is tried as an entry stratum for Radar. The bot's work feeds the edge
   work, and the edge work is what makes the bot's facts predictive rather
   than descriptive.

The one event that changes anything is `Found` twice, and it triggers a
document, not a code change: an ADR, Josh's, about opening the lane at
Canary with capital — and ADR 0012's threshold becomes urgent in the same
change, as that ADR already says.

### 6.4 AI mode under rule 1 — open question 7

What exists: `radar-agent` (read-only tools, observed text fenced, an answer
that is text nobody parses), `radar-model` (the vendor CLI as a subprocess, or
a metered key) and `radar-serve`'s chat route. Off on the box: no provider is
configured. That is GOAL.md's *talking to it*, and it is the first form.

**Form one, the conversation.** A model over the same tools the deterministic
strategy reads, able to ask for more. Two additions: the dossier as a
read-only tool (`radar-onchain` is a read; the allowlist admits it by name),
and the sheet's base rates as tools, so the model can cite a base rate rather
than recall one. "Stop everything" never travels through it, by construction:
nothing parses its reply, and stopping is a `Policy` at the kernel and the
signer's own file (ADR 0008). What it must never be: a thing whose text is
parsed into an action, a thing that sees unfenced metadata, or a thing that
answers from memory when a tool is unfunded.

**Form two, the shadow strategy (V2).** The honest version of "the AI finds
edge the rules do not": it proposes, in shadow, and is measured.
`new:crates/radar-strategy/src/ai_shadow.rs` implements a `Shadow` trait, not
`Strategy`. It sees one `Candidate` rendered as the fact sheet, with the
creator's strings fenced by the same `radar_agent::untrusted::fence`; it
answers a closed vocabulary — propose or pass, plus reason codes from a fixed
list — parsed strictly, anything else being a pass with the reason
`Unparseable`; sizing is the deterministic `size()`, never the model's;
the result is a `ShadowDecision`, recorded to the decisions table with
`strategy = "ai_shadow"` and the model id as the version. **There is no
`From<ShadowDecision> for Decision`**, so the pipeline cannot pick one up and
the kernel never sees it: rule 1 stays on rung 1. E3 scores it as a stratum
("the model said propose") against the same folds as everything else. Cost is
metered through `radar_agent::Agent::begin` — no budget, no calls (rule 8).
Promotion is an ADR, Josh's, only after `Found` twice on that stratum, and
even then it emits `Proposal`s into the same kernel under the same `Policy`.

What it must never be, in one list: a sizing authority; a path to the kernel's
authorisation or to the signer; a parser of free text into a mint or an
amount; a consumer of unfenced metadata or of any social text; a relay for
stop; a strategy that trades because it is an AI.

**Cost.** One model call per considered candidate: at the hourly cap of 40
that is 960 a day, and at about $0.002 a call on a small model — an assumption
to verify with the `claude-api` skill when it is built — about $2 a day. The
line is Josh's (V2).

---

## 7. Part B-2 — the bot judging better

### 7.1 The admission test for a fact — open question 4, first half

A fact may be stated when all five hold. This is the whole of "derived from
Radar's intelligence", made checkable.

1. It is read from the chain, or from a published file, with the slot or the
   moment it was measured.
2. A base rate exists for it over Radar's own record, with a date, in a file
   the sheet reads — never a constant, never a memory.
3. Its honest renderings are enumerated as `Fact.values`, so the fidelity
   check authorises exactly those: a count authorises the count; a share
   authorises the ratio, the percentage and the rounded percentage; a SOL
   figure authorises the integer rendering.
4. Its label says what the number is *not* — accounts, not people; a budget,
   not a ceiling — because the label is what the model reads.
5. It has been an E1 feature first, so the base-rate note was written from
   the same pass that feeds E3.

The boundary is `FactSheet::build` in `crates/radar-roast/src/sheet.rs`. A new
fact is a `push_*` there, tagged `About::Measurement`, and nothing else in the
reply path changes.

### 7.2 The candidates

Every source is on chain or in Radar's own record. Calls are per reply above
the dossier's sixty.

| candidate | source, and calls | base rate needed | fidelity authorises | verdict |
|---|---|---|---|---|
| **a. mint and freeze authority; Token-2022 extensions** | one `getAccountInfo` on the mint; the parser exists in `crates/radar-sim/src/mint.rs`, verified against mainnet: pump.fun mints are Token-2022 with both authorities revoked at creation | none for a state; on pump.fun it is the venue default and a line that never differs | nothing numeric; the label carries the state | **build for the venue-agnostic tier** (§9). On pump.fun state it only when it is not the default — a latch that reopened is a raise, not a fact (rule 5) |
| **b. holder concentration outside the pool** | `getTokenLargestAccounts` and `getTokenSupply`, two calls; the curve's own token account excluded (the dossier holds the curve) | none exists; a stdlib probe over about 500 mints at T gives one, paced on the free RPC | a share and a count | **build after the probe.** Label: token accounts, not people (0012). It cannot be an E1 feature over history — the store holds no balances — so it enters E1 from the day the reply log records it; say so |
| **c. launch-block contiguity** (the bundle's shape) | one `getBlock` with `transactionDetails: "signatures"`: the block's signatures in order; the mint's launch-slot signatures mapped to positions; the longest run of consecutive positions. From the store, `tx_index` on the trades in the launch slot gives the same for every recorded launch (E1) | E1 over the store: the share of launches whose launch-slot buys form one run of at least k, by never / organic / instant | two counts | **build, fixture first.** A Jito bundle is sequential, atomic and inside one slot (external), so its transactions are consecutive in the block; the RPC docs say wire order and a caveat exists for a neighbouring method. Capture a known bundled launch's block and assert the order before anything reads it (ADR 0009's discipline) |
| **d. trades to depth** (liquidity velocity) | for the sheet, zero extra calls: the dossier already pages the token's signatures (at most three pages) and reads `real_sol_reserves`; carrying the successful-signature count onto the `Dossier` as a `Count` is one new field, and "N transactions moved the curve to X SOL" is one division; for E1, `realised_lamports` cumulated in slot order until 10, 20 and 30 SOL | E1 | two numbers and their ratio | **build first among the new facts.** The strongest predictor in arXiv 2602.14860, on chain, free |
| **e. trader prevalence** ("bot-shaped" share) | a published file, written by the creator-index timer's pass over the trades table: addresses that traded at least K distinct mints in the last hour, K read off the histogram the way `REPEAT_FLOOR` was; the launch block's buyers (the dossier holds the transactions) looked up in it | E1 | a count and a share | **build after E1 measures it.** Label: "addresses that traded N other launches this hour", never "bots" — the study's bot flag is its own heuristic and Radar states what it counted |
| **f. the creator's funding source, one hop** | the creator's oldest signatures (already paged) and `getTransaction` on the oldest one to three, to find the first inbound SOL transfer's source; computed locally — a vendor sells a "funded by" field, and LEARNINGS 11 is why Radar does not buy derived fields. The cross-creator join is a funder index: incremental from the reply log's dossiers, plus a batch CryptoHouse query over the store's creators (first inbound per address) — an assumption about the endpoint's row cap, in batches | E1: how many creators in Radar's record share this funder, and their organic / instant / stillborn shares | counts | **V5: measure now, state after J4.** Label: addresses. "N creators in Radar's record were first funded by the same address" is a fact about addresses; "the same group" is on the forbidden list and stays there |
| **g. authority prevalence** (0012, 0013) | recorded on decisions since 2026-08-30 for decided mints; live, it is the CryptoHouse ninety-minute query — about nine seconds (0013), too slow for the first reply, fine for the investigator | activity only, twice replicated (0013); the money half comes from E1's labels | a count and a band name | E1 feature now; an investigator fact after the note; never a refusal on its own until the money half is measured |
| **h. repeated metadata** | the launches table's name, symbol and URI host, indexed in the creator-index pass: how many prior launches share each | E1 | counts | E1 and the index now; the sheet after the note. Cheap, and the shape of a factory |
| **i. post-launch bundling** (rolling) | `radar_graph::ongoing` with the CryptoHouse source; `radar consider` already calls it | E1 carries the strongest sighting | a run length and a slot | E1 now; an investigator fact later (a query per coin) |
| **j. refused** | anything social; anything in the metadata URI (never fetched — a stranger's mint must not choose Radar's outbound requests); anything that links an address to a person; a "bundled %" as one number (a score); a verdict word | — | — | **The cost, stated once:** arXiv 2607.02823's strongest single predictor is a Telegram channel at launch (8.94× lift, external, unverified). It is unavailable by two decisions, and this design accepts that. Section 11 |

**Per-reply cost under rule 6**, all of it decoded locally: the dossier's
sixty calls (35–150 Helius credits, 2026-09-03), plus one to three calls for
a, b and c, plus the X reply at $0.010 and a model call at fractions of a
cent. Nothing here buys a parsed transaction or a derived field.

### 7.3 The investigator — "digs deeper, takes its time"

Two passes, and a rule between them.

**Pass one is the reply as it is today**: sixty calls, three pages, twenty
seconds, then the sheet, the voice, the two checks, the log, the post. It
lands while the thread is alive.

**Pass two runs when a rule says a question is open and worth a budget.** The
rule is a pure function of the first dossier and its sheet, in
`new:crates/radar-analyst/src/investigate.rs`, tested the way the gate is:

- a recipient or transaction count came back `AtLeast` (the budget cut it);
- the launch could not be reached inside the page budget (an older or
  spammed mint);
- the creator has at least five measured launches — a record deserves the
  rolling-bundle read (i) and the funder read (f);
- the curve is complete — a graduated coin's facts are the ladder row and
  the pool, which the first pass does not read.

**The budget is written down, and unset means none.** `Budget` in
`crates/radar-onchain/src/budget.rs` with larger allowances from four
variables: calls (say 600), pages (30), seconds (300), and investigations per
day (10). Unset is zero and zero is refuse — rule 8, the same shape as the
prices and the caps. "However long it feels comfortable" is those four
numbers, and the meter reads them back.

**It is charged to the account, never to the summoner.** The summoner's
allowance was spent on admission for the first read; the second read is the
bot's own choice, so it draws on the account's global cap and the spend meter
as one more `Cost::Reply`. A publisher outage still cannot spend it.

**The output is a second reply in the same thread**, through the same
sheet → voice → render → forbidden → fidelity path, logged before it is said.
The second sheet is a superset of the first — the first's facts plus the
deeper ones — so the two replies cannot disagree on a number they both state,
and a test asserts exactly that. The words are the model's; "looked closer" is
a suggestion, not a template.

**What the model still cannot do:** decide to investigate, choose what to
read, or see anything unfenced. The investigator adds facts to the sheet. The
voice pass writes the sentence.

**Telegram** gets the same rule at no X cost, which is where volume goes
(0009 L5) and where a deeper read is free.

### 7.4 Calibration — open question 4, second half

The bot makes no predictions. It states conditional base rates ("a launch in
that band graduates instantly 12.0% of the time") and counts (the creator's
record). So calibration is not "was the verdict right" — there is none — it is
**does the stated rate match the realised rate over the coins it stated it
about**, seven days later, per band, as counts:

> of the 40 coins the account placed in the ten-to-thirteen band, 5 graduated
> instantly (12.5%); the snapshot said 12.0% [9.1–15.7].

**Mechanism.** The reply log entry gains the band name and the stated
`p_instant` (both already on the sheet). `radar seven-days-later` — which
exists, runs on the timer, and is the one join the analyst may not make —
already joins week-old replies to outcomes; it gains a weekly
`calibration.json` beside its daily rows: per band, the stated rate, n,
realised k and a Wilson interval; per refusal signal, the share of signalled
coins that graduated at all, instantly, or died. The weekly post carries one
line of it — counts about the account's own statements, which is 0009 L6's
mechanism and needs no new decision. The site's leaderboard page gains a
"seven days later" section reading the file, on design 0008's file-backed
rule. A Brier score is computed in the research note and never posted: a
score is a score.

**Never a verdict.** The words are counts and the snapshot's own interval.
"Right" and "wrong" join the forbidden list.

**Two caveats the page prints.** The sample is the coins people asked about,
not a random draw — without that sentence the page is the backtest simulator
GOAL.md refuses. And a band's realised rate over forty coins has a wide
interval; the interval is printed, not the point.

### 7.5 Legal exposure — flagged, not answered

Three things for J4's read, beside 0009 §8's documents. V4's sentence and
7.4's page are public statements about Radar's own record — performance
claims, in the unflattering direction, and still claims. V5's funder fact is
an address-level statement nearest the identity line ADR 0013 draws; the
wording in 7.2 row f is the safest form and is still worth the read. The
investigator changes nothing: same checks, same sheet, one more reply.

---

## 8. Part B-3 — the code at its highest honest tier

### 8.1 The audit — open question 6

AGENTS.md §5's ladder: (1) impossible, (2) one mechanical check, (3) a test,
(4) prose. Each property, where it lives, the rung it sits on, the rung it
claims, and what to do. Nothing below re-tests a rung-1 guarantee.

| # | property | where | rung | claims | action |
|---|---|---|---|---|---|
| 1 | model judgement never authorises capital | `radar-agent` has no `Proposal` and no path to the signer; conformance holds the dependency claim | 1 | 1 | none. The shadow strategy stays on 1 by having no conversion into `Decision` |
| 2 | nothing reads past its watermark, in the store | four `admits` call sites and one test | 3 | 3, since AGENTS.md was corrected on 2026-09-04 | none |
| 3 | `Observed<T>` and `LookAhead` | `crates/radar-asof/src/lib.rs`; no caller. STATE.md's "`AsOf` plus two types nothing uses" is exact: the crate's third type, `PointInTime`, **is** implemented by `Reader` in `crates/radar-store/src/reader.rs` and is not one of the two | 1, unused | 1 | A-1 gives them a caller — a feature value observed at a slot, accepted against `AsOf(T)`. If plan 0007 finds no need, delete both in the same PR. A decision with a date, not an open question |
| 4 | the launch-block threshold | `BUNDLE_CENTRE = 6` in `crates/radar-graph/src/lib.rs`, a constant from 0008 with 0008's figure in its comment | 4, wearing a test | 3 | A-2: derived from the recorded count, dated; the brief fails on drift |
| 5 | post-launch bundling | `radar_graph::ongoing` with the CryptoHouse source in `crates/radar-backfill/src/launch_block.rs`; `radar consider` calls it | 3 | GOAL.md says "not written" | fix the document (A-6). E1 carries it |
| 6 | the reply's numbers | `FactSheet::authorised` and the fidelity check, sanitised before checked | 2 and 3, re-applied | as claimed | none. A new fact is a `push_*` (7.1) |
| 7 | the self-mint rule | `About::Price` dropped before the model; the tag chosen at the call site | 1, with a stated residual | 1 | none; the residual is written where it lives |
| 8 | the gate and the meter | pure; the day from the clock argument; re-applied bugs | 3 | 3 | none |
| 9 | `min_notional` in dollars against a cliff in lamports | pinned by a test at a reference SOL price | 3, a compromise | says so | defer until anything trades; the type change is the fix |
| 10 | the 500 ms budget | an opt-in test behind `RADAR_BUDGET_STORE` that CI cannot run | 4 in CI | 3 | keep the store-scan routes on the operator console only; public routes and D1 never scan (8.2); the budget test becomes a line in the runbook's deploy check, run on the box |
| 11 | declared instrument cost against measured spend | `crates/radar-instruments/src/spec.rs`, "a promise, not a measurement" | 4 | says so | leave; the paid surface is off |
| 12 | the custody lane | `radar-customer`, Privy, Turnkey, `web/src/Wallet.tsx`, `siws` | frozen (J10) | frozen | no edits; not deleted — D6 identifies a customer by `siws` |
| 13 | replay determinism | `radar replay`'s `NotDeterministic` standard | 3 | 3 | E1 meets it: two runs, identical bytes |
| 14 | mutation coverage on changed behaviour | CI shards; `.cargo/mutants.toml` with reasons | 2 | 2 | none |
| 15 | every layer has a caller | the two types in row 3; the x402 client (unbuilt, by decision) | — | — | none new. E1 and E3 live in `radar-research` and `radar-cli`; no crate is added, so no document that compiles |

### 8.2 The store's read cost — open question 5

Who needs what, and none of it scans the store per request:

| surface | reads |
|---|---|
| the public site | three published files, at the edge |
| the daily post | a timer job's file, from one join |
| the bot | RPC and files |
| D1, the pre-trade sheet | the dossier (RPC, about 1.5 s), the curve arithmetic (`radar-pumpfun`, pure), the creator index and the snapshot (files) |
| D4, the journal | a per-wallet append-only file |
| the operator console's `/v1/tokens/{mint}` | the store, behind the cache with labelled staleness |

So the existing cache is the answer for the console and nothing else needs
one; no second index is needed for any public surface, because the creator
index already is that index; and the rule for D1 is the site's rule — files
and RPC, never the store per request. Rule 3's easiest break, a new cache, is
avoided by not building one.

### 8.3 What would raise a rung, and whether it is worth it

The leak guard (row 3) and the threshold (row 4) are the two, and both are
in Part A. A third — a check that every `Fact` the builder pushes carries
values or an explicitly empty list with a label saying so — is one mechanical
check in one place and is *not* proposed now: no survivor has shown it is
needed, and a check that cannot fire is a cost.

### 8.4 Documents describing something else

Section 1.3 has the three that matter. The full list, for A-6: GOAL.md's
rolling-source row; STATE.md's four-or-five prices and one-or-two switches;
deploy/README's four-or-five; the snapshot's `_comment`; the two stale
comments in `radar-graph` and `creator_edge`; deploy/README's verification table, dated
2026-08-25 and missing the creator-index units that run; design 0007 §1's
"the X client is not written" (a dated plan that says it decays — leave);
README's and GOAL.md's counts (dated — leave).

---

## 9. Venues — open question 8, and Josh's question of 2026-09-05

### 9.1 The bot: two tiers, and the measurement that picks the next venue

**The venue-agnostic tier works for any Solana mint today** with two to five
calls: mint and freeze authority and the Token-2022 extensions (7.2 a), holder
concentration outside the pool (b), the token's age and first slot (from the
signatures the dossier already pages), the creator's pump.fun record if that
address is in the index — a creator who launched on pump.fun and now launches
elsewhere is exactly the fact worth stating — and, after V5, the funder (f).
No base rates until the venue is recorded, and the sheet says so in words:
"Radar records no population for this venue."

**The venue tier is pump.fun today**: the launch block, the curve, the fee,
the population rates.

The dossier's first branch decides which tier a mint gets. Today a mint with
no bonding-curve account is refused as "not a pump.fun token"; it becomes a
fact instead — "not a pump.fun launch; Radar records no population for its
venue" — and the venue-agnostic facts follow.

**The measurement that picks the next venue.** The reply log gains the program
that created the mint, decoded locally from the mint's oldest transaction.
Counting refusals-by-program over the first month says which venue people
actually ask the bot about. That venue is recorded next; no venue is recorded
on a guess.

### 9.2 Radar: what changes for a second Solana venue, and what does not

Changes: a decoder in `radar-decode` from mainnet captures (ADR 0001, ADR
0009); the recorder's extraction and the outcomes pass for that program's
events; PDA derivations and instruction builders in a sibling of
`radar-pumpfun`; the cost bands re-measured — 0019's are pump.fun's — so the
bar is per venue; a legacy-transaction route the signer can read (ADR 0003;
GOAL.md records that Raydium and Whirlpool already route legacy).

Does not change: the types (`Amount`, `Slot`, `AsOf`), the store's schema,
the kernel, the signer, the sheet boundary, the gate, the nine rules. Trading
stays closed on every venue until E3 clears the bar on that venue's own cost
bands. That is GOAL.md's sentence, and it is Josh's.

### 9.3 A second chain

Robinhood Chain is an Arbitrum L2: a different transaction encoding, a
different RPC, no `getSignaturesForAddress`, a different signer. Nothing in
this repository below the sheet boundary carries over except the ideas. Out
of scope for this design and recorded as a fact (V7). "USDC-backed" in
Josh's question most likely refers to pump.fun's own reported USDC pairing
(1.5), which is A-5's capture-or-refuse item.

---

## 10. What we stop, freeze or do not start

| what | action | why |
|---|---|---|
| the trading lane | **frozen** | the bar and the edge; E3 is the only instrument |
| the custody lane; billing | **frozen** | J10; nothing to bill |
| AI mode for trading | **shadow after A-1**, never authority (V2) | held to the same bar |
| `radar-graph` thresholds | **re-derived in A-2** | a measurement changed, which is 0007 §10's own condition |
| the x402 client | **not built** | nothing in A or B needs it |
| a new crate for E1 or E3 | **no** | a document that compiles; they live where their callers are |
| a Python dependency for E3 | **no** | the probes promise stdlib-only; the harness is Rust over `Reader` |
| fetching a metadata URI; any social data | **no** | rule 4; a stranger's mint must not choose Radar's requests |
| a second chain | **no** | V7 |
| a per-request store scan on any public route or on D1 | **no** | 8.2 |
| a score, a verdict word, "right" or "wrong" | **no** | GOAL.md; the forbidden list grows by two |
| `docs/plans/` | **kept; the date dropped** (V6) | the thing is read; the test measured the wrong question |

---

## 11. Where this is weakest

Said plainly, because a document that only argues for itself is not worth
much.

1. **E3 will probably find nothing.** 0007 §12 said it and it is still my
   prior. The harness is worth building once because a null from a protocol
   is a result, and because every later fact runs through it.
2. **The strongest predictor in the outside literature is one Radar refuses
   by decision** — a Telegram channel at launch, 8.94× in one study. The
   refusal is right for the product, and it costs exactly that much.
3. **Contiguity rests on block order equalling execution order** for a
   bundle. The RPC docs say wire order; a caveat exists for a neighbouring
   method. A capture disposes, and nothing states it before then.
4. **The investigator is a second public reply per thread.** It may read as
   the bot talking to itself. The daily cap is the only defence until
   engagement on second replies is measured.
5. **Nothing here was run.** Plan mode. Every "exists" is a read of the tip,
   and the box's binaries predate the stack, so the first deploy is where the
   next wrong figure turns up — as it did on 2026-09-04, twice.
6. **Calibration is over a summoned sample.** Without the sentence that says
   so, the page is the backtest simulator GOAL.md refuses.
7. **Effort is a guess.** A-1 two to three sessions; A-2 one to two; A-3 one;
   A-4 one to two; A-5 one to two; the investigator two to three;
   calibration one to two; the shadow strategy two. Twelve to fifteen
   sessions, which is what 0007 guessed too.
8. **The stack is unmerged.** If any of #147 to #154 changes before it
   merges, the rows above that cite it are stale on arrival.
9. **Legal.** 7.5 flags three things and clears none.
10. **The funder join is an assumption about a free endpoint's row cap.** If
    it truncates, the index is a selected sample — LEARNINGS 7's shape — and
    must say so rather than count.

---

## 12. Costs, verified where marked

| item | figure | source |
|---|---|---|
| X summoned reply / top-level post | $0.010 / $0.015 | design 0007 §11, 2026-09-03; not re-raised |
| the investigator's second reply | one more reply | same |
| Helius free tier | 1M credits a month, 10 RPS | 2026-09-03 |
| a dossier | 35–150 credits | 2026-09-03 |
| an investigation | at most 600 calls; at most two credits a call is an **assumption** to verify on the pricing page, so at most 1,200 credits | arithmetic |
| ten investigations a day | at most 360,000 credits a month, leaving room for about 6,000 dossiers inside the free tier; above that the meter says so and the Developer tier is $49 (2026-09-03) | arithmetic |
| the shadow strategy | about $2 a day at 960 calls and $0.002 a call — **assumption** | V2 |
| A-2's daily snapshot; A-3's queries | free public endpoint, paced | ADR 0002 |
| the OG image; the deploy timer; the PR template | nothing | — |
| **all-in before viral** | **still under $50 a month** | arithmetic on the above |

---

## 13. Verification, end to end

- **Every item's gate is above**, and plan 0007 carries the commands for
  A-1. Nothing is done without the line that says what proved it and at which
  commit (`docs/plans/README.md`, rule 1).
- **Re-apply the bug** for every rule: the investigator's trigger; the
  budget's refusal when unset; the calibration's rule-9 handling (an unscored
  week is not zero); the shadow strategy's strict parse.
- **Two instruments compared** wherever a number is produced: E1's
  launch-slot trader count against `Decision.launch_recipients` on the mints
  that have both (a bound, not an equality — traders and token accounts are
  different things); A-2's derived band against 0024's on 0024's window;
  contiguity from the store's `tx_index` against `getBlock`'s order on the
  same slot; the deterministic rule's fit-free result against 0017's null.
- **Determinism:** E1 twice, identical bytes; E3 seeded.
- **Mutants** on changed behaviour in CI, one file at a time locally only to
  diagnose a survivor; equivalents recorded with their reason.
- **`repo-conformance`** on every document this creates or changes.
- **The box:** the runbook's deploy check after A-7; `radar brief` green, or
  red naming the drift with a date.

---

## 14. Documents this creates or changes

- **New:** this file; plan 0007; research 0026 (from plan 0007) and 0027
  (A-3); the PR template; the reviewer agent; the deploy timer's three files;
  the OG image.
- **Changed in the same commit as the behaviour:** GOAL.md — the
  rolling-source row and nothing else; `docs/STATE.md` — the prices, the
  switches, the learning loop's result, `Observed<T>`'s caller;
  `deploy/README.md` — the three checklist lines, the verification table, the
  deploy timer; `README.md`'s crate table only if a crate changes (none is
  proposed); ADR 0012's status line when A-2 lands its commitments;
  `docs/plans/README.md` if V6 drops the date. Designs 0001, 0007, 0008 and
  0009 are unchanged: they decay by their own rule and this document does
  not edit history.
- **Memory, outside the repository:** `memory:project_radar.md` and
  `memory:project_radar_remainder_and_vision_prompt.md` — the vision prompt
  has run, design 0010 is the plan, start with plan 0007.

## External references, read 2026-09-05

Claims, not measurements. Let the reference propose and a capture dispose.

- pump.fun, *Fees* — <https://pump.fun/docs/fees>. The curve split and the
  PumpSwap ladder agree with research 0023 and 0028 to the row; the
  charity opt-in is on the page.
- pump.fun's one-change rule for creator fee settings, and the January
  fee-sharing and February Cashback Coins changes — CoinMarketCap Academy,
  <https://coinmarketcap.com/academy/article/pumpdotfun-caps-creator-fee-changes-to-one-per-token>
  (unverified against the program).
- *Predicting the success of new crypto-tokens: the Pump.fun case*, arXiv
  2602.14860 — <https://arxiv.org/abs/2602.14860>.
- *Pump.fun Graduation Regime Windows: Survival Analysis of 832,941 Token
  Launches and the Social-Presence Effect*, arXiv 2607.02823 —
  <https://arxiv.org/abs/2607.02823>.
- *MELT: A Behavioral Trace Dataset for High-Risk Memecoin Launch Detection*,
  arXiv 2602.13480 — <https://arxiv.org/abs/2602.13480>.
- Jito bundles — <https://www.quicknode.com/guides/solana-development/transactions/jito-bundles>;
  <https://docs.jito.wtf/lowlatencytxnsend/>.
- Solana RPC, `getBlock` — <https://solana.com/docs/rpc/http/getblock>.
- Helius, *Wallet Funding Source* — <https://www.helius.dev/docs/wallet-api/funded-by>
  (the vendor field this design computes locally instead).
- Robinhood Chain (an Arbitrum L2, launched 2026-07-01) — trade press, e.g.
  <https://cryptoticker.io/en/robinhood-chain-memecoins-explained/>.
- López de Prado, *Advances in Financial Machine Learning*, Wiley, 2018,
  chapter 7 (purged k-fold cross-validation and embargo).
