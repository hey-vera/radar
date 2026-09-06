<!-- SPDX-License-Identifier: Apache-2.0 -->
# STATE.md - what has actually been measured and built

Split out of [AGENTS.md](../AGENTS.md) so that file can be an operating
policy rather than a reference. Everything here is a **claim about the
world**, and claims decay: when this file and the repository disagree, the
repository is right and this file is a bug worth fixing in the same change.

The four most likely to be stale: the measured edge, what the trading lane
can reach, which venues are recorded, and whether anything has traded.

Radar is a Solana research and trading platform. The goal is not "a trading bot".
It is infrastructure that can **systematically discover, measure and exploit real
edges in Solana markets**, and expose the resulting intelligence to other agents
over x402.

Two facts set the tone, and both are load-bearing:

1. **The base rate is brutal.** In April 2026 — one of the best months in two
   years — roughly 96% of pump.fun wallets either lost money or made under $500,
   and only 5.4% cleared $1,000. Any design whose success case is "beat the
   average memecoin trader" is a plan to lose slowly.
2. **The realistic edges are unglamorous and measurable**: not buying traps,
   sizing for the exit rather than the entry, disciplined execution cost, and
   composing signals that are individually weak. Radar is built to measure those,
   not to assume them.

So the first deliverable is not a trade. It is a **recorder with a point-in-time
guarantee**, because the dataset only accumulates forward.

**The recorder has now produced its first verdict on the selection, and it is
negative.** Over 4,374 decisions,
[`0014`](../docs/research/0014-the-control-was-entirely-tokens-nobody-could-sell.md)
measured Radar's proposals at a gross median of +21 bps and called that noise
around zero. It is not noise: that figure compares a **sell quote** against a
**mid**, and [`0016`](../docs/research/0016-the-entry-was-a-bid-and-the-exit-was-a-mid.md)
measures the gap between those two instruments at **at least +128 bps** — six
times the signal it was hiding. Corrected, the gross median is **at most −107
bps**, and **−957** after the measured 850 bps round trip.

The comparison against refusals that appeared to make it worse is unusable:
every scoreable refusal is `CapacityBelowFloor`, so the control is composed
entirely of tokens Radar measured and found it could not sell.

[`0017`](../docs/research/0017-a-control-that-could-have-been-traded.md) builds the
control that comparison lacked, against **38,461** tokens Radar never decided on,
priced the same way on both sides and matched on token age and holding period.
It finds **no edge** — a median edge of 0 bps across four matched strata. Both
that note and [`0018`](../docs/research/0018-the-deep-tail-points-the-wrong-way.md)
were re-measured after LEARNINGS 19 corrected the pairing gate, and both
conclusions survived it.

The figures are the **corrected** run: 990 proposals against 38,461 control
tokens. An earlier version of this paragraph carried 121,810, which is 0017's
superseded pre-correction control and is ~3.2× the real one. 0017's own header
line still carried the superseded pair until 2026-09-03; both are now fixed.

0018 is the one to read next. Radar sizes every position as a share of measured
exit capacity, and 80% of proposals sit in a ±13% band around $31 because every
pre-graduation pump.fun token rides the same curve — so the median position is
**$6.21**, needing a +8.5% move to clear the round trip. The one band where a
real position fits, $60+, is **down 68% on 25 rows**.

**0018 concluded from that "the binding constraint is capacity, not signal", and
that conclusion is superseded.** This file said it too, and it was wrong for as
long as 0022 has existed.
[`0022`](../docs/research/0022-capacity-was-a-budget-not-a-ceiling.md) reads four
bonding curves off mainnet and finds the ~$31 is the output of Radar's own
`Search::DEFAULT { max_impact_bps: 100 }` — a **policy setting, not a venue
wall**. The same curve supports roughly **$500** at an impact equal to the round
trip, a factor of about sixteen. 0022 struck its own recommendation to raise the
budget and said why in one line:

> "**Capacity was never the reason this does not work.**"

**The binding constraint is the bar, not the depth.** A strategy has to clear
roughly **456 bps** of expected edge before a single trade is worth making, and
roughly **850 bps** before a position larger than about $59 makes sense.
[`0017`](../docs/research/0017-a-control-that-could-have-been-traded.md) measures
the edge at **0 bps**, so the arithmetically correct size is not small but
negative — which is what `Policy::CLOSED` already is. `max_impact_bps` should not
move until something measures an edge above that bar; raising it first would only
spend money faster in the direction 0017 says there is nothing in.

