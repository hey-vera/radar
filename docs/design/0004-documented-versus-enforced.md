<!-- SPDX-License-Identifier: Apache-2.0 -->
# Design 0004 — Documented versus enforced

**Date:** 2026-09-03
**Status:** proposal, for Josh to accept, change or reject. **Not a decision.**
An accepted version becomes checks in `crates/repo-conformance/`, edits to
`README.md`, `docs/STATE.md` and `AGENTS.md`, and a `**Status:**` line on ten
documents. **Decides nothing yet.**

> **This file is the audit brief's deliverable and it is in the wrong place.**
> [`0003`](0003-audit-brief.md) asks for "one document, the next number under
> `docs/design/`". Plan mode permits writing exactly one file, and that file is a
> machine-slugged plan under `~/.claude/plans/` — the twelfth in the directory
> [`0002`](0002-the-development-workflow.md) §1 says is broken, produced by the
> session auditing this repository for exactly that failure. The implementation
> session's first commit is a `git mv` of this content, unchanged, to
> design number 0004 under `docs/design/`.
>
> Six paths were rewritten across this document to get it past
> `every_file_path_named_in_the_documentation_exists_and_is_tracked` — including
> its own future filename and the plan file it is currently sitting in. The check
> works, and it is stricter than it looks from reading it.

---

## Three premises in the brief that did not survive checking

Stated first, because two of them change the answer.

**"Every session loads both `AGENTS.md` and `LEARNINGS.md`."** It does not.
`CLAUDE.md` is a one-line include of `AGENTS.md`, so 484 lines (~6.5k tokens) load
automatically and `LEARNINGS.md` (1,315 lines, ~17k tokens) loads only if an
agent follows a link in a table. The cost is real but it is the *opposite*
shape: the mandatory document is the policy, and the optional one holds the 28
measured failure modes. That inverts the navigation recommendation — see §4.

**"Three crates exist that nothing depends on."** Not reproducible against the
current workspace. Every crate has at least one non-dev dependent except the two
binaries (`radar-cli`, `radar-serve`) and `repo-conformance`, which are supposed
to. The half of that claim that *is* true is narrower and already documented
correctly: `radar-provider`'s `cache.rs` (438) and `health.rs` (274) are 712 of
its 1,876 lines and have no caller outside the crate — verified, only
`Meter`, `Budget`, `Ledger` and `Commitment` from `cost.rs` are reached, by
`radar-agent` and `radar-serve`'s ledger. `AGENTS.md` §5's "this project has
produced three" is about three historical instances, not three live crates.

**"The five largest sources are 1,938 / 1,784 / 1,517 / 1,445 / 1,279 lines."**
True as line counts, misleading as a measure of source. Four of those five are
roughly half inline test module. Full numbers in §3.

---

## 1. Two scores

**Engineering quality: 8/10.** The checks that exist are better than the ones
most repositories have — a test-count *floor* that makes absence loud, mutation
shards that were made to prove the parts sum to the whole, a `_disk` prerequisite
that refuses to build rather than freeze the owner's machine, and a
`required-checks.txt` that writes down the `gh api` query instead of the answer —
and the one file that is genuinely large in *production* code is
`radar-serve/src/lib.rs` at 1,366 code lines against 79 of test, in the crate
holding the listener, the paywall, the model provider and the embedded frontend.

**Agent-readiness: 7/10.** A fresh agent gets an unusually good spine — filenames
that state their finding, `GOAL.md` and `docs/STATE.md` each declaring their own
authority and decay, twelve ADRs with supersession notes — and then hits the gap
this audit is about: the two most-read documents in the repository both assert
that no production crate depends on `radar-exec`, `crates/radar-cli/Cargo.toml`
has depended on it since 2026-08-31, and both sentences were written on
2026-09-03.

---

## 2. What is genuinely strong

Do not touch any of this.

**Filenames are claims.** `0014-the-control-was-entirely-tokens-nobody-could-sell.md`
tells you the finding before you open it. Twenty-four research notes are
navigable by `ls`, which is worth more than any index and costs nothing to
maintain. This is the single best documentation decision in the repository.

