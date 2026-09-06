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

/* ------------------------------------------------------------------------- *
 * Links.
 *
 * Every function below returns `string | null`, and `null` means "do not
 * render a link". That shape is the whole point: the alternative is a
 * component interpolating an API field straight into an `href`, and the
 * fields these are built from arrive from `/v1/public/*` — a document this
 * site does not write. A `javascript:` URL in a field the site trusts is one
 * stored value away from running in a reader's browser.
 *
 * `wouter` and React escape *text*, and neither escapes a URL scheme. React
 * warns on `javascript:` in newer versions and does not block it, and a warning
 * in somebody else's console is not a defence.
 *
 * These are also the reason the site never builds an `href` by template in a
 * component. If a link is not made here, it is not made.
 * ------------------------------------------------------------------------- */

/** The base58 alphabet, which excludes the confusable characters `0OIl`. */
const BASE58 = /^[1-9A-HJ-NP-Za-km-z]+$/;

/**
 * A URL that is safe to put in an `href`, or `null`.
 *
 * `https` only, and the host must be one the caller named. Parsing with `URL`
 * rather than matching a prefix is deliberate: `https://evil.example/#@x.com`
 * passes a `startsWith` check and is not x.com, and every hand-rolled version
 * of this function in the wild is a prefix check.
 */
export function safeHref(url: string, hosts: readonly string[]): string | null {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return null;
  }
  if (parsed.protocol !== "https:") return null;
  if (!hosts.includes(parsed.hostname)) return null;
  return parsed.toString();
}

/**
 * A link to an X account from its handle, or `null`.
 *
 * X's own rule: 1–15 characters, letters, digits and underscore. A 16th
 * character is not a handle, and rendering it as one produces a link to a
 * profile that does not exist — on the page that is supposed to be the account's
 * introduction.
 */
export function handleHref(handle: string): string | null {
  if (!/^[A-Za-z0-9_]{1,15}$/.test(handle)) return null;
  return `https://x.com/${handle}`;
}

/**
 * A link to an X account from its numeric id, or `null`.
 *
 * The leaderboard has ids and may not have handles: `close()` records the
 * summoner's id because that is what a mention carries, and the handle is a
 * second field that can be missing. The `i/user` form resolves without a
 * handle, exactly as `public.rs` uses `i/web/status` for a post.
 *
 * Digits only. An id is a number, and anything else in that position came from
 * somewhere it should not have.
 */
export function userHref(id: string): string | null {
  if (!/^[0-9]{1,25}$/.test(id)) return null;
  return `https://x.com/i/user/${id}`;
}

/**
 * A link to a transaction on Solscan, or `null`.
 *
 * Signatures are 64 bytes in base58, which is 87 or 88 characters. The bound is
 * exact rather than "long enough", for the reason `mention.rs` gives about
 * addresses: a run that is too long is not a signature, and truncating it to a
 * plausible length hands a reader a link to somebody else's transaction.
 */
export function solscanTx(signature: string): string | null {
  if (signature.length < 86 || signature.length > 88) return null;
  if (!BASE58.test(signature)) return null;
  return `https://solscan.io/tx/${signature}`;
}

/** A link to an account on Solscan, or `null`. Same rule as a mint. */
export function solscanAccount(address: string): string | null {
  if (!mintShaped(address)) return null;
  return `https://solscan.io/account/${address}`;
}

/**
 * Whether this is shaped like a Solana address.
 *
 * The same rule `mention.rs` applies to a summons, restated here because the
 * summon box has to decide *before* anything is sent whether the bot would read
 * what the reader pasted. 32 to 44 base58 characters, bounds exact.
 *
 * This is a shape check and not an existence check, and the interface must say
 * so: a well-formed address for a coin that does not exist is not caught here,
 * and the bot answers that case by refusing rather than by inventing a record.
 */
export function mintShaped(text: string): boolean {
  const t = text.trim();
  return t.length >= 32 && t.length <= 44 && BASE58.test(t);
}

/**
 * A prefilled X post that summons the account about a mint, or `null`.
 *
 * `null` when the handle is not configured or the text is not address-shaped —
 * a summon button that posts `@undefined` would be worse than no button. The
 * handle is a parameter rather than a constant here because this site does not
 * know it: see [`account`].
 */
export function summonIntent(handle: string, mint: string): string | null {
  if (handleHref(handle) === null) return null;
  if (!mintShaped(mint)) return null;
  const text = encodeURIComponent(`@${handle} ${mint.trim()}`);
  return `https://x.com/intent/post?text=${text}`;
}

/**
 * The account's handle, from the build environment, or `null`.
 *
 * **The site does not know this and must not guess it.** `deploy/README.md`
 * uses `CabalHunter` in a `curl` example for looking up the numeric id; no file
 * in this repository records the handle as a fact, the analyst identifies the
 * account by `RADAR_X_USER_ID` rather than by name, and on 2026-09-06 the
 * production analyst's own post log said `no publisher configured: this
 * instance cannot post` — so the account may not be posting under any handle
 * yet.
 *
 * A guessed handle on this page is a link sending strangers to somebody else's
 * profile, which is the one link on the site that cannot be walked back. So it
 * is operator configuration, it is validated on the way in, and everything that
 * needs it renders an honest alternative when it is absent. AGENTS.md rule 8:
 * deny by default when config is missing.
 */
export function account(): string | null {
  const configured = import.meta.env["VITE_X_HANDLE"];
  if (typeof configured !== "string") return null;
  const handle = configured.trim().replace(/^@/, "");
  return handleHref(handle) === null ? null : handle;
}