Read that before adding a filter. The gross median says the selection is not
finding an edge, and the round trip is currently larger than anything the filter
has found.

## The two measured signals, re-measured

Both were measured on a two-day window at 72,193 launches. The store now holds
483,629, and [`0024`](../docs/research/0024-the-spike-became-a-hump-and-the-signal-moved.md)
re-ran both on 2026-09-03.

**The launch-block signal did not survive intact.** `0008`'s headline — 68% of
instantly-graduating launches have exactly six recipients — is **25.1%** on
17,497 launches. The enrichment over the 5.5% control is real; the magnitude was
wrong by 2.7×, and the "spike with holes either side" that made the structural
argument was sampling rather than structure. The strongest band has moved off six
to **ten to thirteen** (10.1× base). `0008` predicted exactly this failure — *"six
is a tool's default, not a law"* — and nothing was built to notice it, so it was
found nine days later by hand.

What replaces it is stronger and points the same way the project already points:
**one to three recipients covers 70.5% of launches and graduates instantly in
0.02% of them.** The reliable signal is the refusal.

**The creator signal strengthened.** `0007`'s separation holds on 2,957 creators
against its original 638, at **3.37×** band-to-band (0007 had 2.12×) and **1.59×**
against the base rate — and it survives controlling for launch frequency in all
three bands. Neither is a reason to buy: `0011` still says graduation predicts
volatility rather than profit, at a median −3,228 bps.

The numbers a consumer should read are in
[`data/0024-base-rates.json`](../docs/research/data/0024-base-rates.json), each
carrying the date it was measured.

## The learning loop has an instrument, and it has not been run yet

**Built 2026-09-05, plan 0007 items 1 and 2.** `radar features` writes one row
per succeeded launch with twenty-three features, every value observed at or
before T = launch + 6,000 slots and accepted through `AsOf::accept`, so a
feature computed from something that had not happened yet is a build error
rather than a column. `radar edge` runs the walk-forward protocol over that
file: five contiguous windows by launch slot, the first three fitted as one,
purged and embargoed by twenty-four hours, and a stratum is `Found` only if it
clears the bar on **both** remaining folds with at least a hundred rows each and
a Wilson lower bound above a half.

**The trades table is empty, and that halves the table.** Read on the box on
2026-09-06: `~/radar/data/store/trades` was created on 2026-08-23 and has never
been written to. Twelve of the twenty-three features are trade-derived, and they
are **absent** rather than zero — decided from the trades table's own partition
coverage, so they become measurements again the day the recorder writes one,
with no code change. Nine features remain: the creator's record, the launch
metadata, the dev buy, and what the decision lane recorded. Getting the recorder
to write trades is the highest-value repair available to this work and it is not
part of it.

**No result exists.** Neither command has been run against the production
store — that needs a Linux binary on the box, and this workstation has no store
to run them against. So the honest state of the number is unchanged: research
0017 measures the selection edge at **0 bps** against a bar of about **456**,
and nothing here has moved it. What exists is the instrument that could.

**Design 0010 §6.1 is superseded on the cost, and this file is why.** The
design charged a `by_notional` band and then asked for 456 bps on top. The
reconciliation table above says those are the same measurement in two bands, so
that charges one number twice; and it says these rows — fresh launches — belong
to the 850 cohort, which 0019 declined to lower because a cost rounded down
launders a trade past the gate. The harness charges **850** by default, offers a
band for sensitivity, and accepts a stratum on three conditions: at least a
hundred rows, a net **measurably** above zero, and more than half the rows
paying at the Wilson lower bound. That is stricter than the design's rule, not
looser.

Two more things the protocol does that design 0010 §6.2 did not say, both found
by a test rather than argued (plan 0007 Q3): the fitting-period row floor is scaled
up from the smallest test fold, because a stratum too narrow to hold a hundred
rows in a test fold can never be accepted and fitting on it displaces one that
could have been; and the fit-fold winner is chosen under a one-standard-error
rule with the Wilson bound breaking ties, because taking the highest median
alone preferred a refinement fitted to noise and dropped a planted 3,000 bps
edge on the floor. `an_engineered_edge_is_found` is the test that failed twice
and made both changes.

