<!-- SPDX-License-Identifier: Apache-2.0 -->
# Plan 0001 — Close the gaps design 0004 found

Status: landed — see the handback
Branch: phase-2-3-roast-and-analyst
Base:   9c41555
Planned by: Opus 5, 2026-09-03

## Objective

[Design 0004](../design/0004-documented-versus-enforced.md) scored the repository
8/10 engineering, 7/10 agent-readiness, and named the gap: claims that matter are
carried by prose with no mechanism, and two of them were false on the day they
were written. Afterwards, every claim 0004 identified as load-bearing is either
checked by `repo-conformance` or has been deleted, and state survives a session
boundary in this directory rather than in a transcript.

The goal Josh set is 10/10 on engineering, agent-readiness and long-term
development. That is the whole of 0004 §7, P0 through P2 — not P0 alone.

## Not in scope

- `Policy::CLOSED` and anything in the trading lane. Changing that value is a
  decision about money (AGENTS.md rule 1). This plan only makes the *documents*
  about it true.
- P3 in 0004 §7. It is conditional on the P1 machinery being cheap, and that is
  not yet known.
- The frontend branch (`fix/interface-truth-repairs`). Separate work.

## Items

- [x] 1. Correct the two false dependency sentences
      done: `README.md`:32 and `docs/STATE.md`:184 now state the true narrower
      claim — `radar-cli` depends on `radar-exec` for `radar route`, and nothing
      in production reaches `pipeline::execute`.
- [x] 2. Pin the claim with `the_documented_dependency_claims_are_true`
      done: `cargo test -p repo-conformance` 22 passed at working tree;
      all three rows verified to bite by reverting each in turn — row 1 by
      naming a different crate, row 2 by adding `use radar_exec::submit`, row 3
      by the `radar-pumpfun` dev-dependency it initially caught by accident.
      `cargo clippy -p repo-conformance --all-targets` clean.
- [x] 3. Create `docs/plans/` per design 0002 §1–§2
      done: `docs/plans/README.md` holds the format and the kill condition;
      this file is its first instance and its own dogfood.
- [x] 4. The four AGENTS.md insertions design 0002 §6 drafted
      done: §6 per-file mutants, §7 stage-by-path, §8 sub-agent-is-a-boundary,
      §10 plans live in `docs/plans/`.
- [x] 5. P1.1 — `**Status:**` becomes a checked field
      done: twelve documents backfilled, not eleven — `0008` had a superseded
      blockquote but no status field, so 0004's count was one low. Each status
      says what the note is worth *now*, in a sentence: `0002` is written and
      not sent, `0004`'s base rates are calibration and its block economics
      hold, `0008`'s headline is superseded by `0024`. `0005` also gained the
      SPDX header it was missing.
      `every_numbered_document_declares_a_status` added and verified to bite by
      removing the field from `0015`; `cargo test -p repo-conformance` 23
      passed, clippy clean.
- [x] 5b. How Claude talks to Josh
      done: `AGENTS.md` §9 gained three paragraphs — write for a tired reader,
      keep identifiers exact, every decision comes with a recommendation. Asked
      for by Josh 2026-09-03 mid-session. Also saved to auto-memory as
      `feedback-talk-to-josh-plainly` so it applies outside this repo.
      Not in 0004's roadmap; folded in because it is the same class of fix —
      a standard that lived in a chat message now lives in the file agents read.
- [x] 5c. Hold the roadmap against published evidence
      done: [`0025`](../research/0025-what-the-evidence-says-about-how-this-repository-is-run.md). Josh asked whether any of this actually
      works or merely reads well. Three habits do not survive: `AGENTS.md` at
      519 lines against a study finding context files give no success-rate lift
      and cost 20% more; mutation testing on changed *lines* against Google
      cutting 450 mutants to under 20 by skipping arid code; and a missing
      false-positive bar for `repo-conformance`. The §9 style paragraphs added
      earlier the same day were reverted to one line by their own argument.
      `AGENTS.md` §5 gained the enforcement ladder and the 10% bar; §6 gained
      the arid-code carve-out.