**One subject each, declared.** `GOAL.md` opens by saying it is Josh's document
and holds no engineering invariants, and says an earlier version was a verbatim
copy of `AGENTS.md`. `docs/STATE.md` opens by saying everything in it is a claim
that decays. `AGENTS.md` opens by saying it is a policy and not a reference.
Three documents that each say what they are *not* is rare and it works.

**The correction culture.** `0016` corrects `0014`'s headline by six times the
signal it hid. `0022` strikes its own recommendation in one line. `README.md`
quotes and retracts its own previous sentence. `docs/STATE.md` names the figure
it carried until 2026-09-03 and why it was wrong. This is the hardest thing on
this list to build and the easiest to lose.

**Checks that check they ran.** `the_extractor_finds_something_in_the_real_documents`,
`the_shipped_policy_is_actually_reached_from_outside_radar_risk`,
`ls_files_output_becomes_paths_and_never_an_empty_one`, the `MIN_TESTS` floor,
the `mutants` job asserting `skipped` and `cancelled` are failures. Almost every
check in this repository has a second check saying it is not vacuous. That is a
higher standard than most production codebases hold.

**One definition per check.** Every CI job runs a `just` recipe and the workflow
contains no copy of a command. `required-checks.txt` maps status-check context to
recipe and tells the reader to run `gh api` rather than trusting the file.

**Crate-level `//!` docs.** Every one of the 24 crates carries 6–60 lines of
module documentation — `radar-risk` 60, `radar-graph` 45, `radar-types` 17. This
is the per-crate README, in the file an agent must already open, rendered by
`cargo doc`, and covered by `repo-conformance`'s path checks. See §6.

**`deploy/README.md`.** An 800-line runbook with a "checked 2026-08-25" section
and a "what CI does not do, on purpose" section. It is why no `docs/runbooks/`
is missing.

---

## 3. What is actively costing something

### 3.1 One fact, three documents, no mechanism, two copies wrong

`AGENTS.md`:173–190, `docs/STATE.md`:166–200 and `README.md`:26–38 each carry an
independent "what this test establishes / what it does not" account of the same
two composition tests. Two of the three have drifted into being false:

> "No production crate depends on `radar-exec`; the composition reaches it
> through a dev-dependency, so the shipped dependency graph is unchanged."
> — `docs/STATE.md`:184, and in near-identical words `README.md`:32

`crates/radar-cli/Cargo.toml` lists `radar-exec.workspace = true` under
`[dependencies]`, and `crates/radar-cli/src/route.rs`:29–30 uses
`radar_exec::pipeline::Routing` and `radar_exec::route::Router`. `radar-cli`
builds the `radar` binary that `deploy/radar-brief.sh` runs in production.

The dates make this worse than a stale sentence. The dependency landed in
`402e76a` on **2026-08-31**. The `docs/STATE.md` sentence was written in
`e1b82d7` and the `README.md` one in `e450fb5`, both on **2026-09-03**. Neither
was true on the day it was committed.

**Cost:** this is not a decorative claim. It is the sentence that says the
shipped dependency graph cannot reach the trading path — the thing `AGENTS.md`
rule 1 exists to guarantee. The narrower statement that *is* true, and which
`docs/STATE.md`:198 makes separately, is that nothing invokes the *pipeline* in
production. `repo-conformance` already reads crate manifests to assert negative
dependency edges, in
`nothing_that_listens_on_a_network_depends_on_the_signer_crate`. The machinery to
decide this exists and was not pointed at it.

### 3.2 `LEARNINGS.md` stopped honouring its own opening standard nine entries ago

Line 4 of the file:

> "Each entry names the check that would catch a recurrence, or says plainly that
> nothing does."

Measured across all 28 entries:

- **Entries 1–19** use `**What catches a recurrence:**` and name an *artefact* —
  `radar-store/tests/watermark_holds.rs`, `older_files_still_read.rs`,
  `discover_capacity`, a `repo-conformance` test. Entry 13 says "not a check — a
  change of shape", which honours the second clause honestly.
- **Entries 20, 21, 23, 27, 28** use `**The check:**` and name a *habit addressed
  to a reader*: "when one shard of a parallel job dies and the rest pass, look at
  what that shard was given". True, useful, and not a mechanism.
- **Entries 22, 24, 25, 26** have neither header.

