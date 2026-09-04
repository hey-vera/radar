<!-- SPDX-License-Identifier: Apache-2.0 -->
# Design 0006 — Where the next phase starts

**Date:** 2026-09-04
**Status:** **a briefing, not a proposal.** It decides nothing and recommends no
implementation. It exists so a session that has never seen this repository can
start planning in one read instead of re-deriving the state, and so the things
that are waiting on Josh are in one place rather than scattered across seven
documents. Written at `e6f5b7e`, immediately after the audit of
[`0004`](0004-documented-versus-enforced.md) landed.

Everything here is checkable, and the four claims most likely to go stale are the
ones `GOAL.md` already names: the measured edge, the bar, which venues are
recorded, and whether anything has traded. If this file disagrees with the
repository, **the repository is right and this file is a bug**.

---

## 1. The one fact that shapes every option

**The bar is 456 bps. The measured edge is 0.**

A strategy must clear roughly 456 bps of expected edge before a single trade is
worth making — the all-in round trip for a position in the `$20–$200` band
([`0022`](../research/0022-capacity-was-a-budget-not-a-ceiling.md),
[`0019`](../research/0019-the-round-trip-is-not-one-number.md)). Across 4,374
recorded decisions the measured edge is negative
([`0017`](../research/0017-a-control-that-could-have-been-traded.md)).

This is not a discouraging footnote. It is the thing that decides what the next
phase should be, because **no amount of execution work matters until a signal
clears the bar**, and two signals have been measured that do not:

- **Creator history predicts a roughly 2x future organic-graduation rate**, on
  non-overlapping intervals, ~2 days, one regime
  ([`0007`](../research/0007-does-creator-history-predict-anything.md)).
- **Coordination is visible in the launch block**, and the reliable half is a
  **refusal**: one to three recipients is 70.5% of all launches and graduates
  instantly 0.02% of the time
  ([`0024`](../research/0024-the-spike-became-a-hump-and-the-signal-moved.md)).

Neither is a reason to buy. `0011` stands: **graduation predicts volatility, not
profit**, at a median −3,228 bps.

## 2. What is actually running

Two processes on the guardian VPS under `setsid nohup`, plus an hourly cron:
`radar-backfill --follow` recording launches ~5 minutes behind chain,
`radar-serve` on `127.0.0.1:8402`, and the outcome pass at `17 * * * *`.

**Not verified from here, and something else in the repository says otherwise.**
[`deploy/README.md`](../../deploy/README.md) records `radar-serve.service`,
`radar-follow.service` and `radar-brief.timer` as enabled and active, checked
2026-08-25 — which is systemd, not `setsid nohup`. Both accounts cannot be
right, and neither was checked on the day this was written.

So resolve it rather than picking one, and the runbook already gives the
command:

```bash
ssh guardian-vps-tail 'systemctl list-unit-files "radar*" --no-pager'
ssh guardian-vps-tail 'pgrep -ax radar-backfill; pgrep -ax radar-serve'
```

Whichever document is wrong gets fixed in the same change as the answer. The
follow recorder has exited silently before — [LEARNINGS](../../LEARNINGS.md) 8
— so an absent process is a plausible state rather than a surprising one, and
a supervised unit and an unsupervised process fail very differently.

An attempt on 2026-09-04 got as far as Tailscale asking for a browser check, so
the answer is one click away and not in this file.

**The trading lane is shut.** `Policy::CLOSED` ships and has never been handed a
real proposal. `radar-cli` reaches `radar_exec::route` for `radar route`, which
prints unsigned bytes; nothing in production reaches `pipeline::execute`, the
signer client or the submitter, and `repo-conformance` pins that.

## 3. The three candidate directions, with what each is waiting on

These are not ranked. Ranking them is the next session's job, and the evidence
for each is above rather than asserted here.

### a. The public analyst — closest to shipping, and it is not trading

