<!-- SPDX-License-Identifier: Apache-2.0 -->
# Design 0009 — Three loops, and no formula

**Date:** 2026-09-05
**Status:** **decided, 2026-09-05.** Six decisions in section 3 were Josh's.
On 2026-09-05 he delegated all six to the author with one instruction — choose
what benefits the project in the long term — so the picks in section 3 are the
decisions, recorded as his by delegation. Each row keeps its options and its
reasoning, because a decision without its reasoning is the thing this
repository refuses to record. Written in plan mode
as `workstation:~/.claude/plans/radar-cabal-hunter-logical-kernighan.md` and
moved here in the first commit of `docs/0009-three-loops-and-no-formula`,
for the reason design 0007 gives: a plan outside the repository is invisible to
`repo-conformance` and did not happen. Two lines in section 1 changed on the
way in, because PR #142 merged while this was being written and the commit
they cite moved. Section 10 is where it is weakest, and
it is the section to read twice.

**What this answers.** Josh wants reinforcing loops for a memecoin whose
product is a public X bot that answers questions about Solana launches with
measured facts. He has one loop. He asked for more, one of them prize-shaped,
and for "whatever the golden formula is for the most successful memecoin."
Section 2 answers the formula question honestly. Section 4 gives three loops.
Sections 5 to 7 say what each needs built.

**What this does not reopen.** ADR 0013's six constraints, the rejected
operator buy-and-burn, the summoned-reply-only rule, no social or identity
scraping, and the x402 lane staying out of the analyst path. Where a
recommendation below would need a constraint amended, it says so and stops.

---

## 1. The honest state, checked 2026-09-05

Everything here was read off the repository at `a952133`, off GitHub, or off
the brief for this document, which carried figures from the creator index on
the production box at slot 444,374,676 on 2026-09-04. I could not see the box
myself; the same limit design 0007 §12 records.

**What exists.**

