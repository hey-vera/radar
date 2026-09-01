// SPDX-License-Identifier: Apache-2.0
//! The price path a token actually took, drawn from the outcome measurements.
//!
//! # Why this is not a candlestick chart
//!
//! It would be easy to draw one, and it would be a lie.
//!
//! `peak_price` and `trough_price` are folded with `max` and `min` **from
//! launch**, not within each checkpoint interval — so at any measurement they
//! are the running extremes over the token's whole life, and they can never go
//! down and up respectively. Rendering them as a candle's high and low would
//! tell a trader that every interval reached those levels, which is false for
//! all but the one that set them.
//!
//! So the line is `last_price`, which is a real price at a real moment, and the
//! band behind it is the running envelope, labelled as such. A reader can see
//! how far the token ever went without being told it went there recently.
//!
//! # What changed, and what did not
//!
//! The store now records `window_peak_price` and `window_trough_price` — the
//! same extremes **without** the fold from launch — added precisely because
//! research 0020 could not answer whether an exit rule helps: counting only new
//! all-time extremes left 96% of price paths looking motionless.
//!
//! So there is now a second, tighter band that means something the launch-folded
//! one cannot: where the price has been *recently*. The argument above is
//! unchanged for the outer band, and it is still not a candlestick — the window
//! is six hours read hourly, so it overlaps by five, and a peak set five hours
//! ago appears in six consecutive measurements. It is a bounded recent lookback,
//! not the movement since the last checkpoint, and the caption says so.
//!
//! It is `null` on every row written before the column existed, which is most of
//! the store. Absent, the band is simply not drawn — never collapsed onto the
//! line, which would claim the price had not moved.
//!
//! # Why no charting library
//!
//! The workspace's dependency posture is that every dependency is one more thing
//! that can be compromised into a process that will eventually hold a signing
//! key, and the bar for adding one is "the alternative is writing it ourselves
//! and getting it wrong". A polyline and a filled path are not that.

import type { Measurement } from "./api";

/** A measurement reduced to the two numbers a chart needs, nulls dropped. */
export interface Point {
  at: number;
  last: number;
  peak: number;
  trough: number;
  /** The recent window's extremes, where the store recorded them. */
  windowPeak: number | null;
  windowTrough: number | null;
}

/** The smallest number of points that can make a line. */
export const MIN_POINTS = 2;

/**
 * The measurements that can actually be drawn, oldest first.
 *
 * A measurement missing any of the three prices is dropped rather than
 * defaulted. Absent is not zero: a missing trough read as zero would drag the
 * envelope to the floor and make every token look like it went to nothing.
 */
export function drawable(measurements: readonly Measurement[]): Point[] {
  return measurements
    .filter(
      (m): m is Measurement & { last_price: number; peak_price: number; trough_price: number } =>
        m.last_price !== null &&
        m.peak_price !== null &&
        m.trough_price !== null &&
        m.last_price > 0 &&
        m.peak_price > 0 &&
        m.trough_price > 0,
    )
    .map((m) => ({
      at: m.measured_at,
      last: m.last_price,
      peak: m.peak_price,
      trough: m.trough_price,
      // Kept as a pair or not at all. Half a band is not a band, and a
      // window peak drawn against a launch-folded trough would be two
      // different measurements presented as one range.
      windowPeak:
        m.window_peak_price !== null && m.window_trough_price !== null
          ? m.window_peak_price
          : null,
      windowTrough:
        m.window_peak_price !== null && m.window_trough_price !== null
          ? m.window_trough_price
          : null,
    }))
    .sort((a, b) => a.at - b.at);
}

/**
 * Whether every point carries a recent-window range.
 *
 * All or nothing, deliberately. The column is `null` on every row written before
 * it existed, so a store mid-migration has some points with it and some without
 * — and a band drawn across that gap would interpolate between "measured" and
 * "not measured", which is rule 9 rendered as a shape.
 *
 * Drawing it only when the whole series has it means the band is either a claim
 * about every checkpoint or absent.
 */
export function hasWindowBand(points: readonly Point[]): boolean {
  return (
    points.length > 0 &&
    points.every((p) => p.windowPeak !== null && p.windowTrough !== null)
  );
}

/** The vertical range the chart must cover. */
export interface Range {
  low: number;
  high: number;
}

