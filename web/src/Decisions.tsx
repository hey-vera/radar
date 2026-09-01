// SPDX-License-Identifier: Apache-2.0
//! What Radar decided, and where it stopped.
//!
//! The honest headline of this product is a **refusal count**, not a P&L curve.
//! Research 0009 measured the population Radar selects from at a median of
//! −13.4% before costs, with fewer than one token in ten finishing above an 850
//! bps round trip. An interface implying easy profit would be lying; one that
//! makes *why we refused* legible is the actual product.
//!
//! So the widest number on the page is what was recorded and the narrowest is
//! what was authorised, and under the shipped policy the last one is zero.
//!
//! # The funnel is the header, not the page
//!
//! The aggregate barely moves: ~960 decisions a day fall into the same handful
//! of reason buckets, so a reader coming back tomorrow learns nothing from it
//! they did not know today. What changes is *which* tokens were refused and
//! *why*, and that is [`Feed`] below it.

import { useCallback, useEffect, useState } from "react";

import { api, ApiError, subscribe, type Funnel } from "./api";
import { Activity } from "./Activity";
import { Feed } from "./Feed";
import { Link } from "wouter";
import { decisionsPath } from "./routes";

/** What the page is doing right now. */
type Load<T> =
  | { state: "loading" }
  | { state: "ready"; value: T }
  | { state: "failed"; status: number; detail: string };

function useFunnel(): [Load<Funnel>, boolean] {
  const [load, setLoad] = useState<Load<Funnel>>({ state: "loading" });
  // Whether the change stream is following the store. `/v1/events` is an
  // operator route, so a customer session will be refused there -- and a refused
  // `EventSource` retries silently forever. Without this the page would keep
  // rendering its first fetch and look like a store where nothing had happened.
  const [stale, setStale] = useState(false);

  const refresh = useCallback(() => {
    const controller = new AbortController();
    api
      .funnel(controller.signal)
      .then((value) => setLoad({ state: "ready", value }))
      .catch((error: unknown) => {
        if (controller.signal.aborted) return;
        const { status, detail } =
          error instanceof ApiError ? error : { status: 0, detail: String(error) };
        setLoad({ state: "failed", status, detail });
      });
    return () => controller.abort();
  }, []);

  useEffect(() => {
    const cancel = refresh();
    // Re-fetch when the store moves rather than on a timer. The stream says
    // *that* something changed; what changed is this fetch.
    const stop = subscribe(refresh, setStale);
    return () => {
      cancel?.();
      stop();
    };
  }, [refresh]);

  return [load, stale];
}

export function Decisions() {
  const [load, stale] = useFunnel();

  return (
    <>
      {stale && (
        <Placeholder tone="warn">
          Not following the store. The figures below are from the last successful
          read and may be older than they look — this is not a report that
          nothing has changed.
        </Placeholder>
      )}

      {load.state === "loading" && <Placeholder>Reading the store…</Placeholder>}

      {load.state === "failed" && (
        <Placeholder tone="warn">
          {load.status === 503
            ? "The store has recorded nothing yet. That is a fresh instance, not a fault."
            : `Could not read the funnel — ${load.detail}`}
        </Placeholder>
      )}

      {/* Whether the recorder is still running, in a form that shows a gap.
          Above the funnel because an outage makes every figure below it stale,
          and a reader should meet that first. */}
      <Activity />

      {load.state === "ready" && <FunnelView funnel={load.value} />}

      {/* The record itself. The funnel above is its header: an aggregate that
          barely moves, over a list whose contents change hourly. */}
      <Feed />
    </>
  );
}

function FunnelView({ funnel }: { funnel: Funnel }) {
  const widest = Math.max(...funnel.stages.map((s) => s.count), 1);

  return (
    <>
      {funnel.policy_closed && (
        <p className="mb-8 rounded-md border border-[var(--color-line)] bg-[var(--color-surface)] px-4 py-3 text-sm">
          <strong className="text-[var(--color-refuse)]">Policy closed.</strong>{" "}
          The policy Radar decides with authorises nothing, whatever a token
          looks like — so every refusal below the last stage is that one fact,
          not a finding about a token.{" "}
          <span className="text-[var(--color-dim)]">
            This describes the deciding policy. It is not a claim about the
            signer, which holds its own policy in another process and can refuse
            what this one permits.
          </span>
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
              <li key={reason.reason}>
                {/* The whole row is the link. "Show me the four thousand
                    refusals behind this number" is the cheapest and most
                    valuable interaction in the product, and it is a question
                    about Radar's rules rather than about a price. */}
                <Link
                  href={decisionsPath({
                    reason: reason.reason,
                    conclusion: null,
                  })}
                  className="flex items-baseline justify-between px-4 py-2 text-sm hover:bg-[var(--color-surface)]"
                >
                  <span>{reason.reason}</span>
                  <span className="tabular-nums text-[var(--color-dim)]">
                    {reason.count.toLocaleString()}
                  </span>
                </Link>
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

export function Placeholder({
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
          ? "mb-6 rounded-md border border-[var(--color-line)] bg-[var(--color-surface)] px-4 py-3 text-sm text-[var(--color-warn)]"
          : "text-sm text-[var(--color-dim)]"
      }
    >
      {children}
    </p>
  );
}
