// SPDX-License-Identifier: Apache-2.0
//! The honest scoreboard.
//!
//! The screen most likely to be read as a performance claim, so it is built to
//! resist that. Four rules govern it, and three of them are here because an
//! earlier version broke them.
//!
//! **The returns are gross, and the page must say so.** `Cohort::returns_bps` is
//! documented `Gross` — costs are applied by the caller. This page rendered the
//! gross median under a footnote reading *"Returns are net of an assumed 850 bps
//! round trip"*. They were not. The single most-read number on the screen
//! overstated itself by the whole round trip, in the flattering direction. Both
//! figures are now shown, labelled, side by side.
//!
//! **The refusals are not a control, and must not be laid out like one.**
//! `AGENTS.md` says it outright: every scoreable refusal is
//! `CapacityBelowFloor`, so the cohort is composed entirely of tokens Radar
//! measured and found it could not sell. Research 0014 concluded the comparison
//! was unusable and 0017 replaced it with a population control. Two rows in one
//! table invite subtraction, and the difference between these two is not an edge.
//!
//! **The entry and the exit are different instruments.** The entry is a sell
//! quote from the exit probe's ladder; the exit pools buys and sells and sits
//! near the mid. Research 0016 measures the gap at **at least +128 bps** against
//! a gross median of +21 — six times the signal, in the direction that flatters
//! the selection. It is not subtracted, because a floor baked into a headline
//! overclaims the other way; it is disclosed above the numbers instead of below
//! them.
//!
//! **"Not enough data" is the headline until it is not.** Below the cohort floor
//! the page says so in as many words rather than showing a number with a caveat
//! beneath it, because a number on a screen is read and a caveat is not. That
//! rule is why the three above are stated *above* the figures.

import { useEffect, useState } from "react";
import { ApiError, research, type Cohort, type Scoreboard as Board } from "./api";
import { CapacityWall } from "./CapacityWall";
import { clearedCost, median, netOfCost, pct } from "./honesty";

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
    return <p className="text-sm text-[var(--color-warn)]">{error}</p>;
  }
  if (!board)
    return <p className="text-sm text-[var(--color-dim)]">Reading the store…</p>;

  const enough = board.proposed.scored >= MIN_COHORT;

  return (
    <div>
      <Basis costBps={board.cost_bps} />

      <dl className="grid grid-cols-2 gap-4 sm:grid-cols-4">
        <Figure label="decisions" value={board.decisions.toLocaleString()} />
        <Figure label="scored" value={board.scored.toLocaleString()} />
        <Figure label="proposed" value={board.proposed.scored.toLocaleString()} />
        <Figure label="refused" value={board.refused.scored.toLocaleString()} />
      </dl>

      {/* The reason the returns above are the wrong thing to look at. Placed
          after them deliberately: a reader arrives wanting a performance
          number, and the honest answer is that the constraint is capacity. */}
      <CapacityWall />

      {enough ? (
        <>
          <Distribution
            heading="What the selection returned"
            cohort={board.proposed}
            costBps={board.cost_bps}
          />
          <NotAControl cohort={board.refused} costBps={board.cost_bps} />
        </>
      ) : (
        <p className="mt-6 rounded-md border border-[var(--color-line)] bg-[var(--color-surface)] px-4 py-3 text-sm">
          <strong className="text-[var(--color-warn)]">Not enough data.</strong>{" "}
          {board.proposed.scored} of {MIN_COHORT} scored proposals. No figures are
          shown, because one drawn from fewer than {MIN_COHORT} tokens in a single
          regime would be noise presented as a finding — and a number on a screen
          gets read while the caveat under it does not.
        </p>
      )}
    </div>
  );
}

