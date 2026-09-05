<!-- SPDX-License-Identifier: Apache-2.0 -->
# Plan 0005 — Cabal Hunter goes online

**Status:** in progress
**Branch:** `feat/cabal-hunter-the-public-site`
**Base:** `4fa83f9`
**Planned by:** Opus 5, plan mode, 2026-09-05
**Design:** [0008](../design/0008-the-public-site.md)

## Objective

Radar has a public face. A stranger who clicks through from one of the bot's
replies lands on a page that says what this measures, shows figures nobody else
has, and makes the weekly contest legible enough to enter — served as static
files that a viral spike cannot knock over, and honest about the two things it
cannot know yet.

## Not in scope

- The operator console at `web/`. It stays behind Cloudflare Access, untouched.
- Anything that reads the store per request. The 3.2 s scan is why this site is
  static and file-backed.
- `Policy::CLOSED`, the custody lane, the trading lane.
- The token itself. This builds the page that will report it; ADR 0013 governs
  the launch and it is Josh's.
- Visual identity as a finished thing. The first build is a draft to react to.

## Items

- [x] 1. Design 0008 and this plan in the repository
      done: committed together at 6cf32c8; `repo-conformance` 33 passed
- [x] 2. `site/` scaffolded — Vite, React 19, TS, Tailwind 4, vitest, wouter
      done: `just site` green; 226 kB / 71.6 kB gzipped
- [x] 3. The design layer: `ui/` primitives, palette, entrance animation
      done: **no `motion`** — measured at 123 kB raw / 41 kB gzipped for one
      fade-in, so it is eight lines of CSS instead. No Radix either: none of the
      four pages has an overlay, and a layer with no caller is what AGENTS.md §5
      refuses. The note saying when to add it is in `site/src/ui/index.tsx`.
- [x] 4. Home — the whole argument, real figures, from a committed fixture
      done: verified in the browser at 375px and desktop
- [x] 5. Leaderboard — the shape, and the honest empty state
      done: `empty.test.tsx`, and the re-applied bug fails it
- [x] 6. Prize pool — the shape, and "no token exists" rather than `0.00`
      done: same file; asserts the rendered text contains no `0.00`
- [x] 7. About — the disclosure X policy requires (design 0007 item B7)
- [x] 8. Open Graph and Twitter card meta, and the `<noscript>` block
      partial: the tags and the noscript block are in `site/index.html`. **The
      OG image itself is not made** — `og.png` is referenced and absent, so the
      card will render without an image until it exists. Item 11.
- [x] 9. `just site`, a CI job, and the `required-checks.txt` line
      done: `just site` and `just web` both green after the shared recipe was
      parameterised. The **ruleset** edit is Josh's and is not done — the line
      is a check that looks like a gate and is not one, said in that file.
- [ ] 11. The 1200×630 OG image
      next: it is the first thing anybody sees of this product on X
- [ ] 10. Phase 1 — the three public endpoints, by exact path, with CORS

## Open questions for Josh

- Q1 (2026-09-05): the `site` status check has to be added to the `main` ruleset
  as well as to the workflow and `required-checks.txt`. That third place needs
  admin. — unanswered, does not block items 1–8.

## Handback

Stopped at: Phase 0 complete. The site builds, tests and renders; four pages,
19 tests, 71.6 kB gzipped.

Next action: item 11 (the OG image — `site/index.html` references `og.png` and
it does not exist), then item 10 (the three public endpoints).

Two bugs this found, both by looking at the running page rather than by a test:
the round trip rendered as `+4.6%` — a cost signed as a gain, on the one figure
the page warns people with — and a `text-[var(--color-dim)}` typo that produced
no class at all. `honesty.ts` now separates `cost` from `bps` and says why.

Do not: add public paths by prefix in `access.rs` — exact paths only, for the
reason its own comments give. Do not add `motion` back without measuring it.
