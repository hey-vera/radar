// SPDX-License-Identifier: Apache-2.0
//! The right rail: the selected coin's header, and the space reserved for
//! Radar's own signals.
//!
//! The signals panel is deliberately empty. The packet is explicit that this
//! is "later, not now" and that the placeholder must say what will go there,
//! never draw a fake chart to fill the space -- a confident wrong number is
//! the one failure this whole product exists to prevent, and an invented
//! signal would be exactly that.

import type { ReactNode } from "react";
import type { MarketToken } from "./api";
import { MarketFigure } from "./Figures";
import {formatCompactUsd, formatPrice} from "./format";
import type { Load } from "./useApi";

export function TokenHeader({ load }: { load: Load<MarketToken> }) {
  return (
    <aside className="flex h-full flex-col border-l border-[var(--color-line)] bg-[var(--color-surface)]">
      <div className="border-b border-[var(--color-line)] p-3">
        <Header load={load} />
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto p-3">
        <SignalsPlaceholder />
      </div>
    </aside>
  );
}

function Header({ load }: { load: Load<MarketToken> }) {
  if (load.state === "loading") {
    return <p className="text-xs text-[var(--color-dim)]">Reading token…</p>;
  }
  if (load.state === "failed") {
    return (
      <p className="text-xs text-[var(--color-warn)]">
        Could not read this token: {load.detail}.
      </p>
    );
  }

  const token = load.value;

  return (
    <div>
      <div className="flex items-baseline justify-between gap-2">
        <h1 className="truncate text-lg font-semibold">
          {token.symbol ?? <span className="text-[var(--color-absent)]">unknown</span>}
        </h1>
        {/* First seen, not age. The endpoint reports when `solana.tokens`
            first carried the mint, which is an indexing date and not a launch
            time -- so the header says "first seen" rather than converting it
            into an age the server never claimed. */}
        <span className="text-xs text-[var(--color-dim)]" title="When this mint first appeared in the chain index. Not necessarily its launch.">
          {token.published_at === null
            ? "first seen unknown"
            : `first seen ${token.published_at.slice(0, 10)}`}
        </span>
      </div>
      <p className="truncate text-xs text-[var(--color-dim)]">
        {token.name ?? "name unknown"}
      </p>

      <p className="mt-3 text-2xl font-semibold tabular-nums">
        <MarketFigure value={token.price} reason={token.price_reason} format={formatPrice} />
      </p>

      <dl className="mt-3 grid grid-cols-2 gap-x-3 gap-y-2 text-xs">
        <Field label="Market cap">
          <MarketFigure value={token.market_cap} reason={token.market_cap_reason} format={formatCompactUsd} />
        </Field>
        <Field label="Liquidity">
          <MarketFigure value={token.liquidity} reason={token.liquidity_reason} format={formatCompactUsd} />
        </Field>
        <Field label="24h change">
          {/* The contract does not put a 24h change on the token header --
              only the coin list carries `change_pct`. Shown here only when a
              caller has it; nothing here invents one to fill the cell. */}
          <span className="text-[var(--color-absent)]" title="Not part of this endpoint's contract">
            see coin list
          </span>
        </Field>
        <Field label="Decimals">
          <span className="text-[var(--color-absent)]" title={token.decimals_reason ?? undefined}>
            per trade
          </span>
        </Field>
      </dl>
    </div>
  );
}

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div>
      <dt className="text-[10px] uppercase tracking-wide text-[var(--color-dim)]">{label}</dt>
      <dd className="tabular-nums">{children}</dd>
    </div>
  );
}

function SignalsPlaceholder() {
  return (
    <div className="rounded-md border border-dashed border-[var(--color-edge)] p-4 text-xs text-[var(--color-dim)]">
      <p className="font-medium text-[var(--color-text)]">Radar&rsquo;s signals</p>
      <p className="mt-2">
        Not live yet. When they are, this panel will show what Radar decided
        about this token and why — the same reason list the decision record
        always carried, not a second price chart.
      </p>
      <p className="mt-2">
        This space is reserved rather than filled, because a confident wrong
        number here is worse than an honest blank one.
      </p>
    </div>
  );
}
