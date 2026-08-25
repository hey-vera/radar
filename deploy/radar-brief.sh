#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# Runs `radar brief` and makes a failure loud.
#
# `brief` already decides what "unhealthy" means and says so through its exit
# status. This script's only job is delivery, and it is separate from the check
# so that adding a notification channel never risks changing the verdict.
#
# Delivery is deliberately layered, because the box has no alert channel
# configured at all and the Cortex monitor has been writing to an unread log for
# months for exactly that reason:
#
#   1. Always: the journal, at error priority, so `journalctl -p err` finds it.
#   2. If RADAR_ALERT_WEBHOOK is set: POST the summary there.
#
# Unset means no webhook, never a silent pass -- the journal line always
# happens. That is rule 8 applied to alerting: missing config must not turn a
# failure into a success.
set -uo pipefail

STORE="${RADAR_STORE:-/home/guardian/radar/data/store}"
BIN="${RADAR_BIN:-/home/guardian/bin/radar}"

output="$("$BIN" brief --store "$STORE" 2>&1)"
status=$?

if [ "$status" -eq 0 ]; then
    logger -t radar-brief -p user.info "healthy"
    echo "$output"
    exit 0
fi

# One journal line per failing check rather than one blob, so the reason is
# visible in `journalctl -p err` without opening anything.
echo "$output" | sed -n '/need attention/,$p' | while IFS= read -r line; do
    [ -n "$line" ] && logger -t radar-brief -p user.err "$line"
done
logger -t radar-brief -p user.err "radar brief exited $status"

if [ -n "${RADAR_ALERT_WEBHOOK:-}" ]; then
    # Best effort. A webhook that is down must not turn a health alert into a
    # script failure, because the exit status below is what systemd records.
    payload=$(printf '%s' "$output" | tail -c 3000 | sed 's/"/\\"/g' | tr '\n' ' ')
    curl -fsS --max-time 10 -X POST -H 'Content-Type: application/json' \
        -d "{\"text\":\"radar brief unhealthy on $(hostname): ${payload}\"}" \
        "$RADAR_ALERT_WEBHOOK" >/dev/null 2>&1 \
        || logger -t radar-brief -p user.err "alert webhook POST failed"
fi

echo "$output"
exit "$status"
