<!-- SPDX-License-Identifier: Apache-2.0 -->
# Plan 0003 — Foundation before the bot

Status: blocked — every item is done except the half of item 5 that is not
        code, which needs one root command from Josh
Branch: fix/foundation-before-the-bot
Base:   192bf63
Planned by: Fable 5.1, plan mode, 2026-09-04 —
[design 0007](../design/0007-the-end-to-end-plan.md) workstream A

## Objective

The things that would bite during a public launch are fixed, and the plan the
next five workstreams follow is in the repository rather than in a chat log.
Afterwards `release-linux` produces a deployable artifact again, every citation
in the documentation resolves, the token decision is recorded as an ADR, and
what is actually running on the VPS is a checked fact rather than two documents
disagreeing.

## Not in scope

- The trading lane. `Policy::CLOSED` is untouched, and nothing here is evidence
  about an edge.
- The custody lane — Privy, Turnkey, `radar-customer`. Frozen either way
  (design 0007 section 10); this plan does not edit it or retire it.
- `radar-graph`'s thresholds. [ADR 0012](../adr/0012-the-launch-block-count-is-recorded-not-the-threshold-retuned.md)
  decided them and nothing here reopens it.
- The X client. That is workstream B, it is gated on two prices only Josh can
  look up, and it gets its own plan.

## Items

- [x] 0. Design 0007 and ADR 0013 into the repository
      done: `cargo test -p repo-conformance` 30 passed at 0f3a9e9.
      GOAL.md's "will not launch one, ever" and design 0001's flywheel changed
      in the same commit, so no two documents disagree about the token.
      Planned files are written `new:path` because the path check correctly
      refused a plan naming twenty-three files nobody has written.
- [x] 1. `release-linux` builds the interface through `just web` (A1)
      done: `cargo test -p repo-conformance` 32 passed at a3e1dd8; clippy and
      fmt clean. Verified by re-applying the bug — with the three inline npm
      commands put back, `no_workflow_runs_the_node_toolchain_itself` names all
      three and fails.
- [x] 2. LEARNINGS 29 written, and citations checked (A4)
      done: 33 passed at 05f8a8f. Verified by removing entry 29 and confirming
      the check names `README.md`.
- [x] 3. The briefing corrected (A5)
      done: 33 passed. Row 6 of design 0006's table said
      `fix/interface-truth-repairs` was unmerged; it merged as #105 on
      2026-09-03, the day before that table was written. Section 2's account of
      what runs on the VPS now records that `deploy/README.md` disagrees with
      it, with the two commands that settle it, rather than asserting either.
- [x] 4. Verify what is running on the VPS, and make the two documents agree (A2)
      done: checked on the box 2026-09-04 05:29 UTC with
      `systemctl list-unit-files "radar*"`, `pgrep -ax`, `crontab -l` and
      `radar brief`. **`deploy/README.md` was right and design 0006 was wrong**:
      three systemd units, all enabled and active, not two `nohup` processes.
      Design 0006 section 2 now carries the checked table and the store's
      figures with the timestamp they were read at.
      Two things nothing had recorded: a **second** cron at `37 * * * *`
      running `radar consider --cap 40 --record`, and the fact that the
      coordination check has been sitting at `[WARN]` — ADR 0012's predicted
      drift, firing to a terminal nobody reads. `Warn` deliberately does not
      alarm ("worth a look, not worth waking anyone"), so the summary line is
      consistent rather than contradictory.
      Production is healthy: ingestion 5m behind, 515,245 launches, 44,741
      graduations, 1,436,409 outcomes, 8,477 decisions, 38% disk, load 0.38.
