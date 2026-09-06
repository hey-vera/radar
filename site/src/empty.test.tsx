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
import { History } from "./History";
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
    // The account went live 2026-09-06 and the page said it was not live for
    // the rest of that day. An empty leaderboard is now "no week has closed",
    // which is a different fact and the true one.
    expect(screen.getByText(/live and answering/i)).toBeTruthy();
    expect(screen.queryByText(/is not live/i)).toBeNull();
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
      expect(screen.getByText(/3 × reposters/)).toBeTruthy();
    });
    // `getAllBy`, because the page says this twice on purpose: once in the
    // introduction and once in the rule. Somebody who skims one should still
    // meet it.
    expect(screen.getAllByText(/Entry is free/i).length).toBeGreaterThan(0);
    // The sentence the rule turns on. Quoters, not quotes -- one account can
    // quote without limit, and a reader has to be told that to check the
    // published numbers against each other.
    expect(screen.getByText(/Accounts, not actions/i)).toBeTruthy();
    // And the honest limit, stated rather than implied.
    expect(screen.getByText(/does not make buying impossible/i)).toBeTruthy();
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
    expect(
      screen.getByText(/any address claiming to be this token is not/i),
    ).toBeTruthy();
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

describe("a week that ran names its entrants the way the record can", () => {
  /** A server that answers the leaderboard with one week and two entries. */
  function withLeaderboard() {
    vi.stubGlobal(
      "fetch",
      vi.fn((url: string) =>
        String(url).includes("/leaderboard")
          ? Promise.resolve({
              ok: true,
              json: () =>
                Promise.resolve({
                  week: "2957",
                  opens_at: "2026-08-31T00:00:00Z",
                  closes_at: "2026-09-07T00:00:00Z",
                  measured_at: "2026-09-07T00:01:00Z",
                  answered: 2,
                  published: 2,
                  entries: [
                    {
                      rank: 1,
                      summoner: "1889496824328880128",
                      handle: "somebody",
                      mint: "So11111111111111111111111111111111111111112",
                      reply_url: "https://x.com/i/web/status/1",
                      score: 12,
                    },
                    {
                      rank: 2,
                      summoner: "2005812292693483520",
                      handle: null,
                      mint: null,
                      reply_url: null,
                      score: 3,
                    },
                  ],
                }),
            })
          : Promise.reject(new Error("no server")),
      ),
    );
  }

  it("shows the handle and links the id, never the other way round", async () => {
    // Finding S4: `public.rs` has sent `handle` since #162 and `api.ts` never
    // declared the field, so every reader saw `@1889496824328880128` on a live
    // page. Re-apply by rendering `@{e.summoner}` again and the first
    // assertion fails.
    //
    // The link is by id on purpose (S27). A handle can be freed and taken by
    // somebody else, and this page would then point a prize -- or an
    // accusation -- at a stranger. The id cannot be reassigned.
    withLeaderboard();
    render(<Leaderboard />);
    await waitFor(() => {
      expect(screen.getByText("@somebody")).toBeTruthy();
    });
    expect(screen.queryByText("@1889496824328880128")).toBeNull();

    const named = screen.getByText("@somebody").closest("a");
    expect(named?.getAttribute("href")).toBe(
      "https://x.com/i/user/1889496824328880128",
    );
  });

  it("shows a bare id when no handle was read, not an @ in front of a number", async () => {
    // Mid-week nothing has read handles at all, so `null` is the ordinary
    // case rather than an edge one. `@2005812292693483520` reads as a name
    // somebody chose and it is not one.
    withLeaderboard();
    render(<Leaderboard />);
    await waitFor(() => {
      expect(screen.getByText("2005812292693483520")).toBeTruthy();
    });
    expect(screen.queryByText("@2005812292693483520")).toBeNull();
    expect(
      screen
        .getByText("2005812292693483520")
        .closest("a")
        ?.getAttribute("href"),
    ).toBe("https://x.com/i/user/2005812292693483520");
  });
});

