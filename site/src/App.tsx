// SPDX-License-Identifier: Apache-2.0
//! The shell: the header, the footer, and which page is showing.

import { Link, Route, Switch, useLocation } from "wouter";

import { About } from "./About";
import { account, handleHref } from "./honesty";
import { Home } from "./Home";
import { Leaderboard } from "./Leaderboard";
import { Pool } from "./Pool";
import { nav } from "./routes";
import { Token } from "./Token";

function Header() {
  const [location] = useLocation();
  return (
    <header className="sticky top-0 z-30 border-b border-[var(--color-line)] bg-[var(--color-ink)]/85 backdrop-blur">
      <div className="mx-auto flex max-w-5xl items-center justify-between gap-3 px-4 py-3 sm:gap-4 sm:px-6 sm:py-4">
        <Link
          href="/"
          className="font-mono text-xs font-semibold tracking-widest whitespace-nowrap text-[var(--color-text)] uppercase sm:text-sm"
        >
          Cabal<span className="text-[var(--color-signal)]">Hunter</span>
        </Link>
        <nav className="flex items-center gap-0.5 text-xs sm:gap-1 sm:text-sm">
          {nav()
            .filter((r) => r.path !== "/")
            .map((r) => (
              <Link
                key={r.path}
                href={r.path}
                className={`rounded px-2 py-1.5 whitespace-nowrap transition-colors sm:px-3 ${
                  location === r.path
                    ? "bg-[var(--color-raised)] text-[var(--color-text)]"
                    : "text-[var(--color-dim)] hover:text-[var(--color-text)]"
                }`}
                // Read out by a screen reader, and it is the only thing telling
                // a non-sighted reader which page they are on -- the background
                // colour above says it to everybody else.
                aria-current={location === r.path ? "page" : undefined}
              >
                {r.short ?? r.label}
              </Link>
            ))}
        </nav>
      </div>
    </header>
  );
}

function Footer() {
  const handle = account();
  return (
    <footer className="relative z-10 border-t border-[var(--color-line)]">
      <div className="mx-auto max-w-5xl px-6 py-10 text-sm text-[var(--color-faint)]">
        <p className="max-w-2xl">
          Cabal Hunter is an automated account. It reports what it measured on
          chain and refuses the rest. <strong>Measured, not predicted.</strong>{" "}
          Nothing here is financial advice, and nothing here is a recommendation
          to buy or sell anything.
        </p>
        <p className="mt-4">
          <Link href="/about" className="underline hover:text-[var(--color-dim)]">
            What this is, who runs it, and what it will never say
          </Link>
        </p>
        {/* The two appointments the whole contest runs on. A reader who wants
            to enter needs to know when the week ends before they need anything
            else on this site. */}
        <p className="mt-4 text-xs">
          The week closes <strong>Mondays at 00:00 UTC</strong>. The account
          posts what it found seven days later, at 12:00 UTC.
        </p>
        {handle !== null && (
          <p className="mt-4 text-xs">
            <a
              href={handleHref(handle) ?? "#"}
              target="_blank"
              rel="noopener noreferrer"
              className="underline hover:text-[var(--color-dim)]"
            >
              @{handle} on X
            </a>
          </p>
        )}
      </div>
    </footer>
  );
}

export function App() {
  return (
    <div className="grid-ground flex min-h-screen flex-col">
      <Header />
      <main className="flex-1">
        <Switch>
          <Route path="/" component={Home} />
          <Route path="/leaderboard" component={Leaderboard} />
          <Route path="/pool" component={Pool} />
          <Route path="/token" component={Token} />
          <Route path="/about" component={About} />
          <Route>
            {/* Static hosting serves index.html for any path, so an unknown one
                reaches the router rather than the host. Said plainly: a blank
                page here would look like the site was broken. */}
            <div className="mx-auto max-w-5xl px-6 py-24">
              <h1 className="text-2xl font-semibold">No such page</h1>
              <p className="mt-3 text-[var(--color-dim)]">
                <Link href="/" className="underline">
                  Back to the start
                </Link>
              </p>
            </div>
          </Route>
        </Switch>
      </main>
      <Footer />
    </div>
  );
}
