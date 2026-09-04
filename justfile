# SPDX-License-Identifier: Apache-2.0
#
# Every check this repository runs, defined once.
#
# The recipes below are named for the required status checks, and the workflows
# in .github/workflows/ invoke them rather than restating them. That is the
# point: a claim about what CI runs can only be kept true by being the thing CI
# runs.
#
# Local prerequisites beyond the Rust toolchain:
#
#   cargo deny     cargo install --locked cargo-deny
#
# On Windows under MSYS or Git Bash the default host toolchain resolves to an
# MSVC target where MSYS `link` shadows MSVC's linker and every build fails at
# the link step. Export the toolchain that works on your machine:
#
#   export RADAR_CARGO="cargo +stable-x86_64-pc-windows-gnullvm"

set shell := ["bash", "-euo", "pipefail", "-c"]

# `-D warnings` is the workspace's real lint posture, so it belongs here rather
# than in workflow env where a local run would never see it.
export RUSTFLAGS := env("RUSTFLAGS", "-D warnings")

cargo := env("RADAR_CARGO", "cargo")

# A floor, not a target. Raise it as the suite grows; lowering it to make a run
# pass is the failure this guards against.
#
# Raised 1069 -> 1503 on 2026-09-04. It had drifted 434 behind the suite, which
# is most of a third of it: a floor that far below the real count would let
# every test in four crates be deleted without the number moving, and a floor
# that cannot notice that is decoration. Raising it is part of adding tests,
# not a separate chore.
#
# The count is platform-independent here, which is what makes a single number
# safe: nothing in the workspace is `cfg(windows)` or `cfg(unix)` gated except
# one function in `radar-exec`'s signer client, and it carries no tests. So a
# Linux runner cannot count fewer than a Windows workstation.
#
# Lowered 1503 -> 1476 in the same session, and this is the one shape of change
# that may do that: `radar-provider`'s cache, breaker and planner were deleted
# for having no caller, and 27 tests went with the code they tested. The floor
# caught it, which is the floor working -- it cannot tell a deletion from a
# regression, and it is not supposed to.
#
# What separates this from the failure this guards against is that the drop is
# argued in the commit that causes it and matches a counted deletion. Lowering
# it to make a red run green is the thing that must never happen; if the number
# has to come down, the commit says which tests went and why they went with
# their subject.
export MIN_TESTS := "1591"

_default:
    @just --list --unsorted

# The four fast checks, for the edit-compile loop. Not a substitute for `just ci`.
check: _disk build tests lint fmt

# Everything runnable off a GitHub runner.
ci: build tests lint fmt cargo-deny licence-headers

# --- required checks, one recipe per status-check context ---------------------

# Compile every target, with the lockfile as committed.
build:
    {{ cargo }} build --all-targets --locked

# Unit tests and doctests. The doctest on radar-provider is the worked example
# of the cost model, so it is load-bearing documentation as well as a test.
#
# The floor is deliberate. A summary that prints nothing when the build fails
# reads, at a glance, exactly like a summary that prints nothing because
# everything passed -- and a commit went out that way once already
# (LEARNINGS entry 5). Asserting a minimum makes absence loud.
tests:
    #!/usr/bin/env bash
    set -euo pipefail
    output=$({{ cargo }} test --locked 2>&1) || { echo "$output"; exit 1; }
    echo "$output"
    passed=$(echo "$output" | awk '/^test result: ok\./ { s += $4 } END { print s + 0 }')
    echo "--- ${passed:-0} tests passed ---"
    if [ "${passed:-0}" -lt "$MIN_TESTS" ]; then
        echo "only ${passed:-0} tests ran; expected at least $MIN_TESTS." >&2
        echo "Either tests were skipped or the harness is lying. Raise MIN_TESTS in" >&2
        echo "the justfile when the suite grows; never lower it to make this pass." >&2
        exit 1
    fi

# Pedantic clippy, denied. The workspace lint table sets the levels; this runs them.
lint:
    {{ cargo }} clippy --all-targets --locked -- -D warnings

fmt:
    {{ cargo }} fmt --check