describe("a closed week publishes the evidence, not a verdict", () => {
  /** A week where one account quoted thirty times and ten people reposted. */
  function withFarmedWeek() {
    vi.stubGlobal(
      "fetch",
      vi.fn((url: string) =>
        String(url).includes("/leaderboard")
          ? Promise.resolve({
              ok: true,
              json: () =>
                Promise.resolve({
                  week: "2957",
                  measured_at: "2026-09-07T00:01:00Z",
                  answered: 2,
                  published: 2,
                  rule: {
                    min_account_age_days: 30,
                    min_engager_age_days: 30,
                    cooldown_weeks: 3,
                  },
                  voided: null,
                  entries: [
                    {
                      rank: 1,
                      summoner: "111",
                      handle: "real",
                      mint: "So11111111111111111111111111111111111111112",
                      reply_url: "https://x.com/i/web/status/1",
                      score: 30,
                      raw: {
                        reposts: 10,
                        quotes: 0,
                        likes: 0,
                        replies: 0,
                        score: 30,
                      },
                      verified: {
                        reposts: 10,
                        quoters: 0,
                        likes: 0,
                        engagers: 10,
                        engagers_under_age: 0,
                      },
                    },
                    {
                      rank: 2,
                      summoner: "222",
                      handle: "farm",
                      mint: null,
                      reply_url: null,
                      score: 3,
                      raw: {
                        reposts: 0,
                        quotes: 30,
                        likes: 0,
                        replies: 30,
                        score: 120,
                      },
                      verified: {
                        reposts: 0,
                        quoters: 1,
                        likes: 0,
                        engagers: 1,
                        engagers_under_age: 1,
                      },
                    },
                  ],
                  excluded: { count: 0, reasons: {} },
                }),
            })
          : Promise.reject(new Error("no server")),
      ),
    );
  }

  it("shows what was counted beside what was reported", async () => {
    // The gap between the two IS the farming, and a reader who cannot see both
    // numbers is being asked to trust the operator rather than check them.
    // Design 0011: publish the measurement, never the verdict.
    //
    // Re-apply by dropping the raw column: the farm's 120 vanishes and the
    // page shows a rank nobody can argue with.
    withFarmedWeek();
    render(<Leaderboard />);
    await waitFor(() => {
      expect(screen.getByText("@real")).toBeTruthy();
    });

    // The farm reported 30 quotes and scored 3, because thirty quotes came
    // from one account. Both numbers are on the page.
    expect(screen.getByTitle("0 reposts / 30 quotes / 0 likes")).toBeTruthy();
    expect(screen.getByTitle("0 reposts / 1 quotes / 0 likes")).toBeTruthy();
    // Ten real reposters outrank it, which is the whole point of the rule.
    // `getAllBy`, because for an honest entry the two columns agree -- and
    // that agreement is itself the thing a reader is checking for.
    expect(
      screen.getAllByTitle("10 reposts / 0 quotes / 0 likes"),
    ).toHaveLength(2);
  });

  it("counts new accounts without calling anybody a bot", async () => {
    // A count a reader weighs, never a threshold that excludes. Design 0011
    // phase 2 turns a cluster measurement into a rule only by ADR, after four
    // closed weeks -- until then the page states numbers and stops.
    withFarmedWeek();
    const { container } = render(<Leaderboard />);
    await waitFor(() => {
      expect(screen.getByText("@real")).toBeTruthy();
    });
    expect(screen.getByTitle("1 were under the age floor")).toBeTruthy();
    // Scoped to the table, not the page. The rule text above it legitimately
    // says "the pool cannot be farmed by one account" -- that is a statement
    // about the rule. What must never appear is a verdict about a ROW.
    const rows = container.querySelector("tbody")?.textContent ?? "";
    expect(rows).not.toBe("");
    for (const verdict of [
      /botted/i,
      /fake/i,
      /suspicious/i,
      /cheat/i,
      /abuse/i,
    ]) {
      expect(rows).not.toMatch(verdict);
    }
  });
});

