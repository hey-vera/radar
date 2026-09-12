// SPDX-License-Identifier: Apache-2.0
//! That the interface's pages and the server's route audiences agree.
//!
//! This is the only thing standing between two lists that live in different
//! languages, and the failure it prevents is silent in the worst way.
//!
//! `access::audience_of` is total and its fallback is `Audience::Operator`. So a
//! page added to `ROUTES` and forgotten in `access.rs` is **not** a 404 — it is
//! a page that quietly requires operator identity. Today that is invisible,
//! because with no customer authenticator configured every route requires
//! operator identity anyway. It becomes visible on the day the customer lane
//! switches on, for every customer at once, on a deploy that changed nothing
//! about the frontend.
//!
//! So the check reads the Rust source. It is a string search rather than a
//! parse, and that is the right trade for a rule this narrow: it can only fail
//! by being *too strict*, which is loud, never by missing a route, which is not.
//!
//! # Why there is no operator case here any more
//!
//! `ROUTES` used to list `/instance` and `/analyst`, the two operator pages the
//! decision-record interface drew. Both were deleted along with the components
//! that drew them (`Health.tsx`, `Analyst.tsx`) -- the terminal has no operator
//! surface, so `ROUTES` has no operator entries, and the tests that swept
//! `ROUTES.filter(r => r.audience === "operator")` now run over an empty list.
//! That is not a hole in coverage: an empty `it.each` runs zero cases rather
//! than failing, and there is nothing left for that direction of the check to
//! find, because there is nothing left in this file classified operator. If an
//! operator page returns to the client, its test case returns with it.

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { ROUTES, isMintLike, tokenPath, type Audience } from "./routes";

/**
 * `ROUTES` today has only customer entries, so TypeScript narrows its
 * `audience` field to the literal `"customer"` and flags a comparison against
 * `"operator"` as unreachable. It is not unreachable in the sense that
 * matters here -- the check exists precisely so an operator route *added*
 * later is caught -- so the comparison is done through the wider [`Audience`]
 * type rather than removed.
 */
function isAudience(route: { audience: string }, audience: Audience): boolean {
  return (route.audience as Audience) === audience;
}

const ACCESS_RS = resolve(
  __dirname,
  "../../crates/radar-serve/src/access.rs",
);

const source = readFileSync(ACCESS_RS, "utf8");

/**
 * The body of `audience_of`'s customer expression.
 *
 * Everything from `let customer =` to the statement's semicolon. Taking the
 * whole function would match the route table in its own tests, which lists
 * operator paths too — and the check would then pass for a path classified the
 * wrong way.
 */
function customerBlock(): string {
  // `let customer` without the `= `, because rustfmt decides where the line
  // breaks and it moved the `=` onto the next line on 2026-09-12 when the
  // expression got shorter. Eleven route assertions then failed at once,
  // reporting that every page was unreachable -- an alarming way to be told
  // about a line break. What this check is for is the two lists agreeing, not
  // the shape the formatter chose.
  const start = source.indexOf("let customer");
  expect(start, "`let customer` not found in access.rs").toBeGreaterThan(-1);
  const end = source.indexOf(";", start);
  expect(end, "the customer expression is not terminated").toBeGreaterThan(start);
  return source.slice(start, end);
}

/**
 * The body of `audience_of`'s public expression.
 *
 * The shell — `/`, the bundle, and the client routes that serve the same HTML —
 * moved here on 2026-09-08 so a visitor reaches the interface's own sign-in
 * control instead of the operator's identity provider.
 *
 * So a customer page may now be classified `Public` *or* `Customer`, and this
 * check has to read both. What it must not lose is the direction that leaks: an
 * **operator** page in either block is an operator page a stranger can reach.
 */
function publicBlock(): string {
  const start = source.indexOf('if path == "/health"');
  expect(start, "the public expression not found in access.rs").toBeGreaterThan(-1);
  const end = source.indexOf("return Audience::Public;", start);
  expect(end, "the public expression is not terminated").toBeGreaterThan(start);
  return source.slice(start, end);
}

/** Everything `audience_of` hands to somebody who is not the operator. */
function reachableBlock(): string {
  return `${publicBlock()}
${customerBlock()}`;
}

/** A wouter pattern reduced to what the server would match on. */
function serverPath(pattern: string): string {
  // `/token/:mint` is `path.starts_with("/token/")` on the server. Anything
  // else is an exact path.
  const param = pattern.indexOf("/:");
  return param === -1 ? pattern : pattern.slice(0, param + 1);
}

