<!-- SPDX-License-Identifier: Apache-2.0 -->
# Plan 0008 — The face, the claim, the scan and the voice

**Status:** in progress
**Branch:** `site/the-face` for item 1;
`feat/the-claim-is-a-reply-to-the-prompt` for item 2;
`feat/scan-the-winner-before-paying` for item 3;
`feat/a-voice-with-teeth` for item 4;
`docs/the-public-surface-reviewed` for item 5. All from `main`.
**Base:** `08156b1`, `main` at #159
**Planned by:** Fable 5.1, plan mode, 2026-09-06
**Designs:** [0007](../design/0007-the-end-to-end-plan.md) §6.2 and C3;
[0008](../design/0008-the-public-site.md) §5.2 and §10;
[0009](../design/0009-three-loops-and-no-formula.md) §2 and §6;
[ADR 0013](../adr/0013-a-community-token-exists-and-radar-holds-none-of-it.md)

## Objective

Afterwards cabalhunter.org is a page a stranger arriving from a reply on a
phone reads to the end, with a tokenomics page and a one-tap route to summon
the account; the claim flow is stated on the site and told to the winner in
their own thread; a winner is scanned before they are paid and the counts are
published beside the score; the bot's replies lead with the fact that is about
*this* coin and every number in them is still on the fact sheet; and the public
surface has been read adversarially with each finding recorded and the one real
defect fixed.

The defect, stated once: `try_claim` accepts any 32-byte base58 address in any
mention by the winner inside the claim window, and runs before `answer`. A
coin's mint address is such an address. A winner who summons the bot during
their own claim week has that mint recorded as their payout address, and
`verify` would confirm the payment, because recipient equals claim. Design 0007
§6.2 specifies the mechanism that prevents this — the bot replies to the winner
and the claim is a reply to *that* — and it was never built.

## Not in scope

- `Policy::CLOSED`, the kernel, `radar-signer`, `radar-exec`, `creator_edge`
  and `radar-graph` thresholds, the store's schema, the recorder.
- The forbidden list's reassurance and advice rules. Nothing is removed from
  `forbidden.rs`.
- Designs 0008, 0009, 0010 and ADR 0013. They decay by their own rule;
  corrections go to [`docs/STATE.md`](../STATE.md).
- Anything that spends money or changes what the account says in public without
  the owner's switch. Every post shape added here lands in the dry-run log
  first, and `RADAR_X_PUBLISH=on` is his.
- A light colour theme for the site. Dark-only is the identity `index.css`
  argues for; recorded as a judgement, not an oversight.
- Metering the existing week-close reads. Recorded as finding S3; fixing it
  changes a shipped path the owner has not asked about.

## Items

- [x] 1. **The site, wholesale.** `site/`. A display face, a fluid type scale,
      a numbered argument that reads top to bottom on a phone, the launch block
      as an illustration, the real reply as a receipt, and a one-tap summon
      box. `new:site/src/Token.tsx` for the tokenomics page; `/token` in
      `routes.ts`; `HowToClaim` steps on the leaderboard;
      `new:site/src/figures.test.ts` pinning `index.html`'s figures to the
      fixture, the fixture's bands to `0024-base-rates.json`, the fee ladder to
      the capture, and every page against a verdict-word list;
      `new:site/public/og.png` at 1200x630 with `new:scripts/site/og.ps1` that
      made it; `honesty.ts` gains `safeHref` and its family and loses nothing.
      Fixture refreshed from the box's index at slot 444,637,451.
      done: `just site` green with 49 tests at the raised floor, at `20f8b12`;
      `cargo test -p repo-conformance` 33 passed; browser at 375 and 1280 with
      no horizontal overflow on any of the five routes and a clean console.
      Three departures from the plan, each argued in the commit: the fee ladder
      is pinned by `radar-pumpfun`'s decoder rather than by a TypeScript copy of
      it; the account handle is build config rather than a literal, because
      `@thecabalhunter` is in no file and the analyst still has no publisher;
      and there is no header CTA, because the fifth nav item is what `routes.ts`
      already records eating a 375px header. Geist measured at 29.4 kB for the
      latin subset, on headings only.

