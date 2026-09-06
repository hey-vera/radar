// SPDX-License-Identifier: Apache-2.0
//! The token, and every rule it is launched under.
//!
//! # There is no price on this page and there will never be one
//!
//! Not a stylistic choice. [ADR 0013](../../docs/adr/0013-a-community-token-exists-and-radar-holds-none-of-it.md)
//! constraint 5 forbids the *bot* from stating the token's price or market
//! capitalisation, and a marketing page that prints what the bot is forbidden to
//! say would make that constraint decorative. `RADAR_SELF_MINT` enforces it on
//! the Rust side by refusing to answer about the token with any price fact; this
//! page holds the same line by having nothing of the kind to render.
//!
//! What is here instead is arithmetic a reader can check: the fee schedule, read
//! off the chain, and where the fee goes. `fee-ladder.json` is pinned to
//! `radar-pumpfun`'s own decoder by a test in that crate, so a number on this
//! page cannot be edited to make the prize look better.
//!
//! # Why this page exists separately from the pool
//!
//! `/pool` answers "what is in the pot this week". This answers "what is this
//! thing and what stops you being the exit liquidity". They are different
//! questions from different readers, and the second one is the one a stranger
//! arriving from a reply actually has.

import ladder from "./fixtures/fee-ladder.json";
import { useTitle } from "./title";
import { Card, Heading, Nothing, Section, Steps } from "./ui";

/** A row of the on-chain fee schedule, as the fixture holds it. */
interface Row {
  readonly from_sol: number;
  readonly lp_bps: number;
  readonly protocol_bps: number;
  readonly creator_bps: number;
}

/**
 * The six rules, in ADR 0013's own order.
 *
 * One sentence each. The ADR argues them; this states them, because a reader
 * deciding in four seconds needs the constraint, not the reasoning.
 */
const RULES: readonly { readonly rule: string; readonly plain: string }[] = [
  {
    rule: "No dev buy, no allocation, no team or treasury tokens.",
    plain: "The launch block pays nobody. It is the shape this bot calls out.",
  },
  {
    rule: "The operator holds zero tokens.",
    plain: "There is no bag to sell you, because there is no bag.",
  },
  {
    rule: "100% of the creator fee is paid out as a public weekly prize.",
    plain: "Every lamport the fee earns leaves again the same week.",
  },
  {
    rule: "Entry is free and never requires holding the token.",
    plain: "Mention the account with a coin. That is the whole entry.",
  },
  {
    rule: "The bot never states the token's price or market capitalisation.",
    plain: "Not once, not if asked, not on this page either.",
  },
  {
    rule: "The token is roasted like anything else.",
    plain: "Same rule, same fact sheet, same refusals. Ask it.",
  },
];

function Rules() {
  return (
    <Section id="rules">
      <Heading kicker="The six rules">What it is launched under</Heading>
      <p className="mb-8 max-w-2xl text-[var(--color-dim)]">
        A badge, not an investment. It is not a share, it does not grant a vote,
        and it buys no feature — every answer the bot gives is free to everyone,
        with or without it. If that sounds like it leaves nothing to speculate
        on, that is the intention.
      </p>
      <div className="grid gap-4 md:grid-cols-2">
        {RULES.map((r, i) => (
          <Card key={i}>
            <div
              className="tnum font-mono text-xs text-[var(--color-signal-dim)]"
              aria-hidden="true"
            >
              {String(i + 1).padStart(2, "0")}
            </div>
            <p className="mt-2 font-medium text-[var(--color-text)]">{r.rule}</p>
            <p className="mt-2 text-sm text-[var(--color-dim)]">{r.plain}</p>
          </Card>
        ))}
      </div>
    </Section>
  );
}

function Money() {
  return (
    <Section id="money">
      <Heading kicker="Where the money goes">
        Volume becomes a fee, and the fee becomes the prize
      </Heading>
      <div className="max-w-2xl">
        <Steps
          steps={[
            {
              what: "Somebody trades the token. pump.fun charges a fee on the trade.",
            },
            {
              what: "Part of that fee is the creator's, and the creator is this token's vault.",
            },
            {
              what: "The vault is swept to the prize pool. Nothing is kept back.",
            },
            {
              what: "The week's best question wins the pool, in one public transaction.",
              when: "Mondays, 00:00 UTC close · payout 01:00 UTC",
            },
          ]}
        />
      </div>
    </Section>
  );
}

