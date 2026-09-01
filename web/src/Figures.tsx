// SPDX-License-Identifier: Apache-2.0
//! The three primitives that carry the interface's honesty rules as **types**
//! rather than as discipline.
//!
//! Every one of them exists because the rule it encodes was broken somewhere by
//! a component that rendered a number directly. A rule enforced by remembering
//! is a rule that holds until the next person, and this file is the attempt to
//! stop relying on that.

import { useCallback, useState } from "react";
import { partitionReasons, pct } from "./honesty";

/**
 * A measured figure, or the fact that there is not one.
 *
 * **A null cannot be rendered as a number by this component.** That is the whole
 * point of it: AGENTS.md rule 9 says absent is not zero, and the failure it
 * prevents is a `?? 0` that turns "nobody measured this" into "broke even" — the
 * whole population's median dressed up as a result.
 *
 * `tone` chooses whether the sign is coloured. It defaults to **off**, because
 * the last component to colour a figure by sign was colouring a gross,
 * wrong-entry return as profit and loss. Colour is a claim that the number means
 * gain or loss to the reader; pass `tone="pnl"` only where that is true.
 */
export function Figure({
  value,
  absent = "—",
  tone = "plain",
  suffix,
}: {
  value: number | null | undefined;
  /** What to show when there is no measurement. Never a number. */
  absent?: string;
  tone?: "plain" | "pnl";
  suffix?: string;
}) {
  if (value === null || value === undefined) {
    return (
      <span className="tabular-nums text-[var(--color-absent)]" title="not measured">
        {absent}
      </span>
    );
  }
  const colour =
    tone === "pnl"
      ? value > 0
        ? "text-[var(--color-gain)]"
        : value < 0
          ? "text-[var(--color-loss)]"
          : ""
      : "";
  return (
    <span className={`tabular-nums ${colour}`}>
      {value.toLocaleString()}
      {suffix}
    </span>
  );
}

/**
 * A return in basis points, as a signed percentage.
 *
 * Separate from [`Figure`] because the sign is not decoration here — it is the
 * redundant channel that makes the figure readable without colour, which is
 * what keeps the screen usable for the ~8% of men with red-green colour vision
 * deficiency. `pct` always emits it.
 */
export function Bps({
  value,
  tone = "plain",
}: {
  value: number | null | undefined;
  tone?: "plain" | "pnl";
}) {
  if (value === null || value === undefined) {
    return (
      <span className="tabular-nums text-[var(--color-absent)]" title="not measured">
        —
      </span>
    );
  }
  const colour =
    tone === "pnl"
      ? value > 0
        ? "text-[var(--color-gain)]"
        : value < 0
          ? "text-[var(--color-loss)]"
          : ""
      : "";
  return <span className={`tabular-nums ${colour}`}>{pct(value)}</span>;
}

/** How many characters of an address to show at each end. */
const KEEP = 6;

/**
 * A Solana address, middle-truncated, with the whole of it one click away.
 *
 * Base58 addresses are 43 or 44 characters. Rendered whole with `break-all` they
 * wrap to two lines and turn every table row into a paragraph; truncated without
 * a way to recover them they are useless, because the only thing anyone does
 * with an address is paste it somewhere else.
 *
 * The full value is in `title` and on the clipboard, so nothing is lost.
 */
export function Address({
  value,
  className = "",
}: {
  value: string;
  className?: string;
}) {
  const [copied, setCopied] = useState(false);

  const copy = useCallback(() => {
    // `navigator.clipboard` is absent over plain HTTP and in some embedded
    // views. Failing quietly is right here -- the address is still visible in
    // the tooltip, so a reader is not stuck -- but it must not throw.
    void navigator.clipboard
      ?.writeText(value)
      .then(() => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1200);
      })
      .catch(() => {
        /* The tooltip still carries it. */
      });
  }, [value]);

  const short =
    value.length > KEEP * 2 + 1
      ? `${value.slice(0, KEEP)}…${value.slice(-KEEP)}`
      : value;

  return (
    <button
      type="button"
      onClick={copy}
      title={value}
      aria-label={`Copy address ${value}`}
      className={`font-mono text-xs text-[var(--color-dim)] hover:text-[var(--color-text)] ${className}`}
    >
      {copied ? "copied" : short}
    </button>
  );
}

/**
 * A kernel or strategy refusal list, split into the three kinds it contains.
 *
 * The kernel already makes this distinction and the interface used to throw two
 * thirds of it away. Under `Policy::CLOSED` every limit is zero, so seven
 * refusals fire at once — and a reader shown seven items concludes there are
 * seven things wrong with the token. None of them is about the token.
 *
 * The split is also the thing a reader most needs in order to act. "Radar will
 * never touch this" and "Radar could not tell yet" are different answers, and
 * they were rendered identically.
 */
export function ReasonList({ reasons }: { reasons: readonly string[] }) {
  const { structural, evidence, policy } = partitionReasons(reasons);

  if (reasons.length === 0) return null;

  return (
    <div className="space-y-3">
      {structural.length > 0 && (
        <Group
          heading="About this token — permanent"
          note="No amount of waiting changes these."
          items={structural}
          tone="text-[var(--color-loss)]"
        />
      )}

      {evidence.length > 0 && (
        <Group
          heading="About the evidence — may change"
          note="These describe what Radar could measure, not what the token is."
          items={evidence}
          tone="text-[var(--color-warn)]"
        />
      )}

      {policy.length > 0 && (
        <p className="text-sm">
          <strong className="text-[var(--color-refuse)]">Policy closed.</strong>{" "}
          <span className="text-[var(--color-dim)]">
            {policy.length} of these refusals are that one fact — every limit is
            zero, so every comparison against zero fails. They are not{" "}
            {policy.length} findings about this token.
          </span>
        </p>
      )}
    </div>
  );
}

function Group({
  heading,
  note,
  items,
  tone,
}: {
  heading: string;
  note: string;
  items: readonly string[];
  tone: string;
}) {
  return (
    <div>
      <p className="text-xs uppercase tracking-wide text-[var(--color-dim)]">
        {heading}
      </p>
      <ul className="mt-1 space-y-0.5 text-sm">
        {items.map((reason) => (
          <li key={reason} className={tone}>
            {reason}
          </li>
        ))}
      </ul>
      <p className="mt-1 text-xs text-[var(--color-dim)]">{note}</p>
    </div>
  );
}
