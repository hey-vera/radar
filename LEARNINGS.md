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

**What catches a recurrence here:** `repo-conformance`'s
`every_file_path_named_in_the_documentation_exists_and_is_tracked`, written on
2026-08-25 — the check this entry asked for on the day it was opened.

It found its own founding case on its first run. `docs/research/0003` cited four
files by path that are in the sibling repository or nowhere, written exactly the
way the lost `src/core/credits.ts` was: in backticks, with no sign to a reader
that they were somewhere else. They now carry a `claw-net:` prefix, and a path
qualified with a repository or rooted at `/` is understood as a claim about
another machine rather than about this tree.

Paths resolve by suffix, because prose names a file the way a reader would follow
it. A check that flagged correct prose would be a check somebody turned off.

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

**What catches a recurrence:** `discover_capacity` removes the constant entirely
— it searches for the size at which impact reaches the budget, scaled to the
token's own supply rather than to a number somebody picked. `the_search_is_scale_free`
holds the property the old code violated, and a mutant restoring the fixed 1e9
rung fails it. `pipeline.rs`'s fixture is now produced by running the real search
instead of hand-written, so it cannot drift out of reach again.

Fixing this was **not enough on its own** — the funnel still raised zero
proposals afterwards, for a second, unrelated reason. See entry 11, which was
found only by re-running rather than declaring this done.

The guard that would have caught it earliest is not a test but a habit: **a funnel
that reports zero is reporting about itself until something has passed through it
at least once.** `0 proposals` becomes a claim about the market only after a
proposal has been observed. Worth applying to every counter in the system, not
just this one.

---

## 11. A vendor's derived field, trusted as a measurement

**Found:** 2026-08-25, by fixing entry 10 and re-running the funnel anyway.

Fixing the hardcoded probe size did not change the answer. `radar consider` still
raised **zero proposals** against the live store, still on `CapacityBelowFloor`.
The probe fix was necessary and not sufficient, and only re-running said so.

`impact_to_bps` reads Jupiter's `priceImpactPct`. For pump.fun bonding-curve
routes **that field does not vary with size**. Measured on `Q5QRogEuf…pump`:

| size (base units) | out (lamports) | lamports/unit | `priceImpactPct` |
|---|---|---|---|
| 1e8  | 2,759 | 0.00002759 | 0.0391 |
| 1e12 | 27,583,797 | 0.00002758 | 0.0393 |
| 1e13 | 273,545,706 | 0.00002735 | 0.0473 |
| 1e14 | 2,525,575,447 | 0.00002526 | 0.1203 |

It moves from 0.039 to 0.048 across a hundredfold increase in size. It is a fee
or a spread, not impact, and it carries no information about depth. Read as a
fraction it is ~395 bps at every size — permanently past the 100 bps budget — so
**`capacity_lamports` returned `None` for every token in the universe**, which
falls through to `CapacityBelowFloor`. That was the real mechanism behind zero
proposals; the probe size was only the first of two.

The realised price says what the vendor's derived field does not. Priced against
the dust quote, the same token shows 2 bps of real impact at 1e12, **85 bps at
1e13**, 846 at 1e14 and 2,180 at 3e14. That is a depth curve, it was recoverable
from quotes already being fetched, and it puts real capacity at $31 — a $6.27
notional against a $1.00 floor.

**Cost:** none in money. The cost was another day of `0 proposals` reading as a
market fact, on top of entry 10, from a different cause with identical symptoms.

**The general shape:** this is [ADR 0001](docs/adr/0001-decode-locally-never-buy-parsed-transactions.md)
one level up. That record says never buy parsed transactions, because decoding is
where a vendor charges fifty times the raw material price. The same argument
applies to *derived* fields: `outAmount` is raw material and `priceImpactPct` is a
derivation, and the derivation is the step the vendor gets wrong. Radar already
owned decoding. It had not noticed it was buying arithmetic.

**What made it survive:** the test fixture had the same defect as the vendor. Its
`Quoter` returned a constant price per unit while reporting a size-varying
`impact_bps` — a pool whose stated impact contradicted its own returns, which is
precisely Jupiter's behaviour inverted. A fixture cannot detect a bug it shares.