- [x] 5d. P2.1, raised — the real `AGENTS.md` reduction
      done: 519 -> 361 lines, 338 deletions against 167 insertions. Every rule
      kept; three status blocks relocated — the customer-lane composition to
      `docs/STATE.md` (which already says it exists to hold what AGENTS.md
      should not), the spend-meter wiring, which STATE.md already carried, and
      the claw-net documentation story, which LEARNINGS already carried.
      Verified by grepping the new file for each dropped headline phrase.
      `repo-conformance` caught an elided path in this very plan file while
      checking it, which is the check earning its place.
      **Stopped at 361, not the ~150 a secondary source suggested.** Rule 1 and
      rule 3 keep their long precise forms on purpose: both paragraphs record
      that an earlier, terser version of themselves was read as claiming more
      than it did. Cutting those is the exact failure the file exists to stop,
      and the 150 figure is a blog-post number, not one from the paper.
- [x] 5e. Close the `Observed<T>` gap
      done: `Observed<T>` and `AsOf::accept` had no caller outside their own
      crate's tests — LEARNINGS 1/9/10's shape, a mechanism with no caller.
      They had one available: `radar-provider`'s cache hand-wrote the same gate
      as `if !as_of.admits(entry.observed_at)`, an `if` a later edit could
      reorder past the freshness logic without breaking a test.
      `Entry::bytes` is now private and readable only through
      `Entry::bytes(as_of) -> Result<&[u8], LookAhead>`, so the read *is* the
      check. Verified by compiling a violation from another module in the crate:
      `error[E0616]: field `bytes` of struct `cache::Entry` is private`.
      No runtime test added for it — per the ladder, a compiler guarantee is not
      re-tested. AGENTS.md rule 3 updated to say which half is type-enforced and
      which is still four call sites. `-p radar-provider` 38 passed, clippy
      clean, workspace `--all-targets` builds.
- [x] 6. P1.2 — `required-checks.txt` consistency check
      done: `the_required_checks_file_agrees_with_the_justfile_and_the_workflow`
      plus three parsers (`required_checks`, `justfile_recipes`,
      `workflow_contexts`) with their own unit test, because an empty parse
      would make the whole assertion vacuous. Checks both directions: every
      required context is one a job can produce, every command is a real recipe
      or an explicit `github-only:`, and every recipe in the `check` matrix is
      required — the last catches the failure the file's own comment records
      about `web`, a check that ran and gated nothing for days.
      Verified to bite in both directions. 26 passed.
      **The first run reported a false positive and that is the useful part.**
      It claimed `just mutants` had no recipe. It does, at `justfile`:98 — it
      takes parameters (`mutants base="origin/main" shard="":`) and the parser
      read everything before the colon as the name. It also read
      `cargo := env(...)` as a recipe. Both fixed and pinned in the parser test.
      Under the 10% effective-false-positive bar this check would have been
      deleted, not tuned, had the fault been in the rule rather than the parser.
- [x] 7. P1.3 — one test file, one owning document
      done: `one_test_file_is_accounted_for_by_one_document`, over AGENTS.md,
      README.md and docs/STATE.md. `test_paths` normalises `../crates/...` from
      inside `docs/` against `crates/...` from the root — without that the rule
      would pass exactly when the collision is between a root document and a
      `docs/` one, which is every case that has actually occurred.
      0004 counted four violations. Two of them were resolved by the AGENTS.md
      trim in item 5d; the remaining two were `lane_composes.rs` and
      `the_customer_lane_composes.rs` in both README.md and docs/STATE.md.
      README now links docs/STATE.md, which owns the account, rather than the
      tests. Verified to bite. 27 passed, clippy clean.
      Also added the third dependency row 0004 §5 item 1 named and item 2 had
      missed: `radar-provider` must have `radar-agent` among its production
      dependents, because docs/STATE.md says so and that sentence has been
      stale once already.
- [x] 8+11. P1.4 and P2.2 — the `LEARNINGS.md` index, and the entry shape
      done together because the index is only honest once the entries are.
      0004 §3.2 measured the split: entries 1-19 used
      `**What catches a recurrence:**` and named an artefact; 20-28 used
      `**The check:**` and named a habit; five (16, 22, 24, 25, 26) had neither.
      All 28 now use the one header. Eight say `nothing mechanical` and then
      name the habit — which is the file's own opening standard
      ("or says plainly that nothing does"), not a gap in it. Four of the five
      that had neither turned out to have a real artefact nobody had named:
      16's kept run, 24's `radar route`, 25's fixture-first ordering, 26's
      `mutants` gate job.
      The index is a 28-row table at the top, generated from the headings and
      the mechanism lines, with the mechanical/habit split stated. Anchor links
      pass the existing link check.
      `every_learnings_entry_names_what_catches_a_recurrence_and_is_indexed`
      holds both halves. Verified to bite on a new entry with no mechanism line
      and on an entry with no index row. 28 passed, clippy clean.
