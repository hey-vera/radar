<!-- SPDX-License-Identifier: Apache-2.0 -->
# Design 0007 - The end-to-end plan

**Date:** 2026-09-04
**Status:** **accepted by Josh, 2026-09-04**, as the ordering for the next
phase. Planned at `192bf63`. It is a plan, so it decays: the workstream being
executed lives in [`docs/plans/`](../plans/), and where this file and the
repository disagree, the repository is right.

The twelve rows in section 3 are decisions Josh has taken, or is assumed to
have taken where he has not said otherwise. Each one that turns into code gets
an ADR of its own; section 3 is not a substitute for one.

**Where this came from.** Written in plan mode as
`workstation:~/.claude/plans/in-simple-terms-the-generic-mountain.md`, and moved here
unchanged in the first commit of `fix/foundation-before-the-bot`. Design 0004
did the same thing and said why: a plan under `~/.claude/plans/` is invisible
to the repository, to a pull request diff, and to `repo-conformance`.

**How to read it.** Section 1 is the honest state. Section 2 is the strategy on
one page. Section 3 is every decision that is Josh's, with a recommendation.
Sections 4-9 are the six workstreams, each with its caller, its gate, its files
and how it is verified. Section 10 is what stops. Section 11 is cost. Section
12 is where this plan is weakest.

**A note on the paths.** A path written `new:crates/…` is a file this plan
proposes and that does not exist yet; `memory:` and `workstation:` name files
outside this repository. The qualifier is not decoration —
`repo-conformance`'s `every_file_path_named_in_the_documentation_exists_and_is_tracked`
rejects a bare path with no tracked file behind it, which is LEARNINGS 1's whole
subject, and a plan is precisely the document most likely to name a file nobody
has written. When the file lands, the prefix comes off in the same commit.

---

## 1. The honest state, checked today

Everything below was verified against the repository at `192bf63` or against
GitHub, except where it says it was not.

**What is good, and unusually so.** The repository has 24 crates, ~1,500 tests
behind a floor of 1,069, mutation testing sharded in CI, a `repo-conformance`
crate with 30 checks, twelve ADRs with supersession notes, 25 research notes
whose filenames state their finding, and a policy file (`AGENTS.md`) held to 400
lines by a check. Most repositories with ten engineers do not have this. **Do
not spend effort re-doing any of it.**

**What is true about the product.**

| claim | state |
|---|---|
| Nothing has ever traded | true. `Policy::CLOSED` ships; no production caller reaches `pipeline::execute` |
| Measured selection edge | **0 bps** across four strata (`0017`); two discriminating strata disagree (−797, +109) |
| The bar to make one trade | **~456 bps** all-in round trip in the $20–200 band (`0022`, `0019`) |
| Two refusal signals are real | creator history ~1.6–3.4×; launch block 1–3 recipients = 70.5% of launches, 0.02% instant graduation (`0024`) |
| Graduation is not profit | organic graduations end at median **−3,228 bps** (`0011`) |
| The public analyst | `radar dossier`, `radar roast`, `radar analyst` run offline. **The X client is not written.** Gated on two prices only the X Developer Console shows |
| The store | 483,629 launches, 14,336 graduations, 1.3M outcomes, 7,543 replayable decisions (`0024-base-rates.json`) |

**Things I found that will bite, in order of how soon.**

1. **The deployable artifact is broken on `main` right now.** `release-linux`
   failed at 03:53 today: it runs `npm audit` inline with no retry, while the
   `web` job runs `just web` *with* the retry that #116 added an hour earlier.
   Same command, two copies, one fixed. `.github/workflows/release-linux.yml:47-51`.
2. **The VPS state cannot be verified from this machine.** Tailscale SSH now
   asks for a browser check (`https://login.tailscale.com/a/…`). Until Josh
   clicks it, nothing about production is checkable, and the repository
   **contradicts itself** about production: `deploy/README.md` §"Verifying"
   says `radar-follow.service` and `radar-serve.service` are enabled and active
   (checked 2026-08-25); `docs/design/0006` §2 says two processes run under
   `setsid nohup`. One of them is wrong.
3. **No alarm channel.** `radar-brief.timer` posts to `RADAR_ALERT_WEBHOOK`
   and it is unset. The recorder has died silently for 13 hours before
   (LEARNINGS 8). A public bot that dies silently is worse.
4. **`README.md:39` cites LEARNINGS 29. There are 28 entries.** A dangling
   reference `repo-conformance` did not catch, in the most-read file.
5. **`docs/design/0006` §4 row 6 says `fix/interface-truth-repairs` is
   unmerged with three blockers.** It merged as #105 *before* 0006 was written.
   The briefing meant to orient a fresh session was stale on its own day.
6. **14 Dependabot PRs open since 2026-08-30**, untouched. Several are majors
   (`arrow`/`parquet` 56→59 touches the store; `ed25519-dalek` 3; `typescript`
   7; `vitest` 4). Unread dependency PRs train the reviewer to merge blind.
7. **Secret scanning, push protection and Dependabot security updates are all
   OFF** on a public repo about to hold an X credential and a wallet key in
   env files. Free to turn on. One accidental commit is unrecoverable.
8. **Zero required reviews.** The ruleset requires 9 checks and 0 approvals.
   Every check is mechanical; nothing reads the diff. Design 0002 §7a named this.
9. **~700 lines with no caller** in `radar-provider` (`cache.rs`, `health.rs`,
   the planner). Flagged three times. Still there.
10. **`radar-serve` decodes ~80 MB of Parquet per request** on `/v1/tokens/{mint}`
    (3.2 s) and `/v1/scoreboard` (1.7 s) against a 500 ms budget, on a 2-core
    box shared with Cortex. Fine private; not fine behind a viral link.

