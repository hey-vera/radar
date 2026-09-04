<!-- SPDX-License-Identifier: Apache-2.0 -->
# Plan 0004 — The bot, to dry-run complete

Status: proposed
Branch: feat/the-analyst-reads-and-posts
Base:   8caf7b1
Planned by: Opus 5, 2026-09-04

## Objective

`radar-analyst` becomes a **service that can run unattended against the live X
API** — reading mentions, answering them, metering what it spends, and logging
every reply beside the fact sheet it came from — while still posting nothing,
because the publisher it is configured with is the dry run.

Afterwards the only thing between Radar and a public bot is a credential and a
decision, rather than any code.

## Why this is the next unit, and what it deliberately is not

Design 0007 puts workstream B here, and the buildable half of it does not
depend on the two X billing figures. Josh deferred those to the end of the
programme on 2026-09-04; they gate **switching the account on**, not writing
any of this. Every item below is testable against a fixture, a local listener
or a `--dry-run` flag.

`radar analyst --mentions <file.jsonl>` already runs the whole interesting
half: parse, admit, dossier, roast, log. What is missing is the two ends — a
source that is X instead of a file, a sink that is X instead of a printer — and
the operational skin that makes it safe to leave running.

## Not in scope

- **Posting anything publicly.** The live publisher is written and is not
  configured. Turning it on is a separate change with Josh in the loop.
- The contest, the token, the page (workstream C).
- `Policy::CLOSED`, the trading lane, the custody lane.
- `radar-graph`'s thresholds — ADR 0012 decided them, and the live drift below
  is a measurement, not a retune.

## Items

- [ ] 1. `radar-analyst`'s X client, behind the existing `Publisher` trait
      A new `x` module holding `mentions(since_id)` and a `Publisher` that replies.
      `ureq`, matching every other client in the workspace. No credential means
      `DryRun`, which is rule 8 and already the crate's resting state.
      Backoff doubles on 429 and 5xx to a 15 minute ceiling and **never retries
      a 4xx**, because a malformed request retried is the same request.
      done when: a fixture of recorded X responses decodes to the same
      `Mention` shape the JSONL reader produces, and the two share one type.

- [ ] 2. The poll loop, with a cursor that survives a restart
      `since_id` persisted next to the log. Adaptive interval: 60s after a
      poll that returned something, doubling to 300s idle.
      **The log entry is written before the publish call**, so a crash between
      them leaves a logged-but-unposted reply rather than a post nothing
      recorded. That ordering is the whole design and it gets a test.
      done when: killing the process mid-loop and restarting it answers no
      mention twice, proved against a fake source.

- [ ] 3. The spend meter
      Every X call and every model call reserves against
      `radar_provider::Meter` before it happens, with the ledger persisted the
      way `radar-serve/src/ledger.rs` already does it.
      No budget configured means the loop starts, says it is unfunded, and
      answers nothing. Rule 8.
      done when: exhausting the budget is proved to stop replies, by test.

- [ ] 4. Reply-safe rendering
      `radar-cli` escapes creator-controlled bytes for a **terminal**. A public
      reply needs a different rule: strip direction overrides and zero-width
      characters, cap length, and never let a token name close a fence.
      done when: the adversarial fixture gains the cases and they pass.

- [ ] 5. The binary, the unit, and the brief check
      `radar-analyst` as a binary beside `radar-follow.service`, with
      `MemoryMax`, `Restart=always`, and its own `EnvironmentFile`.
      `radar brief` gains an `analyst` check reporting cursor age and last
      reply age, `Unknown` when it cannot see — which alarms, per the existing
      convention.
      done when: the unit runs 24 hours in dry run on the VPS against real
      mentions of a throwaway account, and `radar brief` reports it.

- [ ] 6. The reply log, readable
      `/v1/analyst/replies` (Operator) and a page listing what was asked, the
      fact sheet built, and what would have been posted. Reads the log file,
      never the store.
      done when: 200 dry-run replies are readable in a browser.

## The gate this plan does not decide

Design 0007 J12 sets the 30-day numbers for going public. Nothing here changes
them, and this plan finishing is not the same as the account existing.

## Open questions for Josh

- Q1: the throwaway X account for item 5's 24-hour dry run. Reading mentions
  needs an account and an app even when posting nothing. Is there one, or
  should this wait until the real account exists? **Recommendation: wait** —
  items 1 to 4 and 6 are all provable against fixtures, and item 5's unit can
  be written and left unstarted.

## Handback

Stopped at: not started; this is the plan only.
Next action: item 1.
Do not: configure a live publisher, or touch the trading and custody lanes.