**What catches a recurrence:** `discover_capacity` derives impact from the
realised price and never reads the router's field. The test pools are now real
constant-product curves that **deliberately report a constant, wrong
`impact_bps`**, with a compile-time assertion that the lie exceeds the budget — so
a search that trusted the field would find no capacity and fail. Both halves were
mutation-checked: reverting to the router's number, and forcing realised impact
to zero, each fail. `the_measured_pumpfun_curve_reproduces` pins the live numbers
above as a unit test.

**The habit worth keeping:** entry 10's guard said a funnel reporting zero is
reporting about itself until something has passed through it. That is what
prompted re-running after the first fix instead of declaring it done, and it is
the only reason this was found.

---

## 12. An aggregate that included an event that was not a trade

**Found:** 2026-08-25, within an hour of shipping the price path, by using the
data rather than by testing the code.

The first study over the new prices reported a median maximum favourable
excursion of **7,054%** and a 90th percentile of **1,081,162%** across 188
tokens. Nothing in the pipeline objected. Every test passed, the query ran, the
numbers had the right type and the right scale, and they were nonsense.

`solana.token_transfers` carries a `MintTo` row at launch holding the **entire
supply** — 1e15 base units against whatever SOL moved in the creation
transaction, which is 44x the average real fill on the same token. It is also the
*earliest* row for the mint, so `argMin(price, ts)` selected it as
`first_price`, and every excursion in the study was measured against the supply
mint instead of against a trade.

Filtered to `Transfer` and `TransferChecked`, the same cohort reports a median
MFE of 23.7% and a median held-to-end of **-13.4%** — consistent with the base
rate `AGENTS.md` opens with, where the first set was consistent with nothing.

**Cost:** one contaminated outcomes file in production, caught before a second.
It had to be quarantined by hand rather than left to age out, because
`prior_prices` folds each measurement onto the last one, so a wrong `first_price`
would have propagated into every later measurement of the same token
indefinitely. A fold is a ratchet, and a ratchet turns a transient bad row into a
permanent one.

**What made it survive the tests:** every test used a stub quoter or a
hand-written row. Not one of them fetched a real `token_transfers` page, and the
`MintTo` row does not exist in any fixture — because nobody who had not seen it
would think to write one. The unit tests were testing arithmetic on numbers that
were already wrong upstream.

**What catches a recurrence:** the query asserts its own filter
(`the_query_counts_only_transfers_between_accounts`), which is a structural test
rather than a numerical one — the same shape as the signature-pushdown assertion
beside it, and for the same reason: neither changes a value in a way a fixture
would notice.

**The general shape, and it is the sharpest version yet of entries 7 and 10:**
**a wrong number looks like a number, where a missing one looks like a gap.** The
graduation bug (7) produced too *few* rows and was found because a count was
implausible. The probe bug (10) produced a *constant* and was found because zero
proposals was implausible. This one produced a full distribution of the right
shape and magnitude-of-digits, and the only thing that gave it away was a human
reading 1,081,162% and knowing markets do not do that.

Which means the guard cannot be a test. It has to be the habit of computing a
figure whose plausible range is known in advance, and checking against it — the
same move that caught the lamport rescaling in the same module a day earlier, by
comparing a derived price against an independent quote.

---

## 13. Documentation that asserted a state instead of a way to check it

**Found:** 2026-08-25, during an adversarial review, by asking the box what it
was running rather than reading the file that says.

`deploy/README.md` carried a section headed *"What is actually running right now
(2026-08-25)"*. It said `radar-serve` was **running, but not as a service** — a
hand-started process reparented to init in an SSH login-session scope — and that
*First install* step 2 "has never been run on this host and still needs to be".
Further down, above the restart step, the deploy loop said **"This fails today
with `Unit radar-serve.service not found`"**.

Every word was true when written, earlier the same day. It was written *because*
the rest of the file described a deployment that did not match the box, and
correcting it was right. Then the unit was installed and the correction outlived
the thing it corrected:

```
ControlGroup=/system.slice/radar-serve.service
Restart=always   MemoryMax=805306368   ProtectSystem=strict   enabled
```

