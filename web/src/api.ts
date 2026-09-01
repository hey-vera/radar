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
  decisions: (query: DecisionQuery, signal?: AbortSignal) =>
    get<DecisionPage>(`/v1/decisions${decisionSearch(query)}`, signal),
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
 * `/v1/customer/events` carries the watermark and nothing else — `/v1/events`
 * is an operator route because its payload is the operator's store counts.
 *
 * An `EventSource` that is refused fails **silently**: it retries forever and
 * never calls its listener. The page would go on rendering the funnel it
 * fetched once, with nothing to say it had stopped following the store.
 *
 * That is rule 9 in the interface: a page that cannot see changes must not look
 * like a page where nothing has changed. `onStale` fires on the error so the
 * caller can say so.
 */
export function subscribe(
  onChange: () => void,
  onStale?: (stale: boolean) => void,
): () => void {
  const source = new EventSource("/v1/customer/events");
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
  /**
   * The highest fill price within the most recent price window.
   *
   * `peak_price` is folded from launch and can only widen, so it says nothing
   * about *when* the peak happened — which is why 0020 could not answer whether
   * an exit rule helps. This is the same measurement without the fold.
   *
   * **The window overlaps**: six hours, read hourly, so a peak set five hours
   * ago appears in six consecutive measurements. It is a bounded recent
   * lookback, not the movement since the last checkpoint. Null on most rows,
   * which were written before the column existed.
   */
  window_peak_price: number | null;
  window_trough_price: number | null;
  vwap: number | null;
  graduated_at: number | null;
  held_to_end_bps: number | null;
}

/** One recorded decision. */
export interface DecisionRecord {
  mint: string;
  creator: string;
  /**
   * The watermark the decision was taken as of.
   *
   * **Not unique.** It is the watermark of a whole `radar consider` run, so
   * every decision in that batch carries the same value. Anything keyed on it
   * alone — a React key, a cursor — collapses a batch into one row.
   */
  decided_at: number;
  /** When the token launched, so an age can be shown without a second read. */
  launch_slot: number;
  conclusion: string;
  reasons: string[];
  coordination: string | null;
  authority_prevalence: string | null;
  kernel_outcome: string | null;
  kernel_reasons: string[];
  notional_micro_usd: number | null;
  exit_capacity_micro_usd: number | null;
  /**
   * Which rule decided, and which version of it.
   *
   * On the wire all along and never rendered. A decision taken under thresholds
   * that have since moved must not be silently compared with one taken under
   * today's — the store records these precisely so that comparison can be
   * refused, and an interface that hides them makes it again.
   */
  strategy: string;
  strategy_version: string;
  /** What the round trip was assumed to cost when this was judged. */
  assumed_round_trip_bps: number;
  /**
   * The price the decision was sized against, scaled by `PRICE_SCALE`.
   *
   * Null when no exit was probed, which is every refusal that never reached the
   * paid tier's quote. Null is not zero: a decision with no entry price cannot
   * be scored at all.
   */
  entry_price: number | null;
}

/** A page of the decision record, newest first. */
export interface DecisionPage {
  as_of: number;
  decisions: DecisionRecord[];
  /**
   * The cursor for the following page, or null at the end.
   *
   * Distinct from an empty `decisions` list, which also happens when a filter
   * matches nothing.
   */
  next: string | null;
  /**
   * How many decisions matched the filter in total, across every page.
   *
   * Counted before the cursor is applied, so it describes the filter rather
   * than the page — it is what lets a reader see that a reason accounts for
   * four thousand refusals rather than the fifty in front of them.
   */
  matched: number;
}

/** What to ask the decision record for. */
export interface DecisionQuery {
  after?: string | undefined;
  reason?: string | undefined;
  conclusion?: "proposed" | "passed" | undefined;
  limit?: number | undefined;
}

function decisionSearch(query: DecisionQuery): string {
  const params = new URLSearchParams();
  if (query.after) params.set("after", query.after);
  if (query.reason) params.set("reason", query.reason);
  if (query.conclusion) params.set("conclusion", query.conclusion);
  if (query.limit !== undefined) params.set("limit", String(query.limit));
  const search = params.toString();
  return search ? `?${search}` : "";
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

/** One band of the exit-capacity distribution. */
export interface Band {
  /** Inclusive floor, micro-USD. */
  floor: number;
  /** Exclusive ceiling, micro-USD. Null on the open-ended top band. */
  ceiling: number | null;
  decisions: number;
}

/** The capacity wall: what the venue offers, and what Radar sized into it. */
export interface Capacity {
  as_of: number;
  bands: Band[];
  /** Decisions where a capacity was measured. */
  measured: number;
  /**
   * Decisions where it was not.
   *
   * Reported apart from the bands and never drawn inside them. Rule 9: a
   * capacity that could not be measured means "cannot exit", not "thin" and not
   * zero, and bucketing these into the bottom band would draw a wall of tokens
   * nobody measured.
   */
  unmeasured: number;
  median_capacity: number | null;
  median_notional: number | null;
  round_trip_bps: number;
}

/** One bucket of the return distribution. */
export interface Bucket {
  /** Inclusive floor in bps. Null is open-ended downward. */
  floor: number | null;
  /** Exclusive ceiling in bps. Null is open-ended upward. */
  ceiling: number | null;
  scored: number;
}

/** What the selection returned, as a distribution rather than a median. */
export interface Returns {
  as_of: number;
  /** The distribution, **excluding** the exact zeroes. */
  buckets: Bucket[];
  /**
   * How many scored decisions returned exactly zero.
   *
   * Its own figure, never a bucket. 24-43% of this population ends exactly
   * where it started, so drawn as a bar it would be the tallest thing on the
   * chart and read as a finding about the market — when it is a fact about a
   * venue where most tokens trade a handful of times and stop.
   */
  exactly_zero: number;
  scored: number;
  /** Decisions with no entry price or no later observation. Never flat. */
  unscored: number;
  round_trip_bps: number;
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
  capacity: (signal?: AbortSignal) =>
    get<Capacity>("/v1/evidence/capacity", signal),
  returns: (signal?: AbortSignal) =>
    get<Returns>("/v1/evidence/returns", signal),
};
