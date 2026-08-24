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
export MIN_TESTS := "446"

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

# Advisories, licences, and source provenance.
cargo-deny:
    {{ cargo }} deny check

# Every source file carries an SPDX header. Cheap to check, easy to forget.
licence-headers:
    #!/usr/bin/env bash
    set -euo pipefail
    missing=0
    while IFS= read -r f; do
        if ! head -3 "$f" | grep -q "SPDX-License-Identifier"; then
            echo "missing SPDX header: $f"
            missing=1
        fi
    done < <(find crates -name '*.rs' -not -path '*/target/*')
    exit $missing

# --- operator commands --------------------------------------------------------

# What the system is doing right now: ingestion lag, spend against budget,
# provider health, open positions. Reads live state, so it can never be stale.
brief:
    @echo "not implemented yet — Phase 0 deliverable"