**One direction contradiction, and it is the big one.** `GOAL.md` says *"A
token. Radar will not launch one, ever"* — recorded 2026-09-03 from a plan Josh
approved. Josh's brief today says *"I'll make the token on pump.fun once the X
bot works."* That is a reversal, and it is Josh's to make. §2 and §6 design the
version of it that does the least damage to the thing that makes the bot worth
following, and §3 records it as a decision rather than arguing it again.

**Honest answers to the two direct questions.**

*Can Radar learn patterns and trade better?* The store is already the learning
substrate, and the research so far is that learning done by hand: measure a
signal, build a control, publish. What is missing is not data or reasoning —
it is a **protocol** that stops a stratum being hunted until the numbers look
good (which `GOAL.md` forbids by name) and tests a rule on time it was not
fitted on. §8 builds that protocol once, as a command. My honest expectation is
that it finds strong *refusal* signals and weak *entry* signals on pump.fun,
because the venue is adversarial at the seconds scale and Radar acts ~40
minutes late (`0011`). If it finds an entry edge above 456 bps out-of-sample,
that is the one event that unfreezes trading. If it does not, "lose less" is
still a real product and it is the one Radar can ship honestly today.

*Best-in-class trading tool for humans?* Not by competing on speed with the
execution terminals. Radar's lane is the one nobody else occupies: **before you
click, it shows the round trip at your size, the exit capacity, and the reasons
it would refuse — and afterwards it keeps your record.** That needs no custody,
no edge and no vendor. §7 builds it as the manual-sign mode `GOAL.md` already
describes.

---

## 2. The strategy on one page

Four products, one order, and a research track that runs beside all of them.

```
 week 0        weeks 1–2         weeks 2–5           month 2+
 ┌──────────┐  ┌─────────────┐   ┌───────────────┐   ┌──────────────────┐
 │ A. fix   │  │ B. the bot  │──▶│ C. the token  │   │ D. trading for   │
 │ what     │  │ goes public │   │ + the weekly  │   │ humans: manual   │
 │ bites    │  │             │   │ contest       │   │ sign, journal    │
 └──────────┘  └─────────────┘   └───────────────┘   └──────────────────┘
      │              ▲                  ▲                     ▲
      │              │ Josh: X account, │ Josh: token name,   │ Josh: DNS
      │              │ two prices,      │ launch wallet,      │
      │              │ disclosure page  │ legal read          │
      ▼
 ┌──────────────────────────────────────────────────────────────────────┐
 │ E. the learning loop: `radar edge` walk-forward harness; weekly      │
 │    base-rate refresh with a drift alarm; the organic cohort study.   │
 │    Needs no deploy, no credential, no decision about money.          │
 └──────────────────────────────────────────────────────────────────────┘
 ┌──────────────────────────────────────────────────────────────────────┐
 │ F. repo + workflow: GitHub settings, deploy by tag, reviewer agent,  │
 │    `just orient`, Dependabot triage, branch cleanup.                 │
 └──────────────────────────────────────────────────────────────────────┘
```

**Why this order.** The bot is the only product that is honest, useful and
shippable today, and Josh's own condition for the token is "once the X bot
works". The token's entire value is the bot's reach, so the bot must be good
first. Trading for humans needs a public domain and an audience; both arrive
with B and C. The trading *lane* stays frozen until E moves a number — that is
`GOAL.md`'s ordering and it is arithmetic, not caution.

**Why the token is designed the way §6 says.** The bot's credibility rests on
one measured fact: capital committed *before* a token existed shows up in the
launch block. So the token is launched with **nothing committed before it
existed** — no dev buy, no allocation, no treasury of tokens. Its launch block
has one recipient: the bonding curve. By Radar's own signal it sits in the
cleanest band there is, and the bot can say so because it is true. The only
money that flows to the operator is pump.fun's **creator fee — 30 bps of
volume, read off the on-chain `FeeConfig` (`0023`)** — and 100% of it funds a
public weekly prize for the person whose summoned roast travelled furthest. The
operator holds no tokens, ever, and the bot never mentions the token's price.

**The flywheel, stated so it can be checked:** attention → summons → public
roasts (the record) → the best one wins the week's creator fees → the winner
and the prize are on chain and in a reply → more attention. Every arrow is a
public fact. Nothing in it needs anyone to trust Josh.

**What "low-cost" means in numbers.** Under $50/month all-in until it goes
viral: Helius free tier, X pay-per-use at ~$0.01 a reply, a small model for
the voice pass at fractions of a cent. The token launch costs about 0.02 SOL
with no dev buy. §11 has the table.

---

## 3. Decisions that are Josh's

Each row: the decision, the real options in one line each, **my pick**, and
what the plan assumes if Josh says nothing. Money, public surfaces and legal
exposure are not mine to decide.

