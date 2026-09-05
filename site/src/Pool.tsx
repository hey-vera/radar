// SPDX-License-Identifier: Apache-2.0
//! The prize pool.
//!
//! # `0.00 SOL` would be a lie, and it is the obvious thing to render
//!
//! There is no token, so there is no creator vault, so there is no balance. A
//! page showing `0.00 SOL` says a pool exists and is empty — which reads as a
//! contest nobody won, or one that pays nothing. Both are worse than the truth
//! and both are what a reader would take away.
//!
//! Rule 9 of `AGENTS.md`: absent is not zero, and this is the direction that
//! flatters. `lamports === null` and `lamports === 0` render differently here,
//! and a test asserts the first case does not contain "0.00".
//!
//! # The economics are stated, not implied
//!
//! ADR 0013's constraints are the product, not the small print: no dev buy, no
//! allocation, the operator holds zero tokens, and the only cash flow is
//! pump.fun's 30 bps creator fee, all of which becomes the prize. At $10k of
//! weekly volume that is about $3. Saying so is the whole difference between
//! this and the thing it is built to expose.

import { useEffect, useState } from "react";

import { pool as fetchPool, sol, type Pool as Data } from "./api";
import { measuredAgo } from "./honesty";
import { useTitle } from "./title";
import { Card, Heading, Measured, Nothing, Section } from "./ui";

function Economics() {
  return (
    <Card>
      <h3 className="font-semibold text-[var(--color-text)]">
        Where the money comes from
      </h3>
      <p className="mt-3 text-sm text-[var(--color-dim)]">
        pump.fun pays a creator fee of <strong>30 basis points of volume</strong>{" "}
        to whoever launched a coin. That fee is the only money in this contest,
        and <strong>100% of it becomes the prize</strong>.
      </p>
      <ul className="mt-4 space-y-2 text-sm text-[var(--color-dim)]">
        <li>· No dev buy. No allocation. No team or treasury tokens.</li>
        <li>· The operator holds zero tokens, and always will.</li>
        <li>· The bot never mentions the token's price. Not once.</li>
        <li>
          · It answers questions about its own token on exactly the same rule as
          any other coin.
        </li>
      </ul>
      <p className="mt-4 text-sm text-[var(--color-faint)]">
        So the prize scales with volume and nothing else. At $10,000 of weekly
        volume it is roughly <span className="tnum">$3</span>; at $100,000,
        roughly <span className="tnum">$30</span>. That is small, and saying so
        is the point — a memecoin that lies about its economics is the thing this
        bot exists to expose.
      </p>
    </Card>
  );
}

function Winners({ data }: { data: Data }) {
  if (data.winners.length === 0) return null;
  return (
    <div className="mt-12">
      <h3 className="mb-4 font-semibold text-[var(--color-text)]">Paid out</h3>
      <div className="overflow-x-auto">
        <table className="w-full text-left text-sm">
          <thead className="border-b border-[var(--color-line)] text-[var(--color-faint)]">
            <tr>
              <th className="py-3 pr-4 font-normal">Week</th>
              <th className="py-3 pr-4 font-normal">Winner</th>
              <th className="py-3 pr-4 font-normal">Prize</th>
              <th className="py-3 font-normal">Transaction</th>
            </tr>
          </thead>
          <tbody>
            {data.winners.map((w) => (
              <tr key={w.signature} className="border-b border-[var(--color-line)]">
                <td className="py-3 pr-4 text-[var(--color-dim)]">{w.week}</td>
                <td className="py-3 pr-4 text-[var(--color-text)]">@{w.summoner}</td>
                <td className="tnum py-3 pr-4 text-[var(--color-text)]">
                  {sol(w.lamports)} SOL
                </td>
                <td className="py-3">
                  {/* The signature, always. A prize nobody can verify was paid
                      is a claim, and this page is built so that every claim on
                      it can be checked by a stranger. */}
                  <a
                    href={`https://solscan.io/tx/${w.signature}`}
                    rel="noopener noreferrer nofollow"
                    target="_blank"
                    className="font-mono text-xs text-[var(--color-signal)] underline underline-offset-4"
                  >
                    {w.signature.slice(0, 12)}…
                  </a>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

export function Pool() {
  useTitle("Prize pool");
  const [data, setData] = useState<Data | null>(null);

  useEffect(() => {
    let live = true;
    void fetchPool().then((next) => {
      if (live) setData(next);
    });
    return () => {
      live = false;
    };
  }, []);

  // Three states, and they are genuinely three. No vault means no token has
  // been launched. A vault with no balance read means the read failed. A
  // balance of zero means a real, empty pool — which will happen at the start
  // of every week, right after a payout.
  const noToken = data !== null && data.vault === null;
  const unread = data !== null && data.vault !== null && data.lamports === null;

  return (
    <Section>
      <Heading kicker="The prize pool">Every creator fee, paid out weekly</Heading>

      <div className="grid gap-8 lg:grid-cols-[1fr_22rem]">
        <div>
          {data === null ? (
            <p className="text-[var(--color-dim)]">Reading…</p>
          ) : noToken ? (
            <Nothing
              what="There is no token yet, so there is no pool."
              why="The vault is a public address on chain that does not exist until the coin is launched. When it does, its balance appears here and you can check it yourself — this page will never show you a number it did not read."
            />
          ) : unread ? (
            <Nothing
              what="The vault balance could not be read."
              why="The address exists; the read failed. Rather than showing you a stale figure as though it were current, this says nothing until it can say something true."
            />
          ) : (
            <>
              <div className="tnum text-5xl font-semibold text-[var(--color-signal)] sm:text-6xl">
                {sol(data.lamports ?? 0)} SOL
              </div>
              <p className="mt-3 text-[var(--color-dim)]">
                collected this week, and going to one person on Monday.
              </p>
              {data.vault && (
                <p className="mt-4 font-mono text-xs break-all text-[var(--color-faint)]">
                  <a
                    href={`https://solscan.io/account/${data.vault}`}
                    rel="noopener noreferrer nofollow"
                    target="_blank"
                    className="underline underline-offset-4"
                  >
                    {data.vault}
                  </a>
                </p>
              )}
              {data.lamports === 0 && (
                <p className="mt-4 text-sm text-[var(--color-dim)]">
                  Empty right now — a week has just started, or the last prize
                  has just been paid. It fills as the coin trades.
                </p>
              )}
            </>
          )}
          {data?.measured_at && <Measured ago={measuredAgo(data.measured_at)} />}
          {data && <Winners data={data} />}
          {data && data.winners.length === 0 && !noToken && (
            <p className="mt-8 text-sm text-[var(--color-faint)]">
              Nobody has been paid yet. When somebody is, the transaction
              signature appears here.
            </p>
          )}
        </div>
        <Economics />
      </div>
      <p className="mt-10 max-w-2xl text-xs text-[var(--color-faint)]">
        Prizes are shown in SOL because that is the unit the fee is paid in. A
        dollar figure would need a price feed, with its own source and its own
        timestamp, and this page does not have one.
      </p>
    </Section>
  );
}
