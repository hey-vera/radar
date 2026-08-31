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
export MIN_TESTS := "951"

_default:
    @just --list --unsorted

# The four fast checks, for the edit-compile loop. Not a substitute for `just ci`.
check: build tests lint fmt

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
mutants base="origin/main":
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
    {{ cargo }} mutants --in-diff "$diff_file"         --timeout 300 --minimum-test-timeout 60 -- --offline

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
export MIN_WEB_TESTS := "25"

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
    npm audit --audit-level=high
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

# --- operator commands --------------------------------------------------------

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
