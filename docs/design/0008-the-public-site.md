<!-- SPDX-License-Identifier: Apache-2.0 -->
# Design 0008 - Cabal Hunter, the public site

**Date:** 2026-09-05
**Status:** **accepted by Josh, 2026-09-05.** Planned at `4fa83f9`. It is a
design, so it decays: the unit being executed lives in
[`docs/plans/`](../plans/), and where this file and the repository disagree, the
repository is right.

The three rows in section 3 are decisions Josh took on 2026-09-05. Section 11
says where this document is weakest, which is the part worth reading twice.

---

## 1. Context

Radar has one product that is honest and shippable today: a bot that answers
questions about a coin with measured facts. Josh is setting up its X account
now. It needs somewhere to point.

He asked for three things: **one really good home landing page**, **a
leaderboard of the week's top posts and who summoned them**, and **the prize
pool**. The domain will be `cabalhunter.org`, bought later; the site is built
offline and uploaded when it exists.

The intended outcome: when the bot replies to somebody on X and they click
through, they land on a page that explains what this thing measures, shows
numbers nobody else has, and makes the weekly contest legible enough to enter.

---

## 2. What is true today, checked

Everything here was read off the repository or the production box on 2026-09-04.

**Data that exists and is real right now.** From the creator index rebuilt on the
box at slot 444,374,676, and `docs/research/data/0024-base-rates.json`:

| figure | value |
|---|---|
| launches recorded | 508,814 succeeded (540,401 rows; 31,581 failed on chain) |
| creators watched | 116,752 |
| outcomes measured | 506,991 |
| graduate at all | **2.81%** |
| show almost no life at all | **23.0%** |
| launch block of 1–3 recipients | 70.5% of all launches, and 0.02% of them graduate instantly |
| launch block of 10–13 recipients | 2.1% of launches, and **10.1× the base rate** of instant graduation |
| launch block of exactly 6 | 5.7% of launches, 4.4× the base rate |
| round trip, $20–$200 position | **456 bps** |
| organic graduations | end at a median of **−3,228 bps** (research 0011) |

That last row is the one that keeps the site honest: **graduating is not
winning.** A page that presents graduation as success would be the thing this
product exists to expose.

**Data that does not exist yet, and cannot be faked.**

| wanted | blocked on |
|---|---|
| who summoned the bot, and about which coin | the bot has never run — `data/analyst/replies.jsonl` does not exist |
| engagement scores for the week's posts | the X API's `public_metrics`, which needs the credential Josh is setting up |
| the prize pool balance | the token existing, so there is a `creator_vault` PDA to read |

**The architecture constraints found.**

1. **The whole site is behind Cloudflare Access.** `web/src/App.tsx` hard-codes
   `AUDIENCE = "operator"`, and `access::audience_of` classifies unknown paths as
   `Operator`. `Audience::Public` exists but reaches only `/health`, `/x402/`
   and the login bootstrap. **There is no public surface today.**
2. **The interface is compiled into the Rust binary** (`crates/radar-serve/src/embed.rs`)
   so a deploy is one file, and the box deliberately has no Node runtime.
3. **`radar-serve` decodes ~80 MB of Parquet per request** on `/v1/tokens/{mint}`
   (3.2 s) and `/v1/scoreboard` (1.7 s), on a 2-core box shared with Cortex and
   with the recorder. A viral link must never touch those paths.
4. `web/index.html` has **no Open Graph or Twitter card tags**. Every link the
   bot posts would preview as a bare URL.

---

## 3. Decisions taken

Josh's, 2026-09-05:

| # | Decision | Chosen |
|---|---|---|
| K1 | Where the public pages live | **A separate `site/` app.** The operator console stays untouched behind Access. |
| K2 | What "modern stack" means | **Design layer, same tooling.** Vite/React/TS/Tailwind stay; the modernity is components, motion, typography and link previews. |
| K3 | Public identity | **Cabal Hunter is the public face.** Radar stays the engine name inside the repo. |

K3 is well supported by the data rather than just a nice name: the measured
signal *is* coordination detection, and it is a **refuse** signal. The site's
argument is "here is how to see the cabal before you buy", not "here is what to
buy". That is the only claim these numbers can carry.

**One recommendation I am adding, because it follows from §2 point 3:** host the
site as **static files on Cloudflare Pages**, not on the VPS. A viral spike then
costs nothing and cannot take down the recorder. The dynamic figures arrive as
three small JSON documents fetched from `radar-serve`, cached at the edge.

---

## 4. Architecture

```
  cabalhunter.org                       radar.heyvera.org (unchanged)
  ┌────────────────────────┐            ┌──────────────────────────┐
  │ Cloudflare Pages       │            │ Cloudflare Access        │
  │  site/dist (static)    │            │  radar-serve             │
  │   / home               │            │   web/dist, operator only│
  │   /leaderboard         │            └──────────────────────────┘
  │   /about               │                        ▲
  └───────────┬────────────┘                        │ three public,
              │  fetch, cached at the edge          │ tiny, file-backed
              └─────────────────────────────────────┘ endpoints
```

