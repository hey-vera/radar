// SPDX-License-Identifier: Apache-2.0
//! Reading the three public documents, and what happens when they are absent.
//!
//! # The fallback is a real answer, not a placeholder
//!
//! Every figure this site shows arrives with the moment it was measured, and
//! the committed fixture is a measurement too — taken on 2026-09-04 and dated
//! as such. So a failed fetch degrades to *older but true*, never to blank and
//! never to invented.
//!
//! That is the opposite of the usual pattern, where a fetch failure shows a
//! spinner forever or a zero. Both of those have shipped in this repository and
//! both are recorded: `Analyst.tsx` sat on "reading…" because an empty error
//! string is falsy, and rule 9 exists because a missing figure rendered as zero
//! is the default that loses money.

import fixture from "./fixtures/stats.json";
import type { Stats } from "./honesty";

/** Where the endpoints live. Same origin in the dev proxy; absolute in production. */
const BASE = import.meta.env["VITE_API_BASE"] ?? "";

/** How long to wait before falling back. */
const TIMEOUT_MS = 4000;

/** The committed measurement, used when the live one cannot be had. */
export const FALLBACK = fixture as unknown as Stats;

/** Where a figure on the page came from. */
export interface Sourced<T> {
  readonly value: T;
  /** `true` when this is the committed fixture rather than a live read. */
  readonly stale: boolean;
}

/**
 * One week's leaderboard.
 *
 * `entries` empty and `week` null is the honest state before the bot has run,
 * and it is **not** an error. The page must say so in words.
 */
export interface Leaderboard {
  /** ISO date of the week's Monday, or `null` when no week has run. */
  readonly week: string | null;
  readonly measured_at: string | null;
  readonly entries: readonly Entry[];
  /** Replies decided in the week, whether or not they were published. */
  readonly answered: number;
  /** Of those, how many actually reached the platform. */
  readonly published: number;
  /** The rule that decided this week, when the record carried one. */
  readonly rule?: ScoredRule | null;
  /** Present only when the operator voided the week. */
  readonly voided?: Voided | null;
}

/** One entry: somebody summoned the bot, and the bot's reply scored. */
export interface Entry {
  readonly rank: number;
  /**
   * The summoner's numeric X account id -- what a mention carries, and the
   * only identifier that cannot be reassigned. Link with `userHref`.
   */
  readonly summoner: string;
  /**
   * The handle, when the week close read one, and `null` otherwise.
   *
   * Mid-week nothing has read handles at all, so an open week is all `null`.
   * This field documented itself as `summoner` until 2026-09-06, and the site
   * therefore rendered `` at every reader -- finding S4.
   */
  readonly handle: string | null;
  /** The mint asked about, when one resolved. */
  readonly mint: string | null;
  /** The bot's reply on the platform, when it was published. */
  readonly reply_url: string | null;
  /** `null` when engagement has not been read yet — never `0`. */
  readonly score: number | null;
  /**
   * What the platform reported, before the scan.
   *
   * Published as evidence beside `verified`, never as the ranking: quotes and
   * replies are unlimited per account, so this is the number one account can
   * farm to the top for nothing. `null` mid-week -- engagement is read once,
   * at close.
   */
  readonly raw: RawMetrics | null;
  /**
   * What survived the scan, and what ranked the entry.
   *
   * `null` means the scan never reached this entry -- the walk stops as soon as
   * arithmetic says nothing below can win -- which is NOT the same as nobody
   * engaging. An entry with `null` here was scored on `raw`.
   */
  readonly verified: VerifiedMetrics | null;
}

/** Engagement as the platform counts it: actions, not accounts. */
export interface RawMetrics {
  readonly reposts: number;
  readonly quotes: number;
  readonly likes: number;
  readonly replies: number;
  readonly score: number;
}

/** Engagement as the rule counts it: distinct accounts old enough to count. */
export interface VerifiedMetrics {
  readonly reposts: number;
  /** Distinct accounts that quoted, however many times each. */
  readonly quoters: number;
  readonly likes: number;
  /** Distinct accounts seen across all three reads. */
  readonly engagers: number;
  /** How many of those were under the published age floor. A count, not a verdict. */
  readonly engagers_under_age: number;
}

/** The rule a closed week was scored under. `null` on weeks closed before it was recorded. */
export interface ScoredRule {
  readonly min_account_age_days: number;
  readonly min_engager_age_days: number;
  readonly cooldown_weeks: number;
}

/** The operator voided the week: it pays nobody, and this is why, verbatim. */
export interface Voided {
  readonly at: string;
  readonly reason: string;
}

/**
 * The prize pool.
 *
 * `vault` is `null` until a token exists. That is a different state from a
 * balance of zero and the page renders it differently: a pool reading
 * `0.00 SOL` looks like a contest nobody won.
 */
export interface Pool {
  /** The creator vault address, once there is one. */
  readonly vault: string | null;
  readonly lamports: number | null;
  readonly measured_at: string | null;
  readonly winners: readonly Winner[];
}

/** A past winner, and the transaction that paid them. */
export interface Winner {
  readonly week: string;
  /** The numeric X account id. Link with `userHref`. */
  readonly summoner: string;
  /** The handle read at week close, or `null`. */
  readonly handle: string | null;
  readonly lamports: number;
  readonly signature: string;
}