- [x] 2. **The claim is a reply to the prompt.** After the week closes the
      account replies to the winner, under its own winning reply, and the claim
      must be a reply to that post. `try_claim` checks `mention.parent` before
      it parses, so a summons is never a claim. `Record.claim_prompt` and
      `Winner.handle` land with `serde(default)`, because `records_in` skips a
      file it cannot parse and a missing default would drop week 2956 from the
      leaderboard and from the cooldown. `accounts()` returns the username the
      same call already pays for. The leaderboard JSON gains `handle` and the
      exclusion counts.
      done: `just ci` green -- 1,848 tests against a floor of 1,591, clippy and
      fmt clean -- and the fix verified by re-applying it: deleting the
      `claim_prompt` equality fails two tests by name. Stricter than the design
      on purpose (a reply under the winning reply is NOT a claim), with the
      daemon re-posting the prompt every tick so strictness costs a delay rather
      than a week. Two corrections recorded rather than buried:
      `#[serde(default)]` is redundant on an `Option` and the 2956.json test is
      the real enforcement; and the early parent guard is an S5 optimisation,
      not the security property. The site half -- rendering `handle` -- waits on
      item 1's helpers rather than editing the same files on two branches.

- [ ] 3. **Scan the winner before paying.** `new:crates/radar-onchain/src/funder.rs`
      builds the first-funder read from `signatures_back_to_oldest` and
      `transaction`; the X client gains engager reads metered as
      `Cost::PostRead`; `new:crates/radar-contest/src/scan.rs` holds the pure
      facts and `new:crates/radar-analyst/src/scan.rs` gathers them on the tick
      after a claim lands. `Refusal::Unscanned` and `Refusal::NotAWallet` gate
      the payout. No count refuses payment: the gate is "scanned, and a wallet",
      and the counts are published as counts.
      SUPERSEDED 2026-09-06 by [design 0011](../design/0011-scan-before-declaring-not-before-paying.md).
      The owner asked whether the scan belongs before the *winner is declared*
      rather than before they are paid, so a bought account never reaches the
      leaderboard. It does. 0011 rewrites this item: scan **down the ranking**
      rather than across it (bounded cost -- 3-9 reads a week, not ~1,800),
      publish the **measurement** and never a verdict, and **exclude nobody**
      until a baseline exists, because the account has never posted and there is
      nothing to calibrate a threshold against. The claim-address check stays at
      payout; that object does not exist until the claim.
      next: implement 0011 phase 1

- [x] 4. **A voice with teeth.** `voice.rs`'s rule 6 currently says to lead with
      the cost line and `verdict.rs` demoted that line on 2026-09-05 for being
      the same 456 bps in every reply; the prompt and the template disagree and
      the prompt is the one that is wrong. A deterministic `headline` gives the
      model an anchor already on the sheet. `SYSTEM` carries no ASCII digit.
      done: `just ci` green -- 1,848 tests, clippy and fmt clean. `headline` is
      `None` on a sheet with no creator record and no launch block, and the
      launch-block fallback was caught by its own test matching a label the
      sheet does not emit. `SYSTEM` carries no digit outside its rule numbers.

- [ ] 5. **The public surface, reviewed.**
      `new:docs/research/0029-the-public-surface-reviewed.md` — fifteen
      findings, each with its evidence, its severity, and either the item that
      fixed it or the reason it stands.
      next: last, so it cites what landed

## The findings this plan answers

Re-verified against the tip when 0029 is written.

