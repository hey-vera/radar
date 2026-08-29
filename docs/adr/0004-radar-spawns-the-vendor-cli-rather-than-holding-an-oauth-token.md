# 0004 — Radar spawns the vendor CLI rather than holding an OAuth token

**Status:** accepted
**Date:** 2026-08-27

## Context

Radar's reading assistant needs to reach a language model. Two ways to pay for
that were on the table: a personal subscription reached over OAuth, and a
metered API key.

The subscription is the cheaper one to run and the one Josh already has. It is
also the one that requires holding a credential Radar did not issue, cannot
re-issue, and must refresh on a schedule set by somebody else.

Four facts about that credential, verified against the vendor's current
documentation rather than recalled:

- `codex login --device-auth` is the documented headless flow. It prints a
  verification URL and a short code; a human completes the flow in a browser.
- Credentials live in `auth.json` under `CODEX_HOME`.
- The CLI refreshes on its own when the stored token is about eight days old.
  The documented pattern for unattended use is *run the CLI and persist the
  updated file* — explicitly **not** calling refresh endpoints yourself.
- Refresh breaks when another machine rotated the token first. The credential is
  **single-writer**.
- Refresh tokens expire after roughly 14–30 days of inactivity.

The process that would hold this credential is also the process that renders
token names chosen by strangers into a language-model prompt. That is not an
incidental detail; it is the whole reason this decision is written down.

## Options

**1. An OAuth client inside Radar.** Radar stores the refresh token, calls the
token endpoint on a schedule, and handles rotation.

This is a second writer against a single-writer contract, implementing a refresh
protocol owned by a vendor who will change it, inside the process that handles
untrusted content. The failure mode is a rotation nobody scheduled, at three in
the morning, with a token that two processes each believe they own.

**2. Spawn the vendor CLI.** Radar runs `codex` as a subprocess, writes the
prompt to its stdin and reads its stdout. The CLI owns `auth.json`, owns the
refresh, and owns the device-authorisation flow.

Costs a process per call — tens of milliseconds against a model call measured in
seconds — and makes the vendor's CLI a dependency of the deployment.

**3. Metered API key only.** No subscription path at all.

Correct for a sold product and wrong for today: it spends money per call for a
feature that is being evaluated, and Josh already pays for the subscription.

## Decision

**Both 2 and 3, behind one `Provider` trait, chosen by configuration.** Not
option 1, in any form.

`radar-model::Codex` spawns the CLI. **Radar contains no code on that path that
reads, writes, parses or stores a credential.** `radar-model::ApiKey` holds a key
in memory, read once from the environment, and computes cost from the token
counts the provider reports.

An environment naming both is refused rather than resolved. An operator who set
a key in order to move off the subscription, and forgot to unset the old
variable, is otherwise still on the subscription — and nothing would say so,
because it works.

## What this does and does not guarantee

Worth stating precisely, because the tempting version of this section claims the
stronger thing.

**It guarantees** that Radar's source contains no credential handling on the
subscription path. There is no code to review for a token leak, because there is
no code that touches a token.

**It does not guarantee** that the operating system prevents Radar from reading
the file. A CLI running as Radar's own user has an `auth.json` that Radar's user
can read, whether or not any code does.

That is why the command is *configuration* rather than a hardcoded `codex`. Point
`RADAR_MODEL_CODEX` at a wrapper that drops to a separate user with its own
`CODEX_HOME`, and the boundary becomes one the kernel enforces rather than one
this document asserts. `deploy/README.md` carries the unit that does it.

## What it costs

- **The vendor CLI must be installed on the box**, and its upgrades are now part
  of the deployment. A CLI whose output format changes breaks the reply parsing
  — which is why `interpret` refuses an empty answer rather than returning one.
- **A human step, once.** Somebody must complete the device-authorisation flow,
  and again if the credential lapses after a fortnight of inactivity. There is no
  way to automate this that does not amount to option 1.

  It is a *button* rather than an SSH session, and that is not a compromise of
  the decision above. Device authorisation prints a verification URL and a short
  code, both designed to be shown to a person and neither of them a credential;
  the code is useless without somebody able to sign in to the account, and it
  expires in minutes. Radar renders those two strings and nothing else. The CLI
  still owns `auth.json`, still owns refresh, and Radar still has no code that
  reads a token — what the button removes is the need to remember a procedure at
  the moment the credential has already lapsed.

  Only one flow at a time, because the credential is single-writer and two
  concurrent logins would race to be the writer.
- **A process per call.** Negligible against the call, and it buys the property
  that a hung CLI is killed on a deadline rather than holding a request forever.
- **The subscription path is private-use-only.** The vendor's terms and §115 of
  the considerations document agree: a personal subscription credential is not a
  foundation for a sold product. The API-key path is the one that survives
  commercialisation, and the one CI exercises against a stub.

## The environment the child does not inherit

`env_clear`, then a fixed allowlist: `PATH`, `HOME`, `CODEX_HOME`, `LANG`,
`LC_ALL`, `TMPDIR`.

An allowlist rather than a denylist, because a denylist is wrong the moment
somebody adds a variable and does not think of this file. `radar-serve`'s
environment holds an x402 payout address, a facilitator URL and — on the other
path — a model API key. None of that is any business of a subprocess whose input
is partly written by whoever named a token.

The prompt goes in on **stdin**, not as an argument. Arguments are visible in
`ps` to every user on the box.

## Related

- AGENTS.md rule 1 — model judgement never authorises capital. This decision is
  about a credential rather than about capital, but it is the same shape: the
  component that handles untrusted content holds as little authority as possible.
- AGENTS.md rule 8 — missing configuration refuses. No budget, no agent.
- [ADR 0003](0003-legacy-transactions-because-the-signer-must-be-able-to-read-them.md)
  — the other place in Radar where a separate process exists so that one property
  can be stated absolutely.