function Ladder() {
  const rows = ladder.after_graduation.rows as readonly Row[];
  return (
    <Section id="fees">
      <Heading kicker="The fee, read off the chain">
        30 basis points, then a ladder
      </Heading>
      <p className="mb-6 max-w-2xl text-[var(--color-dim)]">
        While a coin is still on its bonding curve the creator's fee is{" "}
        <strong className="text-[var(--color-text)]">
          {ladder.curve.creator_bps} basis points
        </strong>{" "}
        of volume — 0.30%. If it graduates, the fee follows a schedule the fee
        program keeps on chain, keyed on market capitalisation. It steps{" "}
        <em>up</em> to 95 immediately after graduation and then down to 5 at the
        top.
      </p>
      <Card className="scroll-x">
        <table className="w-full text-left text-sm">
          <caption className="sr-only">
            The creator fee by market capitalisation, in basis points
          </caption>
          <thead>
            <tr className="text-[var(--color-faint)]">
              <th scope="col" className="pb-2 font-normal">
                Market cap from
              </th>
              <th scope="col" className="pb-2 text-right font-normal">
                Creator
              </th>
              <th scope="col" className="pb-2 text-right font-normal">
                Protocol
              </th>
              <th scope="col" className="pb-2 text-right font-normal">
                Liquidity
              </th>
            </tr>
          </thead>
          <tbody className="tnum">
            {rows.map((r) => (
              <tr
                key={r.from_sol}
                className="border-t border-[var(--color-line)]"
              >
                <th
                  scope="row"
                  className="py-1.5 font-normal text-[var(--color-dim)]"
                >
                  {r.from_sol.toLocaleString()} SOL
                </th>
                <td className="py-1.5 text-right font-medium text-[var(--color-signal)]">
                  {r.creator_bps}
                </td>
                <td className="py-1.5 text-right text-[var(--color-dim)]">
                  {r.protocol_bps}
                </td>
                <td className="py-1.5 text-right text-[var(--color-dim)]">
                  {r.lp_bps}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </Card>
      <p className="mt-4 text-xs text-[var(--color-faint)]">
        Read from the fee program's own account on {ladder.captured}, and
        checked against that account by a test on every build. Two live swaps
        paid a row further down the ladder than their pool's market cap selects;
        that disagreement is recorded and unresolved, and it is the reason this
        table is dated.
      </p>
    </Section>
  );
}

function Gate() {
  return (
    <Section id="gate">
      <Heading kicker="Before any of this happens">
        What has to be true first
      </Heading>
      <div className="grid gap-6 md:grid-cols-2">
        <Card>
          <p className="text-[var(--color-text)]">The demand gate</p>
          <ul className="mt-3 space-y-2 text-sm text-[var(--color-dim)]">
            <li>· 30 days of the bot live and answering.</li>
            <li>· 200 distinct accounts that have summoned it.</li>
            <li>· 10% of its replies drawing any engagement at all.</li>
          </ul>
          <p className="mt-3 text-sm text-[var(--color-faint)]">
            A token launched into no demand is the thing this account exists to
            point at.
          </p>
        </Card>
        <Card>
          <p className="text-[var(--color-text)]">The legal read</p>
          <p className="mt-3 text-sm text-[var(--color-dim)]">
            A precondition, not a follow-up. Two questions have to be answered
            by somebody qualified before anything is minted, and the answer may
            be no.
          </p>
        </Card>
      </div>
    </Section>
  );
}

function Status() {
  return (
    <Section id="status">
      <Heading kicker="Right now">Status</Heading>
      <div className="max-w-2xl">
        <Nothing
          what="No token exists."
          why="Nothing has been minted, no contract address has been published, and any address claiming to be this token is not. When one exists it will be published here and in the account's own bio, and nowhere else."
        />
      </div>
    </Section>
  );
}

export function Token() {
  useTitle("Tokenomics");
  return (
    <>
      <Section className="pt-16 pb-0">
        <div className="enter">
          <h1 className="display max-w-3xl text-[length:var(--text-display)] leading-[1.05] font-semibold text-balance">
            A badge, not an investment.
          </h1>
          <p className="mt-6 max-w-2xl text-[length:var(--text-lead)] text-[var(--color-dim)]">
            There is no token yet. When there is one, these are the rules it is
            launched under — written down first, so they can be held against it
            afterwards.
          </p>
        </div>
      </Section>
      <Status />
      <Rules />
      <Money />
      <Ladder />
      <Gate />
    </>
  );
}
