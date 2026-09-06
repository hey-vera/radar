<!-- SPDX-License-Identifier: Apache-2.0 -->
# Design 0012 — Four things the owner asked for

**Status:** **a brief, not a decision.** Josh asked for these on 2026-09-06,
hours after the account went live. Recorded here so they are planned from the
repository rather than from a chat log — the failure AGENTS.md §3 names.

**Deliberately not answered here.** Each one is stated as asked, with the
constraint it collides with and the evidence for that constraint. Choosing
between the options is the next planning phase's job, and it is being given to
a different model on purpose: everything merged on 2026-09-06 (#160–#167) was
written by one session, and a plan is worth more from someone who did not write
the code it plans against.

**Written by:** Claude Opus 5, from Josh's words, 2026-09-06

---

## 1. The bio as a noticeboard

> "bot could automatically put the winner into the bot bio with instructions on
> how to claim so the winner sees it's obvious, and maybe a live ranking in bot
> bio so everyone can see and improves the hype factor"

**Nothing writes the bio today.** It would be a new write path on a credential
that already has the scope (OAuth 1.0a, user context).

**What it collides with.**

The bio is already load-bearing and is the *only* place two things can appear:

- **The automation disclosure.** X policy requires an automated account to say
  it is automated and who runs it. Design 0007 B7 gates going live on it. A
  process that rewrites the bio can drop it, and nothing would notice.
- **The site's name and the account's own handle.** `forbidden.rs` refuses
  "cabal", so no *post* can carry `cabalhunter.org` or `@thecabalhunter`
  (plan 0008 Q3 chose this rather than adding an exemption path). The bio is
  the workaround. Overwriting it automatically puts that at risk too.

**The open questions a plan must answer.** Does bio text go through
`fidelity::check` and `forbidden::check`, as every reply does? A bio is a public
statement by the same account and there is no reason it should be held to a
lower standard — but it is not a reply, so nothing applies them today. And a
bio has no version history: a bad write is silent and unrecoverable except by
hand. A "live ranking" implies frequent writes, which is a rate-limit question
and a "how wrong can it be between writes" question.

**Worth noting in favour:** the claim problem this solves is real. #162 built a
claim prompt precisely because the winner was never told they had won. A bio
line is a second, cheaper channel for the same failure.

---

## 2. Following developers it believes are genuine

> "bot follow smart system where it only follows devs it believes are genuine,
> and unfollows if that changes"

**This is the one to think hardest about, and the constraint is not technical.**

`forbidden.rs` refuses to let this account say a coin is "legit", "safe" or
"trustworthy". **A follow is that claim in a stronger and more persistent
form** — it is an endorsement of a *person*, it sits on a public profile
indefinitely, and it is made on statistical evidence.

An unfollow is worse: a public, dated, negative signal about a named developer,
which anybody watching the follow list can diff.

Design [0011](0011-scan-before-declaring-not-before-paying.md) argues the same
line for the leaderboard and settles on **publish the measurement, never the
verdict**. "Devs it believes are genuine" is a verdict about a person, and it is
the exact shape 0011 refuses.

**What a plan must decide before any of this is built:** whether the account is
in the business of vouching for people at all. That is an ADR, not a plan item,
and it is Josh's to make rather than a planner's — it changes what the product
*is*, not how it works.

---

## 3. Posts nobody asked for

> "automated news like posts (like reports on what the bot has been posting
> about, automated reports that don't need a mention)"

**There is precedent, and there is a published promise, and the line between
them is sharper than it first looks.**

The account already posts without being asked: the weekly result and the daily
"seven days later" both do (`weekly.rs`, `daily.rs`). So "never posts unprompted"
is not the rule.

The rule that *is* published, on the home page and in About:

> "It only ever answers when it is asked. It does not post about coins
> unprompted, and it does not decide what you should do."

So the distinction the plan has to hold is **aggregate versus particular**:

| shape | consistent with the promise? |
|---|---|
| "this week Radar answered 40 questions; here is what the launch blocks looked like" | yes — about Radar's own activity |
| "here is a coin nobody asked about, and here is what is wrong with it" | **no** — breaks a promise made in two places |

That promise is a large part of why the account is defensible: it never picks
targets. Changing it is a product decision with legal texture, not a feature.

---

## 4. Replies that argue rather than recite

> "smarter responses and it picks out the details it wants to share (like the
> most damning evidence) and can even build on its post... it shouldn't just
> blurt out random facts that don't matter"

**Step zero, and it is the reason the first live reply looked the way it did:
no model is configured.** Every reply today falls back to the deterministic
template — `fellback: NoProvider` on the first live reply, 2026-09-06. The voice
work in #163 (the sharpened prompt, the `headline`) is **inert in production**
until a provider exists. That is rule 8 working, and it is also the whole of
this complaint's cause.

Selecting the most damning fact is exactly what the model pass is for, and it is
already bounded: `fidelity::check` refuses any number not on the sheet and
`forbidden::check` refuses the verdicts. So the first half of this ask may need
no new mechanism at all — only a provider, and a re-read of whether `LEAD`'s
five-fact cap is doing more harm than good once a model is choosing.

**The second half is different and carries real risk.** "Builds on its post when
someone replies asking for x, y, z" means a stranger's text steers the answer.

Today **mention text never reaches the model.** The sheet goes in; the mention
does not. That is pinned end to end by
`an_adversarial_mention_cannot_change_the_reply.rs`, and it is finding S10 —
verified, not assumed. A conversational follow-up is the first case where user
text would influence what gets said, which is rule 4 territory.

The mechanism exists: `radar_agent::untrusted::fence` and `escape`, already used
for the token's own name and symbol. It has never been used for a conversational
turn, and "the user asked about the launch block, so talk about the launch
block" is a *selection* instruction from an untrusted source — weaker than an
instruction to say something false, but not nothing.

---

## What this brief does not do

It does not rank these, cost them, or recommend one. It does not decide whether
any should be built. Every constraint above is a fact about the current code
with a file named beside it, so a planner can check each one rather than take
it on trust — which is the point of writing it down rather than saying it.
