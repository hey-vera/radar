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

# The serving surface the check should probe. Exported rather than passed so the
# unit's EnvironmentFile can override it without editing this script.
#
# Defaulted rather than left unset: an unset endpoint makes `brief` report
# "cannot see" and alarm, which is correct for a workstation and pure noise on
# the box that is running the server. On this host we know where it is.
export RADAR_SERVE_URL="${RADAR_SERVE_URL:-http://127.0.0.1:8402}"

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
    #
    # Two body shapes, because the plausible destinations do not agree and
    # picking one silently excludes the others. An earlier version sent only
    # `{"text": ...}`, which is Slack's field: **Discord rejects that body with
    # a 400**, and the only trace would be one "POST failed" line in the
    # journal -- an alerting path that fails in exactly the way the outage it
    # exists to report would.
    #
    #   json (default)  {"text": ..., "content": ...}  Slack reads `text`,
    #                                                  Discord reads `content`,
    #                                                  each ignores the other.
    #   text            the message as the raw body    ntfy.sh, and anything
    #                                                  else wanting plain text.
    #
    # Set RADAR_ALERT_FORMAT=text beside the webhook for ntfy. Anything else,
    # including unset, is json -- an unrecognised value must not silently pick
    # the shape the destination cannot read.
    # The two characters that can break a JSON string are REMOVED rather than
    # escaped, and that is the whole trick. Escaping them from shell means a
    # backslash surviving a heredoc, a sed expression and a `curl -d` argument
    # intact. The version of this that tried lost them somewhere in that chain
    # and POSTed an empty message field -- which looks exactly like a delivered
    # alert from the outside, and is the failure this whole path exists to
    # avoid. Found by pointing it at a listener and reading what arrived.
    #
    # A notification is prose for a person, not a record, so a quote becoming a
    # tick and a backslash becoming a slash costs nothing. What it buys is a
    # body that cannot be malformed by its own contents -- and a Windows path
    # or a quoted token symbol in a check detail is exactly that input.
    body=$(printf '%s' "$output" | tail -c 3000 | tr '\n\r\t' '   ' | tr '"\\' "'/")
    summary="radar brief unhealthy on $(hostname): ${body}"
    # Three shapes. Telegram is separate from the other two because it needs a
    # second value -- the chat to deliver to -- which no webhook URL carries on
    # its own, and because its field is `text` inside a body that must also
    # name `chat_id`. Sending it the Slack/Discord body would be answered with
    # `400 Bad Request: chat_id is empty`.
    case "${RADAR_ALERT_FORMAT:-json}" in
    text)
        curl -fsS --max-time 10 -X POST -H 'Content-Type: text/plain' \
            --data-binary "$summary" "$RADAR_ALERT_WEBHOOK" >/dev/null 2>&1 \
            || logger -t radar-brief -p user.err "alert webhook POST failed"
        ;;
    telegram)
        # Deny by default, and loudly: a chat id that is missing means every
        # alert would be accepted by curl and delivered to nobody.
        if [ -z "${RADAR_ALERT_CHAT_ID:-}" ]; then
            logger -t radar-brief -p user.err "RADAR_ALERT_FORMAT=telegram needs RADAR_ALERT_CHAT_ID; no alert sent"
        else
            curl -fsS --max-time 10 -X POST -H 'Content-Type: application/json' \
                -d "{\"chat_id\":\"${RADAR_ALERT_CHAT_ID}\",\"text\":\"${summary}\"}" \
                "$RADAR_ALERT_WEBHOOK" >/dev/null 2>&1 \
                || logger -t radar-brief -p user.err "alert webhook POST failed"
        fi
        ;;
    *)
        curl -fsS --max-time 10 -X POST -H 'Content-Type: application/json' \
            -d "{\"text\":\"${summary}\",\"content\":\"${summary}\"}" \
            "$RADAR_ALERT_WEBHOOK" >/dev/null 2>&1 \
            || logger -t radar-brief -p user.err "alert webhook POST failed"
        ;;
    esac
fi

echo "$output"
exit "$status"
