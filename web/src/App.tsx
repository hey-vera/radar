// SPDX-License-Identifier: Apache-2.0
//! The shell: which page is showing, and how you get to another one.
//!
//! # What this replaced
//!
//! Until this pass, `/` was a narrow, centred document -- a decision feed,
//! an evidence scoreboard, and an assistant, at `max-w-6xl`. The owner's
//! words: "basically like a pumpfun or axiom trading view", and a document is
//! not that. [`Terminal`](./Terminal.tsx) is a full-bleed trading screen and
//! is now `/` and `/token/:mint` both; the three pages it replaced --
//! `Decisions.tsx`, `Scoreboard.tsx`, `Health.tsx`, `Analyst.tsx`, `Token.tsx`
//! -- are deleted, not hidden, along with the components that existed only to
//! draw them (`Feed.tsx`, `Activity.tsx`, `CapacityWall.tsx`,
//! `ReturnDistribution.tsx`, `CostCurve.tsx`, `PricePath.tsx`). See this
//! session's commits for which pieces of `Figures.tsx` earned a place in the
//! terminal instead of going with the rest.
//!
//! `/ask` survives unchanged: it is a conversation, not a report, and the
//! owner's correction named only the report pages for deletion.
//!
//! # Why there is no navigation bar
//!
//! There were three customer pages before this pass, and a nav made sense.
//! There are effectively one now -- the terminal -- plus an assistant reached
//! by URL, so a nav would be one real link dressed up as a menu. The
//! terminal's own top bar is specified down to its contents (wordmark,
//! search, wallet), and none of them is a nav.

import { Link, Route, Switch } from "wouter";

import { Wallet } from "./Wallet";

import { Agent } from "./Agent";
import { Terminal } from "./Terminal";

export function App() {
  return (
    <Switch>
      <Route path="/">
        <Terminal />
      </Route>
      <Route path="/token/:mint">
        {(params) => <Terminal mint={decodeURIComponent(params.mint)} />}
      </Route>
      <Route path="/ask">
        <SimpleShell>
          <Agent alwaysShow />
        </SimpleShell>
      </Route>
      <Route>
        <SimpleShell>
          <NotFound />
        </SimpleShell>
      </Route>
    </Switch>
  );
}

/**
 * The plain document chrome the terminal does not use.
 *
 * Only `/ask` and the 404 page render through this now. It is the old app
 * shell's header, kept for the one page still built as a document rather
 * than a screen -- not a design worth extending, just not worth rebuilding
 * for two routes that are not this pass's subject.
 */
function SimpleShell({ children }: { children: React.ReactNode }) {
  return (
    <div className="mx-auto max-w-6xl px-6 py-10">
      <header className="mb-6 flex items-start justify-between gap-4">
        <h1 className="text-2xl font-semibold tracking-tight">
          <Link href="/" className="hover:text-[var(--color-dim)]">
            Radar
          </Link>
        </h1>
        <Wallet />
      </header>
      <main>{children}</main>
    </div>
  );
}

function NotFound() {
  return (
    <div className="rounded-md border border-[var(--color-line)] bg-[var(--color-surface)] px-4 py-3 text-sm">
      <p>
        <strong className="text-[var(--color-warn)]">No such page.</strong> That
        is a fact about this address, not about the store.
      </p>
      <p className="mt-2 text-[var(--color-dim)]">
        <Link href="/" className="underline">
          Back to the terminal
        </Link>
        .
      </p>
    </div>
  );
}
