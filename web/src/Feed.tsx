// SPDX-License-Identifier: Apache-2.0
//! The decision record: what Radar decided, one token at a time, newest first.
//!
//! This is the primary surface, and it is the screen no competitor can build —
//! not because it is hard, but because it needs a reason list per decision and
//! nobody else records one. Every other terminal shows you what is going up.
//! This shows what Radar would not touch, and lets you check its work.
//!
//! # Why it is not ranked
//!
//! There is no sort control and no "top" anything. On this venue a momentum
//! ranking is a list of tokens whose curve is being bought out, and research
//! 0008 measured that the people buying it out were committed before the token
//! existed — the only role left for a later buyer is to be who they sell to. A
//! ranked list here would rank traps by how attractive the trap looks.
//!
//! So the ordering is time, and the *filter* is the reason. "Show me everything
//! refused for `CapacityBelowFloor`" is a question about Radar's rules; "show me
//! the top gainers" is a question this data answers wrongly.

import { useCallback, useEffect, useState } from "react";
import { Link, useSearch } from "wouter";

import { api, ApiError, type DecisionPage, type DecisionRecord } from "./api";
import { Address } from "./Figures";
import { partitionReasons } from "./honesty";
import {
  decisionsPath,
  NO_FILTERS,
  parseFilters,
  tokenPath,
  type Filters,
} from "./routes";

/** Micro-USD as dollars, for display. */
function usd(micro: number | null): string {
  if (micro === null) return "—";
  return `$${(micro / 1_000_000).toFixed(2)}`;
}

export function Feed() {
  const search = useSearch();
  const filters = parseFilters(search);

  const [page, setPage] = useState<DecisionPage | null>(null);
  const [rows, setRows] = useState<DecisionRecord[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loadingMore, setLoadingMore] = useState(false);

  // Refetch from the top whenever the filter changes. The dependency is the
  // parsed values rather than the object, which is rebuilt on every render.
  const { reason, conclusion } = filters;

  useEffect(() => {
    const controller = new AbortController();
    setPage(null);
    setRows([]);
    setError(null);
    api
      .decisions(
        { reason: reason ?? undefined, conclusion: conclusion ?? undefined },
        controller.signal,
      )
      .then((first) => {
        setPage(first);
        setRows(first.decisions);
      })
      .catch((e: unknown) => {
        if (controller.signal.aborted) return;
        setError(
          e instanceof ApiError && e.status === 503
            ? "The store has recorded nothing yet. That is a fresh instance, not a fault."
            : e instanceof ApiError
              ? e.detail
              : String(e),
        );
      });
    return () => controller.abort();
  }, [reason, conclusion]);

  const more = useCallback(() => {
    if (!page?.next || loadingMore) return;
    setLoadingMore(true);
    api
      .decisions({
        after: page.next,
        reason: reason ?? undefined,
        conclusion: conclusion ?? undefined,
      })
      .then((next) => {
        setPage(next);
        // Appended, not replaced. The cursor is `(slot, mint)` precisely so
        // these cannot overlap.
        setRows((seen) => [...seen, ...next.decisions]);
      })
      .catch((e: unknown) => {
        setError(e instanceof ApiError ? e.detail : String(e));
      })
      .finally(() => setLoadingMore(false));
  }, [page, reason, conclusion, loadingMore]);

  if (error) {
    return (
      <p className="rounded-md border border-[var(--color-line)] bg-[var(--color-surface)] px-4 py-3 text-sm text-[var(--color-warn)]">
        {error}
      </p>
    );
  }

  return (
    <section className="mt-8">
      <Heading filters={filters} matched={page?.matched ?? null} />

      {page === null && (
        <p className="text-sm text-[var(--color-dim)]">Reading the record…</p>
      )}

      {page !== null && rows.length === 0 && (
        <p className="rounded-md border border-[var(--color-line)] bg-[var(--color-surface)] px-4 py-3 text-sm">
          No decision matches that filter.{" "}
          <span className="text-[var(--color-dim)]">
            That is a fact about the filter, not about the store —{" "}
            <Link href={decisionsPath(NO_FILTERS)} className="underline">
              clear it
            </Link>{" "}
            to see the whole record.
          </span>
        </p>
      )}

      {rows.length > 0 && <Rows rows={rows} />}

      {page?.next && (
        <button
          type="button"
          onClick={more}
          disabled={loadingMore}
          className="mt-4 rounded-md border border-[var(--color-edge)] px-3 py-2 text-sm hover:border-[var(--color-dim)] disabled:opacity-50"
        >
          {loadingMore ? "Reading…" : "Show more"}
        </button>
      )}

      {page !== null && !page.next && rows.length > 0 && (
        <p className="mt-4 text-xs text-[var(--color-dim)]">
          That is the whole record at slot {page.as_of.toLocaleString()}.
        </p>
      )}
    </section>
  );
}

