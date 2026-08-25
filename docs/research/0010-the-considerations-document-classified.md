<!-- SPDX-License-Identifier: Apache-2.0 -->
# 0010 — The considerations document, classified

**Date:** 2026-08-25
**Source:** [`vendor/chatgpt-radar-considerations.md`](vendor/chatgpt-radar-considerations.md),
3,430 lines, 124 numbered sections, authored by ChatGPT and opening with a
disclaimer of its own accuracy.

The document sat untracked in `docs/research/` for days and had never been read
by anyone. `git add -A` swept it into two pull requests, and it was removed from
both. This note is Radar's view of it.

## Why it is tracked, and why the verdict is a separate file

Tracking it kills the `git add -A` hazard permanently, which `.gitignore` would
also do. Ignoring it would throw away the record of *why* thirty plausible
features were rejected, and the next agent would re-derive them from scratch.

The source is kept unedited under `vendor/`, because a directory makes "somebody
else's document" structural rather than a sentence in a preamble a reader skims
past. It is a **source document, not a plan**, in the way a vendor's API
reference is: cite it, disagree with it, do not maintain it.

The verdict lives here so it can be cited and superseded on its own — the same
separation the ADRs use.

## What it gets right, sometimes better than Radar does

- **§9 "NO TRADE" as a first-class outcome.** Already `Decision::Pass` with a
  reason list. The document is right that this is load-bearing and right that
  optimising for trade count is the failure mode.
- **§94 autonomy levels.** [`Autonomy`](../../crates/radar-risk/src/policy.rs) is
  already the 0–5 ladder it proposes, almost item for item.
- **§40 execution policy engine, §42 transaction intent validation.** Already
  `Policy` plus the pure kernel, and the signer already re-decodes rather than
  trusting a description. **§42 is weaker than what exists** — it asks the signer
  to verify an intent; Radar's signer refuses address lookup tables outright so
  that every account it authorises is one it read
  ([ADR 0003](../adr/0003-legacy-transactions-because-the-signer-must-be-able-to-read-them.md)).
- **§38 / §102 untrusted content.** `Trust::Untrusted` predates the document.
- **§13 / §36 sellability as the first question.** This is Radar's entire thesis,
  and the document lists it as one item among 124.

Worth stating plainly: on the things Radar has already built, the document is a
reasonable description of them. That is mild evidence the foundations are
conventional-good rather than eccentric, and no evidence at all about the parts
neither has measured.

## What is worth doing, and is in the plan

- **§23 "probable execution cluster" rather than `is_jito_bundle`.** Correct, and
  the direction `radar-graph` should grow: the ledger exposes no reliable "this
  was a bundle" field, so the target is an inference with a confidence.
- **§24 / §27 cross-launch cohort recurrence.** The strongest unbuilt
  coordination feature, and computable from data already recorded.
- **§61 / §89 concept drift.** [0008](0008-the-launch-block-gives-the-bundle-away.md)
  concedes `BUNDLE_CENTRE = 6` is a tool's default setting. The histogram in
  `radar consider` is the first step; a monitor is the second.
- **§109 / §110 property and fuzz tests.** Radar has none of either, against nine
  invariants, several universally quantified.
- **§50 / §51 execution attribution and post-trade records.** The precondition
  for answering whether selection beats the base rate.
- **§97 AI cost metering.** `radar-provider` implements it and nothing depends on
  it.
- **§100 explain why it did *not* trade.** This is the frontend's entire job.

## What is wrong for this system

**§39 lists `prepare_trade`, `request_approval` and `execute_trade` as AI
tools.** Radar gives the model **no action tools at all**, and its output is
never parsed into a structured action. The document's own §14 says never let
AI-generated instructions bypass execution policy; handing the model an action
tool and then policing it is one lock where two are free. If there is nothing to
inject *into*, prompt injection stops being a threat that needs defending and
becomes uninteresting.

**§115 is worth quoting back at whoever commissioned the document:**

> Do not make the product economically dependent on customers sharing or reusing
> personal ChatGPT subscriptions.

That is correct, and it directly constrains the AI lane: a subscription
credential is a private-use path, and the API-key path is the one that survives
commercialisation.

## What would be cargo-culting, and this is the document's largest blind spot

§4 shred and Geyser ingress, §46 leader intelligence, §47 landing-probability
models, §19–§22 Jito bundle economics, §20 multi-path racing, §78 geographic
placement, §75 cross-venue arbitrage, §76 order-book and RFQ support, §80 the
four-store data architecture.

Every one is real engineering, and every one buys a **latency** edge.

Radar's measured edge is not latency. `creator_edge` acts around forty minutes
after launch because its token-age budget says so, and
[0008](0008-the-launch-block-gives-the-bundle-away.md) found that the fast money
was already in the launch block before the token existed — which is why a
launch-block signal is used to **refuse** rather than to buy. Building leader
intelligence to arrive forty minutes late is buying a stopwatch for a marathon
you are walking.

The document is written for a sniper. Radar is deliberately not one, and it
never asks whether the system it is advising competes on speed. That is the one
question that would have reorganised its whole middle section.

**§5 source disagreement and §107 provider redundancy** are wrong-shaped for a
different reason: both assume many feeds. Radar has one historical source, chosen
deliberately in [ADR 0002](../adr/0002-historical-data-comes-from-cryptohouse-not-a-vendor-archive.md).
Redundancy is a cost with no benefit until there is a second source worth having,
and "add a provider" is not a cheap step.

## The part that would have caught this session's bugs

§123 asks five questions of every feature, and the fourth is *"How will we know
whether it actually works?"*

Applied to the price path, that question finds both bugs this session fixed. The
document is better at asking it than at answering the sections it asks it about
— and on the evidence of §4 through §80, it does not apply it to itself.

## What this note does not do

It does not rank the items, and it does not become a backlog. The sections judged
worth doing are worth doing for reasons that were already in the plan before the
document was read, which is the honest way to report a source that arrived after
the decisions it agrees with.