| # | Decision | Options | My pick | Assumed |
|---|---|---|---|---|
| J1 | **Launch a token at all** (reverses `GOAL.md`) | (a) no token, as recorded; (b) token with the §6 constraints; (c) token with a dev buy | **(b)**. (c) makes the bot its own worst example and I would not build the contest on it | (b) |
| J2 | **Contest cadence** | weekly with rollover under a floor; monthly | **weekly**, floor 0.1 SOL, rollover. Appointments recur; monthly $8 prizes look sad | weekly |
| J3 | **Payout automation at launch** | (a) `radar-payout` signs from a hot creator key on the VPS; (b) Josh pays by hand, bot verifies and posts the tx | **(a)**, with (b) as the tested fallback. Blast radius is one week's fees, not user funds | (a) |
| J4 | **Legal read before the bot posts** | (a) a lawyer answers the two gating questions in plan §10 of the roaster plan; (b) a written disclosure page and go | **(a) if you can get a one-hour consult; (b) is the floor, not the ceiling.** I am not a lawyer and the touting line is the brightest one here | (b), and the page is written either way |
| J5 | **X read model** | poll mentions every 60 s; adaptive 60 s→5 min when idle | **adaptive**. At $0.005/read a fixed 60 s poll is ~$216/mo by itself; adaptive is ~$10–45 | adaptive |
| J6 | **RPC tier** | Helius Free (1M credits, 10 RPS); Developer $49 (50 RPS) | **Free first**. A dossier is 35–150 credits; the admission gate caps replies anyway. Upgrade when the meter says so | Free |
| J7 | **Alert channel** | Discord webhook; Slack; email | **Discord webhook** — one minute to make, free, on your phone | Discord |
| J8 | **Deploy path** | keep manual `scp`; VPS pulls tagged GitHub Releases on a timer | **pull by tag** (§9). No secret in Actions, one `sudo` edit from you | pull by tag |
| J9 | **Delete `radar-provider` cache/breaker/planner** | delete; wire into the X client | **delete**. A 30-line backoff in the poller beats wiring 700 generic lines. Third flag | delete |
| J10 | **The custody lane** (Privy/Turnkey, `radar-customer`) | freeze; retire; continue | **freeze** — no edits either way until manual mode has users. ~2,400 lines for zero customers is sunk; do not add to it | freeze |
| J11 | **Bot voice model** | small fast model via the metered API path | the implementing session loads the `claude-api` skill and picks by current price; set `RADAR_MODEL_*` in env, never in code | metered path |
| J12 | **The 30-day gate for the bot** | numbers set in advance | **200 distinct summoners; 10% of replies get any engagement.** Guesses, written down so continuing is not decided by whoever is most excited | those |

**Manual steps only Josh can do, in order** (each blocks a specific item):

