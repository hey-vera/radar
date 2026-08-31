// SPDX-License-Identifier: Apache-2.0
//! That the chart actually renders, and renders something well formed.
//!
//! The arithmetic is tested next door. This is the other half, and it is a
//! different failure: a scale can be perfectly correct while the path string it
//! builds is malformed, and a browser given a broken `d` attribute draws nothing
//! at all — silently, with no error anywhere.
//!
//! So this asserts the shape of what is emitted rather than a snapshot. A
//! snapshot would go green on a blank chart the moment somebody updated it.

import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { PricePath } from "./PricePath";
import type { Measurement } from "./api";

function measurement(over: Partial<Measurement>): Measurement {
  return {
    measured_at: 1,
    fills: 1,
    first_price: 100,
    last_price: 100,
    peak_price: 100,
    trough_price: 100,
    graduated_at: null,
    held_to_end_bps: null,
    ...over,
  };
}

/** A rising token: three checkpoints, the envelope widening as it goes. */
const rising = [
  measurement({ measured_at: 10, last_price: 100, peak_price: 100, trough_price: 100 }),
  measurement({ measured_at: 20, last_price: 150, peak_price: 160, trough_price: 90 }),
  measurement({ measured_at: 30, last_price: 130, peak_price: 200, trough_price: 80 }),
];

/** Every number in a `points` or `d` attribute, as numbers. */
function numbersIn(attribute: string): number[] {
  return (attribute.match(/-?\d+(\.\d+)?/g) ?? []).map(Number);
}

describe("PricePath", () => {
  it("emits a polyline with one point per measurement", () => {
    const { container } = render(<PricePath measurements={rising} />);
    const polyline = container.querySelector("polyline");
    expect(polyline).not.toBeNull();

    // Two numbers per point, and three points.
    const numbers = numbersIn(polyline?.getAttribute("points") ?? "");
    expect(numbers).toHaveLength(6);
    expect(numbers.every(Number.isFinite)).toBe(true);
  });

  it("emits a closed envelope path with both edges", () => {
    // Along the running peak and back along the running trough: six points, and
    // a `Z` to close it. An unclosed path fills as a wedge rather than a band,
    // which looks like a chart and is not one.
    const { container } = render(<PricePath measurements={rising} />);
    const d = container.querySelector("path")?.getAttribute("d") ?? "";

    expect(d.startsWith("M ")).toBe(true);
    expect(d.trimEnd().endsWith("Z")).toBe(true);
    expect(numbersIn(d)).toHaveLength(12);
    expect(numbersIn(d).every(Number.isFinite)).toBe(true);
    expect(d).not.toContain("NaN");
  });

  it("never emits NaN, which draws nothing and reports nothing", () => {
    // The failure this file exists for. A single NaN in a `d` attribute makes a
    // browser discard the whole path without an error in the console, so the
    // chart is simply absent and nothing says why.
    const awkward = [
      measurement({ measured_at: 1, last_price: 5, peak_price: 5, trough_price: 5 }),
      measurement({ measured_at: 2, last_price: 5, peak_price: 5, trough_price: 5 }),
    ];
    const { container } = render(<PricePath measurements={awkward} />);
    expect(container.innerHTML).not.toContain("NaN");
    expect(container.querySelector("polyline")).not.toBeNull();
  });

  it("says what it cannot draw instead of drawing nothing", () => {
    // One point is not a line. An empty box would read as "this token did
    // nothing"; the message says the shortfall is in what Radar observed.
    const { container } = render(
      <PricePath measurements={[measurement({ measured_at: 1 })]} />,
    );
    expect(container.querySelector("svg")).toBeNull();
    expect(screen.getByText(/Not enough priced measurements/)).toBeTruthy();
  });

  it("labels the band as a running total rather than an interval range", () => {
    // The claim the caption has to make, because the picture cannot. Drawing
    // `max`/`min` folded from launch as if they were each interval's high and
    // low would tell a trader every interval reached those levels.
    render(<PricePath measurements={rising} />);
    expect(screen.getByText(/since launch/)).toBeTruthy();
    expect(screen.getByText(/not the range of each interval/)).toBeTruthy();
  });
});
