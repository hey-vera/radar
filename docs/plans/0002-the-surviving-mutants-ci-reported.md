<!-- SPDX-License-Identifier: Apache-2.0 -->
# Plan 0002 — The surviving mutants CI reported

Status: landed — all 13 checks green on 6c0ad34
Branch: phase-2-3-roast-and-analyst
Base:   b4262f9
Planned by: Opus 5, 2026-09-03

## Objective

`mutants-shards` has failed on this branch since `6c68a80`, with 37 survivors
across three files. Afterwards every one is either killed by a test that fails
when the mutation is applied, or recorded in `.cargo/mutants.toml` as equivalent
with the reason — and PR #112 is not blocked by a check that has been red for
three commits.

## Not in scope

- The `web` job, which failed for the first time on `b4262f9` and is not ours:
  `npm audit` hit npmjs's quick-audit endpoint, which is being retired, and got a
  400. The lockfile is `lockfileVersion: 3` with 242 entries, every one resolving
  to `registry.npmjs.org`, no aliases and no links — so the tree npm called
  invalid is valid. See Open questions.
- The four survivors in `radar-cli`'s `run` are **item 5**, in their own commit.
  They were first recorded as a separate task, on the reading that AGENTS.md §3
  forbids widening. It forbids widening *the same change*; a separate commit is
  a different change, so the rule never applied.
- `Policy::CLOSED` and anything in the trading lane.

## Items

- [x] 1. `crates/radar-cli/src/roast.rs` — 24 survivors, all in `today()`
      done: **the cause was a second copy of the algorithm.** `from_days` was
      duplicated inside the test module, so the tests exercised the copy and no
      mutation of the real arithmetic could fail anything. LEARNINGS 18's shape
      applied to a test — two instruments compared as if they were one, except
      only one of them ran.
      `from_days` is now production code with one definition; `today()` is the
      clock and nothing else. The test pins every branch and constant the
      algorithm has: the March pivot either side (1970-02-28/03-01), an ordinary
      leap day (2024), the divisible-by-400 case (2000), the divisible-by-100
      case that is *not* a leap year (2100), a year end, and two pre-epoch dates
      so `div_euclid`/`rem_euclid` are exercised on a negative `z`.
      Three of my own day numbers were off by one and were corrected against a
      real calendar before the test was run — which is the same class of error
      the test exists to catch.
      `cargo mutants -f crates/radar-cli/src/roast.rs`: **64 caught, 4 missed**
      (the four in `run`, above), down from 28 missed.
- [x] 2. `crates/radar-roast/src/baserates.rs` — 8 survivors
      done: `band_for`'s `b.hi - b.lo` survived because the published snapshot
      lists its narrow bands first, so `min_by_key` gave the right answer with
      the wrong key. The new test builds bands wide-first and includes the pair
      where a sum ties (1..=9 and 5..=5 both sum to 10) and the pair where a
      ratio ties. Plus the staleness boundary either side of `STALE_AFTER_DAYS`,
      the `||` in `days`'s validation shown to reject a bad month *or* a bad day
      rather than needing both, and the `y * 372 + m * 31 + d` coefficients shown
      to keep each field outranking the one below it.
      `cargo mutants -f ...baserates.rs`: **37 caught, 0 missed**, 4 unviable.
- [x] 3. `crates/radar-roast/src/fidelity.rs` — 5 survivors
      done so far: two tests. `the_exact_match_is_a_difference_and_not_a_sum_or_a_ratio`
      uses two values 1e-10 apart that round to different grid points at the
      precision written, so only the `(a - value).abs()` branch can pass it — a
      sum or a ratio is ~1.0 and fails.
      `the_epsilon_is_exclusive_and_the_boundary_is_where_it_says` puts the
      difference at exactly 1e-9, which is the only place `<` and `<=` disagree.
      `round_to`'s `/` → `*` is **equivalent** and is recorded in
      `.cargo/mutants.toml` rather than tested: its one caller compares two of
      its results for equality, both sides go through the same strictly monotone
      transform, so both forms decide the same question. Applied by hand, whole
      crate passes identically — that file's own rule for adding an entry.
      confirmed: all five CI-flagged mutants appear in `mutants.out/caught.txt`
      (`74:15` twice, `74:30` twice, `97:26`), with 44 caught and **0 missed**
      at mutant 45 of 83.
- [x] 5. `crates/radar-cli/src/run` — the four pre-existing survivors
      done: three decisions extracted so a mutation of each is visible —
      `mint_arg_from` (the guard that stops `--rpc` being read as the mint),
      `wants_sheet`, `needs_newline` — plus
      `run_refuses_before_it_reaches_the_network`.
      **That last one is the important one and it is cheaper than it looked.**
      `run` parses the mint *before* it builds an RPC client, so a bad address
      returns `Err` with no network and no fixture — and that is what stops the
      whole body being replaceable with `Ok(())`. A `radar roast` that printed
      nothing and exited zero would look exactly like success, which is
      LEARNINGS 5 in the command a public bot runs.
      7 tests in the module, clippy clean.
