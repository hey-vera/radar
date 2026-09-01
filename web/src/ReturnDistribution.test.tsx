// SPDX-License-Identifier: Apache-2.0
//! That the distribution shows the caveat the median hides.
//!
//! The whole reason this component exists is research 0017's point mass: 24–43%
//! of the population returns exactly zero, and a median over that is a report
//! about the point mass. So the assertions are about where that figure appears
//! and, more importantly, where it does not.

import { render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ReturnDistribution } from "./ReturnDistribution";
import type { Returns } from "./api";

function dist(over: Partial<Returns> = {}): Returns {
  return {
    as_of: 441_734_987,
    buckets: [
      { floor: null, ceiling: -5_000, scored: 210 },
      { floor: -5_000, ceiling: -2_000, scored: 140 },
      { floor: -2_000, ceiling: -850, scored: 90 },
      { floor: -850, ceiling: 0, scored: 60 },
      { floor: 1, ceiling: 850, scored: 40 },
      { floor: 850, ceiling: 2_000, scored: 20 },
      { floor: 2_000, ceiling: null, scored: 10 },
    ],
    exactly_zero: 420,
    scored: 990,
    unscored: 3_384,
    round_trip_bps: 850,
    ...over,
  };
}

function serve(body: unknown) {
  vi.stubGlobal(
    "fetch",
    vi.fn(() =>
      Promise.resolve({
        ok: true,
        status: 200,
        statusText: "OK",
        json: () => Promise.resolve(body),
      }),
    ),
  );
}

afterEach(() => vi.unstubAllGlobals());

describe("the return distribution", () => {
  it("makes the zero share the headline rather than a bar", async () => {
    // Drawn as one more bucket it would be the tallest thing on the chart and
    // read as a finding about the market. It is a fact about a venue where most
    // tokens trade a handful of times and stop.
    serve(dist());
    render(<ReturnDistribution />);

    expect(await screen.findByText(/420 of 990 \(42\.4%\) returned exactly zero/)).toBeTruthy();
    expect(screen.getByText(/not drawn below/)).toBeTruthy();
  });

  it("names the note rather than only reporting the number", async () => {
    // A percentage with no explanation reads as a data quality problem. It is a
    // property of the venue, and 0017 is where that is argued.
    serve(dist());
    render(<ReturnDistribution />);
    expect(await screen.findByText(/research\s+0017/)).toBeTruthy();
  });

  it("says the figures are gross and marks the round trip rather than subtracting it", async () => {
    serve(dist());
    render(<ReturnDistribution />);
    expect(await screen.findByText(/gross/)).toBeTruthy();
    expect(screen.getByText(/8\.5% round\s+trip/)).toBeTruthy();
  });

  it("reports unscored decisions as absent rather than as flat", async () => {
    // A decision that never reached the exit probe has no entry price. Folding
    // those into "broke even" is the whole population's median dressed up as a
    // result.
    serve(dist());
    render(<ReturnDistribution />);
    expect(
      await screen.findByText(/3,384 decisions could not be scored/),
    ).toBeTruthy();
  });

  it("blames the measurement rather than the market when nothing is scored", async () => {
    serve(dist({ scored: 0, exactly_zero: 0, unscored: 40 }));
    render(<ReturnDistribution />);
    expect(await screen.findByText(/Nothing has been scored yet/)).toBeTruthy();
    // And no chart, rather than an empty one implying every bucket is zero.
    expect(screen.queryByText(/returned exactly zero/)).toBeNull();
  });

  it("labels the open-ended ends as open-ended", async () => {
    // A token can go to nothing and occasionally one does not. A bounded label
    // at either end would misstate what the bar contains.
    serve(dist());
    render(<ReturnDistribution />);
    expect(await screen.findByText(/below -50%/)).toBeTruthy();
    expect(screen.getByText(/\+20% and up/)).toBeTruthy();
  });

  it("calls a malformed answer a fault rather than crashing the page", async () => {
    serve({ scored: 3 });
    render(<ReturnDistribution />);
    expect(await screen.findByText(/shape this page does not understand/)).toBeTruthy();
  });
});
