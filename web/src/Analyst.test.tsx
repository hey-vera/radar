// SPDX-License-Identifier: Apache-2.0
//! That the analyst page distinguishes the states that matter.
//!
//! Three of them look similar and mean different things, and a page that blurred
//! any pair would be worse than no page: an account that has never run, an
//! account running and answering nobody, and an account answering people while
//! publishing nothing.

import { render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Analyst } from "./Analyst";

function respond(body: unknown) {
  vi.stubGlobal(
    "fetch",
    vi.fn(() =>
      Promise.resolve(
        new Response(JSON.stringify(body), {
          status: 200,
          headers: { "content-type": "application/json" },
        }),
      ),
    ),
  );
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("the analyst page", () => {
  it("says plainly when the analyst has never run", async () => {
    // An ordinary configuration, not a failure -- and never rendered as
    // reassurance, which is rule 9 in the interface.
    respond({
      log: "data/analyst/replies.jsonl",
      running: false,
      answered: 0,
      published: 0,
      replies: [],
    });
    render(<Analyst />);
    await waitFor(() => {
      expect(screen.getByText(/has not run on this instance/)).toBeTruthy();
    });
  });

  it("separates what was answered from what was published", async () => {
    // The gap between the two is the number worth watching: a publisher that is
    // down all night fills the log and tells nobody anything, and a page
    // reporting one number would show a busy account.
    respond({
      log: "l",
      running: true,
      answered: 3,
      published: 1,
      replies: [
        {
          at: 1_788_000_000,
          mention_id: "m1",
          summoner: "alice",
          mint: "MintOne",
          read_at_slot: 444_298_091,
          fact_sheet: "round trip: 850 bps",
          reply: "the round trip is 850 bps",
          fellback: null,
          reply_id: "r1",
        },
        {
          at: 1_787_999_000,
          mention_id: "m2",
          summoner: "bob",
          mint: null,
          read_at_slot: null,
          fact_sheet: "",
          reply: "give me the contract address",
          fellback: "NoProvider",
          reply_id: null,
        },
      ],
    });
    render(<Analyst />);
    await waitFor(() => {
      expect(screen.getByText("3")).toBeTruthy();
    });
    expect(screen.getByText("1")).toBeTruthy();
    expect(screen.getByText("published")).toBeTruthy();
    expect(screen.getByText("not published")).toBeTruthy();
    // The fallback reason is never hidden: one that is invisible is one nobody
    // investigates, and it is the early warning that the voice pass is drifting.
    expect(screen.getByText(/NoProvider/)).toBeTruthy();
  });

  it("names the state where everything was answered and nothing was posted", async () => {
    // This is what a dry run looks like, and it is also what a broken
    // credential looks like. Naming it beats leaving it to arithmetic.
    respond({
      log: "l",
      running: true,
      answered: 4,
      published: 0,
      replies: [],
    });
    render(<Analyst />);
    await waitFor(() => {
      expect(screen.getByText(/nothing has been posted/)).toBeTruthy();
    });
  });

  it("shows a read failure rather than an account that said nothing", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(() => Promise.resolve(new Response("nope", { status: 500 }))),
    );
    render(<Analyst />);
    await waitFor(() => {
      expect(screen.getByText(/cannot read the reply log/)).toBeTruthy();
    });
  });
});
