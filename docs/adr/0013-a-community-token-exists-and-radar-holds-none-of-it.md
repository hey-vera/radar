<!-- SPDX-License-Identifier: Apache-2.0 -->
# ADR 0013 — A community token exists, and Radar holds none of it

**Date:** 2026-09-04
**Status:** accepted. **This is Josh's decision, recorded**, and it reverses
[GOAL.md](../../GOAL.md)'s "Radar will not launch one, ever" — written
2026-09-03 and edited in the same commit as this file.
**Decides:** whether a token is launched at all, what the operator may hold,
where the money goes, and what the analyst may say about it.

## Context

The public analyst's credibility rests on one measured fact: capital committed
*before* a token existed is visible in its launch block, and
[`0024`](../research/0024-the-spike-became-a-hump-and-the-signal-moved.md)
measures the distribution. So the standing argument against Radar launching a
token was structural rather than about intent, and it was right on its own
terms:

> "Any allocation to a team, a treasury or early supporters **is** that set of
> recipients. There is no launch design that escapes it."

Josh's decision is to launch one anyway, as the distribution engine for the
analyst, with the launch conditioned on the bot working first. Two things make
that decidable rather than a contradiction, and both are constraints rather
than arguments.

**The structural objection is about allocation, not about existence.** A launch
with no dev buy and no allocation has one recipient in its launch block: the
bonding curve. The objection names a *set of recipients*; the answer is to have
none. That is checkable by anyone, on chain, with the analyst's own instrument.

**The touting objection is about holding, not about launching.** It is the
sharper of the two and it is not dissolved by the above. It is answered by the
operator holding zero of the token, permanently, and by the analyst never
saying anything about its price.

## Decision

**A community token is launched on pump.fun. Radar — the product, the operator,
and anyone connected to either — holds none of it, ever.**

Six constraints. Each one answers a recorded objection, and together they are
the decision; the token without them is a different decision that this ADR does
not make.

1. **No dev buy, no allocation, no team or treasury tokens.** The launch block
   has exactly one recipient, the bonding curve. The analyst states this about
   the token the same way it states it about any other.
2. **The operator holds zero tokens.** The only flow to the operator is
   pump.fun's **creator fee — 30 bps of volume, in SOL**, read off the on-chain
   `FeeConfig` ([`0023`](../research/0023-the-fee-is-a-schedule-and-the-published-interface-is-incomplete.md)).
   That is the curve. After graduation the same fee program keeps a second
   schedule for the AMM, by market cap — 30 bps below 420 SOL, 95 from there
   to 1,470, stepping down to 5 above 98,240 — measured 2026-09-05 in
   [`0028`](../research/0028-the-fee-after-graduation-is-a-ladder.md). The
   flow is still to the prize, not the operator; only its rate moves.
3. **100% of the creator fee is paid out as a public weekly prize**, to the
   person whose summoned roast travelled furthest that week. The operator keeps
   none of it. The vault, the rule, the scores and the payout signature are all
   public.
4. **Entry is free and never requires holding the token.** Anyone who mentions
   the bot is entered.
5. **The analyst never states the token's price or market capitalisation.** It
   reports the vault balance, the prize, the winner and the transaction — facts
   about money that moved. This is enforced rather than instructed: the mint is
   configured as `RADAR_SELF_MINT`, and a price or market-cap fact for that mint
   is dropped from the fact sheet **before the model sees it**, so the number is
   not in the set the fidelity check would authorise.
6. **The token is roasted like anything else**, on the same rule and the same
   schedule, when somebody asks. Not as a launch stunt — recurring and
   unexceptional is what makes it mean anything.

## What this costs, stated plainly

**The touting objection is reduced, not eliminated.** The operator holds no
tokens, but the creator fee rises with volume, and the analyst's reach
influences volume. The exposure is to *trading activity* rather than to
*price*, which is a weaker and slower coupling than holding — and it is not
zero. Constraints 3 and 5 exist because of this: the fee is not kept, and the
bot never says the thing that would move a price.

**A legal read is a precondition, not a follow-up.** Two questions gate the
first post, and neither is answerable here: whether an automated account
publishing measured facts about specific tokens is regulated financial
promotion in this jurisdiction, and what entity and terms sit behind it. Design
0007 section 3 row J4 is where that decision sits. **This ADR does not clear
it**, and accepting this ADR is not accepting that it has been cleared.

**The prize is small until volume is not.** At $10k of weekly volume the prize
is roughly **$30**; at $1M it is roughly **$3,000**. This paragraph said $3
and $300 until 2026-09-05 — 30 bps read as 0.03% rather than 0.30%, wrong by
10× in the direction that understates; corrected by
[design 0009](../design/0009-three-loops-and-no-formula.md) §1, and the pool
page carries the same correction in a follow-up. A token that oversells its
own economics is the thing the analyst exists to expose.

## What this commits to

- The constraints above, in code where code can hold them: constraint 5 in
  [`crates/radar-roast/src/forbidden.rs`](../../crates/radar-roast/src/forbidden.rs)
  and [`sheet.rs`](../../crates/radar-roast/src/sheet.rs); constraints 3 and 4
  in the contest crate and its payout policy.
- The payout key is **not** the trading signer, does not touch
  [`crates/radar-risk`](../../crates/radar-risk), and never holds customer
  funds. Its blast radius is one week of creator fees.
- `GOAL.md`'s "What Radar will not become" keeps a token bullet, rewritten to
  the thing that is actually forbidden: **holding** a token it comments on.

## What this does not decide

- **When.** The launch is gated on the analyst clearing its 30-day
  demand gate (design 0007 section 5). This ADR says what is launched, not that
  it is launched now.
- **The on-chain claim program.** Paying from a hot key is the v1. A time-locked
  claim program is a separate ADR, gated on the weekly fee being large enough
  to justify an audit.
- **Anything about the trading lane.** `Policy::CLOSED` is untouched by all of
  this, and nothing here is evidence about an edge.