describe("the history page", () => {
  /** A server that answers `/weeks` with one closed, claimed and paid week. */
  function withWeeks(week: unknown) {
    vi.stubGlobal(
      "fetch",
      vi.fn((url: string) =>
        String(url).includes("/weeks")
          ? Promise.resolve({
              ok: true,
              json: () =>
                Promise.resolve({
                  measured_at: "2026-09-07T00:01:00Z",
                  weeks: [week],
                }),
            })
          : Promise.reject(new Error("no server")),
      ),
    );
  }

  const paid = {
    week: "2026-08-31",
    opened_at: "2026-08-31T00:00:00Z",
    closed_at: "2026-09-07T00:00:00Z",
    entries: 4,
    excluded: { count: 2, reasons: { account_too_new: 1, operator: 1 } },
    winner: {
      summoner: "1889496824328880128",
      handle: "somebody",
      reply_url: "https://x.com/i/web/status/1",
      score: 12,
      mint: "So11111111111111111111111111111111111111112",
      verified: {
        reposts: 4,
        quoters: 1,
        likes: 6,
        engagers: 9,
        engagers_under_age: 2,
      },
    },
    rule: {
      operators: 2,
      min_account_age_days: 30,
      min_engager_age_days: 30,
      cooldown_weeks: 3,
    },
    voided: null,
    claim: {
      state: "claimed",
      at: "2026-09-07T00:05:00Z",
      address: "So11111111111111111111111111111111111111112",
      reply_url: "https://x.com/i/web/status/2",
    },
    payout: {
      state: "paid",
      lamports: 3_000_000_000,
      recipient: "So11111111111111111111111111111111111111112",
      signature:
        "5xoBq7f3vT1kQ9mNpLrWcJhYzA2dEuG6sVtXnH4bKfPqxoBq7f3vT1kQ9mNpLrWcJhYzA2dEuG6sVtXnH4bKfPqa",
      at: "2026-09-07T01:00:00Z",
    },
  };

  it("says no week has closed rather than showing an empty table", async () => {
    // The state it ships in today, and the wrong version looks right: an empty
    // table says a week ran and nobody entered it.
    render(<History />);
    expect(await screen.findByText(/No week has closed yet/i)).toBeTruthy();
    expect(document.querySelector("table")).toBeNull();
  });

  it("renders a paid week as three things a stranger can check", async () => {
    withWeeks(paid);
    render(<History />);

    // The winner, by handle, linked by id (S4/S27).
    const winner = await screen.findByText("@somebody");
    expect(winner.getAttribute("href")).toBe(
      "https://x.com/i/user/1889496824328880128",
    );
    // The winning reply, the claim, and the transaction: all links out.
    const links = Array.from(document.querySelectorAll("a")).map((a) =>
      a.getAttribute("href"),
    );
    expect(links).toContain("https://x.com/i/web/status/1");
    expect(links).toContain("https://x.com/i/web/status/2");
    expect(links.some((h) => h?.startsWith("https://solscan.io/tx/"))).toBe(
      true,
    );
    // The prize, in the unit it was paid in.
    expect(screen.getByText(/3\.0000 SOL/)).toBeTruthy();
    // The rule the week was scored under, printed beside it.
    expect(document.body.textContent).toContain("Scored under");
    // And the exclusions as counts, never as names.
    expect(document.body.textContent).toContain(
      "account younger than the rule allows",
    );
  });

  it("says why a week paid nobody instead of leaving the cell blank", async () => {
    // Four different facts. A page that renders all four the same asks the
    // reader to trust the operator about the one thing they would not.
    withWeeks({
      ...paid,
      winner: { ...paid.winner, verified: null },
      claim: { state: "rolled_over", closed_at: "2026-09-14T00:00:00Z" },
      payout: { state: "unclaimed" },
    });
    render(<History />);
    expect(await screen.findByText(/never claimed/i)).toBeTruthy();
    expect(document.body.textContent).not.toContain("SOL");
    // Unread engagement is "not read", never a row of zeroes.
    expect(document.body.textContent).toContain("not read");
    expect(document.body.textContent).not.toContain("0/0/0");
  });

  it("publishes the reason a week was voided, verbatim", async () => {
    // Design 0011: the correction is public or it is not a correction.
    withWeeks({
      ...paid,
      voided: {
        at: "2026-09-08T00:00:00Z",
        reason: "every point came from six accounts made that morning",
      },
      payout: { state: "voided" },
    });
    render(<History />);
    expect(
      await screen.findByText(/six accounts made that morning/),
    ).toBeTruthy();
    expect(document.body.textContent).toContain("pays nobody");
  });

  it("shows no rule at all for a week that did not record one", async () => {
    // Rule 9 on the page somebody opens to dispute a placing. Today's numbers
    // are not evidence about a week that closed before they existed.
    withWeeks({ ...paid, rule: null });
    render(<History />);
    expect(await screen.findByText(/was not recorded/i)).toBeTruthy();
    expect(document.body.textContent).not.toContain("Scored under");
  });
});
