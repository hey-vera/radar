// SPDX-License-Identifier: Apache-2.0
//! Every figure this site publishes, against the file it came from.
//!
//! # This file is here because a comment said it already was
//!
//! `index.html` has carried the sentence "checked by the same test that checks
//! the rendered page" since the `<noscript>` block was written, and no such test
//! existed. The block is the version of the page that link unfurlers and search
//! crawlers read — the readers this product's distribution actually depends on —
//! so it was the copy of the figures least likely to be looked at and most
//! likely to go stale. That is finding S15, and this is the fix.
//!
//! # What is pinned where, and why not all of it is pinned here
//!
//! - `index.html` against `fixtures/stats.json`, through the same `honesty.ts`
//!   functions the rendered page uses. Not against literals: a test that
//!   restated the numbers would be a third copy to update.
//! - `fixtures/stats.json`'s band, cost and aftermath blocks against
//!   `docs/research/data/0024-base-rates.json`, which is where they were
//!   measured. Read with `node:fs` because it lives outside the site's module
//!   graph and should not be bundled into a page.
//! - The fee ladder is **not** checked here. Its capture is hex, and decoding it
//!   in TypeScript would be a second implementation of `radar-pumpfun`'s fee
//!   parser. That check lives in that crate, in
//!   `the_site_publishes_the_ladder_this_crate_decodes.rs`, where the decoder
//!   already is. Named here so the absence reads as a decision.
//!
//! The `watched` block is deliberately not pinned to a file: it is read off the
//! production box's creator index, which is not in the repository. Its derived
//! strings are pinned in `honesty.test.ts` instead, which is the check that
//! caught the last refresh.

import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it } from "vitest";

import ladder from "./fixtures/fee-ladder.json";
import stats from "./fixtures/stats.json";
import { count, graduated, pct, share, type Stats } from "./honesty";

const s = stats as Stats;

/**
 * A file from the repository, by path relative to `site/`.
 *
 * Resolved from the vitest root rather than from `import.meta.url`: under
 * vitest that URL is rewritten to a `/@fs/` form, and `fileURLToPath` turns it
 * into a path with the drive letter twice. `process.cwd()` is the site
 * directory for every runner configured here.
 */
function repoFile(relative: string): string {
  return readFileSync(resolve(process.cwd(), relative), "utf8");
}

const INDEX_HTML = repoFile("index.html");

describe("index.html states what the fixture says", () => {
  // Derived here the same way the page derives them. If `pct` changes its
  // rounding, this fails and the HTML has to be regenerated -- which is
  // correct, because the two would otherwise disagree silently.
  const launches = count(s.watched.launches);
  const creators = count(s.watched.creators);
  const graduate = pct(graduated(s.watched));
  const stillborn = pct(share(s.watched.stillborn, s.watched.measured));

  it("uses the fixture's counts in the noscript block", () => {
    expect(INDEX_HTML).toContain(
      `${launches} launches watched, across ${creators} creators.`,
    );
    expect(INDEX_HTML).toContain(`${graduate} of measured launches ever graduate.`);
    expect(INDEX_HTML).toContain(`${stillborn} show almost no activity at all.`);
  });

  it("uses the fixture's counts in the meta description and the card", () => {
    // Three copies of the headline figures live in the head -- description,
    // og:description and twitter:description -- plus the image alt. Every one
    // of them is a claim to a stranger who never loads the page.
    const claim = `${launches} pump.fun launches watched. ${graduate} ever graduate.`;
    const copies = INDEX_HTML.split(claim).length - 1;
    expect(copies).toBe(3);
    expect(INDEX_HTML).toContain(
      `content="Cabal Hunter: ${launches} launches watched, ${graduate} ever graduate."`,
    );
  });

  it("dates the noscript figures with the day they were measured", () => {
    // "Measured on <date>" with no date, or with a date that is not the
    // fixture's, is the failure 0024 records in capitals.
    const day = s.measured_at.slice(0, 10);
    expect(INDEX_HTML).toContain(`Measured on ${day}.`);
  });

  it("names the band the data actually leads with", () => {
    // Hard-coded in the HTML because there is no JavaScript in a noscript
    // block. Pinned so it cannot state last month's winner.
    const top = s.bands.rows.reduce((a, b) =>
      b.x_base_instant > a.x_base_instant ? b : a,
    );
    expect(INDEX_HTML).toContain(`${top.lo}–${top.hi} recipients`);
    expect(INDEX_HTML).toContain(`${top.x_base_instant.toFixed(1)}×`);
  });
});

