// SPDX-License-Identifier: Apache-2.0
//! That the terminal's honesty primitives cannot be talked into lying.
//!
//! Each of these components exists because the rule it encodes was broken by a
//! component that rendered a value directly. The tests are written the same
//! way: they assert what a reader would take away, and the negative assertions
//! carry the weight — a null price never rendering `0`, and an unknown trade
//! side never rendering `buy`.
//!
//! `Figure`, `Bps`, `ReasonList` and their tests went with the decision-record
//! pages they existed for. `partitionReasons`, the pure logic `ReasonList` was
//! built on, moved to `honesty.test.ts` rather than being deleted with it.

import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Address, MarketFigure, Side } from "./Figures";

describe("Address", () => {
  it("truncates in the middle and keeps the whole value reachable", () => {
    // Truncated without a way to recover it, an address is useless — the only
    // thing anyone does with one is paste it somewhere else.
    const mint = "So11111111111111111111111111111111111111112";
    render(<Address value={mint} />);
    const button = screen.getByRole("button");
    expect(button.textContent).toContain("…");
    expect(button.textContent!.length).toBeLessThan(mint.length);
    expect(button.getAttribute("title")).toBe(mint);
  });

  it("leaves a short value alone rather than truncating it to nothing", () => {
    render(<Address value="abc" />);
    expect(screen.getByRole("button").textContent).toBe("abc");
  });
});

describe("MarketFigure", () => {
  it("renders a null price as the word unknown, never as 0", () => {
    // The packet's rule verbatim: "A null price renders as 'unknown', never as
    // 0 or — without explanation."
    const { container } = render(
      <MarketFigure value={null} reason="no pool found" format={(v) => `$${v}`} />,
    );
    expect(screen.getByText(/unknown/)).toBeTruthy();
    expect(container.textContent).not.toContain("0");
    expect(container.textContent).not.toBe("—");
  });

  it("carries the reason, not just the word unknown", () => {
    render(
      <MarketFigure value={null} reason="no route priced" format={(v) => `$${v}`} />,
    );
    expect(screen.getByText(/unknown — no route priced/)).toBeTruthy();
  });

  it("still says unknown when there is no reason to give", () => {
    // The rule allows "unknown" alone; it does not allow a silent dash.
    render(<MarketFigure value={null} reason={null} format={(v) => `$${v}`} />);
    expect(screen.getByText("unknown")).toBeTruthy();
  });

  it("formats a real measured value, including a real zero", () => {
    // The other half of rule 9, pointed the other way: a *measured* zero
    // price is a fact, not an absence, and must print as one.
    render(<MarketFigure value={0} reason={null} format={(v) => `$${v.toFixed(2)}`} />);
    expect(screen.getByText("$0.00")).toBeTruthy();
    expect(screen.queryByText(/unknown/)).toBeNull();
  });
});

describe("Side", () => {
  it("renders unknown as unknown, never defaulting to a buy", () => {
    // The packet's rule verbatim: "A trade whose side is unknown renders as
    // unknown, not as a buy." The obvious wrong version is `side ?? "buy"`
    // upstream of this component; typing over `TradeSide` makes that a type
    // error rather than a silent mislabel.
    render(<Side side="unknown" />);
    expect(screen.getByText("unknown")).toBeTruthy();
    expect(screen.queryByText("buy")).toBeNull();
  });

  it("renders a buy and a sell as themselves, in different colours", () => {
    const buy = render(<Side side="buy" />);
    const sell = render(<Side side="sell" />);
    expect(buy.getByText("buy").className).toContain("--color-gain");
    expect(sell.getByText("sell").className).toContain("--color-loss");
  });

  it("gives unknown a colour distinct from both buy and sell", () => {
    render(<Side side="unknown" />);
    const el = screen.getByText("unknown");
    expect(el.className).not.toContain("--color-gain");
    expect(el.className).not.toContain("--color-loss");
    expect(el.className).toContain("--color-absent");
  });
});
