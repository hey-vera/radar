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

- [x] 3. The spend meter
      Every X call and every model call reserves against
      `radar_provider::Meter` before it happens, with the ledger persisted the
      way `radar-serve/src/ledger.rs` already does it.
      No budget configured means the loop starts, says it is unfunded, and
      answers nothing. Rule 8.
      done: `crates/radar-analyst/src/spend.rs`. Two refusals, both rule 8.
      **No budget** is `Budget::CLOSED` and refuses every call while the loop
      still starts, because a bot that exits reads as a broken deploy and one
      reporting `unfunded` is legible. **No prices** means the meter cannot be
      built at all — a default price is a spending decision made by whoever
      wrote the code, and that is what makes the two unsettled X billing figures
      a configuration question rather than a blocker: the operator writes down
      what they were charged and nothing here has an opinion.
      Prices are all-or-nothing; a partial list would meter some calls and let
      others through free, and the total would look like a budget being
      respected. A price that is not a number is absent rather than zero.
      The ledger is written by rename and restored on start, because a process
      under `Restart=always` that forgets its spend can spend the day's budget
      as many times as it can crash.
      `cargo mutants -f spend.rs`: **0 missed**. Its one survivor was real —
      every test settled for exactly what it reserved, so nothing pinned that
      settling *corrects* a reservation, which is the meter's whole point.

- [x] 4. Reply-safe rendering
      `radar-cli` escapes creator-controlled bytes for a **terminal**. A public
      reply needs a different rule: strip direction overrides and zero-width
      characters, cap length, and never let a token name close a fence.
      done: `crates/radar-roast/src/render.rs`, and the finding is the ordering
      rather than the sanitiser.
      **The forbidden and fidelity checks were exploitable.** Both read the
      reply as characters, and a zero-width space renders as nothing — so
      `s<ZWSP>cam` is two tokens to the checker and one word to the reader, and
      a figure split the same way is not a number until it reaches the
      timeline. Cleaning after the checks would assemble exactly the statement
      they refused. `render::for_publication` therefore runs **before** both.
      Bidi overrides, zero-width characters and controls go; every script,
      emoji and ordinary character stays, because a reply that mangled a
      Japanese token's name would be wrong in a way nobody asked for. Over-long
      replies are truncated by character rather than lost, since X refuses them
      with a 4xx and a 4xx is never retried.
      Verified by re-applying the bug: moving the call below the checks fails
      all three ordering tests. The fidelity one took two attempts — the first
      used numbers the sheet refuses either way, so it passed with the bug in
      place and proved nothing. It now uses `11<ZWSP>850`, where **both halves
      are authorised and the joined figure is not**.
      `cargo mutants -f render.rs`: 9 of 9 caught.

- [x] 5. The binary, the unit, and the brief check
      `radar-analyst` as a binary beside `radar-follow.service`, with
      `MemoryMax`, `Restart=always`, and its own `EnvironmentFile`.
      `radar brief` gains an `analyst` check reporting cursor age and last
      reply age, `Unknown` when it cannot see — which alarms, per the existing
      convention.
      done, except the 24-hour live dry run, which needs the X account Josh is
      setting up last. Everything that does not need one is finished and proved.
      **The per-mention pipeline moved out of `radar-cli` into
      `radar-analyst::answer`**, so the command and the daemon share one path
      rather than two copies of what the account says. The command is now
      presentation and nothing else.
      The loop lives in `daemon.rs` rather than in `main.rs`, which is four
      lines, so one poll can be driven by a test against a fake platform. That
      is not a testing convenience: the orderings this loop enforces are the
      ones that cost money or credibility when wrong.
      `crates/radar-analyst/tests/one_poll_end_to_end.rs` runs `tick` once
      against a real socket and checks four things the unit tests cannot: the
      **credential reaches the wire**; the cursor takes the largest id from a
      deliberately out-of-order page (1001, 1003, 1002) rather than the last;
      a refused poll **charges nothing and does not advance the cursor**, which
      would lose those questions permanently; and an exhausted budget stops the
      poll before it costs anything.
      A bug found while writing it and worth recording: the first draft settled
      every reservation at **zero**, which hands the whole budget back and makes
      the meter decorative. Settling now uses `Commitment::reserved`.
      `deploy/radar-analyst.service` (`Restart=always`, `MemoryMax=256M`, write
      access to its own directory and nowhere else) and
      `deploy/analyst.env.example`. `radar brief` gains an `analyst` line, and
      it counts **replies rather than log lines** — `publish` writes twice per
      reply, so counting raw lines reports double. Never-run and empty are
      `Unknown`, which alarms; a quiet day is not, because nobody asking is a
      fact about the world rather than a fault.
      `RADAR_X_API_BASE` is the one setting here that defaults to production
      rather than to refusing, and the reason is the same: the default must be
      the outcome that cannot surprise anybody, and for a base URL that is X.

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
