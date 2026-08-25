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

---

## 6. A constant that grew a member the callers could not handle

**Found:** 2026-08-23, on the deployed instance, by running the CLI.

Adding an `Outcomes` variant to `Table` and putting it in `Table::ALL` broke
every caller that iterated `ALL` and called `Reader::read` on each entry.
Outcomes are measurements, not chain events — no signature, no transaction
position, and `measured_at` where events have `slot` — so the read failed with
`stored file has no \`slot\` column`.

**Cost:** the `creators` command was broken on the deployed instance for about
twenty minutes. No data was lost or corrupted; the store is append-only and the
failure was on a read path. The unit tests all passed, because none of them
iterated `ALL` and read.

**What catches a recurrence:** `Table::EVENT_TABLES` now exists alongside `ALL`,
`Reader::read` refuses a measurement table by name rather than failing on a
missing column, and two tests hold both halves — one asserting the constants
differ by exactly the outcomes table, one asserting every table in
`EVENT_TABLES` is actually readable so the list cannot drift the other way.

**The general shape:** a constant named `ALL` invites `for x in ALL`, and every
member has to survive whatever that loop does. Widening one is an API change to
every loop over it, and the compiler cannot see it. The fix was not a better
comment; it was a second constant that means "the ones this loop can handle".


---

## 7. A filter that was right for one instruction and inverted for another

**Found:** 2026-08-24, by trying to reproduce a finding rather than accepting it.

`resolve_mint` requires exactly one non-quote mint per transaction and skips the
row otherwise. Correct for a launch. A pump.fun migration moves the subject token
*and* the LP mint the pool creates in the same transaction, so it always had two
candidates and was always skipped. Measured over one hour: of 66 migrations, 49
had two non-quote mints, 17 had none, and **none had exactly one**.

**Cost:** the store held 4 graduations where roughly 1,480 had happened — a 0.3%
capture rate — for a day. `creator_edge` gates on the graduation rate, so the
strategy could never propose anything, and the reason looked like a market fact.

**The expensive part was not the missing data, it was the conclusion drawn from
it.** The four rows that survived all completed within three slots, which was
written up as "graduation is inherently instant, so the creator rule selects for
bundlers", and the population rate was reported as 0.0133% — argued to be 60×
rarer than the plan assumed. The true rate is about 3%, three times *higher*
than assumed. The surviving sample was biased by the same bug that suppressed
it: a degenerate migration is the one most likely to leave a single resolvable
mint. A broken filter does not produce less data, it produces a **selected**
sample, and a selected sample supports confident conclusions about the selection.

**What catches a recurrence:** the resolution is now keyed on the instruction,
with a test asserting a launch still resolves to the mint it created — the
inverted rule applied everywhere would have destroyed the working half, since 300
of 303 mints in sampled launch transactions are minted in the transaction itself.
A graduation whose subject cannot be determined is counted under its own skip
reason rather than beside a million skipped trades, so the gap is visible at the
rate it actually matters.

**The general shape:** the population that survives a filter is evidence about
the filter before it is evidence about the world. This one was found only by
asking the chain what the answer should be and comparing — the store alone was
internally consistent and entirely wrong.

**Two further errors were found the same way**, by extracting an hour and reading
the rows rather than trusting the count: 35 of 97 migration instructions were in
*failed* transactions, and the 62 successful rows covered only 50 distinct mints
because a token can carry both a `migrate` and a `migrate_v2`. Each would have
overstated the rarest label in the store — by a third and by 24% respectively.
Neither was in the original diagnosis. Running the thing produced three
corrections that reasoning about it did not.

---

## 8. A daemon that exited on a transient upstream error

**Found:** 2026-08-24, by regrounding rather than by an alarm — which is the
part worth recording.

The follow recorder halved a window past the endpoint's row cap, got a bare
`HTTP 500`, and the error propagated out of the loop via `?`. The process
exited. Nothing restarted it, nothing alarmed, and it stayed down.

