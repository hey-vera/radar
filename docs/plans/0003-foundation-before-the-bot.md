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

- Q1 (2026-09-04): the Tailscale browser check. Until it is clicked, nothing
  about production is verifiable from this workstation and item 4 cannot start.
  — unanswered
- Q2 (2026-09-04): which alert channel. Recommendation is a Discord webhook:
  free, one minute to create, and it reaches a phone. — unanswered
- Q3 (2026-09-04): the two X billing figures, which gate workstream B and are
  settleable with one live test post. Not needed for this plan. — unanswered
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

Stopped at: items 0 to 3, 7 and 8 landed. Items 4 and 5 are blocked on Q1 and
Q2. Item 6 is unblocked and is the only one left.
Next action: item 6, the Dependabot batch — the last unblocked item. Fourteen
PRs; the two grouped minor/patch ones go together, `arrow`/`parquet` 56 to 59
needs `older_files_still_read.rs` on its own branch because it touches the
store's format, and `ed25519-dalek` 3 is read by hand because it is under the
signer.
Do not: open `Policy::CLOSED`, edit the custody lane, or retune `radar-graph`.
All three are in Not in scope above, and each has a document saying why.
Do not delete `Observed<T>` on the strength of Q4 without answering it — an
unused type and a deleted invariant look identical in a diffstat.
