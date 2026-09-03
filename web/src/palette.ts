// SPDX-License-Identifier: Apache-2.0
//! Colour arithmetic, so the palette's claims can be checked rather than
//! believed.
//!
//! `index.css` writes a contrast ratio in a comment beside every token. A
//! comment is not a measurement, and this repository has the entry for that: an
//! earlier pass of this work guessed the border contrast at "roughly 2:1" and it
//! was **1.53:1** — under half the threshold it was being compared against.
//!
//! So the ratios are computed here from the values in the stylesheet itself, and
//! `palette.test.ts` asserts them. Changing a token without changing what it is
//! allowed to be used for now fails a test.
//!
//! # Why this is not a colour library
//!
//! One conversion and one formula, both short and both specified. A dependency
//! for this would be a package in the bundle that gets embedded in
//! `radar-serve`, and the bar the workspace sets is "the alternative is writing
//! it ourselves and getting it wrong". Twenty lines of published matrix
//! arithmetic is not that.

/** A colour as OKLCH: lightness 0–1, chroma, hue in degrees. */
export interface Oklch {
  l: number;
  c: number;
  h: number;
}

/** A colour as linear-light sRGB, each channel 0–1. */
export type LinearRgb = readonly [number, number, number];

/**
 * OKLCH to linear-light sRGB, clamped to the gamut.
 *
 * The matrices are Björn Ottosson's published OKLab definition. Clamping rather
 * than erroring on out-of-gamut: a browser clamps too, so the number this
 * produces is what a reader would actually see.
 */
export function toLinearRgb({ l, c, h }: Oklch): LinearRgb {
  const rad = (h * Math.PI) / 180;
  const a = c * Math.cos(rad);
  const b = c * Math.sin(rad);

  const lCube = (l + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const mCube = (l - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const sCube = (l - 0.0894841775 * a - 1.291485548 * b) ** 3;

  const clamp = (v: number) => Math.min(1, Math.max(0, v));
  return [
    clamp(4.0767416621 * lCube - 3.3077115913 * mCube + 0.2309699292 * sCube),
    clamp(-1.2684380046 * lCube + 2.6097574011 * mCube - 0.3413193965 * sCube),
    clamp(-0.0041960863 * lCube - 0.7034186147 * mCube + 1.707614701 * sCube),
  ];
}

/** WCAG relative luminance. */
export function luminance(rgb: LinearRgb): number {
  return 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
}

/**
 * The WCAG contrast ratio between two colours, from 1 to 21.
 *
 * Symmetric: the order of the arguments does not change the answer, which is
 * why the brighter of the two is picked rather than assumed to be the first.
 */
export function contrast(a: Oklch, b: Oklch): number {
  const la = luminance(toLinearRgb(a));
  const lb = luminance(toLinearRgb(b));
  const [hi, lo] = la > lb ? [la, lb] : [lb, la];
  return (hi + 0.05) / (lo + 0.05);
}

/**
 * The `oklch(...)` values declared in a stylesheet, by custom-property name.
 *
 * Deliberately reads the real file rather than duplicating the numbers in
 * TypeScript. A second copy of a palette is a second thing to keep in step, and
 * the copy is always the one that goes stale — which is the failure this whole
 * module exists to catch.
 *
 * Returns only properties whose value is a bare `oklch(l c h)`. A token defined
 * as anything else is skipped rather than guessed at, so a caller asking for one
 * gets `undefined` and a test fails loudly.
 */
export function parseOklchTokens(css: string): Map<string, Oklch> {
  const found = new Map<string, Oklch>();
  const pattern =
    /(--[\w-]+)\s*:\s*oklch\(\s*([\d.]+)\s+([\d.]+)\s+([\d.]+)\s*\)/g;
  for (const match of css.matchAll(pattern)) {
    const [, name, l, c, h] = match;
    if (!name || !l || !c || !h) continue;
    found.set(name, { l: Number(l), c: Number(c), h: Number(h) });
  }
  return found;
}
