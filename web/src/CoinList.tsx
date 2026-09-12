// SPDX-License-Identifier: Apache-2.0
//! The left rail: the live coin list. Dense, sortable, and the thing this
//! screen is built for people to stare at.
//!
//! Sorting is client-side against whatever page the server returned. The
//! contract's `sort=` query parameter is still sent as a hint -- a server
//! that returns pre-sorted pages saves a reflow on first paint -- but nothing
//! here trusts it: a reader who clicks "price" gets rows in price order
//! regardless of what the server did or did not honour.

import { useMemo } from "react";
import type { MarketCoin, MarketCoins, MarketSort } from "./api";
import { formatChangePct, formatCompactNumber, formatPrice } from "./format";
import { MarketFigure } from "./Figures";
import type { Load } from "./useApi";

export interface SortState {
  key: MarketSort;
  dir: "asc" | "desc";
}

const COLUMNS: { key: MarketSort; label: string }[] = [
  { key: "price", label: "Price" },
  { key: "change", label: "24h" },
  { key: "volume", label: "Vol" },
  { key: "txns", label: "Txns" },
];

/** Sorts a page of coins by one column, nulls last regardless of direction --
 *  an unmeasured figure is not "the smallest value", and sorting it to the top
 *  of an ascending column would put a row nobody can price above every row
 *  that was actually priced. */
function sortCoins(coins: readonly MarketCoin[], sort: SortState): MarketCoin[] {
  const pick = (c: MarketCoin): number | null => {
    switch (sort.key) {
      case "price":
        return c.price;
      case "change":
        return c.change_pct;
      case "volume":
        return c.quote_volume;
      case "txns":
        return c.tx_count;
    }
  };
  const sign = sort.dir === "asc" ? 1 : -1;
  return [...coins].sort((a, b) => {
    const av = pick(a);
    const bv = pick(b);
    if (av === null && bv === null) return 0;
    if (av === null) return 1;
    if (bv === null) return -1;
    return (av - bv) * sign;
  });
}

export function CoinList({
  load,
  sort,
  onSortChange,
  selectedMint,
  onSelect,
  listRef,
  filter,
}: {
  load: Load<MarketCoins>;
  sort: SortState;
  onSortChange: (key: MarketSort) => void;
  selectedMint: string | null;
  onSelect: (mint: string) => void;
  /** So the terminal can hand the sorted order to keyboard navigation without
   *  this component owning the sort state itself. */
  listRef?: (coins: MarketCoin[]) => void;
  /** The top bar's search box, applied here rather than at the fetch: it is a
   *  narrowing of a page already in memory, not a new question for the
   *  server. */
  filter?: string;
}) {
  const sorted = useMemo(() => {
    if (load.state !== "ready") return [];
    const needle = filter?.trim().toLowerCase();
    const matched = needle
      // Mint only. The coins endpoint sends no name or symbol -- resolving
      // metadata is one query per mint against a hundred-and-twenty-an-hour
      // budget -- so there is nothing else here to match on, and a filter box
      // that silently searched a field nobody sends would find nothing and
      // look broken.
      ? load.value.coins.filter((c) => c.mint.toLowerCase().includes(needle))
      : load.value.coins;
    const s = sortCoins(matched, sort);
    listRef?.(s);
    return s;
  }, [load, sort, listRef, filter]);

  return (
    <aside className="flex h-full flex-col border-r border-[var(--color-line)] bg-[var(--color-surface)]">
      <div className="border-b border-[var(--color-line)] px-2 py-1.5">
        <table className="w-full table-fixed text-[11px] uppercase tracking-wide text-[var(--color-dim)]">
          <thead>
            <tr>
              <th scope="col" className="w-[38%] pb-0 text-left font-medium">
                Coin
              </th>
              {COLUMNS.map((col) => (
                <SortHeader
                  key={col.key}
                  label={col.label}
                  active={sort.key === col.key}
                  dir={sort.dir}
                  onClick={() => onSortChange(col.key)}
                />
              ))}
            </tr>
          </thead>
        </table>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto">
        {load.state === "loading" && (
          <p className="p-3 text-xs text-[var(--color-dim)]">Reading the market…</p>
        )}

        {load.state === "failed" && (
          <p className="p-3 text-xs text-[var(--color-warn)]">
            Could not read the coin list: {load.detail}. This is a fact about
            the connection, not about the market.
          </p>
        )}

        {load.state === "ready" && sorted.length === 0 && (
          <p className="p-3 text-xs text-[var(--color-dim)]">
            {filter?.trim()
              ? `No coin matches "${filter.trim()}".`
              : "Radar is not tracking any coins right now."}
          </p>
        )}

        {load.state === "ready" && sorted.length > 0 && (
          <table className="w-full table-fixed text-xs">
            <tbody>
              {sorted.map((coin) => (
                <CoinRow
                  key={coin.mint}
                  coin={coin}
                  selected={coin.mint === selectedMint}
                  onSelect={onSelect}
                />
              ))}
            </tbody>
          </table>
        )}
      </div>
    </aside>
  );
}

