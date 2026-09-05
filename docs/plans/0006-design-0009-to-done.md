<!-- SPDX-License-Identifier: Apache-2.0 -->
# Plan 0006 — Design 0009 to done

**Status:** in progress
**Branch:** `feat/contest-crate` for item 1; one branch per item after it, each
carrying this file so it is always a true statement about `main`
**Base:** `a621453`
**Planned by:** Fable 5.1, 2026-09-05
**Design:** [0009](../design/0009-three-loops-and-no-formula.md)

## Objective

Every mechanism in design 0009 that can be built and proved **without** the X
credential, the token, or a root command on the box is built, proved and on
`main`. What remains afterwards is one list of things only Josh can do, each
with what it unblocks. Then, and not before, the vision prompt — the slash
command on PR #146, held open until this plan is done — runs. One plan at a
time.

## Not in scope

- Anything blocked on the credential (C2's scoring reads, the 24-hour live dry
  run), on the token (C7's launch, a real vault balance), or on root on the
  box (installing the analyst unit, the alert file under `/etc/radar`).
- Design 0007's D, E and F beyond what 0009 needs. They are Part A of the
  vision prompt.
- Reopening ADR 0013 or 0009's L1–L6.
- `Policy::CLOSED`, the signer, the trading lane. Nothing here reaches them.

## Items

- [x] 1. **`radar-contest`, pure** (0007 C1, 0009 M3's rule): the week, the
      entry, the exclusions, the score, the winner with a cooldown, the claim,
      the ledger, and the hunter sum with a per-day cap. No clock, no network.
      Caller: item 2's leaderboard endpoint reads its ledger type; item 6's
      week-close job writes it.
      done: `cargo test -p radar-contest` 24 passed; clippy clean; five rules
      re-applied as bugs — lowest score winning, the cooldown a week short, an
      exact payout refused, the cap letting one extra through, the week
      boundary moved a day — each failing exactly its test; the first commit
      on `feat/contest-crate`. Mutation coverage: CI's sharded run on the PR,
      per AGENTS.md §8; survivors, if any, are the next commit
- [x] 2. **The three public endpoints** (0008 phase 1): `/v1/public/stats`,
      `/v1/public/leaderboard`, `/v1/public/pool`, added to `Audience::Public`
      by exact path, CORS for the site's origin only. The leaderboard reads item
      1's ledger; the pool says no token exists rather than `0.00` (rule 9).
      done: `crates/radar-serve/src/public.rs`, nine tests over absent and
      present files; the access table and the router-level guard test carry
      the three paths and refuse a fourth; `cargo test` green on radar-types,
      radar-roast, radar-contest, radar-cli, radar-serve; clippy clean;
      conformance 33. Four bugs re-applied, each failing exactly its test: an
      unscored entry rendered as `0`; a missing aftermath padded from memory;
      one document dropped from the public list; the summary write dropped.
      Two things it needed and got: the creator-index job now publishes
      `population.json` beside the index, so the endpoint reads five totals
      rather than 116,000 records; and research 0011's figure travels in the
      base-rate snapshot with its own date. One date function now lives in
      `radar-types` and the CLI calls it, instead of a second copy. CI's
      mutants named four survivors on the PR — the three handlers replaced
      by an empty 200, and the record filename rule's `&&` — and the second
      commit reads the three bodies at the router and pins both halves of
      the name rule; each re-applied by hand, each failing its test
- [x] 3. **M6 — the fee after graduation, measured.** Capture a PumpSwap swap
      for a graduated pump.fun token; does it pass the fee program and its
      config; re-read the schedule today; write research 0028; correct the pool
      page copy only if the chain disagrees with 30 bps.
      done: 2026-09-05, `docs/research/0028-the-fee-after-graduation-is-a-ladder.md`.
      The fee program keeps a 25-row schedule for the PumpSwap AMM, read by
      `fees.rs` unchanged and captured in
      `crates/radar-pumpfun/tests/fixtures/pumpswap_fees.json`; six tests in
      `crates/radar-pumpfun/tests/the_fee_after_graduation_is_a_ladder.rs`
      walk the ladder and the fossils. A buy and a sell pass the fee program
      and pay the rows to the lamport. The chain disagrees with a flat 30 bps
      above 420 SOL of market cap, so the pool page, ADR 0013 and design 0009
      §1 now say which fee is which; the site test asserts the qualifier and
      fails with it removed. Open: two of six pools paid a row a higher cap
      selects — recorded as a hypothesis, not coded
- [x] 4. **Refusal signals counted at answer time** (M3's other half, and
      0007 B's live gate): a signal count on the fact sheet, carried on the
      reply log entry; the three X-shaped adversarial cases — an instruction in
      a mention, a parent holding an LP mint, a 30-mention burst from one
      account — each producing a sheet or a refusal.
      done: 2026-09-05, `feat/refusal-signals`. `radar_roast::sheet::Signal`,
      three variants, computed in `FactSheet::build` and never rendered; the
      strongest band is `BaseRates::strongest_band`, read off the snapshot;
      `log::Entry.signals` is `Option<Vec<Signal>>`, `None` on a line older
      than the field. The burst case found the bug: the per-summoner cap
      counted *replies*, so thirty mentions with unreadable mints, or thirty
      while the publisher was down, were thirty dossiers and no refusal. The
      cap is now charged on admission; the global cap still on sending. The
      three cases are in `crates/radar-analyst/tests/one_poll_end_to_end.rs`
      with a chain that counts its requests; each re-applied bug fails its
      test (see the commit)
- [x] 5. **M5 — the Telegram publisher and source**, testable in dry run: an
      unset token means nothing is read and nothing is sent; the same parser,
      gate and fact path; not a contest entry and not in the record.
      gate: the daemon's posture line names the state; a fake platform drives
      one poll end to end
      done: 2026-09-05, `feat/telegram-lane`. `crates/radar-analyst/src/telegram.rs`
      — `getUpdates` parsed into the same `Mention` (ids `chat:message`, summoners
      `tg:<id>`), a `Publisher` that replies into the chat, its own caps
      (`RADAR_TELEGRAM_*`, unset = refuse), its own switch, its own log file
      `telegram.jsonl`, which is what keeps it out of the record; a `tick`
      the daemon runs after the X one. Posture line names off / log-only /
      live. End to end: a fake Telegram drives one poll; a message with an
      address is answered into `telegram.jsonl` and never `replies.jsonl`, a
      sticker is skipped and still acknowledged, the offset advances, and
      with no token nothing is read. Re-applied bugs in the commit
- [x] 6. **The weekly post (0007 B6, 0009 M2) and the daily "seven days later"
      post (M4)**, as code that runs in dry run and says "nothing yet" when the
      log is younger than seven days. The week-close job writes item 1's ledger.
      gate: both posts pass the fidelity and forbidden checks on real log lines
      done: 2026-09-05, `feat/weekly-and-daily-posts`. Three modules in
      `radar-analyst`: `contest.rs` closes the week — public metrics and
      account ages read from X, refusals from a new `refusals.jsonl` the gate
      now writes, the cooldown from earlier records, the rule applied, the
      record and the hunter tally written atomically; `weekly.rs` renders the
      result under 280 characters with every numeral authorised and the
      winner's coin torn down as the reply; `daily.rs` renders and posts the
      rows `radar seven-days-later` (new, in `radar-cli`, on
      `deploy/radar-seven-days.timer`) writes from the store — the join the
      analyst may not make. `Publisher::post` on every publisher, priced as
      `RADAR_X_PRICE_POST`. Both posts pass the two checks in tests that
      re-apply a dropped authorisation; a post that fails is recorded and not
      sent, and the thread stops. Nothing has been posted: no credential
- [x] 7. **C4 `radar-payout` and C5 the manual fallback**: the three policy
      refusals — wrong recipient, second payout for a week, amount above
      collected — each proven by re-applying the bug; its own unit and user; no
      network but RPC.
      gate: the refusals; the fallback prints the exact transaction
      done: 2026-09-05, `feat/radar-payout`. New crate `radar-payout` (lib +
      binary): `plan` from the record and the vault balance — the recipient is
      the claim, the amount is everything above the vault's rent reserve, so
      there is no field for a wrong one; the policy's three refusals via
      `Payout::permitted`; `sign` with the creator key; `verify` reads the
      transaction back and accepts exactly the planned transfer; `pay` records
      only after verification; `record_payout` is the fallback and goes
      through the same `verify`. `radar contest pay --dry-run` prints the
      unsigned transaction base64; `radar contest record-payout` records a
      hand-made one. C3 came with it: `try_claim` in the analyst writes a
      winner's address into the record inside the window and does not answer
      it as a summons. `collect_creator_fee` and a system transfer are built
      in `radar-pumpfun` from the program's on-chain IDL — a reference, not a
      capture; the devnet week is the capture. Own unit, user, key path and
      timer in `deploy/`. Re-applied bugs in the commit
- [x] 8. **C7 — the launch checklist** in `deploy/README.md`, and `radar brief`
      gains `contest` and `vault` checks that report Unknown when unreachable.
      gate: `radar brief` prints the two lines
      done: 2026-09-05, `feat/launch-checklist`. Nine steps under *The token:
      the launch checklist* in the deploy guide, the first five Josh's and the
      rest commands that exist. `contest` reports the latest closed week and
      where its prize stands; `vault` the reading `radar-payout` writes,
      Unknown when missing or more than two days old; both alarm on absence
      only once `RADAR_CONTEST_DIR` is set, on the analyst check's rule. Three
      tests; re-applied bugs in the commit
- [x] 9. **Design 0009 §1's sentence** "no crate contains that name" corrected
      to point at STATE.md, and every prose reference in this plan turned into
      a link once #143 has merged.
      done: folded into item 1's commit the day #143 and #145 merged;
      `cargo test -p repo-conformance` 33 passed

## Only Josh can do these, and what each unblocks

| what | unblocks |
|---|---|
| ~~merge #143, #144, #145~~ — done by the agent on 2026-09-05 once Josh granted it in chat. **#146 is held open on purpose** until this plan is done, so its remainder table is true when it runs | — |
| the analyst env file and the alert file under `/etc/radar`, and the unit install (root) | the 24-hour dry run; the alarm that says when the bot dies |
| the X developer account, its credit, the one live test post | C2's scoring reads; the live gate; everything in 0009's M1–M4 with real data |
| a Telegram bot token (BotFather, five minutes) | item 5's live test |
| a fresh wallet for the token, its key to the box | item 7's devnet week; C7 |
| the legal read (0007 J4; 0009 §8 has the documents) | the first public post |
| `cabalhunter.org` and its Cloudflare Pages project | the site going live |

## Open questions for Josh

- Q1 (2026-09-05): merging PRs was refused to the agent by the auto-mode
  permission classifier. — **answered the same day**: Josh granted it in chat
  and #143–#145 merged. A settings rule for `gh pr merge` would spare the
  question next time.

## Handback

Stopped at: **every item done.** Stacked in order: PR #147
(`feat/contest-crate`), #148 (`feat/public-endpoints`), #149
(`research/0028-the-fee-after-graduation`), #150 (`feat/refusal-signals`),
#151 (`feat/telegram-lane`), #152 (`feat/weekly-and-daily-posts`), #153
(`feat/radar-payout`), then `feat/launch-checklist`. All wait on merge
permission, and merge in that order; each later PR retargets `main` as the
one below it lands.
Next action: none in this plan. The only-Josh list stands (design 0009 §11,
the vision prompt's list): the `/etc/radar` files and the analyst unit; the X
developer account and the one live test post; a Telegram bot token and a
channel; the token's wallet, gated on J12; the legal read; the domain. After
that, `/plan-the-vision` (PR #146, its remainder table refreshed the same day)
in a fresh plan-mode session.
What this plan leaves stated rather than proven, all in STATE.md: the
`collect_creator_fee` account order (IDL, not a capture); the vault rent
reserve; the tier rule when a coin's market cap has fallen back through a
step (research 0028); and that none of the bot, the contest or the payout
has touched a live platform or chain, because no credential, wallet or token
exists.
The box needs the new `radar` binary before `population.json` exists there;
until then `/v1/public/stats` answers 404 and the site shows its dated
fixture, which is the designed behaviour.
Do not: reopen L1–L6; touch `Policy::CLOSED`; read the store per request in
any endpoint; add a public path by prefix in `access.rs`.
