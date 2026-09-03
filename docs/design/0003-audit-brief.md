<!-- SPDX-License-Identifier: Apache-2.0 -->
# The repository audit brief

**What this is:** the prompt to paste into a fresh Claude Code session, in plan
mode, to audit this repository's structure, documentation architecture and
enforcement. Kept in the repo so the next one can be a diff against this rather
than written from scratch.

**Why it is a document and not a chat message:** the last plan lived in
`~/.claude/plans/` under a generated slug for a filename and could not be linked to the
work it produced. See [`0002`](0002-the-development-workflow.md) §1.

---

You are auditing this repository's **structure, documentation architecture and
enforcement** — not its code quality. Work in plan mode. **Change nothing.**
Produce one document; the implementation is a separate session.

## The one-sentence goal

This repository is developed almost entirely by AI agents arriving with no
memory of the last session. Make it a place where that works: where an agent can
find the authoritative answer fast, cannot plausibly miss an invariant, and
cannot leave the next agent worse off than it found it.

## Read these first, in this order

1. `AGENTS.md` (484 lines) — the operating policy. §3 scope, §4 the nine
   non-negotiable rules, §6 verification, §7 safety, §8 delegation and the
   workstation limits, §10 persistent state.
2. `docs/design/0002-the-development-workflow.md` — **a workflow proposal
   accepted in substance last session.** It answers where plans live, how
   sessions hand off, and when to use a cheap model. **Do not redo it.** Your job
   starts where it stops: it deliberately says nothing about folder layout,
   README structure, file size, or what enforcement should exist.
3. `LEARNINGS.md` (1,315 lines) — 28 entries, each a mistake that cost something.
   Skim the headings; read 1, 9, 10, 21, 26 in full. They are the failure modes
   this repository actually has.
4. `docs/STATE.md` — claims about the world, which decay.
5. `crates/repo-conformance/` — the existing enforcement. Read what it checks
   before proposing anything new.

Match the house voice: direct, evidence first, willing to say "this is where I am
guessing". `docs/design/0001` and `0002` are the style.

## What already exists — do not propose these

A generic audit will recommend things this repository has had for months. It has:

- **Decision records** — `docs/adr/`, 12 of them, with supersession notes.
- **A findings archive** — `docs/research/`, 24 notes, with `queries/` carrying
  the SQL and `data/` carrying published snapshots.
- **A mistake log** — `LEARNINGS.md`, 28 entries.
- **An owner's product document** — `GOAL.md`, explicitly authoritative over
  agent opinion.
- **A state document** — `docs/STATE.md`, explicitly marked as decaying.
- **Agent instructions** — `AGENTS.md`, with `CLAUDE.md` as a one-line include.
- **Mechanical enforcement** — `repo-conformance` already fails the build when a
  crate directory is not a workspace member, a documented path is untracked, an
  ADR is cited by a number that does not exist, a relative link does not
  resolve, or the README crate table drifts from the workspace.
- **CI** — build, tests, clippy `-D warnings`, fmt, licence headers, MSRV,
  `cargo-deny`, a web suite, and mutation testing sharded across four runners.

If you recommend one of these, you have not read the repository.

## Evidence from the last session — use it, do not rediscover it

These are measured, not hypothetical. They are the failures worth designing
against.

**Context loss is the expensive failure.** Eleven plan files sit in
`~/.claude/plans/` with generated three-word slugs for names, mixing at least two
different projects, none linkable to a commit.

**The checks are all oracles, and the defects that cost money are not.** Every
check listed above is mechanical. LEARNINGS 11, 12, 18, 19 and 24 would each
pass all of them. Nothing reviews a diff for whether it does what was intended.

**One find justified the whole session and it was luck.** A research note
(`docs/research/0024`) counted *failed* transactions as successful launches. It
was caught only because a tool built later in the same session disagreed with it
about one specific token. Two independent instruments, compared by accident.

**Mutation testing caught 21 vacuous assertions**, including one that used one
success and one failure — so inverting the filter still produced the same count
and the test passed either way. Found after a PR was already open.

**Two mechanical accidents AGENTS.md already warns about happened anyway**: a
`git add -A` swept unrelated files into a commit whose message described
something else, and commits landed on local `main` before branch protection
caught them. A rule that is written and still violated is a rule that needs a
mechanism, not more words.

