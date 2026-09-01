// SPDX-License-Identifier: Apache-2.0
//! That the activity strip can show an outage.
//!
//! Its whole reason for existing is that the watermark cannot. The watermark
//! follows the chain, so it keeps advancing while the thing that takes decisions
//! is dead — and the follow recorder has exited on a query error before, with no
//! restart and no alarm, and nothing on any screen said so.
//!
//! So the assertions here are about the empty days: that they are drawn, that
//! they are drawn differently, and that they are counted where a reader will see
//! the number.

import { render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Activity } from "./Activity";
import type { Activity as Record } from "./api";

const DAY = 216_000;

function record(counts: number[]): Record {
  return {
    as_of: DAY * (counts.length + 10),
    intervals: counts.map((decisions, i) => ({
      from_slot: DAY * (10 + i),
      decisions,
      proposed: 0,
    })),
  };
}

function serve(body: unknown, ok = true, status = 200) {
  vi.stubGlobal(
    "fetch",
    vi.fn(() =>
      Promise.resolve({
        ok,
        status,
        statusText: "OK",
        json: () => Promise.resolve(body),
      }),
    ),
  );
}

afterEach(() => vi.unstubAllGlobals());

describe("the activity strip", () => {
  it("counts the days with no decisions where a reader will see it", async () => {
    // The number is the alarm. A gap in a row of small bars is easy to miss;
    // "3 days with none" is not.
    serve(record([900, 0, 0, 0, 880]));
    render(<Activity />);
    expect(await screen.findByText("3 days with none")).toBeTruthy();
  });

  it("draws an empty day rather than leaving it out", async () => {
    // The failure it exists to reveal. Bars drawn only for days that had data
    // close the gap and show an unbroken record straight over an outage.
    serve(record([900, 0, 880]));
    render(<Activity />);

    await screen.findByText(/last fortnight/);
    // One element per bucket, including the empty one.
    const bars = screen.getAllByTitle(/decisions,/);
    expect(bars.length).toBe(3);
    expect(bars.some((b) => b.getAttribute("title")?.includes("0 decisions"))).toBe(
      true,
    );
  });

  it("says nothing about empty days when there are none", async () => {
    // A warning that is usually vacuous is one people stop reading.
    serve(record([900, 880, 910]));
    render(<Activity />);
    await screen.findByText(/last fortnight/);
    expect(screen.queryByText(/with none/)).toBeNull();
  });

  it("explains why the gaps are drawn rather than only drawing them", async () => {
    serve(record([900, 0]));
    render(<Activity />);
    expect(
      await screen.findByText(/a chart that closes its own gaps cannot show that/),
    ).toBeTruthy();
  });

  it("renders nothing at all rather than an empty chart when there is no record", async () => {
    // A row of zero-height bars says the recorder ran and found nothing. On a
    // fresh instance nothing ran, and the screen below already says so.
    serve({ as_of: 1, intervals: [] });
    const { container } = render(<Activity />);
    await new Promise((r) => setTimeout(r, 0));
    expect(container.textContent).toBe("");
  });

  it("stays quiet on an empty store rather than repeating the message below it", async () => {
    serve({ error: "empty" }, false, 503);
    const { container } = render(<Activity />);
    await new Promise((r) => setTimeout(r, 0));
    expect(container.textContent).toBe("");
  });

  it("reports a real fault, since that is not the same as an empty store", async () => {
    serve({ error: "cannot read the store" }, false, 500);
    render(<Activity />);
    expect(await screen.findByText(/Could not read the recorder/)).toBeTruthy();
  });
});
