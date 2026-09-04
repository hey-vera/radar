<!-- SPDX-License-Identifier: Apache-2.0 -->
# Plan 0004 — The bot, to dry-run complete

Status: in progress
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

- [x] 1. `radar-analyst`'s X client, behind the existing `Publisher` trait
      A new `x` module holding `mentions(since_id)` and a `Publisher` that replies.
      `ureq`, matching every other client in the workspace. No credential means
      `DryRun`, which is rule 8 and already the crate's resting state.
      Backoff doubles on 429 and 5xx to a 15 minute ceiling and **never retries
      a 4xx**, because a malformed request retried is the same request.
      done: `crates/radar-analyst/src/x.rs`. `Mention` now lives in the crate
      and **`radar-cli`'s fixture reader uses it**, so the file and the API are
      two sources of one type rather than two types that can drift.
      Request-building and response-reading are pure functions; only `get` and
      `post` touch the wire.
      `just tests` 1502 passed at this commit;
      `cargo mutants -f crates/radar-analyst/src/x.rs`: **32 caught, 0 missed**,
      5 unviable.
      Two things the mutation run changed rather than merely confirmed. It found
      the whole network edge untested -- `get`, `post`, `body_of`, `mentions`
      and the `Publisher` impl could each be replaced with `Ok(...)` with every
      test still passing -- so there is now a one-request `TcpListener` server in
      the test module. It checks what no unit test can: that the
      **Authorization header actually reaches the wire**, that the cursor does,
      that a POST names its parent, and that a 401 becomes `Refused` rather than
      an empty page. And it showed `backoff`'s explicit 4xx arm was
      **behaviourally redundant** with the catch-all, so that arm is gone: two
      arms deciding one thing, and the removable one was not carrying the rule.
      `X::from_env` is recorded in `.cargo/mutants.toml`: its decision is
      `configured`, which is tested directly, and the remaining half cannot be
      tested without `std::env::set_var`, which is `unsafe` in edition 2024
      against a workspace that forbids unsafe.

- [x] 2. The cursor, the interval, and the ordering bug it uncovered
      `since_id` persisted next to the log. Adaptive interval: 60s after a
      poll that returned something, doubling to 300s idle.
      **The log entry is written before the publish call**, so a crash between
      them leaves a logged-but-unposted reply rather than a post nothing
      recorded. That ordering is the whole design and it gets a test.
      done: `crates/radar-analyst/src/poll.rs`, plus a **fix to shipped code**.
      **`publish` replied first and appended afterwards**, so a failed log write
      left a public statement with no record of it. Its own doc said the log was
      written "before the reply is treated as sent", which was true and not the
      guarantee anyone wanted. With `DryRun` as the only publisher nothing could
      observe it, which is how it passed review and a test suite.
      Now: append the intent, reply, append the outcome. The four failure modes
      are asymmetric on purpose and the accepted one is the cheapest — a reply
      whose platform id is lost, still backed by a record holding the fact
      sheet, the slot and the exact text. `log::latest` folds the two records;
      `log::read` stays raw, because an intent with no outcome beside it is an
      interrupted reply and folding that away would hide the case worth seeing.
      Proved by re-applying the bug: `nothing_is_said_before_it_is_recorded`
      uses a publisher that reads the log at the moment it posts, and moving the
      append back below the reply makes it read 0 and fail.
      The cursor is written by rename, so it is either the old id or the new one
      — a half-written cursor is still digits, so it parses, and it points
      somewhere arbitrary. `next_cursor` takes the **largest** id rather than
      the last in the page, compared by length then lexically because these
      outgrew `u64`; the platform returns newest first, so "the last one" would
      re-read the whole page forever.
      `just tests` 1518 passed; `cargo mutants` over `poll.rs`, `publish.rs` and
      `log.rs`: **0 missed**. The one survivor was real — nothing tested a
      mention with a *single* record, which is exactly the interrupted-reply
      case the new ordering creates.
      not done here: the loop that ties these together. It needs the per-mention
      pipeline that currently lives in `radar-cli`, and moving that is item 5's
      binary — where it gains two real callers instead of one and a test.

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