- [ ] 5. An alert channel that reaches somebody (A3)
      the code half is **done**; the file itself needs one root command from
      Josh, because `/etc/radar` is root-owned and guardian's NOPASSWD list
      does not cover writing there.
      Confirmed on the box: `/etc/radar/` contains `radar.env` only. There is
      no `alert.env`, so `RADAR_ALERT_WEBHOOK` has never been set and the only
      detection mechanism for a dead recorder is a person happening to look.
      **Two bugs found in the delivery path, and neither had ever run.** The
      body was `{"text": ...}`, which is Slack's field — **Discord answers that
      with a 400**, so choosing Discord would have produced one "POST failed"
      journal line and nothing else. And the shell escaping of quotes and
      backslashes did not survive the trip through `sed` and `curl -d`: the
      message field arrived **empty**, which from outside looks exactly like a
      delivered alert. Both found by pointing the real script at a local
      listener and reading what arrived.
      Fixed: the body carries `text` and `content` together, and the two
      characters that can break JSON are removed rather than escaped, so the
      payload cannot be malformed by its own contents. Four destinations are
      supported — Telegram (`RADAR_ALERT_FORMAT=telegram` plus a chat id),
      ntfy.sh (`text`), Discord and Slack (the default). Telegram needs a
      second value no webhook URL carries, so a missing chat id sends nothing
      and says why rather than POSTing a body Telegram answers with a 400.
      Verified by building each body from the script's own transform against
      output containing quotes, a backslash, tabs and newlines, and parsing it:
      all four branches correct, both JSON bodies parse, and the no-chat-id
      case issues no request at all. `deploy/alert.env.example` and the runbook
      carry the setup for each.
      next: Josh runs the three commands in `deploy/README.md` under
      "Where a failure goes".
