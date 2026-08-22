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
