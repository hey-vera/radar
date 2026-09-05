// SPDX-License-Identifier: Apache-2.0
//! What this site is allowed to claim.
//!
//! Every case here is a sentence a stranger would read about somebody else's
//! project. That is why the empty and missing cases get as much attention as the
//! populated ones: the wrong version of each of these looks completely right.

import { describe, expect, it } from "vitest";

import fixture from "./fixtures/stats.json";
import {
  bps,
  cost,
  count,
  graduated,
  measuredAgo,
  mostCoordinated,
  pct,
  share,
  type Stats,
} from "./honesty";

const stats = fixture as unknown as Stats;

describe("share", () => {
  it("divides by what was measured, never by what was launched", () => {
    // The gap between the two is Cabal Hunter's own outcome backlog. Dividing
    // by `launches` folds that lag into a claim about pump.fun and understates
    // every rate by the size of the queue.
    //
    // Half the population unmeasured here, so the wrong denominator halves
    // every figure -- a 50% rate published as 25%.
    expect(share(50, 100)).toBeCloseTo(0.5, 12);
    expect(share(30, 100)).toBeCloseTo(0.3, 12);
  });

  it("refuses rather than returning zero when nothing was measured", () => {
    // Rule 9. `0` here is "nothing on this venue ever graduates", which is both
    // false and the direction that sounds authoritative.
    expect(share(0, 0)).toBeNull();
    expect(share(5, 0)).toBeNull();
    expect(share(1, -1)).toBeNull();
  });
});

describe("the figures the landing page leads with", () => {
  it("matches what was measured on the box", () => {
    // Checked against the creator index summed on the production box at slot
    // 444,374,676 on 2026-09-04: 506,991 measured, 8,999 organic, 5,230
    // instant, 116,427 stillborn.
    //
    // This is the test that would have caught the reply bug shipped the same
    // day: a figure looked up by the wrong key published a cost 6.7x the real
    // one, and only running it against production data found it.
    expect(pct(graduated(stats.watched))).toBe("2.81%");
    expect(pct(share(stats.watched.stillborn, stats.watched.measured))).toBe(
      "23.0%",
    );
    // 8,999 of 506,991 is 1.774983%, which rounds down. Written as 1.78 first,
    // from arithmetic done in my head rather than by the code -- the exact
    // habit this file exists to refuse.
    expect(pct(share(stats.watched.organic, stats.watched.measured))).toBe(
      "1.77%",
    );
  });

  it("names the band furthest above the base rate rather than assuming one", () => {
    // The headline claim rests on this row. Hard-coding it would leave the
    // sentence stating last month's winner beside this month's date.
    const top = mostCoordinated(stats.bands.rows);
    expect(top?.name).toBe("ten to thirteen");
    expect(top?.x_base_instant).toBe(10.1);
  });

  it("has no band at all to name when there are none", () => {
    expect(mostCoordinated([])).toBeNull();
  });

  it("keeps the aftermath figure negative, because that is the point", () => {
    // Graduating is not winning. Research 0011: organic graduations end at a
    // median of -3,228 bps. A site that lost this sign would be arguing the
    // opposite of what the data says.
    expect(stats.aftermath.organic_median_bps).toBeLessThan(0);
    expect(bps(stats.aftermath.organic_median_bps)).toBe("-32.3%");
  });
});

describe("pct", () => {
  it("keeps the precision the measurement supports and no more", () => {
    // 2.81% rounded to 3% loses its meaning; 23.00% claims a precision the
    // sample does not carry.
    expect(pct(0.0281)).toBe("2.81%");
    expect(pct(0.23)).toBe("23.0%");
    expect(pct(0.0002)).toBe("0.02%");
  });

  it("renders a missing measurement as words, never as a number", () => {
    expect(pct(null)).toBe("not measured");
  });
});

describe("bps", () => {
  it("leaves zero unsigned", () => {
    // `+0.0%` reads as a gain that rounded away.
    expect(bps(0)).toBe("0.0%");
    expect(bps(456)).toBe("+4.6%");
    expect(bps(-3228)).toBe("-32.3%");
  });
});

describe("measuredAgo", () => {
  const now = new Date("2026-09-05T00:00:00Z");

  it("says how stale each figure is, in words", () => {
    expect(measuredAgo("2026-09-05T00:00:00Z", now)).toBe("just now");
    expect(measuredAgo("2026-09-04T23:00:00Z", now)).toBe("60 minutes ago");
    expect(measuredAgo("2026-09-04T12:00:00Z", now)).toBe("12 hours ago");
    expect(measuredAgo("2026-08-30T00:00:00Z", now)).toBe("6 days ago");
  });

  it("says nothing rather than guessing at a broken timestamp", () => {
    // A clock skew rendering "in 3 hours" looks like a bug in the data, which
    // is worse for trust than an absent line.
    expect(measuredAgo("not a date", now)).toBeNull();
    expect(measuredAgo("2026-09-06T00:00:00Z", now)).toBeNull();
  });
});

describe("count", () => {
  it("separates thousands, because 508814 is unreadable", () => {
    expect(count(508814)).toMatch(/508.814/);
  });
});

describe("cost", () => {
  it("never signs a charge as though it were a gain", () => {
    // Shipped as `+4.6%` on the landing page and caught by looking at the page
    // rather than by a test. A round trip is money that leaves whichever way
    // the trade goes; `bps` signs returns, where the sign carries meaning.
    //
    // It is the flattering direction, on the one figure the page uses to warn
    // people with.
    expect(cost(456)).toBe("4.6%");
    expect(cost(456)).not.toContain("+");
    // And a cost handed in already negative is still a cost, not a gain.
    expect(cost(-456)).toBe("4.6%");
  });
});
