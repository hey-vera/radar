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

import { useState, type ReactNode } from "react";

import { mintShaped, summonIntent } from "../honesty";

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

/**
 * A launch block, drawn as one square per account paid in it.
 *
 * The site's whole claim is "three squares against twelve", and this is the one
 * form of it a reader can check with their eyes before reading a number. A bar
 * chart of the same data would be a chart of two probabilities; this is a
 * picture of the thing itself — the accounts that were paid in the first block
 * of a coin's life, which is what the recipient count *is*.
 *
 * Pure SVG, no library, and `aria-hidden` with the sentence carried in text
 * beside it: a screen reader gets "10 to 13 recipients", not twelve rectangles.
 */
export function LaunchBlock({
  n,
  tone = "quiet",
}: {
  n: number;
  tone?: "quiet" | "signal";
}) {
  // Clamped so a bad figure cannot draw a thousand squares into the layout.
  // The cap is above every band the data has, so it never truncates a real one.
  const squares = Math.max(0, Math.min(n, 24));
  const size = 14;
  const gap = 4;
  const perRow = 8;
  const rows = Math.ceil(squares / perRow) || 1;
  const fill =
    tone === "signal" ? "var(--color-signal)" : "var(--color-edge)";
  return (
    <svg
      className="block"
      width={perRow * size + (perRow - 1) * gap}
      height={rows * size + (rows - 1) * gap}
      viewBox={`0 0 ${perRow * size + (perRow - 1) * gap} ${rows * size + (rows - 1) * gap}`}
      aria-hidden="true"
      focusable="false"
    >
      {Array.from({ length: squares }, (_, i) => (
        <rect
          key={i}
          x={(i % perRow) * (size + gap)}
          y={Math.floor(i / perRow) * (size + gap)}
          width={size}
          height={size}
          rx={2}
          fill={fill}
          shapeRendering="crispEdges"
        />
      ))}
    </svg>
  );
}

/**
 * A reply from the bot, shown as a record rather than as a quotation.
 *
 * These are the product, and they are the shape people already screenshot.
 * Quotation marks would frame one as an opinion.
 */
export function Receipt({
  children,
  label = "An actual reply",
}: {
  children: ReactNode;
  label?: string;
}) {
  return (
    <Card className="scroll-x">
      <div className="mb-3 font-mono text-xs tracking-widest text-[var(--color-faint)] uppercase">
        {label}
      </div>
      <pre className="receipt text-[var(--color-text)]">{children}</pre>
    </Card>
  );
}

/** One step of a numbered sequence. */
export interface Step {
  readonly what: ReactNode;
  /** The appointment this step happens at, when it has one. */
  readonly when?: string;
}

/**
 * A numbered sequence, for a process a reader has to follow in order.
 *
 * An ordered list rather than a set of cards, because the order is the content:
 * claiming a prize out of sequence is how a winner loses one. `<ol>` so that a
 * screen reader announces "3 of 5" without being told to.
 */
export function Steps({ steps }: { steps: readonly Step[] }) {
  return (
    <ol className="space-y-4">
      {steps.map((step, i) => (
        <li key={i} className="flex gap-4">
          <span
            className="tnum mt-0.5 shrink-0 font-mono text-xs text-[var(--color-signal-dim)]"
            aria-hidden="true"
          >
            {String(i + 1).padStart(2, "0")}
          </span>
          <div className="min-w-0">
            <div className="text-[var(--color-dim)]">{step.what}</div>
            {step.when && (
              <div className="mt-1 font-mono text-xs text-[var(--color-faint)]">
                {step.when}
              </div>
            )}
          </div>
        </li>
      ))}
    </ol>
  );
}

/**
 * A link styled as the page's one primary action.
 *
 * Takes an `href` that may be `null`, and renders `fallback` instead when it is.
 * That signature is the point: every URL on this site comes out of `honesty.ts`
 * and every one of those can refuse. A call site that could not express "the
 * link is not available" would push callers back to interpolating a string.
 */
export function Cta({
  href,
  children,
  fallback,
}: {
  href: string | null;
  children: ReactNode;
  fallback?: ReactNode;
}) {
  if (href === null) return <>{fallback ?? null}</>;
  return (
    <a
      href={href}
      target="_blank"
      rel="noopener noreferrer"
      className="inline-flex items-center gap-2 rounded-md bg-[var(--color-signal)] px-4 py-2.5 font-medium text-[var(--color-ink)] transition-opacity hover:opacity-90"
    >
      {children}
    </a>
  );
}

/**
 * Paste a mint, get a prefilled summons.
 *
 * **The site never calls anything.** This builds an `x.com/intent/post` link and
 * hands it to the reader; the post is theirs, sent from their account, and the
 * bot answers it in the thread like any other summons. There is no form, no
 * endpoint, and nothing for this page to get wrong about somebody's coin.
 *
 * The address is validated with the same 32–44 base58 rule the bot's own parser
 * uses, so a reader learns their paste is unreadable *here*, before it costs
 * them a public post that gets no answer.
 *
 * Renders an honest refusal when the account's handle is not configured — see
 * [`account`] in `honesty.ts`. A summon button that posts `@undefined` would be
 * worse than no button.
 */
export function Summon({ handle }: { handle: string | null }) {
  const [mint, setMint] = useState("");
  const typed = mint.trim().length > 0;
  const shaped = mintShaped(mint);
  const href = handle === null ? null : summonIntent(handle, mint);

  if (handle === null) {
    return (
      <Nothing
        what="The account is not announced here yet."
        why="This page will not print a handle it cannot verify — a wrong one sends you to somebody else's profile. It appears here once the operator sets it."
      />
    );
  }

  return (
    <Card>
      <label
        htmlFor="mint"
        className="mb-3 block font-mono text-xs tracking-widest text-[var(--color-faint)] uppercase"
      >
        Ask it about a coin
      </label>
      <input
        id="mint"
        value={mint}
        onChange={(e) => setMint(e.target.value)}
        placeholder="Paste a mint address"
        spellCheck={false}
        autoComplete="off"
        // The address is the only thing this field accepts, and a phone
        // keyboard that autocapitalises breaks base58 silently.
        autoCapitalize="off"
        className="tnum w-full rounded-md border border-[var(--color-line)] bg-[var(--color-ink)] px-3 py-2.5 font-mono text-sm text-[var(--color-text)] placeholder:text-[var(--color-faint)]"
      />
      <div className="mt-4 flex flex-wrap items-center gap-3">
        <Cta href={href}>Summon @{handle} →</Cta>
        <span className="text-xs text-[var(--color-faint)]">
          {!typed
            ? "Opens X with the post written. You send it."
            : shaped
              ? "Opens X with the post written. You send it."
              : "That is not shaped like a Solana address, so the bot would not read it."}
        </span>
      </div>
    </Card>
  );
}
