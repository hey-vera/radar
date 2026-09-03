<!-- SPDX-License-Identifier: Apache-2.0 -->
# Design 0002 — The development workflow

**Date:** 2026-09-03
**Status:** proposal, for Josh to accept, change or reject. **Not a decision.**
An accepted version of this becomes edits to [AGENTS.md](../../AGENTS.md), a
`docs/plans/` directory, and two small files under `.claude/`.
**Decides nothing yet.** Nothing in the repository depends on it.

## What this is answering

How one owner and a fleet of Claude sessions should divide a day's work so that
quality goes up, nothing is done twice, and the quota is spent on the work only
an expensive model can do.

The evidence is the session of 2026-09-02/03, not a survey. Two things about it
matter more than everything else:

**What worked.** The plan document was load-bearing — cited a dozen times over
many hours, and drift never happened. And the `0024` defect (failed transactions
counted as successful launches) was caught *only* because one long-lived context
had built both `0024` and `radar dossier` and noticed they disagreed about one
token. Split that work across two fresh agents and neither finds anything.

**What failed.** A third of the expensive model's time went on clippy chasing,
`write!` to `writeln!`, shell quoting and boilerplate mutation tests. Three
direction questions got chat answers and were never recorded. No state survived
a session boundary. 21 surviving mutants surfaced after a PR was open. A commit
landed on local `main`. A `git add -A` swept unrelated files into a commit whose
message described something else — which AGENTS.md §7 already forbids.

Every proposal below is aimed at one of those seven.

## 1. Where plans, todos and progress live

**Proposed: in the repository, under `docs/plans/` as a numbered slug, on the branch that
implements them.** One file per unit of work. It is the plan, the todo list, the
progress log and the handback note — not four files.

