// SPDX-License-Identifier: Apache-2.0
//! That the scoreboard does not overstate itself.
//!
//! The arithmetic is tested next door. This is the other half, and it is the
//! half that actually broke: every number on this page was correct, and the
//! sentence under them said they were net of a round trip that had not been
//! subtracted. A unit test of `median` passes over that forever.
//!
//! So these assert what the *page* claims, which is the thing that was wrong.
//! Each one is written from the reader's side — what a person would take away —
//! rather than from the component's.

import { render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Scoreboard } from "./Scoreboard";
import type { Scoreboard as Board } from "./api";

/** A cohort whose median is a round number, so the assertions read plainly. */
function cohort(scored: number, returns: number[]) {
  return { scored, returns_bps: returns };
}

/**
 * Forty proposals with a median of exactly +2000 bps.
 *
 * Above `MIN_COHORT`, so the page renders figures rather than refusing to. The
 * median is deliberately *positive and large*: the defect this file exists for
 * flattered the numbers, and a cohort that is already negative cannot show the
 * difference between gross and net at a glance.
 */
const board: Board = {
  decisions: 4374,
  scored: 1092,
  proposed: cohort(40, Array.from({ length: 40 }, () => 2000)),
  refused: cohort(606, Array.from({ length: 606 }, () => -500)),
  cost_bps: 850,
};

function serve(body: Board) {
  vi.stubGlobal(
    "fetch",
    vi.fn(() =>
      Promise.resolve({
        ok: true,
        status: 200,
        statusText: "OK",
        json: () => Promise.resolve(body),
      }),
    ),
  );
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("the scoreboard's claims", () => {
  it("shows gross and net as separate figures, and the net one is smaller", async () => {
    // The original defect in one assertion. The page displayed +20.0% and told
    // the reader it was already net of 850 bps. Both must now be present, and
    // they must differ by exactly the round trip.
    serve(board);
    render(<Scoreboard />);

    const gross = await screen.findByText("median return, gross");
    const grossRow = gross.closest("tr");
    expect(grossRow).not.toBeNull();
    expect(within(grossRow as HTMLElement).getByText("+20.0%")).toBeTruthy();

    const net = screen.getByText("median return, net of 850 bps");
    const netRow = net.closest("tr");
    expect(netRow).not.toBeNull();
    // 2000 - 850 = 1150 bps.
    expect(within(netRow as HTMLElement).getByText("+11.5%")).toBeTruthy();
  });

  it("subtracts the round trip everywhere it says net, not just in one place", async () => {
    // The general form of the bug. Two blocks on this page report a net figure —
    // the selection's table and the refusals — and the original defect was a
    // label applied to an unsubtracted number. Both must have done the
    // arithmetic, and neither may still carry its own gross value.
    serve(board);
    render(<Scoreboard />);

    const labels = await screen.findAllByText(/net of 850 bps/);
    expect(labels.length).toBe(2);

    // The selection: +2000 gross, so +1150 net. Its row must not still show
    // +20.0%.
    const selection = labels[0]?.closest("tr") as HTMLElement;
    expect(within(selection).getByText("+11.5%")).toBeTruthy();
    expect(within(selection).queryByText("+20.0%")).toBeNull();

    // The refusals: -500 gross, so -1350 net. A cohort that is negative either
    // way is where a missing subtraction is least visible, which is why it is
    // asserted separately.
    const refused = labels[1]?.closest("div") as HTMLElement;
    expect(within(refused).getByText("-13.5%")).toBeTruthy();
    expect(within(refused).queryByText("-5.0%")).toBeNull();
  });

  it("says nothing was traded, above the numbers rather than below them", async () => {
    // The page's own rule is that a number is read and a caveat is not. A test
    // that only checked the text was present would pass with it in a footnote,
    // so this checks the order it appears in the document.
    serve(board);
    render(<Scoreboard />);

    const warning = await screen.findByText(/Nothing here was traded/i);
    const figure = screen.getByText("median return, gross");
    // `DOCUMENT_POSITION_FOLLOWING` means the figure comes after the warning.
    expect(
      warning.compareDocumentPosition(figure) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });

  it("does not present the refusals as a control", async () => {
    // 0014 found the comparison unusable: every scoreable refusal is
    // `CapacityBelowFloor`, so the cohort is entirely tokens Radar measured and
    // could not sell. The page must say so where the refusals are shown, not
    // somewhere else.
    serve(board);
    render(<Scoreboard />);

    expect(await screen.findByText(/not a control/i)).toBeTruthy();
    expect(screen.getByText(/CapacityBelowFloor/)).toBeTruthy();

    // And the two cohorts must not share a table, because two rows side by side
    // is an invitation to subtract them.
    const grossRow = screen
      .getByText("median return, gross")
      .closest("table") as HTMLElement;
    expect(within(grossRow).queryByText(/not a control/i)).toBeNull();
  });

  it("refuses to show figures at all below the cohort floor", async () => {
    // Unchanged behaviour, asserted because the rewrite could have lost it: a
    // median over twenty rows has the shape of a finding and the content of
    // noise.
    serve({ ...board, proposed: cohort(11, Array.from({ length: 11 }, () => 2000)) });
    render(<Scoreboard />);

    expect(await screen.findByText(/Not enough data/i)).toBeTruthy();
    expect(screen.queryByText("median return, gross")).toBeNull();
  });

  it("reports an empty cohort as absent rather than as zero", async () => {
    // Rule 9 on this screen. A cohort with nothing in it has no median, and
    // rendering that as 0.0% prints "broke even" for a measurement nobody took.
    serve({ ...board, proposed: cohort(40, []) });
    render(<Scoreboard />);

    const gross = await screen.findByText("median return, gross");
    const row = gross.closest("tr") as HTMLElement;
    expect(within(row).getByText("—")).toBeTruthy();
    expect(within(row).queryByText("+0.0%")).toBeNull();
    expect(within(row).queryByText("0.0%")).toBeNull();
  });
});

describe("the scoreboard when the store cannot answer", () => {
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
    render(<Scoreboard />);
    await waitFor(() =>
      expect(screen.getByText(/recorded nothing yet/i)).toBeTruthy(),
    );
  });
});
