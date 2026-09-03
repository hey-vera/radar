// SPDX-License-Identifier: Apache-2.0
//! The capacity wall.
//!
//! The most important picture in the product, because it is the reason the thing
//! does not work yet — and it is the chart that makes the honest sell rather
//! than undermining it. Eighty per cent of proposals sit in a narrow band around
//! $31 of measured exit capacity, because every pre-graduation pump.fun token
//! rides the same bonding curve with the same supply. Capacity is closer to a
//! property of the venue than of the token, and the median position it produces
//! is $6.21 against a round trip of 850 bps.
//!
//! # Why there is no charting library here
//!
//! A histogram is a `<rect>`. The workspace's dependency posture is that every
//! dependency is one more thing that can be compromised into a bundle
//! `radar-serve` embeds and serves, and the bar for adding one is "the
//! alternative is writing it ourselves and getting it wrong". Five rectangles
//! and a rule are not that.
//!
//! # What it must not draw
//!
//! Decisions where capacity **could not be measured** are reported beside the
//! chart, never inside it. Rule 9: a capacity that could not be measured means
//! "cannot exit", not "thin" and certainly not zero, and bucketing them into the
//! bottom band would draw a wall of tokens nobody measured.

import { useEffect, useState } from "react";
import { ApiError, research, type Capacity } from "./api";

/** Micro-USD as dollars. */
function usd(micro: number | null | undefined): string {
  if (micro === null || micro === undefined) return "—";
  return `$${(micro / 1_000_000).toFixed(2)}`;
}

/** A band's label, from its bounds. */
function label(floor: number, ceiling: number | null): string {
  const from = Math.round(floor / 1_000_000);
  if (ceiling === null) return `$${from}+`;
  return `$${from}–${Math.round(ceiling / 1_000_000)}`;
}

export function CapacityWall() {
  const [wall, setWall] = useState<Capacity | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const controller = new AbortController();
    research
      .capacity(controller.signal)
      .then(setWall)
      .catch((e: unknown) => {
        if (controller.signal.aborted) return;
        setError(
          e instanceof ApiError && e.status === 503
            ? "The store has recorded nothing yet."
            : e instanceof ApiError
              ? e.detail
              : String(e),
        );
      });
    return () => controller.abort();
  }, []);

  if (error) {
    return <p className="text-sm text-[var(--color-warn)]">{error}</p>;
  }
  if (!wall) {
    return <p className="text-sm text-[var(--color-dim)]">Reading the store…</p>;
  }

  // A shape check, not a schema. `api.ts` types the wire by hand and casts, and
  // the comment there says why — but a cast is a promise, and this component
  // crashed the whole page when the promise was broken. A malformed answer is
  // not an empty one, so it says so rather than drawing an empty chart.
  if (!Array.isArray(wall.bands)) {
    return (
      <p className="text-sm text-[var(--color-warn)]">
        The capacity distribution came back in a shape this page does not
        understand. That is a fault, not an empty store.
      </p>
    );
  }

  const widest = Math.max(...wall.bands.map((b) => b.decisions), 1);

  return (
    <section className="mt-10">
      <h3 className="text-sm font-medium">Exit capacity, and what fits in it</h3>
      <p className="mt-1 mb-4 max-w-prose text-sm leading-relaxed text-[var(--color-dim)]">
        What Radar measured it could sell, per decision. The shape is the
        product&rsquo;s central problem: every pre-graduation pump.fun token
        rides the same bonding curve, so capacity is closer to a property of the
        venue than of the token — and the position it supports is far smaller
        than the round trip needs.
      </p>

      {wall.measured === 0 ? (
        <p className="rounded-md border border-[var(--color-line)] bg-[var(--color-surface)] px-4 py-3 text-sm">
          No decision has a measured exit capacity yet.{" "}
          <span className="text-[var(--color-dim)]">
            That is a fact about what Radar has probed, not about the venue.
          </span>
        </p>
      ) : (
        <figure className="space-y-1">
          {wall.bands.map((band) => {
            const share = wall.measured > 0 ? band.decisions / wall.measured : 0;
            return (
              <div
                key={band.floor}
                className="grid grid-cols-[5rem_1fr_7rem] items-center gap-3 text-xs"
              >
                <span className="text-right tabular-nums text-[var(--color-dim)]">
                  {label(band.floor, band.ceiling)}
                </span>
                <span
                  className="h-4 rounded-sm bg-[var(--color-line)]"
                  role="presentation"
                >
                  <span
                    className="block h-full rounded-sm bg-[var(--color-good)]"
                    style={{
                      // A band with rows must be visible, so it never rounds to
                      // nothing. A band with none must be invisible, or an empty
                      // band reads as a small one.
                      width: `${Math.max((band.decisions / widest) * 100, band.decisions > 0 ? 1.5 : 0)}%`,
                    }}
                  />
                </span>
                <span className="tabular-nums text-[var(--color-dim)]">
                  {band.decisions.toLocaleString()}
                  {" · "}
                  {(share * 100).toFixed(1)}%
                </span>
              </div>
            );
          })}
          <figcaption className="pt-2 text-xs leading-relaxed text-[var(--color-dim)]">
            Decisions by measured exit capacity, in the bands research 0018 used
            — fixed rather than fitted to the data, so the picture can be
            compared against the note it illustrates.
          </figcaption>
        </figure>
      )}

      <dl className="mt-6 grid grid-cols-2 gap-4 sm:grid-cols-4">
        <Stat label="median capacity" value={usd(wall.median_capacity)} />
        <Stat label="median position" value={usd(wall.median_notional)} />
        <Stat
          label="round trip"
          value={`${(wall.round_trip_bps / 100).toFixed(1)}%`}
        />
        <Stat
          label="move to break even"
          // Not a function of position size: the round trip is applied as a
          // proportion, so the move required is the round trip. Shown beside the
          // position rather than derived from it, because 0019 measured cost to
          // be strongly size-dependent below $20 and this figure does not know
          // that.
          value={`+${(wall.round_trip_bps / 100).toFixed(1)}%`}
          tone="loss"
        />
      </dl>

      {wall.unmeasured > 0 && (
        <p className="mt-4 rounded-md border border-[var(--color-line)] bg-[var(--color-surface)] px-4 py-3 text-xs leading-relaxed">
          <strong className="text-[var(--color-warn)]">
            {wall.unmeasured.toLocaleString()} decisions have no measured
            capacity
          </strong>{" "}
          <span className="text-[var(--color-dim)]">
            and are not in the chart. A capacity that could not be measured means{" "}
            <em>cannot exit</em> — not &ldquo;thin&rdquo;, and not zero. Drawing
            them in the bottom band would show a wall of tokens nobody measured.
          </span>
        </p>
      )}
    </section>
  );
}

function Stat({
  label: name,
  value,
  tone,
}: {
  label: string;
  value: string;
  tone?: "loss";
}) {
  return (
    <div>
      <dt className="text-xs uppercase tracking-wide text-[var(--color-dim)]">
        {name}
      </dt>
      <dd
        className={`mt-1 text-xl tabular-nums ${
          tone === "loss" ? "text-[var(--color-loss)]" : ""
        }`}
      >
        {value}
      </dd>
    </div>
  );
}