function Heading({
  filters,
  matched,
}: {
  filters: Filters;
  matched: number | null;
}) {
  const filtered = filters.reason !== null || filters.conclusion !== null;

  return (
    <div className="mb-3">
      <h2 className="text-sm font-medium uppercase tracking-wide text-[var(--color-dim)]">
        The decision record
      </h2>
      {filtered && (
        <p className="mt-2 flex flex-wrap items-baseline gap-2 text-sm">
          <span className="text-[var(--color-dim)]">Showing only</span>
          {filters.reason && (
            <span className="rounded border border-[var(--color-edge)] px-2 py-0.5 font-mono text-xs">
              {filters.reason}
            </span>
          )}
          {filters.conclusion && (
            <span className="rounded border border-[var(--color-edge)] px-2 py-0.5 font-mono text-xs">
              {filters.conclusion}
            </span>
          )}
          {/* The total, not the page. It is what says a reason accounts for four
              thousand refusals rather than the fifty on screen. */}
          {matched !== null && (
            <span className="tabular-nums text-[var(--color-dim)]">
              — {matched.toLocaleString()} in the record
            </span>
          )}
          <Link
            href={decisionsPath(NO_FILTERS)}
            className="text-[var(--color-warn)] underline"
          >
            clear
          </Link>
        </p>
      )}
    </div>
  );
}

function Rows({ rows }: { rows: readonly DecisionRecord[] }) {
  return (
    <div className="overflow-x-auto">
      <table className="w-full text-sm">
        <thead>
          <tr className="border-b border-[var(--color-line)] text-left text-xs uppercase tracking-wide text-[var(--color-dim)]">
            <th scope="col" className="pb-2 font-medium">
              token
            </th>
            <th scope="col" className="pb-2 font-medium">
              verdict
            </th>
            <th scope="col" className="pb-2 font-medium">
              why
            </th>
            <th scope="col" className="hidden pb-2 text-right font-medium sm:table-cell">
              size / capacity
            </th>
            <th scope="col" className="hidden pb-2 text-right font-medium md:table-cell">
              at slot
            </th>
          </tr>
        </thead>
        <tbody className="divide-y divide-[var(--color-line)]">
          {rows.map((d) => (
            // `(slot, mint)` and not either alone. A whole `radar consider`
            // batch shares one `decided_at`, and a mint recurs across runs, so
            // either on its own collapses rows React then refuses to update.
            <Row key={`${d.decided_at}:${d.mint}`} decision={d} />
          ))}
        </tbody>
      </table>
    </div>
  );
}

function Row({ decision }: { decision: DecisionRecord }) {
  const proposed = decision.conclusion === "proposed";
  const { structural, evidence, policy } = partitionReasons([
    ...decision.reasons,
    ...decision.kernel_reasons,
  ]);

  // Structural first, then evidence. Policy artefacts are deliberately last and
  // collapsed: under a closed policy every proposal carries seven of them, and
  // they say nothing about the token.
  const leading = [...structural, ...evidence];

  return (
    <tr>
      <td className="py-2">
        <Link href={tokenPath(decision.mint)} className="underline">
          <Address value={decision.mint} />
        </Link>
      </td>
      <td className="py-2">
        <span
          className={
            proposed ? "text-[var(--color-good)]" : "text-[var(--color-dim)]"
          }
        >
          {proposed ? "proposed" : "passed over"}
        </span>
      </td>
      <td className="py-2">
        {leading.length === 0 && policy.length === 0 && (
          <span className="text-[var(--color-absent)]">—</span>
        )}
        <span className="flex flex-wrap gap-1">
          {leading.slice(0, 2).map((reason) => (
            // Every reason is a link back into the record. The cheapest and most
            // valuable interaction here: "show me everything else refused for
            // this" is one click, and it is a question about Radar's rules
            // rather than about a price.
            <Link
              key={reason}
              href={decisionsPath({ reason, conclusion: null })}
              className="rounded border border-[var(--color-line)] px-1.5 py-0.5 font-mono text-xs hover:border-[var(--color-edge)]"
            >
              {reason}
            </Link>
          ))}
          {leading.length > 2 && (
            <span className="text-xs text-[var(--color-dim)]">
              +{leading.length - 2}
            </span>
          )}
          {leading.length === 0 && policy.length > 0 && (
            <span className="text-xs text-[var(--color-refuse)]">
              policy closed
            </span>
          )}
        </span>
      </td>
      <td className="hidden py-2 text-right tabular-nums sm:table-cell">
        {decision.notional_micro_usd === null ? (
          <span className="text-[var(--color-absent)]">—</span>
        ) : (
          <>
            {usd(decision.notional_micro_usd)}
            <span className="text-[var(--color-dim)]">
              {" / "}
              {usd(decision.exit_capacity_micro_usd)}
            </span>
          </>
        )}
      </td>
      <td className="hidden py-2 text-right tabular-nums text-[var(--color-dim)] md:table-cell">
        {decision.decided_at.toLocaleString()}
      </td>
    </tr>
  );
}