describe("the fixture states what 0024 measured", () => {
  const base = JSON.parse(repoFile("../docs/research/data/0024-base-rates.json"));

  it("carries 0024's bands, under this site's own field names", () => {
    // `fires_on` there, `share_of_launches` here. The rename is the reason to
    // check rather than a reason not to: two names for one quantity is exactly
    // where a copy drifts.
    const measured = base.launch_block.bands as {
      name: string;
      lo: number;
      hi: number;
      fires_on: number;
      p_instant: number;
      x_base_instant: number;
    }[];
    for (const row of s.bands.rows) {
      const from = measured.find((m) => m.name === row.name);
      expect(from, `0024 has no band called ${row.name}`).toBeTruthy();
      expect(row.lo).toBe(from!.lo);
      expect(row.hi).toBe(from!.hi);
      expect(row.share_of_launches).toBe(from!.fires_on);
      expect(row.p_instant).toBe(from!.p_instant);
      expect(row.x_base_instant).toBe(from!.x_base_instant);
    }
  });

  it("carries 0024's population, date and base rate for the bands", () => {
    expect(s.bands.measured_on).toBe(base.measured_on);
    expect(s.bands.launches).toBe(base.launch_block.launches);
    expect(s.bands.base_rate_instant).toBe(base.launch_block.base_rate_instant);
  });

  it("quotes the round trip for the band it names, not another one", () => {
    // 0024 records five bands and STATE.md warns never to publish one without
    // knowing which. Looking the figure up by the band the site names is what
    // stops the page quoting $2-$20's 250 bps beside the words "$20-$200".
    const bands = base.round_trip_bps.by_notional as {
      band: string;
      round_trip: number;
    }[];
    const row = bands.find((b) => b.band === s.cost.band);
    expect(row, `0024 has no round trip for ${s.cost.band}`).toBeTruthy();
    expect(s.cost.round_trip_bps).toBe(row!.round_trip);
  });

  it("carries 0024's aftermath median", () => {
    expect(s.aftermath.organic_median_bps).toBe(
      base.aftermath.organic_median_bps,
    );
  });
});

describe("the card that unfurls when the link is shared", () => {
  const png = readFileSync(resolve(process.cwd(), "public/og.png"));

  it("is a PNG at the size the meta tags promise", () => {
    // Every unfurler crops to what og:image:width and og:image:height say. A
    // file whose real size is not that size is letterboxed by somebody else's
    // rules, on the one image that is the product's first impression.
    expect(png.subarray(0, 8).toString("hex")).toBe("89504e470d0a1a0a");
    // IHDR is the first chunk: 8 bytes of magic, 4 of length, 4 of type, then
    // width and height as big-endian u32.
    expect(png.readUInt32BE(16)).toBe(1200);
    expect(png.readUInt32BE(20)).toBe(630);
    expect(INDEX_HTML).toContain('content="1200"');
    expect(INDEX_HTML).toContain('content="630"');
  });
});

describe("the fee ladder the tokenomics page renders", () => {
  // The rows are checked against the chain by radar-pumpfun. What is checked
  // here is only that the file this page imports has the shape the page walks,
  // so a truncated fixture fails as a test rather than as an empty table.
  it("has every row the page needs, with a creator share on each", () => {
    expect(ladder.after_graduation.rows.length).toBe(25);
    for (const row of ladder.after_graduation.rows) {
      expect(typeof row.from_sol).toBe("number");
      expect(typeof row.creator_bps).toBe("number");
      expect(typeof row.protocol_bps).toBe("number");
      expect(typeof row.lp_bps).toBe("number");
    }
    expect(ladder.curve.creator_bps).toBe(30);
  });
});
