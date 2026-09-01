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

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { ROUTES, isMintLike, navFor, tokenPath } from "./routes";

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
  const start = source.indexOf("let customer = ");
  expect(start, "`let customer =` not found in access.rs").toBeGreaterThan(-1);
  const end = source.indexOf(";", start);
  expect(end, "the customer expression is not terminated").toBeGreaterThan(start);
  return source.slice(start, end);
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
    "%s is classified as a customer route by the server",
    (pattern) => {
      const path = serverPath(pattern);
      expect(
        customerBlock(),
        `${pattern} is not in audience_of's customer list, so the server will ` +
          `treat it as Audience::Operator and refuse it to every customer`,
      ).toContain(`"${path}"`);
    },
  );

  it.each(ROUTES.filter((r) => r.audience === "operator").map((r) => [r.path]))(
    "%s is not handed to customers by the server",
    (pattern) => {
      // The other direction, and the one that would actually leak. An operator
      // page appearing in the customer list is a page a paying customer can
      // read.
      expect(customerBlock()).not.toContain(`"${serverPath(pattern)}"`);
    },
  );

  it("has at least one route of each audience, so neither case is vacuous", () => {
    expect(ROUTES.some((r) => r.audience === "customer")).toBe(true);
    expect(ROUTES.some((r) => r.audience === "operator")).toBe(true);
  });
});

describe("navigation", () => {
  it("hides operator pages from a customer, and shows them to an operator", () => {
    // `access::Audience` makes the same asymmetry deliberately: an operator may
    // read a customer page because debugging a customer's problem requires it,
    // and the reverse is refused.
    const customer = navFor("customer");
    const operator = navFor("operator");

    expect(customer.some((r) => r.audience === "operator")).toBe(false);
    expect(operator.some((r) => r.audience === "operator")).toBe(true);
    expect(operator.length).toBeGreaterThan(customer.length);
  });

  it("keeps the token page out of the navigation", () => {
    // It needs a mint to mean anything, and a nav entry leading to an empty form
    // is a nav entry leading nowhere.
    for (const audience of ["customer", "operator"] as const) {
      expect(navFor(audience).some((r) => r.path.includes(":mint"))).toBe(false);
    }
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
