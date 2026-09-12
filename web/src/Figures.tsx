// SPDX-License-Identifier: Apache-2.0
//! The terminal's honesty primitives, carried as **types** rather than as
//! discipline.
//!
//! This file used to hold five components built for the decision-record
//! pages -- `Figure`, `Bps`, `ReasonList` and the `Group` helper beneath it --
//! and all four were deleted with those pages. `Address` and the two below it
//! earned a place here instead of going with them:
//!
//! - **`Address`** is reused as-is: the terminal's info panel and trade tape
//!   both need a copyable, middle-truncated Solana address, which is exactly
//!   what it already did for a mint.
//! - **`MarketFigure`** and **`Side`** are new, and encode the two honesty
//!   rules the packet states for the market panels: a null price renders as
//!   "unknown" with its reason, never as 0 or a bare dash, and a trade side of
//!   `"unknown"` renders as unknown, never defaulted to a buy.
//!
//! Every one of these exists because the rule it encodes was broken somewhere
//! by a component that rendered a value directly. A rule enforced by
//! remembering is a rule that holds until the next person.

import { useCallback, useState } from "react";
import type { TradeSide } from "./api";

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
  value: string | null | undefined;
  className?: string;
}) {
  const [copied, setCopied] = useState(false);

  const copy = useCallback(() => {
    // `navigator.clipboard` is absent over plain HTTP and in some embedded
    // views. Failing quietly is right here -- the address is still visible in
    // the tooltip, so a reader is not stuck -- but it must not throw.
    void navigator.clipboard
      ?.writeText(value ?? "")
      .then(() => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1200);
      })
      .catch(() => {
        /* The tooltip still carries it. */
      });
  }, [value]);

  // An absent address renders as an absent address.
  //
  // **This was a crash, and it took the whole page with it.** `value` was
  // typed `string`, three call sites pass something that is genuinely
  // optional -- a trade whose trader could not be told from the pool, a token
  // with no creator on record, a holder row read through a field name the
  // server does not send -- and `value.length` on the first of those threw
  // inside render. React unmounted the tree, and a terminal showing a live
  // market became an empty black rectangle whose only symptom was in the
  // console. Observed 2026-09-11.
  //
  // Typing it `string | null | undefined` is the fix rather than a guard at
  // each call site: the optionality is real, so the component that draws an
  // address is where it belongs, and the compiler now refuses a caller that
  // assumed otherwise. "Unknown" rather than a dash or an empty cell, for
  // AGENTS §4 rule 9's reason -- absent is not zero, and a blank looks like a
  // value nobody bothered to show.
  if (value === null || value === undefined || value === "") {
    return (
      <span className={`text-[var(--color-dim)] ${className}`} title="No address was recorded for this row.">
        unknown
      </span>
    );
  }

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
 * A market fact that may be unknown, with the reason it is when it is.
 *
 * A market fact's `null` can mean several different things -- no pool found,
 * no route priced, an unsupported quote mint -- and a trader deciding whether
 * to trust the screen needs to know which. The word is always **"unknown"**,
 * spelled out, never a bare dash and never the number 0: rendering
 * `price ?? 0` here is the exact failure this component exists to make
 * impossible to write by accident.
 */
export function MarketFigure({
  value,
  reason,
  format,
}: {
  value: number | null;
  reason: string | null;
  format: (v: number) => string;
}) {
  if (value === null) {
    return (
      <span
        className="text-[var(--color-absent)]"
        title={reason ?? "Radar did not say why"}
      >
        unknown{reason ? ` — ${reason}` : ""}
      </span>
    );
  }
  return <span className="tabular-nums">{format(value)}</span>;
}

/**
 * A trade's side, exactly as reported -- never defaulted toward a buy.
 *
 * The contract is explicit that a side can be `"unknown"`, and that it "renders
 * as unknown, not as a buy". The obvious wrong version is a component typed
 * `side: "buy" | "sell"` with the caller coercing an unrecognised value on the
 * way in; typing this one over [`TradeSide`] instead makes that coercion a
 * type error at the call site rather than a silent mislabel here.
 */
export function Side({ side }: { side: TradeSide }) {
  if (side === "unknown") {
    return (
      <span className="text-[var(--color-absent)]" title="Radar could not tell which side of the trade this was">
        unknown
      </span>
    );
  }
  const colour = side === "buy" ? "text-[var(--color-gain)]" : "text-[var(--color-loss)]";
  return <span className={colour}>{side}</span>;
}
