// SPDX-License-Identifier: Apache-2.0
//! What a trade costs, as a function of how big it is.
//!
//! # Why this is a constant and not an endpoint
//!
//! It is **one measured hour** — 183,647 pump.fun trade legs on 2026-08-25,
//! 04:00–05:00 UTC — and re-running it is `radar cost --from … --to …`. An
//! endpoint would imply the figure is live, and it is not: a chart that refetches
//! a constant every page load is a chart that claims a freshness it does not
//! have.
//!
//! So the numbers are here, with the note they came from named beside them, and
//! the date they were taken on is on the screen rather than in a comment.
//!
//! # What it is for
//!
//! The finding that survives every caveat in 0019: **a leg under $2 costs 1,521
//! bps**. Radar's own `min_notional` is $1.00, which lands squarely in that band
//! — so a position at the system's own floor faces a round trip of roughly 30%,
//! and nothing in the system knew that. The median proposal, $6.21, sits one
//! band up.
//!
//! Two things this must not do, and both are 0019's own caveats:
//!
//! - It must not lean on the $2–$20 figure of 125 bps being *cheaper* than
//!   $20–$200 at 228. The bands are not monotonic, 0019 does not explain it, and
//!   it says in as many words that the 125 should not be leaned on.
//! - It must not present the numbers as a round trip. They are **a leg**.

/**
 * Cost by notional, from research 0019.
 *
 * `bps` is per **leg**, not per round trip. Doubling is the reader's to do and
 * the caption says so — a table silently doubled is one nobody can check against
 * the note.
 */
const BANDS = [
  { from: 1_000_000, to: 10_000_000, label: "$0.20 – $2", bps: 1521, fills: 29_996 },
  { from: 10_000_000, to: 100_000_000, label: "$2 – $20", bps: 125, fills: 79_061 },
  { from: 100_000_000, to: 1_000_000_000, label: "$20 – $200", bps: 228, fills: 56_777 },
  { from: 1_000_000_000, to: 10_000_000_000, label: "$200 – $2,000", bps: 225, fills: 17_723 },
  { from: 10_000_000_000, to: null, label: "$2,000+", bps: 130, fills: 90 },
] as const;

/** Radar's own floor, in lamports, and the band it lands in. */
const MIN_NOTIONAL_LAMPORTS = 5_000_000;

/** The median proposal, in lamports, per research 0018. */
const MEDIAN_NOTIONAL_LAMPORTS = 31_000_000;

/** Which band a lamport amount falls in, or -1. */
function bandOf(lamports: number): number {
  return BANDS.findIndex(
    (b) => lamports >= b.from && (b.to === null || lamports < b.to),
  );
}

export function CostCurve() {
  const widest = Math.max(...BANDS.map((b) => b.bps));
  const floorBand = bandOf(MIN_NOTIONAL_LAMPORTS);
  const medianBand = bandOf(MEDIAN_NOTIONAL_LAMPORTS);

  return (
    <section className="mt-10">
      <h3 className="text-sm font-medium">What a trade costs, by how big it is</h3>
      <p className="mt-1 mb-4 max-w-prose text-sm leading-relaxed text-[var(--color-dim)]">
        Cost is strongly size-dependent, and the dependence is at the bottom. A
        leg under $2 costs <strong>1,521 bps</strong>; above $20 it settles near
        225. That is the signature of a fixed cost — an associated token account
        costs about the same whatever the trade is worth — and it means a
        position at Radar&rsquo;s own floor faces a round trip of roughly 30%.
      </p>

      <figure className="space-y-1">
        {BANDS.map((band, i) => (
          <div
            key={band.label}
            className="grid grid-cols-[7rem_1fr_9rem] items-center gap-3 text-xs"
          >
            <span className="text-right tabular-nums text-[var(--color-dim)]">
              {band.label}
            </span>
            <span className="h-4 rounded-sm bg-[var(--color-line)]" role="presentation">
              <span
                className={`block h-full rounded-sm ${
                  // The band Radar's floor lands in is the finding. Everything
                  // else is context.
                  i === floorBand
                    ? "bg-[var(--color-loss)]"
                    : "bg-[var(--color-line-2,var(--color-edge))]"
                }`}
                style={{ width: `${Math.max((band.bps / widest) * 100, 1.5)}%` }}
              />
            </span>
            <span className="tabular-nums text-[var(--color-dim)]">
              {band.bps.toLocaleString()} bps
              {i === floorBand && (
                <span className="ml-1 text-[var(--color-loss)]">← floor</span>
              )}
              {i === medianBand && (
                <span className="ml-1 text-[var(--color-warn)]">← median</span>
              )}
            </span>
          </div>
        ))}
        <figcaption className="pt-2 max-w-prose text-xs leading-relaxed text-[var(--color-dim)]">
          Median cost per <strong>leg</strong>, not per round trip — double it
          for both sides, as the note does. Measured over 183,647 pump.fun trade
          legs in one hour on 2026-08-25 (research 0019), and it has not been
          re-run since; this is a constant on this page rather than a live
          figure, because pretending otherwise would claim a freshness it does
          not have.
        </figcaption>
      </figure>

      <p className="mt-4 rounded-md border border-[var(--color-line)] bg-[var(--color-surface)] px-4 py-3 text-xs leading-relaxed">
        <strong className="text-[var(--color-warn)]">Two caveats from the note itself.</strong>{" "}
        <span className="text-[var(--color-dim)]">
          The bands are <strong>not monotonic</strong> — $2–$20 at 125 bps being
          cheaper than $20–$200 at 228 is not explained, and 0019 says the 125
          should not be leaned on. The finding that survives it is the 1,521 at
          the bottom, which is an order of magnitude clear of everything else.
          And failed transactions are excluded: they burn a fee against no
          notional, so the true cost of <em>attempting</em> to trade is above
          every figure here.
        </span>
      </p>
    </section>
  );
}