**Three crates exist that nothing depends on** — LEARNINGS 1, 9 and 10 are all
that shape, and `radar-provider`'s cache, breaker and planner (712 of 1,876
lines) have no caller today.

## The questions to answer

Answer these. Do not answer questions nobody asked.

**Structure.** 24 crates and a `docs/` split four ways (`adr/`, `research/`,
`design/`, plus `STATE.md`). Is that the right shape? Specifically: is `design/`
distinct enough from `adr/` to justify existing, and where do *plans* go given
`0002` proposes `docs/plans/`? Name any folder that should exist and does not,
and any that should be merged.

**Navigation.** `AGENTS.md` is 484 lines and `LEARNINGS.md` is 1,315. Every
session loads both. Is that the right trade between completeness and context
cost, and if not, what is the split — an index with detail behind it, or
something else? Be concrete; this is the single largest recurring token cost.

**Authority and staleness.** A fresh agent must be able to tell in seconds which
document is current, which is superseded, and which is a proposal. Today that is
carried in prose (`0001` says "proposal, not a decision"; ADR 0005 says
"superseded by 0011"). Should it be mechanical, and can `repo-conformance`
enforce it?

**File size.** The five largest sources are 1,938 / 1,784 / 1,517 / 1,445 /
1,279 lines. Is that a real problem for an agent that must load a file to change
one function, or is it fine because the modules are coherent? Answer with a
reason, not a rule of thumb.

**READMEs.** There is one at the root, and none per crate. Is that right for a
24-crate workspace, or should crates carry their own? Note that `AGENTS.md`
already warns documentation that outlives its subject is how this project has
lost work before — so argue the maintenance cost, not just the benefit.

**Enforcement.** This is the important one. For each convention this repository
has, say whether it is enforced mechanically, enforced by convention, or merely
described — and for the ones merely described, whether a check is worth writing.
`repo-conformance` is the obvious home. Prefer one check that fails loudly over
three paragraphs asking nicely.

**What is missing.** Name at most five things this repository does not have and
should. Justify each against a specific failure above, not against best practice
in general.

## Constraints

- **Anti-bloat is the hard one.** This repository's culture is that every line of
  `AGENTS.md` is load-bearing. A recommendation that adds a document nobody will
  read is worse than nothing — it is LEARNINGS 1's shape, and this project has
  produced it three times. Every proposed artefact needs a named reader, a
  trigger for reading it, and a condition under which it gets deleted.
- **Preserve the goal, challenge the implementation.** Where `GOAL.md` or the
  owner states an intent, that is the objective. Say plainly where an assumption
  is wrong and show the evidence, then solve for the stated goal rather than
  substituting your own.
- **Machine limits are real.** `AGENTS.md` §8: this is the owner's daily-use
  workstation, one cargo process at a time, no wide mutation runs, `target/` has
  frozen the machine before. Do not propose anything that grinds it.
- **No new paid services.** No tooling you have not verified exists.
- **Separate verified fact from inference from guess**, and say which you are
  offering. Do not present speculation as fact.

## Output

One document, the next number under `docs/design/`, readable in fifteen minutes.
Length is not quality. Sections:

1. **Two scores, 1–10** — engineering quality, and agent-readiness — each with
   the one sentence that most justifies it.
2. **What is genuinely strong**, and should not be touched.
3. **What is actively costing something**, with the evidence.
4. **The documentation architecture** you recommend, as a file tree, marking each
   entry authoritative / historical / proposal and naming who reads it and when.
5. **Enforcement** — the table of convention against mechanism, and which checks
   are worth writing.
6. **Structure changes**, only where they materially help.
7. **Roadmap** — P0 to P3, each with problem, action, rationale, risk, rough
   scope. Nothing lands in P0 unless the next session is worse without it.
8. **Where you are guessing**, and what would settle it.

Then stop. Do not implement.

## One thing to be suspicious of

This repository documents itself unusually well, which makes it easy to conclude
it is in good shape and recommend polish. Resist that. The last session found a
real defect in a published research note, twenty-one vacuous assertions, and two
violations of written rules — inside a repository whose documentation is its
strongest feature. **Good documentation and working enforcement are different
things, and the gap between them is what you are looking for.**
