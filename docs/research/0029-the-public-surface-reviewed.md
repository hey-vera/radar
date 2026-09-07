<!-- SPDX-License-Identifier: Apache-2.0 -->
# Research 0029 — The public surface, reviewed

**Status:** **complete.** Every finding below has a disposition, and every
disposition names either a merged change or the reason nothing was changed.

**What this is.** Plan 0008 item 5 asked for a review of everything a stranger
can reach — the X account, the public endpoints, the site, and the payment path
behind them. Fifteen findings (S1–S15) came out of that review and were
recorded in plan 0008's table. Plan 0009's own review added fifteen more
(S16–S30). This is the single place both sets live, re-verified at the commit
this document was written against, with what actually happened to each.

**Written last on purpose.** A review written before the fixes is a list of
worries. This one cites what landed, so a reader can check each line rather
than take it.

**Re-verified at:** `9c98617`, 2026-09-07 — the tip after plan 0009 items 1–4
and 6 merged. Every "stands" and "fixed" below was checked against that
commit's source, not recalled.

**Written by:** Claude Opus 5, 2026-09-06 to 2026-09-07.

---

## How to read the severity column

A severity is about **what a stranger could make happen**, not about how hard
the fix was. "High" means somebody could take money, be paid money they did not
win, or be publicly accused of something the data does not support. "Latent"
means the code is wrong and the state that triggers it has not occurred yet —
usually because no token exists.

---

## S1–S15, from plan 0008, re-verified

| # | finding | severity | where it stands now |
|---|---|---|---|
| S1 | a winner's summons inside the claim window is taken as a claim, and the prize goes to a mint address | high | **fixed.** `try_claim` requires `record.claim_prompt == Some(parent)` — a claim must be a reply to the prompt, and a summons is not. `contest.rs:484` |
| S2 | the claim address is never checked to be a wallet; a program account would be paid | medium | **fixed twice.** `try_claim` reads the owner and refuses anything the system program does not own (`contest.rs:499`); `radar-payout::pay` refuses again before signing |
| S3 | the week-close reads are unmetered and `Cost::PostRead` is charged nowhere | low | **fixed.** `Cost::UserRead` was added with the verified scan and is a required price; `Cost::PostRead` exists and is charged where a post is read |
| S4 | the leaderboard publishes numeric author ids labelled as handles | low | **fixed** at #172 and again on `/v1/public/weeks`: the handle is shown when the record read one, the numeric id is the link, and with no handle the id is shown bare — `@1234567890` reads as a name somebody chose |
| S5 | `try_claim` lists the contest directory once per mention | low | **reduced.** The listing is behind the prompt check, so a mention that is not a reply to a prompt does not reach it |
| S6 | the appointment posts skip `render::for_publication` | low | **stands, with the reason.** They go through `weekly::check`, which is `forbidden::check` and `fidelity::check` — the two that decide whether a post may be made. `for_publication` is a reply-shaping step and there is no reply to shape |
| S7 | the CORS origin is echoed from config verbatim | none | **stands.** It is operator configuration, set to one origin and never `*`, and unset means the header is absent |
| S8 | the site builds hrefs from API fields | low | **fixed.** Every outbound link goes through `safeHref`, `userHref` or `solscanTx`, each of which returns `null` rather than a link it cannot vouch for |
| S9 | digits inside base58 addresses are blanked before the fidelity check | verified | **stands.** `blank_addresses` runs first in `fidelity`, so an address's digits cannot be read as a fabricated figure |
| S10 | mention text never reaches the model and metadata is fenced | verified | **stands**, and is pinned end to end by `an_adversarial_mention_cannot_change_the_reply.rs` |
| S11 | `records_in` skips a file that does not parse, so a schema change without a default silently drops weeks | medium, latent | **mitigated, not closed.** Every field added since carries `serde(default)` and the doc comment on the function names the hazard. The skip is still silent, and that is the residual: a torn write is not evidence and the weeks either side still are |
| S12 | week records are world-readable and will carry engager ids | none | **stands, and is now load-bearing.** The verified scan reads engager ids and writes **counts**; no id reaches the record or the public JSON |
| S13 | with no API base the site silently shows its fixture | low | **fixed.** `Sourced.stale` travels with every figure and the page says which it is showing |
| S14 | the payout trusts a record another process wrote | accepted | **accepted, and the blast radius is stated.** ADR 0013: the key holds no tokens and one week of creator fees |
| S15 | `index.html` claimed a test that did not exist | none | **fixed.** `figures.test.ts` now checks the copy crawlers read |

---

## S16–S30, from this review

