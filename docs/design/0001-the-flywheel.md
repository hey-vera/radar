<!-- SPDX-License-Identifier: Apache-2.0 -->
# Design 0001 — The flywheel

**Date:** 2026-09-03
**Status:** proposal, for Josh to accept, change or reject. **Not a decision.**
An accepted version of this becomes a section in [GOAL.md](../../GOAL.md) and an
ADR for anything it commits code to.
**Decides nothing yet.** Nothing in the repository depends on it.

## What this is answering

A token exists — `$WAR` here as a placeholder — and the X account is the thing
that gives it attention. The question is what turns that attention into
something that **accrues**, rather than a spike that decays at the same rate it
arrived.

Written down rather than answered in a chat message because a direction decision
that lives only in a chat log did not happen (AGENTS.md §10).

## The problem with the obvious version

The default memecoin loop is:

```
attention ──▶ buys ──▶ price up ──▶ more attention
```

It is real, it works, and it is **not a moat**, because it runs in reverse at
the same speed. Every project on the venue has this loop. When attention dips,
nothing is left over.

So the design question is not "how do we get attention" — the bot answers that.
It is **"what does the attention deposit that is still there next month?"**

## What Radar has that nothing else does

**A record.** Every roast is a public, timestamped, checkable statement about a
specific token, made before anyone knew what happened next. Radar already keeps
the fact sheet behind each one, and the slot it was read at.

That compounds and it cannot be faked retroactively. A competitor can copy the
bot's voice in an afternoon. They cannot copy a six-month public record of
having been right in advance, and they cannot start one without waiting six
months.

So the flywheel should be:

```
attention ──▶ summons ──▶ public calls ──▶ RECORD ──▶ credibility
    ▲                                                      │
    └──────────────── harder to copy every week ◀──────────┘
```

The token rides that, and the record is what stops the ride ending.

## The layers

### 0. Free forever, zero friction

Anyone `@`-mentions, anyone gets the truth. No wallet, no token, no sign-up, no
rate limit a normal person would notice. This is the entire top of the funnel
and **nothing may be added to it**. Every gate here costs more reach than it
earns.

This is already built and running as `radar analyst`.

### 1. The token buys standing in the record, never the answer

This is the design constraint that keeps the product intact, and it is the same
separation the architecture already uses everywhere else:

| | who decides |
|---|---|
| what gets **looked at** | **holders** |
| what the numbers are | the instruments |
| what gets **said** | a rule, then a model, then two checks |

**Holders point the gun. They never touch the trigger.** A token that could
influence a verdict would destroy the record, which is the only asset here that
compounds.

### 2. The Hunt — the mechanism

Anyone can summon a roast, free, any time. **Holders nominate coins for a Deep
Hunt**: the expensive analysis — the creator's full history, the funding graph,
social-versus-on-chain divergence — published as a **standalone post from the
main account**, not a reply buried in someone's thread.

Nominations are ranked by holding-weighted votes. The top one gets hunted on a
schedule.

Why this specific mechanism, against the list of what people actually want:

- **Attention** — your nomination becomes a post the whole account sees. This is
  the real prize and it costs nothing to give.
- **Money** — not sold. Holding is what gets you standing, so the incentive is
  to hold rather than to flip.
- **Reward** — a public **hunter leaderboard**: nominations that turned out to
  be right. Scored on what the chain did afterwards, not on votes.
- **Mastery** — the leaderboard is a skill game with a real skill in it.
  Spotting a bad launch early is learnable, and the bot teaches it every time it
  answers. This is the most underrated item on the list and the stickiest.
- **Information / truth** — unchanged and uncorrupted, which is what makes the
  rest worth anything.
- **Losing less** — the hunts are genuinely protective. That is the honest pitch
  and it is the one that survives contact with a bad month.

### 3. What makes holding accrue, verifiably

The part where tokenomics can help without becoming the thing the bot exists to
expose. Both of these are **on-chain and checkable, so the bot can report on
them and anyone can verify**:

- **Creator fees to a public treasury.** pump.fun pays the creator a share of
  trading fees. Route them to one address, published, and let the bot report the
  balance and the spend on a schedule: *"this month the account cost $X to run
  and the treasury took $Y."*
- **Usage-linked burn.** A share of the treasury buys and burns on a published
  schedule, tied to how much the bot was actually used.

The point is not the deflation. It is that **the token's own claims are audited
by the product**. Nobody else in this category can say that, and it is a story
that writes itself weekly.

### 4. The events layer — the appointment

Virality is spiky. Appointments are what make it recur:

- **The Weekly Hunt** — top nomination, full teardown, same time every week.
- **The Wall** — a public page of every coin the bot flagged and what happened
  next, updated automatically. This is the record made visible, and it is the
  single most screenshot-able artefact the project can own: *"the bot said this
  thirty days ago."*
- **The self-audit** — the bot roasts `$WAR` on the same schedule and in the
  same format as everything else.

On the self-audit: **not first, and Josh is right that first is backwards.** The
bot exists and gets good; the token funds and amplifies it; the bot then covers
the token *as part of normal operation*, because someone mentioned it, like any
other coin. Making it the launch stunt would frame the whole project as being
about the token, which is the opposite of the pitch. Making it **recurring and
unexceptional** is what actually kills the hypocrisy charge.

## Whales, specifically

Whales are not looking for a newsletter. They want size, edge, and not to be
exit liquidity.

- Holding-weighted nominations give them real influence over what gets
  investigated, which is worth more to them than a discount.
- The alert they actually want is *"a thing you are already in has started
  behaving like the pattern"* — that is the anomaly engine, and it is the
  strongest reason for the social-divergence work.
- Their nominations being public makes them a signal other people follow, which
  is status, which is the thing they cannot buy elsewhere.

## What this deliberately does not do

- **No paid silence.** Not a tier, not a whitelist, not a "verified" badge that
  can be bought. The moment silence has a price, silence means nothing and
  speech is suspect. This was proposed and rejected.
- **No token-gated answers.** Gating the roast kills the funnel that makes the
  token worth holding.
- **No influence on verdicts.** Holders choose targets. Nothing else.
- **No price talk about `$WAR` from the bot**, ever. It reports the treasury and
  the burn, which are facts, and never the price, which is a solicitation.

## Where I think this is weakest

Stated plainly, because a proposal that only argues for itself is not worth
much.

**The audience may be structurally wrong for conversion.** The people who
`@`-mention a roaster under a memecoin are there to buy it, and the message is
often *don't*. `STATE.md`'s own base rate is that ~96% of pump.fun wallets lost
money or made under $500 — that is the population being acquired. Expect a large
audience that converts poorly on anything requiring spend.

That is survivable **only** if the flywheel runs on attention and status, which
are free, rather than on paid conversion. Everything above is built that way on
purpose, and if it is changed to run on paid conversion it should be expected to
fail.

**Being right does not travel on its own; funny does.** The accounts that win
are savage first. The drift risk is real, and the fidelity check is the thing
that makes drift impossible rather than merely discouraged — which is why it was
built before any of this.

**The leaderboard needs an honest scoring rule before it ships.** "Was the
nomination right" has to be defined in advance and computed by a rule, or it
becomes a popularity contest with the project's credibility attached to it.

## What would have to be built

Roughly in order, and none of it is started:

1. **Structural red flags in the fact sheet** — mint authority, freeze
   authority, LP lock status, creator balance and sells. Cheap on-chain reads,
   and the largest single upgrade to what the bot can say.
2. **The earned verdict** — harsher conclusions gated on how many of those flags
   are present, so brutality is justified by evidence rather than forbidden.
3. **The Wall** — the public record page. Mostly a renderer over the reply log
   that already exists.
4. **Social/on-chain divergence** — the genuinely new signal, and the one with
   real research risk.
5. **Nominations and the leaderboard** — needs the scoring rule settled first.