## The round trip is three numbers, and they are not in conflict

Three figures circulate in this repository and **no document reconciled them
until this one did**. They measure different things, and quoting the wrong one
understates cost by up to 3.4×. Anything that publishes a cost figure — the
kernel, a research note, the roaster — resolves it here.

| bps | what it is | measured in | scope |
|---|---|---|---|
| **250** | the pump.fun **venue fee**, round trip — 125 bps a side, read off the on-chain `FeeConfig` | [`0023`](../docs/research/0023-the-fee-is-a-schedule-and-the-published-interface-is-incomplete.md) | the fee alone. Excludes impact, rent and failed transactions. |
| **456** | the **bar**: expected edge a strategy must clear before one trade is worth making | [`0022`](../docs/research/0022-capacity-was-a-budget-not-a-ceiling.md) | all-in round trip for a position in the **$20–$200** band, used as the fee rate `a` in `s* = (r − a) / 2b` |
| **850** | the **measured all-in round trip** the kernel assumes, `creator_edge::Thresholds::assumed_round_trip_bps` | [`0019`](../docs/research/0019-the-round-trip-is-not-one-number.md) | fresh-launch pump.fun tokens — the cohort Radar's own trades belong to. Every net figure in this repository rests on it. |

**Why they differ, precisely.** 0019 measured 183,647 legs and found cost is
**size-dependent, and the dependence is at the bottom**:

```
notional band   median bps / leg   round trip
$0.20 – $2                 1,521       3,042
$2 – $20                     125         250
$20 – $200                   228         456
$200 – $2,000                225         450
$2,000+                      130         260
```

So 250 and 456 are the same measurement read in two different bands, and 0022's
"fees rt" column is this table doubled rather than a fee schedule. That 250
coincides with 0023's venue fee is a real finding rather than a clash: in the
$2–$20 band the measured all-in cost is **the venue fee and essentially nothing
else**.

**850 is none of these bands and is not superseded by them.** 0019 measured *all*
pump.fun trades in an hour; the 850 came from trades on 200 tokens launched in
that hour — the fresh-launch cohort, where a new associated token account is rent
and early curve positions carry more slippage. 0019 **explicitly declines to
lower the constant** on that evidence, because the population is wrong and the
error direction is the dangerous one: a cost estimate rounded down launders a
trade past the gate that should have refused it.

**The number nobody else publishes.** `min_notional` is `MicroUsd::DOLLAR` —
$1.00 — which lands in the 1,521 bps band. **A position at Radar's own floor
faces a round trip of roughly 30%.** That is measured, it is about the venue
rather than a prediction, and it is the strongest single fact this repository
holds.

The trading lane exists and is shut, and it is worth being exact about *where*
it is shut, because an earlier version of this paragraph was not.

`radar-strategy`, `radar-risk`, `radar-signer` and `radar-exec` are each built and
tested, and **as of 2026-08-31 the lane is composed end to end** by
[`crates/radar-exec/tests/lane_composes.rs`](../crates/radar-exec/tests/lane_composes.rs):
one real candidate runs strategy → kernel → executor, and the executor's
`Routing`, `Signing` and `Sending` traits are stubbed so the ordering can be
exercised without a network or a key.

Be exact about what that does and does not establish, because the previous
version of this paragraph was exact and it is worth staying that way.

**What it establishes.** The stages agree about what a fundable proposal looks
like. The signer is handed the kernel's authorisation verbatim rather than a
reconstruction. A signer refusal ends the attempt without sending. A trade that
does not pay for itself never reaches the process holding the key. And
`Policy::CLOSED` refuses the same candidate a permissive policy authorises —
which is what makes the other tests statements about the lane rather than about
the fixture.

**What it does not.** One production crate depends on `radar-exec`:
`radar-cli` reaches `radar_exec::route` for `radar route`, which prints unsigned
bytes. Nothing in production reaches `pipeline::execute`, the signer client or
the submitter — the composition reaches *those* through a dev-dependency, so the
shipped graph still cannot sign. Nothing has been signed, sent, or filled.
`repo-conformance`'s `the_documented_dependency_claims_are_true` pins it.

