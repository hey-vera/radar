// SPDX-License-Identifier: Apache-2.0
//! The shell: which page is showing, and how you get to another one.
//!
//! # Why there is a router now
//!
//! There was a `useState` here, and a comment saying a router goes in "when the
//! first screen needs a URL that means something, not before". Three do.
//!
//! A token page is the obvious one — a decision about a mint is the thing anyone
//! would send to somebody else, and it was unlinkable. The other two are less
//! obvious and matter more: with one state variable, a refresh silently returned
//! the reader to the funnel, and the back button left the application entirely.
//! Both read as the page losing your place.
//!
//! `wouter` rather than TanStack Router: 2 kB against 45. Its three transitive
//! dependencies are `mitt`, `regexparam` and `use-sync-external-store`, all
//! tiny. The plan for this work said "zero dependencies", which was wrong —
//! checked, and corrected here rather than left to be found later.
//!
//! # The seam this leaves
//!
//! `AUDIENCE` below is a constant, and that is the honest shape of what the
//! interface currently knows: nothing authenticates a customer, Cloudflare
//! Access gates the whole site, so every reader really is an operator. A lookup
//! would be a guess wearing the shape of a fact. It is the single place that
//! changes when the customer lane switches on.

import { Link, Route, Switch, useRoute } from "wouter";

import { Agent } from "./Agent";
import { Decisions } from "./Decisions";
import { Health } from "./Health";
import { Scoreboard } from "./Scoreboard";
import { Token, TokenLookup } from "./Token";
import { navFor, type Audience } from "./routes";

/**
 * Who the interface believes it is talking to.
 *
 * A constant, deliberately. Cloudflare Access gates the whole site to one
 * operator and no customer authenticator is configured, so `"operator"` is not
 * an assumption — it is the only thing a reader can currently be.
 *
 * When `RADAR_PRIVY_APP_ID` is set this becomes an answer derived from the
 * verified session, and `Instance` stops appearing for customers.
 */
const AUDIENCE: Audience = "operator";

export function App() {
  return (
    <div className="mx-auto max-w-6xl px-6 py-10">
      <header className="mb-6">
        <h1 className="text-2xl font-semibold tracking-tight">
          <Link href="/" className="hover:text-[var(--color-dim)]">
            Radar
          </Link>
        </h1>
        <p className="mt-1 text-sm text-[var(--color-dim)]">
          Solana research intelligence. A record of what was refused, and why.
        </p>
      </header>

      <Nav />

      <main>
        <Switch>
          <Route path="/" component={Decisions} />
          <Route path="/evidence" component={Scoreboard} />
          <Route path="/ask">
            <Agent alwaysShow />
          </Route>
          <Route path="/instance" component={Health} />
          <Route path="/token/:mint" component={TokenPage} />
          <Route>
            <NotFound />
          </Route>
        </Switch>
      </main>
    </div>
  );
}

function Nav() {
  const pages = navFor(AUDIENCE);
  return (
    <nav className="mb-8 flex flex-wrap gap-1 border-b border-[var(--color-line)]">
      {pages.map((page) => (
        <NavLink key={page.path} href={page.path} label={page.label} />
      ))}
    </nav>
  );
}

function NavLink({ href, label }: { href: string; label: string }) {
  const [active] = useRoute(href);
  return (
    <Link
      href={href}
      // A real anchor, not a button. It was a `<button>` carrying
      // `aria-current`, which is correct markup for something that is not a link
      // and wrong for something that is: a reader could not open it in a new
      // tab, copy its address, or see where it led before clicking.
      aria-current={active ? "page" : undefined}
      className={`-mb-px border-b-2 px-3 py-2 text-sm ${
        active
          ? "border-[var(--color-good)] text-[var(--color-text)]"
          : "border-transparent text-[var(--color-dim)] hover:text-[var(--color-text)]"
      }`}
    >
      {label}
    </Link>
  );
}

/// One token's evidence, plus the box that got you here.
///
/// The lookup stays on the page rather than being replaced by its result: the
/// common action after reading one token is looking up another, and a form that
/// vanishes on submit turns that into a navigation.
function TokenPage({ params }: { params: { mint: string } }) {
  const mint = decodeURIComponent(params.mint);
  return (
    <div className="space-y-6">
      <TokenLookup initial={mint} />
      <Token mint={mint} />
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
          Back to the decision record
        </Link>
        .
      </p>
    </div>
  );
}
