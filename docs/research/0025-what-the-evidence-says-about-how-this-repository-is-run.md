<!-- SPDX-License-Identifier: Apache-2.0 -->
# 0025 — What the evidence says about how this repository is run

**Date:** 2026-09-03
**Method:** literature review. No measurement of this repository was taken.
**Status:** **a review of other people's evidence, not a finding of ours.** Every
number below is somebody else's, measured on somebody else's code, and each is
attributed. Three of this repository's standing habits do not survive it, one of
them added earlier the same day. Where the evidence runs out this note says so
rather than filling the gap with taste.

## Why this exists

Design [`0004`](../design/0004-documented-versus-enforced.md) produced a roadmap
of eleven items. Every one of them was justified by judgement — mine — and
judgement is exactly what this repository already distrusts in prose. Josh asked
the obvious question: does any of it actually work, or does it merely read well?

So this is the roadmap held against published evidence. It changes three things
and kills one.

## 1. The finding that reverses a decision made today

**`AGENTS.md` is 519 lines. The evidence says that is roughly 3.5x too long, and
that the excess is not free.**

Gloaguen, Mündler, Müller, Raychev and Vechev, *Evaluating `AGENTS.md`: Are
Repository-Level Context Files Helpful for Coding Agents?* (MemAgents @ ICLR
2026) evaluated context files on SWE-bench tasks plus a collection of issues
carrying developer-written context. Their headline: context files produced **no
improvement in task success rate while increasing inference cost by over 20%**.
The split reported from that work is that developer-written files help slightly
(about +4%) and generated ones hurt (about -3%) — but both pay the same cost.
Their conclusion is the part that matters here:

> unnecessary requirements from context files make tasks harder, and
> human-written context files should describe only minimal requirements

Two consequences, and the first is uncomfortable.

**a. The three paragraphs added to `AGENTS.md` §9 this morning were a mistake of
exactly the kind this note is checking for.** Josh asked for a communication
style; I wrote it into the file every agent loads on every turn, which is a
15-line tax on every task in the repository to fix a problem that occurs in the
last paragraph of a session. The instruction is right. The location is wrong.

**b. `0004`'s P2.1 — "trim `AGENTS.md` §4 rule 1, about 25-35 lines" — is the
right direction and far too timid.** It proposed removing 6% of the file. The
evidence argues for something closer to 70%, and it argues that the removed
material is not lost but *relocated*: to `docs/STATE.md`, to the checks in
`repo-conformance`, and to the plan file the session is actually working from.

The honest caveat: that study measures single-issue SWE-bench-shaped tasks, and
this repository's sessions are long, multi-hour and multi-file. A file that costs
20% on a ten-minute task may amortise differently over four hours. **Nobody has
measured that**, here or anywhere I could find. What survives the caveat is the
direction and the mechanism — instructions are followed at 1.6-2.5x baseline
rates, so a wrong or merely unnecessary line in that file is *obeyed*, which is
worse than ignored.

## 2. Josh's point, and it is the best-evidenced thing here

> *We don't need to measure every little thing to prove that it works. There are
> other ways to reason it. Don't default to testing and measuring everything.*

This is not a shortcut. It is the finding of the single most relevant paper to
how this repository works.