So nine of twenty-eight entries — every one since roughly 2026-08-30, the nine
closest to current work — leave no mechanism behind, and the format gives a
reader no way to tell them from the nineteen that do. This is the gap the brief
asked to look for, inside the document that exists to record it.

### 3.3 Nothing survives a session boundary, and this audit is instance twelve

`~/.claude/plans/` holds eleven files with generated slugs from at least two
projects. This session added a twelfth.
`docs/design/0002` diagnosed this on 2026-09-03 and none of its mechanisms
exist: no `docs/plans/`, no `.claude/agents/`, no `scripts/hooks/`, and
`git config core.hooksPath` is unset. `.claude/` contains one file, `launch.json`.

**Cost:** it is the reason this document has to open with an instruction to move
itself.

### 3.4 The status convention is 60% adhered to and enforced nowhere

Of 48 tracked markdown documents, the `**Status:**` line is present on:

| category | with Status | without |
|---|---|---|
| `docs/adr/` | 12 of 12 | — |
| `docs/design/` | 2 of 3 | `0003-audit-brief.md` |
| `docs/research/` | 14 of 24 | `0001`–`0008`, `0010`, `0015` |

The ten research notes without one are the *oldest*, which is the wrong way
round: `0008`'s headline (68% of instant graduations have exactly six recipients)
was superseded by `0024` at 25.1%, and `0007`'s central figure was re-measured by
`0024` from 2.12× to 3.37×. `0008` does carry a hand-written supersession block
at line 9 — the convention working by discipline. `0007` carries no forward
pointer at all, so a reader who opens it directly gets 638 creators and 2.12×
with no sign that a 4.6× larger re-measurement exists.

**Cost:** the brief's question — can an agent tell in seconds which document is
current — is answered "yes, if the author remembered". It has been remembered
most of the time. Most of the time is how `docs/STATE.md`:184 happened.

### 3.5 The file-size number is mostly an artefact, and the one real case is the worst-placed one

| file | total | code | tests |
|---|---|---|---|
| `crates/radar-cli/src/consider.rs` | 1,938 | 943 | 995 |
| `crates/radar-serve/src/api.rs` | 1,784 | 862 | 922 |
| `crates/radar-cli/src/brief.rs` | 1,517 | 828 | 689 |
| **`crates/radar-serve/src/lib.rs`** | **1,445** | **1,366** | **79** |
| `crates/radar-serve/src/access.rs` | 1,279 | 680 | 599 |
| `crates/radar-signer/src/verify.rs` | 1,166 | 381 | 785 |

Four of the five largest files are about half test module, and `verify.rs` — the
one holding the signing guarantee — is 2:1 test to code, which is the right
answer. `radar-serve/src/lib.rs` is the outlier in both directions: the most
production code of any file in the repository and the thinnest test module, in
the internet-facing crate. It is a router root plus ~25 handlers plus the whole
SIWS sign-in path (challenge, verify, nonce parsing, domain extraction, lines
231–386) that would sit naturally beside `challenges.rs` and `access.rs`.

**Cost:** `just mutants --in-diff` mutates changed lines. A file with 1,366 code
lines and 79 test lines is where surviving mutants come from, and the twenty-one
that surfaced on an open PR are the measured version of this.

---

## 4. The documentation architecture

The recommendation is **close to what exists**. The shape is right; three
entries move and one field becomes mechanical.

