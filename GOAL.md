<!-- SPDX-License-Identifier: Apache-2.0 -->
# GOAL.md — what Radar is for

**This file is the product, in plain terms.** It is Josh's document. If anything
here is wrong, an AI model working on Radar should say so rather than build
around it. If Josh decides something different, this file gets updated to say
what Radar is *actually* supposed to be — not what it was.

It deliberately holds **no** engineering invariants, no build commands and no
crate notes. Those live in [AGENTS.md](AGENTS.md), and an earlier version of this
file was a verbatim copy of that one — which meant two documents saying the same
thing, free to drift, with nothing telling a reader which was authoritative.
One subject each.

---

## The one-sentence version

**Radar is an evidence-driven autonomous trading system whose AI can reason and
execute, but whose execution is constrained by measurable edge, explicit customer
authorisation, and deterministic risk controls.**

## What it is optimising for

**More profit and less loss for the customer, over time and in the moment.**

Both halves are load-bearing. "Over time" rules out a strategy that wins often
and ruins you once. "In the moment" rules out one that is theoretically sound and
unexecutable at the size and speed the venue actually offers — which is the trap
`0018` found Radar already in, sizing positions the market could not absorb.

## The AI is a source of edge, not only a risk to be contained

This is the part most easily lost when writing down the safety rules, so it is
stated first: **the AI is expected to find edge that the deterministic strategy
does not.** It reasons over the signals, the launch structure, the creator
history and the execution costs together, and the claim is that reasoning across
them yields something none of them yields alone.

That is the product. The constraints exist so the claim can be tested with real
money without the failure mode being ruinous.

**"Reason and execute" and "never authorise" are not in tension**, and the
distinction is the whole design:

| | Who |
|---|---|
| Decide what looks worth doing | **the AI** — reasoning over everything available |
| Decide whether it is permitted | the deterministic risk kernel, purely |
| Decide whether to sign it | the separate signer, re-reading the bytes |
| Grant the authority at all | **the customer**, revocably |

The AI drives. It does not also hold the brake, and it cannot remove it. See
[AGENTS.md](AGENTS.md) rule 1 for why that boundary is not negotiable — an AI
that could authorise its own trades is a system whose worst day is unbounded.

**The AI is measured, not trusted.** Whatever it proposes is held to the same bar
as everything else: roughly 456 bps of expected edge before a trade is worth
making at all. An AI strategy that cannot clear it does not get to trade because
it is an AI.

## Who it is for — two modes, each with two ways to sign

The mode says **where the judgement comes from**. Signing says **who presses
go**. They are separate choices and a customer makes both.

### Signal mode — no AI

Radar's deterministic strategy decides. What a customer gets is the evidence and
the reasoning, unchanged from what the recorder holds.

- **Manual** — the customer signs every trade Radar offers.
- **Automated** — a signed session signs on their behalf.
- **Risk level** — a threshold that keeps high-risk trades from being offered at
  all, and from being auto-signed if they are.

### AI mode

A model reasons over the same tools and evidence, and can go looking for more.

- **Manual** and **automated**, the same two ways to sign.
- The model may decide the evidence is not enough and **ask for more** — pull the
  launch block, check the creator's other launches, look at who funded them —
  and then refuse on what it finds.

**The AI is the safety officer and the expert trader at once**, and the two are
the same job: deciding what not to touch is most of trading.

### Talking to it

AI mode is a conversation, not a dashboard.

- *"Look at $COIN, the narrative seems good."* The model investigates, reasons,
  answers — and may start trading it.
- *"Stop trading that one, I found proof it is bundled."*
- *"Stop everything, right now."*
- *"Do all my open trades have stop losses? I want to sleep."*

**This is a product for a person, not a well-built machine that happens to have a
screen.** A customer who cannot ask it a question in their own words, or stop it
in one sentence, does not actually control it — whatever the settings page says.

### Two consequences that are not negotiable

**A stop command must never travel through the model.** "Stop everything" has to
revoke the session directly, at the same layer that grants it. If stopping means
*asking the AI to stop*, then a confused or wedged model is a customer who cannot
stop it — precisely when they most need to. The kill switch works when the AI is
broken, or it is not a kill switch.

**The AI may only subtract.** It can refuse what the risk kernel would allow. It
can never allow what the kernel refuses, and neither can a risk level, a
conversation, or a customer insisting. Refusals compose; permissions do not.
[AGENTS.md](AGENTS.md) rule 1 is the code form of this.

## What makes it different

Every competitor shows what is going up. **Radar shows what it refused, and can
tell you exactly why** — because every decision it has ever made carries a reason
list and can be replayed at the watermark it was taken under.

That is the asset. Roughly 4,400 recorded decisions that a human clicking *buy*
would never have produced.

## The honest state, which is the point rather than an apology

**Radar has not found an edge yet, and it says so.**

- Measured selection edge: **0 bps**
  ([research 0017](docs/research/0017-a-control-that-could-have-been-traded.md)).