`radar dossier`, `radar roast` and `radar analyst` all run today, from a mentions
file, holding no credential. Phases 1–3 landed in
[#112](https://github.com/hey-vera/radar/pull/112).

**The X client is deliberately not written**, and it is gated on two numbers
neither of which anyone has looked up: whether the mentions endpoint bills at
$0.001 or $0.005 (5x on the dominant read line), and whether a URL-bearing reply
is $0.010 or $0.200 (20x). AGENTS.md §2 requires a price be verified before a
decision turns on it; ADR 0011 is the standing example of what happens otherwise.

**This is the shortest path to something public that does not touch money.**

### b. Research toward an edge — the only path that unblocks trading

`0007` and `0024` are each one window. `0007` says so in its own words and asks
to be re-run weekly; the confound nobody has touched is **time**, and two days is
one regime.

The cheapest next measurement is the one `0007` names: re-run it on a wider
window and see whether the direction holds in all three frequency bands as they
fill. The store has 483,629 launches and 14,336 graduations, so the data likely
exists already.

**Nothing here needs a deployment, a credential or a decision about money.**

### c. Execution and venues — deliberately last

`GOAL.md` is explicit: broadening venues is worth doing *after* an edge exists,
because venues make a working edge bigger and do not create one. Established
tokens on Raydium and Whirlpool already route in a form the signer can read.

## 4. What is waiting on Josh, in one place

| | What | Where |
|---|---|---|
| 1 | **DNS for `radar.heyvera.org`**, then the two systemd units, then the Caddy block — **in that order**. Adding Caddy before DNS resolves burns ACME failures against a Let's Encrypt account shared with two other sites. | [`deploy/README.md`](../../deploy/README.md) §75 |
| 2 | **The two clawapis billing figures** that gate the X client. | §3a above |
| 3 | **`docs/research/0002`** — eleven feature requests to clawapis, written and never sent. Items 1 and 3 would let Radar buy all its data pay-per-call. | [`0002`](../research/0002-clawapis-feature-requests.md) |
| 4 | **`radar-graph`'s thresholds** — the gate is tuned to numbers `0024` withdrew, and its decay detector is calibrated so it cannot fire. Two of five proposals change what Radar refuses. | [`0005`](0005-what-radar-graph-should-refuse-after-0024.md) |
| 5 | ~~`radar-provider`'s `Cache`, `Breaker` and planner — 733 of 1,897 lines with no caller. Wire or delete.~~ **Deleted 2026-09-04**, with the planner and the doctest that composed them: 1,300 lines, not 733. | [`docs/STATE.md`](../STATE.md) "Where to start" |
| 6 | ~~`fix/interface-truth-repairs` — 19 commits pushed, unmerged, three blockers.~~ **Merged as [#105](https://github.com/hey-vera/radar/pull/105) on 2026-09-03, `e1b82d7`** — the day *before* this table was written. Corrected 2026-09-04. | — |
| 7 | **x402 paid surface is OFF.** Implemented and verified; needs `RADAR_X402_PAY_TO` and `RADAR_X402_FACILITATOR`. | `crates/radar-serve/src/x402.rs` |

None of 1–7 blocks §3b. That matters: **research can proceed while every one of
these is outstanding.**

## 5. What the repository will now refuse

Added on 2026-09-03/04, and worth knowing before writing anything:

- `repo-conformance` is **30 checks**. It pins the documented dependency claims,
  requires `**Status:**` on every numbered document, refuses an untracked
  markdown file, cross-checks `required-checks.txt` against the justfile and the
  workflow, allows one owning document per test file, requires every LEARNINGS
  entry to name what catches a recurrence and to have an index row, and holds
  `AGENTS.md` to **400 lines**.
- `just hooks` installs a `pre-commit` that refuses a commit on `main`, and a
  `pre-push` that **refuses** to push over a CI run in flight
  (`RADAR_PUSH_OVER_CI=1` overrides).
- `just mutants` uses `--timeout 60`, not 300. **If it fires, fix the loop** —
  the number is low because no unbounded cursor scanner remains.
- `docs/plans/` carries a kill date: **delete it by 2026-09-17** if handback
  blocks are being written and not read. That is `0002`'s own condition and it
  is not sentimental.

## 6. Where a fresh session should start

1. `GOAL.md` — what Radar is for. The owner's document.
2. This file, then [`docs/STATE.md`](../STATE.md) for what has been measured.
3. `AGENTS.md` — how to work here. 396 lines, and every one is meant to be
   load-bearing.
4. `docs/plans/` — what the last session was doing and where it stopped.

**Do not** start by reading all 25 research notes. `0017` (the measured edge),
`0022` (the bar), `0011` (graduation predicts volatility, not profit) and `0024`
(the coordination signal, re-measured) are the four that carry the argument.

## 7. Where this document is weakest

It asserts what is running on the VPS from memory rather than from a check, and
says so in §2. It ranks nothing, because ranking is a decision about where the
next month goes and the evidence for the three directions is genuinely close. And
it is a snapshot: everything in §4 is true on 2026-09-04 and each row is exactly
the kind of claim this repository has watched go stale before.
