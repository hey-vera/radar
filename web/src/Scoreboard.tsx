// SPDX-License-Identifier: Apache-2.0
//! The honest scoreboard.
//!
//! The screen most likely to be read as a performance claim, so it is built to
//! resist that. Two rules govern it:
//!
//! **The comparison is Radar's own refusals, not a constant.** An earlier
//! version of this compared the selected cohort to research 0009's population
//! median, and those are different quantities — 0009 enters at the token's first
//! fill and Radar enters forty minutes later — while the constant itself moved
//! from −1,340 to −863 bps as the cohort grew. Refusals are priced in the same
//! passes, the same way, over the same universe.
//!
//! **"Not enough data" is the headline until it is not.** Below the cohort floor
//! the page says so in as many words rather than showing a number with a caveat
//! beneath it, because a number on a screen is read and a caveat is not.

import { useEffect, useState } from "react";
import { ApiError, research, type Scoreboard as Board } from "./api";
import { clearedCost, median, pct } from "./honesty";

/** Below this many scored proposals, no comparison is reported at all. */
const MIN_COHORT = 30;

export function Scoreboard() {
  const [board, setBoard] = useState<Board | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const controller = new AbortController();
    research
      .scoreboard(controller.signal)
      .then(setBoard)
      .catch((e: unknown) => {
        if (controller.signal.aborted) return;
        setError(
          e instanceof ApiError && e.status === 503
            ? "The store has recorded nothing yet."
            : e instanceof ApiError
              ? e.detail
              : String(e),
        );
      });
    return () => controller.abort();
  }, []);

  if (error) {
    return (
      <p className="text-sm text-[var(--color-warn)]">{error}</p>
    );
  }
  if (!board) return <p className="text-sm text-[var(--color-dim)]">Reading the store…</p>;

  const enough = board.proposed.scored >= MIN_COHORT;

  return (
    <div>
      <div className="mb-6 rounded-md border border-[var(--color-line)] bg-[var(--color-surface)] px-4 py-3 text-sm leading-relaxed">
        Radar's selection is compared against{" "}
        <strong>the tokens it refused</strong>, priced in the same passes and the
        same way. Not against a published population figure: those enter at a
        token's first fill and Radar enters around forty minutes later, so the
        two are different quantities and the difference between them would be
        the measurement rather than the selection.
      </div>

      <dl className="grid grid-cols-2 gap-4 sm:grid-cols-4">
        <Figure label="decisions" value={board.decisions.toLocaleString()} />
        <Figure label="scored" value={board.scored.toLocaleString()} />
        <Figure label="proposed" value={board.proposed.scored.toLocaleString()} />
        <Figure label="refused" value={board.refused.scored.toLocaleString()} />
      </dl>

      {enough ? (
        <Comparison board={board} />
      ) : (
        <p className="mt-6 rounded-md border border-[var(--color-line)] bg-[var(--color-surface)] px-4 py-3 text-sm">
          <strong className="text-[var(--color-warn)]">Not enough data.</strong>{" "}
          {board.proposed.scored} of {MIN_COHORT} scored proposals. No comparison
          is shown, because one drawn from fewer than {MIN_COHORT} tokens in a
          single regime would be noise presented as a finding — and a number on a
          screen gets read while the caveat under it does not.
        </p>
      )}

      <p className="mt-6 text-xs leading-relaxed text-[var(--color-dim)]">
        Returns are net of an assumed {board.cost_bps} bps round trip — the
        measured figure. Fewer than one token in ten ever finishes above it,
        which is the single most important number here.
      </p>
    </div>
  );
}

function Comparison({ board }: { board: Board }) {
  const rows = [
    { name: "Proposed", cohort: board.proposed },
    { name: "Refused (control)", cohort: board.refused },
  ];

  return (
    <table className="mt-6 w-full text-sm">
      <thead>
        <tr className="border-b border-[var(--color-line)] text-left text-xs uppercase tracking-wide text-[var(--color-dim)]">
          <th className="pb-2 font-medium">cohort</th>
          <th className="pb-2 text-right font-medium">n</th>
          <th className="pb-2 text-right font-medium">median</th>
          <th className="pb-2 text-right font-medium">cleared cost</th>
        </tr>
      </thead>
      <tbody className="divide-y divide-[var(--color-line)]">
        {rows.map(({ name, cohort }) => {
          const med = median(cohort.returns_bps);
          const cleared = clearedCost(cohort.returns_bps, board.cost_bps);
          return (
            <tr key={name}>
              <td className="py-2">{name}</td>
              <td className="py-2 text-right tabular-nums">
                {cohort.scored.toLocaleString()}
              </td>
              <td
                className={`py-2 text-right tabular-nums ${
                  med !== null && med > 0
                    ? "text-[var(--color-good)]"
                    : "text-[var(--color-refuse)]"
                }`}
              >
                {med === null ? "—" : pct(med)}
              </td>
              <td className="py-2 text-right tabular-nums text-[var(--color-dim)]">
                {cohort.returns_bps.length === 0
                  ? "—"
                  : `${cleared} / ${cohort.returns_bps.length}`}
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}

function Figure({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <dt className="text-xs uppercase tracking-wide text-[var(--color-dim)]">
        {label}
      </dt>
      <dd className="mt-1 text-xl tabular-nums">{value}</dd>
    </div>
  );
}
