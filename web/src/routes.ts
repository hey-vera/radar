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