| # | finding | severity | disposition |
|---|---|---|---|
| S16 | quotes and replies are unbounded per account, so one account farms the top score for nothing | high | **fixed.** The score now prefers `Verified` — distinct accounts that reposted, quoted or liked, each counted once and each old enough — and keeps the raw metrics beside it as evidence |
| S17 | any refusal, including "somebody asked first", excludes an entrant for the week; the global cap can be burned to exclude everyone | high | **fixed.** `RefusalKind::costs_the_week` returns true for exactly one kind, the per-summoner daily cap. Being refused because somebody else was ahead of you is not a thing you did |
| S18 | `--due` pays the whole vault to whichever due week the directory lists first; no floor | medium, latent | **fixed.** Sorted ascending — the earliest due week has waited longest — and `Refusal::BelowFloor` from `RADAR_PAYOUT_FLOOR_LAMPORTS` rolls a thin week over |
| S19 | a claim that mentions the coin before the wallet claims the mint; a program-owned recipient is paid | high | **fixed.** See S1 and S2 |
| S20 | the model call is metered nowhere | medium | **fixed.** The call is reserved before `answer` and settled against what the provider reported; an unreported cost is `Billed::Unreported`, which is not zero |
| S21 | the gate's ignore list is the literal `radar`, never the bot's id | low | **fixed.** `ignored` returns the account's own id from the credential |
| S22 | the analyst runs on the free public RPC | medium | **fixed.** `RADAR_RPC` points at Helius. Measured before: 7 of 10 reads returned 429. After: 10 of 10, no 429s |
| S23 | `RADAR_CONTEST_OPERATORS` set twice; nothing prints the set in force | low | **fixed.** The duplicate line is gone, and the analyst prints `operators: N ids` on start — which is the only visible difference between a correct file and one that leaves the managing account eligible to win |
| S24 | no alert channel on a public bot | medium | **fixed.** A private channel, on its own bot, that publishes when the system is broken |
| S25 | the seven-days timer is not installed | low | **open, and Josh's.** It is a root command. Without it the first daily post finds no file and posts nothing |
| S26 | the public leaderboard folds the whole reply log per request; the edge cache is per URL, so a query string busts it | low | **recorded, not fixed.** The fold is a file read, not a store scan, and the endpoint is edge-cached for sixty seconds. The query-string hole is a Cloudflare cache rule and is Josh's. Worth doing before the first link goes wide; not worth code |
| S27 | the leaderboard links by handle, which can be reassigned | low | **fixed.** The link is always `userHref(id)`; the handle is only ever the label |
| S28 | the claim prompt may be refused by X's reply rule, and it does not mention the winner | high until tested | **fixed by construction, still untested live.** The prompt now goes under the winner's own summons, which is the one reply X guarantees, with the bot's winning reply as the fallback for the single week closed before mention ids were recorded. **Nothing has posted one yet**, because no week has had a winner |
| S29 | the payout timer and a hand payment can both send | low | **fixed in the runbook.** Stop `radar-payout.timer` before a hand payment. A code fix would be a lock, and the operator is one person |
| S30 | AI reply bots may need X's written approval | unknown | **open, and only Josh can close it.** The rules page returned 403 to the session that found this, so it is a report and not a verified fact — and it stopped being hypothetical the moment a model provider was configured. Design 0014 |

---

## The four that are still open, and why

Everything above is either fixed or has a named reason. Four are open, and they
split cleanly into two kinds.

**Two need a root command or an account action from Josh** — S25 (the daily
timer) and S30 (the automation-rules read). Neither is a code question and
neither can be closed from this side.

**One needs a Cloudflare rule** — S26. The endpoint is cached at the edge for
sixty seconds, and the cache key includes the query string, so
`?x=1`, `?x=2` and so on each miss. A rule that ignores the query string on
`/v1/public/*` closes it. Nothing in the repository can do this and no code
change would help, because the fold behind it is already cheap.

**One is a live test that has not been possible** — S28. The claim prompt is
built against the reply rule X published on 2026-02-23, and it has never been
posted, because no week has had a winner. The first one is the test. If it is
refused, that is a `LEARNINGS.md` entry — *a mechanism built against a platform
rule that had already changed* — and if it is accepted, this line closes with
the post id.

---

## What this review did not look at

Said plainly, because a review that does not name its edges reads as more
complete than it is.

- **The trading path.** No customer capital has ever moved through it and the
  signer is not installed. It has its own reviews and its own ADRs.
- **The store.** Read-only from every public endpoint, and the endpoints read
  published files rather than the store on purpose (design 0008 §4).
- **The model's output as a legal surface.** `forbidden::check` refuses the
  verdicts and `fidelity::check` refuses every number not on the sheet, and
  both are mechanical. Whether that is *enough* is design 0007 J4's question
  and is a lawyer's, not a reviewer's.