describe("the route table matches the server", () => {
  it("reads the real access.rs rather than passing on an empty string", () => {
    // Without this every assertion below holds vacuously the moment the path
    // changes — which is exactly how a cross-language check dies.
    expect(source.length).toBeGreaterThan(1000);
    expect(source).toContain("pub fn audience_of");
  });

  it.each(ROUTES.filter((r) => r.audience === "customer").map((r) => [r.path]))(
    "%s is reachable without operator identity",
    (pattern) => {
      const path = serverPath(pattern);
      expect(
        reachableBlock(),
        `${pattern} is in neither audience_of's public nor its customer list, ` +
          `so the server will treat it as Audience::Operator and refuse it to ` +
          `every customer`,
      ).toContain(`"${path}"`);
    },
  );

  it.each(ROUTES.filter((r) => isAudience(r, "operator")).map((r) => [r.path]))(
    "%s is not handed to customers or to strangers by the server",
    (pattern) => {
      // The other direction, and the one that would actually leak. An operator
      // page in the customer list is a page a paying customer can read; in the
      // public list it is a page anybody can read. Both blocks are checked,
      // because the shell moving to `Public` created the second way to get this
      // wrong. `ROUTES` carries no operator entry today, so this runs zero
      // cases -- see the module comment above for why that is not a gap.
      expect(reachableBlock()).not.toContain(`"${serverPath(pattern)}"`);
    },
  );

  it("has at least one customer route, so the direction above is not vacuous", () => {
    expect(ROUTES.some((r) => r.audience === "customer")).toBe(true);
  });
});

describe("tokenPath", () => {
  it("encodes what it is given", () => {
    expect(tokenPath("So11111111111111111111111111111111111111112")).toBe(
      "/token/So11111111111111111111111111111111111111112",
    );
    // Not reachable from `isMintLike`, but this function is public and a path
    // built by concatenation is how a slash ends up meaning a route boundary.
    expect(tokenPath("a/b")).toBe("/token/a%2Fb");
  });
});

describe("isMintLike", () => {
  it("accepts real Solana addresses", () => {
    expect(isMintLike("So11111111111111111111111111111111111111112")).toBe(true);
    expect(isMintLike("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v")).toBe(true);
    // Leading and trailing space is a paste, not a typo.
    expect(isMintLike("  So11111111111111111111111111111111111111112  ")).toBe(true);
  });

  it("rejects the characters base58 leaves out", () => {
    // 0, O, I and l are excluded precisely because they look like each other,
    // and a string containing one is a transcription error rather than an
    // address. Each is tested separately: a single case would pass with three of
    // the four missing from the character class.
    const good = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
    for (const bad of ["0", "O", "I", "l"]) {
      expect(isMintLike(good.slice(0, -1) + bad), bad).toBe(false);
    }
  });

  it("rejects things that are the wrong length", () => {
    expect(isMintLike("")).toBe(false);
    expect(isMintLike("abc")).toBe(false);
    expect(isMintLike("a".repeat(31))).toBe(false);
    expect(isMintLike("a".repeat(45))).toBe(false);
    // The boundaries themselves, swept. Off by one here means either refusing a
    // real address with leading zero bytes or accepting a longer string.
    expect(isMintLike("a".repeat(32))).toBe(true);
    expect(isMintLike("a".repeat(44))).toBe(true);
  });

  it("rejects a URL or a sentence containing an address", () => {
    // It answers "is this an address", not "does this contain one". Anything
    // looser turns a pasted block of prose into a lookup.
    expect(
      isMintLike("https://solscan.io/token/So11111111111111111111111111111111111111112"),
    ).toBe(false);
    expect(isMintLike("mint So11111111111111111111111111111111111111112")).toBe(false);
  });
});

describe("the API routes the interface calls", () => {
  // Not pages, so not in `ROUTES` — but the same silent failure applies: an API
  // route the interface calls that the server classifies as Operator answers
  // today (everything falls back to the operator check) and refuses every
  // customer the day the customer lane switches on.
  //
  // The decision-record reads (`/v1/funnel`, `/v1/decisions`, `/v1/scoreboard`,
  // `/v1/tokens/`) were asserted here until the pages that called them were
  // deleted; a route this interface no longer calls is not this file's claim
  // to verify. `/v1/market/*` is not listed either, in the other direction: it
  // does not exist in `access.rs` yet (a parallel session is building it), so
  // asserting it here would fail against a classification that has not been
  // written, for a fact this test cannot yet check. See this session's return
  // for the classification the market routes still need.
  it.each([["/v1/customer/events"], ["/v1/customer/wallet"], ["/v1/chat"]])(
    "%s is a customer route on the server",
    (path) => {
      expect(customerBlock(), `${path} would refuse every customer`).toContain(
        `"${path}"`,
      );
    },
  );

  it.each([["/v1/store"], ["/v1/events"], ["/v1/link"], ["/mcp"], ["/ops"]])(
    "%s stays an operator route",
    (path) => {
      // `/v1/events` in particular: its payload is the operator's store counts,
      // and opening it to save writing a customer stream is how an operator
      // surface ends up in front of a paying stranger.
      expect(customerBlock()).not.toContain(`"${path}"`);
    },
  );
});
