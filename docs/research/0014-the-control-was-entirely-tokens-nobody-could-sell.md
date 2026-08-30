# 0014 — The control was entirely tokens nobody could sell

**Date:** 2026-08-30
**Status:** measured on the full recorded history, 4,374 decisions.

## The number the project exists to produce

Plan B/8: *"Join decisions to the price path and report what Radar's selection
returns against the baseline. **Report a null result as loudly as a positive
one.** This is the number the project exists to produce."*

Here it is.

```
                      proposed      refused
median return (net)       -829         -426
cleared costs           8.05%        47.19%
```

Read flat, Radar's selection is **anti-predictive**: the tokens it refused
returned better than the ones it proposed, and cleared costs six times as often.

That reading is wrong, and the reason is one line of the breakdown.

## Every scored refusal is the same refusal

```
reason                      decisions   scored     median    cleared
CapacityBelowFloor                889      606        424     47.19%
ExitCanBeStopped                   78        0          —          -
LaunchLooksCoordinated            137        0          —          -
NoExitSimulated                   137        0          —          -
NoRoute                           986        0          —          -
```

**606 of 606.** The control is not "tokens Radar refused". It is *exclusively*
tokens refused because **their exit capacity was below the floor** — which is to
say, tokens Radar measured and found it could not sell.

A +424 bps median and a p75 of **+6,490 bps** on a cohort defined by having no
exit is not an edge that was passed up. It is paper money. Buying those tokens
returns the loss and not the gain, because the gain is the part you cannot get
out of. That is what the refusal is *for*.

So the comparison was against a benchmark composed entirely of trades that were
impossible. LEARNINGS 11 is the same mistake made with MFE, and this is the
second time a cohort's headline number has been produced by the filter that
defined it.

## Why the other four are unscoreable, and why that matters

Not a gap in the data. A decision can only be scored if it has an **entry
price**, and an entry price comes from the exit probe — which runs only in the
paid tier, on candidates that already survived the free filters.

`NoRoute`, `NoExitSimulated`, `LaunchLooksCoordinated` and `ExitCanBeStopped` are
all refusals raised *before or instead of* that probe. They have no entry price
by construction and never will.

**So the control can only ever contain tokens that got far enough to be priced,
and the only refusal that happens after pricing is `CapacityBelowFloor`.** The
"matched control" is structurally a single cohort wearing the name of five. That
is a limitation of the selection measurement itself, not of this run, and it will
not improve with more data.

## What can honestly be said about the selection

Stripping the broken comparison away, what remains is the proposed cohort on its
own:

| | gross | net of 850 bps |
|---|---|---|
| p25 | −440 | −1,290 |
| **median** | **+21** | **−829** |
| p75 | +204 | −646 |
| cleared costs | — | **8.05%** |

**Radar's proposals are, at the median, almost exactly break-even before costs
and lose the entire round trip after them.** A gross median of +21 bps is 0.21%:
noise around zero.

That is a null result, and it is the honest headline. It is not evidence the
selection is *harmful* — the control that suggested that is unusable — and it is
not evidence it works.

## What would move this

- **A control that could actually have been traded.** The obvious candidate is
  tokens the paid tier priced, found sellable, and the *strategy* passed over —
  which requires the strategy to refuse something after the exit probe, and today
  it does not.
- **Costs.** 8.05% clearing an 850 bps round trip against a gross median of
  +21 bps says the round trip is most of the problem. A cheaper exit changes this
  number more than a better filter would.
- **An exit rule.** Every figure here is *held to the last observation*. Radar
  has never taken a profit because it has never sold. Plan E/21.

## What this does not establish

- **One instance, one regime.** 4,374 decisions since 2026-08-26, all from one
  machine watching one venue.
- **No execution.** These are prices, not fills. Nothing here has slippage,
  partial fills or a failed transaction in it.
- **`CapacityBelowFloor` is a Radar measurement**, not a fact about the token. A
  different probe size or a different floor would move which tokens land in it.