/// Everything a reader has to know *before* the numbers, not after them.
///
/// Placed above deliberately. The page's own rule is that a number is read and a
/// caveat is not, and every item here changes what the numbers mean rather than
/// merely qualifying them.
function Basis({ costBps }: { costBps: number }) {
  return (
    <div className="mb-6 space-y-3 rounded-md border border-[var(--color-line)] bg-[var(--color-surface)] px-4 py-3 text-sm leading-relaxed">
      <p>
        <strong className="text-[var(--color-warn)]">
          Nothing here was traded.
        </strong>{" "}
        These are what the selection would have returned had it been traded. Two
        separate things stop that being what happened, and the second is not a
        matter of policy: Radar cannot <strong>build</strong> a transaction for a
        token it selects at all. The signer reads every account it signs, so it
        takes legacy transactions only; pump.fun&rsquo;s pre-graduation
        liquidity routes versioned (research 0021). The round trip below has
        never been <em>available</em>, not merely never taken.
      </p>
      <p>
        <strong>The entry and the exit are different instruments.</strong> The
        entry is a sell quote off the exit probe&rsquo;s ladder; the exit pools
        buys and sells and sits near the mid. Research 0016 measures that gap at{" "}
        <strong>at least +128 bps</strong>, in the direction that flatters the
        selection. It is not subtracted here — a floor baked into a headline
        overclaims the other way — so read every figure below as an{" "}
        <strong>upper bound</strong>.
      </p>
      <p>
        <strong>Gross and net are both shown.</strong> The store records returns
        gross; the measured round trip is {costBps} bps. Fewer than one token in
        ten ever finishes above it.
      </p>
    </div>
  );
}

/// One cohort's returns, gross and net, with the count that cleared costs.
function Distribution({
  heading,
  cohort,
  costBps,
}: {
  heading: string;
  cohort: Cohort;
  costBps: number;
}) {
  const gross = median(cohort.returns_bps);
  const cleared = clearedCost(cohort.returns_bps, costBps);

  return (
    <section className="mt-6">
      <h3 className="mb-2 text-sm font-medium">{heading}</h3>
      <div className="overflow-x-auto">
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b border-[var(--color-line)] text-left text-xs uppercase tracking-wide text-[var(--color-dim)]">
              <th scope="col" className="pb-2 font-medium">
                measure
              </th>
              <th scope="col" className="pb-2 text-right font-medium">
                value
              </th>
            </tr>
          </thead>
          <tbody className="divide-y divide-[var(--color-line)]">
            <Row label="scored" value={cohort.scored.toLocaleString()} />
            <Row
              label="median return, gross"
              // Not coloured. A positive gross median on a cohort that could
              // not have been traded, measured across two instruments, is not a
              // gain — and green is what says it was.
              value={gross === null ? "—" : pct(gross)}
            />
            <Row
              label={`median return, net of ${costBps} bps`}
              // The figure the page used to claim it was already showing.
              value={gross === null ? "—" : pct(netOfCost(gross, costBps))}
            />
            <Row
              label={`cleared ${costBps} bps`}
              value={
                cohort.returns_bps.length === 0
                  ? "—"
                  : `${cleared} / ${cohort.returns_bps.length}`
              }
            />
          </tbody>
        </table>
      </div>
    </section>
  );
}

/// The refusals, shown apart from the selection and labelled as not a control.
///
/// Deliberately not a second row in the table above. Two rows side by side is an
/// invitation to subtract them, and this difference is not an edge.
function NotAControl({ cohort, costBps }: { cohort: Cohort; costBps: number }) {
  const gross = median(cohort.returns_bps);

  return (
    <section className="mt-8">
      <h3 className="mb-2 text-sm font-medium">
        What Radar refused{" "}
        <span className="text-[var(--color-refuse)]">— not a control</span>
      </h3>
      <p className="mb-3 text-sm leading-relaxed text-[var(--color-dim)]">
        Every scoreable refusal here is <code>CapacityBelowFloor</code>, so this
        cohort is composed entirely of tokens Radar measured and found{" "}
        <strong>it could not sell</strong>. Research 0014 compared the two and
        concluded the comparison was unusable for exactly that reason; 0017
        replaced it with a population control of 38,461 tokens Radar never
        decided on, and found a median edge of <strong>0 bps</strong> across four
        matched strata. That is the result, and it is not on this page because
        this endpoint does not compute it.
      </p>
      <dl className="grid grid-cols-2 gap-4 sm:grid-cols-3">
        <Figure label="scored" value={cohort.scored.toLocaleString()} />
        <Figure
          label="median, gross"
          value={gross === null ? "—" : pct(gross)}
        />
        <Figure
          label={`median, net of ${costBps} bps`}
          value={gross === null ? "—" : pct(netOfCost(gross, costBps))}
        />
      </dl>
    </section>
  );
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <tr>
      <td className="py-2">{label}</td>
      <td className="py-2 text-right tabular-nums">{value}</td>
    </tr>
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
