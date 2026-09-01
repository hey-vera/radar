// SPDX-License-Identifier: Apache-2.0
//! Everything Radar recorded about one mint.
//!
//! The screen where a refusal becomes legible. A decision arrives as a list of
//! reason codes, and under `Policy::CLOSED` seven of them fire at once — not
//! because seven things are wrong, but because a closed policy's limits are all
//! zero and every comparison against zero fails.
//!
//! Rendering that verbatim tells a novice there are seven problems. So the
//! policy-artifact refusals collapse into one line and the findings are listed
//! individually, which is the distinction the kernel already makes and the
//! interface previously threw away.

import { useCallback, useEffect, useState } from "react";
import { ApiError, research, type DecisionRecord, type TokenEvidence } from "./api";
import { POLICY_ARTIFACTS } from "./honesty";
import { PricePath } from "./PricePath";

export function Token({ mint }: { mint: string }) {
  const [evidence, setEvidence] = useState<TokenEvidence | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const controller = new AbortController();
    setEvidence(null);
    setError(null);
    research
      .token(mint, controller.signal)
      .then(setEvidence)
      .catch((e: unknown) => {
        if (!controller.signal.aborted) {
          setError(e instanceof ApiError ? e.detail : String(e));
        }
      });
    return () => controller.abort();
  }, [mint]);

  if (error) return <p className="text-sm text-[var(--color-warn)]">{error}</p>;
  if (!evidence) return <p className="text-sm text-[var(--color-dim)]">Reading…</p>;

  const nothing =
    evidence.decisions.length === 0 && evidence.measurements.length === 0;

  return (
    <div className="space-y-6">
      <p className="break-all font-mono text-xs text-[var(--color-dim)]">
        {evidence.mint}
      </p>

      {nothing && (
        <p className="rounded-md border border-[var(--color-line)] bg-[var(--color-surface)] px-4 py-3 text-sm">
          Nothing recorded about this mint. That is not a verdict on the token —
          Radar records what it observed, and it did not observe this.
        </p>
      )}

      {evidence.decisions.map((decision) => (
        <DecisionCard key={decision.decided_at} decision={decision} />
      ))}

      {evidence.measurements.length > 0 && (
        <>
          <PricePath measurements={evidence.measurements} />
          <Measurements evidence={evidence} />
        </>
      )}
    </div>
  );
}

function DecisionCard({ decision }: { decision: DecisionRecord }) {
  const kernelReasons = decision.kernel_reasons ?? [];
  const artifacts = kernelReasons.filter((r) => POLICY_ARTIFACTS.has(r));
  const findings = kernelReasons.filter((r) => !POLICY_ARTIFACTS.has(r));

  return (
    <div className="rounded-md border border-[var(--color-line)] bg-[var(--color-surface)] p-4">
      <div className="flex items-baseline justify-between gap-4">
        <span className="text-sm font-medium">{decision.conclusion}</span>
        <span className="text-xs tabular-nums text-[var(--color-dim)]">
          slot {decision.decided_at.toLocaleString()}
        </span>
      </div>

      {decision.reasons.length > 0 && (
        <p className="mt-2 text-xs text-[var(--color-dim)]">
          Strategy passed over it: {decision.reasons.join(", ")}
        </p>
      )}

      {artifacts.length > 0 && (
        <p className="mt-3 text-sm">
          <strong className="text-[var(--color-refuse)]">Policy closed.</strong>{" "}
          <span className="text-[var(--color-dim)]">
            {artifacts.length} of the kernel's refusals are that one fact — every
            limit is zero, so every comparison against zero fails. They are not{" "}
            {artifacts.length} findings about this token.
          </span>
        </p>
      )}

      {findings.length > 0 && (
        <div className="mt-3">
          <p className="text-xs uppercase tracking-wide text-[var(--color-dim)]">
            Findings about this token
          </p>
          <ul className="mt-1 space-y-0.5 text-sm">
            {findings.map((reason) => (
              <li key={reason} className="text-[var(--color-warn)]">
                {reason}
              </li>
            ))}
          </ul>
        </div>
      )}

      <dl className="mt-3 grid grid-cols-2 gap-3 text-xs">
        <Evidence
          label="coordination"
          // Absent because the source could not answer, never because the
          // launch looked clean. Collapsing those would quietly clear a bundle.
          value={decision.coordination}
          absent="launch block unreadable"
        />
        <Evidence
          label="authority prevalence"
          value={decision.authority_prevalence}
          absent="table unreadable"
        />
      </dl>
    </div>
  );
}

function Evidence({
  label,
  value,
  absent,
}: {
  label: string;
  value: string | null;
  absent: string;
}) {
  return (
    <div>
      <dt className="uppercase tracking-wide text-[var(--color-dim)]">{label}</dt>
      <dd className={value ? "" : "text-[var(--color-warn)]"}>
        {value ?? absent}
      </dd>
    </div>
  );
}

