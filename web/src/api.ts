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

/**
 * Routes this interface may call as a **customer**.
 *
 * The distinction is not cosmetic. `access::audience_of` classifies every path,
 * and `/v1/store`, `/v1/events` and `/v1/link` are `Audience::Operator` — the
 * operator's debugging surface rather than product. They are reachable today
 * only because no customer authenticator is configured, so every request falls
 * back to the operator check. The day `RADAR_PRIVY_APP_ID` is set they begin
 * refusing customer sessions, and anything built on them stops working for
 * everyone who is not Josh.
 *
 * So they are grouped separately below rather than left to look like product.
 */
export const api = {
  funnel: (signal?: AbortSignal) => get<Funnel>("/v1/funnel", signal),
};

/**
 * Routes that require **operator** identity.
 *
 * Kept apart so a screen built on one is a deliberate choice rather than an
 * accident, and so the seam is already drawn when the customer lane switches on.
 */
export const operator = {
  store: (signal?: AbortSignal) => get<StoreCounts>("/v1/store", signal),
};

/**
 * Subscribes to store changes.
 *
 * Returns a teardown. The browser reconnects a dropped `EventSource` on its
 * own, which is most of why this is server-sent events rather than a socket:
 * the reconnect logic is the part that would otherwise be written badly here.
 *
 * # Why `onStale` exists
 *
 * `/v1/events` is an **operator** route. When the customer lane switches on it
 * will refuse a customer session, and an `EventSource` that is refused fails
 * silently — it retries forever and never calls its listener. The page would go
 * on rendering the funnel it fetched once, with nothing to say it had stopped
 * following the store.
 *
 * That is rule 9 in the interface: a page that cannot see changes must not look
 * like a page where nothing has changed. `onStale` fires on the error so the
 * caller can say so.
 */
export function subscribe(
  onChange: () => void,
  onStale?: (stale: boolean) => void,
): () => void {
  const source = new EventSource("/v1/events");
  source.addEventListener("store", () => {
    onStale?.(false);
    onChange();
  });
  source.addEventListener("open", () => onStale?.(false));
  source.addEventListener("error", () => onStale?.(true));
  return () => source.close();
}

/** How the credential-linking flow is going. */
export type Progress =
  | {
      state: "waiting";
      verification_url: string;
      user_code: string;
      seconds_elapsed: number;
    }
  | { state: "linked" }
  | { state: "failed"; status: string }
  | { state: "idle" };

/** What the model said, and what it was shown. */
export interface Answered {
  text: string;
  citations: string[];
  uncited: boolean;
}

async function send<T>(path: string, body?: unknown): Promise<T> {
  const response = await fetch(path, {
    method: "POST",
    headers: { "content-type": "application/json", accept: "application/json" },
    // `null` rather than `undefined`: strict optional properties treat an
    // explicitly-undefined field as a type error, and `null` is what fetch
    // documents for "no body".
    body: body === undefined ? null : JSON.stringify(body),
  });
  if (!response.ok) {
    let detail = response.statusText;
    try {
      const parsed = (await response.json()) as { error?: string };
      if (parsed.error) detail = parsed.error;
    } catch {
      // A non-JSON body is itself informative: something upstream answered.
    }
    throw new ApiError(response.status, detail);
  }
  return (await response.json()) as T;
}

export const agent = {
  /** Starts a device-authorisation flow, or returns the one already open. */
  link: () => send<Progress>("/v1/link"),
  /** Where the current flow has got to. */
  linkStatus: (signal?: AbortSignal) => get<Progress>("/v1/link", signal),
  /** Asks a question. */
  ask: (question: string) => send<Answered>("/v1/chat", { question }),
};

/** One price measurement of a token. */
export interface Measurement {
  measured_at: number;
  /**
   * Price reads the figures below were computed from. **Not a fill count.**
   *
   * The server renames it on the wire for this reason. It is folded with
   * `saturating_add` across windows that overlap by five of their six hours, so
   * it grows while nothing trades and two measurements are not comparable —
   * LEARNINGS 19, the defect that invalidated the first runs of research 0017
   * and 0018. It was previously rendered in a column headed `fills`, which is
   * the one thing it must not be read as.
   */
  price_reads: number;
  /**
   * The last slot a transfer was observed, or null if none ever was.
   *
   * A `max`, so it cannot be inflated by re-reading. This is what answers
   * whether the token still trades; `price_reads` does not.
   */
  last_transfer_slot: number | null;
  first_price: number | null;
  last_price: number | null;
  peak_price: number | null;
  trough_price: number | null;
  graduated_at: number | null;
  held_to_end_bps: number | null;
}

/** One recorded decision. */
export interface DecisionRecord {
  mint: string;
  creator: string;
  decided_at: number;
  conclusion: string;
  reasons: string[];
  coordination: string | null;
  authority_prevalence: string | null;
  kernel_outcome: string | null;
  kernel_reasons: string[];
  notional_micro_usd: number | null;
  exit_capacity_micro_usd: number | null;
}

/** Everything recorded about one mint. */
export interface TokenEvidence {
  mint: string;
  decisions: DecisionRecord[];
  measurements: Measurement[];
}

/** One cohort's return distribution. */
export interface Cohort {
  scored: number;
  returns_bps: number[];
}

/** Radar's selection against its own refusals. */
export interface Scoreboard {
  decisions: number;
  scored: number;
  proposed: Cohort;
  refused: Cohort;
  cost_bps: number;
}

/** What the server says about itself. */
export interface Health {
  status: string;
  version: string;
  instruments: number;
  watermarkSlot: number | null;
  paidSurface: boolean;
  /**
   * Whether the policy Radar **decides** with could authorise anything.
   *
   * Reported by the server rather than asserted here. This screen used to print
   * "policy closed" as literal text, which is a claim the page could not stop
   * making — the same shape as the four backend instances it was found beside.
   *
   * It scopes to the deciding policy. The signer holds its own (ADR 0008) and
   * can refuse what this one permits, never the reverse.
   */
  policyClosed: boolean;
  agent: { configured: boolean; [k: string]: unknown };
}

export const research = {
  token: (mint: string, signal?: AbortSignal) =>
    get<TokenEvidence>(`/v1/tokens/${encodeURIComponent(mint)}`, signal),
  scoreboard: (signal?: AbortSignal) => get<Scoreboard>("/v1/scoreboard", signal),
  health: (signal?: AbortSignal) => get<Health>("/health", signal),
};
