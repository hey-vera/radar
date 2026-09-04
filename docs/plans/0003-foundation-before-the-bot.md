<!-- SPDX-License-Identifier: Apache-2.0 -->
# Plan 0003 — Foundation before the bot

Status: in progress
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
- [ ] 4. Verify what is running on the VPS, and make the two documents agree (A2)
      blocked: Tailscale SSH asks for a browser check —
      `https://login.tailscale.com/a/l4e1b2b828ad11`. One click from Josh, then
      this is ten minutes. See Open questions Q1.
- [ ] 5. An alert channel that reaches somebody (A3)
      blocked on Q2. `radar-brief.timer` has posted to `RADAR_ALERT_WEBHOOK`
      since PR #10 and the variable has never been set, so the only detection
      mechanism for a dead recorder is a person happening to look. It went
      unnoticed for thirteen hours once (LEARNINGS 8).
- [ ] 6. Dependabot triage (A6)
      next: fourteen open since 2026-08-30. The two grouped minor/patch PRs
      merge together; `arrow`/`parquet` 56 to 59 touches the store and needs
      `older_files_still_read.rs` run against it on its own branch;
      `ed25519-dalek` 3 touches the signer's dependency and is read by hand.
- [ ] 7. Delete `radar-provider`'s cache, breaker and planner (A8)
      next: about 700 of 1,897 lines with no caller anywhere, flagged three
      times (LEARNINGS 1 and 9 are the same shape). Design 0007 J9 is Josh
      choosing delete over wire. `README.md`, `docs/STATE.md` and design 0006
      row 5 all describe it and change in the same commit.
- [ ] 8. `just orient` (A9)
      next: one recipe printing branch, that branch's last CI result, this
      file's Handback block, `target/` size, and the four claims
      `docs/STATE.md` names as most likely to be stale. It makes design 0006
      section 6 a command instead of a paragraph asking somebody to remember.

## Open questions for Josh

- Q1 (2026-09-04): the Tailscale browser check. Until it is clicked, nothing
  about production is verifiable from this workstation and item 4 cannot start.
  — unanswered
- Q2 (2026-09-04): which alert channel. Recommendation is a Discord webhook:
  free, one minute to create, and it reaches a phone. — unanswered
- Q3 (2026-09-04): the two X billing figures, which gate workstream B and are
  settleable with one live test post. Not needed for this plan. — unanswered

## Handback

Stopped at: item 3 landed; items 4 and 5 blocked on Q1 and Q2; items 6 to 8 are
unblocked and are the next work.
Next action: item 7, on its own branch — it is the largest unblocked one, it is
pure deletion, and design 0007 J9 already decided it.
Do not: open `Policy::CLOSED`, edit the custody lane, or retune `radar-graph`.
All three are in Not in scope above, and each has a document saying why.
