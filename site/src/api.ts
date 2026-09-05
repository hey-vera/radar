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
}

/** One entry: somebody summoned the bot, and the bot's reply scored. */
export interface Entry {
  readonly rank: number;
  /** The X handle that summoned it. */
  readonly summoner: string;
  /** The mint asked about, when one resolved. */
  readonly mint: string | null;
  /** The bot's reply on the platform, when it was published. */
  readonly reply_url: string | null;
  /** `null` when engagement has not been read yet — never `0`. */
  readonly score: number | null;
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
  readonly summoner: string;
  readonly lamports: number;
  readonly signature: string;
}

/** Fetches JSON, or gives up quietly. */
async function get<T>(path: string): Promise<T | null> {
  try {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), TIMEOUT_MS);
    const response = await fetch(`${BASE}${path}`, { signal: controller.signal });
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
  return live ? { value: live, stale: false } : { value: FALLBACK, stale: true };
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
    live ?? { week: null, measured_at: null, entries: [], answered: 0, published: 0 }
  );
}

/** The prize pool, or the fact that there is not one yet. */
export async function pool(): Promise<Pool> {
  const live = await get<Pool>("/v1/public/pool");
  return live ?? { vault: null, lamports: null, measured_at: null, winners: [] };
}

/** Lamports as SOL, at the precision a prize is worth quoting to. */
export function sol(lamports: number): string {
  return (lamports / 1_000_000_000).toFixed(4);
}