- The bar a strategy has to clear before a single trade is worth making:
  **~456 bps** of expected edge, and ~850 before a position larger than about $59
  makes sense ([research 0022](docs/research/0022-capacity-was-a-budget-not-a-ceiling.md)).
- The shipped policy refuses everything, and nothing has ever traded.

A product that claimed an edge it could not show would be the ordinary thing to
build. The base rate is that ~96% of pump.fun wallets lose money or make under
$500, and most tools serving them are selling the feeling of an edge. **Radar's
entire pitch is that it will tell you the truth about its own performance**, and
that only means something if it does so while the truth is unflattering.

## What "working" would look like

In order. Nothing below is skippable, and nothing later is worth building first.

1. **A measured edge above ~456 bps** in a stratum Radar can actually size into.
   Everything else is plumbing until this exists.
2. **A customer can connect a wallet, see the decisions, and act on them.**
3. **A customer can grant bounded autonomy** and revoke it, and Radar trades
   within it.
4. **The record shows it worked** — realised, net of the round trip, against a
   control that could have been traded.

## The public analyst — receipts in the reply

**A Radar account on X that anyone can @-mention under a memecoin post, which
answers with what Radar measured.** Not an opinion, not a rating: the evidence.

The tone is a roaster and the function is a cabal hunter. Somebody asks, in
public, whether a coin and its community are sound — and everybody watching sees
the answer at the same time. Being *funny* is a distribution strategy; being
*right* is the product.

> Six recipients in the launch block. The creator has twelve prior launches and
> none graduated. Exit capacity at 1% impact is $31, against a 250 bps round
> trip.

Why this belongs in the goal rather than in a marketing plan: **it is the one
product that works before an edge exists.** Radar can be genuinely useful and
completely honest today, because "this launch was bundled" is a measured fact
about a block, not a prediction about a price. And the flywheel is the strongest
demonstration available of the thing that makes Radar different — everyone else
gives opinions, Radar gives receipts.

It is also the cheapest possible test of whether anyone wants what Radar knows.

**What exists and what does not**, because "we mostly have this already" is the
kind of assumption that turns a week into a quarter:

| | |
|---|---|
| launch-block bundle detection | **exists** |
| creator history, exit capacity, curve pricing | **exists** |
| bundling detected *after* launch | partly — the detector is written, the rolling block source is not |
| sniper counts, bot counts | **do not exist** |
| any X or social data at all | **does not exist** |

So the on-chain half is real and the social half is zero.

**Three constraints, to be designed in rather than discovered:**

- **It states only what was measured.** Never "this is a scam" — always the
  count, the history, the number. Safer, and more differentiating.
- **Mentions are untrusted input.** An account that reads replies and answers
  with a model is a prompt-injection surface, and somebody will try it on day
  one. [AGENTS.md](AGENTS.md) rule 4 is not optional here.
- **The flywheel points at a token's price, and that is a legal question before
  it is a product one.** Attention drives buying. If Radar, or anyone connected
  to it, holds or is paid to mention a token it comments on, that is promotion
  wearing analysis's clothes — and the rules about touting do not care that the
  facts were true. Get advice on this before the account posts once, not after
  it works.
- **It is public, automated, and about identifiable people's projects.** A wrong
  accusation at scale is a different risk from a wrong number on a private
  dashboard. What it may assert needs bounds before it posts anything.

## What Radar will not become

Each of these is a decision, not an oversight.

- **A momentum feed.** "Top gainers" on this venue ranks tokens whose curve is
  being bought out by people who were committed before the token existed. It
  ranks traps by how attractive the trap looks.
- **A custodian.** The customer holds their own funds. Radar is at most a
  bounded, revocable signer, and never the owner.
- **A backtest simulator.** Letting a user hunt for the stratum where the numbers
  look good is how a null result gets sold as a strategy.
- **A single safety score.** Radar has fourteen reason codes and a structural
  split. A green shield is "unknown rendered as safe".
- **Something that trades faster than it can explain.** If a decision cannot be
  replayed and justified, it does not get made.

## The direction of travel

**Venues.** Radar records pump.fun only. The ceiling there is capacity, not
signal — the median position is about $6. Established tokens on Raydium and
Whirlpool do not have that ceiling, and route in a form the signer can already
read. Broadening is worth doing *after* an edge exists, because venues make a
working edge bigger and do not create one.

**The AI.** Today the strategy is deterministic rules. The next step is a model
reasoning over the same evidence and proposing what the rules do not see — held
to the same bar, and authorised the same way. Nothing about that path needs the
boundary in rule 1 to move.

---

## For a model reading this

If you find something here contradicted by what is actually in the repository,
**say so in your next message rather than quietly working around it**. A stale
goal is worse than an absent one, because it looks like a decision.

The specific things most likely to go stale: the measured edge, the bar, which
venues are recorded, and whether anything has traded. All four are claims about
the world, and all four are checkable.
