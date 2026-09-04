<!-- SPDX-License-Identifier: Apache-2.0 -->
# Design 0005 — What `radar-graph` should refuse, after `0024`

**Date:** 2026-09-03
**Status:** **proposal, for Josh to accept, change or reject. Not a decision, and
nothing here is implemented.** It changes what Radar refuses, which is a decision
about money rather than a wiring task (AGENTS.md rule 1 and §3). Every number
below is read from
[`docs/research/data/0024-base-rates.json`](../research/data/0024-base-rates.json)
or from `crates/radar-graph/src/lib.rs`, both at `origin/main`.

## Why this exists

[PR #110](https://github.com/hey-vera/radar/pull/110) deferred it in one line:

> `radar-graph`'s thresholds derive from 0008 and should be re-derived. That
> changes a refusal rule, which is a separate change with its own tests.

Deferring it was right. Leaving it undocumented was not: `radar-graph` is the
coordination gate, and it is currently tuned to a distribution
[`0024`](../research/0024-the-spike-became-a-hump-and-the-signal-moved.md)
withdrew. The snapshot even records the supersession — `"supersedes": "0008"` —
and nothing in the crate reads it.

This note measures the gap. It proposes no code.

## 1. A retracted figure is stated as fact in production code

`crates/radar-graph/src/lib.rs`:136, the doc comment on `BUNDLE_CENTRE`:

> 68% of instantly-graduating launches had exactly this many recipients, against
> 5% of launches that never graduated.

`0024` measured **25.1%** and **5.5%**, on 17,497 launches against `0008`'s 80
per population. The snapshot's own comment is blunter than anything here:

> 0008 reported 0.05 / 0.16 / 0.68. Quoting "six, versus 5% otherwise" is wrong
> twice: the instant figure is 2.7x too high, and "otherwise" hides the organic
> cohort.

The 68% is the number `GOAL.md`'s sample roast quoted and Phase 0 corrected
everywhere else. This is where it still lives.

## 2. The lift constants, measured against `0024`

| Constant | In the code | `0024` measures | |
|---|---|---|---|
| `CENTRE_LIFT_X100` (six) | **11.7x** | **4.4x** | 2.7x too high |
| `BAND_LIFT_X100` (5–7) | **6.7x** | **3.2x** | 2.1x too high |
| `MEASURED_CENTRE_RATE_BPS` | 580 | 570 | holds |
| `MEASURED_BAND_RATE_BPS` | **1,310** | **1,700** | 390 bps low |

Two different kinds of wrong, and they fail in opposite directions.

**The lifts overstate the signal.** A launch with six recipients is treated as
carrying 11.7x the base instant-graduation rate when it carries 4.4x. Six is
still enriched — the signal is real — but by a factor the evidence no longer
supports.

**`MEASURED_BAND_RATE_BPS` desensitises the decay detector.** That constant is
the comparison point for "has the bundler's default moved off six". It says 13.1%
of all launches fall in 5–7; `0024` measures 17.0%. A detector comparing a live
sample against a baseline that is 30% too low will read a healthy population as
*over*-represented and will not fire. **The check most likely to notice that this
whole gate has gone stale is the one calibrated so it cannot.** That is
LEARNINGS 5's shape — a check that cannot report absence — pointed at itself.

`MEASURED_CENTRE_RATE_BPS` at 580 against a measured 570 is fine and should be
left alone.

## 3. The gate is pointed at the weaker band

`assess` recognises exactly two shapes: six, and five-to-seven. `0024`'s bands,
by instant-graduation lift over base:

| Band | Share of launches | p(instant) | Lift |
|---|---|---|---|
| one to three | **70.5%** | 0.0002 | **0.0x** |
| exactly six | 5.7% | 0.0517 | 4.4x |
| five to seven | 17.0% | 0.0383 | 3.2x |
| **ten to thirteen** | 2.1% | 0.1200 | **10.1x** |

**The strongest band in the measurement is the one `radar-graph` does not
have.** Ten-to-thirteen carries 10.1x — close to the 11.7x the code currently
attributes to six — and `assess` scores it `Unremarkable`, the neutral
multiplier. `0008`'s headline did not survive; the *magnitude* it claimed did,
and it moved.

## 4. The finding with the most population behind it is not represented at all

One to three recipients covers **70.5% of all launches** and graduates instantly
**0.02%** of the time — 2 in 12,908, upper bound 0.06%. Lift over base: 0.0x.

`assess` scores that `Unremarkable` too, identically to a launch with forty
recipients about which nothing is known. Those are not the same statement. One is
"no evidence either way"; the other is "measured, over most of the population, and
it almost never happens."

This is the **refuse** signal, and it is the one `0024` states most confidently.
`radar-graph` is a gate, and the strongest thing it could say is currently
unsayable in its own vocabulary.

## 5. What I would propose, and where it is weakest

Not implemented, and deliberately separable so parts can be rejected
independently.

1. **Correct the doc comment on `BUNDLE_CENTRE` now.** It is a retracted number
   stated as fact, it changes no behaviour, and it is the only item here that is
   not a decision about money. *(§1)*
2. **Re-derive the four constants from the snapshot rather than from prose**, so
   the next re-measurement moves them by editing data. The snapshot already
   carries `supersedes` and `measured_on`, and `radar-roast` already reads it —
   `radar-graph` does not. *(§2)*
3. **Fix `MEASURED_BAND_RATE_BPS` first and separately.** It is the decay
   detector's baseline, it is wrong in the direction that silences the detector,
   and correcting it changes no refusal. *(§2)*
4. **Add the ten-to-thirteen band**, which is a change to what Radar treats as
   coordinated. *(§3)*
5. **Give `Coordination` a variant for measured-and-negligible**, so one-to-three
   stops sharing a verdict with the genuinely unknown. *(§4)*

**Where this is weakest.** `0024` is one twelve-hour window on one day, 17,497
launches, and it says so. Re-deriving constants from a single window is how
`0008` got here — and `0008`'s own warning came true, which is the strongest
argument for reading the constants from a dated snapshot rather than baking them
in again. Items 4 and 5 change refusals and should not be taken on one window
without a second one agreeing.

**What I did not check.** Whether `radar-backfill`'s reporting, which reads
`BUNDLE_BAND` at `launch_block.rs`:165, has recorded enough shapes for an
independent re-measurement from the store. If it has, that second window exists
already and items 4 and 5 get much cheaper.
