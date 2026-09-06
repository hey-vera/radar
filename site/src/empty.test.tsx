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

import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { About } from "./About";
import { Home } from "./Home";
import { Leaderboard } from "./Leaderboard";
import { Pool } from "./Pool";
import { Token } from "./Token";
import { Summon } from "./ui";

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
    // Research 0028, 2026-09-05: 30 bps is the curve. After graduation the
    // chain's own schedule runs 30 / 95 / ... / 5 by market cap, and a page
    // that said "30 bps" without the qualifier would be understating a coin
    // that graduated and kept going by three times.
    const text =
      screen.getByText(/30 basis points of volume/i).closest("p")
        ?.textContent ?? "";
    expect(text).toMatch(/on the bonding curve/);
    expect(text).toMatch(/After graduation/);
    expect(text).toMatch(/95 from there to 1,470 SOL/);
    expect(text).toMatch(/5 above 98,240 SOL/);
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

describe("the tokenomics page before any token exists", () => {
  it("says no token exists, in words, rather than showing a zero", async () => {
    render(<Token />);
    expect(screen.getByText(/No token exists/i)).toBeTruthy();
    expect(screen.getByText(/any address claiming to be this token is not/i))
      .toBeTruthy();
  });

  it("puts no valuation of its own token on the page", () => {
    // ADR 0013 constraint 5 forbids the *bot* from stating the token's price or
    // market capitalisation. A marketing page printing what the bot is
    // forbidden to say would make the constraint decorative, so the same line
    // is held here -- by a test, because this is the page where the pressure to
    // add a number will come from.
    //
    // The check is a dollar figure, not the words. A first version banned
    // "market cap" outright and failed on the fee table, whose rows are keyed
    // on market capitalisation -- of whichever coin is being traded, not of
    // this token. Describing somebody else's fee schedule is not valuing your
    // own token, and a check that cannot tell those apart fires on a correct
    // page. Every figure here is in basis points or SOL, and a dollar sign is
    // how a valuation would arrive.
    const { container } = render(<Token />);
    const text = container.textContent ?? "";
    expect(text).not.toMatch(/\$\s?[\d.]/);
    // And it still says, in words, that the bot will not state the price.
    expect(text).toMatch(/never states the token's price/i);
  });

  it("renders the fee ladder it imports rather than a summary of it", async () => {
    render(<Token />);
    // The row that matters: 95 bps immediately after graduation is where the
    // prize actually comes from, and it is the row a reader is most likely to
    // be surprised by.
    // getAllBy: 95 and 420 each appear in the prose above the table as well as
    // in the row itself, which is the page working rather than a duplicate.
    expect(screen.getAllByText("95").length).toBeGreaterThan(0);
    expect(screen.getAllByText(/420 SOL/).length).toBeGreaterThan(0);
  });
});

describe("no page delivers a verdict", () => {
  // The bot is forbidden from saying a coin is a scam or that it is safe.
  // `forbidden.rs` enforces that on every reply. The site is the same product
  // speaking to the same strangers, and nothing enforced it here at all.
  //
  // Word-bounded, so "safety" and "farmer" do not fire, and applied to rendered
  // text rather than to source, so a word arriving through a fixture is caught
  // too.
  const FORBIDDEN = [
    "scam",
    "rug",
    "fraud",
    "sybil",
    "fake",
    "legit",
    "safe",
    "guaranteed",
  ];

  const pages: readonly [string, () => React.ReactElement][] = [
    ["home", () => <Home />],
    ["leaderboard", () => <Leaderboard />],
    ["pool", () => <Pool />],
    ["token", () => <Token />],
    ["about", () => <About />],
  ];

  for (const [name, page] of pages) {
    it(`${name} says none of them`, async () => {
      const { container } = render(page());
      await waitFor(() => expect(container.textContent).toBeTruthy());
      const text = (container.textContent ?? "").toLowerCase();
      for (const word of FORBIDDEN) {
        expect(
          new RegExp(`\b${word}\b`).test(text),
          `${name} contains the word "${word}"`,
        ).toBe(false);
      }
    });
  }
});

describe("the summon box", () => {
  it("says so plainly when the account's handle is not configured", () => {
    // Production today: no VITE_X_HANDLE, and no file in the repository records
    // the handle as a fact. Rendering a guessed one would send a stranger to
    // somebody else's profile.
    render(<Summon handle={null} />);
    expect(screen.getByText(/not announced here yet/i)).toBeTruthy();
    expect(screen.queryByPlaceholderText(/mint address/i)).toBe(null);
  });

  it("builds a prefilled post once a real address is typed", () => {
    // fireEvent rather than user-event: one controlled input does not justify
    // another devDependency, and the component reads `e.target.value` either
    // way. The comment in ui/index.tsx about not installing a library before a
    // component needs one applies to test libraries too.
    render(<Summon handle="thecabalhunter" />);
    const box = screen.getByPlaceholderText(/mint address/i);

    // Nothing typed: no link, so the button is absent rather than dead.
    expect(screen.queryByRole("link")).toBe(null);

    // Something that is not an address: the reader is told here, before it
    // costs them a public post that gets no answer.
    fireEvent.change(box, { target: { value: "not an address" } });
    expect(screen.getByText(/not shaped like a Solana address/i)).toBeTruthy();
    expect(screen.queryByRole("link")).toBe(null);

    // A real one: the intent link, with the mint encoded into it.
    const mint = "HWvHqvfFVQdLZ1K3kMygpvhivVZEcrzVShgJFgtXpump";
    fireEvent.change(box, { target: { value: mint } });
    const link = screen.getByRole("link");
    expect(link.getAttribute("href")).toBe(
      `https://x.com/intent/post?text=%40thecabalhunter%20${mint}`,
    );
  });
});
