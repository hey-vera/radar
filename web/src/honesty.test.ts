// SPDX-License-Identifier: Apache-2.0
//! The interface's claims, tested where they are computed.
//!
//! This file is the frontend's answer to the question the Rust side answers with
//! 856 tests: **which of the things this code says are true?**
//!
//! It deliberately does not test that buttons render. A snapshot of a `<div>` is
//! a test that fails when someone changes a class name and passes when the page
//! lies. What is worth pinning is the small set of pure functions that decide
//! *what the page claims* — the median it prints, the count it says cleared
//! costs, and which refusals it collapses into "policy closed".
//!
//! Each of those has a wrong version that looks right, which is the only reason
//! to test any of them.

import { describe, expect, it } from "vitest";

import { POLICY_ARTIFACTS, clearedCost, median, pct } from "./honesty";

describe("median", () => {
  it("is the middle of the sorted values, not of the given order", () => {
    // The bug this catches: forgetting to sort. Returns can arrive from the API
    // in decision order, and the "median" of an unsorted list is whatever
    // happened to land in the middle — a number that looks plausible and means
    // nothing.
    expect(median([500, -1000, 200])).toBe(200);
    expect(median([-1000, 200, 500])).toBe(200);
  });

  it("sorts numerically rather than as strings", () => {
    // JavaScript's default sort is lexicographic, so `[10, 9, 100].sort()` is
    // `[10, 100, 9]`. On basis points that is not a rounding error, it is a
    // different token.
    expect(median([10, 9, 100])).toBe(10);
    expect(median([-2000, -300, -40])).toBe(-300);
  });

  it("averages the two middles on an even count", () => {
    expect(median([100, 300])).toBe(200);
    // And rounds rather than emitting a fraction of a basis point, which would
    // render as `-13.405%` and imply precision nobody measured.
    expect(median([100, 301])).toBe(201);
  });

  it("is null for an empty cohort rather than zero", () => {
    // Rule 9, in the interface. A cohort with nothing in it has no median, and
    // rendering that as 0% would print "broke even" for a measurement that was
    // never taken.
    expect(median([])).toBeNull();
  });
});

describe("clearedCost", () => {
  it("counts strictly above the cost, not at it", () => {
    // A round trip that returned exactly its cost cleared nothing. Counting it
    // makes the headline figure — the share of tokens that beat costs —
    // overstate itself at precisely the boundary the number exists to describe.
    expect(clearedCost([849, 850, 851], 850)).toBe(1);
  });

  it("counts nothing in an empty cohort", () => {
    expect(clearedCost([], 850)).toBe(0);
  });

  it("does not treat a loss as a gain", () => {
    expect(clearedCost([-9000, -100, 0], 850)).toBe(0);
  });
});

describe("pct", () => {
  it("signs a gain and does not sign a loss twice", () => {
    expect(pct(1234)).toBe("+12.3%");
    expect(pct(-1340)).toBe("-13.4%");
  });

  it("does not sign zero as a gain", () => {
    // "+0.0%" reads as a gain that rounded away. Zero is zero.
    expect(pct(0)).toBe("0.0%");
  });
});

describe("POLICY_ARTIFACTS", () => {
  it("contains every refusal a closed policy produces on its own", () => {
    // The seven that fire together under `Policy::CLOSED` because every limit
    // is zero and every comparison against zero fails. Rendering them
    // individually tells a novice there are seven problems with a token when
    // there is one fact about the policy.
    for (const artifact of [
      "NoAutonomy",
      "OverPositionLimit",
      "OverDeploymentLimit",
      "OverCreatorLimit",
      "DailyLossReached",
      "RoundTripTooExpensive",
      "InputsTooStale",
    ]) {
      expect(POLICY_ARTIFACTS.has(artifact)).toBe(true);
    }
  });

  it("does not swallow a refusal that is about the token", () => {
    // The other direction, and the one that matters more. These are findings —
    // the exit could not be simulated, or was too small — and collapsing them
    // into "policy closed" would hide the only refusals that say anything about
    // the token being looked at.
    for (const finding of [
      "ExitNotSimulated",
      "ExitCapacityTooSmall",
      "OverCanaryLimit",
      "Halted",
      "TooManyFailures",
    ]) {
      expect(POLICY_ARTIFACTS.has(finding)).toBe(false);
    }
  });
});
