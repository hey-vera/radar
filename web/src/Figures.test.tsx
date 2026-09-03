// SPDX-License-Identifier: Apache-2.0
//! That the honesty primitives cannot be talked into lying.
//!
//! Each of these components exists because the rule it encodes was broken by a
//! component that rendered a number directly. The tests are written the same
//! way: they assert what a reader would take away, and the negative assertions
//! carry the weight — `queryByText("0")` returning null is the whole of rule 9
//! on this screen.

import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Address, Bps, Figure, ReasonList } from "./Figures";
import { partitionReasons } from "./honesty";

describe("Figure", () => {
  it("renders a missing measurement as absent, never as zero", () => {
    // AGENTS.md rule 9 in one assertion. The failure it prevents is a `?? 0`
    // turning "nobody measured this" into "broke even".
    const { container } = render(<Figure value={null} />);
    expect(screen.getByText("—")).toBeTruthy();
    expect(container.textContent).not.toContain("0");
  });

  it("treats undefined the same as null", () => {
    // A field the server did not send and a field it sent as null are the same
    // fact to a reader, and an optional property arrives as `undefined`.
    render(<Figure value={undefined} />);
    expect(screen.getByText("—")).toBeTruthy();
  });

  it("renders a real zero as zero", () => {
    // The other half, and the one a careless fix breaks: a *measured* zero is a
    // measurement. Collapsing it into absent would be the same error pointing
    // the other way.
    render(<Figure value={0} />);
    expect(screen.getByText("0")).toBeTruthy();
  });

  it("does not colour a figure unless asked", () => {
    // The default is plain because the last component to colour by sign was
    // colouring a gross, wrong-entry return as profit and loss. Colour is a
    // claim; it has to be opted into.
    const { container } = render(<Figure value={42} />);
    expect(container.innerHTML).not.toContain("--color-gain");
    expect(container.innerHTML).not.toContain("--color-loss");
  });

  it("colours by sign when the number really is a gain or a loss", () => {
    const gain = render(<Figure value={42} tone="pnl" />);
    expect(gain.container.innerHTML).toContain("--color-gain");
    const loss = render(<Figure value={-42} tone="pnl" />);
    expect(loss.container.innerHTML).toContain("--color-loss");
  });

  it("gives absent its own colour, not gain's or loss's", () => {
    // "Not measured" must not be mistakeable for either end of the pair.
    const { container } = render(<Figure value={null} tone="pnl" />);
    expect(container.innerHTML).toContain("--color-absent");
    expect(container.innerHTML).not.toContain("--color-gain");
    expect(container.innerHTML).not.toContain("--color-loss");
  });
});

describe("Bps", () => {
  it("always emits the sign, so colour is never the only channel", () => {
    // The redundant channel that keeps the screen readable for the ~8% of men
    // with red-green colour vision deficiency. Without it, a positive and a
    // negative return differ only by hue.
    render(<Bps value={1150} />);
    expect(screen.getByText("+11.5%")).toBeTruthy();
  });

  it("leaves zero unsigned", () => {
    // `+0.0%` reads as a gain that rounded away.
    render(<Bps value={0} />);
    expect(screen.getByText("0.0%")).toBeTruthy();
  });

  it("renders a null as absent rather than as 0.0%", () => {
    const { container } = render(<Bps value={null} />);
    expect(screen.getByText("—")).toBeTruthy();
    expect(container.textContent).not.toContain("0.0%");
  });
});

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

describe("partitionReasons", () => {
  it("splits the three kinds the kernel already distinguishes", () => {
    const split = partitionReasons([
      "NoRoute",
      "CreatorNeverGraduated",
      "OverPositionLimit",
    ]);
    expect(split.structural).toEqual(["NoRoute"]);
    expect(split.evidence).toEqual(["CreatorNeverGraduated"]);
    expect(split.policy).toEqual(["OverPositionLimit"]);
  });

  it("shows an unrecognised reason rather than hiding it", () => {
    // The direction that matters. A reason wrongly sorted into `policy` is
    // collapsed into one line and effectively hidden, and findings are the only
    // refusals that say anything about the token being looked at.
    const split = partitionReasons(["SomethingAddedNextYear"]);
    expect(split.evidence).toEqual(["SomethingAddedNextYear"]);
    expect(split.policy).toEqual([]);
  });

  it("preserves the order within each group", () => {
    // The strategy emits reasons worst-first and re-sorting would throw that
    // away.
    const split = partitionReasons(["ExitUnmeasurable", "NoRoute"]);
    expect(split.structural).toEqual(["ExitUnmeasurable", "NoRoute"]);
  });
});

describe("ReasonList", () => {
  it("collapses a closed policy into one fact instead of seven problems", () => {
    // Under `Policy::CLOSED` every limit is zero, so seven refusals fire at
    // once. A reader shown seven items concludes there are seven things wrong
    // with the token. None of them is about the token.
    render(
      <ReasonList
        reasons={[
          "NoAutonomy",
          "OverPositionLimit",
          "OverDeploymentLimit",
          "OverCreatorLimit",
          "DailyLossReached",
          "RoundTripTooExpensive",
          "InputsTooStale",
        ]}
      />,
    );
    expect(screen.getByText("Policy closed.")).toBeTruthy();
    expect(screen.getByText(/7 of these refusals are that one fact/)).toBeTruthy();
    // None of the seven is listed individually.
    expect(screen.queryByText("OverPositionLimit")).toBeNull();
  });

  it("separates a permanent fact about the token from a gap in the evidence", () => {
    // "Radar will never touch this" and "Radar could not tell yet" are
    // different answers and were rendered identically.
    render(<ReasonList reasons={["NoRoute", "CreatorRecordTooOld"]} />);
    expect(screen.getByText(/About this token — permanent/)).toBeTruthy();
    expect(screen.getByText(/About the evidence — may change/)).toBeTruthy();
    expect(screen.getByText("NoRoute")).toBeTruthy();
    expect(screen.getByText("CreatorRecordTooOld")).toBeTruthy();
  });

  it("renders nothing at all when there were no refusals", () => {
    const { container } = render(<ReasonList reasons={[]} />);
    expect(container.textContent).toBe("");
  });

  it("omits a heading for a group with nothing in it", () => {
    // An empty "About this token" heading reads as a finding nobody wrote down.
    render(<ReasonList reasons={["CreatorRecordTooOld"]} />);
    expect(screen.queryByText(/About this token/)).toBeNull();
    expect(screen.getByText(/About the evidence/)).toBeTruthy();
  });
});
