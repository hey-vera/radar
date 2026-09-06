<!-- SPDX-License-Identifier: Apache-2.0 -->
# Design 0013 — What Telegram is for, now that nothing is gated

**Status:** **decided, 2026-09-06: the lane stays off.** Josh asked the question
in section 1 while being walked through wiring the lane up, read section 5, and
answered "lets turn telegram off". `RADAR_TELEGRAM_BOT_TOKEN` was removed from
`/etc/radar/analyst.env` the same day, before any restart could switch it on.

Section 6 is **also decided, and by X rather than by us**: Communities were shut
down on 2026-05-06. There is no alternative surface. See the note at the head of
that section.

**Supersedes nothing.** [Design 0009](0009-three-loops-and-no-formula.md) L5
decided *what* Telegram is; this asks whether that still holds after L2, L3 and
ADR 0013 removed every holder-side mechanic around it.

## 1. The question, in Josh's words

> "Unless I'm making something up — what is the point of this Telegram group?
> If there's no gated token access, the original idea was people holding for
> faster alerts = tokenomics flywheel. But now it feels like Telegram is just
> offloading X users to a Telegram channel, for what? All the action is on the
> X account, so the flywheel should be there."

He is not making it up. This document exists because the answer is largely
"you are right", and that is worth writing down before a second surface is
switched on.

## 2. What was removed, and when

Three holder-side mechanics were designed and then rejected, each for its own
good reason, and **no document connects them**. Read one at a time each looks
like a narrow call. Read together they are the whole holder side of the
product.

| mechanic | where it was designed | killed by | why |
|---|---|---|---|
| usage-linked burn | [0001](0001-the-flywheel.md) §3 | [ADR 0013](../adr/0013-a-community-token-exists-and-radar-holds-none-of-it.md) constraint 2 | burning means holding, and the operator holds zero |
| holder-weighted nominations | [0001](0001-the-flywheel.md) §2 | [0009](0009-three-loops-and-no-formula.md) L3 | weighting by holding gives the token a governance-shaped use |
| burn-for-access to the free lane | [0009](0009-three-loops-and-no-formula.md) L2 | 0009 L2 | a benefit for having held; SEC staff statement 2025-02-27, UK PS23/6 |

**So there is no holder benefit left, by design.** That is not an oversight —
it is ADR 0013's entire point, and the reasoning is regulatory rather than
economic. Nothing here argues for putting any of it back.

## 3. So what is the flywheel now

Precisely, with no holder in it:

> attention on X → people summon the bot → the replies get engagement → the
> best-engaged summoner wins the week → the prize is the creator fee from the
> token's **trading volume** → the payout is a post → attention.

Holding does nothing. **Trading** funds the prize. That loop is coherent and it
is entirely on X, which is Josh's second point and it is correct: X is the only
surface that is public, recorded, scoreable and shareable.

## 4. What Telegram actually does to that loop

[0009](0009-three-loops-and-no-formula.md) L5 justifies the lane as *"volume
without a bill — somebody who wants to check twenty coins a day does it there
for free instead of hitting the X gate. That is where the 'higher volume'
demand from L2 actually goes."*

Two problems with that sentence today.

**The demand it catches is not the demand L2 described.** L2's higher-volume
user was going to *pay* for the privilege, by burning. With gating rejected,
what L5 catches is simply the user the X gate refuses — three replies per
account per day, fifty a day globally.

**Moving that user to Telegram removes them from the loop.** A Telegram answer
produces no public reply, no engagement to score, no contest entry, and nothing
anybody can screenshot. L5 concedes half of this itself:

> "**What Telegram cannot add:** the record. A private answer is not a public
> call. If Telegram grows faster than X, the product's public half is the
> smaller half, and that is section 10's fifth weakness."

The design already recorded this as a known weakness. Josh's version is
sharper: it is not only that Telegram fails to add to the loop, it is that the
loop's fuel is exactly what it consumes.

**And there is now a measured cost that did not exist on 2026-09-05.** Every
Telegram summon is a chain read. On 2026-09-06 a ten-mention burst against the
free public RPC returned **seven HTTP 429s** — the analyst could not serve four
requests in a row. A free, uncapped second lane feeding the same endpoint makes
that worse, and the load it adds is the load that does not count. Turning
Telegram on before `RADAR_RPC` is set would degrade the surface that *is* the
flywheel in order to serve the one that is not.

## 5. Recommendation

**Do not turn the Telegram lane on now.** Specifically:

1. **Leave `RADAR_TELEGRAM_BOT_TOKEN` unset.** The lane is built (0009 M5,
   `telegram.rs`) and costs nothing to leave off. Rule 8 means an unset token
   is a lane that reads nothing and says nothing — the resting state, not a
   broken one.
2. **Keep the two bots and the two chats anyway.** They cost nothing, the alert
   channel is genuinely needed, and the group is where a community forms
   whether or not the bot answers in it.