**As of 2026-09-01 the pipeline's traits have real implementations**, which they
did not before: `Routing` and `Sending` were satisfied only by stubs inside
`pipeline.rs`'s own test module, while `route::Router` and `submit::Submitter` —
which talk to Jupiter and to an RPC node — sat beside them unconnected. So the
executor could be composed only against a fixture, which is LEARNINGS 10's shape
exactly. `Router` and `Submitter` now implement the traits, and
[`the_pipeline_has_real_implementations.rs`](../crates/radar-exec/tests/the_pipeline_has_real_implementations.rs)
checks the trait methods delegate to the real ones rather than being present and
inert.

**There is still no production caller** for the *trading* path. Nothing invokes
the pipeline, for the local wallet or a customer's. Writing one is opening the
trading path, and it is a decision about money rather than a wiring task.

**The customer lane also composes, as of 2026-09-01**, in
[`the_customer_lane_composes.rs`](../crates/radar-exec/tests/the_customer_lane_composes.rs),
and its shape differs from the local one in the way that matters: **no process in
Radar can produce a signature on its own.** Three parties hold one thing each —
the executor an application credential that authorises nothing, `radar-signer` a
P-256 key that authorises one request it has checked, and Privy the wallet key.
Moved here from AGENTS.md rule 1 on 2026-09-03: it is status, and rule 1 is a
rule.

**What that test establishes.** An authorised trade reaches Privy signed. A trade
for another token stops **inside Radar** and never reaches the network. A
`Policy::CLOSED` in the signer's own file refuses the identical request a
permissive one allows. The body Privy receives is the body the signer authorised.
A spent signature allowance refuses before anything is signed. The signer in it
is the real `verify::check`, not a stub.

**What it does not.** Nothing has been signed by Privy, sent, or filled, and no
customer has ever existed. `Policy::CLOSED` is still what ships.

`radar route` is the first thing that runs any of it. It builds a swap
transaction and describes it, holding no key and no RPC endpoint, so it cannot
sign and cannot send. Every cost and failure rate in that composition is supplied
by the test rather than measured.

**[`0021`](../docs/research/0021-the-signer-cannot-read-the-only-venue-that-lists-them.md)
found that Radar could not build a transaction for any token it selects, and
never could** — Jupiter routes pre-graduation pump.fun liquidity only as a
versioned transaction, the signer accepts only legacy ones (ADR 0003), and Radar
selects only pre-graduation pump.fun tokens. Eight of eight candidates confirmed
it. LEARNINGS 24.

**That is fixed as of 2026-09-02, and it is worth knowing what the fix does and
does not give you.** [ADR 0009](../docs/adr/0009-radar-builds-its-own-pump-fun-swaps.md)
builds the swap directly rather than asking an aggregator for one, in
[`radar-pumpfun`](../crates/radar-pumpfun) — a pure crate with no network and no key.
A buy rebuilt from a mainnet capture **simulates against mainnet with no error**,
and `radar-signer`'s real `verify::check` reads a transaction that crate built
([`the_signer_reads_what_this_crate_builds.rs`](../crates/radar-pumpfun/tests/the_signer_reads_what_this_crate_builds.rs)),
including refusing one whose authorisation names a different mint. The venue was
never the obstacle: every capture behind that crate is a **legacy** transaction.

What it does not give you: nothing has been signed by a wallet, sent, or filled.
A simulation with `sigVerify: false` proves the instruction is well formed and the
accounts resolve. It does not prove a signed transaction lands, or at what price.

Two things that came out of building it and change other numbers.
[`0023`](../docs/research/0023-the-fee-is-a-schedule-and-the-published-interface-is-incomplete.md)
reads the venue fee off the chain at **125 bps a side** — 250 for a round trip,
which is what 0022 assumed without checking — and finds that the program's own
published IDL declares **sixteen** accounts for a buy where mainnet passes
**eighteen**. A builder written against the most authoritative available reference
would have been two accounts short. LEARNINGS 25.
[`0028`](../docs/research/0028-the-fee-after-graduation-is-a-ladder.md) then
read the same fee program's **second** schedule, the one for the PumpSwap AMM a
coin graduates into: twenty-five rows by market cap, the creator's share 30 bps
below 420 SOL, 95 from there to 1,470, down to 5 above 98,240 — the venue's
published ladder, to the row, with live swaps paying it. The same parser read
it; the prize page now says which fee applies where.