1. Click the Tailscale check link, so production can be verified (blocks A2).
2. DNS `radar.heyvera.org` → then units → then Caddy, in that order
   (`deploy/README.md` §First install; blocks D, C's page).
3. Make a Discord webhook; put it in `/etc/radar/alert.env` (blocks A3).
4. Add `systemctl restart radar-serve|radar-follow|radar-analyst` to
   guardian's NOPASSWD list (blocks A2 and F-deploy).
5. X: developer account on pay-per-use, fund $20 of credit, then **one live
   test post** to settle the two prices and one more question — does an empty
   mentions poll bill? (blocks B going live).
6. Turn on secret scanning + push protection + Dependabot security updates
   (repo Settings → Code security; blocks nothing, do it today).
7. Token: pick the name; create a **fresh** wallet for it; the key goes only
   into `/etc/radar/payout.json` on the VPS, mode 0400, never in the repo
   (blocks C launch).
8. Legal: J4.

---

## 4. Workstream A — fix what bites (week 0, one session)

**Why:** every item is a way the next public moment goes wrong quietly.
**Caller:** `just ci`, `release-linux`, `radar-brief.timer`, the next session.
**Branch:** `fix/foundation-before-the-bot`. Small PRs, one per item, so
`mutants-shards` stays cheap.

| # | Item | Files | Proof |
|---|---|---|---|
| A1 | `release-linux` calls `just web` instead of three inline npm commands. Delete the copy; the justfile is the one place | `.github/workflows/release-linux.yml:47-51` | the next `main` run is green; `grep -c "npm audit" .github/workflows/*.yml` is 0 |
| A2 | **Verify production, then make the two documents agree.** After Josh's Tailscale click: `systemctl list-unit-files "radar*"`, `pgrep -ax radar-backfill`, store size, cursor age, disk. Whichever of `deploy/README.md` and `docs/design/0006` §2 is wrong gets fixed in the same commit | `deploy/README.md`, `docs/design/0006-…md` | the runbook's own commands, pasted with output |
| A3 | Alert channel live: `RADAR_ALERT_WEBHOOK` set; prove it by stopping the recorder for one tick and reading the Discord message | `/etc/radar/alert.env` (VPS), `deploy/radar-brief.sh` | a message arrived; then it cleared |
| A4 | `README.md:39` LEARNINGS 29 → the right entry (it is describing the `radar-cli`→`radar-exec` dependency claim; either write entry 29 or point at 13). **Add one conformance check:** every `LEARNINGS N` reference resolves to an existing heading. Low false-positive rate: the pattern is exact | `README.md`, `crates/repo-conformance/` | the check fails on the current tree, then passes |
| A5 | `docs/design/0006` §4 row 6 → "merged as #105"; §2 per A2 | `docs/design/0006-…md` | conformance green |
| A6 | Dependabot triage in one batch: merge the two minor/patch groups; open one branch per major with `older_files_still_read.rs` run for `arrow`/`parquet`; close what nobody will do with a one-line reason | the 14 PRs | 0 PRs older than a week |
| A7 | Record the token reversal: **ADR 0013 — "A community token exists; Radar the product holds none of it"** with the §6 constraints, and edit `GOAL.md` "What Radar will not become" from "a token" to "a holder of any token it comments on, including its own". Record it as **Josh's decision**, consequence noted once | `docs/adr/0013-…md`, `GOAL.md` | conformance green; `**Status:**` present |
| A8 | Delete `radar-provider/src/cache.rs`, `health.rs` and the planner (J9). Update the README crate row and `docs/STATE.md` "Where to start" in the same commit | `crates/radar-provider/`, `README.md`, `docs/STATE.md` | build green; `the_documented_dependency_claims_are_true` still passes |
| A9 | `just orient` — one recipe that prints: branch, last CI result for it (reuse `scripts/hooks/pre-push` logic), the open `docs/plans/` file's Handback block, `target/` size, and the four decaying claims from `docs/STATE.md`'s header. It makes 0006 §6 a command instead of prose | `justfile`, `scripts/orient.sh` | run it; it prints the right plan file |

**Not in scope for A:** anything in the trading lane; the custody lane;
`radar-graph` thresholds (ADR 0012 decided them).

---

## 5. Workstream B — the bot goes public (weeks 1–2)

**Why:** it is the only product that is honest and shippable today, and
everything else Josh wants depends on its reach.
**What exists:** the whole reply pipeline. `radar-onchain` builds a `Dossier`
from RPC in bounded calls; `radar-roast` builds a `FactSheet`, a rule-based
`Verdict`, a voice pass, and two post-checks (`fidelity.rs`: every numeral is
on the sheet; `forbidden.rs`: no verdict words); `radar-analyst` has the strict
mention parser (`mention.rs` — only a base58 mint or `$TICKER` survives), the
admission `Gate`, the reply `log`, and the `Publisher` trait with one
implementation, `DryRun`. **Everything below is the thin part.**

**Caller:** `radar-analyst` as a **binary** under `new:deploy/radar-analyst.service`
with `MemoryMax=`, beside `radar-follow.service`. A binary's caller is systemd.

| # | Item | Design | Files |
|---|---|---|---|
| B1 | **X client** — `struct X { bearer, user_id }` implementing `Publisher` (POST `/2/tweets` with `reply.in_reply_to_tweet_id`) plus `fn mentions(since_id)` (GET `/2/users/:id/mentions`, `tweet.fields=author_id,public_metrics,conversation_id,referenced_tweets`). `ureq`, like every other client here. Credential from `RADAR_X_BEARER`; unset ⇒ `DryRun` (rule 8). **Backoff on 429/5xx doubles to 15 min and never retries a 4xx**; that is the whole of what J9 deleted, written where it is used | `new:crates/radar-analyst/src/x.rs` |
| B2 | **The poll loop** — adaptive: 60 s while the last poll returned a mention, doubling to 300 s idle (J5). `since_id` persisted to `~/radar/data/analyst/cursor` so a restart does not re-answer. Each mention → `mention::read` → `Gate::admit` → dossier → roast → `log::append` → `publish`. **The log entry is written before the publish call**, so a crash between them is a logged-but-unposted reply, never an unlogged post | `new:crates/radar-analyst/src/main.rs`, `new:loop.rs` |
| B3 | **Spend meter.** Every X call and every model call reserves against `radar_provider::Meter` with a daily USD budget from env; the ledger persists like `radar-serve/src/ledger.rs`. No budget ⇒ the loop starts, logs "unfunded", answers nothing (rule 8) | `new:crates/radar-analyst/src/spend.rs` |
| B4 | **Parent-post read for the address.** When the mention has no mint but is a reply, read the parent once ($0.005), same strict parse. Cap: one parent read per mention | `mention.rs`, `new:x.rs` |
| B5 | **Homoglyph/RTL-safe rendering** for the token name in the reply — `radar-cli/src/main.rs:284` and `roast.rs:98` escape for a *terminal*; a public reply needs a different rule (strip RTL overrides and zero-width characters, cap length) because X renders them | `crates/radar-roast/src/voice.rs` |
| B6 | **The weekly measured post** — a top-level post ($0.015) from the refreshed base rates (E2): launches this week, share in the 1–3 band, median round trip at $50. Cron-shaped, in the same binary | `new:crates/radar-analyst/src/weekly.rs` |
| B7 | **Disclosure** — bio says automated + who runs it (X policy requires it); a `/about` route on the web app: what it measures, what it will never say, the correction policy, "not financial advice", who operates it. Served static | `web/src/About.tsx`, `routes.ts`, `access::audience_of` (Public) |
| B8 | **Unit + runbook.** `new:radar-analyst.service` (Restart=always, `MemoryMax=256M`, `EnvironmentFile=/etc/radar/analyst.env`); a section in `deploy/README.md`; `radar brief` gains an `analyst` check: cursor age and last-reply age, Unknown when unreachable | `new:deploy/radar-analyst.service`, `deploy/README.md`, `crates/radar-cli/src/brief.rs` |
| B9 | **Reply-log viewer** — `/v1/analyst/replies` (Operator) and a page listing what was asked, the sheet, what was posted. Reads the analyst log file, never the store | `crates/radar-serve/src/api.rs`, `web/src/Replies.tsx` |

**Gate to go live (all of them):** Josh's price answers are in `publish.rs`'s
doc comment with the date; 100 dry-run replies read by Josh with the fact
sheets beside them; the adversarial fixture
(`an_adversarial_mention_cannot_change_the_reply.rs`) extended with three X-shaped
cases — a mention whose text is an instruction, a reply chain whose parent
holds an LP mint, a 30-mention burst from one account — all producing a
fact sheet or a refusal; the meter proven by exhausting it (the bot answers
nothing); B7 live.

**Gate to C and D (J12), written before launch:** 30 days; 200 distinct
summoners; 10% of replies with any engagement. Miss it and the token waits.

**Verify:** `just check -p radar-analyst`; the service unit runs 24 h in
`DryRun` on the VPS against live mentions of a throwaway account before the
credential is set; then the first 20 live replies are read by hand.

---

## 6. Workstream C — the token and the weekly contest (weeks 2–5)

**Why:** Josh's priority, and the flywheel that makes attention accrue rather
than decay. Built so every claim the token makes is a fact the bot can verify.

### 6.1 The constraints, which are the design

These go into ADR 0013 (A7). Each answers a recorded objection.

| objection (`GOAL.md`, roaster plan §11) | constraint |
|---|---|
| any allocation *is* a launch-block recipient set | **no dev buy, no allocation, no team or treasury tokens.** Launch block recipients: the bonding curve, and nothing else. Verifiable; the bot states it |
| the operator would hold a token whose price the bot's reach moves | **the operator holds zero tokens, ever.** The only flow is the creator fee (30 bps of volume, SOL). 100% of it is the prize |
| touting | **the bot never mentions the token's price.** It reports the vault balance, the prize, the winner, the tx — facts. It roasts the token on the same schedule and rule as any other coin (design 0001's self-audit, recurring and unexceptional) |
| a contest with paid entry is a lottery | **entry is free.** Anyone who @-mentions the bot is entered. Holding the token is never required to enter or to win |
| the winner has to trust Josh to pay | the vault is a **public on-chain PDA** (`radar_pumpfun::pda::creator_vault`), the scoring rule is published with the tweet ids, the payout is a signed transaction whose signature the bot posts |