- [x] 6. Dependabot triage (A6)
      done: fourteen pull requests became **four**, one per thing that actually
      had to be decided, and two findings came out of it that merging on green
      would have missed.
      **arrow/parquet (#122, replacing #66 and #68) failed because they were
      two.** Bumped apart the tree holds two `RecordBatch` types; together at
      59.3.0 it compiles with no source change. The real risk was never the
      compile: every existing test writes and reads with the same `parquet`, so
      none could see the library's own output format change, while the recorder
      holds 345MB written by 56. Two real production partitions are now
      fixtures and `files_written_by_an_older_parquet_are_still_readable` reads
      them; truncating one fails the test, restoring it passes.
      **vite 8 does not work and the peer range says it does (#124).**
      `@tailwindcss/vite@4.3.3` declares `peer vite: "...|| ^8"` and still calls
      `transformWithEsbuild`, which vite 8 removed. Resolution is clean and only
      the build knows — AGENTS.md rule 1's "a reference proposes, a capture
      disposes", in a package manifest. #67 and #69 closed with that reason
      rather than left looking mergeable; typescript 7, vitest 4 and jsdom 30
      landed.
      **The five action bumps (#123) verify themselves** — every job is checked
      out and tooled by the actions being bumped.
      **sha2 0.11 and ed25519-dalek 3 (#125) were read, not ticked.** `sha2`
      derives every PDA and `ed25519-dalek` is the signing key. What establishes
      safety is that the PDA tests assert derived addresses against mainnet
      captures, over every captured mint — a digest changed by one bit fails
      them. Workspace 1,477 passed, unchanged, which is the point.
      triaged 2026-09-04 by reading every one of the fourteen; **the batch is
      not uniform and the grouping in the plan was wrong** — Dependabot opened
      all fourteen individually because every one of them is a major.

      **Eleven are green** on all thirteen checks: the four GitHub Actions
      bumps plus `taiki-e/install-action`, and `vite` 8, `jsdom` 30, `sha2`
      0.11, `vitest` 4, `typescript` 7, `ed25519-dalek` 3.

      **Three are not, and each fails for its own reason:**
      - `arrow` #68 and `parquet` #66, 56.2.1 to 59.2.0 — `build`, `lint`,
        `msrv` and `tests` all fail, and it is a real API break rather than a
        lockfile problem: `crates/radar-store/src/reader.rs:194-196` no longer
        converts into `StoreError` and `RecordBatch` changed shape. These two
        move together, need code, and need `older_files_still_read.rs` run
        against a store written by the old version — the format is the point.
      - `@vitejs/plugin-react` #67 — `web` fails on `ERESOLVE`: version 6
        wants vite 8 and the tree has vite 7. **It is blocked on #69, not
        broken.** Merge #69 first and this goes green on a rebase.

      **Order to merge, and why it is not "all the green ones":** the four
      Actions bumps touch `.github/workflows/release-linux.yml`, which this
      plan's own item 1 rewrote, so they conflict until that has landed.
      `ed25519-dalek` 3 is the crate under `radar-signer` and green CI is not
      the whole answer there — read what changed in verification before
      merging it, because the signer's guarantee is the one thing in this
      repository whose failure is silent.
      next: merge the stack (#117, #118, #119), then Actions, then the web
      chain #69 into #67, then `ed25519-dalek` by hand, then the arrow work as
      its own branch.
- [x] 7. Delete `radar-provider`'s cache, breaker and planner (A8)
      done: `just tests` 1476 passed, `just lint`, `just fmt`,
      `just licence-headers` clean; `cargo test -p repo-conformance` 33 passed.
      **1,350 lines, not the 700 the plan estimated** — the planner lived in
      `lib.rs` and its doctest composed the two deleted modules, so both went
      with them.
      Two things the estimate missed, and both are recorded rather than
      absorbed. AGENTS.md rule 3 cited that cache as the place the watermark
      gate lived *in the type system*; it does not any more, and the rule says
      so instead of describing a deleted module. And that cache was the only
      caller of `radar-asof`'s `Observed<T>` and `LookAhead` outside
      `radar-asof`'s own tests, so those two types now have none — see Q4.
      `MIN_TESTS` 1503 -> 1476: 27 tests went with the code they tested. The
      floor caught the drop, which is the floor working.
- [x] 8. `just orient` (A9)
      done: `just orient` run against this branch and against a branch with no
      runs; `cargo test -p repo-conformance` 33 passed. It prints the branch,
      that branch's last CI result, every plan whose status is not landed or
      abandoned together with its Handback block, the sentence `docs/STATE.md`
      uses to name its own decaying claims, and the size of `target/`.
      Every line is read from the file that owns it at the moment it prints, so
      there is no copy here to go stale — which is the failure design 0006's
      own table demonstrated. Design 0006 section 6 now opens with the command
      instead of asking a reader to remember four documents.

## Open questions for Josh

- Q1 (2026-09-04): the Tailscale browser check. — **answered: authenticated.**
  Item 4 is done.
- Q2 (2026-09-04): which alert channel. — **answered, and the answer separated
  two things that were being asked as one.** Josh: a website is the lowest
  friction *for users*; Discord and Telegram are medium friction. Both true,
  and they are different questions. The user-facing surface is a page and it is
  design 0007's workstream C. The *operator alarm* cannot be a page, because
  waiting to be looked at is the property that produced the thirteen-hour
  outage.
  Josh then asked whether Telegram beats ntfy.sh. **It does, and it is the
  recommendation.** ntfy wins only on setup time — about a minute against five.
  Telegram is protected by a bot token rather than by a topic name a stranger
  could guess, and it keeps a searchable history, which is what the question
  after an outage always needs. All four are supported; the choice is one line
  in `/etc/radar/alert.env`.
- Q3: **withdrawn.** Josh, 2026-09-04: the X billing figures get settled at the
  end of the programme. **Do not raise them again** — they gate only the moment
  the bot posts publicly, and every other part of workstream B is buildable and
  testable without them.
- Q4 (2026-09-04): `radar-asof`'s `Observed<T>` and `LookAhead` have no caller
  now that the provider cache is gone. Three honest options: delete them and
  leave `radar-asof` as `AsOf` alone; keep them for the `radar-serve` cache
  work in design 0007 D5, which is the next thing that genuinely needs a
  watermark-gated read; or keep them and say plainly in the crate that they are
  a pattern with no current user. **Recommendation: keep, and say so** — D5 is
  weeks away rather than hypothetical, and this is 40 lines rather than 1,300.
  Deleting them is also the harder change to reverse, because what would be
  lost is the reasoning rather than the code. — unanswered

## Handback

Stopped at: **every item landed.** The plan is done except for the half of item
5 that is not code: `/etc/radar` is root-owned and guardian's NOPASSWD list
does not cover writing there, so the alert file needs one root command from
Josh. Three destinations are supported and Telegram is the recommendation —
`deploy/README.md`, "Where a failure goes", has the commands.
Next action: [plan 0004](0004-the-bot-to-dry-run-complete.md), the bot.
Two things found here and deliberately not acted on, both recorded rather than
carried in anyone's head: the coordination detector is drifting live at 3,500
bps against 580 calibrated, which is a measurement question for the learning
loop; and vite 8 comes back when `@tailwindcss/vite` migrates off
`transformWithEsbuild`.
Do not: open `Policy::CLOSED`, edit the custody lane, or retune `radar-graph`.
All three are in Not in scope above, and each has a document saying why.
Do not delete `Observed<T>` on the strength of Q4 without answering it — an
unused type and a deleted invariant look identical in a diffstat.
