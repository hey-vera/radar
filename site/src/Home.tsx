// SPDX-License-Identifier: Apache-2.0
//! The whole argument, on one page, in six acts.
//!
//! # The order is the argument, and now it is numbered
//!
//! 01. Coordination is visible before you buy, and here is the picture of it.
//! 02. This is how much has been watched, so the number means something.
//! 03. Here is what it costs you to be wrong — including the part nobody says,
//!     which is that graduating is not winning.
//! 04. Here is how to ask it about a coin, and here is what it answers.
//! 05. Here is what it will never say.
//! 06. Here is the contest and the token.
//!
//! Act 03 is the one a growth-minded version of this page would cut. It is the
//! reason to trust the rest of it. Act 05 is the second one it would cut.
//!
//! The acts are numbered on the page because the order *is* the content: a
//! reader who scrolls past 03 knows they skipped something, which a stack of
//! undifferentiated sections cannot tell them.

import { useEffect, useState } from "react";
import { Link } from "wouter";

import { FALLBACK, stats as fetchStats, type Sourced } from "./api";
import {
  account,
  bps,
  cost,
  count,
  graduated,
  handleHref,
  measuredAgo,
  mostCoordinated,
  pct,
  share,
  type Band,
  type Stats,
} from "./honesty";
import {
  Card,
  Cta,
  Figure,
  Heading,
  LaunchBlock,
  Measured,
  Receipt,
  Section,
  Summon,
} from "./ui";

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

/** The small label that numbers an act. */
function Act({ n, children }: { n: string; children: React.ReactNode }) {
  return (
    <div className="mb-3 flex items-baseline gap-3">
      <span className="act-no" aria-hidden="true">
        {n}
      </span>
      <span className="font-mono text-xs tracking-widest text-[var(--color-signal-dim)] uppercase">
        {children}
      </span>
    </div>
  );
}

/**
 * One band, drawn.
 *
 * The count is in the caption and the squares are `aria-hidden`, so this is a
 * picture for readers who have one and a sentence for readers who do not.
 */
function BandGlyph({
  band,
  tone,
  caption,
}: {
  band: Band;
  tone: "quiet" | "signal";
  caption: string;
}) {
  return (
    <div>
      <LaunchBlock n={band.hi} tone={tone} />
      <p className="mt-3 text-sm text-[var(--color-text)]">
        {band.lo}–{band.hi} recipients
      </p>
      <p className="mt-1 text-sm text-[var(--color-dim)]">{caption}</p>
    </div>
  );
}