**Three public endpoints, and they read published JSON files — never the store.**

| path | reads | shape |
|---|---|---|
| `/v1/public/stats` | `creator-index.json` population + `0024-base-rates.json` | the §2 table |
| `/v1/public/leaderboard` | `data/contest/<week>.json` | this week's ranked entries |
| `/v1/public/pool` | `data/contest/pool.json` | vault balance and its timestamp |

Each is a small file read, microseconds, on the same pattern
`radar_roast::baserates` and `radar_roast::creator` already use: a published
measurement with the moment it was taken, refused rather than guessed at when
absent.

**Every one of them carries the time it was measured, and the site renders it.**
That is `BaseRates::measured_on` and `STALE_AFTER_DAYS` applied to the interface.

---

## 5. The three surfaces

### 5.1 Home — the whole argument on one page

Real today, top to bottom. No empty states.

1. **The claim.** Most launches on pump.fun are coordinated, and the coordination
   is visible in the launch block before you buy. One sentence, then the number
   that proves it: a launch block with 10–13 recipients is **10.1× more likely**
   to be bought out instantly than the average launch, while the 70.5% of
   launches with 1–3 recipients essentially never are.
2. **What Cabal Hunter has watched.** 508,814 launches, 116,752 creators,
   506,991 outcomes measured. **2.81% ever graduate. 23.0% show almost no life
   at all.** Animated counters, but the figures come from `/v1/public/stats` and
   the measurement time is printed under them.
3. **What it costs you to be wrong.** Entering and leaving a $20–$200 position is
   456 bps before anything moves — and organic graduations end at a median of
   −3,228 bps. **Graduating is not winning.**
4. **How to use it.** Reply to the bot with a mint or a `$TICKER`. Show a real
   reply, verbatim, from the dry-run log.
5. **What it will never say.** No price predictions, no buy calls, no verdict
   words. It reports what it measured and refuses the rest.
6. **The disclosure**, which X policy requires of an automated account: that it
   is automated, who operates it, the correction policy, not financial advice.
   This folds in item B7 of design 0007, which is a launch blocker.

### 5.2 Leaderboard — the week's top posts

**Nothing has run, so today this page is honest about being empty.**

- Columns: rank, who summoned it (X handle), the coin, a link to the bot's reply,
  the score.
- The scoring rule printed **in full** on the page: `3·reposts + 3·quotes +
  1·likes + 1·replies` over the bot's own replies, ties to the earlier post,
  exclusions for the operator, accounts under 30 days old, and anyone the
  admission gate refused that week.
- **The empty state is a sentence, not an empty table.** "No week has run yet —
  the bot has not answered anyone." An empty table implies a week ran and nobody
  engaged, which is the LEARNINGS 5 failure rendered in HTML.
- The same distinction the analyst page already makes between **answered** and
  **published**, because a publisher that is down all night fills the log and
  tells nobody anything.

### 5.3 Prize pool

- One figure: the vault balance in SOL, with the moment it was read and a link to
  the address on a block explorer.
- **Today it says no token exists.** Not `0.00 SOL` — rule 9, and this is the
  direction that flatters: a pool reading zero looks like a contest nobody won,
  not like a contest that has not started.
- Past winners, each with the transaction signature, once there are any.
- The economics stated plainly, per ADR 0013: the only cash flow is pump.fun's
  30 bps creator fee, 100% of it is the prize, the operator holds zero tokens,
  and at $10k of weekly volume the prize is about $3. A memecoin that lies about
  its economics is the thing this bot exists to expose.

---

## 6. The stack, concretely

Keep: **Vite 7, React 19, TypeScript 7, Tailwind 4, vitest 4, wouter.** Already
current; nothing here needs replacing.

Add, and this is the whole of "modern":

| dependency | why | size |
|---|---|---|
| `@radix-ui/react-*` (only what is used) | accessible primitives — focus, keyboard, ARIA — that hand-rolled components get wrong | ~5 kB each |
| `motion` | entrance and counter animation, respecting `prefers-reduced-motion` | ~18 kB |
| `clsx` + `tailwind-merge` | the `cn()` helper the shadcn component pattern needs | ~2 kB |
| `@fontsource-variable/inter` | self-hosted variable font; no Google Fonts request, so no third-party call from a privacy-sensitive audience | subset |

Components are **copied in** shadcn-style under `site/src/ui/`, not installed as
a framework. There is no CLI dependency and no upgrade treadmill.

**Link previews are not optional here.** The bot's whole distribution is people
sharing its replies, so `site/index.html` carries full Open Graph and Twitter
card tags and a committed 1200×630 OG image. Ship a `<noscript>` block carrying
the headline claim and the three numbers, so a link that is scraped or opened
without JavaScript still says something true.

Dark-first, `prefers-color-scheme` respected, `prefers-reduced-motion` respected.

---

## 7. Phases

**Phase 0 — the site, offline, with real numbers.** Fully deliverable now, no
server changes. `site/` scaffolded, home page complete, leaderboard and pool
pages built with their honest empty states. Figures come from a committed
`site/src/fixtures/stats.json` holding the §2 table, so the page is real before
the endpoint exists. This is the thing Josh asked to see.

