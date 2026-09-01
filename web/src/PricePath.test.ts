// SPDX-License-Identifier: Apache-2.0
//! The chart's arithmetic, tested without rendering it.
//!
//! A chart fails silently: a scale that is upside down, or that quietly drops a
//! measurement, still draws a plausible picture. So the mapping is pure
//! functions with assertions on the numbers, rather than a snapshot of an SVG
//! nobody reads.

import { describe, expect, it } from "vitest";
import { drawable, hasWindowBand, rangeOf, type Point, x, y } from "./PricePath";
import type { Measurement } from "./api";

function measurement(over: Partial<Measurement>): Measurement {
  return {
    measured_at: 1,
    price_reads: 1,
    last_transfer_slot: null,
    first_price: 100,
    last_price: 100,
    peak_price: 100,
    trough_price: 100,
    window_peak_price: null,
    window_trough_price: null,
    vwap: null,
    graduated_at: null,
    held_to_end_bps: null,
    ...over,
  };
}

describe("drawable", () => {
  it("drops a measurement missing any price rather than defaulting it", () => {
    // Absent is not zero. A missing trough read as zero drags the envelope to
    // the floor and makes every token look like it went to nothing.
    const kept = drawable([
      measurement({ measured_at: 1 }),
      measurement({ measured_at: 2, trough_price: null }),
      measurement({ measured_at: 3, last_price: null }),
      measurement({ measured_at: 4, peak_price: null }),
      measurement({ measured_at: 5 }),
    ]);
    expect(kept.map((p) => p.at)).toEqual([1, 5]);
  });

  it("drops a zero price, which is a claim the token was worthless", () => {
    expect(drawable([measurement({ trough_price: 0 })])).toHaveLength(0);
    expect(drawable([measurement({ last_price: 0 })])).toHaveLength(0);
  });

  it("orders by measurement slot, whatever order the server sent", () => {
    // A path drawn in arrival order rather than time order is a scribble that
    // looks like volatility.
    const points = drawable([
      measurement({ measured_at: 30 }),
      measurement({ measured_at: 10 }),
      measurement({ measured_at: 20 }),
    ]);
    expect(points.map((p) => p.at)).toEqual([10, 20, 30]);
  });
});

describe("rangeOf", () => {
  const at = (peak: number, trough: number, last: number): Point => ({
    at: 1,
    peak,
    trough,
    last,
    windowPeak: null,
    windowTrough: null,
  });

  it("covers the envelope, not just the line", () => {
    // Scaling to the line alone pushes the band off the top and bottom.
    const range = rangeOf([at(500, 10, 100), at(500, 10, 200)]);
    expect(range).toEqual({ low: 10, high: 500 });
  });

  it("widens a flat series instead of dividing by zero", () => {
    // A token whose price never moved must draw as a level line through the
    // middle -- which is what happened -- rather than pinned to an edge or
    // crashing on a zero span.
    const range = rangeOf([at(100, 100, 100), at(100, 100, 100)]);
    expect(range.high).toBeGreaterThan(range.low);
    expect(y(100, range, 160)).toBeCloseTo(80);
  });
});

describe("y", () => {
  const range = { low: 0, high: 100 };

  it("puts the high price at the top, because SVG grows downward", () => {
    // Inverting this draws the chart upside down and it still looks plausible,
    // which is why it is asserted rather than eyeballed.
    expect(y(100, range, 160)).toBe(0);
    expect(y(0, range, 160)).toBe(160);
    expect(y(50, range, 160)).toBe(80);
  });

  it("is monotonic: a higher price is never lower on the page", () => {
    let previous = Infinity;
    for (let price = 0; price <= 100; price += 10) {
      const here = y(price, range, 160);
      expect(here).toBeLessThan(previous);
      previous = here;
    }
  });
});

describe("x", () => {
  it("spreads the points evenly and reaches both edges", () => {
    expect(x(0, 5, 640)).toBe(0);
    expect(x(4, 5, 640)).toBe(640);
    expect(x(2, 5, 640)).toBe(320);
  });

  it("does not divide by zero on a single point", () => {
    expect(x(0, 1, 640)).toBe(0);
    expect(Number.isFinite(x(0, 1, 640))).toBe(true);
  });
});

describe("hasWindowBand", () => {
  const point = (windowPeak: number | null, windowTrough: number | null): Point => ({
    at: 1,
    last: 100,
    peak: 200,
    trough: 50,
    windowPeak,
    windowTrough,
  });

  it("is true only when every point carries the pair", () => {
    expect(hasWindowBand([point(120, 90), point(130, 95)])).toBe(true);
  });

  it("is false when any point is missing it", () => {
    // The column is null on every row written before it existed, so a store
    // mid-migration has some points with it and some without. A band across
    // that gap would interpolate between "measured" and "not measured", which
    // is rule 9 rendered as a shape.
    expect(hasWindowBand([point(120, 90), point(null, null)])).toBe(false);
  });

  it("is false for half a pair", () => {
    // A window peak against a launch-folded trough is two different
    // measurements presented as one range.
    expect(hasWindowBand([point(120, null)])).toBe(false);
    expect(hasWindowBand([point(null, 90)])).toBe(false);
  });

  it("is false for no points at all", () => {
    // `every` on an empty array is true, which would draw a band over nothing.
    expect(hasWindowBand([])).toBe(false);
  });
});
