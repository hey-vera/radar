// SPDX-License-Identifier: Apache-2.0
//! The whole argument, on one page.
//!
//! # The order is the argument
//!
//! 1. Coordination is visible before you buy, and here is the number.
//! 2. This is how much has been watched, so the number means something.
//! 3. Here is what it costs you to be wrong — including the part nobody says,
//!    which is that graduating is not winning.
//! 4. Here is how to ask it about a coin.
//!
//! Section 3 is the one a growth-minded version of this page would cut. It is
//! the reason to trust the rest of it.

import { useEffect, useState } from "react";
import { Link } from "wouter";

import { FALLBACK, stats as fetchStats, type Sourced } from "./api";
import {
  bps,
  cost,
  count,
  graduated,
  measuredAgo,
  mostCoordinated,
  pct,
  share,
  type Stats,
} from "./honesty";
import { Card, Figure, Heading, Measured, Section } from "./ui";

/** A real reply, produced by `radar roast` against a live mint on 2026-09-04. */
const A_REAL_REPLY = `Radar on HWvHqvfFVQdLZ1K3kMygpvhivVZEcrzVShgJFgtXpump:
- tokens this creator has launched: 93
- of those, how many ever filled their curve over time: 0
- token accounts in the launch block (accounts, not people): 3
- across every launch Radar has measured, how many graduated at all: 2.8%
- and how many showed almost no activity at all: 23.0%
Entering and leaving a $20-$200 position: 456 bps (4.6%).
Read at slot 444390507.
Measured, not predicted. Not financial advice.`;

function Hero({ s }: { s: Stats }) {
  const top = mostCoordinated(s.bands.rows);
  const quiet = s.bands.rows.find((r) => r.lo === 1);

  return (
    <Section className="pt-20 pb-10 sm:pt-28">
      <div className="enter">
        <div className="mb-5 font-mono text-xs tracking-widest text-[var(--color-signal-dim)] uppercase">
          Solana · pump.fun · measured since August
        </div>
        <h1 className="max-w-3xl text-4xl font-semibold tracking-tight text-balance sm:text-6xl">
          Most launches are coordinated.{" "}
          <span className="text-[var(--color-signal)]">
            You can see it before you buy.
          </span>
        </h1>
        <p className="mt-6 max-w-2xl text-lg text-[var(--color-dim)]">
          When capital is committed to a token <em>before</em> it exists, the
          evidence is sitting in the launch block — the very first block of the
          coin's life. Cabal Hunter has been reading every one of them.
        </p>

        {/* The claim above is only worth as much as the number under it, so the
            number is immediately under it and it is found in the data rather
            than written into this sentence. */}
        {top && quiet ? (
          <Card className="mt-10 max-w-2xl">
            <p className="text-[var(--color-text)]">
              A launch block paying{" "}
              <strong className="text-[var(--color-signal)]">
                {top.lo}–{top.hi} recipients
              </strong>{" "}
              is{" "}
              <strong className="tnum text-[var(--color-signal)]">
                {top.x_base_instant.toFixed(1)}×
              </strong>{" "}
              likelier to have its curve bought out instantly than the average
              launch.
            </p>
            <p className="mt-3 text-sm text-[var(--color-dim)]">
              The {pct(quiet.share_of_launches)} of launches paying{" "}
              {quiet.lo}–{quiet.hi} recipients almost never are —{" "}
              {pct(quiet.p_instant)} of them.
            </p>
            <p className="mt-4 text-xs text-[var(--color-faint)]">
              Over {count(s.bands.launches)} launches, measured{" "}
              {s.bands.measured_on}. A smaller and older sample than the figures
              below, and it is the weaker of the two measurements here.
            </p>
          </Card>
        ) : (
          <Card className="mt-10 max-w-2xl border-dashed">
            <p className="text-[var(--color-dim)]">
              The recipient distribution has not been measured, so this claim
              cannot be made.
            </p>
          </Card>
        )}
      </div>
    </Section>
  );
}