function SortHeader({
  label,
  active,
  dir,
  onClick,
}: {
  label: string;
  active: boolean;
  dir: "asc" | "desc";
  onClick: () => void;
}) {
  return (
    <th scope="col" className="pb-0 text-right font-medium">
      <button
        type="button"
        onClick={onClick}
        className={`hover:text-[var(--color-text)] ${active ? "text-[var(--color-text)]" : ""}`}
        aria-pressed={active}
      >
        {label}
        {active ? (dir === "asc" ? " ▲" : " ▼") : ""}
      </button>
    </th>
  );
}

function CoinRow({
  coin,
  selected,
  onSelect,
}: {
  coin: MarketCoin;
  selected: boolean;
  onSelect: (mint: string) => void;
}) {
  return (
    <tr
      onClick={() => onSelect(coin.mint)}
      aria-current={selected ? "true" : undefined}
      className={`w-full cursor-pointer border-b border-[var(--color-line)] text-left align-middle ${
        selected ? "bg-[var(--color-ink)] outline outline-1 -outline-offset-1 outline-[var(--color-warn)]" : "hover:bg-[var(--color-ink)]"
      }`}
    >
      {/* The mint, abbreviated, because that is what this endpoint knows.
          A name and symbol need a metadata lookup per mint, which the query
          budget does not allow for a whole list -- so the row shows the
          identifier it has rather than a column of "unknown" where a name
          would go. The token header resolves the name for the selected coin. */}
      <td className="w-[38%] py-1.5 pl-2">
        <div className="truncate font-mono text-[11px] font-medium text-[var(--color-text)]">
          {coin.mint.slice(0, 4)}…{coin.mint.slice(-4)}
        </div>
        <div className="truncate text-[10px] text-[var(--color-dim)]">
          {coin.quote_mint === null
            ? "no priced fill in window"
            : `vs ${coin.quote_mint.slice(0, 4)}…`}
        </div>
      </td>
      <td className="py-1.5 text-right tabular-nums">
        <MarketFigure
          value={coin.price}
          reason={
            coin.price === null ? "no trade in this window carried both legs" : null
          }
          format={formatPrice}
        />
      </td>
      <td className="py-1.5 text-right tabular-nums">
        {coin.change_pct === null ? (
          <MarketFigure
            value={null}
            reason="fewer than two priced fills in this window"
            format={() => ""}
          />
        ) : (
          <span className={coin.change_pct >= 0 ? "text-[var(--color-gain)]" : "text-[var(--color-loss)]"}>
            {formatChangePct(coin.change_pct)}
          </span>
        )}
      </td>
      <td className="py-1.5 text-right tabular-nums text-[var(--color-dim)]">
        <MarketFigure
          value={coin.quote_volume}
          reason={coin.quote_volume === null ? "no priced fill in this window" : null}
          format={formatCompactNumber}
        />
      </td>
      {/* No age column. The coins endpoint does not carry a launch time and
          this screen will not compute one from a window it only partly sees.
          An age column of "unknown" is worse than no age column. */}
      <td className="py-1.5 pr-2 text-right tabular-nums text-[var(--color-dim)]">
        {formatCompactNumber(coin.tx_count)}
      </td>
    </tr>
  );
}
