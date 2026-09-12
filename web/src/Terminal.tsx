// SPDX-License-Identifier: Apache-2.0
//! The trading terminal: the whole free public tier, on one screen.
//!
//! Full viewport height, no page scroll -- each panel scrolls on its own.
//! Roughly a five-region layout: a top bar, a left rail of coins, a centre
//! column of chart-over-tabs, and a right rail with the selected token's
//! header and Radar's (not yet built) signals.
//!
//! # Selection lives in the URL
//!
//! `mint` is `undefined` at `/` and a real address at `/token/:mint` — the
//! packet's requirement that a coin be linkable. Landing on `/` with a list
//! but no pinned mint promotes the first coin once the list loads, so the
//! address bar always ends up naming what is on screen without a reader
//! having to click anything first.

import { useCallback, useEffect, useRef, useState } from "react";
import { useLocation } from "wouter";
import { market, type MarketCoin, type MarketSort, type MarketToken } from "./api";
import { CandleChart } from "./CandleChart";
import { CoinList, type SortState } from "./CoinList";
import { HoldersPanel } from "./HoldersPanel";
import { InfoPanel } from "./InfoPanel";
import { isMintLike, tokenPath } from "./routes";
import { TokenHeader } from "./TokenHeader";
import { TradeTape } from "./TradeTape";
import { Wallet } from "./Wallet";
import { useApi } from "./useApi";

const COIN_LIST_LIMIT = 100;
/** How often the coin list re-fetches. There is no push feed for the market
 *  surface (`/v1/customer/events` carries the decision-record watermark, not
 *  a market one), so "live" here means polled rather than pushed. */
const REFRESH_MS = 15_000;

type Tab = "trades" | "holders" | "info";

export function Terminal({ mint }: { mint?: string }) {
  const [, navigate] = useLocation();
  const [sort, setSort] = useState<SortState>({ key: "volume", dir: "desc" });
  const [query, setQuery] = useState("");
  const [tab, setTab] = useState<Tab>("trades");
  const [tick, setTick] = useState(0);
  const searchRef = useRef<HTMLInputElement | null>(null);
  const orderedRef = useRef<MarketCoin[]>([]);

  useEffect(() => {
    const id = window.setInterval(() => setTick((t) => t + 1), REFRESH_MS);
    return () => window.clearInterval(id);
  }, []);

  const sortHint = SORT_TO_QUERY[sort.key];
  const coinsLoad = useApi(
    (signal) => market.coins({ limit: COIN_LIST_LIMIT, sort: sortHint }, signal),
    [sortHint, tick],
  );

  // No mint pinned in the URL: once the list is in, promote its first row so
  // the address bar names what is actually on screen. `replace` so landing on
  // `/` does not leave a back-button trail of every list that ever loaded.
  useEffect(() => {
    if (mint) return;
    if (coinsLoad.state !== "ready") return;
    const first = orderedRef.current[0] ?? coinsLoad.value.coins[0];
    if (first) navigate(tokenPath(first.mint), { replace: true });
  }, [mint, coinsLoad, navigate]);

  const select = useCallback(
    (next: string) => navigate(tokenPath(next), { replace: true }),
    [navigate],
  );

  // Arrow keys move the selection; `/` focuses the search box -- both only
  // when focus is not already inside a text field, so a reader typing a mint
  // into the search box does not have every keystroke stolen.
  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      const target = e.target as HTMLElement | null;
      const typing =
        target?.tagName === "INPUT" ||
        target?.tagName === "TEXTAREA" ||
        target?.isContentEditable;

      if (e.key === "/" && !typing) {
        e.preventDefault();
        searchRef.current?.focus();
        return;
      }

      if (typing) return;
      if (e.key !== "ArrowUp" && e.key !== "ArrowDown") return;

      const list = orderedRef.current;
      if (list.length === 0) return;
      const index = list.findIndex((c) => c.mint === mint);
      const delta = e.key === "ArrowDown" ? 1 : -1;
      const nextIndex = index === -1 ? 0 : Math.min(Math.max(index + delta, 0), list.length - 1);
      const next = list[nextIndex];
      if (next && next.mint !== mint) {
        e.preventDefault();
        select(next.mint);
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [mint, select]);

  // No request is made until there is a mint to ask about -- an empty-string
  // request would just be a guaranteed 404, and the "nothing selected yet"
  // screens below never read `tokenLoad` while `mint` is undefined anyway.
  const tokenLoad = useApi<MarketToken>(
    (signal) => (mint ? market.token(mint, signal) : new Promise(() => {})),
    [mint],
  );

  const submitSearch = useCallback(
    (e: React.FormEvent) => {
      e.preventDefault();
      const trimmed = query.trim();
      if (isMintLike(trimmed)) select(trimmed);
    },
    [query, select],
  );

  return (
    <div className="flex h-screen min-w-[1024px] flex-col overflow-x-auto bg-[var(--color-ink)] text-[var(--color-text)]">
      <TopBar
        query={query}
        onQueryChange={setQuery}
        onSubmit={submitSearch}
        searchRef={searchRef}
      />

      <div className="grid min-h-0 flex-1 grid-cols-[300px_1fr_320px]">
        <CoinList
          load={coinsLoad}
          sort={sort}
          onSortChange={(key) => setSort((s) => ({ key, dir: s.key === key && s.dir === "desc" ? "asc" : "desc" }))}
          selectedMint={mint ?? null}
          onSelect={select}
          filter={query}
          listRef={(coins) => {
            orderedRef.current = coins;
          }}
        />

        <div className="flex min-h-0 min-w-0 flex-col border-r border-[var(--color-line)]">
          <div className="min-h-0 flex-[3]">
            {mint ? (
              <CandleChart mint={mint} />
            ) : (
              <EmptyCentre text="Pick a coin from the list to see its chart." />
            )}
          </div>
          <div className="min-h-0 flex-[2] border-t border-[var(--color-line)]">
            {mint ? (
              <div className="flex h-full flex-col">
                <TabBar tab={tab} onChange={setTab} />
                <div className="min-h-0 flex-1">
                  {tab === "trades" && <TradeTape mint={mint} />}
                  {tab === "holders" && <HoldersPanel mint={mint} />}
                  {tab === "info" && (
                    <InfoPanel load={tokenLoad} />
                  )}
                </div>
              </div>
            ) : (
              <EmptyCentre text="Nothing selected yet." />
            )}
          </div>
        </div>

        {mint ? (
          <TokenHeader load={tokenLoad} />
        ) : (
          <aside className="border-l border-[var(--color-line)] bg-[var(--color-surface)] p-3 text-xs text-[var(--color-dim)]">
            Select a coin to see its details.
          </aside>
        )}
      </div>
    </div>
  );
}