The trigger was real and rare: a two-minute burst of **7,233 failed `migrate`
transactions** — spam against the old migration path, every one of them
reverted. Normal migration rate in that window is about one a minute, so this
was roughly a 3,600× spike, and it pushed one window over a cap that a hundred
thousand ordinary windows had cleared.

**Cost:** hours of chain never recorded. The exact hole is knowable — the cursor
did not advance — so it is recoverable, but only because someone looked.

**What makes it bad rather than unlucky** is the shape of the damage. A missing
slot range in this store is indistinguishable from a quiet market, which is the
failure this entire project is organised against, and the recorder created one
by failing in the way that leaves the least evidence.

**What catches a recurrence:** follow mode no longer exits on a query error. It
retries with a doubling backoff capped at five minutes, and **the cursor stays
put**, so a stall is visible in the log and nothing is skipped. Skipping would
have been the easier fix and the wrong one: a stall is recoverable and a gap is
not. `retry_backoff` is a pure function with tests, because the loop it lives in
cannot be tested.

**Two things went right and both were accidents of an earlier decision.** The
`succeeded` filter added a day earlier — for the unrelated reason that 35 of 97
migrations in a sampled hour had failed — meant all 1,486 spam events the
recorder did capture were excluded from every graduation count. The population
rate moved from 3.2064% to 3.2089% across a 7,233-event spam burst. And the
one-off backfill path still fails loudly, which is correct: an operator is
watching that one.

**The general shape:** a check that is right for a foreground tool is wrong for
a daemon. `?` on an error is a decision to stop, and stopping is only safe when
something is watching. Nothing was.

**The first fix was not enough, and the deploy proved it.** Retrying kept the
process alive, and it then stalled for twenty minutes on a second, larger burst
— 10,674 migrations across three minutes — retrying the same five-minute window
and failing the same way each time. Retrying an operation unchanged is only
useful when the failure was transient; this one was a property of the window's
size. A five-minute window in a burst is thousands of rows, which `fetch_window`
has to split into dozens of sub-queries that must *all* succeed together, and one
HTTP 500 near the end discards every one that worked.

So the window now **shrinks on failure** and recovers on success, which turns
all-or-nothing into incremental progress. Verified against the exact window that
stalled in production: it crawled the burst and then advanced two hours of chain
in seven minutes.

Worth noting what did *not* fix it. The obvious suspect was the extraction
query, which had been changed the day before to group every token transfer in the
window rather than only the non-quote ones. Filtering it back down to the
pump.fun transactions measured **slower** — 6.1s against 3.3s, because the
subquery re-scans — so the tempting fix was the wrong one, and only measuring
said so.

---

## 9. An invariant documented as stronger than its enforcement

**Found:** 2026-08-24, while auditing the foundation rather than by a failure.

`AGENTS.md` rule 3 said a value observed after the watermark "cannot be
unwrapped — not 'should not', cannot", naming [`AsOf::accept`] and `Observed<T>`
as the reason. `Observed<T>` is used **nowhere outside its own crate**. The real
enforcement is four call sites comparing slots through `AsOf::admits`.

**Cost:** none yet, and that is why it is worth recording. The code was correct;
the claim about *why* it was correct was not. An invariant defended by "the type
system makes this impossible" is audited very differently from one defended by
"four functions each remember to check", and the second needs tests where the
first does not.

**What is actually true**, now written down: a **scan filters** and a **single
observation is refused**, and those are deliberately different. A partition file
holds slots either side of a watermark, so a read must drop rows as it goes —
erroring would make a normal query fail. `accept` is the right shape for one
value arriving from outside the store, and nothing needs it today.

**What catches a recurrence:** `radar-store/tests/watermark_holds.rs`, which
reads across a file that straddles the watermark and sweeps every boundary.

