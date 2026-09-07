<!-- SPDX-License-Identifier: Apache-2.0 -->
# Design 0014 — The four asks answered

**Status:** **decided.** Design
[0012](0012-four-things-the-owner-asked-for.md) stated four things Josh asked
for on 2026-09-06 and deliberately answered none of them. This answers all
four.

**Numbered 0014 and not 0013.** Plan
[0009](../plans/0009-the-evidence-the-rule-and-the-voice.md) item 11 says
"design 0013"; 0013 was taken the same day by
[what Telegram is for](0013-what-telegram-is-for-now-that-nothing-is-gated.md).
Recorded here rather than by editing the plan, so the plan is not quietly
rewritten into having been right.

**Written by:** Claude Opus 5, 2026-09-06, executing plan 0009.

**Whose decision each one is.** Two of these are Josh's and are recorded as
his: he reaffirmed the bio after the concern was put to him, and he chose the
model path. Two are settled by facts about the platform and the existing
documents rather than by anyone's preference, and this document shows the work
so the reasoning can be disagreed with rather than taken on trust.

---

## 1. The bio as a noticeboard — **yes, guarded**

> "bot could automatically put the winner into the bot bio with instructions on
> how to claim so the winner sees it's obvious, and maybe a live ranking in bot
> bio so everyone can see and improves the hype factor"

**Josh's decision, made after the concern was raised.** Design 0012 put the
objection to him plainly: the bio is the only place the automation disclosure
lives, a bio write has no version history, and a bad write is silent and
unrecoverable except by hand. He asked for it anyway. That is his call, it is
recorded as his, and the consequence is stated once here and not argued again.

**What follows from it is a build constraint, not a caveat.** Because dropping
the disclosure is the failure that matters and nothing would notice it, the
writer is built so that dropping it is *impossible* rather than *unlikely*: the
disclosure is the first line of every template, the templates are the only way
to produce bio text, and a test asserts the disclosure survives every branch —
AGENTS.md §5, enforce it at the cheapest level that can hold it.

Every render goes through `forbidden::check` and `fidelity::check` against the
week record's authorised numbers. A bio is a public statement by the same
account, and there is no reason it should be held to a lower standard than a
reply.

**Blocked, and on a fact rather than on effort.** Plan 0009 phase 0.6(b) has
to establish that `POST /1.1/account/update_profile` works on this account's
plan before any of it is worth writing. That check is in the Developer Console
and it is Josh's. Until it comes back, plan 0009 item 7 is not started, and
that is recorded in the plan's handback rather than left looking like an
oversight.

**One thing this genuinely solves.** #162 built a claim prompt precisely
because the winner was never told they had won. A bio line is a second, cheaper
channel for the same failure, and the failure is real.

---

## 2. Following developers it believes are genuine — **cannot be built**

> "bot follow smart system where it only follows devs it believes are genuine,
> and unfollows if that changes"

**Not a judgement about whether it should be built. It cannot be.**

Follow, like and quote-post write endpoints were removed from every self-serve
tier on 2026-04-16 and are Enterprise-only. X's Automation Rules prohibit
automated following at every tier, including Enterprise. Both halves of that
have to be false before this is a design question at all, and neither is.

**The principled objection stands underneath the platform one, and it is worth
recording because the platform could change.** `forbidden.rs` refuses to let
this account say a coin is "legit", "safe" or "trustworthy". A follow is that
claim in a stronger and more persistent form: it is an endorsement of a
*person*, it sits on a public profile indefinitely, and it is made on
statistical evidence. An unfollow is worse — a public, dated, negative signal
about a named developer that anybody can diff out of the follow list.

Design [0011](0011-scan-before-declaring-not-before-paying.md) settles the same
line for the leaderboard: **publish the measurement, never the verdict.** "Devs
it believes are genuine" is a verdict about a person.

**So if X ever opens follows on self-serve, this is an ADR and not a plan
item** — it changes what the product *is*. It does not become buildable by
becoming possible.

**What Josh actually wanted from it is being built.** Good accounts for people
to look at, and interaction that spreads. That is the **hunter board**: the
tally has been written to `hunter-<week>.json` at every close since #164 and
served nowhere. Plan 0009 item 8 serves it at `/v1/public/hunters` and names
the week's top three by handle in the weekly thread, which reaches their
notifications. Design 0009 L4 called that badge the real prize, and unlike a
follow it is a measurement: *this account found this many launches worth
refusing.*

---

## 3. Posts nobody asked for — **already built, and the promise was wrong**

> "automated news like posts (like reports on what the bot has been posting
> about, automated reports that don't need a mention)"

**The account already does this and has since #158.** The weekly result and the
daily "seven days later" are both automated reports that need no mention. There
is no third format to design; there are two appointments that already exist,
and one of them has never run.