3. **Revisit on a number, not a feeling.** The trigger: the X gate refusing
   real people. `grep -c SummonerDaily refusals.jsonl` over a week. Below a
   handful there is no demand to catch and the lane is a second surface to
   watch for no reason. Above it, there is, and this recommendation is wrong.
4. **Before any of that, `RADAR_RPC`.** Seven in ten is not a lane problem; it
   is the whole account's problem, and it is the only item here that is
   currently costing real replies to real people.

The weekly teardown and the daily "seven days later" can be cross-posted to a
Telegram channel later at zero marginal cost — that is broadcast, not a lane,
and it does not consume a chain read or divert a summon.

## 6. The better alternative Josh raised, and the reason it is gone

**Settled 2026-09-06, and not by the API question below: X shut Communities
down on 2026-05-06**, extending the migration deadline to 05-30. Josh found this
by trying to create one. X cited under 0.4% of users against 80% of the
platform's spam reports, financial scams and malware
([TechCrunch, 2026-04-23](https://techcrunch.com/2026/04/23/x-is-shutting-down-communities-because-of-low-usage-and-lots-of-spam/),
[Engadget](https://www.engadget.com/social-media/x-is-shutting-down-its-communities-feature-182843958.html)).
The replacement X offers is **XChat group chats** -- private, so no public
metrics, no contest entry, nothing shareable. That is the same flaw as a group
DM and as Telegram, which section 4 already rejects.

So the free-lane question is Telegram-or-nothing after all, and section 5's
answer stands unchanged. **The rest of this section is kept as the reasoning
that would have applied**, because if X ever ships a public group surface again
this is the argument for using it and the test to run first.

---

Asked immediately after the above: **why Telegram at all, rather than an X
Community plus an X group DM with the bot in it?**

It is the better instinct, and if it works it makes this whole document moot.
A community post is still a post: public, on the record, carrying
`public_metrics`, screenshotable, and **scoreable for the contest**. It keeps
every summon on the surface that feeds the loop instead of draining it. Section
4's objection to Telegram does not apply to it at all.

**The group DM half is not the answer.** A DM is private, so it carries no
public metrics, is not a contest entry, and cannot be shared -- the same flaw as
Telegram with a more restricted API on top.

**The community half may not be reachable by the API**, and the evidence points
that way. Two reports on X's own developer forum, both from 2026:

- `POST /2/tweets` returns **403 Forbidden** when the payload carries
  `community_id`, on the pay-per-use tier, while timeline-only posts succeed
  ([devcommunity, 2026-02-17](https://devcommunity.x.com/t/api-v2-403-forbidden-on-community-posting-community-id-payload-despite-pay-per-use-tier-and-prior-functionality-feb-17-2026/257434)).
- Community posts are reported as **filtered out of API endpoints**, with
  developers asking whether it is "no longer possible to retrieve community
  posts (and replies to community posts) via the api"
  ([devcommunity](https://devcommunity.x.com/t/community-posts-from-broken-in-api/238960)).

The second is the fatal one: if a mention inside a community never reaches
`GET /2/users/:id/mentions`, the bot cannot see it, and people summoning it
there would watch it sit silent. That is worse than having no community.

Both are forum reports rather than documentation, and both may be stale.
**The test is five minutes and definitive:** create the community, post in it
mentioning the bot with a mint, and read whether it reached the mentions poll.
Plan 0009 Phase 0.6 already reserves (e) for this.

If it reaches: build the community, and **delete the Telegram lane** rather
than leave dead code -- the free-lane question is answered better on X. If it
does not: the community is still worth having for people, the bot cannot serve
it, and the free-lane question returns to Telegram-or-nothing, where section 5's
answer is nothing.

## 7. Where this is weakest

**The retention argument is not answered.** A person the X gate refuses gets
nothing today. With Telegram they get an answer, and might come back to X
tomorrow. This document assumes they would have come back anyway. Nobody has
measured that, and it cannot be measured until the gate refuses somebody real —
which, per `refusals.jsonl`, has not yet happened once.

**"A community forms in a room, not in a reply thread" is probably true**, and
this recommendation trades it away. X replies are broadcast; a group is a
place. If the token's social layer matters more than the contest's throughput,
turning the lane on is the right call and section 4's argument is an
optimisation nobody asked for.

**It argues from four hours of data.** The account went live 2026-09-06 at
16:16 UTC with one reply in the log. Any claim here about where volume will go
is a guess with a confident sentence around it.

**It does not answer the real question underneath.** Josh asked what makes
holding worth anything. The honest answer is *nothing, deliberately*, and that
is ADR 0013 working as designed — but it means the token's only demand driver
is speculation on the account's attention, and no document in this repository
says that in one sentence. That is a bigger question than Telegram and it
deserves its own document.

## 8. What this changes if accepted

Nothing is built or deleted. Two lines are not added to
`/etc/radar/analyst.env`, and [plan 0009](../plans/0009-the-evidence-the-rule-and-the-voice.md)'s
"not in scope" line about the Telegram lane stops being provisional.