The shipped policy is `Policy::CLOSED`, which refuses every proposal. But the lane
is shut a long way upstream of that too: on 2026-08-25 a live run over 41,254
candidates raised **zero proposals**, and the cause was a hardcoded exit-probe size
that made a proposal arithmetically impossible rather than a market that offered
nothing. `Policy::CLOSED` has never refused a real proposal, because it has never
been handed one. See [LEARNINGS](../LEARNINGS.md) entry 10.

If you are changing `Policy::CLOSED`, you are making a decision about money — make
it deliberately, and not as a side effect of something else. Note that opening it
before the funnel has been exercised with a real proposal would be opening a path
nothing has ever tested.

## The public analyst, as of 2026-09-04

**Code-complete and posting nothing.** `radar dossier`, `radar roast` and
`radar analyst --mentions <file>` run offline; `radar-analyst` is also a daemon
under `deploy/radar-analyst.service` that polls mentions, answers them, meters
what it spends and logs every reply beside the fact sheet it was built from.

**The only thing between it and a public account is a credential and a
decision.** `RADAR_X_BEARER` and `RADAR_X_USER_ID` is the switch; with either
absent the publisher is the dry run and nothing can be posted. The four prices
have **no defaults**, so an instance that has not been told what it is charged
answers nothing rather than spending an unmetered amount — which is what makes
the two unverified X billing figures a line in a config file rather than a
blocker.

**The voice, as of 2026-09-06.** "Meters what it spends" was an overclaim until
that date: `Cost::ModelCall` was priced and never authorised, so the model call —
the one billable thing a stranger can trigger — was the one thing the day's
budget never saw. It is now reserved before the call and settled at what the
provider reported, on both the X and Telegram lanes, against one budget.
`voice::Reply` carries `Billed` rather than an optional cost, because *no call
was made* and *a call whose cost nobody reported* settle in opposite directions
and rule 9 is exactly that distinction.

Three metered providers are compiled in and **exactly one may be configured**;
two is refused at start-up with both named. `RADAR_MODEL_OPENAI_KEY` speaks Chat
Completions, `RADAR_MODEL_API_KEY` speaks Anthropic's Messages shape
(`claude-sonnet-5` and its siblings), and `RADAR_MODEL_CODEX` is the
subscription CLI, private use only and not for this account. The endpoint, the
model name and the two prices are shared, so moving between vendors is an env
edit and a restart. **No key is on the box yet**, so every reply in the log is
still the deterministic template.

Three properties are load-bearing and each is pinned by a test that was verified
by re-applying the bug it prevents:

- **A reply is recorded before it is said.** `publish` appends the intent, posts,
  then appends the outcome. It used to post first, and a failed log write would
  have left a public statement with no record of it.
- **The reply is sanitised before it is checked, never after.** The forbidden and
  fidelity checks read characters, and a zero-width space renders as nothing —
  `s<ZWSP>cam` is two tokens to a checker and one word to a reader. Cleaning
  afterwards would assemble exactly the statement the checks refused.
- **The gate refuses before the chain is read**, because the read is the
  expensive part. **And, since 2026-09-05, the summoner's cap is charged on
  admission**, not on sending: the read is the cost, and until then thirty
  mentions from one account naming thirty unreadable mints — or thirty of
  anything while the publisher was down — were thirty dossiers and no refusal,
  because the cap counted replies and none of those was one. The account's
  global cap is still charged on sending, so a publisher outage cannot spend
  it. Found by the 30-mention burst case in
  `crates/radar-analyst/tests/one_poll_end_to_end.rs`, which now runs against
  a chain that counts its requests.

**Every reply carries its refusal-signal count, as of 2026-09-05** (design
0009 M3). `radar_roast::sheet::Signal` has three variants — the launch block in
the snapshot's strongest band or above it, a creator with measured launches and
no organic graduation, a dev buy that was seen — computed where the sheet is
built and never rendered, so the model is shown facts and not a verdict. The
strongest band is read off the snapshot (`BaseRates::strongest_band`), which is
research 0024's lesson made mechanical. The reply log entry carries them as
`Option<Vec<Signal>>`: `None` on a line older than the field, which is unknown
and not zero. The tally in `radar-contest` sums them; the job that feeds it the
log is plan 0006 item 6.

