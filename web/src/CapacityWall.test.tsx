// SPDX-License-Identifier: Apache-2.0
//! That the capacity wall draws what was measured and not what was not.
//!
//! The chart's whole job is to be the honest picture of why the product does not
//! work yet, so the assertions that matter are the negative ones: what must
//! *not* appear in the bars.

import { render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CapacityWall } from "./CapacityWall";
import type { Capacity } from "./api";

function wall(over: Partial<Capacity> = {}): Capacity {
  return {
    as_of: 441_734_987,
    bands: [
      { floor: 0, ceiling: 25_000_000, decisions: 123 },
      { floor: 25_000_000, ceiling: 30_000_000, decisions: 461 },
      { floor: 30_000_000, ceiling: 35_000_000, decisions: 265 },
      { floor: 35_000_000, ceiling: 60_000_000, decisions: 116 },
      { floor: 60_000_000, ceiling: null, decisions: 25 },
    ],
    measured: 990,
    unmeasured: 0,
    median_capacity: 31_030_000,
    median_notional: 6_210_000,
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

describe("the capacity wall", () => {
  it("draws the bands research 0018 used, including the open-ended top", () => {
    serve(wall());
    render(<CapacityWall />);
    return waitFor(() => {
      expect(screen.getByText("$0–25")).toBeTruthy();
      expect(screen.getByText("$30–35")).toBeTruthy();
      // The band where a real position fits, and the one the argument turns on.
      expect(screen.getByText("$60+")).toBeTruthy();
    });
  });

  it("reports unmeasured capacity beside the chart, never inside it", async () => {
    // Rule 9. A capacity that could not be measured means "cannot exit" — not
    // "thin", and not zero. In the bottom band it would draw a wall of tokens
    // nobody measured, which is the opposite of what the picture is for.
    serve(wall({ unmeasured: 1384 }));
    render(<CapacityWall />);

    expect(await screen.findByText(/1,384 decisions have no measured/)).toBeTruthy();
    expect(screen.getByText(/not in the chart/)).toBeTruthy();
    // The bottom band still shows only what was measured into it.
    expect(screen.getByText(/^123 ·/)).toBeTruthy();
  });

  it("says nothing about unmeasured capacity when there is none", async () => {
    // A permanent warning that is usually vacuous is one people stop reading.
    serve(wall({ unmeasured: 0 }));
    render(<CapacityWall />);
    await screen.findByText("$60+");
    expect(screen.queryByText(/no measured capacity/)).toBeNull();
  });

  it("renders absent medians as absent rather than as zero dollars", async () => {
    serve(wall({ median_capacity: null, median_notional: null, measured: 0 }));
    render(<CapacityWall />);
    await waitFor(() => expect(screen.getAllByText("—").length).toBe(2));
    expect(screen.queryByText("$0.00")).toBeNull();
  });

  it("blames the probe rather than the venue when nothing was measured", async () => {
    // "No capacity" would read as a claim about the market. It is a claim about
    // what Radar has probed.
    serve(wall({ measured: 0, unmeasured: 40 }));
    render(<CapacityWall />);
    expect(
      await screen.findByText(/a fact about what Radar has probed, not about the venue/),
    ).toBeTruthy();
  });

  it("calls a malformed answer a fault rather than crashing the page", async () => {
    // It used to throw, and take the whole screen with it. `api.ts` types the
    // wire by hand and casts; a cast is a promise, and this is what happens when
    // the promise is broken.
    serve({ measured: 3 });
    render(<CapacityWall />);
    expect(await screen.findByText(/shape this page does not understand/)).toBeTruthy();
    expect(screen.getByText(/a fault, not an empty store/)).toBeTruthy();
  });

  it("says a fresh instance is not a fault", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(() =>
        Promise.resolve({
          ok: false,
          status: 503,
          statusText: "Service Unavailable",
          json: () => Promise.resolve({ error: "empty" }),
        }),
      ),
    );
    render(<CapacityWall />);
    expect(await screen.findByText(/recorded nothing yet/)).toBeTruthy();
  });
});
