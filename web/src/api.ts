// SPDX-License-Identifier: Apache-2.0
//! The shapes `radar-serve` returns.
//!
//! Hand-written rather than generated, and deliberately narrow: only the fields
//! the interface actually reads. A type that claimed to mirror the whole server
//! response would drift silently, and the drift would be invisible until a
//! field nobody rendered turned out to matter.

/** One stage of the recorded funnel. */
export interface Stage {
  name: string;
  count: number;
  detail: string;
}

/** How often a reason was given. */
export interface ReasonCount {
  reason: string;
  count: number;
}

/** What Radar has decided, and where it stopped. */
export interface Funnel {
  as_of: number;
  stages: Stage[];
  reasons: ReasonCount[];
  policy_closed: boolean;
}

/** How many rows each table holds. */
export interface StoreCounts {
  launches: number;
  graduations: number;
  outcomes: number;
  decisions: number;
}

/**
 * A failed fetch, carrying enough to say what went wrong.
 *
 * A page that renders "error" and nothing else sends someone to the logs. The
 * status is usually the whole answer: 503 means the store is empty, which is a
 * normal state for a fresh instance rather than a fault.
 */
export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly detail: string,
  ) {
    super(`${status}: ${detail}`);
    this.name = "ApiError";
  }
}

async function get<T>(path: string, signal?: AbortSignal): Promise<T> {
  const response = await fetch(path, {
    signal: signal ?? null,
    headers: { accept: "application/json" },
  });
  if (!response.ok) {
    // The server sends `{"error": "..."}` for every failure it authors, so this
    // usually says something useful. When it does not, the status still does.
    let detail = response.statusText;
    try {
      const body = (await response.json()) as { error?: string };
      if (body.error) detail = body.error;
    } catch {
      // A non-JSON body is itself informative: something upstream answered.
    }
    throw new ApiError(response.status, detail);
  }
  return (await response.json()) as T;
}

export const api = {
  funnel: (signal?: AbortSignal) => get<Funnel>("/v1/funnel", signal),
  store: (signal?: AbortSignal) => get<StoreCounts>("/v1/store", signal),
};

/**
 * Subscribes to store changes.
 *
 * Returns a teardown. The browser reconnects a dropped `EventSource` on its
 * own, which is most of why this is server-sent events rather than a socket:
 * the reconnect logic is the part that would otherwise be written badly here.
 */
export function subscribe(onChange: () => void): () => void {
  const source = new EventSource("/v1/events");
  source.addEventListener("store", onChange);
  return () => source.close();
}