function Hero({ s }: { s: Stats }) {
  const top = mostCoordinated(s.bands.rows);
  const quiet = s.bands.rows.find((r) => r.lo === 1);

  return (
    <Section className="pt-16 pb-8 sm:pt-24">
      <div className="enter">
        <div className="mb-5">
          <Act n="01">Solana · pump.fun · measured since August</Act>
        </div>
        <h1 className="display max-w-3xl text-[length:var(--text-display)] leading-[1.03] font-semibold text-balance">
          Most launches are coordinated.{" "}
          <span className="text-[var(--color-signal)]">
            You can see it before you buy.
          </span>
        </h1>
        <p className="mt-6 max-w-2xl text-[length:var(--text-lead)] text-[var(--color-dim)]">
          When capital is committed to a token <em>before</em> it exists, the
          evidence is sitting in the launch block — the very first block of the
          coin's life. Cabal Hunter has been reading every one of them.
        </p>

        {/* The claim above is only worth as much as the number under it, so the
            number is immediately under it, it is found in the data rather than
            written into this sentence, and now it is also drawn: two rows of
            squares are a claim a reader can check before reading a figure. */}
        {top && quiet ? (
          <Card className="mt-10 max-w-2xl">
            <div className="grid gap-8 sm:grid-cols-2">
              <BandGlyph
                band={quiet}
                tone="quiet"
                caption={`${pct(quiet.share_of_launches)} of launches. ${pct(
                  quiet.p_instant,
                )} bought out instantly.`}
              />
              <BandGlyph
                band={top}
                tone="signal"
                caption={`${pct(top.share_of_launches)} of launches. ${pct(
                  top.p_instant,
                )} bought out instantly.`}
              />
            </div>
            <p className="mt-6 border-t border-[var(--color-line)] pt-5 text-[var(--color-text)]">
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
      <Act n="02">What has been watched</Act>
      <Heading>Every launch, and what became of it</Heading>
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
      <Act n="03">What it costs to be wrong</Act>
      <Heading>And graduating is not winning</Heading>
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

function Ask({ handle }: { handle: string | null }) {
  return (
    <Section id="ask">
      <Act n="04">How to use it</Act>
      <Heading>Reply to it with a coin</Heading>
      <div className="grid items-start gap-8 lg:grid-cols-2">
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
            It never picks a coin to post about. Every coin it names, somebody
            asked it about — and it does not decide what you should do.
          </p>
          <div className="mt-6">
            <Summon handle={handle} />
          </div>
        </div>
        <div>
          <Receipt>{A_REAL_REPLY}</Receipt>
          <p className="mt-4 text-xs text-[var(--color-faint)]">
            Ninety-three launches by one creator, and not one of them ever filled
            its curve. Against a base rate of 2.8%.
          </p>
        </div>
      </div>
    </Section>
  );
}

/**
 * What the bot refuses to say, in words rather than as a list of rule names.
 *
 * The refusals are the product's spine and they are invisible in a reply that
 * simply does not contain them, so they are stated once, here, where a reader
 * deciding whether to trust the account can see the shape of what it will not
 * do. Each line is a group the Rust side enforces; none of them is softened.
 */
function NeverSays() {
  return (
    <Section id="never">
      <Act n="05">What it will never say</Act>
      <Heading>The refusals are the product</Heading>
      <div className="grid gap-4 md:grid-cols-3">
        <Card>
          <p className="font-medium text-[var(--color-text)]">
            That a coin will go up
          </p>
          <p className="mt-2 text-sm text-[var(--color-dim)]">
            No price target, no prediction, no "this one is going to run".
            Nothing it measures supports a sentence about the future.
          </p>
        </Card>
        <Card>
          <p className="font-medium text-[var(--color-text)]">
            That you should buy, sell or hold
          </p>
          <p className="mt-2 text-sm text-[var(--color-dim)]">
            It does not give advice, and a clean reading is a reason to keep
            reading rather than a reason to buy.
          </p>
        </Card>
        <Card>
          <p className="font-medium text-[var(--color-text)]">
            That a coin is safe
          </p>
          <p className="mt-2 text-sm text-[var(--color-dim)]">
            No verdict, no score, no "legit". It reports what it read and lets
            the numbers say it. A quiet launch block is not a promise.
          </p>
        </Card>
      </div>
      <p className="mt-8 max-w-2xl text-[var(--color-dim)]">
        These are enforced in code, on every reply, before it is sent — not a
        tone of voice it tries to keep. A reply that would break one of them is
        not softened, it is refused, and the account says nothing instead.
      </p>
      <p className="mt-4">
        <Link
          href="/about"
          className="text-[var(--color-signal)] underline underline-offset-4 hover:text-[var(--color-text)]"
        >
          How it reads a coin, and who runs it →
        </Link>
      </p>
    </Section>
  );
}

function Contest({ handle }: { handle: string | null }) {
  return (
    <Section id="contest">
      <Act n="06">The contest</Act>
      <Heading>The best question each week wins the pool</Heading>
      <div className="grid gap-6 md:grid-cols-2">
        <Card>
          <p className="text-[var(--color-text)]">
            Every coin you ask it about is an entry. The week closes Monday at
            00:00 UTC, the account posts what it found, and the winner claims by
            replying with a wallet address.
          </p>
          <p className="mt-4">
            <Link
              href="/leaderboard"
              className="text-[var(--color-signal)] underline underline-offset-4 hover:text-[var(--color-text)]"
            >
              The rule, the leaderboard and how to claim →
            </Link>
          </p>
        </Card>
        <Card>
          <p className="text-[var(--color-text)]">
            The prize is the token's entire creator fee. The operator holds none
            of it, takes none of it, and the bot will never tell you its price.
          </p>
          <p className="mt-4">
            <Link
              href="/token"
              className="text-[var(--color-signal)] underline underline-offset-4 hover:text-[var(--color-text)]"
            >
              What the token is, and the six rules →
            </Link>
          </p>
        </Card>
      </div>
      {handle !== null && (
        <div className="mt-10">
          <Cta href={handleHref(handle)}>Follow @{handle} on X →</Cta>
        </div>
      )}
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

  const handle = account();

  return (
    <>
      <Hero s={s.value} />
      <Watched s={s.value} stale={s.stale} />
      <Cost s={s.value} />
      <Ask handle={handle} />
      <NeverSays />
      <Contest handle={handle} />
    </>
  );
}