# Mutation testing over the changed lines.
#
# The answer to the thing MIN_TESTS cannot see. A floor catches tests being
# *deleted*; it says nothing about an assertion *loosened in place* --
#
#     assert_eq!(v, Verdict::Blocked);   ->   assert!(matches!(v, _));
#
# -- which leaves the count identical and the coverage identical and the suite
# green. A mutant is a small edit to the implementation; if the tests still pass
# with it in place, they do not constrain that behaviour.
#
# This session found two such holes by hand: one test in `watermark_holds.rs` was
# decorative, and nothing at all constrained the no-route ceiling in the capacity
# search. Both took ninety seconds to find once somebody thought to look, and
# neither would ever have been found by counting.
#
# `--in-diff` scopes it to what the branch changed, so cost tracks the size of
# the change rather than the size of the repository.
#
# A timeout is `inconclusive`, never a pass: mutation testing runs the suite once
# per mutant, and an infinite loop introduced by a mutant looks exactly like a
# slow one. Reporting that as "caught" would be the check lying in the direction
# that feels good.
mutants base="origin/main" shard="":
    #!/usr/bin/env bash
    set -euo pipefail
    if ! command -v cargo-mutants >/dev/null 2>&1; then
        echo "cargo-mutants is not installed:" >&2
        echo "  cargo install --locked cargo-mutants" >&2
        exit 127
    fi
    # No fetching. A recipe that changes the repository it is run in is a recipe
    # that can damage it: an earlier draft used `--depth=1` and left the working
    # clone shallow, which silently broke `merge-base` for every command after
    # it. CI checks out with `fetch-depth: 0`, and a local run uses the history
    # that is already there.
    if ! git rev-parse --verify --quiet {{ base }} >/dev/null; then
        echo "{{ base }} is not in this repository." >&2
        echo "Fetch it yourself, then re-run -- this recipe will not touch your git state." >&2
        exit 1
    fi
    # A real file rather than `<(...)`: process substitution hands the tool a
    # /dev/fd path, which it cannot open on every platform this recipe has to run
    # on. Verified by it failing that way first.
    # `git diff` does not see untracked files, so a brand-new module is invisible
    # to `--in-diff` and passes without being mutated at all. That happened: a
    # local run reported 28 mutants and CI, where the files were committed,
    # found 74 and four misses in the new code. A check that reports absence the
    # same way it reports success is not a check -- LEARNINGS 5, in the tooling
    # rather than in the code.
    #
    # Refusing rather than staging them, because a recipe that changes the
    # repository it is run in is a recipe that can damage one.
    untracked=$(git ls-files --others --exclude-standard -- '*.rs')
    if [ -n "$untracked" ]; then
        echo "untracked Rust files are invisible to \`git diff\` and would NOT be mutated:" >&2
        echo "$untracked" | sed 's/^/  /' >&2
        echo "" >&2
        echo "Stage them first (\`git add -N <path>\` is enough), or this check" >&2
        echo "passes without having looked at your new code." >&2
        exit 1
    fi

    diff_file=$(mktemp)
    trap 'rm -f "$diff_file"' EXIT
    # Merge-base on the left so the scope is what this branch changed rather than
    # everything that has landed on the base since. Working tree on the right --
    # not HEAD -- because `--in-diff` matches the diff against the *source it is
    # mutating*, and a diff of committed state against a tree with uncommitted
    # edits is rejected as stale. In CI the two are the same thing.
    merge_base=$(git merge-base {{ base }} HEAD)
    git diff "$merge_base" > "$diff_file"
    if [ ! -s "$diff_file" ]; then
        echo "no changes against {{ base }}; nothing to mutate."
        exit 0
    fi
    # Sharded, because this check scales with the size of the diff and nothing
    # else in CI does. An early branch here produced 28 mutants; a forty-commit
    # one produced 408, which never once finished inside a runner's life -- every
    # attempt was killed part-way, reporting nothing, which is the worst failure
    # a check can have because it looks identical to a real finding.
    #
    # `--shard k/n` splits the *set*, so every mutant is still tested; the work
    # is spread across parallel jobs rather than dropped. `--jobs 2` inside each
    # shard rather than the runner's four cores: each job builds the workspace,
    # so the limit is memory, and a job that OOMs intermittently fails in a way
    # that looks like a finding too.
    shard_arg=""
    if [ -n "{{ shard }}" ]; then
        shard_arg="--shard {{ shard }}"
    fi
    # `--jobs 1`. Two parallel jobs means cargo-mutants keeps two complete
    # source-and-target copies, and a hosted runner has about 14GB free -- a
    # workspace target directory is several of those. Shards died mid-run with
    # no error and no exit code, which is what a runner running out of room
    # looks like from inside the job.
    #
    # Sharding is what buys the parallelism now, across machines that each have
    # their own disk, rather than inside one.
    # `--timeout 60`, lowered from 300 on 2026-09-03. That budget existed
    # because a mutant that hangs was an expected outcome: several loops in this
    # workspace were a single mutation away from never terminating, and each one
    # cost a runner five full minutes to report nothing useful. Two of them took
    # ten minutes of a fourteen-minute shard in one run.
    #
    # Those loops are bounded now, so a timeout is a signal rather than a cost of
    # doing business. If this fires, the mutant found a loop that can be made not
    # to terminate -- fix the loop, do not raise the number back. Raise it only
    # for a genuinely slow *test*, and say which test in the same commit.
    {{ cargo }} mutants --in-diff "$diff_file" --jobs 1 $shard_arg         --timeout 60 --minimum-test-timeout 60 -- --offline