**Cost:** none yet, and the direction is the safe one — it reports a *missing*
safeguard that is present, rather than a present one that is missing. The
plausible damage was an operator re-running an install step that had already
been run (harmless), or reading "this fails today" directly above the deploy
procedure and concluding the deploy path was broken when it works.

**What made it invisible:** `repo-conformance` enforces that every file path
named in the documentation exists and is tracked — the check LEARNINGS 1 asked
for, which found its own founding case. It cannot help here, and no amount of
extending it would. **A claim about another machine has no path to resolve.**

**The same review found a second instance, which is why this is an entry rather
than a fix.** `.github/required-checks.txt` opened *"The required status checks
on the `main` ruleset"* and listed seven, singling out `mutants` as the one "NOT
YET IN THE RULESET". That reads as six enforced checks and one gap. It was seven
gaps:

```
gh api repos/hey-vera/radar/rules/branches/main             -> []
gh api repos/hey-vera/radar/rulesets?includes_parents=true  -> []
gh api repos/hey-vera/radar/branches/main/protection        -> 404 Branch not protected
```

There is no ruleset and never was. Every check runs on every pull request and
reports; **none of them can block a merge, and nothing prevents a direct push to
`main`.** The file describing the enforcement was the only evidence the
enforcement existed, and it was written by the same hand that would have created
it. That is the shape to watch for: a document that asserts a control, standing
in for the control.

**What catches a recurrence:** not a check — a change of shape, and it is worth
being explicit that no mechanical guard was added.

