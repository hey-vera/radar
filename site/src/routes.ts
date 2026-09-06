// SPDX-License-Identifier: Apache-2.0
//! The site's pages.
//!
//! A flat list rather than the console's audience-classified table, because
//! every page here is public by construction: this application is served as
//! static files from a host that has no idea who is reading. There is nothing to
//! classify and nothing that could leak.
//!
//! That is the whole argument for it being a separate application. In `web/`,
//! a page added to the routes and forgotten in `access::audience_of` is not a
//! 404 -- it is a page that silently requires operator identity. Here there is
//! no such seam to get wrong.

/** One page. */
export interface Route {
  readonly path: string;
  readonly label: string;
  /**
   * What the header calls it, when that differs.
   *
   * "Prize pool" wrapped onto two lines at 375px and made the sticky header
   * eat a third of the screen — on the width most of this traffic arrives at,
   * since the whole distribution is a link in a reply on a phone. The page's
   * own heading still says "The prize pool", so nothing is lost.
   */
  readonly short?: string;
  /** Whether it appears in the header. */
  readonly inNav: boolean;
}

export const ROUTES = [
  { path: "/", label: "Home", inNav: true },
  { path: "/leaderboard", label: "Leaderboard", inNav: true },
  { path: "/pool", label: "Prize pool", short: "Pool", inNav: true },
  { path: "/history", label: "Past weeks", short: "History", inNav: true },
  { path: "/token", label: "Tokenomics", short: "Token", inNav: true },
  { path: "/about", label: "About", inNav: true },
] as const satisfies readonly Route[];

/** The pages the header shows. */
export function nav(): readonly Route[] {
  return ROUTES.filter((r) => r.inNav);
}
