// SPDX-License-Identifier: Apache-2.0
//! The tab's title, per page.
//!
//! A single-page application keeps the title in `index.html` unless something
//! changes it, so every route showed "see the cabal before you buy" — including
//! the leaderboard and the pool.
//!
//! That matters more here than on most sites. This product's whole distribution
//! is people sharing links, and a bookmark, a tab strip and a browser history
//! entry all read this string. Three pages that cannot be told apart in a tab
//! strip is a small failure repeated by every reader.
//!
//! The site's name goes **last**. A tab is truncated from the right, so
//! "Prize pool · Cabal Hunter" keeps the useful half when six tabs are open and
//! "Cabal Hunter · Prize pool" does not.

import { useEffect } from "react";

/** The name every title ends with. */
const SITE = "Cabal Hunter";

/** Sets the document title for as long as the calling page is mounted. */
export function useTitle(page: string | null): void {
  useEffect(() => {
    const previous = document.title;
    document.title = page === null ? SITE : `${page} · ${SITE}`;
    // Restored on unmount so a page cannot leave its title behind on the next
    // one -- which is what happens when two routes both set it and one forgets.
    return () => {
      document.title = previous;
    };
  }, [page]);
}