**Honest note on what the token is.** Its value is narrative and attention.
The creator fee is the only cash flow and it scales with volume: at $10k/week
of volume the prize is ~$3; at $100k it is ~$30; at $1M it is ~$300. Say this
on the page. A memecoin that lies about its economics is the thing the bot
exists to expose.

### 6.2 The mechanism

```
 week opens ──▶ every summoned roast is an entry (summoner = entrant)
                        │
 week closes ──▶ read public_metrics of the BOT'S OWN replies (owned reads, $0.001)
                        │   score = 3·reposts + 3·quotes + 1·likes + 1·replies
                        │   ties → earlier; exclusions → operator, accounts <30d,
                        │   anyone the admission gate refused that week
                        ▼
              publish results JSON (entries, scores, tweet ids, winner)
                        │
              bot replies to the winner: "you won N SOL — reply with a Solana address"
                        │
 winner replies ──▶ strict base58 parse (reuse mention::read); author_id must match
                        │   7-day claim window; unclaimed rolls into next week
                        ▼
              radar-payout: collect_creator_fee, then transfer to the recorded
              address; policy: recipient == ledger winner, amount ≤ collected,
              once per week id; signs from /etc/radar/payout.json; posts the sig
```

Why score the **bot's reply** and not the summoner's post: it is the cheapest
read (owned, $0.001), it is harder to buy engagement for someone else's tweet,
and it rewards bringing a coin worth roasting. Why the claim is **a tweet, not
a button**: zero new auth surface, the X↔wallet link is public, and the claim
itself is content.

**Gaming, honestly.** Engagement can be bought. The weights, the account-age
floor, the per-week cap and full publication make it visible rather than
impossible. Accept the residual; publish everything; tighten the rule when a
real case appears, and record the change.

### 6.3 What gets built

| # | Item | Design | Files |
|---|---|---|---|
| C1 | **`radar-contest` crate, pure.** Week boundaries (UTC, Monday 00:00), `Entry`, `Score`, the scoring rule as one function, `Winner`, `Claim`, and a JSON ledger type. No network, no clock (the slot/time is an argument, like `radar-risk`) | `crates/radar-contest/` |
| C2 | **Scoring read** in the analyst binary at week close: `GET /2/tweets?ids=…&tweet.fields=public_metrics` over the week's reply ids from the log; writes `~/radar/data/contest/<week>.json` | `new:crates/radar-analyst/src/contest.rs` |
| C3 | **Claim parse** — the winner's reply, same `mention::read`, author check, address recorded in the ledger | `new:contest.rs`, `radar-analyst/src/mention.rs` |
| C4 | **`radar-payout` binary.** Reads the ledger, builds `collect_creator_fee` (discriminator already in `radar-decode/src/pumpfun.rs`; add the instruction builder to `radar-pumpfun/src/instruction.rs`) + a system transfer; checks the three policy lines; signs with the key at `RADAR_PAYOUT_KEY`; submits via direct RPC (rule 7); appends the signature to the ledger. **Its own unit and user**, modelled on `radar-signer@.service` (no network except RPC; key readable by nobody else). It is *not* the trading signer and does not touch `radar-risk` | `crates/radar-payout/`, `new:deploy/radar-payout.service`, `deploy/payout.env.example` |
| C5 | **Manual fallback, tested.** `radar contest pay --week N --dry-run` prints the exact transaction for Josh to sign elsewhere; the bot's verification step (read the tx, check recipient and amount, post the sig) is identical in both paths, so the fallback is exercised by the automated path's own test | `new:crates/radar-cli/src/contest.rs` |
| C6 | **The page.** `/contest`: live vault balance (one RPC call, cached 60 s, `Entry::bytes(as_of)`-style), this week's ranked entries, past winners with tx signatures, the rule in full, the disclosure. Reads the contest JSON, **never the store**. This is design 0001's "Wall" with the leaderboard folded in | `web/src/Contest.tsx`, `routes.ts`, `crates/radar-serve/src/api.rs` |
| C7 | **Launch checklist** in `deploy/README.md` §Token: fresh wallet → key to VPS → create on pump.fun **with no dev buy** → the bot's first post about it is its own fact sheet (one recipient) → page live → `radar brief` gains `contest` and `vault` checks | `deploy/README.md`, `brief.rs` |
| C8 | **Bot rule:** the token's mint is in `RADAR_SELF_MINT`; a roast of it is answered like any other; the weekly post reports vault, prize and winner and **never a price**. `forbidden.rs` gains a check: if the sheet's mint equals `RADAR_SELF_MINT`, any price or market-cap fact is dropped from the sheet before the model sees it | `crates/radar-roast/src/forbidden.rs`, `sheet.rs` |

