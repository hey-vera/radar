<!-- SPDX-License-Identifier: Apache-2.0 -->
# Plans

**Status:** accepted — [design 0002](../design/0002-the-development-workflow.md)
§1–§2, implemented 2026-09-03.

One file per unit of work, numbered, on the branch that implements it. It is the
plan, the todo list, the progress log and the handback note — not four files.

Plans lived in `C:\Users\Josh\.claude\plans\` before this, and that location
failed on its own evidence: eleven files with generated three-word slugs for
names, from at least two projects, none linked to a branch, none visible in a PR
diff, none reachable by `repo-conformance`. This audit's own design document was
orphan number twelve.

A plan is a **log**, like `docs/research/`. It is stale by design after merge:
its status becomes `landed <sha>` and it stops being read. Do not maintain one
after the work lands.

## Format

    # Plan 0003 — the summoned-reply credential

    Status: in progress | blocked | landed <sha> | abandoned
    Branch: feat/summoned-reply-credential
    Base:   a4ebb52
    Planned by: Opus 5, plan mode, 2026-09-03

    ## Objective
    One paragraph. What is true afterwards that is not true now.

    ## Not in scope
    The list that stops a task widening into a project (AGENTS.md §3).

    ## Items
    - [x] 1. radar-agent reserves before it spends
          done: `just check` green at 8f396df;
          `cargo mutants -f crates/radar-agent/src/spend.rs` 0 survived
    - [ ] 2. Persist the ledger across a restart
          next: Agent::restore has one caller and it is a test

    ## Open questions for Josh
    - Q1 (2026-09-03): does a customer's refusal get logged? — unanswered

    ## Handback
    Stopped at: item 2, ledger.rs written, not wired into startup.
    Next action: call Agent::restore from radar-serve main, not Agent::new.
    Do not: touch Policy::CLOSED — out of scope, see Not in scope.

## The three rules that make it worth reading

1. **An item is ticked only with evidence on the same line** — the command that
   decided it, and the commit it was green at. "Implemented" is not done.
   AGENTS.md §2 already requires this of prose; this applies it to a checkbox.
2. **The plan file is committed with the work it describes**, in the same
   commit, exactly as AGENTS.md §10 requires of any document that describes
   behaviour. The plan is then always a true statement about `HEAD`.
3. **A session ends by writing the Handback block, or it did not end** — it was
   abandoned. Stopped at, Next action, Do not. The next session reads one file
   and starts working; it does not re-derive. This is also what a compaction
   should land on: afterwards the agent re-reads `docs/plans/NNNN`, not the
   transcript.

`.claude/` keeps only what is *not* about this codebase's content: settings,
subagents, slash commands. Those are configuration; plans are work product.

## Kill condition

Design 0002 set one and it stands. This directory is the shape of LEARNINGS 1, 9
and 10 — a documented-as-central layer with no caller. It earns its place only
if the next session opens the file first. **If by 2026-09-17 handback blocks are
being written and not read, delete the directory rather than keeping it.**
