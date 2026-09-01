// SPDX-License-Identifier: Apache-2.0
//! What the selection returned, as a distribution rather than a median.
//!
//! # Why this exists when the median is already on the page
//!
//! Research 0017's central caveat: **24–43% of both cohorts return exactly
//! zero**. A median over a point mass that large is a report about the point
//! mass and not about the market, and the note says in as many words that a bare
//! median of 0 must not be read as "the two cohorts performed identically".
//!
//! A histogram is the only rendering that shows that. So the zero share is the
//! **headline** here rather than a bar: drawn as one more bucket it would be the
//! tallest thing on the chart and read as a finding about the market, when it is
//! a fact about a venue where most tokens trade a handful of times and stop.
//!
//! Everything here is **gross**. The round trip is drawn as a line rather than
//! subtracted from the data, so a reader can see both where the mass is and
//! where break-even sits without the chart having moved anything.

import { useEffect, useState } from "react";
import { ApiError, research, type Returns } from "./api";

/** A bucket's label, from its open-ended bounds. */
function label(floor: number | null, ceiling: number | null): string {
  const pct = (bps: number) => `${bps > 0 ? "+" : ""}${(bps / 100).toFixed(0)}%`;
  if (floor === null) return `below ${pct(ceiling ?? 0)}`;
  if (ceiling === null) return `${pct(floor)} and up`;
  return `${pct(floor)} to ${pct(ceiling)}`;
}

export function ReturnDistribution() {
  const [dist, setDist] = useState<Returns | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const controller = new AbortController();
    research
      .returns(controller.signal)
      .then(setDist)
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

  if (error) return <p className="text-sm text-[var(--color-warn)]">{error}</p>;
  if (!dist)
    return <p className="text-sm text-[var(--color-dim)]">Reading the store…</p>;

  // The same guard `CapacityWall` carries, for the same reason: `api.ts` types
  // the wire by hand and casts, and a cast is a promise.
  if (!Array.isArray(dist.buckets)) {
    return (
      <p className="text-sm text-[var(--color-warn)]">
        The return distribution came back in a shape this page does not
        understand. That is a fault, not an empty store.
      </p>
    );
  }

  const widest = Math.max(...dist.buckets.map((b) => b.scored), 1);
  const zeroShare = dist.scored > 0 ? dist.exactly_zero / dist.scored : 0;

  return (
    <section className="mt-10">
      <h3 className="text-sm font-medium">Where the returns actually fell</h3>
      <p className="mt-1 mb-4 max-w-prose text-sm leading-relaxed text-[var(--color-dim)]">
        The distribution rather than the median, because the median here is a
        report about a point mass. Everything is <strong>gross</strong> — the
        round trip is marked below rather than subtracted, so nothing has been
        moved under the reader.
      </p>

      {dist.scored === 0 ? (
        <p className="rounded-md border border-[var(--color-line)] bg-[var(--color-surface)] px-4 py-3 text-sm">
          Nothing has been scored yet.{" "}
          <span className="text-[var(--color-dim)]">
            A decision needs an entry price and a later observation, and{" "}
            {dist.unscored.toLocaleString()} have one or neither.
          </span>
        </p>
      ) : (
        <>
          {/* The caveat first, and as a figure rather than a bar. Drawn as one
              more bucket this would be the tallest thing on the chart and read
              as a finding about the market. */}
          <p className="mb-4 rounded-md border border-[var(--color-line)] bg-[var(--color-surface)] px-4 py-3 text-sm leading-relaxed">
            <strong className="text-[var(--color-warn)]">
              {dist.exactly_zero.toLocaleString()} of {dist.scored.toLocaleString()}{" "}
              ({(zeroShare * 100).toFixed(1)}%) returned exactly zero
            </strong>{" "}
            <span className="text-[var(--color-dim)]">
              and are not drawn below. Most tokens on this venue trade a handful
              of times and stop, so they end exactly where they started. A median
              over that is a report about the point mass — which is why research
              0017 says a median of 0 must not be read as two cohorts performing
              identically.
            </span>
          </p>

          <figure className="space-y-1">
            {dist.buckets.map((bucket) => {
              const losing = (bucket.ceiling ?? 1) <= 0;
              const clearsCost = (bucket.floor ?? -1) >= dist.round_trip_bps;
              return (
                <div
                  key={`${bucket.floor}:${bucket.ceiling}`}
                  className="grid grid-cols-[9rem_1fr_5rem] items-center gap-3 text-xs"
                >
                  <span className="text-right tabular-nums text-[var(--color-dim)]">
                    {label(bucket.floor, bucket.ceiling)}
                  </span>
                  <span
                    className="h-4 rounded-sm bg-[var(--color-line)]"
                    role="presentation"
                  >
                    <span
                      className={`block h-full rounded-sm ${
                        losing
                          ? "bg-[var(--color-loss)]"
                          : clearsCost
                            ? "bg-[var(--color-gain)]"
                            : "bg-[var(--color-edge)]"
                      }`}
                      style={{
                        width: `${Math.max((bucket.scored / widest) * 100, bucket.scored > 0 ? 1.5 : 0)}%`,
                      }}
                    />
                  </span>
                  <span className="tabular-nums text-[var(--color-dim)]">
                    {bucket.scored.toLocaleString()}
                  </span>
                </div>
              );
            })}
            <figcaption className="pt-2 max-w-prose text-xs leading-relaxed text-[var(--color-dim)]">
              Green is above the {(dist.round_trip_bps / 100).toFixed(1)}% round
              trip and therefore the only band that made money; grey is a gain
              that did not cover its costs; amber is a loss. The colours are
              backed by the sign in the label, so the split reads without them.
            </figcaption>
          </figure>
        </>
      )}

      {dist.unscored > 0 && (
        <p className="mt-4 text-xs leading-relaxed text-[var(--color-dim)]">
          <strong>{dist.unscored.toLocaleString()} decisions could not be scored</strong>{" "}
          and are absent rather than counted as flat. A decision that never
          reached the exit probe has no entry price, and folding those into
          &ldquo;broke even&rdquo; is the whole population&rsquo;s median dressed
          up as a result.
        </p>
      )}
    </section>
  );
}