- The **weekly** result posts on the first tick after Monday 00:00 UTC. It has
  run. Since plan 0009 item 8 it is a three-post thread: the summary, the
  winner's coin torn down, and the week's best hunters.
- The **daily** "seven days later" starts 2026-09-13 — but only once
  `radar-seven-days.timer` is installed. It is not, and without it the first
  one finds no file and posts nothing. That is plan 0009 phase 0.4, and it is a
  root command that is Josh's.

**The site's promise was false, and that is the actual finding here.** The home
page said *"It does not post about coins unprompted"* and About said *"It
replies only when it is mentioned"* — both untrue since the weekly teardown
shipped. The teardown posts about a coin nobody asked about in that post.

Both now read: **"It never picks a coin to post about. Every coin it names,
somebody asked it about."** That is true of the teardown (the winner asked), of
the daily post (somebody asked seven days earlier), and of every reply. It is
also the promise actually worth keeping — the account never selects targets,
which is a large part of why it is defensible.

**No fourth format until the daily post has four weeks of measured
engagement.** Design 0009 §6 stands: the way to find out whether people want
more posts is to measure the ones that exist, and there is not yet one week of
that.

---

## 4. Replies that argue rather than recite — **the first half is done; the second is gated**

> "smarter responses and it picks out the details it wants to share (like the
> most damning evidence) and can even build on its post... it shouldn't just
> blurt out random facts that don't matter"

**Step zero was the whole of the cause, and it is fixed.** No model was
configured, so every reply fell back to the deterministic template —
`fellback: NoProvider` on the very first live reply. The voice work in #163 was
inert in production. Josh chose the OpenAI path on 2026-09-06 (he had credits
there), an OpenAI-shaped provider shipped in plan 0009 item 1, and the account
now has one.

Selecting the most damning fact is exactly what the model pass is for, and it
was already bounded before the provider existed: `fidelity::check` refuses any
number not on the fact sheet and `forbidden::check` refuses the verdicts. So
the first half of this ask needed **no new mechanism** — only a provider.

One thing it did need, and it was found by running it: the model fabricated a
figure, four times out of four, and every time it was the identical literal
`97.1` — a share it had computed by subtracting a stated share from a hundred.
The system prompt now forbids arithmetic on the sheet's numbers in those words.
Zero fabrications in the thirteen calls since. That is the shape this file's
claims should be checked in: a number, from a run, not an argument.

**The second half carries real risk and is gated on measurement.** "Builds on
its post when someone replies asking for x, y, z" means a stranger's text
steers the answer, and today **mention text never reaches the model** — pinned
end to end by `an_adversarial_mention_cannot_change_the_reply.rs`, which is
finding S10 and is verified rather than assumed.

Plan 0009 item 12 is the safe form of it: `Focus`, a **closed vocabulary**
parsed from the mention by keyword — creator, launch block, cost, graduation —
passed to the model as one of Radar's own fixed phrases. A stranger chooses
among Radar's labels and supplies no text, so the sheet is identical and only
the order changes. The adversarial test gains that case.

**It is gated on evidence, not on appetite.** Item 12 is built only if
`journalctl -u radar-analyst | grep -c -- '-> Nothing'` over the first week
shows more than a handful of mentions that named no coin. If people are not
asking that way, the parser is a feature nobody needed with a rule 4 surface
attached, and the right number of those is zero.

---

## The approval clause, which is Josh's to read

Plan 0009 fact 3: X's Automation Rules, updated April 2026, are **reported** to
require prior written approval for AI-powered reply bots. The rules page
returned 403 to the session that found this, so it is a report and not a
verified fact, and it is recorded that way.

It stopped being hypothetical the moment the provider was configured. Until
then the account posted a deterministic template; it is now unambiguously an AI
reply bot.

<https://help.x.com/en/rules-and-policies/x-automation>

**This is for Josh to read and settle**, and the outcome — applied for, or
recorded as not applicable with the sentence that makes it so — belongs in this
repository either way. It is the one item on this page that nobody else can
close.

---

## What this document decides, in one table

| ask | answer | where it lives |
|---|---|---|
| 1. bio noticeboard | **yes, guarded** — Josh's, reaffirmed | plan 0009 item 7, blocked on phase 0.6(b) |
| 2. follow / unfollow | **cannot be built** — Enterprise-only, and prohibited at every tier | substitute shipped: the hunter board, item 8 |
| 3. unprompted posts | **already built** — two appointments; the promise was wrong and is reworded | item 5; the daily one needs phase 0.4 |
| 4. smarter replies | **first half shipped** (a provider); second half gated on a measurement | items 1 and 12 |
