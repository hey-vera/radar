// SPDX-License-Identifier: Apache-2.0
//! Where the interface's pages live, and who may reach them.
//!
//! # Why the paths are a constant rather than strings at call sites
//!
//! Every path here is also classified in `access::audience_of` on the server,
//! and the server's fallback for an unclassified path is `Audience::Operator`.
//! So a page added here and forgotten there is not a 404 — it is a page that
//! silently requires operator identity, which today is invisible because
//! everything requires operator identity anyway. It becomes visible on the day
//! the customer lane switches on, for every customer at once.
//!
//! One table, and `routes.test.ts` asserts it matches the audiences the server
//! declares. That test is the only thing standing between the two lists.

/** Which audience a page belongs to. Mirrors `access::Audience`. */
export type Audience = "customer" | "operator";

/** One page of the interface. */
export interface Route {
  /** The path pattern, in wouter's syntax. */
  readonly path: string;
  /** What the navigation calls it. */
  readonly label: string;
  /** Who may reach it. */
  readonly audience: Audience;
  /** Whether it appears in the navigation, or is only reached by a link. */
  readonly inNav: boolean;
}

/**
 * The interface's pages.
 *
 * Order is the navigation order, and it is the order the product argues for
 * itself in: what was decided, then the evidence behind the decisions, then the
 * assistant, then the instance.
 */
export const ROUTES = [
  {
    path: "/",
    label: "Decisions",
    audience: "customer",
    inNav: true,
  },
  {
    path: "/evidence",
    label: "Evidence",
    audience: "customer",
    inNav: true,
  },
  {
    path: "/ask",
    label: "Ask",
    audience: "customer",
    inNav: true,
  },
  {
    path: "/instance",
    label: "Instance",
    audience: "operator",
    inNav: true,
  },
  {
    // Reached from a decision row or from the lookup box, never from the nav —
    // it needs a mint to mean anything, and a nav entry that leads to an empty
    // form is a nav entry that leads nowhere.
    path: "/token/:mint",
    label: "Token",
    audience: "customer",
    inNav: false,
  },
] as const satisfies readonly Route[];

/** The pages that appear in the navigation, for a given audience. */
export function navFor(audience: Audience): readonly Route[] {
  // An operator sees everything; a customer sees only customer pages. The
  // asymmetry is `access::Audience`'s and it is deliberate there: an operator
  // may read a customer page because debugging a customer's problem requires
  // it, and the reverse is refused.
  return ROUTES.filter(
    (r) => r.inNav && (audience === "operator" || r.audience === "customer"),
  );
}

/** The path to one token's evidence. */
export function tokenPath(mint: string): string {
  return `/token/${encodeURIComponent(mint)}`;
}

/**
 * The shortest and longest a base58 Solana address can be.
 *
 * A 32-byte key is 43 or 44 base58 characters; addresses with leading zero
 * bytes are shorter. The server uses the same bounds in `evidence::addresses_in`
 * and for the same reason: the point is to find addresses, not to match every
 * long word.
 */
const ADDRESS_LENGTH = { min: 32, max: 44 } as const;

/** Base58 excludes the four characters that look like each other. */
const BASE58 = /^[1-9A-HJ-NP-Za-km-z]+$/;

/**
 * Whether a string could be a mint address.
 *
 * Syntactic only, and it must stay that way: this decides whether to *ask* the
 * server, never whether an answer is trustworthy. A caller that treated a `true`
 * here as "this token exists" would be inventing a fact.
 *
 * Its job is to tell a typo from a lookup that found nothing, which are
 * different things a reader needs told differently — "that is not an address"
 * versus "Radar never observed this token".
 */
export function isMintLike(value: string): boolean {
  const trimmed = value.trim();
  return (
    trimmed.length >= ADDRESS_LENGTH.min &&
    trimmed.length <= ADDRESS_LENGTH.max &&
    BASE58.test(trimmed)
  );
}

/** What the decision record is being filtered to. */
export interface Filters {
  /** Only decisions carrying this reason. */
  reason: string | null;
  /** Only proposals, or only tokens passed over. */
  conclusion: "proposed" | "passed" | null;
}

/** Nothing filtered. */
export const NO_FILTERS: Filters = { reason: null, conclusion: null };

/**
 * Reads the filters out of a URL query string.
 *
 * Pure, and separated from the screen for the reason `honesty.ts` gives about
 * everything else in this interface: it has a wrong version that looks right.
 * An unrecognised `conclusion` silently treated as `"proposed"` would show a
 * reader a filtered record while the control said otherwise — a page lying about
 * what it is showing, which is this repository's whole subject.
 *
 * So an unrecognised value is **dropped**, not coerced. The filter it names does
 * not apply, the reader sees the unfiltered record, and nothing claims a filter
 * that is not in force.
 *
 * An empty `reason` is dropped for the same reason: `?reason=` is not a request
 * for decisions whose reason is the empty string.
 */
export function parseFilters(search: string): Filters {
  const params = new URLSearchParams(
    search.startsWith("?") ? search.slice(1) : search,
  );

  const reason = params.get("reason")?.trim();
  const conclusion = params.get("conclusion")?.trim();

  return {
    reason: reason ? reason : null,
    conclusion:
      conclusion === "proposed" || conclusion === "passed" ? conclusion : null,
  };
}

/**
 * The address of the decision record under a set of filters.
 *
 * The inverse of [`parseFilters`], and `routes.test.ts` holds them to being
 * exactly that. A pair that drifts apart produces a link nobody can follow back.
 */
export function decisionsPath(filters: Filters): string {
  const params = new URLSearchParams();
  if (filters.reason) params.set("reason", filters.reason);
  if (filters.conclusion) params.set("conclusion", filters.conclusion);
  const search = params.toString();
  return search ? `/?${search}` : "/";
}
