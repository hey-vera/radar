// SPDX-License-Identifier: Apache-2.0
//! The small set of functions that decide what the interface *claims*.
//!
//! Separated from the components that render them, because these are the part
//! worth testing and a component is not. A snapshot of a `<div>` fails when
//! somebody renames a class and passes when the page lies; these each have a
//! wrong version that looks right, which is the only reason to pull them out.

/// Basis points as a signed percentage, at one decimal place.
///
/// Zero is unsigned. `+0.0%` reads as a gain that rounded away, and the whole
/// point of this screen is to not flatter the numbers.
export function pct(bps: number): string {
  const sign = bps > 0 ? "+" : "";
  return `${sign}${(bps / 100).toFixed(1)}%`;
}

/// The median of a return distribution, or `null` if there is nothing to take
/// one of.
///
/// **Sorted numerically**, which is not the default: JavaScript's `sort` is
/// lexicographic, so `[10, 9, 100]` becomes `[10, 100, 9]`. On basis points that
/// is not a rounding error, it is a different token.
///
/// `null` rather than zero for an empty cohort. Rule 9 applies to the interface
/// too — a cohort with nothing in it has no median, and rendering that as 0%
/// prints "broke even" for a measurement nobody took.
export function median(returns: number[]): number | null {
  if (returns.length === 0) return null;
  const sorted = [...returns].sort((a, b) => a - b);
  const mid = Math.floor(sorted.length / 2);
  if (sorted.length % 2 === 1) return sorted[mid] ?? null;
  // Rounded, not fractional. A basis point is already the smallest unit anyone
  // measured, and `-13.405%` implies a precision that does not exist.
  return Math.round(((sorted[mid - 1] ?? 0) + (sorted[mid] ?? 0)) / 2);
}

/// A gross return with the round trip taken off.
///
/// One subtraction, named and tested, because the screen it serves spent its
/// whole life claiming to have done it and had not. `Cohort::returns_bps` is
/// documented `Gross`; the scoreboard rendered that median under a footnote
/// reading "Returns are net of an assumed 850 bps round trip". The most-read
/// number on the page overstated itself by the entire round trip, in the
/// flattering direction, and no test could catch it because the arithmetic
/// existed nowhere.
///
/// It stays a function rather than an inline `a - b` for exactly that reason: a
/// claim the interface makes about a number should be somewhere a test can
/// reach.
///
/// **Signed, and deliberately not clamped.** A return that does not cover its
/// costs is negative, and flooring it at zero would turn every losing trade
/// into a break-even one.
export function netOfCost(grossBps: number, costBps: number): number {
  return grossBps - costBps;
}

/// How many of a distribution cleared the assumed round-trip cost.
///
/// **Strictly above.** A round trip that returned exactly its cost cleared
/// nothing, and counting it overstates the headline figure at precisely the
/// boundary that figure exists to describe.
export function clearedCost(returns: number[], costBps: number): number {
  return returns.filter((r) => r > costBps).length;
}

/// Refusals that are consequences of the policy being shut, not findings about
/// a token.
///
/// Under `Policy::CLOSED` every limit is zero, so `0 >= 0` is true at zero
/// realised loss, a staleness ceiling of zero fails every input, and a cost
/// ceiling of zero fails any cost. One fact — the policy is closed — arrives as
/// seven, and rendering them individually tells a novice there are seven
/// problems with a token.
///
/// The membership matters in **both** directions, and the second is the one that
/// matters more: a finding wrongly listed here would be collapsed into "policy
/// closed" and hidden, and findings are the only refusals that say anything
/// about the token being looked at.
export const POLICY_ARTIFACTS: ReadonlySet<string> = new Set([
  "NoAutonomy",
  "OverPositionLimit",
  "OverDeploymentLimit",
  "OverCreatorLimit",
  "DailyLossReached",
  "RoundTripTooExpensive",
  "InputsTooStale",
]);

/// Refusals that are facts about the token, and will not change with more data.
///
/// Mirrors `PassReason::is_structural` in `radar-strategy`: no amount of waiting
/// makes a freezable token unfreezable, or gives a route to a token that has
/// none. Everything else the strategy says is a fact about the **evidence**, and
/// might read differently tomorrow.
///
/// The distinction exists in the kernel and the interface threw it away. It is
/// the difference between "Radar will never touch this" and "Radar could not
/// tell yet", and a reader deciding whether to trust a refusal needs to know
/// which one they are looking at.
export const STRUCTURAL_REASONS: ReadonlySet<string> = new Set([
  "ExitCanBeStopped",
  "NoRoute",
  "ExitUnmeasurable",
]);

/// A refusal list split into the three kinds it actually contains.
export interface Reasons {
  /// Facts about the token. Permanent.
  structural: string[];
  /// Facts about the evidence. May change.
  evidence: string[];
  /// Consequences of the policy being shut. Not about the token at all.
  policy: string[];
}

/// Splits a refusal list three ways.
///
/// Order within each group is preserved, because the strategy emits them
/// worst-first and re-sorting would discard that.
///
/// The policy group is checked **first**. Under `Policy::CLOSED` every limit is
/// zero and seven refusals fire at once, and a reader shown seven items believes
/// there are seven problems with the token. None of them is about the token.
///
/// A reason in neither set is `evidence`, which is the safe default in the sense
/// that matters here: an unrecognised reason is *shown* rather than collapsed
/// into "policy closed" and hidden. Findings are the only refusals that say
/// anything about the token being looked at, and losing one is the expensive
/// direction.
export function partitionReasons(reasons: readonly string[]): Reasons {
  const out: Reasons = { structural: [], evidence: [], policy: [] };
  for (const reason of reasons) {
    if (POLICY_ARTIFACTS.has(reason)) out.policy.push(reason);
    else if (STRUCTURAL_REASONS.has(reason)) out.structural.push(reason);
    else out.evidence.push(reason);
  }
  return out;
}
