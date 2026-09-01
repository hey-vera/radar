// SPDX-License-Identifier: Apache-2.0
//! That the cost curve carries the note's caveats and not only its numbers.
//!
//! Every figure on this screen comes from one measured hour, and 0019 spends
//! more words on what cannot be concluded from it than on what can. A chart that
//! ships the table and drops the caveats is the failure this whole interface
//! exists to avoid — so the caveats are what is asserted here.

import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { CostCurve } from "./CostCurve";

describe("the cost curve", () => {
  it("marks the band Radar's own floor lands in", () => {
    // The finding. `min_notional` is $1.00, which is in the band that costs
    // 1,521 bps a leg — so a position at the system's own floor faces a round
    // trip of roughly 30%, and nothing in the system knew that.
    render(<CostCurve />);
    expect(screen.getByText("← floor")).toBeTruthy();
    // Twice: once on the bar and once in the prose above it. Both are
    // deliberate -- the number is the finding, and a reader who skims the
    // chart should still meet it in the sentence.
    expect(screen.getAllByText(/1,521/).length).toBeGreaterThanOrEqual(1);
  });

  it("marks where the median proposal actually sits", () => {
    // One band up from the floor, and an order of magnitude cheaper. The floor
    // and the median living in bands whose costs differ tenfold is the point.
    render(<CostCurve />);
    expect(screen.getByText("← median")).toBeTruthy();
  });

  it("says the figures are a leg rather than a round trip", () => {
    // A table silently doubled is one nobody can check against the note it
    // claims to come from.
    render(<CostCurve />);
    expect(screen.getByText(/double it/i)).toBeTruthy();
  });

  it("carries the non-monotonic caveat the note insists on", () => {
    // 0019 says in as many words that the 125 should not be leaned on. A chart
    // that draws it as a clean downward curve is making a claim the measurement
    // refused to make.
    render(<CostCurve />);
    expect(screen.getByText(/not monotonic/i)).toBeTruthy();
    expect(screen.getByText(/should not be leaned on/i)).toBeTruthy();
  });

  it("says failed transactions are excluded, so the real cost is higher", () => {
    // They burn a fee against no notional. Omitting this makes every figure look
    // like a ceiling when it is a floor.
    render(<CostCurve />);
    expect(screen.getByText(/failed transactions are excluded/i)).toBeTruthy();
  });

  it("dates the measurement on the screen rather than in a comment", () => {
    // It is one hour, months ago, and has not been re-run. A number with no date
    // reads as current.
    render(<CostCurve />);
    expect(screen.getByText(/2026-08-25/)).toBeTruthy();
    expect(screen.getByText(/constant on this page rather than a live/i)).toBeTruthy();
  });
});