**Gate to launch:** B's 30-day gate met (J12); ADR 0013 merged; C4's policy
proven by re-applying the bug (wrong recipient refused; second payout for the
same week refused; amount above collected refused); one full week run on a
devnet or a throwaway mint end to end, including the claim tweet and the posted
signature; the page live on `radar.heyvera.org`.

**Gate to v2 (the on-chain claim program Josh described):** weekly fees above
~$1,000 for four consecutive weeks. Below that, an audit costs more than a
year of prizes and a custom program is more attack surface than the hot key it
replaces. Above it, the time-locked claim program is the right next step, and
it is a separate ADR.

**Verify:** `just check -p radar-contest -p radar-payout`; the policy tests
above; a devnet week; `radar brief` shows `contest: ok`, `vault: <balance>`.

---

## 7. Workstream D — trading for humans: manual sign, pre-trade truth, a journal (month 2+)

**Why:** this is the "best in class for humans" claim made honest. No custody,
no edge required, no vendor in the money path. `GOAL.md` already calls it
*Signal mode — Manual*. The web app already has wallet connect
(`web/src/Wallet.tsx`, `siws.ts`) and a token page (`Token.tsx`).

**What Radar does that the execution terminals do not:** before the click, the
**round trip at your size** (`0019`'s bands + the on-chain fee schedule), the
**exit capacity at a stated impact** (`radar-sim`), the **launch-block and
creator facts** (`radar-onchain`), and the **refusal reasons** the kernel
would give. After the click, **your record**: what you saw, what you did, what
happened.

**The line that does not move:** Radar signs nothing here. It builds an
unsigned transaction with `radar-pumpfun` (which already rebuilds a buy that
simulates clean against mainnet), the customer's wallet signs it, and the
customer submits it. `Policy::CLOSED` is untouched because nothing here reaches
the kernel's authorisation path at all.

| # | Item | Design | Files |
|---|---|---|---|
| D1 | **Pre-trade sheet** on `/token/:mint`: size input → round trip in bps and dollars at that size, exit capacity at 1% and at "impact = round trip", the dossier facts, and the reasons `creator_edge` would refuse. Every number carries its source and slot | `web/src/Token.tsx`, `PreTrade.tsx`, `crates/radar-serve/src/api.rs` (`/v1/tokens/{mint}/pretrade?lamports=`) |
| D2 | **Build-only endpoint**: `/v1/tokens/{mint}/build?side=buy&lamports=` returns an unsigned legacy transaction from `radar-pumpfun::transaction` for the connected wallet. Customer audience. No key on the server side of this path, by construction | `api.rs`, `crates/radar-pumpfun/src/transaction.rs` |
| D3 | **Sign in the wallet, submit from the browser.** Wallet-standard `signAndSendTransaction`. Radar records the signature the browser reports and later reads the fill from chain | `web/src/Trade.tsx` |
| D4 | **The journal** — per-wallet: pre-trade sheet as shown, action, signature, fill, and the outcome checkpoints the recorder already measures. Append-only file per wallet under the store's rules (ADR 0006: record only what cannot be recovered). Rendered as the customer's own "decisions" page | `new:crates/radar-store/src/journal.rs`, `web/src/Journal.tsx` |
| D5 | **Store read cost.** D1 and the journal must not decode 80 MB per request. Cache the decoded launch and outcome tables in `radar-serve` behind the watermark (rule 3: `Entry::bytes(as_of)` shape; the cache key includes the watermark). Measured target: `/v1/tokens/{mint}` under 500 ms on the VPS | `crates/radar-serve/src/cache.rs` |
| D6 | **Admission.** `RADAR_CUSTOMER_ACCESS=open` is the public switch and someone types it (rule 8). Wallet sign-in (`siws.rs`) is the identity; no Privy, no Turnkey | `deploy/README.md` |

**Not in scope:** automated signing of any kind; AI mode; venues other than
pump.fun's curve; any edit to `radar-risk`, `radar-signer`, `radar-exec`.

**Gate:** ten trades by Josh through the manual path with the journal showing
each, before the customer switch is flipped.

**Verify:** the pre-trade numbers against `radar cost` and `radar dossier` for
the same mint (two instruments, compared — design 0002 §7b); D2's transaction
simulates against mainnet with `sigVerify: false`; the B5 budget test
(`the_customer_endpoints_meet_their_budget`) passes with the store copy.

---

## 8. Workstream E — the learning loop (continuous, parallel, no deploy)

**Why:** this is the honest version of "Radar learns patterns". It needs no
credential, no deployment and no decision about money, so it runs beside
everything else from day one.

| # | Item | Design | Files |
|---|---|---|---|
| E1 | **`radar features`** — one deterministic, watermark-gated pass over the store to a Parquet feature table, one row per mint at decision time T (launch + 40 min, matching `creator_edge`; a `--at 5m` variant). Features: launch-block recipient count (recorded going forward per ADR 0012 — **only launches after 2026-09-03 have it**; say so), dev-buy lamports, creator prior launches / organic graduations / cadence, curve progress at T, buyers and volume in the first N slots, repeated-metadata flag (`0013`). Labels: forward return T→6h and T→24h, **net of the round trip for the band the position would sit in** | `new:crates/radar-research/src/features.rs`, `radar-cli` |
| E2 | **Weekly base-rate refresh with a drift alarm.** `radar baserates --out docs/research/data/<date>.json` on `new:radar-baserates.timer`; alarm when a band's mode or `fires_on` moves past a published tolerance — **and alarm differently when the run did not happen** (LEARNINGS 5, 21, 26). Feeds B6's weekly post | `new:crates/radar-research/src/baserates.rs`, `deploy/radar-baserates.{service,timer}` |
| E3 | **`radar edge` — the walk-forward protocol.** Time-ordered folds; fit on `[t0,t1)`, test on `[t1,t2)`; report the top-decile stratum's edge in bps with a Wilson interval per fold; **a stratum counts only if it holds on two non-overlapping test folds.** The modelling itself is a pinned Python script under `scripts/probe/` (where every research number already comes from) reading E1's Parquet; the promotion of any rule into `radar-strategy` is Rust and tested | `new:crates/radar-cli/src/edge.rs`, `scripts/probe/edge.py`, `docs/research/0026-…md` |
| E4 | **The organic-cohort study** `0011` asked for: organic graduations, first 24 h after graduation on the AMM, priced both sides the same way (`0016`'s lesson). It is the one cohort that clears costs twice as often and is not structurally spoken for | `docs/research/0027-…md`, `docs/research/queries/0027-….sql` |
| E5 | **Re-run `0007` weekly** as `0007` itself asks, from E2's timer, watching the prior-coverage line | same timer |

**The un-freeze condition, unchanged:** an edge ≥ 456 bps in a stratum Radar
can size into, on folds it was not fitted on, twice. Nothing else opens
`Policy::CLOSED`. If E3 finds it, the next document is an ADR about opening
the lane with capital at Canary, and it is Josh's.

**Verify:** E1 is deterministic (`radar replay`'s `NotDeterministic` standard:
two runs, identical bytes); E3's protocol is verified by **planting a leak** —
a feature that peeks at the label — and confirming the fold design reports it
as a fold-1 miracle that dies on fold 2.

---

## 9. Workstream F — the repository and the workflow ("GitHub magic", sessions)

**Why:** Josh asked for a repo that keeps future sessions on track and does not
bite. Most of this is settings and ten-line scripts. None of it adds a check
with a false-positive problem.

| # | Item | Why | Files / where |
|---|---|---|---|
| F1 | **Secret scanning + push protection + Dependabot security updates ON** | a creator key or X bearer in a commit is unrecoverable on a public repo | repo Settings → Code security (Josh, or `gh api -X PATCH repos/hey-vera/radar` with admin) |
| F2 | **Deploy by tag, pulled by the box.** `release-linux` also runs on `v*` tags and attaches the binaries + `BUILD-INFO.txt` to a **GitHub Release**. On the VPS, `new:radar-deploy.timer` (15 min) fetches the latest release, verifies sha256 against `BUILD-INFO.txt`, installs by rename, restarts the units. No secret in Actions (the deliberate constraint in `release-linux.yml`'s header stands); one NOPASSWD line from Josh. Tag `v0.1.0` at the bot's launch | `.github/workflows/release-linux.yml`, `deploy/radar-deploy.{sh,service,timer}`, `deploy/README.md` |
| F3 | **Reviewer agent** (design 0002 §7a): `new:.claude/agents/reviewer.md`, a cheaper model, given exactly the plan file and `git diff origin/main`, asked "does this diff do what the plan says, and nothing else". Run before a PR leaves draft. It is the only review the ruleset does not require | `new:.claude/agents/reviewer.md`, AGENTS.md §8 one sentence |
| F4 | **PR template**, five lines: plan file, what command proved it, what the diff deliberately does not do, docs changed in the same commit, mutants result | `new:.github/pull_request_template.md` |
| F5 | **Branch hygiene**: 116 remote branches with `delete_branch_on_merge` on means most predate it. One command: delete remote branches whose PR is merged; leave the rest | `gh pr list --state merged --json headRefName` |
| F6 | **Session protocol, mechanical where it can be.** Start: `just orient` (A9). End: the Handback block in `docs/plans/NNNN`. **The kill date stands**: on 2026-09-17, if handbacks were written and not read, delete `docs/plans/` — and note this plan tells the next session to read one, which makes the test soft; the honest test is a session that was not told | `justfile`, `docs/plans/README.md` |
| F7 | **`LEARNINGS.md` growth rule**: a new entry names a mechanical catch or is a one-paragraph habit entry — the index already enforces the line; add the length to the conformance check for new entries only | `crates/repo-conformance/` |
| F8 | **Version and tags.** Workspace stays `0.0.1` today with no tags. Bump to `0.1.0` at B launch; every deploy is a tag | `Cargo.toml`, F2 |

**Not proposed, and why:** a merge queue (one author, squash-only, `strict:
false` — nothing to queue); required approvals (there is one human; F3 is the
review); CODEOWNERS (one owner); a mutation-testing floor (design 0004 rejected
tuning checks into noise).

---

## 10. What we stop, freeze or delete

| what | action | why |
|---|---|---|
| the trading lane (`Policy::CLOSED`, `pipeline::execute`, the submitter) | **frozen** | the bar is 456 and the edge is 0; E3 is the only thing that changes this |
| the custody lane (Privy, Turnkey, `radar-customer`, ADRs 0005/0007/0011) | **frozen, no edits** (J10) | ~2,400 lines for zero customers; manual mode (D) needs none of it |
| billing (Stripe, ADR 0010) | **frozen** | nothing to bill for until D has users |
| `radar-provider` cache / breaker / planner | **delete** (A8, J9) | third flag; the one named caller needs 30 lines, not 700 |
| the x402 client | **not built** | a funded hot wallet spending at a stranger's request; nothing in B–D needs it. The x402 *server* stays off until someone asks to pay |
| AI mode for trading | **deferred** | it is held to the same bar and the bar is not met |
| `radar-graph` thresholds | **leave** | ADR 0012 decided: record the count, do not retune |
| `docs/plans/` | **kill date stands** (F6) | |

---

## 11. Costs, verified where marked

| item | figure | source |
|---|---|---|
| X summoned reply | $0.010 | docs.x.com, verified 2026-09-03 |
| X post read / owned read | $0.005 / $0.001 | same — **which one mentions bill as is Josh's step 5** |
| X top-level post (weekly) | $0.015 | same |
| X rate limits | mentions 450/15 min; posts 10,000/24 h | same |
| Helius Free | 1M credits/mo, 10 RPS; dossier 35–150 credits ⇒ 6k–28k dossiers/mo | verified 2026-09-03 |
| Helius Developer (later) | $49/mo, 50 RPS | same |
| model voice pass | fractions of a cent per reply on a small model | to verify via the `claude-api` skill when implementing |
| pump.fun creator fee | **30 bps of curve volume** to the creator | on-chain `FeeConfig`, `0023` |
| token creation | ~0.02 SOL, no dev buy | pump.fun; verify at launch |
| VPS | already paid; 2 cores / 3.8 GiB shared | measured 2026-08-18 |
| **all-in before viral** | **under $50/month** | arithmetic on the above |

The two X figures Josh checks in step 5 move the monthly X line by 5× and the
page flywheel by 20×. Nothing else in this table can surprise us by more than
2×.

---

## 12. Where this plan is weakest

Said plainly, because a plan that only argues for itself is not worth much.

1. **I could not see production.** Every claim about the VPS is from documents
   that disagree with each other. A2 is the first thing that runs after Josh
   clicks the Tailscale link, and it may change B8's unit design.
2. **The token reverses a decision Josh approved 36 hours ago**, and the
   version I designed keeps the operator's hands off the token but not off the
   *volume*: creator fees rise when the bot's audience trades it. J4 exists
   because I cannot tell Josh that is fine. The constraints in §6.1 are the
   strongest honest version I can build; they are not a legal opinion.
3. **Demand is a guess.** J12's numbers are invented so that continuing is a
   decision made in advance rather than by excitement. If the bot gets 40
   summoners in 30 days, the token waits and that is the plan working.
4. **The contest can be gamed** for about $5 of bought engagement. §6.2
   accepts that and makes it visible. If the first winner is obviously bought,
   the rule changes and the change is recorded.
5. **E3 will probably find nothing tradable.** That is my honest prior and I
   would rather write it here than discover it as a surprise. The harness is
   still worth building once, because it replaces a series of one-off notes
   with a protocol, and because a null result from a protocol is a result.
6. **Effort.** Roughly: A+F 1–2 sessions; B 2 sessions plus Josh's steps; C
   3–4 sessions plus launch; D 3 sessions; E 2–3 per study. Twelve to fifteen
   sessions. A guess, and the first two workstreams will show whether it is a
   good one.
7. **The 2-core box.** B and C are bounded by the admission gate and by X's
   own limits (~7 replies/min), and their reads are RPC not store, so they
   should fit. D5 is the one item that touches the known CPU pin, and it has a
   measured target for that reason.

---

## 13. Verification, end to end

- **Every workstream's gate is above, with the command.** Nothing is "done"
  without the line that says what proved it and at which commit
  (`docs/plans/README.md` rule 1).
- **`just ci` green before every push; small PRs** so `mutants-shards` tracks
  the diff. `cargo mutants -f <file>` when a file is finished, never wider
  locally (AGENTS.md §8).
- **Re-apply the bug** for every policy: C4's three refusals, B3's meter,
  `forbidden.rs`'s self-mint rule, E3's planted leak.
- **Two instruments compared** wherever a number is produced: D1 against
  `radar cost`; E2 against the committed `0024` snapshot; the contest score
  against a hand count of one week's replies.
- **The 24-hour dry run** for B on the VPS before any credential exists.
- **`repo-conformance` will fail** a new crate that is not a workspace member,
  an untracked document, an ADR cited by a number that does not exist, a
  README crate row that drifts, and — after A4 — a LEARNINGS reference that
  does not resolve. Expect it and satisfy it.
- **Machine discipline:** one cargo process at a time, `-p <crate>`, no
  release builds locally, watch `target/` (`just disk`).

---

## 14. Documents this creates or changes

- **New:** ADR 0013 (the community token and what Radar holds of it);
  `docs/design/0007` (this plan, moved in); `docs/plans/0003` (A + B as the
  first unit); research 0026 (the walk-forward protocol) and 0027 (the organic
  cohort); `deploy/README.md` sections for the analyst, the payout, the token
  launch, and deploy-by-tag.
- **Changed in the same commit as the behaviour:** `GOAL.md` (the token line;
  the manual-mode paragraph gains "built"); `docs/STATE.md` (provider deletion,
  the analyst's state, the contest); `README.md` (LEARNINGS 29; crate table
  rows for `radar-contest`, `radar-payout`; the provider row); `docs/design/0006`
  (§2 after A2, §4 row 6); `.github/required-checks.txt` if F2 adds a job;
  `AGENTS.md` one sentence for F3, staying under 400.
- **Memory (outside the repo):** update `memory:project_radar.md` and
  `memory:project_radar_roaster_direction.md` — the "no token, ever" line is
  superseded by ADR 0013 the day it merges, and the memory must not keep
  saying the old thing.
