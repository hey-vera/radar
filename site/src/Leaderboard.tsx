// SPDX-License-Identifier: Apache-2.0
//! The week's top questions, and who asked them.
//!
//! # This page is empty today, and that is the interesting part
//!
//! The bot has not run. There is no reply log, no engagement to read, and no
//! week to rank. Every version of this page that shows a table with no rows is
//! **lying by layout**: an empty table says a week ran and nobody engaged, which
//! is a claim about the product's reception rather than about its existence.
//!
//! That is LEARNINGS entry 5 — a job that did not run must not look like a job
//! that ran and found nothing — rendered in HTML, on a public surface, where
//! the reader has no way to check.
//!
//! So the empty state is a sentence, the rule is printed whether or not there
//! is anything to apply it to, and a score that has not been read is `null` and
//! renders as a dash. Never `0`.

import { useEffect, useState } from "react";

import { leaderboard as fetchLeaderboard, type Leaderboard as Data } from "./api";
import { count, measuredAgo, safeHref } from "./honesty";
import { useTitle } from "./title";
import { Card, Heading, Measured, Nothing, Section, Steps } from "./ui";

/** The scoring rule, published in full because the prize is real money. */
function TheRule() {
  return (
    <Card>
      <h3 className="font-semibold text-[var(--color-text)]">How it is scored</h3>
      <p className="mt-3 font-mono text-sm text-[var(--color-signal)]">
        3 × reposts + 3 × quotes + 1 × likes + 1 × replies
      </p>
      <p className="mt-3 text-sm text-[var(--color-dim)]">
        Measured on <strong>Cabal Hunter's own reply</strong>, not on your post.
        That is deliberate: it is the cheaper read, it is harder to buy
        engagement on somebody else's tweet, and it rewards bringing a coin worth
        answering rather than bringing an audience.
      </p>
      <ul className="mt-4 space-y-2 text-sm text-[var(--color-dim)]">
        <li>· Weeks run Monday 00:00 UTC to Monday 00:00 UTC.</li>
        <li>· Ties go to whoever asked first.</li>
        <li>
          · Excluded: every account the operator controls — the bot itself and
          the account that manages it — accounts under 30 days old, and anyone the
          admission gate refused that week.
        </li>
        <li>
          · One win in any four weeks. A winner sits out the next three, so the
          pool cannot be farmed by one account.
        </li>
        <li>· Entry is free. You never need to hold anything to enter or to win.</li>
      </ul>
      <p className="mt-4 text-xs text-[var(--color-faint)]">
        Engagement can be bought. The weights, the account-age floor and full
        publication of every score make that visible rather than impossible. If
        a winner is obviously bought, the rule changes and the change is
        recorded.
      </p>
    </Card>
  );
}

/**
 * How a winner actually gets paid.
 *
 * **This was written down nowhere a winner could read it.** The mechanism
 * exists in `radar-analyst` and in design 0007 §6.2, and a winner's only view
 * of it was a reply naming them by URL. A prize nobody knows how to claim is a
 * prize that rolls over, and the rollover looks identical to a contest that
 * pays nobody.
 *
 * Step 3 is the one that matters and it is stated as a warning rather than as
 * an instruction: the claim is a reply **to the account's claim post**, and a
 * mention that is not a reply to it is read as an ordinary summons. Pasting an
 * address anywhere else does not claim anything.
 */
function HowToClaim() {
  return (
    <Card>
      <h3 className="font-semibold text-[var(--color-text)]">
        If you win, how to claim
      </h3>
      <div className="mt-4">
        <Steps
          steps={[
            {
              what: "The week closes and the account posts what it found.",
              when: "Mondays, 00:00 UTC",
            },
            {
              what: "Under the winning reply, the account posts a claim prompt addressed to the winner.",
            },
            {
              what: (
                <>
                  Reply to <strong>that post</strong> with a Solana wallet
                  address, from the same account that asked the question.
                </>
              ),
              when: "Within seven days",
            },
            {
              what: "The payout runs once a day: one transaction, the whole pool above the vault's rent reserve, signature published here.",
              when: "Daily, 01:00 UTC",
            },
            {
              what: "Unclaimed after seven days, the pool rolls into next week.",
            },
          ]}
        />
      </div>
      <p className="mt-4 text-sm text-[var(--color-dim)]">
        No wallet to connect, no login, no form. The reply is the proof, because
        X has already proved who you are.
      </p>
      <p className="mt-3 text-xs text-[var(--color-faint)]">
        Paste a wallet address, not a coin. An address you do not control is an
        address the prize goes to.
      </p>
    </Card>
  );
}