# Advisories, licences, and source provenance.
cargo-deny:
    {{ cargo }} deny check

# Every source file carries an SPDX header. Cheap to check, easy to forget.
# Every source file carries an SPDX header. Cheap to check, easy to forget.
#
# TypeScript is included, not exempted. The interface is compiled into the same
# binary as everything else and ships under the same licence, and a check that
# covered only one language would be a check that silently stopped covering the
# repository the day a second one arrived -- which is the day it did.
#
# `web/dist` and `node_modules` are build output and other people's code.
licence-headers:
    #!/usr/bin/env bash
    set -euo pipefail
    missing=0
    while IFS= read -r f; do
        if ! head -3 "$f" | grep -q "SPDX-License-Identifier"; then
            echo "missing SPDX header: $f"
            missing=1
        fi
    done < <(
        find crates -name '*.rs' -not -path '*/target/*'
        find web -name '*.ts' -o -name '*.tsx' 2>/dev/null             | grep -v node_modules | grep -v '/dist/' || true
    )
    exit $missing

# The floor the frontend suite may not fall below.
#
# The same discipline `MIN_TESTS` applies to the Rust side, for the same reason:
# a suite that quietly shrinks is a suite somebody deleted a test from, and the
# number going down is the only thing that says so. Raise it when tests are
# added; never lower it to make a run go green.
export MIN_WEB_TESTS := "173"

# The interface: install exactly the locked dependencies, check them for known
# advisories, type-check, test, and build.
#
# `npm ci` rather than `npm install`, because the lockfile is the point: it is
# what makes a build reproducible and what `npm audit` has an opinion about.
#
# Not part of `just ci`. The Rust suite must stay runnable without a Node
# toolchain -- backend work should not require one -- so this is a separate
# recipe with its own CI job, and `web/dist/.gitkeep` is what lets the crate
# compile when nobody has run it.
web:
    #!/usr/bin/env bash
    set -euo pipefail
    cd web
    npm ci
    # `npm audit`, retried -- but only when it could not *run*.
    #
    # A vulnerability and an unreachable registry are different answers and this
    # must not confuse them. The exit code alone does, so the transport failure
    # is matched on the output: npm prints `audit endpoint returned an error`
    # when the advisory endpoint refuses it, and says nothing of the kind when it
    # has actually found something. A findings failure exits on the first
    # attempt, unretried.
    #
    # Three different transport failures were seen in seven runs on 2026-09-03/04
    # -- a bulk-endpoint timeout, a 400 from the retired quick endpoint that npm
    # falls back to, and a 503 -- each costing a hand re-run of a job that says
    # nothing about this repository.
    #
    # `|| true` is deliberately not the fix. That turns a security check into
    # decoration, which AGENTS.md §6 forbids by name. This keeps the gate and
    # retries the network.
    audit_log=$(mktemp)
    for attempt in 1 2 3; do
      if npm audit --audit-level=high 2>&1 | tee "$audit_log"; then
        break
      fi
      if ! grep -q 'audit endpoint returned an error' "$audit_log"; then
        echo "--- npm audit found something; not a transport failure, not retrying ---" >&2
        exit 1
      fi
      if [ "$attempt" = 3 ]; then
        echo "--- npm audit could not reach the registry in 3 attempts ---" >&2
        echo "--- that is not a clean audit; it is an unknown one, and it fails ---" >&2
        exit 1
      fi
      echo "--- npm audit could not reach the registry (attempt $attempt); retrying ---" >&2
      sleep $((attempt * 15))
    done
    # `NO_COLOR` because the count is grepped out of this, and vitest wraps the
    # number in ANSI escapes that a naive pattern reads straight past -- which
    # would leave the floor comparing against an empty string forever.
    NO_COLOR=1 npm run test 2>&1 | tee /tmp/radar-web-tests.log
    # The count, checked rather than trusted. `vitest run` exits zero when it
    # finds no test files at all, so a broken `include` pattern would turn the
    # whole suite off and still go green -- which is the failure LEARNINGS 5
    # records one layer up: a check that reports absence the same way it reports
    # success.
    # `|| true` on both: under `pipefail` a grep that matches nothing exits
    # non-zero and takes the recipe with it, which would report "no tests" as a
    # crash rather than as the floor failing.
    passed=$(grep -oE 'Tests +[0-9]+ passed' /tmp/radar-web-tests.log || true)
    passed=$(echo "$passed" | grep -oE '[0-9]+' | head -1 || true)
    passed=${passed:-0}
    if [ "$passed" -lt "$MIN_WEB_TESTS" ]; then
      echo "--- $passed web tests passed, floor is $MIN_WEB_TESTS ---" >&2
      exit 1
    fi
    echo "--- $passed web tests passed ---"
    npm run build


