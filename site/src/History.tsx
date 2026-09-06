// SPDX-License-Identifier: Apache-2.0
//! Past weeks: who won, whether they collected, and whether they were paid.
//!
//! # This is the page that makes the contest checkable
//!
//! Every other page is a claim about the present. This one is the record, and
//! it is the only place a stranger can establish that the thing has ever
//! actually paid anybody. So nothing here is a bare assertion: the winning
//! reply is a link, the claim is a link, the payment is a transaction
//! signature on Solscan, and the rule the week was scored under is printed
//! beside the week rather than looked up from today's rule.
//!
//! # "Not paid" is four different facts and the page says which
//!
//! Nobody entered. The winner never claimed and the prize rolled over. The
//! operator voided the week, with the reason. Or the payment is simply still
//! owed. A page that renders all four as an empty cell asks the reader to
//! trust the operator about the one thing they most reasonably would not, and
//! `payout.state` exists so it does not have to.
//!
//! # An old week's rule is not today's rule
//!
//! `rule: null` means the record does not carry one -- a week closed before
//! 2026-09-06, when nothing wrote it down. The page says "not recorded", never
//! the current numbers. Rule 9, on the page somebody opens to dispute a
//! placing.

import { useEffect, useState } from "react";

import {
  sol,
  weeks as fetchWeeks,
  type Payout,
  type Claim,
  type Week,
  type Weeks,
} from "./api";
import { measuredAgo, solscanTx } from "./honesty";
import { useTitle } from "./title";
import { Heading, Measured, Nothing, Section, Summoner } from "./ui";

/** One reason an entry did not count, in words a reader can check. */
const REASONS: Readonly<Record<string, string>> = {
  operator: "an account the operator controls",
  account_too_new: "account younger than the rule allows",
  account_age_unknown: "account age could not be read",
  refused_this_week: "refused by the gate during the week",
  won_within_cooldown: "won recently, still in cooldown",
  unscored: "engagement was never read",
};

/** An external link that is always the same shape, or nothing. */
function Out({
  href,
  children,
}: {
  href: string | null;
  children: React.ReactNode;
}) {
  if (!href)
    return <span className="text-[var(--color-faint)]">{children}</span>;
  return (
    <a
      href={href}
      target="_blank"
      rel="noopener noreferrer nofollow"
      className="text-[var(--color-signal)] underline underline-offset-4"
    >
      {children}
    </a>
  );
}

/**
 * The payment, or the reason there was not one.
 *
 * The signature is the whole point of the paid case: a prize nobody can verify
 * was paid is a claim, and this page exists so that every claim on it can be
 * checked by a stranger who trusts nobody here.
 */
function Paid({ payout }: { payout: Payout }) {
  if (payout.state === "paid") {
    const href = payout.signature ? solscanTx(payout.signature) : null;
    return (
      <div>
        <div className="tnum text-[var(--color-text)]">
          {sol(payout.lamports ?? 0)} SOL
        </div>
        <div className="mt-1 font-mono text-xs">
          {payout.signature ? (
            <Out href={href}>
              {href ? `${payout.signature.slice(0, 12)}…` : "not a signature"}
            </Out>
          ) : (
            <span className="text-[var(--color-faint)]">no signature</span>
          )}
        </div>
      </div>
    );
  }
  const said: Readonly<Record<string, string>> = {
    owed: "claimed, not yet paid",
    unclaimed: "never claimed — rolled over",
    awaiting_claim: "waiting on the winner",
    voided: "voided — pays nobody",
    no_winner: "nobody counted",
  };
  return (
    <span className="text-sm text-[var(--color-dim)]">
      {said[payout.state] ?? payout.state}
    </span>
  );
}

/** Whether the winner collected, and by when they must. */
function Collected({ claim }: { claim: Claim }) {
  if (claim.state === "claimed") {
    return <Out href={claim.reply_url ?? null}>claimed</Out>;
  }
  if (claim.state === "open") {
    return (
      <span className="text-sm text-[var(--color-dim)]">
        open until {claim.closes_at?.slice(0, 10) ?? "the window closes"}
      </span>
    );
  }
  if (claim.state === "rolled_over") {
    return <span className="text-sm text-[var(--color-dim)]">rolled over</span>;
  }
  return <span className="text-sm text-[var(--color-faint)]">—</span>;
}

/** The counts a score was built from, so it can be recomputed. */
function Evidence({ week }: { week: Week }) {
  const v = week.winner?.verified;
  if (!v) {
    // Not zero. The scan stops as soon as arithmetic says nothing below can
    // win, so most entries in a busy week are never scanned -- and a row of
    // zeroes here would say nobody engaged with the winning reply.
    return <span className="text-sm text-[var(--color-faint)]">not read</span>;
  }
  return (
    <span
      className="tnum text-sm text-[var(--color-dim)]"
      title={`${v.reposts} reposted, ${v.quoters} quoted, ${v.likes} liked — ${v.engagers} distinct accounts, ${v.engagers_under_age} of them under the age floor`}
    >
      {v.reposts}/{v.quoters}/{v.likes}
      {v.engagers_under_age > 0 && (
        <span className="text-[var(--color-faint)]">
          {" "}
          ({v.engagers_under_age} new)
        </span>
      )}
    </span>
  );
}

