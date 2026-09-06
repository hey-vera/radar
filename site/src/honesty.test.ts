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
  handleHref,
  mintShaped,
  mostCoordinated,
  pct,
  safeHref,
  share,
  solscanAccount,
  solscanTx,
  summonIntent,
  userHref,
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
    // Checked against the creator index on the production box at slot
    // 444,637,451, built 2026-09-05T22:55:16Z: 527,490 measured, 9,431 organic,
    // 5,708 instant, 119,318 stillborn.
    //
    // This is the test that would have caught the reply bug shipped on
    // 2026-09-04: a figure looked up by the wrong key published a cost 6.7x the
    // real one, and only running it against production data found it.
    //
    // It earned its keep again on the refresh above. The strings below were
    // predicted before the fixture moved and one of them was predicted wrong --
    // 1.79% was read as the *instant* share, which is 1.08%, and it is the
    // organic one. The prediction was discarded and these three are what the
    // code prints. That is the whole arrangement: nobody's arithmetic gets to
    // decide what this page says.
    expect(pct(graduated(stats.watched))).toBe("2.87%");
    expect(pct(share(stats.watched.stillborn, stats.watched.measured))).toBe(
      "22.6%",
    );
    // 9,431 of 527,490 is 1.787902%.
    expect(pct(share(stats.watched.organic, stats.watched.measured))).toBe(
      "1.79%",
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

describe("links", () => {
  // Refusals first, because a link helper that never returns null is a link
  // helper that is not doing anything.

  it("refuses a scheme that is not https", () => {
    // The one that matters. React warns on this and does not block it.
    expect(safeHref("javascript:alert(1)", ["x.com"])).toBe(null);
    expect(safeHref("data:text/html,<script>", ["x.com"])).toBe(null);
    expect(safeHref("http://x.com/a", ["x.com"])).toBe(null);
  });

  it("refuses a host that only looks like the allowed one", () => {
    // Every hand-rolled version of this check is a prefix match, and every one
    // of them passes this case.
    expect(safeHref("https://evil.example/#@x.com", ["x.com"])).toBe(null);
    expect(safeHref("https://x.com.evil.example/a", ["x.com"])).toBe(null);
    expect(safeHref("https://notx.com/a", ["x.com"])).toBe(null);
    expect(safeHref("not a url at all", ["x.com"])).toBe(null);
  });

  it("allows the host it was given", () => {
    expect(safeHref("https://x.com/CabalHunter", ["x.com"])).toBe(
      "https://x.com/CabalHunter",
    );
  });

  it("refuses a sixteenth character in a handle", () => {
    // X's own bound. A 16-character handle renders as a link to a profile that
    // does not exist, on the page introducing the account.
    expect(handleHref("a".repeat(15))).toBe(`https://x.com/${"a".repeat(15)}`);
    expect(handleHref("a".repeat(16))).toBe(null);
    expect(handleHref("")).toBe(null);
    expect(handleHref("has space")).toBe(null);
    expect(handleHref("has-dash")).toBe(null);
    expect(handleHref("@leading")).toBe(null);
  });

  it("refuses anything but digits in a user id", () => {
    expect(userHref("1234567890")).toBe("https://x.com/i/user/1234567890");
    expect(userHref("12a")).toBe(null);
    expect(userHref("")).toBe(null);
    expect(userHref("../../evil")).toBe(null);
  });

  it("refuses a signature with a character base58 does not have", () => {
    // '0' is excluded from base58 precisely because it is confusable with 'O',
    // and a signature containing one did not come off a chain.
    const good = "5".repeat(88);
    expect(solscanTx(good)).toBe(`https://solscan.io/tx/${good}`);
    expect(solscanTx(`0${"5".repeat(87)}`)).toBe(null);
    expect(solscanTx("5".repeat(85))).toBe(null);
    expect(solscanTx("5".repeat(89))).toBe(null);
  });

  it("reads a mint the way the bot's own parser does", () => {
    // 32 to 44, bounds exact -- mention.rs MIN_ADDRESS and MAX_ADDRESS. A
    // reader whose paste this rejects would have been refused by the bot too,
    // and the summon box should say so before it costs them a post.
    expect(mintShaped("a".repeat(32))).toBe(true);
    expect(mintShaped("a".repeat(44))).toBe(true);
    expect(mintShaped("a".repeat(31))).toBe(false);
    expect(mintShaped("a".repeat(45))).toBe(false);
    expect(mintShaped(`  ${"a".repeat(32)}  `)).toBe(true);
    expect(mintShaped(`0${"a".repeat(31)}`)).toBe(false);
    expect(solscanAccount("a".repeat(44))).toBe(
      `https://solscan.io/account/${"a".repeat(44)}`,
    );
    expect(solscanAccount("not an address")).toBe(null);
  });

  it("builds a summons only when both halves are real", () => {
    const mint = "a".repeat(43);
    expect(summonIntent("CabalHunter", mint)).toBe(
      `https://x.com/intent/post?text=%40CabalHunter%20${mint}`,
    );
    // A button that posts "@undefined <mint>" is worse than no button.
    expect(summonIntent("", mint)).toBe(null);
    expect(summonIntent("a".repeat(16), mint)).toBe(null);
    expect(summonIntent("CabalHunter", "not an address")).toBe(null);
  });

  it("encodes the mint rather than pasting it into a query string", () => {
    // The mint is address-shaped by the time it reaches here, so nothing needs
    // escaping today. The encoding is asserted anyway: the day this function
    // takes a ticker instead, '$' and '&' arrive with it.
    const url = summonIntent("CabalHunter", "a".repeat(32));
    expect(url).not.toBe(null);
    expect(url).toContain("%40");
    expect(url).not.toContain("@");
  });
});