| # | finding | severity | disposition |
|---|---|---|---|
| S1 | a winner's summons inside the claim window is taken as a claim and the prize goes to a mint address | high | item 2 |
| S2 | the claim address is never checked to be a wallet; a program account would be paid | medium | item 3 |
| S3 | the week-close reads are unmetered and `Cost::PostRead` is charged nowhere | low | recorded; the owner's call |
| S4 | the leaderboard publishes numeric author ids labelled as handles | low | item 2 |
| S5 | `try_claim` lists the contest directory once per mention | low | reduced in item 2 |
| S6 | the appointment posts skip `render::for_publication` | low | stands, with the reason |
| S7 | the CORS origin is echoed from config verbatim | none | stands; operator config |
| S8 | the site builds hrefs from API fields | low | item 1 |
| S9 | digits inside base58 addresses are blanked before the fidelity check | verified | stands |
| S10 | mention text never reaches the model and metadata is fenced | verified | stands |
| S11 | `records_in` skips a file that does not parse, so a schema change without a default silently drops weeks | medium, latent | mitigated in items 2 and 3 |
| S12 | week records are world-readable and will carry engager ids | none | ids never enter the public JSON |
| S13 | with no API base the site silently shows its fixture | low | the page says so |
| S14 | the payout trusts a record another process wrote | accepted | ADR 0013's blast radius |
| S15 | `index.html` claimed a test that did not exist | none | item 1 |

## Verification

- **Re-apply the bug** for every rule added: the parent rule (2); the
  `Unscanned` and `NotAWallet` arms and the wrong-destination transfer that is
  not a funder (3); the `serde(default)`s (2, 3); the headline on an unknown
  creator (4).
- **Two instruments compared** wherever a figure is produced: the fixture
  against the box's index; the fee-ladder fixture against the capture, by test;
  the OG image's dimensions against the meta tags, by test.
- **The browser, not a screenshot of intent** — design 0008 §10. Every route
  read, console clean, two widths.
- **Mutants** in CI on each item's changed lines. A survivor becomes a test or
  a `.cargo/mutants.toml` entry with its reason.
- `cargo test -p repo-conformance` on every document created or changed.

## Open questions

Each carries the answer assumed if the owner says nothing.

- **Q1** Should any scan count refuse payment automatically? **No.** The
  shared-funder rule would fire on two honest winners funded from one exchange
  hot wallet, and Radar has no exchange list. A check that fires on a
  reasonable case spends the credibility of every other check.
- **Q2** The claim prompt is a new public post shape, one reply a week. **Yes**
  — it is design 0007 §6.2's own mechanism, and it lands in the dry-run log
  first.
- **Q3** `forbidden.rs` refuses "cabal", so no post can carry the site's name
  or the account's. **Leave it.** `weekly.rs` already chose this, and an
  exact-string allowance is an exemption path the checks were built not to
  have. The bio carries both. Raised rather than routed around.
- **Q4** Handles on the leaderboard, read at week close from the call that
  already runs. **Yes** — design 0008 §5.2 asked for them. The posts stay
  URL-only.
- **Q5** One self-hosted display face, roughly 40-60 kB on a 72 kB page.
  **Yes**, measured in the pull request body and vetoable by reading the size.
- **Q6** Reading the winner's engager list once a week. Design 0009 §6 refused
  reading engagers to *weight scores*; this reads one account's engagers to
  *check the winner* and publishes counts. **Yes**, recorded in `docs/STATE.md`
  as a refinement, not a reversal.
- **Q7** The OG image generator is a PowerShell script in a Linux-first
  repository. **Commit the PNG and the script**, with a header saying so; the
  test checks only the file.

## Only the owner can do

- Install `radar-serve` from the merged build so the public endpoints answer,
  and set `RADAR_SITE_ORIGIN` and `RADAR_CONTEST_DIR` on it. Until then the
  site shows its fixture and says so.
- A Cloudflare Access bypass for the public routes, and `VITE_API_BASE` in the
  Pages build environment.
- The account bio: the site's name, "automated account, operated by Josh Fair",
  and one sentence on how to claim — the only place the site's name can appear.
- `RADAR_X_PUBLISH=on`, after reading the dry-run replies.

## Handback

Written by the last item to land. Until then: stopped at nothing, started
nothing.
