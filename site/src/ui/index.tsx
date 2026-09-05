// SPDX-License-Identifier: Apache-2.0
//! The small set of pieces every page is built from.
//!
//! # Why these are here and not installed
//!
//! Copied in, shadcn-style, rather than pulled from a component library. Four
//! pages need about six shapes between them, and a library would be a
//! dependency, an upgrade treadmill, and a set of decisions made by somebody
//! who has not read `AGENTS.md` §5.
//!
//! # Why there is no Radix here yet
//!
//! Radix primitives are the right answer for a dropdown, a dialog or a tooltip:
//! focus management and keyboard behaviour are genuinely hard and hand-rolled
//! versions get them wrong. **None of these four pages has one.** Adding the
//! dependency before a component needs it would be a layer with no caller,
//! which this repository has produced three of.
//!
//! The moment a real overlay appears, `@radix-ui/react-*` is the answer and this
//! comment is the note saying so.

import type { ReactNode } from "react";

/**
 * A figure, its label, and where it came from.
 *
 * **The provenance line is not optional and is not a prop.** Every number on
 * this site is a claim made to a stranger about somebody else's project, and
 * `0024` records in capitals what happens to a figure published without its
 * date: the note measuring the same quantities before it was wrong by 2.7×
 * nine days later.
 */
export function Figure({
  value,
  label,
  note,
  tone = "text",
}: {
  value: string;
  label: string;
  note?: string;
  tone?: "text" | "signal" | "good";
}) {
  const colour =
    tone === "signal"
      ? "text-[var(--color-signal)]"
      : tone === "good"
        ? "text-[var(--color-good)]"
        : "text-[var(--color-text)]";
  return (
    <div>
      <div
        className={`tnum text-4xl font-semibold tracking-tight sm:text-5xl ${colour}`}
      >
        {value}
      </div>
      <div className="mt-2 text-sm text-[var(--color-dim)]">{label}</div>
      {note && (
        <div className="mt-1 text-xs text-[var(--color-faint)]">{note}</div>
      )}
    </div>
  );
}

/** A bounded band of page, so every section lines up with every other. */
export function Section({
  children,
  id,
  className = "",
}: {
  children: ReactNode;
  id?: string;
  className?: string;
}) {
  return (
    <section
      {...(id ? { id } : {})}
      className={`relative z-10 mx-auto w-full max-w-5xl px-6 py-16 sm:py-24 ${className}`}
    >
      {children}
    </section>
  );
}

/** A section's heading, with the small label above it. */
export function Heading({
  kicker,
  children,
}: {
  kicker?: string;
  children: ReactNode;
}) {
  return (
    <div className="mb-8">
      {kicker && (
        <div className="mb-3 font-mono text-xs tracking-widest text-[var(--color-signal-dim)] uppercase">
          {kicker}
        </div>
      )}
      <h2 className="text-2xl font-semibold tracking-tight text-balance sm:text-3xl">
        {children}
      </h2>
    </div>
  );
}

/** A bordered panel. */
export function Card({
  children,
  className = "",
}: {
  children: ReactNode;
  className?: string;
}) {
  return (
    <div
      className={`rounded-lg border border-[var(--color-line)] bg-[var(--color-surface)] p-6 ${className}`}
    >
      {children}
    </div>
  );
}

/**
 * What a page says when it has nothing to show.
 *
 * **A component, so that "nothing yet" cannot be rendered as an empty table by
 * accident.** An empty table implies a week ran and nobody engaged; a pool
 * reading `0.00 SOL` implies a contest nobody won. Both are the LEARNINGS 5
 * failure — a thing that did not run looking exactly like a thing that ran and
 * found nothing — and on this site both are visible to strangers.
 *
 * The `why` is required. "No data" on its own is the failure with better
 * typography.
 */
export function Nothing({ what, why }: { what: string; why: string }) {
  return (
    <Card className="border-dashed">
      <p className="text-[var(--color-text)]">{what}</p>
      <p className="mt-2 text-sm text-[var(--color-dim)]">{why}</p>
    </Card>
  );
}

/** When a figure was measured, in the same words everywhere. */
export function Measured({ ago, at }: { ago: string | null; at?: string }) {
  // `null` means the timestamp did not parse or is in the future. Saying
  // nothing beats rendering "in 3 hours", which reads as broken data.
  if (ago === null) return null;
  return (
    <p className="mt-6 text-xs text-[var(--color-faint)]">
      Measured {ago}
      {at ? ` (${at})` : ""}. Every figure here has a date on it and is expected
      to move.
    </p>
  );
}