function Measurements({ evidence }: { evidence: TokenEvidence }) {
  return (
    <section>
      <h3 className="mb-2 text-xs uppercase tracking-wide text-[var(--color-dim)]">
        Measurements
      </h3>
      <div className="overflow-x-auto">
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b border-[var(--color-line)] text-left text-xs uppercase tracking-wide text-[var(--color-dim)]">
              <th scope="col" className="pb-2 font-medium">
                at slot
              </th>
              <th scope="col" className="pb-2 text-right font-medium">
                last trade
              </th>
              <th scope="col" className="pb-2 text-right font-medium">
                price reads
              </th>
              <th scope="col" className="pb-2 text-right font-medium">
                held to end
              </th>
              <th scope="col" className="pb-2 text-right font-medium">
                graduated
              </th>
            </tr>
          </thead>
          <tbody className="divide-y divide-[var(--color-line)]">
            {evidence.measurements.map((m) => (
              <tr key={m.measured_at}>
                <td className="py-1.5 tabular-nums">
                  {m.measured_at.toLocaleString()}
                </td>
                {/* A `max`, so it cannot be inflated by re-reading. This is the
                    column that answers whether the token still trades. */}
                <td className="py-1.5 text-right tabular-nums">
                  {m.last_transfer_slot === null ? (
                    <span className="text-[var(--color-dim)]">never</span>
                  ) : (
                    m.last_transfer_slot.toLocaleString()
                  )}
                </td>
                {/* Not "fills", which is what this column used to be headed.
                    The number over-counts across overlapping windows and grows
                    while nothing trades, so a reader must not take it as
                    evidence that something changed hands. */}
                <td className="py-1.5 text-right tabular-nums text-[var(--color-dim)]">
                  {m.price_reads.toLocaleString()}
                </td>
                {/* Deliberately not coloured. It was green above zero and red
                    below, which reads as "this is what you made" -- and it is
                    neither net of the 850 bps round trip nor measured from
                    Radar's entry. The sign carries the direction. */}
                <td className="py-1.5 text-right tabular-nums">
                  {m.held_to_end_bps === null ? (
                    <span className="text-[var(--color-dim)]">not priced</span>
                  ) : (
                    `${m.held_to_end_bps > 0 ? "+" : ""}${(m.held_to_end_bps / 100).toFixed(1)}%`
                  )}
                </td>
                <td className="py-1.5 text-right tabular-nums text-[var(--color-dim)]">
                  {m.graduated_at === null ? "no" : m.graduated_at.toLocaleString()}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      <dl className="mt-3 space-y-1.5 text-xs leading-relaxed text-[var(--color-dim)]">
        <div>
          <dt className="inline font-medium">Price reads </dt>
          <dd className="inline">
            is not a fill count. The six-hour price windows overlap by five
            hours and the fold adds, so a single trade is counted again on every
            hourly pass — it grows while nothing trades, and two rows are not
            comparable. Use <strong>last trade</strong> for whether anything
            changed hands.
          </dd>
        </div>
        <div>
          <dt className="inline font-medium">Held to end </dt>
          <dd className="inline">
            is <strong>gross</strong> — nothing here is net of the measured 850
            bps round trip — and it runs from the token&rsquo;s{" "}
            <strong>first fill</strong>, which is not Radar&rsquo;s entry. Radar
            decides around forty minutes later, by which point the token has
            usually already moved. It is what the token did, not what Radar
            would have made.
          </dd>
        </div>
      </dl>
    </section>
  );
}

/// A box for pasting a mint, so the screen is reachable without a router.
export function TokenLookup() {
  const [typed, setTyped] = useState("");
  const [mint, setMint] = useState<string | null>(null);
  const submit = useCallback(
    (event: React.FormEvent) => {
      event.preventDefault();
      const trimmed = typed.trim();
      if (trimmed) setMint(trimmed);
    },
    [typed],
  );

  return (
    <div>
      <form onSubmit={submit} className="flex gap-2">
        <input
          value={typed}
          onChange={(e) => setTyped(e.target.value)}
          placeholder="Paste a mint address"
          className="min-w-0 flex-1 rounded-md border border-[var(--color-line)] bg-[var(--color-ink)] px-3 py-2 font-mono text-xs outline-none focus:border-[var(--color-dim)]"
        />
        <button
          type="submit"
          disabled={!typed.trim()}
          className="shrink-0 rounded-md border border-[var(--color-line)] px-3 py-2 text-sm hover:border-[var(--color-dim)] disabled:opacity-50"
        >
          Look up
        </button>
      </form>
      {mint && (
        <div className="mt-4">
          <Token mint={mint} />
        </div>
      )}
    </div>
  );
}