**The prompt and the template stopped disagreeing, as of 2026-09-06** (plan 0008
item 4). `verdict::template` was changed on 2026-09-05 to put the round trip
**last**, because it is the same 456 bps in every reply and three real launches
had produced three identical replies. `voice::SYSTEM` rule 6 still said *"lead
with the cost line"*. Both shipped, they contradicted each other, and the prompt
was the wrong one: it now says to lead with the fact that is about *this* coin —
the creator's record, or the launch block — and to put the round trip last or
not at all.

`verdict::headline` is the new anchor: one sentence under a hundred characters,
built only from the sheet, so it passes `fidelity::check` and `forbidden::check`
by construction. The template prints it as its second line and the request
offers it to the model as "what Radar prints if you say nothing". It is `None`
when the sheet has neither a creator record nor a launch block — rule 9, since
an unknown creator is not a creator with zero launches, and *"0 launches by this
creator"* would be a damning sentence invented out of an absence.

**`SYSTEM` now carries no figures at all**, enforced by a test that strips the
rule numbers and asserts no digit remains. A number in the system prompt is on
no fact sheet and is in front of the model for every coin, so an example figure
is one the model can echo into a reply about a token it does not describe —
where `fidelity::check` would then bin an otherwise good reply. "Under a hundred
characters" is spelled out in words for exactly that reason.

**The Telegram lane exists, as of 2026-09-05** (design 0009 L5, M5; plan 0006
item 5). `crates/radar-analyst/src/telegram.rs`: `getUpdates` polled in the same
loop as X, each text message through the same parser, an admission gate of the
same shape with its own caps, the same fact path and the same two checks; the
reply goes back to the chat the question was in. Rule 8 twice: no
`RADAR_TELEGRAM_BOT_TOKEN` reads nothing, and a token alone answers into
`telegram.jsonl` only until `RADAR_TELEGRAM_PUBLISH=on`. **A Telegram answer is
not in the record and cannot enter the contest**, because it is in a different
file and nothing that scores the contest reads that file. The daemon names the
lane's state on start; `radar brief` counts it once the file exists. **Nothing
has been sent through it**: there is no bot token on the box, and the box does
not run the analyst.

**The week closes and the account has two appointments, as of 2026-09-05**
(design 0009 §7, M2, M4; plan 0006 item 6). On the first tick after Monday
00:00 UTC, `contest.rs` reads the closed week's published replies from the
log, their public metrics and the entrants' account ages from X, the week's
refusals from `refusals.jsonl` — which the gate now writes — and earlier
records for the cooldown, applies the published rule, and writes
`data/contest/<week>.json` and the hunter tally beside it, atomically. A reply
the platform returned no metrics for is excluded as unscored, never scored
zero. Then `weekly.rs` posts the result under 280 characters — counts, the top
reply's score and URL, the pool in SOL, never a price, never a handle — with
the winning coin's fact sheet as the reply, every numeral on an authorised
list the same fidelity and forbidden checks read. Daily at 12:00 UTC,
`daily.rs` posts "seven days later" from a file `radar seven-days-later`
writes on a timer: the join of week-old replies against the store's outcomes,
which the daemon is not allowed to make itself. Both go to X, priced as a
top-level post (`RADAR_X_PRICE_POST`, the fifth required price), and to a
Telegram channel when one is configured. **Nothing has been posted**, for the
same reason nothing has been replied to.

**The payout exists and has paid nothing, as of 2026-09-05** (design 0007 C3,
C4, C5; ADR 0013; plan 0006 item 7). `crates/radar-payout` is its own binary,
unit, user and key: it reads a week's record, reads the creator vault, plans
one transaction — `collect_creator_fee` then a system transfer of everything
above the vault's rent reserve to the claimed address — under
`Payout::permitted`'s three refusals, signs with the creator key, sends over
direct RPC, **reads the transaction back** and records the payout only when
the chain confirms exactly the planned transfer. The manual fallback (`radar
contest pay --dry-run`, `radar contest record-payout`) prints the unsigned
transaction and records a hand-made one through the same read-back. A winner's
claim is a reply naming an address inside the seven-day window, written into
the record by the analyst's loop and not answered as a summons. Three things
are stated rather than proven: the `collect_creator_fee` account order comes
from the program's on-chain IDL and not yet from a mainnet capture; the vault's
rent reserve is the runtime's zero-data minimum and not yet a measured
post-collect balance; and no key, wallet or token exists, so nothing here has
run against a chain. The devnet week design 0007 §6.3 requires is where all
three become captures.