They are in `C:\Users\Josh\.claude\plans\` today, and that location fails on its
own evidence. The directory currently holds eleven files with generated
three-word slugs for names -- `cozy-sauteeing-lagoon`, `abundant-shannon` and
nine more like them -- from at
least two different projects. The filenames are machine slugs, so you cannot
find the plan for a commit; nothing links a plan to the branch it produced; a
plan cannot be read in a PR diff; `repo-conformance` cannot see it; and the
whole class of failure AGENTS.md §2 exists to prevent — documentation outliving
the thing it documents — is guaranteed rather than merely possible.

The costs of moving them in, stated plainly:

- The repository gains a directory of documents that are stale by design after
  merge. Mitigation: a plan's status becomes `landed <sha>` and it stops being
  read. It is a **log**, like `docs/research/`, not a live document.
- Plan churn shows up in `git log`. That is a feature — it is the only place the
  order of work is recorded.
- Half-finished plan files on abandoned branches. Acceptable: they die with the
  branch.

The format, deliberately minimal:

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

`.claude/` keeps only what is *not* about this codebase's content: settings, the
reviewer subagent, slash commands. Those are configuration; plans are work
product.

## 2. How a worker hands state back

The Handback block above is the whole mechanism. Three rules make it worth
reading:

1. **An item is ticked only with evidence on the same line** — the command that
   decided it, and the commit it was green at. "Implemented" is not done.
   AGENTS.md §2 already requires this of prose; this applies it to a checkbox.
2. **The plan file is committed with the work it describes**, in the same
   commit, exactly as §10 requires of any document that describes behaviour. The
   plan is then always a true statement about `HEAD`.
3. **A session ends by writing the Handback block, or it did not end** — it was
   abandoned. Stopped at, Next action, Do not. The next session reads one file
   and starts working; it does not re-derive.

This also fixes compaction: afterwards the agent re-reads `docs/plans/NNNN`, not
the transcript.

Guess, flagged: I am assuming a small commit whose only change is a checkbox and
an evidence line is acceptable. If that reads as noise, the alternative is
batching the plan update into the next real commit, which loses rule 2's
guarantee. I would take the noise.

## 3. When to use a cheaper model — and the axis is not seniority

The instinct is *planner = expensive, worker = medium, sub-agents = cheap*. The
evidence says that is the wrong axis twice over.

The `0024` bug was found by the **worker**, deep in the session, doing what
looks like routine work. The clippy chasing was also the worker. Same tier, same
session, and the two tasks could not be more different in what they needed.

The distinguishing property is not who is doing the work. It is:

> **Is there a command whose output decides whether this is right, and can it be
> done correctly seeing only the files named in the request?**
>
> - **Both yes** — cheap model. The compiler, clippy or a named test is the
>   oracle, and the context radius is one file.
> - **Either no** — expensive model, *in the session that already holds the
>   context*.

Applied to the actual session:

| work | oracle | radius | tier |
|---|---|---|---|
| clippy `-D warnings` chasing | clippy | the file | cheap |
| `write!` to `writeln!` | the compiler | the file | cheap |
| shell-quoting a justfile recipe | the recipe runs | the recipe | cheap |
| a test to kill a *named* surviving mutant | `cargo mutants -f` | file and test | cheap |
| noticing `0024` and `dossier` disagree | none | the whole session | expensive, never delegated |
| choosing a research method | none | the whole domain | expensive |
| is `Policy::CLOSED` still right | none | the architecture | expensive |

**The second correction is about sub-agents, and it is the important one.** A
sub-agent is not the cheap tier. A sub-agent is a **context boundary**, and the
boundary is precisely what would have destroyed the `0024` find. Cheapness and
isolation are independent knobs that the instinct fuses together.

Concretely, with Claude Code as it actually exists:

- To make a *stretch of the same session* cheap, use `/model sonnet`, do the
  mechanical run, `/model opus` to continue. Same conversation, same context, no
  boundary. This is the fix for the "third of the time" problem and it costs
  nothing structurally. (`/model opusplan` — Opus in plan mode, Sonnet for
  execution — is the pre-baked version of the same idea, worth a whole session
  of known-mechanical work.)
- Use a **sub-agent only when isolation is the point**: a wide independent
  search, or the reviewer in §7 below, whose value comes from *not* having seen
  the reasoning. That is AGENTS.md §8 already; the addition is naming isolation
  as the reason rather than cost.

Grounded versus guessed: the axis is grounded — it sorts every row of that table
correctly and it explains the one find nothing else explains. The claim that
`/model` switching materially saves quota is **a guess**. The mechanical third
was token-heavy (build output, lint spew), so it should help, but I have not
measured it, and a week of watching the limits would settle it.

## 4. Direction questions arriving mid-implementation

The rule just added to AGENTS.md §3 is **right in substance and belongs where it
is** — it is a priority-ordering rule and §3 is the priority-ordering section.
Two amendments, from the way it actually failed:

**Amendment A — get to a clean tree first, then stop.** "Stop the
implementation" read literally, mid-refactor, means abandoning a non-compiling
tree nobody can resume. The sequence should be: reach a committable point
(commit or `git stash`), record it in the plan's Handback, *then* stop. One or
two minutes, and it makes the stop reversible.

**Amendment B — write the document before answering in chat, not after.** This
is the actual mechanism of the failure. Each time, the answer went to chat
first, and the intention to write it up did not survive the return to the diff.
The owner's word for the result was "half-assed", and that is the same cause: an
answer composed as a chat interruption is composed to be short so coding can
resume. An answer composed as `docs/design/NNNN` is composed to be right. The
chat message should be a pointer to the document.

**Corollary worth stating:** a direction question is, by §3's own test above, an
oracle-free, whole-domain question. So it is never answered by a cheap model and
never delegated to a sub-agent. If the session is in `/model sonnet` for a
mechanical stretch when one arrives, switch back before answering.

In the plan file rather than in AGENTS.md: unanswered questions live under Open
questions for Josh so they cannot be silently dropped, and an answered one is
replaced by a link to the design doc that answers it.

## 5. What runs when

The 21 mutants were not a CI problem. They were a **batching** problem: a large
diff, checked once, late. The ladder below moves each check to the earliest
point it can run, and none of it violates §8.

**Per edit** — `cargo clippy -p <crate>` and `cargo test -p <crate>`. Already
the rule.

**Per file finished** — `cargo mutants -f <that file>`. §8 explicitly permits
this: "a couple of minutes". **Proposed new standard: a file is not done until
its own single-file mutant run is clean.** Four survivors at a time, on the file
whose test is still fresh in context, is a different task from twenty-one at
once on an open PR — cheaper in wall clock, much cheaper in quota, and exactly
the case where a cheap model in the same session can write the test.

**Per commit** — read the staged diff. Two mechanical guards, cheap, proposed
because §7 already said this in prose and it happened anyway:

- `git branch --show-current` before the first commit of a session. The `main`
  accident is a one-line check.
- Stage by path. `git add -A` should not appear in this repository; if
  everything really is wanted, `git add -u` plus the new files named says so
  deliberately.
- If it should be enforced rather than remembered: a `scripts/hooks/pre-commit`
  plus `git config core.hooksPath scripts/hooks`, refusing a commit on `main`
  and printing the staged diffstat. Ten lines, versioned, no service, nothing
  paid for. I would do this — prose has now failed at it twice.

**Per push** — open the PR as a **draft immediately**, on a small branch. CI's
`mutants-shards` is `--in-diff` against the base, so its cost tracks the diff.
The justfile's own comment records the shape: 28 mutants on an early branch, 408
on a forty-commit one, never finishing inside a runner's life. **Small PRs are
the mutation-testing strategy.** Then §8's "do not wait" applies: push, and go
do the next unblocked item.

**Never locally** — `just mutants` over a branch, `--release`, full-workspace
rebuild loops. And check `target/` at the end of a session that built a lot; it
reached 127GB once and froze the machine.

## 6. Proposed AGENTS.md text

Short, because that file's value is that every line is load-bearing. Four
insertions, about twenty lines in total.

**§6, after the mutation-testing paragraph:**

> **Mutate the file when you finish the file, not the branch when you finish the
> branch.** `cargo mutants -f <one file>` takes a couple of minutes and is the
> one form of this check §8 permits locally. Twenty-one survivors reported on an
> open PR is the same information, arriving after it is expensive to act on.

**§7, after the `git add -A` paragraph:**

> **Stage by path, and know your branch.** `git add -A` does not appear in this
> repository. Run `git branch --show-current` before the first commit of a
> session; a commit that lands on local `main` has to be moved by hand.

**§8, after "Investigate directly rather than delegating":**

> **A sub-agent is a context boundary, not a discount.** Use one when isolation
> is the point. Use `/model sonnet` — same session, same context — when the work
> is merely mechanical.
>
> **The test for both:** is there a command whose output decides whether this is
> right, *and* can it be done seeing only the files named in the request? Both
> yes, it is cheap work. Either no, it stays with the expensive model in the
> session that holds the context. The defect in `docs/research/0024` was found
> by neither half of a split alone.

**§10, after the first bullet:**

> Plans, their task lists and their handback notes live in `docs/plans/`,
> committed on the branch they describe. A task is complete when the line says
> which command proved it and at which commit. A session ends by writing the
> handback block.

## 7. What Josh is not asking about

Three, and no more.

**a. Nobody reviews the diff.** Every check in this repository is mechanical —
build, tests, clippy, fmt, deny, headers, mutants. All of them are oracles, and
by §3's test they say nothing about the class of defect that has actually cost
this project money: LEARNINGS 11, 12, 18, 19 and 24 would each pass every one of
them. The author-agent cannot review its own diff, because the reasoning that
produced the bug is the reasoning that would review it. **This is the one place
where a context boundary is worth paying for.** Propose
a reviewer agent definition under `.claude/agents/` with `model: sonnet`, invoked before a PR leaves
draft, given exactly two things: the plan file and `git diff origin/main`. Not
the transcript. Its question is "does this diff do what the plan says, and
nothing else". Cheap, isolated, and the missing half of the loop.

**b. Institutionalise the accident.** The `0024` find was luck — two
independently built instruments happened to be compared. It is the highest
value-per-hour event of the whole session and it is currently unrepeatable.
Propose: **every plan that produces a number carries one item that computes it a
second way and compares.** Not a second test; a second instrument. The
repository already believes this — §1's "let a reference propose and a capture
dispose", LEARNINGS 18's two instruments compared as one — but nothing in the
workflow makes it happen on purpose.

**c. Quota goes on context, not on tier.** The model tier is the visible lever
and probably not the biggest one. A session that pastes whole `cargo test`
outputs, re-reads `LEARNINGS.md` (1,315 lines) and `AGENTS.md` (484) repeatedly,
and then rides into an auto-compaction spends more than the Opus/Sonnet delta.
Two habits: pipe long build output through `tail`, and **end sessions
deliberately at a plan-item boundary with a handback written**, rather than
letting compaction end them. Compaction is where the quota goes *and* where the
thread was lost — one fix addresses both. This is the least evidenced item here;
it is reasoning about how the tools bill, not a measurement.

## Where I think this is weakest

**The single-file mutant rule may not cover what it claims.** `--in-diff` over a
branch mutates changed lines across files; `-f` mutates one whole file in the
working tree. They are not the same set, and a change spanning two files can
leave a survivor only the branch-wide run finds. Per-file runs reduce the CI
surprise; they do not eliminate it. Keep reading the PR job.

**`docs/plans/` could become another documented-as-central layer with no
caller** — the shape of LEARNINGS 1, 9 and 10, and of §5's "a crate nothing
depends on is a document that compiles". It earns its place only if the next
session actually opens the file first. If after two weeks handback blocks are
being written and not read, delete the directory rather than keeping it.

**The `/model` saving is unmeasured**, as flagged in §3. If the mechanical third
turns out to be cheap in tokens and expensive only in wall clock, the tiering
argument is a smaller win than it looks, and the review sub-agent and the
per-file mutants are the parts worth keeping.

**The pre-commit hook is a behavioural claim I cannot verify.** It stops the two
specific accidents that happened. It does not stop the general case of a commit
message that does not describe its diff, and nothing mechanical will.