/**
 * The band of prices to fit, taken from the envelope rather than the line.
 *
 * The envelope is what the line has to sit inside, so scaling to the line alone
 * would push the band off the top and bottom of the picture.
 *
 * A flat series has zero range, which would divide by zero when scaling. It is
 * widened symmetrically instead of clamped, so a token whose price never moved
 * draws as a level line through the middle rather than pinned to an edge —
 * which is what actually happened, and the honest way to show it.
 */
export function rangeOf(points: readonly Point[]): Range {
  const low = Math.min(...points.map((p) => p.trough));
  const high = Math.max(...points.map((p) => p.peak));
  if (high > low) return { low, high };
  const pad = Math.abs(high) || 1;
  return { low: low - pad, high: high + pad };
}

/**
 * Maps a price to a y coordinate in a box of `height`.
 *
 * SVG's y grows downward, so the high price maps to 0 and the low to `height` —
 * inverting this is the classic way to draw a chart upside down and have it look
 * plausible.
 */
export function y(price: number, range: Range, height: number): number {
  const span = range.high - range.low;
  return height - ((price - range.low) / span) * height;
}

/** Maps an index to an x coordinate spread evenly across `width`. */
export function x(index: number, count: number, width: number): number {
  if (count <= 1) return 0;
  return (index / (count - 1)) * width;
}

const WIDTH = 640;
const HEIGHT = 160;

/** The token's price path, with its running envelope behind it. */
export function PricePath({ measurements }: { measurements: readonly Measurement[] }) {
  const points = drawable(measurements);

  if (points.length < MIN_POINTS) {
    return (
      <p className="text-sm text-[var(--color-dim)]">
        Not enough priced measurements to draw a path. That is a fact about what
        Radar observed, not about the token.
      </p>
    );
  }

  const range = rangeOf(points);
  const at = (i: number) => `${x(i, points.length, WIDTH)},`;

  const line = points
    .map((p, i) => `${at(i)}${y(p.last, range, HEIGHT)}`)
    .join(" ");

  // The envelope, as one closed path: along the running peak, back along the
  // running trough.
  const top = points.map((p, i) => `${at(i)}${y(p.peak, range, HEIGHT)}`);
  const bottom = points
    .map((p, i) => `${at(i)}${y(p.trough, range, HEIGHT)}`)
    .reverse();
  const envelope = `M ${top.join(" L ")} L ${bottom.join(" L ")} Z`;

  // The recent-window band, drawn only when every point has one.
  const windowed = hasWindowBand(points);
  const windowTop = points.map(
    (p, i) => `${at(i)}${y(p.windowPeak ?? p.last, range, HEIGHT)}`,
  );
  const windowBottom = points
    .map((p, i) => `${at(i)}${y(p.windowTrough ?? p.last, range, HEIGHT)}`)
    .reverse();
  const recent = `M ${windowTop.join(" L ")} L ${windowBottom.join(" L ")} Z`;

  return (
    <figure className="space-y-2">
      <svg
        viewBox={`0 0 ${WIDTH} ${HEIGHT}`}
        className="w-full"
        role="img"
        aria-label="Price path with the running high and low since launch"
        preserveAspectRatio="none"
      >
        <path d={envelope} fill="var(--color-line)" opacity="0.5" />
        {windowed && (
          <path d={recent} fill="var(--color-edge)" opacity="0.55" />
        )}
        <polyline
          points={line}
          fill="none"
          stroke="var(--color-accent)"
          strokeWidth="2"
          vectorEffect="non-scaling-stroke"
        />
      </svg>
      <figcaption className="text-xs leading-relaxed text-[var(--color-dim)]">
        The line is the last observed price at each checkpoint. The outer band is
        the highest and lowest price seen <strong>since launch</strong> — a
        running total, not the range of each interval, so it can only widen.
        {windowed ? (
          <>
            {" "}
            The inner band is the <strong>recent</strong> range: the extremes of
            a six-hour window, read hourly, so consecutive windows overlap by
            five hours and a move shows in six of them. It is a bounded lookback,
            not the movement since the last checkpoint.
          </>
        ) : (
          <>
            {" "}
            No recent-window range is drawn: the store did not record one for
            every checkpoint here, and a band across that gap would interpolate
            between measured and not measured.
          </>
        )}{" "}
        Neither is a candlestick, which would claim every interval reached its
        levels.
      </figcaption>
    </figure>
  );
}