```
GOAL.md                     AUTHORITATIVE  the owner's product document
                                           read: every session, before proposing direction
                                           delete: never; superseded only by editing it

AGENTS.md                   AUTHORITATIVE  operating policy, auto-loaded via CLAUDE.md
                                           read: automatic, every session
                                           TRIM: §4 rule 1's status prose moves to STATE.md (§7, P2)

README.md                   AUTHORITATIVE  what Radar is, for a human arriving cold
                                           read: first contact only
                                           owns: the crate table (checked)

LEARNINGS.md                AUTHORITATIVE  28 paid-for failures
                                           read: before changing a subsystem, via a NEW index (P1)
                                           delete: never; entries are append-only

SECURITY.md                 AUTHORITATIVE  disclosure
LICENSE                     AUTHORITATIVE

docs/STATE.md               AUTHORITATIVE, DECAYING  claims about the world
                                           read: after compaction, before trusting any number
                                           owns: what the composition tests establish (§7, P0)

docs/adr/           0001-0012  HISTORICAL + BINDING  a decision that constrains code
                                           read: when touching what it decided; on citation
                                           status: accepted | superseded by NNNN  (CHECKED, P1)

docs/design/        0001-0004  PROPOSAL until accepted, then HISTORICAL
                                           read: when the question it answers comes up again
                                           terminal state: becomes an ADR or a GOAL.md edit,
                                           then it is a record of the reasoning, not a live doc

docs/research/      0001-0024  HISTORICAL MEASUREMENT  what was true when measured
                                           read: before asserting a number; via STATE.md's index
      queries/                 the SQL behind a note
      data/                    published snapshots, each carrying its measurement date
      vendor/                  inputs classified by 0010 and 0015, never findings

docs/plans/         NEW        WORKING, then HISTORICAL after `landed <sha>`
                                           read: first action of a resumed session
                                           per docs/design/0002 §1 — its format, not a new one
                                           delete condition: 0002's own, two weeks unread

deploy/README.md    AUTHORITATIVE, DECAYING  the runbook
                                           read: before touching production
                                           the "checked <date>" line is the decay marker

crates/*/src/lib.rs //! docs   AUTHORITATIVE  the per-crate README (see §6)
```

**Why `design/` earns its own directory.** The separating test is *what is wrong
if the document is wrong*. A wrong ADR means the code is wrong. A wrong research
note means a belief about the market is wrong. A wrong design document means
nothing, because — in this repository's own words at the top of `0001` and
`0002` — "nothing in the repository depends on it". `design/` is also the only
category with a **terminal state**: an accepted design becomes an ADR or a
`GOAL.md` edit and the design doc becomes a record of the reasoning. Merging it
into `adr/` would put proposals that decide nothing next to decisions that bind
code, in one numbered sequence, with `**Status:**` as the only separator. Keep
them apart.

**Navigation — the answer is not to split `AGENTS.md` into an index.** Given the
corrected premise (§0), the two problems are different and need different fixes:

1. **`AGENTS.md` is 484 lines because state grew back into it.** §4 spans lines
   135–299, a third of the file, and rule 1 alone spans 140–208. Of those 69
   lines, roughly 25–35 are *status* rather than invariant: what
   `the_customer_lane_composes.rs` establishes as of 2026-09-01, what
   `the_budget_survives_a_restart.rs` fixed on 2026-08-31, which components the
   spend meter is wired for today. `docs/STATE.md` exists because this surgery
   was already done once; its own first line says so. The fix is to finish it,
   not to add an index layer on top.

   **This is a smaller win than it first looked.** A first pass of this section
   said rule 1 spanned 93 lines of which 78 were status; recounting against the
   file gives 69 and 25–35, because most of the Privy material is the *reasoning*
   for ADR 0007 and belongs in a policy document. So the saving is roughly 6% of
   the automatic load, not 16%, which is why this sits at P2 and not P1.

2. **`LEARNINGS.md` is optional, unindexed, and all-or-nothing at ~17k tokens.**
   The fix is a table at the top of the file itself — number, the shape in five
   words, and whether a mechanism exists — about thirty lines. Not a new
   document: a section inside the file it indexes, so it cannot outlive its
   subject. An agent loads thirty lines and reads the three entries that touch
   what it is about to change.

**No index document under `docs/`.** Twenty-four descriptive filenames and
`docs/STATE.md`'s own "where to start" section already do it, and an index of a
directory is the artefact most likely to name a file that moved.

---

## 5. Enforcement

### Already mechanical — do not propose these again

`repo-conformance`: crate dirs are workspace members · members have manifests ·
members have source · relative links resolve · ADR numbers cited exist · README
crate table matches workspace · deploy guide's files exist · listeners do not
depend on `radar-signer` · AI crates cannot reach `radar-risk`/`radar-exec`/
`radar-strategy`/`radar-store` · only `radar-risk` names `Policy::CLOSED` ·
`Policy::SHIPPED` is reached from outside · documented paths exist *and are
tracked* · `web/dist/.gitkeep` is tracked.

`justfile` + CI: build · tests with a `MIN_TESTS` floor · clippy `-D warnings` ·
fmt · SPDX headers on `*.rs` and `*.ts` · `cargo-deny` · MSRV 1.90 · web suite with
a `MIN_WEB_TESTS` floor · mutation testing sharded four ways with a union job
that treats `skipped` and `cancelled` as failures · `target/` size ceiling as a
prerequisite of `just check`.