function Table({ data }: { data: Data }) {
  return (
    <div className="overflow-x-auto">
      <table className="w-full text-left text-sm">
        <thead className="border-b border-[var(--color-line)] text-[var(--color-faint)]">
          <tr>
            <th className="py-3 pr-4 font-normal">#</th>
            <th className="py-3 pr-4 font-normal">Who asked</th>
            <th className="py-3 pr-4 font-normal">Coin</th>
            <th className="py-3 pr-4 font-normal">Reply</th>
            <th className="py-3 text-right font-normal">Score</th>
          </tr>
        </thead>
        <tbody>
          {data.entries.map((e) => (
            <tr
              key={`${e.rank}-${e.summoner}`}
              className="border-b border-[var(--color-line)]"
            >
              <td className="tnum py-3 pr-4 text-[var(--color-faint)]">{e.rank}</td>
              <td className="py-3 pr-4 text-[var(--color-text)]">@{e.summoner}</td>
              <td className="py-3 pr-4 font-mono text-xs text-[var(--color-dim)]">
                {/* A mint that never resolved is not a blank cell. The bot
                    answers questions it could not tie to a coin, and hiding
                    that would overstate how much it knew. */}
                {e.mint ? `${e.mint.slice(0, 8)}…` : "no coin resolved"}
              </td>
              <td className="py-3 pr-4">
                {safeHref(e.reply_url ?? "", ["x.com"]) ? (
                  <a
                    href={safeHref(e.reply_url ?? "", ["x.com"]) ?? undefined}
                    rel="noopener noreferrer nofollow"
                    target="_blank"
                    className="text-[var(--color-signal)] underline underline-offset-4"
                  >
                    read it
                  </a>
                ) : (
                  // Answered but not published: the distinction the operator's
                  // own analyst page makes, and it belongs here too.
                  <span className="text-[var(--color-faint)]">not published</span>
                )}
              </td>
              <td className="tnum py-3 text-right text-[var(--color-text)]">
                {/* `null` is "engagement has not been read yet", which is not
                    a score of zero. A dash says so without inventing one. */}
                {e.score === null ? (
                  <span className="text-[var(--color-faint)]" title="not read yet">
                    —
                  </span>
                ) : (
                  count(e.score)
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export function Leaderboard() {
  useTitle("Leaderboard");
  const [data, setData] = useState<Data | null>(null);

  useEffect(() => {
    let live = true;
    void fetchLeaderboard().then((next) => {
      if (live) setData(next);
    });
    return () => {
      live = false;
    };
  }, []);

  const empty = data !== null && data.entries.length === 0;

  return (
    <Section>
      <Heading kicker="This week">The questions worth asking</Heading>
      <p className="mb-8 max-w-2xl text-[var(--color-dim)]">
        Every week, the summoner whose question produced the reply that travelled
        furthest takes the whole prize pool. Entry is free: ask Cabal Hunter
        about a coin and you are in.
      </p>

      <div className="grid gap-8 lg:grid-cols-[1fr_20rem]">
        <div>
          {data === null ? (
            <p className="text-[var(--color-dim)]">Reading…</p>
          ) : empty ? (
            <Nothing
              what="No week has run yet."
              why="Cabal Hunter has not answered anyone. The account is not live — when it is, every question asked that week appears here with its score, and the rule beside this is what decides them."
            />
          ) : (
            <>
              <p className="mb-4 text-sm text-[var(--color-dim)]">
                <strong className="text-[var(--color-text)]">
                  {count(data.answered)}
                </strong>{" "}
                answered,{" "}
                <strong className="text-[var(--color-text)]">
                  {count(data.published)}
                </strong>{" "}
                published
                {data.answered > 0 && data.published === 0 && (
                  <span className="text-[var(--color-faint)]">
                    {" "}
                    — nothing reached the platform, which is either a dry run or
                    a publisher that could not post
                  </span>
                )}
              </p>
              <Table data={data} />
            </>
          )}
          {data?.measured_at && (
            <Measured ago={measuredAgo(data.measured_at)} />
          )}
        </div>
        <div className="space-y-8">
          <TheRule />
          <HowToClaim />
        </div>
      </div>
    </Section>
  );
}
