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

---

## 4. Two `u64`s that swap meaning between instruction variants

**Found:** 2026-08-22, deriving pump.fun argument layouts.

Every pump.fun trade instruction takes two `u64`s, and which one holds lamports
depends on the variant. `buy` is `(token_amount, max_sol_cost)`; `buy_exact_sol_in`
is `(sol_amount, min_token_output)`. Same shape, same size, opposite meaning.

**Cost:** would have been catastrophic and invisible. The median `buy_v2` token
field is 3.6 trillion; read as lamports that is 3,614 SOL against a true spend of
about 0.007. Every position size, execution cost and P&L figure would have been
wrong by six orders of magnitude, and none of them would have looked broken --
memecoin numbers are large and unintuitive, so nothing would have flagged it.

**What catches a recurrence:** the unit now lives in the type
([`Amount::Tokens`] / [`Amount::Lamports`]), so a token quantity cannot be read
as lamports at all. `tests/payload_layouts_hold.rs` runs 1,757 real payloads
through the decoder and asserts the layout is distinguishable from its opposite.

**The part worth remembering** is how long that test took to get right. The first
version asserted "our SOL field holds plausible values" and passed trivially --
it only confirmed the implementation. The second asserted the swapped reading
would be absurd, and failed at 45%, because token and lamport magnitudes overlap
more than expected. What finally settled it was structural rather than
statistical: 146 of 250 sells set their minimum output to *exactly zero*, and a
token amount is never exactly zero, because a trade for nothing is not a trade.
A field that is often a sentinel is a bound; a field that never is, is an amount.

A test that cannot fail against the plausible wrong answer is not evidence.

---

## 5. Reading an empty result as a pass

**Found:** 2026-08-22, immediately after pushing a broken build.

The pre-commit check pipes `cargo test` through `grep "^test result"` and sums
the counts. When the build fails there are no result lines, so the sum prints as
empty — and an empty string next to a zero clippy count reads, at a glance, like
everything passed. It did not; `radar-cli` did not compile, and the commit went
out anyway.

**Cost:** one broken commit on `main`, fixed in the next. CI would have caught it
within a minute, so the damage was bounded — but only by a check that runs after
the push rather than before it.

**What catches a recurrence:** nothing automatic yet. The check should assert a
minimum test count rather than print a sum, so that "no tests ran" is loud
instead of blank. Until that exists this entry is the only thing standing between
the same silence and the same conclusion.

**The general shape** is worth more than the instance: a check that reports
*absence* the same way it reports *success* is not a check. This is the same
failure as an empty CryptoHouse result standing in for a timeout, which the
backfill guards against explicitly — and it was made here anyway, one layer up,
in the tooling rather than the code.

