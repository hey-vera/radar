---
description: Plan Radar and Cabal Hunter to the vision — close designs 0007–0009 first, then raise the ceiling — as one design document in plan mode.
---

# Plan Radar to the vision

You are planning, not implementing. Produce ONE document,
`new:docs/design/0010-<slug>.md`, in the shape of designs 0007 and 0009: a
status header with the date, the honest state re-verified, the decisions that
are Josh's with a recommendation and an assumed answer for each, the work with
what each item requires and what proves it, what stops, and where it is weakest.
Its first commit moves it into the repository. No code.

## Why this prompt exists

Design 0007 is the plan. Designs 0008 and 0009 extend it. On 2026-09-05 a check
of every row found the foundation half-built and the vision not started; the
table below is that check. A vision built on an unfinished foundation is the
failure design 0004 records, so the document has two parts and Part A comes
first, in the document and in the work:

- **Part A — close the remainder.** Every unblocked row of 0007, 0008 and 0009,
  ordered, each with a gate that says what proves it.
- **Part B — raise the ceiling.** Radar smarter, the bot judging better, and the
  code at the highest tier it can honestly reach. Each defined in numbers below,
  so "better" is a measurement and not a feeling.

## Start here, in this order

1. `AGENTS.md`. Section 3 on direction questions and section 4's nine rules
   govern. Section 5's enforcement ladder is the definition of code quality in
   this repository; use it, do not invent another.
2. `GOAL.md`. What Radar is for, the honest state, and "what working would look
   like" — four steps, in order, none skippable.
3. `docs/STATE.md`. Claims that decay. Re-verify the four it names before
   quoting any.
4. `docs/design/0007-the-end-to-end-plan.md`, then
   `docs/design/0008-the-public-site.md`, then design 0009
   (docs/design/0009-three-loops-and-no-formula.md — on PR #143 if it has not
   merged). Status headers first. 0009 section 3 records six decisions Josh
   made by delegation on 2026-09-05; they are not yours to reopen.
5. `LEARNINGS.md`. The index first, then every habit-only row twice.
6. Research, for the numbers you may use:
   `docs/research/0011-graduation-predicts-volatility-not-profit.md`,
   `docs/research/0017-a-control-that-could-have-been-traded.md`,
   `docs/research/0019-the-round-trip-is-not-one-number.md`,
   `docs/research/0022-capacity-was-a-budget-not-a-ceiling.md`,
   `docs/research/0023-the-fee-is-a-schedule-and-the-published-interface-is-incomplete.md`,
   `docs/research/0024-the-spike-became-a-hump-and-the-signal-moved.md`,
   `docs/research/0025-what-the-evidence-says-about-how-this-repository-is-run.md`.
7. The handbacks in `docs/plans/`, then `just orient`.

## The remainder, as found on 2026-09-05

**Re-check every row against the repository and the box. Do not trust this
table; it is where to look, not what is true.**

