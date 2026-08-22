<!-- SPDX-License-Identifier: Apache-2.0 -->
# LEARNINGS

Traps that cost real time, and claims that shipped stronger than the code behind
them. Each entry names the check that would catch a recurrence, or says plainly
that nothing does.

The standard this list defends: every claim in this repository is backed by
something that runs. Under-claiming costs nothing; over-claiming costs the
benefit of the doubt on everything else.

---

## 1. A design documented as canonical, with its source lost

**Found:** 2026-08-22, while evaluating whether to reuse a sibling repo.

That repo documents a cache-pricing layer as canonical, citing functions in
`src/core/credits.ts`. The file is not in the working tree and `git ls-files`
does not list it under any path. The functions exist only in gitignored `dist/`
build output from March and April, and the two surviving artifacts already
disagree with each other about a constant.

**Cost:** an initial reading of this concluded the layer was "designed but never
built", which was wrong in a way that mattered — it was built, and the source is
gone. Correcting it required going back through the evidence.

**What catches a recurrence here:** nothing yet. A conformance test asserting
that every file path named in the docs exists and is tracked is the obvious
answer, and is not written. Until it is, this entry is the only thing standing
between Radar and the same failure.

---

## 2. A test that asserted an implementation detail it had guessed

**Found:** 2026-08-22, first run of the `radar-types` suite.

A test asserted that base58-decoding `"abc"` yields exactly 2 bytes. It yields 3.
The code was right and the assertion was invented.

**Cost:** minutes, because it failed immediately. It is recorded because the
failure mode generalises: an assertion on a number nobody verified is a test of
the author's memory, not of the code. The test now asserts the property that
actually matters — that a short input is rejected rather than silently padded —
and leaves the exact length to the library.

**What catches a recurrence:** the test suite, but only because this one was
wrong in a direction that failed. An invented assertion that happens to pass is
invisible.

---

## 3. An exact-match on a name that had been versioned

**Found:** 2026-08-22, probing pump.fun launches on mainnet.

A probe tested for the dev buy with `"Buy" in instruction_names` and reported
`False` on a launch whose instruction list plainly contained `BuyV2`. The dev buy
inside the create transaction is the single most load-bearing piece of Tier-A
coordination evidence, and the check said it was absent when it was present.

**Cost:** would have been a coordination detector calibrated on a base rate of
roughly zero for its strongest signal. Caught within minutes only because the
probe printed the raw instruction list next to its verdict.

**What catches a recurrence:** printing the evidence beside the conclusion, which
is now the house style for probes. Structurally, `radar-decode` must match on
discriminator bytes rather than log-line names, and must record an unrecognised
discriminator as unknown rather than guessing — a decoder that silently stops
understanding a program looks exactly like a program that has gone quiet.

