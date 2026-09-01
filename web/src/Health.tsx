// SPDX-License-Identifier: Apache-2.0
//! What the instance is doing, and whether that is fine.
//!
//! The browser half of `radar brief`, and it inherits that command's one rule:
//! **absent is not healthy.** A field the server did not send is rendered as
//! "cannot see" rather than omitted, because a missing row and a healthy row
//! look identical when the missing one is simply not drawn.
//!
//! It does not duplicate `brief`'s alarms. `brief` exits non-zero on a timer and
//! is what wakes somebody up; this is what you read once it has.

import { useEffect, useState } from "react";
import {
  ApiError,
  operator,
  research,
  type Health as HealthReport,
  type StoreCounts,
} from "./api";

export function Health() {
  const [health, setHealth] = useState<HealthReport | null>(null);
  const [counts, setCounts] = useState<StoreCounts | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const controller = new AbortController();
    research
      .health(controller.signal)
      .then(setHealth)
      .catch((e: unknown) => {
        if (!controller.signal.aborted) {
          setError(e instanceof ApiError ? e.detail : String(e));
        }
      });
    // `operator`, not `research`, and the import is the point: `/v1/store` is
    // an `Audience::Operator` route. It answers today only because no customer
    // authenticator is configured, so every request falls back to the operator
    // check. The day the customer lane switches on it refuses customer sessions
    // and this block shows its failure message permanently -- which is correct,
    // and is why this screen is the operator's rather than the product's.
    operator
      .store(controller.signal)
      .then(setCounts)
      .catch(() => {
        // Counts are secondary. The health block is what says whether the
        // instance is up, and it renders on its own.
      });
    return () => controller.abort();
  }, []);

  if (error) return <p className="text-sm text-[var(--color-warn)]">{error}</p>;
  if (!health) return <p className="text-sm text-[var(--color-dim)]">Asking…</p>;

  return (
    <div className="space-y-6">
      <dl className="grid grid-cols-2 gap-4 sm:grid-cols-4">
        <Row label="status" value={health.status} good={health.status === "ok"} />
        <Row label="version" value={health.version} />
        <Row
          label="watermark"
          // Null rather than zero for an empty store: "nothing recorded" and
          // "recorded up to genesis" are different states, and the server is
          // careful to distinguish them, so the page must be too.
          value={
            health.watermarkSlot === null
              ? "nothing recorded"
              : `slot ${health.watermarkSlot.toLocaleString()}`
          }
          good={health.watermarkSlot !== null}
        />
        <Row label="instruments" value={String(health.instruments)} />
      </dl>

      <section>
        <h3 className="mb-2 text-xs uppercase tracking-wide text-[var(--color-dim)]">
          Store
        </h3>
        {counts ? (
          <dl className="grid grid-cols-2 gap-4 sm:grid-cols-4">
            <Row label="launches" value={counts.launches.toLocaleString()} />
            <Row label="graduations" value={counts.graduations.toLocaleString()} />
            <Row label="outcomes" value={counts.outcomes.toLocaleString()} />
            <Row label="decisions" value={counts.decisions.toLocaleString()} />
          </dl>
        ) : (
          <p className="text-sm text-[var(--color-warn)]">
            Counts could not be read. That is different from a store holding
            nothing.
          </p>
        )}
      </section>

      <section>
        <h3 className="mb-2 text-xs uppercase tracking-wide text-[var(--color-dim)]">
          Surfaces
        </h3>
        <ul className="space-y-1 text-sm">
          <li>
            Paid surface:{" "}
            <span className="text-[var(--color-dim)]">
              {health.paidSurface
                ? "on"
                : "off — the routes do not exist rather than serving free"}
            </span>
          </li>
          <li>
            Reading assistant:{" "}
            <span className="text-[var(--color-dim)]">
              {health.agent.configured
                ? `${String(health.agent["provider"] ?? "configured")}, last call ${lastCall(health)}`
                : "off — no provider configured"}
            </span>
          </li>
          <li>
            Trading:{" "}
            <span
              className={
                health.policyClosed
                  ? "text-[var(--color-refuse)]"
                  : "text-[var(--color-warn)]"
              }
            >
              {health.policyClosed
                ? "policy closed — the deciding policy authorises nothing"
                : "CAPITAL IS ARMED — the deciding policy can authorise"}
            </span>
            <p className="mt-1 text-xs leading-relaxed text-[var(--color-dim)]">
              The policy is only one of two independent reasons nothing trades,
              and it is the reversible one. Radar also cannot{" "}
              <strong>build</strong> a transaction for a token it selects: the
              signer reads every account it signs, so it takes legacy
              transactions only, and pump.fun&rsquo;s pre-graduation liquidity —
              the only venue that lists these tokens — routes versioned. Eight of
              eight recent candidates refused legacy (research 0021). Opening the
              policy would not change that.
            </p>
          </li>
        </ul>
      </section>
    </div>
  );
}

/// How the agent's last call went, as the server reported it.
///
/// "never" is its own answer rather than being folded into either success or
/// failure: a restart makes it the normal state, so reading it as a failure
/// would alarm on every deploy, and reading it as a success would call an
/// untested provider working.
function lastCall(health: HealthReport): string {
  const last = health.agent["last"];
  if (typeof last !== "object" || last === null) return "unknown";
  const state = (last as { last_call?: string }).last_call;
  return state ?? "unknown";
}

function Row({
  label,
  value,
  good,
}: {
  label: string;
  value: string;
  good?: boolean;
}) {
  return (
    <div>
      <dt className="text-xs uppercase tracking-wide text-[var(--color-dim)]">
        {label}
      </dt>
      <dd
        className={`mt-1 text-sm tabular-nums ${
          good === undefined
            ? ""
            : good
              ? "text-[var(--color-good)]"
              : "text-[var(--color-warn)]"
        }`}
      >
        {value}
      </dd>
    </div>
  );
}
