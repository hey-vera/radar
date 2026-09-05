// SPDX-License-Identifier: Apache-2.0
//! What the pages say when they have nothing to say.
//!
//! **This is the most important test file on the site**, because these are the
//! two states the whole thing ships in today and they will hold for as long as
//! the X account and the token take.
//!
//! Both have a wrong version that looks completely right. An empty table says a
//! week ran and nobody engaged. `0.00 SOL` says a contest exists and pays
//! nothing. Neither is true, both are what a reader takes away, and neither
//! would fail a test that only checked the page rendered.

import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { Leaderboard } from "./Leaderboard";
import { Pool } from "./Pool";

/** No endpoint, which is exactly production today. */
function withNoServer() {
  vi.stubGlobal(
    "fetch",
    vi.fn(() => Promise.reject(new Error("no server"))),
  );
}

beforeEach(withNoServer);
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe("the leaderboard before any week has run", () => {
  it("says no week has run, in words", async () => {
    render(<Leaderboard />);
    await waitFor(() => {
      expect(screen.getByText(/No week has run yet/i)).toBeTruthy();
    });
    expect(screen.getByText(/has not answered anyone/i)).toBeTruthy();
  });

  it("renders no table at all rather than an empty one", async () => {
    // An empty table is a claim about the product's reception. The rule beside
    // it is still shown -- somebody arriving early should be able to read how
    // it will work -- but the ranking itself is absent, not blank.
    const { container } = render(<Leaderboard />);
    await waitFor(() => {
      expect(screen.getByText(/No week has run yet/i)).toBeTruthy();
    });
    expect(container.querySelector("tbody")).toBeNull();
  });

  it("still publishes the rule, so the contest is legible before it starts", async () => {
    render(<Leaderboard />);
    await waitFor(() => {
      expect(screen.getByText(/3 × reposts/)).toBeTruthy();
    });
    // `getAllBy`, because the page says this twice on purpose: once in the
    // introduction and once in the rule. Somebody who skims one should still
    // meet it.
    expect(screen.getAllByText(/Entry is free/i).length).toBeGreaterThan(0);
  });
});

describe("the prize pool before a token exists", () => {
  it("says there is no token, and shows no balance", async () => {
    render(<Pool />);
    await waitFor(() => {
      expect(screen.getByText(/There is no token yet/i)).toBeTruthy();
    });
  });

  it("never renders a zero balance", async () => {
    // The assertion this file exists for. `0.00 SOL` is the obvious thing to
    // render from `lamports ?? 0`, it looks completely fine, and it tells a
    // stranger that a contest exists and is empty.
    //
    // Re-apply the bug by replacing the `noToken` branch with the balance
    // branch and this fails while every other test here still passes.
    const { container } = render(<Pool />);
    await waitFor(() => {
      expect(screen.getByText(/There is no token yet/i)).toBeTruthy();
    });
    const text = container.textContent ?? "";
    expect(text).not.toMatch(/0\.0000\s*SOL/);
    expect(text).not.toMatch(/\b0\.00\b/);
  });

  it("states the economics whether or not there is a pool to state them about", async () => {
    // ADR 0013's constraints are the product rather than the small print, and
    // they are as true before the launch as after it.
    render(<Pool />);
    await waitFor(() => {
      expect(screen.getByText(/30 basis points of volume/i)).toBeTruthy();
    });
    expect(screen.getByText(/operator holds zero tokens/i)).toBeTruthy();
    expect(screen.getByText(/No dev buy/i)).toBeTruthy();
  });

  it("does the arithmetic on the fee it states", async () => {
    // 30 bps is 0.30%, so $10,000 of weekly volume is $30 and $100,000 is
    // $300. The page said $3 and $30 until 2026-09-05, having read the rate as
    // 0.03%, and three documents said the same. A page that understates the
    // token's own economics by 10x is wrong in the flattering-to-nobody
    // direction, and nothing here noticed until somebody multiplied.
    //
    // Re-apply the bug by putting `$3` back and the first assertion fails.
    const { container } = render(<Pool />);
    await waitFor(() => {
      expect(screen.getByText(/30 basis points of volume/i)).toBeTruthy();
    });
    const text = container.textContent ?? "";
    expect(text).toMatch(/\$30;/);
    expect(text).toMatch(/\$300\./);
    expect(text).not.toMatch(/\$3;/);
  });
});
