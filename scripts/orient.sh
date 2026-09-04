#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# Where a session starts. `just orient`.
#
# Design 0006 §6 tells a fresh session to read four documents in order. That is
# a paragraph asking somebody to remember, which is the weakest rung of the
# ladder in AGENTS.md §5 -- and the session that wrote it then produced a table
# whose row 6 pointed at a branch that had merged the day before.
#
# So this prints the things that go stale, from the files that own them, rather
# than restating any of them. Every line is read at the moment it is printed:
# there is no second copy here to drift.
#
# It is silent about anything it cannot establish, and says so rather than
# guessing. An absent answer is not a good answer -- LEARNINGS 5.

set -uo pipefail
cd "$(git rev-parse --show-toplevel 2>/dev/null || echo .)" || exit 0

rule() { printf '\n  \033[2m%s\033[0m\n' "$1"; }

# --- where you are ------------------------------------------------------------

branch=$(git symbolic-ref --quiet --short HEAD 2>/dev/null || echo "detached")
head=$(git rev-parse --short HEAD 2>/dev/null || echo "?")
rule "branch"
printf '    %s at %s\n' "$branch" "$head"
if [ "$branch" = "main" ]; then
    printf '    \033[33mon main: the pre-commit hook refuses a commit here.\033[0m\n'
fi

dirty=$(git status --porcelain 2>/dev/null | wc -l | tr -d ' ')
[ "$dirty" != "0" ] && printf '    %s uncommitted path(s)\n' "$dirty"

# --- how CI last felt about it ------------------------------------------------
#
# The same question `scripts/hooks/pre-push` asks, and the same discipline: a
# slow or absent answer prints nothing rather than something alarming.

rule "last CI run on this branch"
if command -v gh >/dev/null 2>&1; then
    gh_run() {
        if command -v timeout >/dev/null 2>&1; then timeout 10 gh "$@"; else gh "$@"; fi
    }
    last=$(gh_run run list --branch "$branch" --limit 1 \
        --json status,conclusion,headSha \
        --jq '.[0] | "\(.status) \(.conclusion // "") \(.headSha[0:7])"' 2>/dev/null || true)
    # `--jq` on an empty list yields "null null", not an empty string, so an
    # unpushed branch would otherwise report a run that does not exist.
    case "$last" in
        "" | null*) printf '    no runs for this branch, or GitHub did not answer.\n' ;;
        *) printf '    %s\n' "$last" ;;
    esac
else
    printf '    gh is not installed; cannot say.\n'
fi

# --- what the last session was doing ------------------------------------------
#
# A plan whose Status is neither landed nor abandoned is work in flight, and its
# Handback is the paragraph that says where it stopped. This is the whole reason
# `docs/plans/` exists, and the directory carries a kill date for exactly the
# case where nobody reads it.

rule "plans in flight (docs/plans/)"
found=0
for plan in docs/plans/[0-9]*.md; do
    [ -e "$plan" ] || continue
    status=$(grep -m1 '^Status:' "$plan" 2>/dev/null | sed 's/^Status:[[:space:]]*//')
    case "$status" in
        landed*|abandoned*) continue ;;
    esac
    found=1
    printf '    \033[1m%s\033[0m — %s\n' "$plan" "${status:-no status line}"
    # The Handback block, which is the part written to be read first.
    awk '/^## Handback/{f=1;next} f&&/^## /{exit} f' "$plan" | sed 's/^/      /'
done
[ "$found" = "0" ] && printf '    none open. Design 0007 is the standing plan.\n'

# --- the claims most likely to be wrong ---------------------------------------
#
# Read out of docs/STATE.md rather than repeated here. That file names its own
# decaying claims in its header, and a copy of that sentence in this script is
# a second thing to keep true.

rule "docs/STATE.md says these decay fastest"
awk '/most likely to be stale/{p=1} p{print "    " $0} p&&/^$/{exit}' docs/STATE.md 2>/dev/null \
    || printf '    could not read docs/STATE.md\n'

# --- the machine --------------------------------------------------------------
#
# `target/` reached 127GB once and froze the workstation hard enough to need a
# forced power-off. `just check` refuses above 40GB; this is the earlier warning.

rule "this checkout's build cache"
if [ -d target ]; then
    gb=$(du -sk target 2>/dev/null | awk '{printf "%.1f", $1/1048576}')
    printf '    target/ is %sGB (just check refuses at 40; just tidy clears it)\n' "${gb:-?}"
else
    printf '    no target/ yet; the first build starts cold.\n'
fi

printf '\n  \033[2mRead next: GOAL.md, then docs/design/0007, then AGENTS.md.\033[0m\n\n'