| thing | state |
|---|---|
| The analyst | code-complete, posting nothing ([STATE.md](../STATE.md)). The X client, OAuth 1.0a, the poll loop and the daemon are written; PR #142, the OAuth half, merged as `a952133` while this was being written |
| The admission gate | built and pure: a per-summoner daily cap, a global daily cap, a dedupe window, an ignore list, and **refuses everything when unconfigured** — [`crates/radar-analyst/src/admission.rs`](../../crates/radar-analyst/src/admission.rs) |
| The public site | phase 0 merged (#139). Home is real; the leaderboard and the pool are honest empty states ([design 0008](0008-the-public-site.md) §5). The OG image is missing; the three public endpoints are not built ([plan 0005](../plans/0005-cabal-hunter-goes-online.md)) |
| The contest crate | **does not exist.** `new:crates/radar-contest/` is design 0007 C1 and design 0008 phase 2, blocked on the X credential |
| The fee | 30 bps of curve volume to the creator, read off the on-chain schedule ([research 0023](../research/0023-the-fee-is-a-schedule-and-the-published-interface-is-incomplete.md)). **Post-graduation is unmeasured** — see the box below |
| Telegram | in the deploy path as the recommended **alert** channel only ([`deploy/README.md`](../../deploy/README.md)). The analyst has no Telegram publisher |

**One thing that bit, and is fixed.** [ADR 0013](../adr/0013-a-community-token-exists-and-radar-holds-none-of-it.md)
constraint 5 says the token's price is kept out of the fact sheet "enforced
rather than instructed", via `RADAR_SELF_MINT`. When this was written, **no
crate contained that name**: design 0007 C8 listed it as something to build,
and the ADR read as if it were built — LEARNINGS 9's shape, an invariant
documented as stronger than its enforcement, gating the token launch. **Built
the same day**, in the PR that merged just after this one: every fact on the
sheet carries a tag, and the sheet drops price facts for the configured mint
before the model sees them. [STATE.md](../STATE.md) carries what it does and
what it does not yet have to do; where this paragraph and STATE.md disagree,
STATE.md is right.

**A second thing, and it is arithmetic.** Four places say the prize at $10k
of weekly volume is about $3: ADR 0013's "What this costs", design 0007 §6.1,
design 0008 §5.3 and the pool page. 30 bps is 0.30%, and 0.30% of $10,000 is
**$30**. The figure was written as if 30 bps were 0.03%, and it was copied
three times without anyone multiplying — including into the brief for this
document, which is how I first read it. Wrong by 10× in the direction that
understates the token's own economics, which is the safe direction and is
still wrong. The three documents are corrected in the same PR as this one;
the page is a one-string follow-up that lands before the domain does. Said
once, plainly, per AGENTS.md §2, and the corrected figures are used below.

**The figures this document uses.** Every number below comes from one of these
rows or is marked as an assumption or an external claim.

| figure | value | source |
|---|---|---|
| launches recorded, succeeded | 508,814 | creator index, 2026-09-04 (the brief) |
| creators watched | 116,752 | same |
| outcomes measured | 506,991 | same |
| graduate at all | **2.81%** | same |
| graduate instantly — bought out within three slots | **1.03%** | same. The committed snapshot has 0.96% on 483,629 at 2026-09-03 ([`0024-base-rates.json`](../research/data/0024-base-rates.json)) |
| show almost no activity | **23.0%** | same |
| launch block of 1–3 recipients | 70.5% of launches; 0.02% graduate instantly | [research 0024](../research/0024-the-spike-became-a-hump-and-the-signal-moved.md), 17,497 launches sampled 2026-08-25 |
| launch block of 10–13 recipients | 2.1% of launches; **10.1×** the base rate of instant graduation | same, and weaker than the rows above because it is sampled |
| organic graduations, held to the end | median **−3,228 bps** | [research 0011](../research/0011-graduation-predicts-volatility-not-profit.md) |
| instant graduations, held to the end | median **−5,981 bps** | same |
| measured selection edge | **0 bps** | [research 0017](../research/0017-a-control-that-could-have-been-traded.md) |
| the bar to make one trade | **~456 bps** | [research 0022](../research/0022-capacity-was-a-budget-not-a-ceiling.md) |
| pump.fun wallets that lost money or made under $500, April 2026 | ~96% | [STATE.md](../STATE.md) |
| creator fee on the curve | 30 bps of volume, in SOL | research 0023 |
| prize at $10k / $100k / $1M weekly volume | **~$30 / ~$300 / ~$3,000** | arithmetic on 30 bps. ADR 0013, design 0007 §6.1, design 0008 §5.3 and [`site/src/Pool.tsx`](../../site/src/Pool.tsx) all say ~$3 / ~$30 / ~$300, which is 30 bps read as 0.03%. **Wrong by 10×** — see above |
| X summoned reply / top-level post | $0.010 / $0.015 | design 0007 §11, verified 2026-09-03 |
| X rate limits | 10,000 posts per 24 h; 450 mention reads per 15 min | same |
| Helius free tier | 1M credits a month; a dossier is 35–150 credits, so 6,000–28,000 dossiers a month | same |
| all-in before viral | under $50 a month | same |
| Telegram Bot API | free; about one message a second per chat, 20 a minute per group, about 30 a second in bulk | core.telegram.org/bots/faq, read 2026-09-05 |

**The fee after graduation, which nothing here has measured.** ADR 0013 and
the pool page say 30 bps. That is the curve. pump.fun's own fee article
(updated 2025-09-26) claims the creator keeps earning after graduation on a
schedule keyed to market cap: 30 bps below 420 SOL of market cap, **95 bps**
from 420 to 1,470 SOL, stepping down to 5 bps above 98,000 SOL. Radar's read
of the on-chain schedule on 2026-09-01 found one flat tier for the curve and
parsed, but did not use, the fees for other pools
([`crates/radar-pumpfun/src/fees.rs`](../../crates/radar-pumpfun/src/fees.rs)).
A reference and a capture disagree here, which is LEARNINGS 25, and this
repository has already found the venue's global account and its fee config
disagreeing with each other once. **Measure it before any page states a
post-graduation figure** (section 5, M6). Until then the prize arithmetic is
right for the curve and unknown after it, in either direction.

---

## 2. The golden formula, answered

**There is no formula the operator of this product can run.** Here is what
the evidence says, in three steps.

**Step one: the measured formula for graduating on this venue is
coordination.** A launch block with 10–13 recipients is 10.1× likelier to be
bought out in three slots than the average launch; the 70.5% of launches with
1–3 recipients almost never are (research 0024). Two studies outside this
repository, on different data, say the same thing in different words, and
both are external claims this repository has not verified against the chain:

- A study of 655,770 tokens launched in September 2025 found 0.63% graduated,
  and the strongest predictor was **reaching a given curve depth in fewer
  trades** — a few large early buys. Creator identity was a weak signal.
  92.22% of tokens with at least 30 swaps carried a dump event, and a naive
  buy-and-hold was unprofitable under nearly every conditioning
  (arXiv 2602.14860).
- A study of 832,941 launches from May to June 2026 found the strongest
  single predictor was **the creator buying their own token at launch**
  (hazard ratio 4.51), and a Telegram link at launch lifted graduation from
  0.166% to 1.485%. Its observation window was about six minutes, so what it
  mostly measures is the instant cohort — the coordinated one
  (arXiv 2607.02823).

So the published "success factors" are a dev buy, a few big wallets on the
first block, and socials attached before launch. **ADR 0013 constraint 1
forbids the first two by decision.** The third — an account, a site and a
community that exist before the token — Cabal Hunter has by construction, and
it is a fact to note rather than a lever to pull.

**Step two: the formula does not even win for the people who follow it.**
Organic graduations end at a median of −3,228 bps and instant ones at −5,981
(research 0011). Graduating is a volatility signal that has been read as a
profit signal, and the biggest memecoins are survivors of that distribution,
not evidence about its mean. The base rate under all of it is that about 96%
of wallets on this venue lose money or make under $500.

**Step three: what the very largest memecoins had is not a mechanism.** This
is my reading and not a measurement: a character people want to wear, a
moment nobody planned, and a crowd that made content for free. None of it can
be scheduled. Anyone selling a formula for it is selling the feeling of an
edge, which is the thing [GOAL.md](../../GOAL.md) says this product exists to
refuse.

**So what raises the ceiling.** Three things, and all three are the bot, not
the token:

1. **Reach.** The token's ceiling is the bot's ceiling. Every hour spent on
   the token instead of the bot lowers both.
2. **The record.** Every reply is a dated public statement about a specific
   coin, made before anyone knew what happened next. It compounds, it cannot
   be faked backwards, and nobody can start one without waiting as long as we
   did. Design 0001 said this and it is still the whole argument.
3. **The appointment.** Virality is a spike; a fixed time every day and every
   week is what makes people come back when there is no spike. Section 7.

**What the token is, said plainly so the page can say it.** A badge for people
who like the bot, with a prize pool attached that is a public fact about the
token's own economics. Not an investment, not a utility, not a governance
right. Design 0001 §"Where I think this is weakest" already said the audience
converts poorly on anything requiring spend, and that the design survives only
if it runs on attention and status. Everything below is built that way.

---

## 3. Decisions that are Josh's

Each row: the decision, the real options, **my pick** and why, and the
decision line. Numbered L1–L6 so they do not collide with design 0007's J-rows
or design 0008's K-rows.

**How these were decided.** The rows were written as recommendations with an
assumed answer. Josh read them on 2026-09-05 and delegated all six, asking for
the choices that benefit the project in the long term. The picks stood on that
test — each one trades a short-term lever for a slower thing that compounds:
the record over the price, status over cash, a clean bio sentence over half a
fee, one mechanism over two. So the "decided" lines below are the picks, and
they are Josh's by delegation. Reopening one is a design note, not a chat.

### L1 — Does the creator fee go 100% to the prize, or 50/50 with the operator?

ADR 0013 constraint 3 says 100%. Josh proposed 50/50, so the question was his
to reopen and this row treats it as open. **Decided 2026-09-05: (a). ADR 0013
constraint 3 stands unchanged.**

| option | what it is |
|---|---|
| (a) | 100% to the prize, as ADR 0013 says |
| (b) | 50/50, the operator's half as income |
| (c) | the bot's measured running cost first, published, zero margin; the remainder is the prize |

**My pick: (a).** Four reasons.

- **The split is irrelevant until it is dangerous.** At $10k a week of volume
  the fee is about $30 and half of it is $15; the bot costs under $50 a month
  either way and Josh pays most of it. The split only means something above
  roughly $100k a week, where half the fee is about $150 a week and climbing
  with every reply the bot posts — and that is exactly where
  the coupling ADR 0013 calls "reduced, not eliminated" becomes the operator
  being paid by volume in a token his bot's reach moves. Section 8 has the
  regulators' own words on promoter effort; I am not a lawyer and neither is
  Josh, and (b) is the option that most needs one.
- **It interacts with L2.** 50/50 plus any token utility means the operator
  earns from volume in a token he also designed a demand mechanism for. Each
  is defensible alone; together they are the shape the ADR was written to
  avoid.
- **The sentence fits in a bio.** "The operator keeps nothing" is checkable in
  one transaction. "The operator keeps half" needs a paragraph, and the
  paragraph is on a page whose whole pitch is that the token does not lie
  about its economics.
- **If Josh wants revenue, the honest source is a product priced in SOL or
  USDC, not a slice of the memecoin.** The x402 instrument surface already
  exists in [`crates/radar-serve/src/facilitator.rs`](../../crates/radar-serve/src/facilitator.rs)
  and is public by exact path in [`access.rs`](../../crates/radar-serve/src/access.rs).
  Revenue from customers does not move with the token's volume. That is
  design 0007 §7 and its manual-sign product, and it needs nothing from this
  document.

**Why (c) is tempting and still not my pick.** It answers "why should I run
this for free" without creating a profit coupling, and it is the design 0001
§3 sentence that survived: "this month the account cost $X and the treasury
took $Y." But below about $4,000 a week of volume the cost exceeds the fee
(assumption: about $12 a week of running cost against 30 bps), so (c) means
**no prize at all** in a quiet week, and a contest that pays nothing on a
quiet week is worse than one that pays $30. If Josh wants (c) later, it is a
policy line in the payout binary and one sentence on the page, and it is
cleaner than (b).

**If Josh picks (b) or (c):** ADR 0013 constraint 3 is amended in the same
PR, and [`site/src/Pool.tsx`](../../site/src/Pool.tsx),
[`site/src/About.tsx`](../../site/src/About.tsx) and
[`site/src/empty.test.tsx`](../../site/src/empty.test.tsx) change in the same
commit, because all three state 100%. News reports of 2026-01-09 (external,
unverified) say pump.fun now splits creator fees across up to ten wallets on
chain; if that is true a split is venue configuration rather than payout code,
and it is publicly readable, which is the only form of (b) I would build.

**Decided, 2026-09-05, by delegation: (a).**

### L2 — Should users be able to burn their own tokens for faster or higher-volume answers?

This is **not** the mechanism design 0001's header rejected. That one had the
operator buying and burning, which means holding. Here the holder burns, the
operator never touches a token, and ADR 0013's six constraints all survive
word for word. The question is whether it is worth building.

**My pick: no.** Four reasons, and the first is arithmetic.

1. **A burn pays nobody.** The only scarce things in this product are paid X
   replies at $0.010 and RPC above the free tier. A burn funds neither. If
   the free lane ever hits the Helius ceiling, burn-for-access makes the bill
   worse — more demand, no revenue — and the correct answer is x402 in USDC
   on the dossier instrument, with the analyst path untouched.
2. **It gates something that is not scarce.** A Telegram answer costs
   nothing to send and fractions of a cent to compute. Below 6,000–28,000
   dossiers a month there is nothing to ration. A gate on a free thing is a
   gate.
3. **It gives the token a use, and "no use" is the description the regulators
   wrote down.** The SEC staff statement of 2025-02-27 describes meme coins
   as having "limited or no use or functionality"; the UK's PS23/6 regime bans
   benefits based on cryptoassets held. Burn-for-access is a benefit for
   having held. Section 8 says more; a lawyer says the rest.
4. **It makes the operator the designer of the token's supply sink.** Not a
   holder, not a buyer — but the author of a lever. ADR 0013's "What this
   costs" would need a new paragraph, and the bio sentence gets longer again.

**What it would cost ADR 0013 if built anyway:** nothing in the letter of the
six constraints; the spirit of "the operator has no lever on this token."

**Revisit trigger:** never for funding. If holders ask for a way to signal
support, the cheapest honest answer is a plain "supporters" list on the site
that costs nothing to serve and gives nothing back — and even that goes to the
lawyer first.

**Decided, 2026-09-05, by delegation: not built.**

### L3 — Is "The Hunt" still the mechanism, given the contest exists?

[Design 0001](0001-the-flywheel.md) §2 has holders nominate coins by
holding-weighted vote for a Deep Hunt published as a standalone post, plus a
hunter leaderboard. [Design 0007](0007-the-end-to-end-plan.md) §6 has anyone
who summons the bot entered in a weekly contest scored on the bot's own
reply's engagement, with the creator fee as the prize. They overlap: both are
"bring the bot a coin worth looking at" and both end in a leaderboard.

**My pick: one mechanism. The contest is the entry and the money; The Hunt is
what the winner gets.**

- **Keep** from The Hunt: the standalone teardown post from the main account.
  It is the attention prize, it costs $0.015, and design 0007 already budgets
  a weekly top-level post (B6, C8). Make that post the full dossier of the
  winning coin — creator history, launch block, curve — with the winner's
  handle on it.
- **Keep** the hunter leaderboard, as status only, with a rule that is honest
  (section 5, M3). Design 0001 said the rule had to be settled before it
  shipped; it was not, and it is settled below as a proposal.
- **Drop** holder-weighted nominations. A summon is a nomination, so the
  contest already has them. Weighting by holding needs a wallet-to-X link and
  a vote count, and it gives the token a governance-shaped use — L2's
  problem again — for a prize that is attention either way.

Design 0001's header gets a third bullet recording the drop, in the same
commit as this document.

**Decided, 2026-09-05, by delegation: this.**

### L4 — A $3 weekly prize motivates nobody. What is the prize?

First the correction from section 1: at $10k of weekly volume it is about
$30, not $3. Ten times better and still not money anyone changes their week
for. The question stands.

**My pick: status is the prize; money is the receipt.**

The winner gets three things, and none of them is cash:

1. **Their coin torn down in the weekly post**, from the main account, with
   their handle on it. Design 0001 called attention "the real prize and it
   costs nothing to give," and that is still true.
2. **Their handle on the site's leaderboard** and in the weekly post.
3. **Rank** — a hunter score that persists across weeks (M3). Winning a week
   is an event; being ranked is a reason to look tomorrow.

The money stays exactly as design 0007 J2 decided: weekly, a 0.1 SOL floor,
rollover until the floor is reached. Show the pot filling. At $30 a week the
floor is met in the first week at any SOL price under $300 (a condition, not
a price claim), so rollover matters only in quiet weeks — and a pot visibly
growing toward a line in a quiet week is itself a check-back reason.

**Two things not to do.** Do not top the pot up from Josh's pocket: it hides
the token's real economics, which is the thing the bot exists to expose. Do
not accept donations into it: it changes C4's payout policy (`amount ≤
collected`) and turns "100% of the fee" into "the fee plus whatever", which is
a longer sentence again.

**Decided, 2026-09-05, by delegation: this.**

### L5 — Telegram as a second surface: what belongs where?

**My pick: X is the public record and the contest. Telegram is the free lane
and the community's room.** The two are not the same product wearing two
coats, and the split is by what each surface is for:

| | X | Telegram |
|---|---|---|
| cost per answer | $0.010 | nothing |
| who sees the answer | everyone | the asker, or the group |
| in the record | **yes** — dated, public, checkable | no |
| contest entry | **yes** — the score reads the reply's public metrics | no — nothing public to score |
| the daily post | yes, $0.015 | yes, free, in the channel |
| the weekly teardown | yes | yes, free |
| rate limit that binds | 10,000 posts a day, the gate | about one message a second per chat, 20 a minute per group |

Same parser (only a base58 mint or a `$TICKER` survives,
[`crates/radar-analyst/src/mention.rs`](../../crates/radar-analyst/src/mention.rs)),
same admission gate, same fact path, same two checks on the text. Only the
transport differs, so the injection defence is unchanged: a Telegram message
is untrusted input on the same rule as an X mention. The Telegram bot answers
only when spoken to, which is the summoned rule in a different room.

**What Telegram adds that X cannot:** volume without a bill. Somebody who
wants to check twenty coins a day does it there for free instead of hitting
the X gate. That is where the "higher volume" demand from L2 actually goes,
and it costs nothing.

**What Telegram cannot add:** the record. A private answer is not a public
call. If Telegram grows faster than X, the product's public half is the
smaller half, and that is section 10's fifth weakness.

**Decided, 2026-09-05, by delegation: this.**

### L6 — What, concretely, makes somebody check back daily?

**My pick: one daily post at a fixed time, called "seven days later."**

It lists the coins the bot was summoned about seven days ago and what the
chain did since, from the recorder's own outcomes: graduated or not, bought
out in three slots or not, still trading or dead, last price against the
price at the time of the reply. Underneath: yesterday's launch count, how
many were in the 1–3 band, and the pot.

Why this and not a digest of numbers: it is the record loop with a daily beat,
it is a fact nobody else can post, it is funny when the coin died and useful
when it did not, and it is the mechanism by which the bot's calls age in
public — which is the "hit rate" the ChatGPT notes asked for, done the only
honest way. $0.015 a day on X, about $0.45 a month; free on Telegram.

Secondary: the hunter rank moving (M3), and the pot filling (L4). Both are
numbers that change daily and are on the site.

**Decided, 2026-09-05, by delegation: this.**

---

## 4. The three loops

```
 loop 1 — attention (Josh's; exists; runs in both directions at the same speed)

   reply seen ──▶ bio ──▶ buy ──▶ volume ──▶ talk ──▶ more mentions ──▶ reply seen


 loop 2 — the record (design 0001; runs one way; the moat)

   summons ──▶ dated public reply ──▶ "seven days later" ──▶ credibility ──▶ summons
                                       (nobody else can post this step)


 loop 3 — the prize (design 0007 §6 with The Hunt folded in; runs on status)

   summons ──▶ the week's best reply ──▶ the fee, the teardown post, the rank ──▶ summons
```

All three take the same input, a summons, and all three emit public facts.
Nothing in any of them asks anyone to trust Josh, mention a price, or hold the
token.

**Loop 1 is the only one the token needs, and this document leaves it alone
on purpose.** Design 0001 said it: the attention loop is real, it is not a
moat, and it runs backwards at the same speed. Every intervention that would
strengthen it directly — an allocation, a buy, a burn, a gate — is refused
above. Loops 2 and 3 make the bot better; the token rides.

**Loop 2 is the one that compounds**, and its new piece is the daily beat.
Design 0001 had "The Wall" as a page; L6 makes it a post, which is the
difference between a thing people can visit and a thing that arrives.

**Loop 3 is the prize-shaped loop Josh asked for.** It is design 0007 §6 as
already decided, with The Hunt's teardown post as the prize and a rank that
outlives the week. It runs on status because the money is a receipt.

---

## 5. Mechanisms, and what each requires

In the order I would build them. Only M6 and M5 are unblocked today.

| # | mechanism | exists | to build | files | gate |
|---|---|---|---|---|---|
| M6 | **Measure the post-graduation fee.** Read the fee for a graduated mint off the chain, the way research 0023 read the curve's; write the result up | the parser: `flat` and `tiers` in [`fees.rs`](../../crates/radar-pumpfun/src/fees.rs) | one read, one research note `new:docs/research/0028-…md` | `crates/radar-pumpfun/src/fees.rs` | none. Do it before the pool page states any post-graduation figure |
| M5 | **Telegram publisher.** Long-poll `getUpdates`; each message → `mention::read` → `Gate::admit` → dossier → roast → log → reply. Unset token ⇒ nothing (rule 8). Not a contest entry, not in the record | parser, gate, fact path, `Publisher` trait in [`publish.rs`](../../crates/radar-analyst/src/publish.rs) | `new:crates/radar-analyst/src/telegram.rs`; a second env file; a `radar brief` check | `deploy/README.md` | a bot token from Josh via BotFather, five minutes. A **different** bot from the alert one |
| M1 | **The contest**, exactly as design 0007 §6.3 C1–C8 | nothing; `new:crates/radar-contest/` | C1–C8 unchanged | as 0007 | the X credential; ADR 0013; C8's self-mint rule **built** |
| M4 | **"Seven days later."** Once a day: replies seven days old from the log, joined once against the store's outcomes on the creator-index timer's pattern, rendered as one post to X and Telegram | the reply log ([`log.rs`](../../crates/radar-analyst/src/log.rs)); the outcomes; the timer pattern in [`deploy/radar-creator-index.service`](../../deploy/radar-creator-index.service) | `new:crates/radar-analyst/src/daily.rs`; the same fidelity and forbidden checks on the text | — | the bot live for seven days. Day one has nothing to say and says so |
| M2 | **The weekly teardown post.** The winner's mint gets the full dossier as the week's top-level post, with the winner's handle; the vault, the prize and the transaction in the same post (C8) | B6's weekly post is planned, not written | `new:crates/radar-analyst/src/weekly.rs` as 0007 B6 names it | — | M1 |
| M3 | **The hunter rank.** Status only. Rule below | the signals are on the sheet ([`sheet.rs`](../../crates/radar-roast/src/sheet.rs)); the per-summoner cap is in the gate | a scoring function in `new:crates/radar-contest/`; a second tab on [`site/src/Leaderboard.tsx`](../../site/src/Leaderboard.tsx) | — | M1. The rule below is accepted by the same delegation as L1–L6, with its change policy |

**M3's rule, proposed.** Per summoned reply, count the refusal signals the
fact sheet carried at the time — the launch block in the 10–13 band or above,
a creator with prior launches and no organic graduation, and whichever others
the sheet already states. Sum per summoner. The gate's `per_summoner_daily`
is the cap, so volume cannot win. **No earliness** (a script wins on speed),
**no engagement** (it can be bought), **no outcomes** (they need a window; a
v2 can add "and it died" once M4 exists). What the rule rewards is finding
launches worth refusing, which is the skill the bot teaches every time it
answers — design 0001's "mastery" item. What it cannot stop is a script that
summons random fresh launches and collects the average signal density; the
cap bounds that, and if the first leaderboard is a script the rule changes
and the change is recorded, which is design 0007 §6.2's own policy.

**Everything in M1–M3 that touches money is unchanged from design 0007:** the
hot-key payout with its three policy lines, the tweet-shaped claim, the
devnet week, the v2 gate of about $1,000 a week for four consecutive weeks
before an on-chain claim program is worth an audit.

---

## 6. What we stop, or do not start

| what | action | why |
|---|---|---|
| holder-weighted nominations (design 0001 §2) | **dropped** | L3. A summon is a nomination; weighting needs a wallet link and gives the token a governance use |
| user burn-for-access | **not built** | L2. Pays nobody, gates nothing scarce, gives the token a use |
| operator buy-and-burn, a token treasury | **stay rejected** | design 0001's header; ADR 0013 constraint 2 |
| a claim dApp, a Merkle distributor, a multisig vault at launch | **not at v1** | design 0007 §6.3 chose a tweet-shaped claim to add no auth surface, and gated v2 on the fee being worth an audit. ADR 0013 says the same. Whether pump.fun's fee recipient can be a multisig is unknown and not needed for one week's fees |
| weighting engagement by reading the engagers' accounts | **no** | it is paid reads of strangers' accounts to score a prize worth tens of dollars, and it is adjacent to the scraping ADR 0013's context refuses |
| a bonus for linking a wallet to an X account | **no** | ADR 0013 constraint 4: entry never requires holding |
| a tip jar in the token | **no** | the operator would hold it |
| vesting, whitelist spots, merch tiers | **no** | there is no allocation to give, by constraint 1 |
| community votes on the bot's voice or targets | **no** | the voice is bounded by two checks and targets come from summons; a vote is a way to move a verdict |
| a second daily format, seasonal modes | **not yet** | one appointment first; add a format when M4's engagement is measured, not before |

---

## 7. The appointments

Three, at fixed times, and the times are printed on the site.

| when | what | surface | cost |
|---|---|---|---|
| daily | "seven days later" (L6, M4) | X + Telegram | $0.015 |
| weekly, Monday 00:00 UTC close | the week's result: the winner, the teardown, the vault, the prize, the transaction (M2) | X + Telegram | $0.015 |
| weekly, same post | the token roasted on the same rule as any other, when somebody asked that week | — | — |

That last row is ADR 0013 constraint 6 and design 0001's self-audit, and it
is worth saying why it stays unexceptional: the day the bot's own coin gets a
special post is the day the whole thing reads as being about the coin.

---

## 8. Legal exposure — flagged, not answered

Neither Josh nor I is a lawyer. Design 0007 J4 says a legal read gates the
first post, and nothing here changes that. What this section adds is the
documents a lawyer should be handed with this design, and the three lines
this design sits near. It offers no opinion on any of them.

**Two regulators' own texts, read 2026-09-05.**

- The SEC's Division of Corporation Finance, *Staff Statement on Meme Coins*,
  2025-02-27. It describes meme coins as having "limited or no use or
  functionality", with value driven by demand and speculation, and promoter
  activity limited to hyping and getting listed. It says it does not extend
  to products inconsistent with that description, and that it is staff views
  with no legal force. A commissioner published a dissenting response the
  same day. **L1(b) and L2 each move this token away from that description**
  — a paid operator running the product that drives volume, and a use for the
  token — which is the main reason both are recommended against.
- The UK FCA's PS23/6, in force 2023-10-08, with FG23/3 as guidance. The
  financial promotions regime covers social media, applies to overseas firms
  promoting to UK consumers, requires risk warnings, and bans incentives to
  invest, including benefits based on cryptoassets held. **ADR 0013
  constraint 4 — free entry, holding never required — is what keeps the
  contest away from "incentive to invest",** and L2 is a benefit for holding.
  This repository writes in British English; if the operator is in the UK,
  this is the document.

**The three lines.**

1. **Touting.** GOAL.md's own paragraph: a bot whose reach moves a price it is
   exposed to. ADR 0013 reduced the exposure to volume through the fee and
   said so. L1(a) keeps it there; L1(b) widens it.
2. **A security.** Whether an operator paid from the fee, running the product
   that drives the volume, is "managerial effort from which purchasers expect
   profit" is exactly the question the staff statement turns on, and exactly
   the one I cannot answer.
3. **A lottery.** A prize contest with free entry and a skill element is
   treated differently from a paid-entry draw in most places, and how
   differently is jurisdictional. Constraint 4 and the published scoring rule
   are the design's answer; whether they are enough is the lawyer's.

---

## 9. The ChatGPT notes, classified

Josh forwarded two sets of suggestions. Short enough to classify here rather
than under `docs/research/vendor/` as research 0010 and 0015 did for the long
ones; same method — what is taken, what is already built, what is refused and
why.

| suggestion | verdict |
|---|---|
| score virality on more than likes; require conversation quality | **already decided** — design 0007 §6.2's weights and exclusions. Reading every engager's account to weight them is refused (section 6) |
| max one win per account per 3–4 weeks | **taken** — one line in the scoring rule, cheap, and it answers the obvious farm |
| reward wallet-to-X linking | **refused** — ADR 0013 constraint 4 |
| downrank serial low-effort taggers; a prompt-quality bar | **already built** — the parser refuses anything without a mint or ticker; the gate caps per summoner |
| a public rubric; cite specific signals, not vibes | **already the design** — the reply is measured facts with dates, and the rule is printed in full on the site |
| calibrated confidence; update a prior take | **taken in spirit** — the bot has no takes, only measurements; "seven days later" (L6) is the update, done in public |
| roast witch-hunters too; balance | **refused** — the bot roasts launches, not people, and ADR 0013's context says nothing may imply an identity it cannot see |
| emissions allocation; tip jar in the token; treasury top-ups by vote | **refused** — constraints 1 and 2, and design 0001's rejected treasury |
| tips in SOL; revenue from API access into the prize | **partly** — SOL can reach a public address without anyone building anything; not wired into the prize (L4). API revenue stays the operator's, in USDC, decoupled (L1) |
| vesting, tiered prizes, whitelist, merch | **refused** — nothing to vest or allocate |
| format refreshes; community votes on style | **not yet / refused** — section 6 |
| public hit-rate tracking | **taken** — it is loop 2, and L6 is how it ships |
| memory of past calls | **already built** — the reply log and the gate's dedupe |
| heavy human review at the start | **already decided** — design 0007 §5's gate: 100 dry-run replies read by Josh, the first 20 live read by hand |
| the claim vault, Merkle proofs, Squads, a "War Room" dApp | **not at v1** — section 6, and ADR 0013's "does not decide" |

---

## 10. Where this is weakest

Said plainly, because a document that only argues for itself is not worth
much.

1. **Status may not carry it.** The whole prize design runs on attention and
   rank because the money is tens of dollars a week at any volume this token
   is likely to see soon. The audience the bot attracts is being told
   "don't buy", and about 96% of this venue's wallets lose money; design 0001
   said this audience converts poorly and nothing here changes that. If
   status does not motivate summons, loop 3 does not turn and the token has
   only loop 1.
2. **The hunter rule is my invention.** It is computable, it cannot be bought,
   and it rewards the right skill — and a script summoning random launches
   collects the average signal density up to the cap. Design 0001 said the
   rule had to be honest before shipping; this one is honest and beatable,
   and the first leaderboard will say which matters more.
3. **The fee is measured on the curve only.** Post-graduation, the venue's own
   page and the chain read on 2026-09-01 do not describe the same schedule,
   and references have been wrong here twice. The prize arithmetic could be
   off by three times in either direction after graduation. M6 is first for
   that reason.
4. **Section 8 is arguments, not clearance.** The SEC text is staff-level and
   was dissented from the day it appeared; the FCA regime bans precisely the
   benefits this design refuses, which is reassuring and is not an opinion.
   J4 stands, and if a lawyer says the contest itself is the problem, the
   token launches without it or not at all.
5. **Telegram produces no record.** It is a second bot, a second token, a
   second thing to keep alive, and every answer it gives is one the public
   cannot check. If it becomes the bigger surface, the compounding half of
   the product is the smaller half.
6. **Loop 1 is untouched, on purpose.** If the token needs more than the bot's
   reach to exist, it will not get it from this document. That is ADR 0013's
   choice, recorded, and not something to route around.
7. **The daily post needs seven days of history.** Day one is an honest empty
   state, again. Everything in this repository ships that way and it still
   feels thin.
8. **Two external studies are cited and neither has been verified against the
   chain by Radar.** They agree with research 0024 and 0011 in direction,
   which is why they are cited; a study whose window is six minutes is
   measuring the coordinated cohort, and it says so only in its caveats.
9. **I could not see production.** The figures come from the brief and the
   committed snapshot, not from a box I read today.

---

## 11. Documents this changes

- **This file**, new.
- **Design 0001's header**: a third bullet, recording that holder-weighted
  nominations are dropped by L3 and the Deep Hunt post survives as the
  winner's prize. Same commit.
- **ADR 0013, design 0007 §6.1, design 0008 §5.3**: the 10× prize figure
  corrected in the same PR as this document, each with a dated one-line note.
- **[`site/src/Pool.tsx`](../../site/src/Pool.tsx)** lines 20 and 52–53: the
  same correction, as a one-string follow-up PR before the domain is live.
  Not in this PR because this document is a document.
- **ADR 0013 otherwise**: unchanged if L1 is (a). Amended in the same PR as
  the site files if not. In either case its constraint 5 describes a
  mechanism that is not built; that is M1's gate and should be fixed by
  building C8, not by editing the ADR.
- **Design 0007**: otherwise unchanged. M2 and M4 extend B6 and C8; M5 is new
  and belongs in a plan file when it starts.
- **GOAL.md**: unchanged.
- **Memory, outside the repo**: `memory:project_radar_token_decision.md`,
  updated 2026-09-05 with the six decisions and where they are recorded.

## External references, read 2026-09-05

Claims, not measurements. Let the reference propose and a capture dispose.

- pump.fun, *Transaction Fees on Pump.fun*, help centre, updated 2025-09-26 —
  <https://intercom.help/pumpfun-web/en/articles/11002413-transaction-fees-on-pump-fun>
- pump.fun creator fee sharing across up to ten wallets, news reports dated
  2026-01-09 — e.g. <https://www.mexc.com/news/453000> (unverified against the program)
- Telegram, *Bots FAQ* — <https://core.telegram.org/bots/faq>
- SEC Division of Corporation Finance, *Staff Statement on Meme Coins*,
  2025-02-27 — <https://www.sec.gov/newsroom/speeches-statements/staff-statement-meme-coins>;
  the same-day response — <https://www.sec.gov/newsroom/speeches-statements/crenshaw-response-staff-statement-meme-coins-022725>
- FCA, *PS23/6: Financial promotion rules for cryptoassets* —
  <https://www.fca.org.uk/publications/policy-statements/ps23-6-financial-promotion-rules-cryptoassets>;
  *FG23/3* guidance — <https://www.fca.org.uk/publications/fg23-3-finalised-non-handbook-guidance-cryptoasset-financial-promotions>
- *Predicting the success of new crypto-tokens: the Pump.fun case*,
  arXiv 2602.14860 — <https://arxiv.org/html/2602.14860v1>
- *Pump.fun Graduation Regime Windows: Survival Analysis of 832,941 Token
  Launches and the Social-Presence Effect*, arXiv 2607.02823 —
  <https://arxiv.org/html/2607.02823>