const SORT_TO_QUERY: Record<SortState["key"], MarketSort> = {
  price: "price",
  change: "change",
  volume: "volume",
  txns: "txns",
};

function TopBar({
  query,
  onQueryChange,
  onSubmit,
  searchRef,
}: {
  query: string;
  onQueryChange: (v: string) => void;
  onSubmit: (e: React.FormEvent) => void;
  searchRef: React.RefObject<HTMLInputElement | null>;
}) {
  return (
    <header className="flex h-12 shrink-0 items-center gap-4 border-b border-[var(--color-line)] px-3">
      <a href="/" className="text-sm font-semibold tracking-tight hover:text-[var(--color-dim)]">
        Radar
      </a>

      <form onSubmit={onSubmit} className="flex-1">
        <label htmlFor="terminal-search" className="sr-only">
          Search by mint or symbol
        </label>
        <input
          id="terminal-search"
          ref={searchRef}
          value={query}
          onChange={(e) => onQueryChange(e.target.value)}
          placeholder="Search a symbol, or paste a mint  (press / to focus)"
          spellCheck={false}
          autoComplete="off"
          className="w-full max-w-md rounded-md border border-[var(--color-edge)] bg-[var(--color-surface)] px-3 py-1.5 text-sm outline-none focus:border-[var(--color-dim)]"
        />
      </form>

      <Wallet />
    </header>
  );
}

function TabBar({ tab, onChange }: { tab: Tab; onChange: (tab: Tab) => void }) {
  const tabs: { key: Tab; label: string }[] = [
    { key: "trades", label: "Trades" },
    { key: "holders", label: "Holders" },
    { key: "info", label: "Info" },
  ];
  return (
    <div className="flex shrink-0 border-b border-[var(--color-line)]">
      {tabs.map((t) => (
        <button
          key={t.key}
          type="button"
          onClick={() => onChange(t.key)}
          aria-current={tab === t.key ? "page" : undefined}
          className={`-mb-px border-b-2 px-3 py-1.5 text-xs ${
            tab === t.key
              ? "border-[var(--color-warn)] text-[var(--color-text)]"
              : "border-transparent text-[var(--color-dim)] hover:text-[var(--color-text)]"
          }`}
        >
          {t.label}
        </button>
      ))}
    </div>
  );
}

function EmptyCentre({ text }: { text: string }) {
  return (
    <div className="flex h-full items-center justify-center text-sm text-[var(--color-dim)]">
      {text}
    </div>
  );
}
