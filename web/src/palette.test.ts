// SPDX-License-Identifier: Apache-2.0
//! The palette's accessibility claims, measured against the real stylesheet.
//!
//! Every assertion here reads `index.css` rather than a copy of its numbers, so
//! changing a token without changing what it may be used for fails.
//!
//! The reason this file exists at all: an earlier pass wrote that the border
//! contrast was "roughly 2:1, under the 3:1 UI components need". It was
//! **1.53:1**. The estimate was wrong by enough to matter and nothing would have
//! caught it, because a ratio in a comment is a claim and this repository's rule
//! is that a claim should be backed by something that runs.

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { contrast, luminance, parseOklchTokens, toLinearRgb } from "./palette";
import type { Oklch } from "./palette";

// The real stylesheet, off disk.
//
// Not `import "./index.css?raw"`: vitest stubs CSS modules, so that import
// resolves to an empty string and every assertion below would hold vacuously —
// which is precisely the failure `finds the tokens rather than passing on an
// empty map` exists to catch, and it caught it.
const stylesheet = readFileSync(resolve(__dirname, "index.css"), "utf8");

const tokens: Map<string, Oklch> = parseOklchTokens(stylesheet);

/** A token that must exist. Missing is a failure, never a silent skip. */
function token(name: string): Oklch {
  const value = tokens.get(name);
  expect(
    value,
    `${name} is not declared as a bare oklch() in index.css`,
  ).toBeDefined();
  return value as Oklch;
}

describe("the stylesheet parses at all", () => {
  it("finds the tokens rather than passing on an empty map", () => {
    // Without this every assertion below would hold vacuously the moment the
    // regex stopped matching -- which is the way a check like this dies.
    expect(tokens.size).toBeGreaterThanOrEqual(10);
    expect(tokens.has("--color-ink")).toBe(true);
  });
});

describe("text is legible on its ground", () => {
  it.each([
    ["--color-text", 4.5],
    ["--color-dim", 4.5],
    ["--color-absent", 4.5],
    ["--color-gain", 4.5],
    ["--color-loss", 4.5],
    ["--color-warn", 4.5],
    ["--color-refuse", 4.5],
    ["--color-good", 4.5],
  ])("%s clears %s:1 against ink", (name, min) => {
    expect(contrast(token(name), token("--color-ink"))).toBeGreaterThanOrEqual(min);
  });

  it("holds on the raised surface too, not only on the page ground", () => {
    // Cards and tables sit on `surface`, which is lighter than `ink`. A colour
    // checked only against the darker ground can fail on the one it is actually
    // drawn on, and every figure in this interface is inside a card.
    const surface = token("--color-surface");
    for (const name of ["--color-text", "--color-dim", "--color-gain", "--color-loss"]) {
      expect(contrast(token(name), surface), `${name} on surface`).toBeGreaterThanOrEqual(4.5);
    }
  });
});

describe("gain and loss survive colour blindness", () => {
  it("separates them on lightness, not only on hue", () => {
    // The failure this exists for. `good` and `refuse` -- the pair previously
    // used for figures -- sit at 1.36:1 against each other, so in greyscale or
    // to a reader who cannot separate the hues they are the same colour. The
    // number encoded this way is the most important one on the screen.
    const separation = contrast(token("--color-gain"), token("--color-loss"));
    expect(separation).toBeGreaterThanOrEqual(1.8);
  });

  it("keeps them apart from absent, so a null cannot read as either", () => {
    // Rule 9's colour. "Not measured" must not be mistakeable for a small gain
    // or a small loss.
    const absent = token("--color-absent");
    expect(contrast(token("--color-gain"), absent)).toBeGreaterThanOrEqual(1.5);
    expect(contrast(token("--color-loss"), absent)).toBeGreaterThanOrEqual(1.1);
  });

  it("uses green and amber rather than green and red", () => {
    // Hue, asserted rather than assumed: red-green is the pairing 8% of men
    // cannot separate, and it is the one every other trading interface reaches
    // for. Amber sits well away from the green.
    const gain = token("--color-gain");
    const loss = token("--color-loss");
    expect(gain.h).toBeGreaterThan(120);
    expect(gain.h).toBeLessThan(190);
    // Amber/orange, not red. Red is nearer 25-30 degrees in OKLCH.
    expect(loss.h).toBeGreaterThanOrEqual(40);
    expect(loss.h).toBeLessThan(95);
  });
});

describe("borders say what they are for", () => {
  it("gives perceivable component edges their own token at 3:1", () => {
    // WCAG 1.4.11. Anything a person has to *find* -- an input, a button --
    // needs this; a table's hairline does not.
    expect(contrast(token("--color-edge"), token("--color-ink"))).toBeGreaterThanOrEqual(3);
  });

  it("keeps the decorative hairline decorative, and below the edge token", () => {
    // Not an oversight, and the assertion is what says so. Raising every rule in
    // a dense table to 3:1 makes a page of glaring lines; the split is the
    // point, so `line` must stay quieter than `edge`.
    const line = contrast(token("--color-line"), token("--color-ink"));
    const edge = contrast(token("--color-edge"), token("--color-ink"));
    expect(line).toBeLessThan(edge);
    expect(line).toBeLessThan(3);
  });
});

describe("the arithmetic itself", () => {
  it("agrees with WCAG at the two ends it defines", () => {
    // Black on white is 21:1 and a colour on itself is 1:1. Without these the
    // whole file could be measuring something else consistently.
    const white: Oklch = { l: 1, c: 0, h: 0 };
    const black: Oklch = { l: 0, c: 0, h: 0 };
    expect(contrast(white, black)).toBeCloseTo(21, 1);
    expect(contrast(white, white)).toBeCloseTo(1, 5);
  });

  it("is symmetric in its arguments", () => {
    const a: Oklch = { l: 0.9, c: 0.1, h: 200 };
    const b: Oklch = { l: 0.2, c: 0.05, h: 30 };
    expect(contrast(a, b)).toBeCloseTo(contrast(b, a), 10);
  });

  it("clamps out-of-gamut colours instead of returning negative light", () => {
    // A vivid hue at high chroma leaves sRGB. Unclamped, a negative channel
    // makes luminance smaller than black's and every ratio built on it wrong.
    const vivid = toLinearRgb({ l: 0.5, c: 0.4, h: 150 });
    expect(Math.min(...vivid)).toBeGreaterThanOrEqual(0);
    expect(Math.max(...vivid)).toBeLessThanOrEqual(1);
    expect(luminance(vivid)).toBeGreaterThanOrEqual(0);
  });

  it("skips a token it cannot read rather than guessing at it", () => {
    // `undefined` sends a caller to a failing assertion with the token's name in
    // it. A guess would put a wrong number into a passing test.
    const parsed = parseOklchTokens(
      "--a: oklch(0.5 0.1 200); --b: var(--a); --c: #ff0000;",
    );
    expect(parsed.get("--a")).toEqual({ l: 0.5, c: 0.1, h: 200 });
    expect(parsed.get("--b")).toBeUndefined();
    expect(parsed.get("--c")).toBeUndefined();
  });
});