**Phase 1 — the public endpoints.** `/v1/public/stats`, `/v1/public/leaderboard`,
`/v1/public/pool` on `radar-serve`, each reading a published file. Added to
`Audience::Public` **by exact path, never by prefix** — `access.rs` already warns
in its own comments that a prefix rule is how `/v1/store` ends up in front of the
wrong reader. CORS for the site's origin only. Deliverable now.

**Phase 2 — the leaderboard's data. Blocked on the X credential.** The
`radar-contest` crate (pure: week boundaries, `Entry`, `Score`, the scoring rule
as one function, `Winner`), and the weekly read of `public_metrics` over the
week's reply ids. This is C1 and C2 of design 0007, pulled forward because the
leaderboard needs them.

**Phase 3 — the prize pool's data. Blocked on the token existing.**
`radar_pumpfun::pda::creator_vault` already derives the address and is tested
against an observed derivation; the balance is one `getBalance` on a timer
writing `pool.json`.

Phases 0 and 1 do not depend on 2 and 3, and 2 and 3 cannot invalidate them —
the empty states are the same code path the real data flows through.

---

## 8. Files

**New:**
- `site/` — `package.json`, `vite.config.ts`, `index.html` (with the meta tags),
  `src/main.tsx`, `src/App.tsx`, `src/Home.tsx`, `src/Leaderboard.tsx`,
  `src/Pool.tsx`, `src/About.tsx`, `src/api.ts`, `src/ui/*`, `src/honesty.ts`
- `site/public/og.png`
- `crates/radar-contest/` (Phase 2)

**Changed:**
- `crates/radar-serve/src/access.rs` — three exact paths into `Audience::Public`,
  and the mirror entries in its own table test
- `crates/radar-serve/src/lib.rs`, `api.rs` — the three handlers
- `justfile` — a `site` recipe mirroring `web`
- `.github/workflows/ci.yml` — a `site` job
- `.github/required-checks.txt` — the `site` line, **and the ruleset itself**,
  which is a third place and is Josh's. That file's own header records what
  happens when a check looks like a gate and is not one.
- `README.md`, `docs/STATE.md` — the new surface

**Reused rather than rewritten:** `web/src/honesty.ts`'s rules (`null` not zero
for an empty cohort; unsigned zero), the `Analyst.tsx` answered-versus-published
distinction, `radar_roast::baserates`'s measured-on discipline,
`radar_pumpfun::pda::creator_vault`.

---

## 9. What only Josh can do

1. Buy `cabalhunter.org`.
2. Create the Cloudflare Pages project and point the domain at it.
3. Add the `site` check to the `main` ruleset (`gh api` with admin, or the UI).
4. The X credential — already in flight — which unblocks Phase 2.
5. The token launch, per ADR 0013's constraints, which unblocks Phase 3.

None of 1–3 blocks Phase 0.

---

## 10. Verification

- **`just site`** — typecheck, tests, `npm audit`, build. Mirrors `just web`.
- **The browser, not a screenshot of my intent:** `preview_start` on the `site`
  dev server, then `read_page` for structure, `read_console_messages` for errors,
  `resize_window` at mobile and desktop, and `prefers-color-scheme` both ways.
- **The honest-empty-state test, which is the one worth writing:** render the
  leaderboard with no data and assert the page says no week has run, and render
  the pool with no token and assert it does **not** contain `0.00`. Both have a
  wrong version that looks right, which is `honesty.ts`'s stated reason to exist.
- **Every figure on the home page is checked against its source** — the stats
  fixture against the creator index on the box, the bands against
  `0024-base-rates.json` — because a landing page that overstates by 6.7× is the
  bug I shipped into a reply today and caught only by running it.
- **Link preview**: validate the OG tags against a card validator once the domain
  is live.

---

## 11. Where this is weakest

1. **Two of the three things Josh asked for have no data yet.** The leaderboard
   and the pool will be honest empty pages for as long as the X account and the
   token take. That is the correct behaviour and it will still feel thin. If it
   matters, Phase 2's scoring can run against the dry-run log and show real
   summoners with no scores, which is a partial page rather than an empty one.
2. **A second frontend is a second thing to maintain**, and the two will drift in
   styling. I judged that cheaper than the risk of a misclassified path exposing
   the operator console, but it is a real cost and K1 could be revisited.
3. **The landing page is a React SPA**, so its content is JavaScript-dependent.
   The meta tags and the `<noscript>` block cover link previews and scrapers, but
   a static-rendered page would be strictly better for SEO. Astro was the option
   that bought that; K2 chose otherwise, and this is what that costs.
4. **`0024`'s bands are from a sampled window** — 17,497 launches through a public
   SQL endpoint — while the graduation rates now come from the full store. The
   two agree closely where they overlap, but the band figures on the landing page
   are the weaker measurement and the page should not imply otherwise.
5. **I have not designed the visual identity**, only the structure and the
   constraints. "One really good landing page" is a judgement Josh will have an
   opinion about, and the first build should be treated as a draft to react to.
