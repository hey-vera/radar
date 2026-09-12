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

// --- the market terminal's honesty rules -----------------------------------
//
// Everything below backs a claim the terminal makes about a live market
// rather than about a recorded decision, and each one has a wrong version
// that looks right -- the reason this file exists at all.

/**
 * The words for an empty trade tape.
 *
 * "No trades yet" and "could not read the trade feed" are different facts
 * about the world, and a single empty-state message would print the wrong one
 * half the time. A coin that has never traded and a data source that is
 * unreachable both render zero rows; only the words say which happened.
 *
 * `detail` is the transport's own error text, folded in rather than replaced,
 * because "unreachable" alone sends a reader nowhere and the detail is
 * usually the whole diagnosis -- the same reasoning `ApiError` already
 * carries for the rest of the interface.
 */
export function emptyTapeMessage(
  reason: "never-traded" | "unreachable",
  detail?: string,
): string {
  if (reason === "never-traded") {
    return "No trades recorded for this mint. That is a fact about the token, not about the connection.";
  }
  return `Could not read the trade feed${detail ? `: ${detail}` : ""}. This says nothing about whether the token has traded -- Radar could not look.`;
}

/**
 * Whether a page of rows might be a cropped view of a longer list, inferred
 * rather than asserted.
 *
 * No route in this contract reports a row-cap flag, so this compares what
 * came back against what was asked for: a full page **might** be the whole
 * list or might be the first slice of a longer one, and there is no way to
 * tell them apart without asking for one more row than the limit. Treating a
 * full page as "possibly more" is the safe direction to be wrong in -- the
 * failure this exists to prevent is the opposite one, a capped response
 * presented as a complete window.
 *
 * `returned` greater than `limit` is a defensive branch: it should not
 * happen, and if it does, it is certainly not evidence the list is complete.
 */
export function isPossiblyCapped(returned: number, limit: number): boolean {
  return returned >= limit;
}

/**
 * The caption for a possibly-capped list, or `null` when the whole of it
 * came back.
 */
export function capCaption(
  returned: number,
  limit: number,
  noun: string,
): string | null {
  if (!isPossiblyCapped(returned, limit)) return null;
  return `Showing the most recent ${limit} ${noun}. There may be more; this is not the whole history.`;
}

/**
 * The holders panel's caption: what kind of fact the list is.
 *
 * The contract requires every holders response to carry this, and the rule
 * says it "must not be dropped for being ugly" -- so a response that omits it
 * does not fall back to silence. It falls back to a caption of its own, one
 * that says the methodology is unknown rather than pretending there is
 * nothing to say. A blank caption and a stated one both make a
 * methodology claim; the difference is whether it is true.
 */
/**
 * The holders panel's caption, built from what the server actually states.
 *
 * **Three fields, not one sentence.** `/v1/market/holders/{mint}` reports
 * `fact` (what the list was computed from), `granularity` (what one row
 * counts) and the window it folded over. Each carries a different warning and
 * collapsing them loses one:
 *
 * - `fact: "folded_transfers"` means balances were summed from transfer
 *   history, which is **not** a read of current token-account state. They
 *   agree only if the fold saw every transfer.
 * - `granularity: "token_account"` means a row is an account, not a person.
 *   One owner holding the same mint in three accounts appears three times, so
 *   the row count **overstates** the number of holders. Printing it as "827
 *   holders" would be a crowd size nobody measured.
 * - The window is the one that separates two sentences a reader must not
 *   confuse: *this coin has no holders*, and *this coin is older than this
 *   fold reaches*.
 *
 * An unrecognised `fact` or `granularity` is passed through rather than
 * dropped or silently normalised -- a server that starts reporting something
 * new should make the caption read oddly, not read reassuringly.
 */
export function holdersBasis(
  fact: string,
  granularity: string,
  from: string,
  to: string,
): string {
  const source =
    fact === "folded_transfers"
      ? "Folded from transfer history, not read from current account balances"
      : `Computed as "${fact}"`;
  const unit =
    granularity === "token_account"
      ? "one row is a token account, not a person, so this over-counts holders"
      : `one row is ${granularity}`;
  return `${source} — ${unit}. Covers ${from} to ${to}; anything before that is outside this fold.`;
}

export function holdersBasisCaption(basis: string | null | undefined): string {
  const trimmed = basis?.trim();
  return trimmed
    ? trimmed
    : "Radar does not know how this holder list was computed. Treat the ranking as unverified.";
}

/**
 * Whether the range a candle response actually covers is narrower than what
 * was requested.
 *
 * "May be narrower than asked for" is the contract's own words, and a chart
 * that just draws whatever came back without saying so implies it drew the
 * whole of what was requested. A young token given a 1d/4h request and
 * answered with three days of candles is not a data error; the caption this
 * drives says so instead of leaving a reader to wonder why the chart looks
 * short.
 */
export function isNarrowerThanRequested(
  requestedFrom: number,
  requestedTo: number,
  coveredFrom: number,
  coveredTo: number,
): boolean {
  return coveredFrom > requestedFrom || coveredTo < requestedTo;
}
