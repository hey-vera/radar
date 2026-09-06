<!-- SPDX-License-Identifier: Apache-2.0 -->
# Design 0011 — Scan before declaring, not before paying

**Status:** **proposed, and deliberately not accepted yet.** Josh asked the
question and could have accepted this on the spot; he declined to, and the
reason is worth recording: **the model that wrote this design also wrote the
code it governs** (#161-#165, in one session). A recommendation reviewed against
the assumptions that produced it is the weak form of review. So this goes to a
fresh reviewer *undecided*, and that reviewer is free to reject it outright.
Supersedes plan [0008](../plans/0008-the-face-the-claim-the-scan-and-the-voice.md)
item 3 and its Q1 **if** accepted.
**Asked by:** Josh, 2026-09-06
**Written by:** Claude Opus 5
**Awaiting:** a fresh review, then Josh's decision

## The question

Plan 0008 item 3 scans **the winner**, after they claim, before they are paid.
The owner asked whether it should instead scan **the entrants**, before a winner
is declared — so that an account whose engagement was bought never reaches the
leaderboard at all, and the leaderboard says why, publicly.

## The answer, in one line

**Yes, it belongs at ranking time.** Two changes to the proposal, and one thing
it cannot replace.

## Why ranking time is right

Plan 0008 put the scan at payout because that is where the money moves. That
reasoning is incomplete: **the leaderboard is the product, not the payment.**

The account's whole claim is that it measures things and publishes what it
found. A bought winner sitting at the top of a public page for a week is the
damage. Refusing to pay them afterwards is a private correction to a public
error — and worse, it is invisible: an unpaid week and an unclaimed week look
identical from outside.

The existing machinery already agrees. [`rank`](../../crates/radar-contest/src/score.rs)
excludes entries with a stated reason today — the operator, accounts under 30
days, anyone the gate refused, anyone inside the cooldown — and design 0007
§6.2's principle is that **an entry that does not count is returned beside the
reason, never dropped**. A scan-derived exclusion is that same shape. Putting it
at payout instead invents a second, hidden mechanism for the same job.

## Change 1: scan down the ranking, not across the leaderboard

Scanning every entrant is the version that does not survive contact with the
X API quota.

Engager reads are per-post and paginated. The week close today costs **two**
calls (`metrics`, `accounts`). Scanning every entrant costs `2 + 3·N·P` — three
engagement kinds, `P` pages each, `N` entrants. At 200 entrants and 3 pages that
is roughly **1,800 metered reads a week**, against a quota that is bought.

It is also unnecessary. Nobody ranked 47th can win. The cheap form:

> Walk the ranking from the top. Scan each candidate. The first one that passes
> is the winner. Stop.

Cost is bounded by how many excluded entries sit above the first honest one,
which is normally zero or one — so **3–9 reads in the ordinary week**, not 1,800,
for exactly the outcome asked for. A bought account never gets crowned.

The entries below the winner go unscanned, and that is correct: their score is
published, they were not chosen, and nothing about them is being asserted.

## Change 2: publish the measurement, never the verdict

This is the change I feel strongest about, and it is a genuine disagreement with
"it would say why, live for everyone to see" — not with the intent, with the
wording.

`forbidden.rs` refuses to let this account call a **coin** a scam, a rug or a
fraud. Publishing *"excluded: bought engagement"* beside a named person's handle
is a materially stronger claim than any of those: it is an accusation of conduct,
about a person, on evidence that is statistical.

So publish the fact and let the reader conclude:

| never | instead |
|---|---|
| `excluded: botted` | `excluded: 38 of 40 engagers were created in the same week` |
| `excluded: sybil` | `excluded: 31 engagers also engaged with last week's winner` |

The second column is checkable, survives being wrong, and is the same voice the
replies already use. The first is a verdict this product does not issue about
anything else, and it would be the one place the site says something its own bot
is forbidden from saying.

**This also decides the naming question left open in #162.** The leaderboard
publishes exclusions as counts by reason today, deliberately not naming refused
accounts. A scan exclusion is different — the excluded entry would otherwise be
*visible at the top of the ranking*, so its absence needs explaining in a way an
"account too new" exclusion does not. Name the entry, publish the measurement,
never the verdict.

## Change 3, and it is the one that should gate shipping: there is no baseline

**Nothing has measured what normal engagement looks like on this account's
replies, because the account has never posted.** `posts.jsonl` holds one dry-run
week and `publisher=dry-run`.

A threshold set without that baseline is the failure research
[`0024`](../research/0024-the-spike-became-a-hump-and-the-signal-moved.md)
records in capitals: design 0008 said *"six is a tool's default, not a law"*,
nothing was built to notice, and the headline was wrong by 2.7× nine days later.
Here the same mistake costs a named stranger a public exclusion and a prize.

So this ships in two phases:

**Phase 1 — measure, exclude nobody.** Scan down the ranking, record the facts
on the week record, publish them beside the winner. Change nothing about who
wins. This is plan 0008's Q1 answer ("no count refuses payment") kept, but moved
to the right place and made visible.

**Phase 2 — exclude, once there is something to calibrate against.** After
enough closed weeks to know what an ordinary reply's engagement looks like, set
a threshold, and the exclusion carries the measurement as its reason.

Phase 2 needs its own ADR, because by then it is a rule about who gets money and
that is the owner's to set, not a tuning exercise.

## What this does not replace

The **claim address check** stays at payout, unchanged from plan 0008 item 3:
`Refusal::NotAWallet` when the claimed address is program-owned.

That is a different check on a different object. The address does not exist
until the winner replies with it, which is necessarily after they have won. No
amount of ranking-time scanning can check a string that has not been sent yet.

So the split is:

| when | checks | why then |
|---|---|---|
| week close, down the ranking | the entrant's account and engagement | before anything is published |
| payout, on the winner | the claimed address is a wallet | it does not exist before the claim |

## Where this is weakest

- **Phase 1 publishes facts about a named person's engagers.** Counts only, no
  ids — but "31 of this account's engagers were created in the same week" is
  still a statement about somebody, derived from data they did not volunteer. It
  is public data and the framing is a measurement, which is why I think it
  passes; a reasonable person could disagree.
- **The scan is evadable by anyone who reads this document.** Buy older
  accounts, spread the engagement, stay under whatever Phase 2 sets. The honest
  claim is that it raises the cost, not that it works.
- **"Scan down the ranking" leaks information.** A candidate scanned and skipped
  learns they were considered. With Phase 1 excluding nobody this is moot; in
  Phase 2 it is a real edge for someone probing the threshold.
- **A transient API failure must not exclude anyone.** Rule 9 says unknown is
  not eligible, and that precedent (`AccountAgeUnknown`) is right for a cheap
  weekly re-run and wrong for money. Phase 2 must decide explicitly: retry
  before close, or roll the pool. Not silently exclude.

## Recommendation

Accept Phase 1 as the replacement for plan 0008 item 3. Keep the claim-address
check where it is. Do not set an exclusion threshold until a measured baseline
exists, and record that threshold as an ADR when it does.

**This is a recommendation, not a decision, and it is the author's own** — see
the status above. The reviewer should treat every number in it as unverified:
the `2 + 3·N·P` read estimate, the claim that 3-9 reads covers the ordinary
week, and the assertion that no baseline exists were all established by the
same session that proposes the design. The last of those is the one to check
first, because everything in Phase 1 rests on it.