The obvious one would assert that host claims carry a recent date. It would fail
CI with no code change, on a clock, which makes it a check somebody turns off —
the same reasoning entry 1 gives for why path resolution matches by suffix ("a
check that flagged correct prose would be a check somebody turned off"). A
decorative check here would be worse than none, because it would look like this
entry had been closed.

Instead the section is now *Verifying the deployment*, and it gives the
**command rather than the answer**: `systemctl list-unit-files "radar*"`,
`systemctl show radar-serve -p ControlGroup …`, with the expected output beside
each and a note that a `ControlGroup` under `/user.slice/` is the specific
failure being looked for. A reader can now tell "this is what it was" from "this
is what it is" in one line.

**It went wrong in the other direction the same day, which is the part that
settles it.** The ruleset was created hours after this entry was written, and
`.github/required-checks.txt` — rewritten above to say plainly that *nothing*
was required — became false again immediately. A file that has been wrong in
both directions within a day is not a file that needed a more careful answer. It
needed to stop giving an answer:

```
gh api repos/hey-vera/radar/rules/branches/main --jq '[.[].type]'
```

The enforcement was also confirmed in the direction that fails, because a
control verified only where it passes is not verified. A direct push to `main`
is now refused with *"Changes must be made through a pull request"* and *"8 of 8
required status checks are expected"*.

**The general shape:** documentation asserts two kinds of thing, and only one of
them is checkable from inside the repository. A claim about the code is
checkable and should be checked. **A claim about a remote host is a
measurement, and a measurement written down as prose is stale the moment
anything changes** — including, as here, when someone fixes the very problem it
reports. Write the query and the expected result instead. A date in the heading
lets a reader discount the claim; the command lets them replace it.

The uncomfortable half: documentation in this project has now outlived what it
described three times, and the first (entry 1, a design documented as
canonical with its source lost) produced a check that genuinely works. The
lesson is not "write another check". It is that the two failures are different
in kind, and only one of them has a mechanical fix.

---

## 14. A fix for one instance of a problem, mistaken for a fix for the problem

**Found:** 2026-08-25, by picking the token with the largest recorded excursion
and asking the chain which transaction produced it.

Entry 12 found that `token_transfers` carries a `MintTo` row at launch holding
the entire supply, and that it was becoming `first_price`. The fix filtered
`transfer_type` to `Transfer` and `TransferChecked`, the numbers became
plausible, and the entry was closed.

The numbers were plausible and still wrong. `MintTo` was an *instance*. The
problem was that **`lam` — the largest SOL balance change in the transaction —
is not necessarily the counterparty of the tokens that moved in it.** A supply
mint is one way for those two to be unrelated. A dust transfer beside incidental
SOL is another, and it survived the filter because a dust transfer is a
`Transfer`.

The store's largest recorded excursion was **56,632,867%**. The transaction
behind it moved **one base unit** of the token against 3,935,002 lamports:

| | tokens moved | lamports | price |
|---|---|---|---|
| the peak-price transaction | 1 | 3,935,002 | 3,935,002 |
| an ordinary fill on the same token | ~5e11 | ~2e6 | ~0.5 |

**What made it survive a second time** is worth more than the instance.
`peak_price` is `max(lam/tok)` and `trough_price` is `min`. Those are the two
functions guaranteed to find the contamination: **an aggregate over extremes
selects its own worst errors, by construction.** Meanwhile `vwap` is
size-weighted and looked sane, and `first` and `last` are chosen by *timestamp*
rather than by price — so the one figure anybody quoted, research 0009's median
held-to-end of −13.4%, was very nearly right. Across three mints the ratio of
maximum price to median ran from 765,000x to 8 billion x while the 99th
percentile sat at 4.5x to 44x. The distribution was fine and only its edges were
destroyed, which is the hardest version of entry 12's shape: not a wrong number
that looks like a number, but a *mostly right* set of numbers with two wrong
ones inside it.

0009 published a 90th-percentile MFE of **2,306,382 bps** and nothing objected,
including the review that merged it.

**What catches a recurrence:** the floor is a share of the mint's own median
trade, applied to **both** sides. One side is not enough — guarding only tokens
leaves `min(lam/tok)` picking a large transfer with trivial SOL, which is how
MAE was reporting −100.00%. Relative rather than absolute, because a floor in
base units depends on the token's decimals and supply and would be right for the
token it was chosen on.

---

### The second bug, found only by not shrugging at four rows

Verifying the fix meant running the query the Rust *generates* beside the
hand-written one that had actually been measured — because validating a
prototype and then shipping different code is its own failure. Four of forty
mints differed. Identical fill counts, identical peak, identical trough; only
`first_price` moved.

That should not have been possible, so it was worth a second look rather than a
shrug about live data. `ts` is `min(block_timestamp)` for the transaction and
many fills land in the same block, so ties are ordinary rather than rare.
`argMin(lam / tok, ts)` picks an **arbitrary** row among tied keys, and does not
pick the same one twice.

One unchanged query, three runs, one closed window, forty launches:

```
11 of the 40 returned a different first_price each time, one over a 3.3x range
```

`first_price` is the denominator of MFE, MAE and held-to-end. **Every return
figure this system produces was varying by up to a factor of three between
identical runs over identical data.**

That is not precision, it is the replay guarantee. `AGENTS.md` rule 2 exists so
a recorded verdict can be re-derived and compared against its recording, and a
measurement that disagrees with itself cannot be re-derived at all — a replay
mismatch would have been indistinguishable from a leak. Ordering by `(ts, sig)`
makes the choice total, verified the way it was found: three identical runs,
zero differences.

**What catches a recurrence:** a structural assertion that the ordering key is
the pair and specifically *not* `ts` alone. It cannot be numerical — a fixture
cannot contain a tie whose resolution is arbitrary, because the arbitrariness
lives in the server.

---

**The general shape, and it is three things:**

1. **A fix aimed at an instance leaves the class.** Entry 12's filter was
   correct and complete for `MintTo` and silent about every other way two
   quantities in one transaction can be unrelated. Ask what the instance was an
   instance *of*, and fix that.
2. **An aggregate over extremes selects its own worst errors.** `max` and `min`
   are not neutral summaries of a distribution; they are searches for the tail,
   and a contaminated tail is what they will find. A median hides the same
   corruption completely, which is why the headline figure stayed right while
   everything beside it was nonsense.
3. **Four rows out of forty is a finding.** The temptation was to attribute it
   to live data and move on — the first comparison genuinely *was* confounded by
   an open window, which made the excuse available and plausible. Closing the
   window and re-running is what turned a dismissable discrepancy into the more
   serious of the two bugs.

**Fixing the query repairs nothing already recorded.** The fold is a ratchet —
`peak` combines with `max`, `trough` with `min` — so a contaminated extreme
survives every later correct measurement, which is entry 12's ratchet
observation in a different field. `--reprice` replaces a recorded path instead
of extending it, and reaches a token only while that token is still due for a
checkpoint. It repairs what is in flight rather than the whole store, and says
so, because a repair that looked complete and was not would be worse than none.

---

## 15. A binary deployed ahead of the configuration it needed

**2026-08-27.** The Radar interface — Vite, React, embedded in the binary with
`rust-embed` — was installed on the box and the site went **blank**. Not an
error page, not a partial render: a white document with a `<div id="root">` that
never filled.

The binary was fine. The bundle was fine. The live Caddyfile still carried the
CSP written for the previous, server-rendered ops page:

```
Content-Security-Policy "default-src 'none'; style-src 'unsafe-inline'"
```

`default-src 'none'` blocks a `<script src>`. Every asset the new interface
needed was refused by the browser before it ran, and the only evidence was in a
console nobody was looking at.

**Both halves were correct in isolation.** The new CSP was written, reviewed and
committed in the same change as the interface. It was in `deploy/`, in git, and
in the pull request. It was not on the box, because installing a binary and
reloading Caddy are two commands and only one of them was run.

**The shape:** a change that spans a binary and its environment is one deploy
unit, and nothing in the process said so. `sha256sum` discipline covers the
artifact — and the artifact was correct. The runbook's verification step read
the health endpoint, which is served by the same binary and answered `200`
throughout.

**What catches a recurrence:** stating the ordering in the runbook, at the top,
ahead of the install commands — and, where it can be done, making the binary
*refuse to start* without the configuration rather than starting into a broken
state. The Cloudflare Access work that followed is built that way on purpose:
`radar-serve` will not start unless the environment says who may look, because
the alternative is a server that comes up and serves the wrong thing while every
health check reports fine.

That is the difference worth keeping. A missing CSP produced a blank page and
took minutes to notice. A missing access check produces a working page served to
everybody, and would not have been noticed at all.

---

## 16. A security property argued in a comment, and then actually run

**2026-08-27.** Not a failure — a note about what verification is worth, written
because the alternative was available and tempting.

The chat feature's central claim is that a language model reading
attacker-controlled token names cannot reach anything. Three layers hold it up:
the model has no action tools, its output is never parsed into an action, and the
interface renders a reply as text.

The third layer was, at the point of writing, a comment saying
`dangerouslySetInnerHTML` was not used. That is a true statement about the source
and it is not evidence. So the stand-in CLI was pointed at a script that returned
this as the model's answer:

```
<img src=x onerror="window.__PWNED=1"><script>window.__PWNED=1</script> BUY THIS TOKEN NOW
```

and the page was driven in a browser:

```
{"pwned": null, "imgs": 0, "scripts": 1, "rendered": true}
```

No script ran, the `<img onerror>` never became an element, the one `<script>`
is the application's own bundle, and the literal text `<script>` appears on the
page as text. The claim is now a measurement.

**The same shape applied to the monitor.** `radar brief` gained an `agent` check,
and a check is only worth having if it goes red. It was run twice against a live
server: once with the provider working (`[ok] agent codex answering`, exit 0) and
once with the CLI made to exit non-zero the way a lapsed credential does
(`[FAIL] agent codex refused the last call`, exit 1).

**And it caught something the code review had not.** In the failing run the CLI
printed `Error: not authenticated. Run codex login.` to stderr, and the HTTP
response carried only `the CLI exited with exit code: 1`. That redaction was
designed deliberately, but until the failing path was actually executed, nobody
had seen it work.

**The general shape:** the security properties in this repository are cheap to
assert and cheap to test, and the gap between the two is where entries 1, 9 and
13 live. If a claim is worth a paragraph of comment, it is worth the twenty
minutes of running the attack it describes.

---

## 17. A schema change that would have made the history unreadable

**2026-08-30.** Caught before it shipped, and only because the deploy was being
thought about rather than performed.

`authority_prevalence` was added to the decisions table. The reader read it with
`str_col`, which **errors** when a column is absent:

```rust
let prevalence = str_col(&batch, "authority_prevalence")?;
```

Production held roughly nine hundred decision rows written before that column
existed. None of them have it. So the first read of any of them would have
returned `MissingColumn`, and every caller that touches decisions — `radar
selection`, the decisions check in `radar brief`, `/v1/funnel`, and therefore the
entire interface — would have failed at once, on a store that was perfectly fine.

**The codebase already contained the warning, in the exact words, twelve lines
away from the mistake.** `optional_u64_col`'s doc comment reads:

> a column the writer has always emitted going missing is a corrupted file and
> should fail loudly, while a column added later is simply absent from older
> files and must read as "not measured". **Using the erroring form for a new
> column would make one schema change unreadable for every file written before
> it.**

The change was written, reviewed, mutation-tested at 100%, and merged. Every test
passed because every test wrote its fixtures with the *current* writer, which
emits the current schema. A test suite that only ever reads what it just wrote
cannot see a schema change at all.

**What catches a recurrence:** `older_files_still_read.rs`, which builds a
decisions file by hand in the *old* shape — deliberately copied rather than
derived from the live schema, because deriving it would make the test track
whatever the schema currently is, which is the one thing it must not do.

Verified in both directions: with the erroring accessor restored it fails with
`MissingColumn`, and with the fix it reads the row intact and the absent column
reads as `None`.

**The general shape:** an append-only store's tests are written by its own
writer, so they are blind to the difference between "the schema" and "the schema
on disk". Any store that cannot rewrite its history needs at least one test that
reads a shape the writer can no longer produce.

**And the smaller lesson.** This was found by asking "what happens when this
deploys", not by any check. The mutation score was 100% on the change that
introduced it. Coverage of the code says nothing about coverage of the *data*.

---

## 18. Two instruments compared as if they were one

**2026-08-30.** Found by an adversarial re-grounding, by reading what the two
prices in a comparison actually were rather than what the comment beside them
said.

`radar selection` reports the number this project exists to produce. It prices a
decision's entry from the smallest rung of the exit probe — a **sell quote**,
which is a bid net of fees — and its exit from `argMax(lam / tok, (ts, sig))`
over realised fills, which pools buys and sells and therefore sits near the
**mid**.

A bid measured against a mid is positive before the market has moved at all.

`selection.rs`'s own module documentation said, in as many words:

> Both prices come from the sell side: the entry from the smallest rung of the
> exit probe's ladder, the exit from realised fills.

The first clause is true and the second contradicts it in the same sentence.
Realised fills are not the sell side; the price query filters by transfer type
and by size and deliberately not by side.

**Cost:** the headline. `0014` published a gross median of **+21 bps** and called
it "noise around zero". Measured, the artefact is **at least +128 bps** — six
times the signal it was hiding — so the corrected median is at most **−107 bps**.
The published reading was not "we cannot tell"; it was the wrong sign.

**What made it survive:** every check that could have caught it was internal.
`return_bps` is correct arithmetic on its two inputs. `entry_price_of` is tested,
including `the_entry_price_is_on_the_same_scale_as_a_recorded_outcome` — which
asserts the two numbers share a *scale*, and a shared scale is exactly what makes
two incomparable prices look comparable. The test that would have caught it does
not exist in the suite and could not: it needs the live store, because the gap
between the instruments is a market fact and not a property of the code.

**What catches a recurrence:** `radar basis`, which measures the gap instead of
arguing about it, and reports it bucketed by the time between the two
observations — because the instrument difference and the market's own movement
can only be separated by how they behave with time. A pure artefact is flat
across the buckets; real movement grows with the gap.

**The part worth keeping is what the first run of that tool did.** Its two
tightest buckets held **zero** pairs, and not because the sample was thin: the
outcome pass runs at `:17` and `radar consider` at `:37`, so no pair can be
closer than about twenty minutes. Two of five buckets were spent on gaps that
cannot occur, and the verdict then refused to report because the bucket it cared
about was empty — discarding 1,779 usable pairs to protect a resolution the
system does not have.

**A measurement has to be designed against the cadence that exists, not the one
that would be convenient.** The buckets were wrong in the direction that looks
rigorous, which is the hardest direction to notice: a tool that refuses to answer
looks careful rather than mis-specified.

**And the shape of the answer mattered more than the answer.** The corrected
buckets read +128, +72, +46, −84 as the gap widens. Monotonic decay is neither a
pure artefact (which would be flat) nor a pure market effect (which would grow) —
it is a positive constant with a negative drift added. That is what licenses
reading +128 as a **floor** rather than an estimate, and the premise it rests on
is computed in the same data as the conclusion and carried beside it, so a reader
can refuse it. A one-sided claim that survives is worth more than a two-sided one
that needs extrapolation.

**The general shape, and it is entry 9's with money attached:** entry 9 recorded
an invariant documented more strongly than its enforcement, and cost nothing
because the code was right. This is the same failure where the code was wrong —
and the documentation was not merely stronger than the enforcement, it was
self-contradicting in a single sentence that had been read and reviewed and
merged. **Two numbers with the same units and the same scale are not therefore
the same measurement**, and the only thing that distinguishes them is where each
one came from.

---

## 19. A counter that counts the same thing again every hour

**2026-08-30.** Found by refusing to accept an unexplained number in a research
note that had already been written, reviewed and merged.

[`0017`](docs/research/0017-a-control-that-could-have-been-traded.md) reported
that **64–91% of its short-hold pairs returned exactly zero**, and said plainly
that it did not know why — the pairing already required the exit observation to
have *more fills* than the entry, so something was supposed to have traded. Two
readings were offered, wanting opposite responses, and the note declined to pick
one.

The answer was in `prices.rs`, twelve lines apart:

```rust
fills: self.fills.saturating_add(later.fills),
last:  later.last.or(self.last),
```

`WINDOW_HOURS` is **6**. The pass runs **hourly**. So every run re-reads five
hours it has already read, and the fold **adds** the fill counts.

A token whose single fill sits inside the window gains `fills += 1` on every
pass, for six passes, while `last_price` correctly never changes. **The counter
grows while nothing trades.**

**Cost:** the gate in two research notes. `control.rs` required
`exit.fills > entry.fills` to establish that a trade had happened between two
observations. It establishes nothing of the kind — the condition is satisfied by
the passage of time. 0017's headline and 0018's bands both rest on it.

**What made it survive:** every test of `fold` used hand-written windows and
asserted the arithmetic it was given. `folding_keeps_the_earliest_first_and_the
_latest_last` pins the asymmetry between `first` and `last` precisely, and is
right to. Nothing asserted anything about `fills`, because summing counts is the
obvious thing to do with counts — and it is correct for *disjoint* windows. The
bug is not in the fold. It is in the fold meeting a cadence chosen elsewhere, in
a cron line, in another repository's deploy notes.

**The general shape:** an aggregate is only correct with respect to how its
inputs were sampled, and neither half knows about the other. `fold` cannot see
`WINDOW_HOURS`; `WINDOW_HOURS` cannot see the crontab. Each is defensible alone.
The composition double-counts by up to six, and nothing in the type system,
the tests or the review had both facts in view at once.

**What catches a recurrence:** the consumer no longer asks `fills`. It asks
`last_transfer_slot`, which is `max(block_slot)` over the transfer aggregate — a
maximum cannot be inflated by re-reading the same rows, so an advance in it is a
transfer that actually happened. `a_growing_fill_count_is_not_evidence_that
_anything_traded` pins the property with a fixture whose `fills` climbs 3 → 9 →
27 while nothing trades.

**What is deliberately NOT fixed, and why that is a decision rather than
laziness.** The fold still sums. Changing it would make every row written
afterwards incomparable with every row already written, on an append-only store
that cannot be rewritten — which is entry 17's constraint arriving from the other
direction. The field's documentation now says it over-counts and says what to use
instead; the change to the fold, and whether the historical rows are worth
repairing, is a decision for whoever owns the data rather than a side effect of
finding the bug.

**The part worth keeping.** 0017 was merged with an unexplained number in it,
named as unexplained, with both candidate mechanisms written down. That is what
made this findable an hour later: the note did not round the anomaly away, and it
did not guess. **Publishing "I do not know why" beside a result is what let the
result be corrected instead of quietly believed.**