| design | row | found | blocked on |
|---|---|---|---|
| 0007 A | fix what bites | done, except the alert file under `/etc/radar` which is root-owned | one root command from Josh |
| 0007 B | the bot goes public | code complete; the unit file exists in `deploy/`; **not installed on the box, no `/etc/radar/analyst.env`**, no 24-hour live dry run, no X-shaped cases in the adversarial fixture, B6's weekly post unwritten | the X credential and Josh's two prices; the install is a root step |
| 0007 C | the token and the contest | only C8, the self-mint rule (PR #145). No `new:crates/radar-contest/`, no `new:crates/radar-payout/` | the credential; J12's 30-day gate; the token |
| 0007 D | manual-sign trading for humans | nothing built | an audience (B and C) |
| 0007 E | the learning loop | E2 done (live base rates in the creator index). E1 features, E3 `radar edge`, E4 the organic cohort, E5 the weekly re-run: **not built, and nothing blocks them** | — |
| 0007 F | repository and workflow | F1 partial (secret scanning and push protection on; Dependabot security updates off), F6 `just orient` done, F5 withdrawn. F2 deploy by tag, F3 reviewer agent, F4 PR template, F8 version and tags: not done. Workspace is 0.0.1 with no tags | — |
| 0008 | the public site | phase 0 merged. Phase 1's three public endpoints and the OG image: not done. Phases 2 and 3 | the credential; the token |
| 0009 | three loops | L1–L6 decided. M6 measure the post-graduation fee and M5 the Telegram publisher: **not built, and unblocked** (M5 needs a bot token, five minutes). M1–M4 | the credential; the launch |
| plans | `docs/plans/` | kill date 2026-09-17 stands: delete it if handbacks are written and not read | evidence |

**Only Josh can do these**, from 0007 section 3, 0008 section 9 and 0009:
the alert file and the analyst env file under `/etc/radar`; a NOPASSWD line
for restarting the radar units; the X developer account, its credit and the
one live test post that settles the two prices; Dependabot security updates
(one click); the token's name and a fresh wallet, gated on J12; the legal read
(J4, and 0009 section 8 has what to hand the lawyer); the `cabalhunter.org`
domain and its Cloudflare Pages project; a Telegram bot token for M5. List them
in the document with what each one unblocks, and put nothing else on his list.

## The question

Two products in one repository, held to one bar. Take each to the highest tier
it can honestly reach, and say what "highest" means as a number that can be
checked.

**Radar smarter** means a measured edge or a measured refusal signal — never a
feeling, never a backtest somebody hunted for. The bar is ~456 bps of expected
edge, out of sample, on two non-overlapping folds (0022; 0007 section 8). The
learning loop is the only instrument that can move it: E1's feature table, E3's
walk-forward protocol with a planted leak to prove it, E4's organic cohort, E5's
drift alarm. Design the protocol so that a null result is a result, and say
what the plan does when it finds nothing — 0007 section 12 item 5 is the
honest prior. AI mode is held to the same bar and never reaches the signer
(rule 1); say what it is, concretely, and what it must never be.

**The bot judging better** means more measured facts on the sheet, each with a
base rate and a date it was measured, and its own calls scored in public —
0009's daily "seven days later" post is the mechanism. Never a verdict word,
never an identity the data cannot carry (0012). Evaluate, for each candidate:
the source, the cost per reply under rule 6, the base rate needed before it may
be stated, and what the fidelity check must then authorise. Candidates from
data Radar has or can read on demand: mint and freeze authority (rule 5's
latch); authority prevalence, recorded and never refused on
(`crates/radar-graph/src/prevalence.rs`, research 0012); repeated metadata
(research 0013); the share of curve trades that are bot-shaped, which an
external study found the strongest predictor of graduation and this repository
has not verified; holder concentration; the creator's funding source, a join
Radar does not have. Candidates to refuse: anything social, anything that
links an address to a person (GOAL.md; ADR 0013's context).

**The highest tier of code** means AGENTS.md section 5 applied with numbers:
what is impossible by type, what is one mechanical check, what is a test, what
is prose, and where a property sits one rung lower than it claims. Mutation
coverage on changed behaviour. Every layer has a named caller; name the ones
that do not, starting with `crates/radar-asof`'s unused types and the frozen
custody lane. The 500 ms budget on public routes (0007 D5;
`crates/radar-serve/src/cache.rs`). Replay determinism. Documents that describe
behaviour changed in the same commit. Two things found on 2026-09-05 are the
pattern to hunt for: an ADR describing a mechanism as enforced that no crate
contained, and a prize figure ten times wrong in four documents at once. Audit
the tree for more of that shape and list what you find, with its rung.

## Facts to use rather than estimate

Every figure carries a date and a source. If you need one that is not here,
measure it or mark it an assumption. This repository has an entry for a figure
that was wrong by 2.7× nine days after it was measured, and another for one
wrong by 10× in four places.

| figure | value | source |
|---|---|---|
| launches recorded, creators, outcomes measured | 508,814; 116,752; 506,991 | creator index, 2026-09-04 |
| graduate at all / instantly / almost no activity | 2.81% / 1.03% / 23.0% | same |
| launch block 1–3 recipients | 70.5% of launches; 0.02% graduate instantly | 0024, sampled 2026-08-25 |
| launch block 10–13 recipients | 2.1% of launches; 10.1× base rate of instant graduation | same, weaker because sampled |
| organic / instant graduations, held to the end | median −3,228 / −5,981 bps | 0011 |
| measured selection edge | 0 bps | 0017 |
| the bar | ~456 bps; ~850 before a position above about $59 | 0022 |
| venue fee on the curve | 125 bps a side; creator 30 bps | 0023 |
| the fee after graduation | **unmeasured**; the venue's own page claims a market-cap schedule | 0009 section 1, M6 |
| X: summoned reply / top-level post | $0.010 / $0.015 | 0007 section 11 |
| Helius free tier | 6,000–28,000 dossiers a month | same |
| all-in before viral | under $50 a month | same |
| store read on `/v1/tokens/{mint}` | ~3.2 s against a 500 ms budget | 0007 section 1 |

## Decided. Do not reopen.

- ADR 0013's six constraints, and design 0009 section 3's L1–L6 as decided by
  delegation on 2026-09-05: 100% of the fee to the prize; no user
  burn-for-access; one contest with the teardown post as the prize; status is
  the prize and money the receipt; Telegram is the free lane and X the record;
  one daily "seven days later" post.
- Trading stays frozen until the bar is cleared out of sample, twice. Nothing in
  the document may unfreeze it; if E3 clears it, that is a separate ADR and
  Josh's.
- The bot is summoned-reply-only. No social or identity scraping. The x402 lane
  never touches the analyst path. Rule 1: model judgement never authorises
  capital.
- pump.fun is the only venue until an edge exists (GOAL.md, direction of
  travel).
- The two X billing figures are settled by one live test post at the end. Do not
  raise them.

## Genuinely open — the document's job

1. The order of Part A, with a gate per item, and which items share a PR.
2. What of Part B is worth building before an edge exists, and what happens to
   the plan when E3 finds nothing.
3. Which new sheet signals are measurable now, which need a new source and at
   what cost, which are refused and why.
4. How the bot's calls are scored in public without a verdict, and what
   "calibration" means for a bot that states measurements rather than opinions.
5. The store's read cost: what the public site, the daily post and D1 need, and
   whether the existing cache is the answer or a second index is.
6. The code audit: what to delete, freeze or harden, and the rung each property
   should sit on.
7. What AI mode is under rule 1 and the bar.
8. Venues after an edge: what changes and what does not.
9. The `docs/plans/` kill date: keep or delete, on the evidence of whether
   handbacks were read.

## Constraints on the answer

- Every number from the repository or marked assumption or external. Say which.
- Name the caller for every layer you propose. A crate nothing depends on is a
  document that compiles (AGENTS.md section 5; LEARNINGS 1, 9, 10).
- Say where the recommendation is weakest, in its own section. It is the section
  that gets read twice.
- Distinguish what is your recommendation from what is already Josh's decision,
  and give an assumed answer for each open one.
- Flag legal exposure; do not give an opinion. 0009 section 8 has the documents.
- Do not propose anything 0007 section 10 stopped unless a measurement changed.
- Small words, short paragraphs, exact paths. Paths that do not exist yet carry
  `new:`; `repo-conformance` rejects a bare path with nothing behind it.
- `**Status:**` within the first fifteen lines, or conformance fails.

## Deliverable

`new:docs/design/0010-<slug>.md` on a branch from `origin/main`, opened as a
PR, with `new:docs/plans/0006-<slug>.md` for the first unit of Part A on the
same branch. A plan under `~/.claude/plans/` is invisible to the repository
and does not count as written down.