/**
 * One closed week, with everything a reader needs to check it.
 *
 * The page's whole job is that nothing on it has to be taken on trust: the
 * reply is a link, the payment is a signature, the claim is a link, and the
 * rule the week was scored under travels with the week rather than being
 * looked up. A field that is `null` is a fact the record does not carry, never
 * a zero and never the current value standing in for a past one.
 */
export interface Week {
  /** ISO date of the week's Monday. */
  readonly week: string;
  readonly opened_at: string;
  readonly closed_at: string;
  /** How many entries counted. */
  readonly entries: number;
  readonly excluded: Excluded;
  /** `null` when nothing counted at all. */
  readonly winner: WeekWinner | null;
  /**
   * The rule this week was scored under.
   *
   * `null` on a week closed before 2026-09-06, when nothing recorded it. That
   * is **unknown**, not "the current rule" -- the difference decides whether
   * somebody disputing a placing is looking at the rule that actually applied.
   */
  readonly rule: Rule | null;
  /** Set only when the operator voided the week, with the published reason. */
  readonly voided: Voided | null;
  readonly claim: Claim;
  readonly payout: Payout;
}

/** Entries that did not count, as counts by reason and never as names. */
export interface Excluded {
  readonly count: number;
  /** Reason key to how many. The page renders a sentence per key. */
  readonly reasons: Readonly<Record<string, number>>;
}

/** The winner, and the counts their score was built from. */
export interface WeekWinner {
  readonly summoner: string;
  readonly handle: string | null;
  readonly reply_url: string;
  readonly score: number;
  readonly mint: string | null;
  /** `null` when the week's scan never reached this entry — not "nobody engaged". */
  readonly verified: Verified | null;
}

/** Distinct accounts, old enough to count, behind a score. */
export interface Verified {
  readonly reposts: number;
  readonly quoters: number;
  readonly likes: number;
  readonly engagers: number;
  readonly engagers_under_age: number;
}

/** The published exclusions, as the week was actually scored. */
export interface Rule {
  /** How many accounts the operator declared. Ids are not published. */
  readonly operators: number;
  readonly min_account_age_days: number;
  readonly min_engager_age_days: number;
  readonly cooldown_weeks: number;
}

/** The operator cancelled the week, and why. */
export interface Voided {
  readonly at: string;
  readonly reason: string;
}

/**
 * Whether the winner collected.
 *
 * `open` is a deadline somebody can still meet; `rolled_over` is money that has
 * already moved to the next week. They are not the same fact and the page must
 * not render them the same way.
 */
export interface Claim {
  readonly state: "claimed" | "open" | "rolled_over" | "no_winner";
  readonly at?: string;
  readonly address?: string;
  readonly reply_url?: string;
  readonly closes_at?: string;
  readonly closed_at?: string;
}

/**
 * Whether the prize was paid, or the reason it was not.
 *
 * Five states, because "not paid" has four causes and they say very different
 * things about whoever runs this.
 */
export interface Payout {
  readonly state:
    "paid" | "owed" | "unclaimed" | "awaiting_claim" | "voided" | "no_winner";
  readonly lamports?: number;
  readonly recipient?: string;
  readonly signature?: string;
  readonly at?: string;
}

/** Every closed week, newest first. */
export interface Weeks {
  readonly measured_at: string | null;
  readonly weeks: readonly Week[];
}

/** Fetches JSON, or gives up quietly. */
async function get<T>(path: string): Promise<T | null> {
  try {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), TIMEOUT_MS);
    const response = await fetch(`${BASE}${path}`, {
      signal: controller.signal,
    });
    clearTimeout(timer);
    if (!response.ok) return null;
    return (await response.json()) as T;
  } catch {
    // Deliberately swallowed. Every caller has a truthful answer without this
    // request, and a public page must not render a stack trace at a stranger.
    // The distinction that matters -- live or committed -- is carried in
    // `Sourced.stale` and shown.
    return null;
  }
}

/** The population figures, live if possible and committed otherwise. */
export async function stats(): Promise<Sourced<Stats>> {
  const live = await get<Stats>("/v1/public/stats");
  return live
    ? { value: live, stale: false }
    : { value: FALLBACK, stale: true };
}

/**
 * This week's leaderboard.
 *
 * An unreachable endpoint and a week that has not run produce the **same**
 * empty result on purpose: from the reader's side both mean "there is nothing
 * to show", and inventing a distinction the page cannot act on would be
 * decoration. The page says nothing has run and why.
 */
export async function leaderboard(): Promise<Leaderboard> {
  const live = await get<Leaderboard>("/v1/public/leaderboard");
  return (
    live ?? {
      week: null,
      measured_at: null,
      entries: [],
      answered: 0,
      published: 0,
    }
  );
}

/** The prize pool, or the fact that there is not one yet. */
export async function pool(): Promise<Pool> {
  const live = await get<Pool>("/v1/public/pool");
  return (
    live ?? { vault: null, lamports: null, measured_at: null, winners: [] }
  );
}

/**
 * Every closed week.
 *
 * An empty list, like the leaderboard's, is the honest state before any week
 * has closed -- and an unreachable endpoint produces the same one, for the
 * reason `leaderboard` gives: from the reader's side both mean there is
 * nothing to show, and the page says which.
 */
export async function weeks(): Promise<Weeks> {
  const live = await get<Weeks>("/v1/public/weeks");
  return live ?? { measured_at: null, weeks: [] };
}

/** Lamports as SOL, at the precision a prize is worth quoting to. */
export function sol(lamports: number): string {
  return (lamports / 1_000_000_000).toFixed(4);
}
