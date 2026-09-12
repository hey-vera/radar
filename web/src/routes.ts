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
//!
//! # Why there is no `navFor` any more
//!
//! The decision-record pages -- `/evidence`, `/analyst`, `/instance`, and the
//! `/` decision feed -- were deleted along with the components that drew them:
//! the terminal replaces all four, and the owner's target for this pass is
//! narrower than any of them served -- find a coin, look at it, trade it,
//! track what you hold. None of that needs a navigation bar, and the terminal's
//! top bar is specified down to its three contents (wordmark, search, wallet),
//! none of which is a nav. `navFor` and the `inNav` flag it read existed only
//! for the nav `App.tsx` no longer renders, so they went with it -- a function
//! with no caller is not a design, it is a comment that compiles.
//!
//! The `audience` field on [`Route`] stays: it is still what `routes.test.ts`
//! cross-checks against `access::audience_of`, independent of whether anything
//! renders a menu from it.

/** Which audience a page belongs to. Mirrors `access::Audience`. */
export type Audience = "customer" | "operator";

/** One page of the interface. */
export interface Route {
  /** The path pattern, in wouter's syntax. */
  readonly path: string;
  /** Who may reach it. */
  readonly audience: Audience;
}

/**
 * The interface's pages, all of them.
 *
 * Three: the terminal, the token it can be pinned to, and the assistant --
 * kept because it is a distinct feature (a conversation) rather than a report,
 * and the owner's correction named only the report pages for deletion.
 */
export const ROUTES = [
  {
    path: "/",
    audience: "customer",
  },
  {
    path: "/ask",
    audience: "customer",
  },
  {
    // Reached from the coin list or the search box, never typed from nothing —
    // it needs a mint to mean anything.
    path: "/token/:mint",
    audience: "customer",
  },
] as const satisfies readonly Route[];

/** The path to one token's terminal view. */
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
