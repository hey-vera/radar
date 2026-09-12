// SPDX-License-Identifier: Apache-2.0
//! One fetch lifecycle, used everywhere the terminal reads the market.
//!
//! `Decisions.tsx` wrote this shape first, inline and un-exported. The
//! terminal needs the same three states in five different panels -- the coin
//! list, the header, the chart, the tape, the holders list -- and copying the
//! `useEffect` five times is how one of the five ends up not aborting its
//! stale request. Pulled out once, used everywhere.

import { useEffect, useState } from "react";
import { ApiError } from "./api";

/** What a fetch is doing right now. */
export type Load<T> =
  | { state: "loading" }
  | { state: "ready"; value: T }
  | { state: "failed"; status: number; detail: string };

/**
 * Runs `fetcher` whenever `deps` changes, aborting the previous request.
 *
 * The abort matters more here than it did in one page: a reader flicking
 * through the coin list fires a new request on every row, and without the
 * abort the *responses* can arrive out of order -- the panel would show
 * mint A's price under mint B's name for however long A's request took
 * longer than B's.
 */
export function useApi<T>(
  fetcher: (signal: AbortSignal) => Promise<T>,
  deps: readonly unknown[],
): Load<T> {
  const [load, setLoad] = useState<Load<T>>({ state: "loading" });

  useEffect(() => {
    const controller = new AbortController();
    setLoad({ state: "loading" });
    fetcher(controller.signal)
      .then((value) => {
        if (!controller.signal.aborted) setLoad({ state: "ready", value });
      })
      .catch((e: unknown) => {
        if (controller.signal.aborted) return;
        if (e instanceof ApiError) {
          setLoad({ state: "failed", status: e.status, detail: e.detail });
        } else {
          setLoad({ state: "failed", status: 0, detail: String(e) });
        }
      });
    return () => controller.abort();
    // `fetcher` is deliberately not a dependency: callers pass a fresh closure
    // every render, and `deps` names the values that actually identify the
    // request (a mint, an interval, a limit). Depending on the closure too
    // would refetch every render regardless of what changed.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);

  return load;
}
