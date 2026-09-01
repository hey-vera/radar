// SPDX-License-Identifier: Apache-2.0
//! Whether the recorder is still running, in a form that shows a gap.
//!
//! # Why this is not the watermark
//!
//! The Instance screen already reports a watermark, and it answers the obvious
//! version of this question. It cannot answer the useful one, because the
//! watermark follows the **chain** rather than the decider: it keeps advancing
//! while the thing that takes decisions is dead.
//!
//! That is not hypothetical here. The follow recorder has exited on a query
//! error before, with no restart and no alarm, and nothing on any screen said
//! so. A watermark tells you where it got to. A row of bars tells you it stopped
//! on Tuesday.
//!
//! So the one rule this component has: **a day with no decisions is a zero, not
//! a missing bar.** Bars drawn only for the days that had data close the gap and
//! draw an unbroken record straight over an outage — which is the exact failure
//! it exists to reveal.

import { useEffect, useState } from "react";
import { ApiError, research, type Activity as Record } from "./api";

export function Activity() {
  const [record, setRecord] = useState<Record | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const controller = new AbortController();
    research
      .activity(controller.signal)
      .then(setRecord)
      .catch((e: unknown) => {
        if (controller.signal.aborted) return;
        // Quiet on 503. This sits above the record on the default screen, and
        // an empty store already says so once below — twice is noise.
        if (!(e instanceof ApiError && e.status === 503)) {
          setError(e instanceof ApiError ? e.detail : String(e));
        }
      });
    return () => controller.abort();
  }, []);

  if (error) {
    return (
      <p className="mb-6 text-xs text-[var(--color-warn)]">
        Could not read the recorder&rsquo;s activity — {error}
      </p>
    );
  }
  if (!record || !Array.isArray(record.intervals) || record.intervals.length === 0) {
    return null;
  }

  const busiest = Math.max(...record.intervals.map((i) => i.decisions), 1);
  const quiet = record.intervals.filter((i) => i.decisions === 0).length;
  const last = record.intervals[record.intervals.length - 1];

  return (
    <section className="mb-8">
      <div className="flex items-baseline justify-between gap-4">
        <h2 className="text-sm font-medium uppercase tracking-wide text-[var(--color-dim)]">
          Decisions per day
        </h2>
        {quiet > 0 && (
          <span className="text-xs text-[var(--color-warn)]">
            {quiet} {quiet === 1 ? "day" : "days"} with none
          </span>
        )}
      </div>

      <div className="mt-2 flex items-end gap-1" role="presentation">
        {record.intervals.map((interval) => (
          <span
            key={interval.from_slot}
            title={`slot ${interval.from_slot.toLocaleString()} — ${interval.decisions.toLocaleString()} decisions, ${interval.proposed.toLocaleString()} proposed`}
            className="flex-1"
          >
            <span
              className={`block rounded-sm ${
                // A day with nothing is drawn, and drawn differently. Absent, a
                // reader's eye closes the gap; the same colour as a busy day and
                // they do not see it at all.
                interval.decisions === 0
                  ? "bg-[var(--color-loss)]"
                  : "bg-[var(--color-good)]"
              }`}
              style={{
                height: `${Math.max((interval.decisions / busiest) * 40, 2)}px`,
              }}
            />
          </span>
        ))}
      </div>

      <p className="mt-2 text-xs leading-relaxed text-[var(--color-dim)]">
        The last fortnight of the decision record, most recent on the right, up
        to slot {last?.from_slot.toLocaleString() ?? "—"}. A day with no
        decisions is drawn as an empty bar rather than left out: the recorder has
        stopped before without saying so, and a chart that closes its own gaps
        cannot show that.
      </p>
    </section>
  );
}