### Convention or prose only

| convention | today | worth a check? |
|---|---|---|
| documents' claims about the dependency graph | **prose only — false in 2 docs** | **yes, P0** |
| every ADR / design / research doc carries `**Status:**` | convention, 38 of 48 | **yes, P1** |
| a superseded document names what superseded it | convention, by hand | **yes, P1** — as a `**Status:**` value |
| `required-checks.txt` matches the justfile and `ci.yml` | prose only; the file says to run `gh api` | **yes, P1** |
| one test file is accounted for by one document | convention, 4 collisions today | **yes, P1** |
| every LEARNINGS entry names a check or says none exists | convention, 19 of 28 | **yes, P2 — shape only** |
| `AGENTS.md` holds policy, not status | convention, has regressed once | **no — not decidable; P2 is a trim** |
| `git add -A` never appears | prose (§7); violated 2026-09-02 | hook, per `0002` §5 — not conformance |
| no commit on local `main` | prose (§7); violated | hook, per `0002` §5 |
| SPDX header on markdown | convention, 39 of 48 | **no** — ceremonial, `LICENSE` covers it |
| document numbers are unique and sequential | convention | **no** — a collision is visible in `ls` |
| `AGENTS.md` "§N" / "rule N" citations resolve | convention | **marginal** — see §8 |
| a new document is checked before it is committed | **not checked at all** | **yes, P1** — see below |

### Five things missing, each against a named failure

1. **A mechanism that can falsify a prose claim about the dependency graph.**
   Against §3.1. Concretely: `the_documented_dependency_claims_are_true` pinning
   the non-dev dependent set of exactly the three crates documents make claims
   about — `radar-exec` → `{radar-cli}`, `radar-provider` → `{radar-agent}`,
   `radar-signer` → `{radar-exec}` — with a comment naming the paragraphs that
   restate it. Scoped to three rows deliberately: 42 of 134 commits since
   2026-08-22 touched a crate manifest, so pinning the whole edge set would churn
   on a third of all commits.

2. **`**Status:**` as a field rather than a paragraph**, with `superseded by
   NNNN` required to resolve to a document that exists. Against §3.4. Reuses the
   ADR-number extractor already in `repo-conformance`.

3. **A check that `.github/required-checks.txt` matches the two things it
   describes** — each `NAME = just RECIPE` line's recipe exists in the justfile,
   and `NAME` appears as a job `name:` in `ci.yml`. Against LEARNINGS 13, whose
   shape that file already carries. It closes two of three legs; the GitHub
   ruleset itself is off-machine and stays a manual `gh api` query, which the
   file already says.

4. **An index into `LEARNINGS.md`.** Against §3.2 and the navigation question. A
   thirty-line table at the top of the file, not a new document.

5. **One test file, one owning document.** Against §3.1. A test path may appear
   in at most one of `AGENTS.md`, `README.md`, `docs/STATE.md`. Four violations
   today: `the_customer_lane_composes.rs` (AGENTS + README), `lane_composes.rs`
   (STATE + README), `cost.rs` and `ledger.rs` (AGENTS + STATE). The honest cost:
   a document wanting to *link* a test would also fail, and the right response is
   to link the document that owns the account instead — which is the behaviour
   the "one subject each" rule already wants.