**The launch has a checklist and the brief has two more lines, as of
2026-09-05** (design 0007 C7; plan 0006 item 8, the last). `deploy/README.md`
*The token: the launch checklist* is nine steps in order, the first five
Josh's. `radar brief` gains `contest` — the latest closed week and where its
prize stands — and `vault` — the creator vault as `radar-payout` last read it,
`????` when the reading is missing or more than two days old. Both alarm on
absence only once `RADAR_CONTEST_DIR` is set, on the analyst check's rule.
**With that, plan 0006 is complete**: everything design 0009 asked for that
could be built without a credential, a wallet or a token is built, tested with
its bugs re-applied, and documented here. What remains is on Josh's list.

**The self-mint rule is built, as of 2026-09-05** (ADR 0013 constraint 5,
design 0007 C8). Every fact on the sheet carries an `About` tag, and the sheet
builder drops every `About::Price` fact for the mint in `RADAR_SELF_MINT` before
the model sees the sheet, leaving one line that says why. **No fact on the sheet
is a price today** — the builder emits structure, history, depth, cost and
population — so the rule has nothing to drop yet; it exists so the first price
or market-cap fact anyone adds is withheld for the analyst's own token by
construction. The residual, stated: `Fact::exact` and `Fact::share` tag a
measurement, so an author adding a market-cap line through them has to choose
`About::Price` themselves, and nothing makes the compiler ask. A
`RADAR_SELF_MINT` that is set and will not parse idles the daemon rather than
running with the rule off; unset means no token is special, which is the right
configuration until the token exists.

**The contest crate exists, as of 2026-09-05, and is pure** (design 0007 C1,
design 0009 M3): weeks that open Monday 00:00 UTC; the published score,
three per repost and quote and one per like and reply, over the bot's own
replies; every exclusion returned with its reason; a winner with a three-week
cooldown; the seven-day claim window; one JSON record per week; the payout
policy as one function with three refusals; and the hunter tally with the
daily cap applied again so volume cannot win. No clock, no network, no key.
**Its callers are not built yet** — the public leaderboard endpoint reads its
record, the week-close job writes one, `radar-payout` asks its policy — and
plan 0006 lands them in that order. **The first landed the same day**: the
public site's three documents — `/v1/public/stats`, `/v1/public/leaderboard`,
`/v1/public/pool` — are served by exact path to anyone, each from a published
file and never the store, with the CORS header set only for the one origin in
`RADAR_SITE_ORIGIN` and absent otherwise. The leaderboard reads the contest
crate's week record when a week has closed, and the reply log with every
score `null` before that; the pool says no token exists rather than `0.00`.
The stats document needs `population.json`, which the creator-index job now
writes beside the index; until the box runs that binary the endpoint answers
404 and the site shows its dated fixture.

**The public site is five pages as of 2026-09-06** (plan 0008 item 1): home,
leaderboard, prize pool, tokenomics and about, still static files with no
runtime of its own. What changed beyond the redesign is what is now *checked*.
`index.html` carried the sentence "checked by the same test that checks the
rendered page" and no such test existed, so the `<noscript>` figures — the copy
crawlers and link unfurlers read — were the copy nothing checked;
`figures.test.ts` now derives them from `stats.json` through the same functions
the page uses, and pins that fixture's band, cost and aftermath blocks to
`0024-base-rates.json`. The fixture itself was refreshed from the box's creator
index at slot 444,637,451, and the tokenomics page's fee ladder is asserted
against `radar-pumpfun`'s own decoder from that crate, because the capture is
hex and a TypeScript copy of the fee parser would be a second answer to get
wrong. Every URL on the site is now built by a function in `honesty.ts` that
can refuse, rather than by string template in a component.

**The account's handle is `thecabalhunter`**, confirmed by the operator on
2026-09-06. It had been recorded in no file — `deploy/README.md`'s `curl`
example said `CabalHunter`, which is a different account, and anyone following
it would have set `RADAR_X_USER_ID` to the wrong id or none at all. That example
is corrected.

