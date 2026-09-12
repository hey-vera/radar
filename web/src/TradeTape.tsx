// SPDX-License-Identifier: Apache-2.0
//! The live tape: every trade, newest first, buys and sells colour-separated.
//!
//! The two empty states this screen can show for the same zero rows are the
//! reason `emptyTapeMessage` exists in `honesty.ts` — "this coin never
//! traded" and "Radar could not reach the trade feed" are different facts,
//! and collapsing them into one blank table is the exact failure the packet
//! calls out by name.

import { market, type Trade } from "./api";
import { capCaption, emptyTapeMessage } from "./honesty";
import { Address, MarketFigure, Side } from "./Figures";
import {explorerUrl, formatCompactNumber, formatPrice, formatStampTime} from "./format";
import { useApi } from "./useApi";

const LIMIT = 60;

export function TradeTape({ mint }: { mint: string }) {
  const load = useApi((signal) => market.trades(mint, { limit: LIMIT }, signal), [mint]);

  if (load.state === "loading") {
    return <p className="p-3 text-xs text-[var(--color-dim)]">Reading the tape…</p>;
  }

  if (load.state === "failed") {
    return (
      <p className="p-3 text-xs text-[var(--color-warn)]">
        {emptyTapeMessage("unreachable", load.detail)}
      </p>
    );
  }

  const { trades } = load.value;

  if (trades.length === 0) {
    return <p className="p-3 text-xs text-[var(--color-dim)]">{emptyTapeMessage("never-traded")}</p>;
  }

  const cap = capCaption(trades.length, LIMIT, "trades");

  return (
    <div className="flex h-full flex-col">
      <div className="min-h-0 flex-1 overflow-y-auto">
        <table className="w-full text-xs">
          <thead className="sticky top-0 bg-[var(--color-surface)] text-[10px] uppercase tracking-wide text-[var(--color-dim)]">
            <tr>
              <th scope="col" className="py-1 pl-3 text-left font-medium">Time</th>
              <th scope="col" className="py-1 text-left font-medium">Side</th>
              <th scope="col" className="py-1 text-right font-medium">Price</th>
              <th scope="col" className="py-1 text-right font-medium">Amount</th>
              <th scope="col" className="py-1 pr-3 text-right font-medium">Trader</th>
            </tr>
          </thead>
          <tbody>
            {trades.map((trade) => (
              <TapeRow key={trade.signature} trade={trade} />
            ))}
          </tbody>
        </table>
      </div>
      {cap && (
        <p className="border-t border-[var(--color-line)] px-3 py-1 text-[10px] text-[var(--color-dim)]">
          {cap}
        </p>
      )}
    </div>
  );
}

function TapeRow({ trade }: { trade: Trade }) {
  return (
    <tr className="border-b border-[var(--color-line)] hover:bg-[var(--color-ink)]">
      <td className="py-1 pl-3 tabular-nums text-[var(--color-dim)]">
        {formatStampTime(trade.ts)}
      </td>
      <td className="py-1">
        <Side side={trade.side} />
      </td>
      <td className="py-1 text-right tabular-nums">
        <MarketFigure value={trade.price} reason={trade.price === null ? "no route priced at fill time" : null} format={formatPrice} />
      </td>
      <td className="py-1 text-right tabular-nums text-[var(--color-dim)]">
        {formatCompactNumber(trade.token_amount)}
      </td>
      <td className="py-1 pr-3 text-right">
        {/* A button (`Address`, which copies) and a link cannot nest -- both
            are interactive and a `<button>` inside an `<a>` is invalid HTML,
            silently swallowing one of the two actions in most browsers. They
            sit side by side instead: copy the address, or open it. */}
        <span className="inline-flex items-center gap-1">
          <Address value={trade.trader} />
          <a
            href={explorerUrl(trade.trader)}
            target="_blank"
            rel="noreferrer"
            title="Open in explorer"
            className="text-[var(--color-dim)] hover:text-[var(--color-text)]"
          >
            ↗
          </a>
        </span>
      </td>
    </tr>
  );
}