**Google's mutation testing at scale.** Petrović and Ivanković
([ICSE-SEIP 2018](https://research.google/pubs/state-of-mutation-testing-at-google/),
extended in [TSE 2021](https://arxiv.org/pdf/2102.11378)) ran mutation testing
across Google and found the naive form unusable: **more than 450 mutants for a
typical changelist.** Their fix was not a faster machine. It was to stop
generating most of them — skipping uncovered lines and what they named **"arid"
code, lines where a mutation teaches nothing** — which brought a typical
changelist to **fewer than 20 mutants, with a 25th-75th percentile of 3 to 19.**
Over 87% of mutant test runs kill the mutant, so the vast majority of that work
was confirming what was already true.

The lesson is Josh's, stated by people with Google's corpus behind them: **the
value is in the small number of checks that could fail, and generating the rest
costs real money and real attention.** This repository already half-knows it —
`.cargo/mutants.toml` exists to record equivalent mutants — but `AGENTS.md` §6
still says mutation testing is required on changed code with no arid-code
carve-out.

**The static-analysis half of the same argument.** Sadowski et al., *Lessons from
Building Static Analysis Tools at Google*
([CACM 2018](https://cacm.acm.org/research/lessons-from-building-static-analysis-tools-at-google/))
put a number on when a check is worth having: **an effective false-positive rate
under 10%**, where "effective" means the developer did not act on it — including
when they did not understand it. Their history is the warning: FindBugs' false
positives cost the *whole tool* its credibility, and trust had to be rebuilt from
zero with Tricorder.

Applied here: **a `repo-conformance` check that fires on something a reader would
not have changed is worse than no check.** Five were added today. Each was
verified to fail when its subject is broken, which is the necessary half. The
other half — does it stay quiet otherwise — is only observable over weeks.

**And the strongest form of the argument is older than either.** Minsky's *make
illegal states unrepresentable* and King's *parse, don't validate* say the same
thing from the other end: a property the type system carries needs no test,
because there is no program that violates it. This repository already does this
in its best places — `Amount` carries its unit in the type, so the pump.fun
field-meaning swap is a compile error rather than a test — and does not do it in
its worst. `AGENTS.md` invariant 3 claims `Observed<T>` cannot be unwrapped;
enforcement is a hand-written filter in `reader.rs`. **The document describes a
type-level guarantee that the code implements as a convention.**

**The rule this justifies**, in order, cheapest and strongest first:

1. **Make it impossible.** A type, a private field, an absent API. Costs nothing
   at review time and cannot regress.
2. **Make it one mechanical check** at one place, with a low false-positive rate.
3. **Test it** — when it is a behaviour, not a shape.
4. **Write it down** — only when 1-3 genuinely cannot carry it.

A thing enforced at level 1 must not also be tested at level 3. That is the
redundancy Josh named, and Google's arid-code result is what it costs.

## 3. What the delivery evidence actually supports

**Small batches.** The strongest empirical base in software delivery is DORA's,
now over a decade of survey data. Its consistent result is that batch size,
deployment frequency and lead time move together with stability rather than
against it. `0002` §5's "small PRs are the mutation-testing strategy" is
therefore doubly supported: it is right for the reason `0002` gave — mutant count
tracks the diff — and right for a reason `0002` did not know it had.

**AI is an amplifier, not a lift.** The [DORA 2025 report on AI-assisted
development](https://dora.dev/dora-report-2025/) surveyed roughly 5,000
practitioners: about 90% now use AI, and adoption correlates positively with
throughput **and with instability** — more change failures, more rework. Its
framing is that AI magnifies whatever the team already is. For this repository
the reading is direct: **the fleet makes the enforcement question more urgent,
not less.** A repository whose claims are checked gets faster. One whose claims
are prose gets wrong faster.

**Documentation decay is measured, and it is a defect source.** Wen et al., *A
Large-Scale Empirical Study on Code-Comment Inconsistencies* (ICPC 2019), mined
1.3 billion AST-level changes across 1,500 systems; older work on FreeBSD,
PostgreSQL and Eclipse connects comment-update practice to later bugs. This is
the published version of `0004`'s central finding and of LEARNINGS 10 — with the
useful extra that the dominant causes are **deprecation and refactoring**, which
is to say the document is not edited because the person editing the code never
opens it. That is an argument for `repo-conformance` reading manifests, not for
anybody promising to be careful.

## 4. Where the evidence runs out, and I am not going to pretend otherwise

- **Handback notes and `docs/plans/`.** No study. `0002`'s two-week kill
  condition is the correct response to an unevidenced idea and it stands.
- **ADR practice.** Widely adopted, essentially unmeasured.
- **Whether long agent sessions amortise a long context file.** The one relevant
  study measures short tasks. This is the largest gap under Josh's actual
  question and I could not close it.
- **Writing style for a tired reader.** No evidence. It is a preference, it is
  legitimate as a preference, and it should be recorded as one — which is the
  argument for moving it out of `AGENTS.md`.

## 5. What changes

1. **Move the §9 style paragraphs out of `AGENTS.md`.** They are a preference
   about output, not a rule about the codebase. A pointer stays; the rest goes to
   auto-memory, which is loaded once rather than per turn. *(§1a)*
2. **Raise `0004` P2.1 from a 30-line trim to a real reduction**, with a target
   and a rule for what belongs: `AGENTS.md` holds what an agent must know
   *before* it can safely act. Status goes to `docs/STATE.md`; anything a check
   enforces gets a pointer, not a restatement. *(§1b)*
3. **Give `AGENTS.md` §6 the arid-code carve-out.** Mutation testing is required
   on changed *behaviour*, not changed lines. *(§2)*
4. **Add the enforcement ladder** — impossible, then checked, then tested, then
   written — as the rule for choosing, and its corollary that a level-1
   guarantee must not be re-tested at level 3. *(§2)*
5. **Close the `Observed<T>` gap or downgrade the claim.** Either the watermark
   cannot be unwrapped, or `AGENTS.md` stops saying it cannot. *(§2)*
6. **Adopt the 10% effective-false-positive bar for `repo-conformance`.** A check
   that fires on something a reasonable change would do gets deleted, not
   tuned. *(§2)*

Items 3, 4 and 6 are the ones Josh asked for by name: they exist to stop work
that proves what is already true.

## Sources

- Gloaguen, Mündler, Müller, Raychev, Vechev. *Evaluating `AGENTS.md`: Are Repository-Level Context Files Helpful for Coding Agents?* MemAgents @ ICLR 2026. <https://www.sri.inf.ethz.ch/publications/gloaguen2026agentsmd>
- Petrović, Ivanković. *State of Mutation Testing at Google*. ICSE-SEIP 2018. <https://research.google/pubs/state-of-mutation-testing-at-google/>
- Petrović, Ivanković, Fraser, Just. *Practical Mutation Testing at Scale: A View from Google*. TSE 2021. <https://arxiv.org/pdf/2102.11378>
- Sadowski, Aftandilian, Eagle, Miller-Cushon, Jaspan. *Lessons from Building Static Analysis Tools at Google*. CACM 61(4), 2018. <https://cacm.acm.org/research/lessons-from-building-static-analysis-tools-at-google/>
- Wen, Nagy, Bavota, Lanza. *A Large-Scale Empirical Study on Code-Comment Inconsistencies*. ICPC 2019. <https://www.inf.usi.ch/lanza/Downloads/Wen2019a.pdf>
- DORA. *State of AI-assisted Software Development*, 2025. <https://dora.dev/dora-report-2025/>
- King. *Parse, Don't Validate*, 2019. Minsky. *Make Illegal States Unrepresentable*, 2011.