**The part worth keeping** is how the tests were checked. All four passed on the
first run, which proves nothing — so the filter was deleted and they were run
again. Two failed. **The outcomes test still passed**, because its fixture
spread measurements across partitions and the whole-file skip did all the
filtering, so the per-row path it claimed to test was never reached. The fixture
now sits inside one partition and fails when the filter is removed.

A test written against correct code cannot tell you whether it tests anything.
Only breaking the code can, and it took ninety seconds to find that one of these
four was decorative.

---

## 10. A hardcoded probe size that made the answer a constant

**Found:** 2026-08-25, during an adversarial review, by running the decision lane
against production and asking why the number was round.

`radar consider` reported `0 proposal(s) raised` over 41,254 candidates, with
`CapacityBelowFloor` on 20 of the 25 candidates that survived every free filter.
Read as a market observation that is a finding about liquidity. It is not a
market observation. It is a constant.

`consider.rs` called `radar_sim::probe(quoter, mint, structure, 1_000_000_000)`.
That fourth argument is an intended position in **raw token units**, hardcoded,
used for every token regardless of supply, decimals or price. Probe multiples are
`[1, 2, 5]`, so the largest size ever quoted was 5e9 raw units.

A pump.fun token has six decimals and a supply around 1–2e15 raw units. So the
probe asked what it could get for roughly **0.00005% of the supply**. Measured
live against a token the run had just refused: 1e9 raw units quoted at **4,000
lamports** — about $0.0004 — at 0.59 bps of impact. A thousand times more moved
the price less than one basis point, so the token had ample depth and the probe
never went near it.

Carried through: capacity ≤ ~20,000 lamports ≈ $0.002; `capacity_share_bps` of
2,000 takes a fifth, giving a notional of ~$0.0004 against a `min_notional` of
$1.00. **The maximum notional the pipeline could ever propose was 2,457× below the
floor it had to clear.** No token, in any market, at any time, could have produced
a proposal.

**Cost:** none in money, because the lane is shut. The cost was in belief. Two
claims rested on this: that the trading lane was "built and tested end to end",
and that `Policy::CLOSED` was what stood between Radar and a trade. Neither was
true — the lane was shut four stages upstream of the policy, and the policy had
never been handed a proposal to refuse.

**The general shape, for the third time:** a broken instrument does not produce
less data, it produces a **selected** sample, and a selected sample supports
confident conclusions about the selection. Entry 7 was a filter that selected
degenerate migrations and got written up as "graduation is inherently instant".
This one selected nothing at all and read as "these tokens are too thin to trade".
Both times the store was internally consistent and entirely wrong, and both times
the only thing that found it was asking the outside world what the answer should
be and comparing.

**What made it invisible** is worth recording separately.
`radar-strategy/tests/pipeline.rs` exists precisely to catch this class of bug —
its own doc comment says *"a strategy whose proposals the kernel always refuses is
a broken pipeline that both halves' tests call green"*. It missed it, because its
fixture supplies a synthetic `ExitReport` with `out_lamports` in the hundreds of
millions while the real probe returns four thousand. The test written to catch the
bug was immunised against it by a fixture five orders of magnitude away from
anything the system can measure. A fixture that cannot be produced by the code
under test is a fixture that tests a different system.

**What catches a recurrence: nothing yet, and this entry is written before the
fix rather than after it.** The probe still takes a caller-chosen size. The
planned fix removes the constant entirely — search for the size at which impact
reaches the budget, denominated in SOL, reading the token's decimals rather than
assuming them — and re-points `pipeline.rs`'s fixture at a curve the real probe
can produce. Until both land, this entry is the only thing standing between Radar
and the same silence.

The guard that would have caught it earliest is not a test but a habit: **a funnel
that reports zero is reporting about itself until something has passed through it
at least once.** `0 proposals` becomes a claim about the market only after a
proposal has been observed. Worth applying to every counter in the system, not
just this one.
