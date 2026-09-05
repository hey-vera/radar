// SPDX-License-Identifier: Apache-2.0
//! The functions that decide what this site *claims*.
//!
//! Separated from the components that render them, for the reason `web`'s file
//! of the same name gives: a snapshot of a `<div>` fails when somebody renames a
//! class and passes when the page lies. Each function here has a wrong version
//! that looks right, which is the only reason to pull one out.
//!
//! This is a public marketing surface, which makes it the *most* important place
//! in the repository to get this right rather than the least. Every figure it
//! shows is a claim made to a stranger about somebody else's project.

/** The shape `/v1/public/stats` returns, and `fixtures/stats.json` holds. */
export interface Stats {
  /** When the figures were measured, ISO 8601. */
  readonly measured_at: string;
  /** The store watermark they were measured at. */
  readonly watermark_slot: number;
  readonly watched: {
    /** Succeeded launches recorded. */
    readonly launches: number;
    /** Distinct creators seen. */
    readonly creators: number;
    /** Of those launches, how many have had an outcome measured. */
    readonly measured: number;
    /** Measured tokens whose curve filled over time. */
    readonly organic: number;
    /** Measured tokens whose curve completed inside three slots. */
    readonly instant: number;
    /** Measured tokens that showed almost no life. */
    readonly stillborn: number;
  };
  readonly bands: {
    readonly measured_on: string;
    readonly launches: number;
    readonly base_rate_instant: number;
    readonly rows: readonly Band[];
  };
  readonly cost: { readonly band: string; readonly round_trip_bps: number };
  readonly aftermath: { readonly organic_median_bps: number };
}

/** One band of the launch-block recipient distribution. */
export interface Band {
  readonly name: string;
  readonly lo: number;
  readonly hi: number;
  /** Share of all launches whose block falls in this band. */
  readonly share_of_launches: number;
  /** Probability a launch in this band graduates instantly. */
  readonly p_instant: number;
  /** How many times the population rate that is. */
  readonly x_base_instant: number;
}

/**
 * A share of the **measured** population, or `null` if nothing was measured.
 *
 * Two decisions, and both have a wrong version that looks right.
 *
 * **The denominator is `measured`, never `launches`.** The gap between them is
 * Cabal Hunter's own outcome backlog. Dividing by `launches` folds that lag into
 * a claim about the venue and understates every rate by exactly the size of the
 * queue — and it would be invisible today, because the backlog is 0.4%.
 *
 * **Nothing measured yields `null`, not `0`.** "0% of launches graduate" read
 * off an empty denominator is a measurement of the outcome pass published as a
 * fact about pump.fun, and it is the direction that sounds authoritative. This
 * is rule 9 of `AGENTS.md`, in the interface.
 */
export function share(part: number, measured: number): number | null {
  if (measured <= 0) return null;
  return part / measured;
}

/** Every measured token that graduated, by either route, or `null`. */
export function graduated(w: Stats["watched"]): number | null {
  return share(w.organic + w.instant, w.measured);
}

/**
 * A share as a percentage, at the precision the measurement supports.
 *
 * Two decimals below 10%, one above. A rate of 2.81% loses its meaning rounded
 * to 3%, and a rate of 23.00% claims a precision the sample does not carry.
 *
 * `null` renders as a refusal, never as a number. A caller that wanted a dash
 * can have one; a caller that gets "0.00%" from a missing measurement cannot
 * tell.
 */
export function pct(value: number | null): string {
  if (value === null) return "not measured";
  const scaled = value * 100;
  return `${scaled < 10 ? scaled.toFixed(2) : scaled.toFixed(1)}%`;
}

/**
 * Basis points as a signed percentage.
 *
 * Zero is unsigned: `+0.0%` reads as a gain that rounded away, and nothing here
 * should flatter a number.
 */
export function bps(value: number): string {
  const sign = value > 0 ? "+" : "";
  return `${sign}${(value / 100).toFixed(1)}%`;
}

/**
 * A **cost** in basis points, as an unsigned percentage.
 *
 * Separate from [`bps`] and the separation is the point. `bps` signs a
 * **return**, where the sign carries the meaning. A round trip is not a return:
 * it is money that leaves whichever way the trade goes, and rendering 456 bps
 * of cost through `bps` produces `+4.6%` — a charge displayed as a gain.
 *
 * That shipped on the landing page and was caught by looking at it, not by a
 * test. It is the flattering direction, on the one figure the page uses to warn
 * people, so it is the worst place on the site for it to have happened.
 */
export function cost(value: number): string {
  return `${(Math.abs(value) / 100).toFixed(1)}%`;
}

/** A count with thousands separators, in the reader's own locale. */
export function count(value: number): string {
  return value.toLocaleString();
}

/**
 * How long ago a measurement was taken, in words.
 *
 * Every figure on this site is printed with one of these beside it. A number
 * with no date is the failure `0024` records in capitals: the note that measured
 * these quantities before it was wrong by 2.7× nine days later, and a reader who
 * cannot see the date cannot know to doubt it.
 *
 * Returns `null` for an unparseable or future timestamp rather than guessing.
 * A clock skew rendering "in 3 hours" would look like a bug in the data, which
 * is worse than saying nothing.
 */
export function measuredAgo(iso: string, now: Date = new Date()): string | null {
  const then = Date.parse(iso);
  if (Number.isNaN(then)) return null;
  const seconds = Math.floor((now.getTime() - then) / 1000);
  if (seconds < 0) return null;
  if (seconds < 90) return "just now";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 90) return `${minutes} minutes ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 36) return `${hours} hours ago`;
  const days = Math.floor(hours / 24);
  return `${days} days ago`;
}

/**
 * The band whose instant-graduation rate is the furthest above the base rate.
 *
 * The site's headline claim rests on this row, so it is **found** rather than
 * hard-coded: a snapshot in which some other band leads should change the
 * sentence, not leave it stating last month's winner with this month's date.
 *
 * `null` for an empty set, which the caller must render as a refusal.
 */
export function mostCoordinated(rows: readonly Band[]): Band | null {
  let best: Band | null = null;
  for (const row of rows) {
    if (best === null || row.x_base_instant > best.x_base_instant) best = row;
  }
  return best;
}