**Found while verifying this document, and it is the same shape as the rest.**
`documents()` derives its list from `known_files()`, which is `git ls-files`. So an
**untracked** markdown file is not checked by any conformance rule — a document is
only checked from the commit that adds it, which is one commit too late to stop
the author committing a broken one. This audit's own file passed all 21 checks
only after `git add -N`. The justfile's `mutants` recipe already guards the
identical hole for Rust (*"untracked Rust files are invisible to `git diff` and
would NOT be mutated"*) and refuses rather than skipping; `repo-conformance` has
no equivalent. The fix is the same shape: include untracked-but-present markdown
in `documents()`, or refuse when any exists. Roughly ten lines. Fold into P1.

`docs/plans/` is deliberately not on this list. It is `0002`'s and it is P0.

---

## 6. Structure changes

**Crates: none.** Twenty-four is defensible because the boundaries are the
security argument — `repo-conformance` asserts negative edges between them, and
that assertion is only possible because they are separate crates. Nothing should
be merged. `radar-provider`'s 712 uncalled lines are a *deletion or wiring*
question, already documented in `README.md` and `docs/STATE.md` in the correct
words, and it is a decision for the owner rather than a structural finding.

**`docs/`: one addition, no merges.** `docs/plans/` per `0002` §1. No
`docs/runbooks/` — `deploy/README.md` is one, at 800 lines with a checked date.

**Per-crate READMEs: no.** Every crate already carries 6–60 lines of `//!`
module documentation. That artefact is strictly better than a README on four
counts: it is in the file an agent must open to change the crate, `cargo doc`
renders it, `rustdoc` link-checks it, and `repo-conformance`'s
`every_file_path_named_in_the_documentation_exists_and_is_tracked` covers the
paths inside it. A per-crate README would be a second copy with no compiler and
no reader — `LEARNINGS` 1's shape, which `AGENTS.md` §2 names as the specific way
this project has lost work. The maintenance cost is not hypothetical: 24 new
files, each stale the first time a crate's purpose shifts, and nothing to notice.

**File size: not a problem as measured, one real case.** The rule of thumb is
wrong here because four of the five largest files are half test module, and the
repository's idiom of a `#[cfg(test)]` block at the bottom is what
`repo-conformance::code_references` already relies on. An agent loading
`verify.rs` to change one function loads 381 lines of code and 785 lines showing
how that code is expected to behave, which is the better trade. The one change
worth making is `radar-serve/src/lib.rs`: lift the SIWS sign-in path (lines
231–386) out beside `challenges.rs`, and treat the 79-line test module as the
finding. This is at the edge of the brief's scope — noted, not argued further.

---

## 7. Roadmap

### P0 — the next session is worse without these

**P0.1 — Make the composition-test account true, and pinnable.**
*Problem:* §3.1. Two documents assert a dependency-graph property that has been
false since 2026-08-31, about the invariant `AGENTS.md` rule 1 exists to hold.
*Action:* correct `README.md`:32 and `docs/STATE.md`:184 to the true narrower
claim (nothing invokes the *pipeline* in production; `radar-cli` reaches
`radar-exec::route` for `radar route`); add
`the_documented_dependency_claims_are_true` to `crates/repo-conformance/src/lib.rs`
pinning three rows.
*Rationale:* a claim this load-bearing that is wrong on the day it is written is
the exact failure `repo-conformance` was built for, and the machinery is already
in the file.
*Risk:* the pin needs editing when a real dependency changes. That is the trigger
to re-read the paragraphs, which is the point; the three-row scope keeps it off
the other two thirds of manifest commits.
*Scope:* ~60 lines of test, three prose edits. One session, under an hour.

**P0.2 — Implement `docs/design/0002` §1–§2 (`docs/plans/`).**
*Problem:* §3.3. This document is orphan number twelve.
*Action:* exactly what `0002` §1 specifies — the directory, its plan format, and
the four `AGENTS.md` insertions `0002` §6 already drafted. Nothing new here.
*Rationale:* every other item in this roadmap crosses a session boundary.
*Risk:* `0002`'s own, stated in its own words — a layer with no caller. Its kill
condition (two weeks; delete if handbacks are written and not read) stands.
*Scope:* one session. `0002` is the spec.

### P1 — worth doing next

**P1.1 — `**Status:**` becomes a checked field.**
*Problem:* §3.4 — ten research notes and one design doc carry no status, and the
oldest are the ones most likely to be superseded.
*Action:* backfill eleven documents; add a `repo-conformance` check requiring
`**Status:**` in the first fifteen lines of every file under `docs/adr/`,
`docs/design/` and `docs/research/`, with `superseded by NNNN` required to
resolve. Add a forward pointer to `0007` naming `0024`.
*Rationale:* answers the brief's authority-and-staleness question mechanically.
*Risk:* a status somebody writes to satisfy the check. Mitigated by the value
being constrained: `proposal` / `accepted` / `measured` / `superseded by NNNN`.
*Scope:* ~50 lines of check, eleven small edits.

**P1.2 — `required-checks.txt` consistency check.** §5, item 3. ~40 lines.

**P1.3 — One test file, one owning document.** §5, item 5. Fails on four rows
today; each fix is deleting a duplicated account, which is `AGENTS.md` §2's own
rule. ~30 lines plus the deletions.

**P1.4 — The `LEARNINGS.md` index table.** §4, navigation item 2. ~30 lines, in
the file it indexes.

**P1.5 — The pre-commit hook.** `0002` §5 already specifies it (refuse a commit
on `main`, print the staged diffstat, `core.hooksPath`). Listed for sequencing
only; do not redesign it.

### P2 — worth doing

**P2.1 — Trim `AGENTS.md` §4 rule 1.** Move the 25–35 lines of status about the
customer lane, the budget restart and which components the meter is wired for
into `docs/STATE.md`, which already owns that subject and already says so. Keep
the invariant and the ADR 0007 reasoning, which are policy. *Risk:* this is the
one recommendation with no mechanism behind it; §5 says why it is not decidable,
and it has regressed once already. *Scope:* small, and the measured benefit is
small with it — see §4.

**P2.2 — `LEARNINGS.md` entry shape.** Backfill the nine entries from §3.2 —
either name an artefact or write "nothing catches this" — and add a check that
every `## N.` heading is followed by one or the other. *Stated weakness:* this
enforces shape, not substance, and can be satisfied by writing the words. It is
still worth having, because the nine entries did not decline to answer the
question; they answered a different one, and the format hid the substitution.

**P2.3 — `radar-serve/src/lib.rs`.** §6. Lift SIWS out; the test ratio is the
finding.

### P3 — only if it becomes cheap

`AGENTS.md` §N and "rule N" citations resolving mechanically, in the shape of the
existing ADR-number check. Six documents cite section numbers of a file whose
sections renumber when a rule is added. No failure has been caused by this yet,
which is why it is P3.

---

## 8. Where I am guessing

**That the `AGENTS.md` trim saves anything measurable.** 25–35 lines is roughly
400–500 tokens off an automatic load, and I revised that estimate down by half
while writing §4 — the first count was wrong in the flattering direction. I have
also not measured how often an agent re-reads the file within a session, and
`0002` §7c flags the same figure as its own least evidenced item. *What would
settle it:* one session's token accounting, which is the same measurement `0002`
§3 needs for the `/model` claim. Do them together.

**That pinning three dependency rows is the right width.** I measured the churn —
42 of 134 commits touched a crate manifest — and chose three rows on that basis.
I did not measure how many of those 42 touched *these* three crates. *What would
settle it:* `git log -- crates/radar-{exec,provider,signer}/Cargo.toml`, one
command, before writing the test.

**That a constrained `**Status:**` vocabulary covers the real cases.** Eleven
documents need backfill and I have not tried to assign a value to each. `0003`,
the audit brief, is a document with no obvious status at all. *What would settle
it:* attempt the eleven before writing the check, and if more than two need a
judgement call, the vocabulary is wrong.

**That the LEARNINGS index gets read.** It is the artefact on this list most
likely to be `LEARNINGS` 1's shape — a document nobody opens. *What would settle
it:* the same condition `0002` sets for `docs/plans/`. If two weeks of sessions
cite entries without citing the index, delete the index.

**That the one-test-one-document check is not too blunt.** Four collisions today
all look like genuine duplication, but four is a small sample and the rule
forbids a legitimate bare link. *What would settle it:* write the check, run it,
and read all four failures before deciding whether the rule or the documents are
wrong.

**What I did not audit.** Code quality, per the brief. The `web/` frontend beyond
its CI job. Whether the research notes' *conclusions* are right — `0024` found a
defect in `0024`'s own predecessor by comparing two instruments, and this audit
compared documents against the tree, which is a different and weaker instrument.
The `.github` ruleset itself, which is off-machine.

**And the thing I could not check from here.** Every finding above is a
disagreement between a document and the repository. None of them is a
disagreement between the repository and the world, which is the class that has
actually cost this project money — `LEARNINGS` 10, 11, 18 and 24 are all that
shape, and a structural audit is constitutionally unable to find them. The
recommendation in `0002` §7b — every plan that produces a number carries an item
that computes it a second way — remains the only proposal in this repository
aimed at that class, and nothing in this document substitutes for it.
