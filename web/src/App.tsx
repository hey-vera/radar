// SPDX-License-Identifier: Apache-2.0
//! The funnel: what Radar decided, and where it stopped.
//!
//! The honest headline of this product is a **refusal count**, not a P&L curve.
//! Research 0009 measured the population Radar selects from at a median of
//! −13.4% before costs, with fewer than one token in ten finishing above an 850
//! bps round trip. An interface implying easy profit would be lying; one that
//! makes *why we refused* legible is the actual product.
//!
//! So the widest number on the page is what was recorded and the narrowest is
//! what was authorised, and under the shipped policy the last one is zero.

import { useCallback, useEffect, useState } from "react";
import { Agent } from "./Agent";
import { api, ApiError, subscribe, type Funnel } from "./api";

/** What the page is doing right now. */
type Load<T> =
  | { state: "loading" }
  | { state: "ready"; value: T }
  | { state: "failed"; status: number; detail: string };

function useFunnel(): [Load<Funnel>, () => void] {
  const [load, setLoad] = useState<Load<Funnel>>({ state: "loading" });

  const refresh = useCallback(() => {
    const controller = new AbortController();
    api
      .funnel(controller.signal)
      .then((value) => setLoad({ state: "ready", value }))
      .catch((error: unknown) => {
        if (controller.signal.aborted) return;
        const { status, detail } =
          error instanceof ApiError
            ? error
            : { status: 0, detail: String(error) };
        setLoad({ state: "failed", status, detail });
      });
    return () => controller.abort();
  }, []);

  useEffect(() => {
    const cancel = refresh();
    // Re-fetch when the store moves rather than on a timer. The stream says
    // *that* something changed; what changed is this fetch.
    const stop = subscribe(refresh);
    return () => {
      cancel?.();
      stop();
    };
  }, [refresh]);

  return [load, refresh];
}

export function App() {
  const [load] = useFunnel();

  return (
    <main className="mx-auto max-w-3xl px-6 py-12">
      <header className="mb-10">
        <h1 className="text-2xl font-semibold tracking-tight">Radar</h1>
        <p className="mt-1 text-sm text-[var(--color-dim)]">
          Solana research intelligence. A record of what was refused, and why.
        </p>
      </header>

      {load.state === "loading" && <Placeholder>Reading the store…</Placeholder>}

      {load.state === "failed" && (
        <Placeholder tone="warn">
          {load.status === 503
            ? "The store has recorded nothing yet. That is a fresh instance, not a fault."
            : `Could not read the funnel — ${load.detail}`}
        </Placeholder>
      )}

      {load.state === "ready" && <FunnelView funnel={load.value} />}

      <Agent />
    </main>
  );
}

function FunnelView({ funnel }: { funnel: Funnel }) {
  const widest = Math.max(...funnel.stages.map((s) => s.count), 1);

  return (
    <>
      {funnel.policy_closed && (
        <p className="mb-8 rounded-md border border-[var(--color-line)] bg-[var(--color-surface)] px-4 py-3 text-sm">
          <strong className="text-[var(--color-refuse)]">Policy closed.</strong>{" "}
          Nothing can be authorised, whatever a token looks like. Every refusal
          below the last stage is that one fact, not a finding about a token.
        </p>
      )}

      <ol className="space-y-4">
        {funnel.stages.map((stage) => (
          <li key={stage.name}>
            <div className="flex items-baseline justify-between gap-4">
              <span className="text-sm font-medium">{stage.name}</span>
              <span className="text-lg tabular-nums">
                {stage.count.toLocaleString()}
              </span>
            </div>
            <div
              className="mt-1 h-1.5 rounded-full bg-[var(--color-line)]"
              role="presentation"
            >
              <div
                className="h-full rounded-full bg-[var(--color-good)]"
                style={{
                  width: `${Math.max((stage.count / widest) * 100, stage.count > 0 ? 1 : 0)}%`,
                }}
              />
            </div>
            <p className="mt-1 text-xs leading-relaxed text-[var(--color-dim)]">
              {stage.detail}
            </p>
          </li>
        ))}
      </ol>

      {funnel.reasons.length > 0 && (
        <section className="mt-10">
          <h2 className="mb-3 text-sm font-medium uppercase tracking-wide text-[var(--color-dim)]">
            Why candidates were passed over
          </h2>
          <ul className="divide-y divide-[var(--color-line)] rounded-md border border-[var(--color-line)]">
            {funnel.reasons.map((reason) => (
              <li
                key={reason.reason}
                className="flex items-baseline justify-between px-4 py-2 text-sm"
              >
                <span>{reason.reason}</span>
                <span className="tabular-nums text-[var(--color-dim)]">
                  {reason.count.toLocaleString()}
                </span>
              </li>
            ))}
          </ul>
        </section>
      )}

      <p className="mt-10 text-xs text-[var(--color-dim)]">
        As of slot {funnel.as_of.toLocaleString()}.
      </p>
    </>
  );
}

function Placeholder({
  children,
  tone = "dim",
}: {
  children: React.ReactNode;
  tone?: "dim" | "warn";
}) {
  return (
    <p
      className={
        tone === "warn"
          ? "rounded-md border border-[var(--color-line)] bg-[var(--color-surface)] px-4 py-3 text-sm text-[var(--color-warn)]"
          : "text-sm text-[var(--color-dim)]"
      }
    >
      {children}
    </p>
  );
}