/** The rule the week was actually scored under. */
function Rule({ week }: { week: Week }) {
  if (!week.rule) {
    return (
      <p className="text-xs text-[var(--color-faint)]">
        The rule this week was scored under was not recorded. It is not shown
        here, because today's rule is not evidence about a week that closed
        before it.
      </p>
    );
  }
  const r = week.rule;
  return (
    <p className="text-xs text-[var(--color-faint)]">
      Scored under: entrants at least{" "}
      <span className="tnum">{r.min_account_age_days}</span> days old, engagers
      at least <span className="tnum">{r.min_engager_age_days}</span>,{" "}
      <span className="tnum">{r.cooldown_weeks}</span>-week cooldown after a
      win, <span className="tnum">{r.operators}</span> operator{" "}
      {r.operators === 1 ? "account" : "accounts"} excluded.
    </p>
  );
}

/** Who did not count, and why — as counts, never as names. */
function Excluded({ week }: { week: Week }) {
  const rows = Object.entries(week.excluded.reasons);
  if (rows.length === 0) return null;
  return (
    <p className="mt-2 text-xs text-[var(--color-faint)]">
      Did not count:{" "}
      {rows.map(([key, n]) => `${n} × ${REASONS[key] ?? key}`).join(", ")}.
    </p>
  );
}

function Row({ week }: { week: Week }) {
  return (
    <div className="border-b border-[var(--color-line)] py-6">
      <div className="flex flex-wrap items-baseline justify-between gap-2">
        <h3 className="tnum font-semibold text-[var(--color-text)]">
          Week of {week.week}
        </h3>
        <span className="tnum text-xs text-[var(--color-faint)]">
          {week.entries} {week.entries === 1 ? "entry" : "entries"}
        </span>
      </div>

      {week.voided && (
        // Published verbatim, and prominently. A void that a reader has to
        // hunt for is the private correction design 0011 rejects.
        <p className="mt-3 rounded border border-[var(--color-line)] bg-[var(--color-raised)] px-3 py-2 text-sm text-[var(--color-dim)]">
          <strong className="text-[var(--color-text)]">Voided.</strong> This
          week paid nobody. The reason, as published: “{week.voided.reason}”
        </p>
      )}

      <dl className="mt-4 grid grid-cols-2 gap-x-6 gap-y-3 text-sm sm:grid-cols-4">
        <div>
          <dt className="text-xs text-[var(--color-faint)]">Winner</dt>
          <dd className="mt-1">
            {week.winner ? (
              <Summoner id={week.winner.summoner} handle={week.winner.handle} />
            ) : (
              <span className="text-[var(--color-faint)]">nobody counted</span>
            )}
          </dd>
        </div>
        <div>
          <dt className="text-xs text-[var(--color-faint)]">Reply</dt>
          <dd className="mt-1">
            {week.winner ? (
              <Out href={week.winner.reply_url}>the winning reply</Out>
            ) : (
              <span className="text-[var(--color-faint)]">—</span>
            )}
          </dd>
        </div>
        <div>
          <dt className="text-xs text-[var(--color-faint)]">Claim</dt>
          <dd className="mt-1">
            <Collected claim={week.claim} />
          </dd>
        </div>
        <div>
          <dt className="text-xs text-[var(--color-faint)]">Paid</dt>
          <dd className="mt-1">
            <Paid payout={week.payout} />
          </dd>
        </div>
      </dl>

      {week.winner && (
        <p className="mt-3 text-xs text-[var(--color-faint)]">
          Score{" "}
          <span className="tnum text-[var(--color-dim)]">
            {week.winner.score}
          </span>{" "}
          from <Evidence week={week} /> (reposts / quoters / likes, distinct
          accounts).
        </p>
      )}
      <div className="mt-2">
        <Rule week={week} />
      </div>
      <Excluded week={week} />
    </div>
  );
}

export function History() {
  useTitle("Past weeks");
  const [data, setData] = useState<Weeks | null>(null);

  useEffect(() => {
    let live = true;
    void fetchWeeks().then((next) => {
      if (live) setData(next);
    });
    return () => {
      live = false;
    };
  }, []);

  return (
    <Section>
      <Heading kicker="Past weeks">
        Who won, whether they claimed, and whether they were paid
      </Heading>

      <p className="max-w-2xl text-[var(--color-dim)]">
        Every closed week, newest first. Each row links to the thing it claims:
        the reply that won, the reply that claimed it, and the transaction that
        paid it. Nothing here has to be taken on trust.
      </p>

      <div className="mt-10">
        {data === null ? (
          <p className="text-[var(--color-dim)]">Reading…</p>
        ) : data.weeks.length === 0 ? (
          <Nothing
            what="No week has closed yet."
            why="The first week closes on a Monday at 00:00 UTC. When it does, it appears here with the winner, the claim, and the transaction that paid it — and it stays here."
          />
        ) : (
          <div>
            {data.weeks.map((w) => (
              <Row key={w.week} week={w} />
            ))}
          </div>
        )}
        {data?.measured_at && <Measured ago={measuredAgo(data.measured_at)} />}
      </div>

      <p className="mt-10 max-w-2xl text-xs text-[var(--color-faint)]">
        Entries that did not count are shown as counts by reason and never as
        names. The rule they were measured against is published, so the counts
        are enough to check that it was applied — an entrant excluded for being
        thirty days too new does not need that published beside their handle.
      </p>
    </Section>
  );
}