function Watched({ s, stale }: { s: Stats; stale: boolean }) {
  const w = s.watched;
  return (
    <Section id="watched">
      <Heading kicker="What has been watched">
        Every launch, and what became of it
      </Heading>
      <div className="grid gap-10 sm:grid-cols-2 lg:grid-cols-4">
        <Figure
          value={count(w.launches)}
          label="launches recorded"
          note={`${count(w.creators)} distinct creators`}
        />
        <Figure
          value={pct(graduated(w))}
          label="ever graduate"
          note={`of ${count(w.measured)} measured`}
          tone="signal"
        />
        <Figure
          value={pct(share(w.stillborn, w.measured))}
          label="show almost no activity at all"
          note="five or fewer transfers, then nothing"
          tone="signal"
        />
        <Figure
          value={pct(share(w.instant, w.measured))}
          label="filled inside their own launch block"
          note="bought by capital committed before the coin existed"
        />
      </div>
      <Measured
        ago={measuredAgo(s.measured_at)}
        at={`slot ${count(s.watermark_slot)}`}
      />
      {stale && (
        <p className="mt-2 text-xs text-[var(--color-faint)]">
          Showing the last published measurement — the live figures could not be
          reached just now.
        </p>
      )}
    </Section>
  );
}

function Cost({ s }: { s: Stats }) {
  return (
    <Section id="cost">
      <Heading kicker="What it costs to be wrong">
        And graduating is not winning
      </Heading>
      <div className="grid gap-6 md:grid-cols-2">
        <Card>
          <div className="tnum text-3xl font-semibold text-[var(--color-text)]">
            {/* `cost`, not `bps`. This is a charge, not a return, and `bps`
                would sign it `+4.6%` -- a fee rendered as a gain. */}
            {cost(s.cost.round_trip_bps)}
          </div>
          <p className="mt-3 text-sm text-[var(--color-dim)]">
            The measured round trip on a {s.cost.band} position: what entering
            and leaving costs you before the price moves at all.
          </p>
        </Card>
        <Card>
          <div className="tnum text-3xl font-semibold text-[var(--color-signal)]">
            {bps(s.aftermath.organic_median_bps)}
          </div>
          <p className="mt-3 text-sm text-[var(--color-dim)]">
            Where tokens that graduated <em>the honest way</em> — filling their
            curve over time — ended up, at the median.
          </p>
        </Card>
      </div>
      <p className="mt-8 max-w-2xl text-[var(--color-dim)]">
        This is the part a page trying to sell you something would leave out.
        Graduation is the event everybody celebrates, and the median graduated
        token still ends deep underwater. Cabal Hunter will tell you a coin looks
        clean. It will never tell you a coin will go up, because nothing measured
        here supports that sentence.
      </p>
    </Section>
  );
}

function HowToAsk() {
  return (
    <Section id="ask">
      <Heading kicker="How to use it">Reply to it with a coin</Heading>
      <div className="grid gap-8 lg:grid-cols-2">
        <div>
          <p className="text-[var(--color-dim)]">
            Mention the account with a mint address or a{" "}
            <code className="rounded bg-[var(--color-raised)] px-1.5 py-0.5 font-mono text-sm text-[var(--color-text)]">
              $TICKER
            </code>
            . It reads the chain right then — the launch block, the curve, the
            creator's whole history — and answers in the thread.
          </p>
          <p className="mt-4 text-[var(--color-dim)]">
            It only ever answers when it is asked. It does not post about coins
            unprompted, and it does not decide what you should do.
          </p>
          <p className="mt-6">
            <Link
              href="/leaderboard"
              className="text-[var(--color-signal)] underline underline-offset-4 hover:text-[var(--color-text)]"
            >
              The best question each week wins the prize pool →
            </Link>
          </p>
        </div>
        <Card className="overflow-x-auto">
          <div className="mb-3 font-mono text-xs tracking-widest text-[var(--color-faint)] uppercase">
            An actual reply
          </div>
          <pre className="tnum font-mono text-xs leading-relaxed whitespace-pre-wrap text-[var(--color-text)]">
            {A_REAL_REPLY}
          </pre>
          <p className="mt-4 text-xs text-[var(--color-faint)]">
            Ninety-three launches by one creator, and not one of them ever filled
            its curve. Against a base rate of 2.8%.
          </p>
        </Card>
      </div>
    </Section>
  );
}

export function Home() {
  // Starts on the committed measurement so the page has content on the first
  // paint, then upgrades if the live document can be reached. There is no
  // loading state because there is nothing to wait for -- a spinner over a page
  // that already has true content would be a page pretending not to.
  const [s, setStats] = useState<Sourced<Stats>>({
    value: FALLBACK,
    stale: true,
  });

  useEffect(() => {
    let live = true;
    void fetchStats().then((next) => {
      if (live) setStats(next);
    });
    return () => {
      live = false;
    };
  }, []);

  return (
    <>
      <Hero s={s.value} />
      <Watched s={s.value} stale={s.stale} />
      <Cost s={s.value} />
      <HowToAsk />
    </>
  );
}