- [x] 9. P1.5 — both halves
      done (first half, earlier): `untracked_documents()` and
      `no_markdown_document_is_invisible_to_these_checks`.
      done (second half): `scripts/hooks/pre-commit` plus a `just hooks` recipe
      that sets `core.hooksPath` — the hook is the file in the tree, so
      reviewing it is reviewing a diff and changing it needs no reinstall.
      It refuses a commit on `main` and prints the staged diffstat, with
      deletions listed separately because that is the change least likely to
      look wrong in a diffstat nobody opened — the one that lost `AGENTS.md` on
      2026-09-02. **It fails open** on everything else; a hook that refuses for
      its own reasons is worse than no hook, and this one runs on Josh's
      machine.
      Verified in a throwaway repository, all three paths: refused on `main`
      with no commit created, allowed on a branch with the diffstat printed,
      and the deletion warning shown. Installed here (`core.hooksPath` was
      unset before); `git config --unset core.hooksPath` undoes it.
- [x] 10. P2.1 — done as item 5d, raised. Left here so the roadmap's own
      numbering still resolves.
- [x] 11. P2.2 — done as item 8+11, together with the index, because an index
      over entries that do not share a shape is an index of nothing.
- [x] 12. P2.3 — `radar-serve/src/lib.rs`
      done: Sign-In With Solana lifted into
      [`crates/radar-serve/src/siws.rs`](../../crates/radar-serve/src/siws.rs) —
      both handlers, both request bodies, `nonce_in`, `domain_from`,
      `random_nonce`, and the four tests that were theirs. `lib.rs` goes
      1,366 code lines to 1,216; the new module is 182 code against 57 of test,
      and its tests are visibly its tests rather than four of twenty-six in a
      file about everything.
      **It exposed a defect that was already there.** Two doc-comment blocks —
      one for `guard`, one for `customer_wallet` — were stacked with no item
      between them and were being silently absorbed onto `ChallengeBody`. So
      the auth layer and the wallet endpoint, the two things in this crate most
      worth documenting, had their documentation attached to a struct that is
      three fields of deserialisation. Both reattached.
      `-p radar-serve` 196 passed; workspace 89 test binaries green, clippy
      clean `--all-targets`, `cargo fmt --check` clean.
      **Not claimed:** `lib.rs` is still 1,216 code lines against 26 of test.
      0004 §3.5's finding was the ratio in the internet-facing crate, and
      lifting SIWS improves it without fixing it. `api.rs` at 1,784 lines and
      `access.rs` at 1,279 are the next two, and neither is in this plan.

## Open questions for Josh

- Q1 (2026-09-03): design 0002 is still marked "proposal, for Josh to accept,
  change or reject". Items 3 and 4 implement its §1–§2 and §6 because 0004 §7
  ranked them P0. If that pre-empts a decision that was yours, say so and they
  revert cleanly — nothing else depends on them. — unanswered
- Q2 (2026-09-03): `target/` is 34GB, past the justfile's 20GB warn threshold.
  `just tidy` when convenient. 222GB free, so nothing is at risk. — unanswered

## Handback

Stopped at: **nothing outstanding in this plan.** Items 1 through 12 are done,
committed and pushed on `phase-2-3-roast-and-analyst`. Design 0004's roadmap is
complete, P0 through P3, plus three items that came out of `0025` and were not
in it.

What exists now that did not: `repo-conformance` is 28 tests, up from 21. Each
of the seven new ones was verified by breaking the thing it asserts and watching
it fail, and two of them found real defects on their first run — an elided path
in this plan file, and a parser fault in the check itself, which is recorded in
the commit rather than quietly fixed.

Next action: nothing here. If picking this up cold, the open work is elsewhere —
`docs/STATE.md`'s "Where to start", and the three blockers on
`fix/interface-truth-repairs` that need Josh.

Do not: touch `Policy::CLOSED`. Do not commit onto local `main` — `just hooks`
now refuses it, but the hook can be bypassed and the refusal is the point.

**The one date to keep.** `docs/plans/` carries design 0002's kill condition and
this plan is its first and only instance. **By 2026-09-17, if handback blocks are
being written and not read, delete the directory.** Nothing about having built it
makes it worth keeping — that is the whole argument of LEARNINGS 1, 9 and 10, and
this file is not exempt from it.
