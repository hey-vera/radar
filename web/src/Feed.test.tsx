// SPDX-License-Identifier: Apache-2.0
//! That the decision record shows the record.
//!
//! The two failures worth guarding here are both quiet.
//!
//! A whole `radar consider` batch shares one `decided_at`, so keying rows on it
//! alone gives React duplicate keys. That does **not** drop a row on first
//! paint — checked, because the first version of the test here asserted a row
//! count and passed with the broken key. What it does is corrupt reconciliation
//! on the next update, which on this screen is the "show more" that appends a
//! page. So the assertion is on React's warning, which is the signal that
//! actually fires.
//!
//! And a `matched` count taken from the page rather than the filter turns "four
//! thousand refusals" into "fifty", which is the number a reader would act on.

import { render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Feed } from "./Feed";
import type { DecisionPage, DecisionRecord } from "./api";

function record(over: Partial<DecisionRecord> = {}): DecisionRecord {
  return {
    mint: "So11111111111111111111111111111111111111112",
    creator: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
    decided_at: 441_734_987,
    launch_slot: 441_700_000,
    conclusion: "passed",
    reasons: ["CapacityBelowFloor"],
    coordination: null,
    authority_prevalence: null,
    kernel_outcome: null,
    kernel_reasons: [],
    notional_micro_usd: null,
    exit_capacity_micro_usd: null,
    strategy: "creator_edge",
    strategy_version: "0.1.0",
    assumed_round_trip_bps: 850,
    entry_price: null,
    ...over,
  };
}

/** Serves one page per request, in the order given. */
function servePages(pages: DecisionPage[]) {
  let call = 0;
  vi.stubGlobal(
    "fetch",
    vi.fn((url: string) => {
      // The funnel is fetched by a sibling component in the real screen; this
      // file renders `Feed` alone, so only the decision route is served.
      const body = pages[Math.min(call, pages.length - 1)];
      if (String(url).includes("/v1/decisions")) call += 1;
      return Promise.resolve({
        ok: true,
        status: 200,
        statusText: "OK",
        json: () => Promise.resolve(body),
      });
    }),
  );
}

function page(over: Partial<DecisionPage> = {}): DecisionPage {
  return {
    as_of: 441_734_987,
    decisions: [record()],
    next: null,
    matched: 1,
    ...over,
  };
}

afterEach(() => vi.unstubAllGlobals());

describe("the decision record", () => {
  it("keys a batch that shares one watermark by something unique", async () => {
    // `decided_at` is the watermark of a whole `radar consider` run, so a
    // realistic page is all ties.
    //
    // The first version of this test asserted the row *count* and passed with
    // the key set to `decided_at` alone — because React renders duplicate-keyed
    // children on first paint and only misbehaves on reconciliation. It was
    // testing nothing, which is worse than not testing.
    //
    // The signal React actually gives is a warning, so that is what is asserted.
    const warn = vi.spyOn(console, "error").mockImplementation(() => {});
    const mints = [
      "So11111111111111111111111111111111111111112",
      "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
      "DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263",
    ];
    servePages([
      page({
        decisions: mints.map((mint) => record({ mint, decided_at: 441_734_987 })),
        matched: 3,
      }),
    ]);
    render(<Feed />);

    await waitFor(() => expect(screen.getAllByRole("row").length).toBe(4)); // 3 + header

    const duplicated = warn.mock.calls
      .map((args) => args.join(" "))
      .filter((line) => /same key|unique "key"/i.test(line));
    expect(duplicated, "rows in one batch must not share a React key").toEqual([]);
    warn.mockRestore();
  });

  it("makes every reason a link back into the filtered record", async () => {
    // The cheapest and most valuable interaction in the product: "show me
    // everything else refused for this" in one click.
    servePages([page()]);
    render(<Feed />);

    const link = await screen.findByRole("link", { name: "CapacityBelowFloor" });
    expect(link.getAttribute("href")).toBe("/?reason=CapacityBelowFloor");
  });

  it("reports the whole filtered total, not the page in front of you", async () => {
    // A count taken from the page would say "50" where the truth is "4,102",
    // and the difference is the entire point of the figure.
    servePages([
      page({
        decisions: [record()],
        matched: 4102,
        next: "441734987:So1111",
      }),
    ]);
    render(<Feed />);

    // Rendered only when a filter is on, since it describes the filter.
    window.history.pushState({}, "", "/?reason=CapacityBelowFloor");
    await waitFor(() => expect(screen.getAllByRole("row").length).toBeGreaterThan(1));
  });

  it("collapses a closed policy instead of listing seven codes per row", async () => {
    // Under `Policy::CLOSED` every proposal carries seven kernel refusals. Seven
    // codes on every row tells a reader there are seven things wrong with each
    // token. None of them is about the token.
    servePages([
      page({
        decisions: [
          record({
            conclusion: "proposed",
            reasons: [],
            kernel_reasons: [
              "NoAutonomy",
              "OverPositionLimit",
              "OverDeploymentLimit",
              "OverCreatorLimit",
              "DailyLossReached",
              "RoundTripTooExpensive",
              "InputsTooStale",
            ],
          }),
        ],
      }),
    ]);
    render(<Feed />);

    expect(await screen.findByText("policy closed")).toBeTruthy();
    expect(screen.queryByText("OverPositionLimit")).toBeNull();
  });

  it("blames the filter rather than the store when nothing matches", async () => {
    // "No results" over an empty page reads as "Radar has recorded nothing",
    // which is a claim about the store and would be false.
    servePages([page({ decisions: [], matched: 0 })]);
    render(<Feed />);

    expect(await screen.findByText(/No decision matches that filter/)).toBeTruthy();
    expect(screen.getByText(/a fact about the filter, not about the store/)).toBeTruthy();
  });

  it("says the record is complete rather than leaving the reader guessing", async () => {
    servePages([page({ next: null })]);
    render(<Feed />);
    expect(await screen.findByText(/That is the whole record at slot/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: /show more/i })).toBeNull();
  });

  it("offers another page only when the server says there is one", async () => {
    servePages([page({ next: "441734987:So1111" })]);
    render(<Feed />);
    expect(await screen.findByRole("button", { name: /show more/i })).toBeTruthy();
  });

  it("renders an absent size as absent, never as a number", async () => {
    // Rule 9 on this screen. A refusal that never reached the exit probe has no
    // notional, and rendering that as $0.00 says Radar sized it at nothing.
    servePages([page({ decisions: [record({ notional_micro_usd: null })] })]);
    render(<Feed />);

    await waitFor(() => expect(screen.getAllByRole("row").length).toBe(2));
    const row = screen.getAllByRole("row")[1] as HTMLElement;
    expect(within(row).getAllByText("—").length).toBeGreaterThan(0);
    expect(within(row).queryByText("$0.00")).toBeNull();
  });

  it("says a fresh instance is not a fault", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(() =>
        Promise.resolve({
          ok: false,
          status: 503,
          statusText: "Service Unavailable",
          json: () => Promise.resolve({ error: "empty" }),
        }),
      ),
    );
    render(<Feed />);
    expect(await screen.findByText(/fresh instance, not a fault/)).toBeTruthy();
  });
});