- [x] 6. Prevent the two classes that had no mechanism
      done: **a line ceiling on `AGENTS.md`** (400, with a floor of 150 because
      a ceiling alone is satisfied by an empty file, and that file has been
      deleted by accident once). It makes `0025` §1's finding binding instead of
      advisory — the number moving is the signal, and raising it has to be
      argued for in the same commit. `CLAUDE.md` is pinned as an import.
      And **`scripts/hooks/pre-push`**, which prints how the branch's last CI
      run finished. This plan's whole subject is a check that caught a defect
      immediately and reported it to a page nobody opened. It **never blocks** —
      a red previous run is often exactly why you are pushing — and it is silent
      when it cannot know: no `gh`, no network, no runs. An absent answer is not
      a red answer. It also tells `cancelled` apart from `failed`, which
      AGENTS.md §6 records a lost session to.
      Verified in a throwaway worktree on `phase-1-radar-dossier`, whose last
      run genuinely failed: it named all five failing jobs and exited 0; silent
      on a branch CI has never seen; silent with `gh` off the PATH.
      **A third check was designed and then refused.** Flagging any function
      defined both in production code and in the same file's test module would
      have caught item 1's duplicate directly. Run across the repository it hits
      13 files, of which 1 is a real duplicate and 12 are ordinary test fixtures
      and stub trait impls. That is a ~93% effective false-positive rate against
      the 10% bar adopted in `AGENTS.md` §5 the same day, so the rule applies to
      its own author: delete it rather than tune it. Mutation testing is the
      right detector for that class and it worked.
- [x] 4. Push, and read the CI result rather than assuming it

      **Stopped verifying locally on purpose.** Three serial whole-file
      `cargo mutants` runs took most of an hour on the owner's workstation, and
      AGENTS.md §8 says plainly where a full run belongs: *"Move it, do not skip
      it. CI runs the mutation check sharded across four runners."* The local
      runs had already done their job — diagnosis, and proof that every
      CI-flagged mutant is caught. The remainder are mutants CI never flagged,
      on four runners in parallel rather than one laptop in series.

## Open questions for Josh

- Q1 (2026-09-03): the `web` job, and it is **diagnosed rather than guessed at**,
  so this may need no answer. `npm audit --audit-level=high` failed with a 400
  from `/-/npm/v1/security/audits/quick` and the notice *"This endpoint is being
  retired"*. npmjs retired the legacy audit endpoints in favour of
  `/-/npm/v1/security/advisories/bulk`; the npm CLI already uses bulk and only
  falls back to quick when the bulk call fails. So the quick 400 is the *second*
  failure, not the first.

  Evidence it is transient rather than structural: `web` passed at 21:15 and
  failed at 23:02 with no change to `web/` in between; `npm ci` in the failing
  run took **seven minutes**, which is registry trouble on its own; the lockfile
  is `lockfileVersion: 3`, 242 entries, all resolving to `registry.npmjs.org`
  with no aliases or links; and `npm audit --package-lock-only
  --audit-level=high` run here against that same lockfile returns **found 0
  vulnerabilities**, exit 0, on npm 11.9.0.

  So: re-run, which the next push does anyway. If it recurs, the fix is to make
  the runner's npm new enough to stay on bulk, as a change of its own. **The one
  thing not to do is `|| true`** — that turns a security check into decoration,
  which AGENTS.md §6 forbids by name. — no answer needed unless it recurs

## What it took, and why that is the interesting part

Seven rounds. Only **two** were a missing test.

| Round | What the failure actually was |
|---|---|
| 1 | 37 survivors — real gaps, including a test reading a copy of the code |
| 2 | 4 survivors — three of them *my* fixes, verified against the wrong expression |
| 3 | 0 survivors, 2 timeouts — `test_paths` hangs |
| 4 | 0 survivors, 2 timeouts — `fidelity` hangs; **one reclaimed runner** |
| 5 | 4 survivors + 1 timeout — `decode_base58`, a loop shape the grep missed |
| 6 | 14 survivors — **shard 0's first ever report** |
| 7 | green |

Three distinct classes wearing one uniform on the dashboard:

**Missing tests** (rounds 1, 2, 5, 6). Ordinary, and what mutation testing is
for.

**Code shape** (3, 4, 5). Hand-rolled index scanners are one mutation from an
infinite loop, and mutation testing reports that by *hanging for five minutes*
rather than by failing. The fix is never a test — it is bounding the loop so the
state cannot exist. Every instance in `crates/*/src` is now bounded.

**Infrastructure** (4, and shard 0 throughout). A reclaimed runner and a
cancelled job look exactly like a failure until you read the log. AGENTS.md §6's
rule — establish that it *ran* — earned its place three times.

**And one that was mine.** Shard 0 was cancelled in seven of seven runs, every
time by my own next push, so a quarter of the mutants went unchecked all session
while I "fixed" the other three. It reported once and held fourteen, in the
files that read untrusted input and meter spending. `AGENTS.md` §6 forbids
pushing over an in-flight check; I broke it every round, each time for a good
local reason, which is exactly how a rule erodes.

## Handback

Stopped at: **nothing outstanding.** All 13 checks green on `6c0ad34`; the four
mutation shards each complete in under four minutes, down from fourteen.

Next action: nothing here. `docs/design/0005` on branch `research/graph-vs-0024`
is unmerged and needs a decision from Josh — it proposes no code.

Do not: raise `--timeout` back to 300. It is 60 because the loops are bounded
now, and a timeout is a signal rather than a cost. If one fires, fix the loop.