**The site still does not hard-code it, and that is deliberate.** Nothing on the
Rust side needs a handle: the analyst identifies the account by
`RADAR_X_USER_ID` and `radar-serve` builds reply links as
`x.com/i/web/status/<id>`. The site is the only surface that wants a name, and a
name can change without anything breaking loudly — so it is gated on
`VITE_X_HANDLE` at build time, validated against X's own rule, and every link to
the account renders an honest sentence when it is absent. Rule 8, in the
interface. Until it is set in the Cloudflare Pages environment the site has no
outbound link to X.

**A claim is a reply to the account's claim prompt, as of 2026-09-06** (plan
0008 item 2). This closed a real hole rather than adding a feature. `try_claim`
accepted any base58 address in any mention by the winner inside their claim
window; a coin's mint address is such an address, so a winner who summoned the
bot about a coin during their own claim week had that mint written in as their
payout address, and `Payout::permitted` would have approved paying it, because
the recipient really did equal the claim. Design 0007 §6.2 specified the
mechanism that prevents this — the account replies to the winner, and the claim
is a reply to *that* — and it had never been built, so there was also no post
telling a winner they had won.

`Record::claim_prompt` now holds that post's id and a claim must reply to it.
**Stricter than the design in one place, deliberately**: 0007 also accepts a
reply under the winning reply itself when the prompt failed to post, and that is
refused, because a winner replying "now do this one" with a fresh coin under
their own winning reply is an ordinary summons. The daemon re-posts the prompt
on every tick while the window is open, so the strictness costs a delay rather
than a week, and an unclaimed pool rolls over.

The week close reads each entrant's handle from the `/2/users` call it already
makes — same call, same page, same price — so the leaderboard document carries
`handle` beside `summoner`, and exclusions as counts by reason rather than as
named rows. `api.ts` had documented a handle field the Rust side never sent,
which is why the site rendered `@1234567890`.

One correction worth recording, because it was believed while writing this:
`#[serde(default)]` is **not** what keeps old records parseable. Serde already
maps a missing `Option` field to `None` without it — established by probe. The
enforcement is a test that parses the exact bytes of the box's `2956.json`, and
the hazard it guards is a *required* field being added, which `records_in`
would answer by skipping the file silently, dropping the week from the
leaderboard and from the cooldown that reads it.

Nothing here touches the store, the signer or `Policy::CLOSED`. It is read-only
against the chain and append-only against its own log.

## Where to start

- [`docs/research/`](../docs/research/) — what was investigated and what it found,
  including the data-sourcing landscape and the freshness/caching design.
- [`docs/adr/`](../docs/adr/) — decisions, with what each one costs.
- `crates/radar-provider` — the spend meter, and as of 2026-09-04 nothing else.

  What runs, and did before: `radar-agent` reserves against `Budget`, `Ledger`,
  `Meter` and `Commitment` for every model call, and
  [`radar-serve`'s ledger](../crates/radar-serve/src/ledger.rs) persists that
  across a restart. It is rule 8 enforced in the running system.

  What is gone: the cache, the breaker and the planner that composed them —
  about 1,300 lines against the 484 that run, with no caller outside the crate
  and none since it was written. This file called for them to be wired or
  deleted, three documents flagged them, and deleting is what happened
  (design 0007 J9). The crate's own module doc records why, and what to extend
  instead: `radar-serve` has a live cache that already carries the lesson
  LEARNINGS 27 paid for.

  Two consequences worth knowing. `radar-asof`'s `Observed<T>` and `LookAhead`
  lost their only caller when that cache went, and **they have one again since
  2026-09-05**: `radar-research`'s feature table takes every value as an
  `Observed<f64>` and accepts it against `AsOf(T)`, so a feature computed from
  something that had not happened yet is a build error rather than a plausible
  column. That is design 0010 §8.1 row 3 settled in the direction of keeping
  them. And AGENTS.md rule 3 no longer claims a type-system half; it was
  describing that cache.

  The economics that actually run for *pricing* are a separate, static cost model in
  [`radar-instruments`](../crates/radar-instruments/src/spec.rs), where each
  instrument *declares* its cost by hand ("a promise, not a measurement") and the
  x402 price is derived from that declaration. So the price Radar charges is not
  connected to what Radar spends, and nothing notices if the two diverge.
