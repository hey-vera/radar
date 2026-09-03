<!-- SPDX-License-Identifier: Apache-2.0 -->
# Plan 0001 — Close the gaps design 0004 found

Status: in progress
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
      done: `docs/research/0025-...md`. Josh asked whether any of this actually
      works or merely reads well. Three habits do not survive: `AGENTS.md` at
      519 lines against a study finding context files give no success-rate lift
      and cost 20% more; mutation testing on changed *lines* against Google
      cutting 450 mutants to under 20 by skipping arid code; and a missing
      false-positive bar for `repo-conformance`. The §9 style paragraphs added
      earlier the same day were reverted to one line by their own argument.
      `AGENTS.md` §5 gained the enforcement ladder and the 10% bar; §6 gained
      the arid-code carve-out.
- [ ] 5d. P2.1, raised — the real `AGENTS.md` reduction
      next: 0004 proposed a 30-line trim. `0025` §1b argues for far more, with
      a rule for what belongs: `AGENTS.md` holds what an agent must know
      *before* it can safely act. Status moves to `docs/STATE.md`; anything a
      check enforces becomes a pointer, not a restatement. **Rule 1 and the
      money-touching invariants do not move.** The file is 532 lines today —
      it went *up* by 13 while writing the note that says to cut it, which is
      the argument's own evidence.
- [ ] 5e. Close the `Observed<T>` gap or downgrade the claim
      next: `AGENTS.md` invariant 3 says the watermark cannot be unwrapped;
      enforcement is a hand-written `as_of.admits()` filter in `reader.rs`.
      By the ladder this is a level-4 claim about a level-1 guarantee. Either
      make it true in the type or stop saying it.
- [ ] 6. P1.2 — `required-checks.txt` consistency check (~40 lines)
- [ ] 7. P1.3 — one test file, one owning document. Fails on four rows today.
- [ ] 8. P1.4 — the `LEARNINGS.md` index table (~30 lines)
- [~] 9. P1.5, second half — the invisible-document hole
      done: `untracked_documents()` and
      `no_markdown_document_is_invisible_to_these_checks` refuse when a
      markdown file exists that `git ls-files` cannot see; `.gitignore` is the
      escape hatch. Verified to bite with a scratch file. 24 passed, clippy
      clean.
      next: the other half — `scripts/hooks/pre-commit` plus
      `git config core.hooksPath scripts/hooks`, refusing a commit on `main`
      and printing the staged diffstat. Design 0002 §5 specifies it.
- [ ] 10. P2.1 — trim `AGENTS.md` §4 rule 1's status prose into STATE.md
- [ ] 11. P2.2 — backfill the nine `LEARNINGS.md` entries that name a habit
      rather than an artefact
- [ ] 12. P2.3 — `radar-serve/src/lib.rs`: 1,366 code lines against 79 of test,
      in the internet-facing crate. Lift SIWS out. The test ratio is the
      finding, not the line count.

## Open questions for Josh

- Q1 (2026-09-03): design 0002 is still marked "proposal, for Josh to accept,
  change or reject". Items 3 and 4 implement its §1–§2 and §6 because 0004 §7
  ranked them P0. If that pre-empts a decision that was yours, say so and they
  revert cleanly — nothing else depends on them. — unanswered
- Q2 (2026-09-03): `target/` is 34GB, past the justfile's 20GB warn threshold.
  `just tidy` when convenient. 222GB free, so nothing is at risk. — unanswered

## Handback

Stopped at: item 6. Items 1–5 and half of 9 are on disk and green; **nothing is
committed** — Josh asked to be asked first. `repo-conformance` is 24 tests, up
from 21, and each of the five new rows was verified to fail when the thing it
asserts is broken.
Next action: item 6, `required-checks.txt` (0004 §5 item 3, ~40 lines) — read
`required-checks.txt` against the justfile recipe names and `ci.yml`'s job
names, and assert they agree. Then item 7, which fails on four rows today and so
needs the four collisions resolved before the check can go green.
Do not: touch `Policy::CLOSED`, and do not commit onto local `main` — this work
belongs on `phase-2-3-roast-and-analyst`.