# --- keeping the machine usable ----------------------------------------------
#
# `target/` only ever grows. Cargo never garbage-collects it, every distinct set
# of flags keeps its own artefacts, and a mutation run multiplies both. On
# 2026-09-03 it reached 127GB on a disk with 130GB free and the machine froze
# hard enough to need a forced power-off.
#
# So the tooling enforces this rather than a document asking someone to remember.

# Gigabytes of `target/` that trigger a warning, and a refusal.
warn_gb := "20"
stop_gb := "40"

# Refuses to build on top of a `target/` that is about to fill the disk.
#
# A prerequisite of `check` rather than advice, because the failure it prevents
# is one where the machine stops responding -- at which point nobody is reading
# advice. Fixing it is one command and costs only a rebuild.
_disk:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ ! -d target ]; then exit 0; fi
    gb=$(du -sk target 2>/dev/null | awk '{print int($1/1048576)}')
    if [ "${gb:-0}" -ge {{ stop_gb }} ]; then
        echo "target/ is ${gb}GB, at or past the {{ stop_gb }}GB ceiling." >&2
        echo "" >&2
        echo "Cargo never prunes this directory: every distinct set of flags keeps" >&2
        echo "its own artefacts and nothing removes the stale ones. Run:" >&2
        echo "" >&2
        echo "  just tidy" >&2
        echo "" >&2
        echo "It costs a rebuild and nothing else." >&2
        exit 1
    fi
    if [ "${gb:-0}" -ge {{ warn_gb }} ]; then
        echo "note: target/ is ${gb}GB. Run 'just tidy' before it becomes a problem." >&2
    fi

# What this checkout is costing the disk.
disk:
    #!/usr/bin/env bash
    set -euo pipefail
    for d in target target/debug target/release mutants.out mutants.out.old; do
        [ -e "$d" ] && du -sh "$d" 2>/dev/null || true
    done
    echo ""
    df -h . 2>/dev/null | tail -1 || true

# Reclaims the build cache and stops anything left running.
tidy:
    #!/usr/bin/env bash
    set -euo pipefail
    # Only ever touches build output -- `target/debug` and the mutation scratch
    # directories -- all gitignored and regenerable. Nothing here can lose work.
    # Stray cargo and rustc processes outlive the command that started them,
    # especially a backgrounded one, and they hold both CPU and the files below.
    # Killed first, or the removal races them.
    if command -v taskkill >/dev/null 2>&1; then
        taskkill //F //IM cargo.exe //T >/dev/null 2>&1 || true
        taskkill //F //IM rustc.exe //T >/dev/null 2>&1 || true
        taskkill //F //IM cargo-mutants.exe //T >/dev/null 2>&1 || true
    else
        pkill -f cargo-mutants >/dev/null 2>&1 || true
        pkill -x rustc >/dev/null 2>&1 || true
    fi
    before=$(du -sk target 2>/dev/null | awk '{print int($1/1048576)}' || echo 0)
    rm -rf target/debug mutants.out mutants.out.old
    echo "freed ~${before}GB of build cache; the next build starts cold."

# Points git at the versioned hooks. Run once per clone.
hooks:
    #!/usr/bin/env bash
    set -euo pipefail
    # `core.hooksPath` rather than copying into `.git/hooks`: the hook is then
    # the file in the tree, so a change to it takes effect without reinstalling
    # and a review of the hook is a review of the diff.
    git config core.hooksPath scripts/hooks
    chmod +x scripts/hooks/* 2>/dev/null || true
    echo "hooks: $(git config --get core.hooksPath)"
    echo "undo with: git config --unset core.hooksPath"

# --- operator commands --------------------------------------------------------

# Design 0006 section 6 asked a fresh session to remember to read four things in
# order. That is prose, the weakest rung of the ladder in AGENTS.md section 5 --
# and the very session that wrote it produced a table whose row 6 pointed at a
# branch that had merged the day before. This reads each fact from the file that
# owns it at the moment it prints, so there is no second copy to go stale.
#
# Where a session starts: branch, its last CI result, the plan in flight, and what decays.
orient:
    @bash scripts/orient.sh

# What the system is doing right now: ingestion lag, store contents, the serving
# surface, and what the trading lane is authorised to do. Reads live state, so it
# can never be stale.
#
# `serve` defaults to empty, and an empty endpoint makes the serving check report
# Unknown -- which alarms. That is deliberate: on a workstation there is no server
# to look at, and the check saying so is more useful than the check being silent.
# Pass `just brief data/store http://127.0.0.1:8402` when there is one.
brief store="data/store" serve="":
    @RADAR_SERVE_URL={{ serve }} {{ cargo }} run --release -q --bin radar -- brief --store {{ store }}
