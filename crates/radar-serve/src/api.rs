// SPDX-License-Identifier: Apache-2.0
//! The JSON surface the operator interface reads.
//!
//! Separate from the instrument registry on purpose. An instrument answers a
//! question about a *token* and is priced per call; these routes describe what
//! **Radar itself** has done, which is not something anyone should be charged
//! for and not something the x402 lane should serve.
//!
//! Every function here is a pure shaping of rows the store already holds, so
//! the shape can be tested without a server and without a network. The axum
//! handlers in [`crate`] are three lines each and contain no logic.
//!
//! # Nothing here computes a number the store does not have
//!
//! The free tier of `radar consider` refuses tens of thousands of candidates a
//! day and records none of them, because a free-tier refusal is a pure function
//! of the store and can be re-derived. So the funnel below starts at what was
//! *examined*, not at what was considered, and says so. Reporting a
//! considered-count from a stale run would be inventing the widest number on the
//! page.

use radar_asof::AsOf;
use radar_store::{Conclusion, Decision, Reader, Table};
use serde::Serialize;
use std::collections::BTreeMap;

/// One stage of the recorded funnel.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct Stage {
    /// Short identifier, stable across releases.
    pub name: &'static str,
    /// How many reached this stage.
    pub count: usize,
    /// What the stage means, in a sentence a novice can read.
    pub detail: &'static str,
}

/// How often a reason was given.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct ReasonCount {
    /// The reason, as the strategy names it.
    pub reason: String,
    /// How many decisions carried it.
    pub count: usize,
}

/// What Radar has decided, and where it stopped.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct Funnel {
    /// The watermark this was read at.
    pub as_of: u64,
    /// The stages, widest first.
    pub stages: Vec<Stage>,
    /// Why candidates were passed over, most common first.
    pub reasons: Vec<ReasonCount>,
    /// Whether the policy this build **decides** with could authorise anything.
    ///
    /// The single most important fact on the page, and the one a raw refusal
    /// list buries under six other reasons.
    ///
    /// # What it does not cover, stated because the field reads stronger than
    /// # it is
    ///
    /// This is `radar_risk::Policy::SHIPPED` — the constant `radar consider`
    /// judges against. It is **not** a guarantee that nothing can be signed.
    /// [ADR 0008](https://github.com/hey-vera/radar/blob/main/docs/adr/0008-the-signer-holds-its-own-policy.md)
    /// put a second policy in `radar-signer`, loaded from `RADAR_SIGNER_POLICY`
    /// at start, and this process cannot read it. The signer clamps against its
    /// own copy unconditionally, so it can refuse what this policy permits —
    /// never the reverse.
    ///
    /// So `true` means *the decider authorises nothing*, which is the honest
    /// claim. Rendering it as "nothing can be authorised, ever, anywhere" is a
    /// claim about a file in another process, and LEARNINGS 23 records what
    /// happens when a boundary is described as covering more than it does.
    pub policy_closed: bool,
}

/// Builds the funnel from recorded decisions.
///
/// `launches` is passed rather than read so the caller can decide how expensive
/// a read to do; the funnel does not need every launch, only the count.
#[must_use]
pub fn funnel(decisions: &[Decision], launches: usize, as_of: u64, policy_closed: bool) -> Funnel {
    let examined = decisions.len();
    let proposed = decisions.iter().filter(|d| d.proposed()).count();
    let authorised = decisions.iter().filter(|d| d.would_have_traded()).count();

    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for reason in decisions.iter().flat_map(|d| d.reasons.iter()) {
        *counts.entry(reason.as_str()).or_default() += 1;
    }
    let mut reasons: Vec<ReasonCount> = counts
        .into_iter()
        .map(|(reason, count)| ReasonCount {
            reason: reason.to_owned(),
            count,
        })
        .collect();
    // Most common first, then by name so the order is total and a client
    // rendering it twice gets the same list.
    reasons.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.reason.cmp(&b.reason)));

    Funnel {
        as_of,
        stages: vec![
            Stage {
                name: "recorded",
                count: launches,
                detail: "token launches the recorder has seen",
            },
            Stage {
                name: "examined",
                count: examined,
                detail: "candidates a paid look was spent on; the cheaper filters \
                         refuse far more and are not recorded because they can be \
                         re-derived",
            },
            Stage {
                name: "proposed",
                count: proposed,
                detail: "candidates the strategy proposed acting on",
            },
            Stage {
                name: "authorised",
                count: authorised,
                detail: "proposals the risk kernel authorised",
            },
        ],
        reasons,
        policy_closed,
    }
}

/// Everything recorded about one token.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct TokenEvidence {
    /// The mint.
    pub mint: String,
    /// Every decision taken about it, oldest first.
    pub decisions: Vec<Decision>,
    /// Every price measurement, oldest first.
    pub measurements: Vec<Measurement>,
}

/// One outcome measurement, flattened for a client.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct Measurement {
    /// When it was taken.
    pub measured_at: u64,
    /// Price reads the figures below were computed from.
    ///
    /// **Not a fill count, and it over-counts.** The price windows it is folded
    /// across overlap by five of their six hours and the fold is
    /// `saturating_add`, so a fill inside the window is counted again on every
    /// hourly pass: it grows while nothing trades, and two measurements of the
    /// same token are not comparable
    /// ([LEARNINGS](https://github.com/hey-vera/radar/blob/main/LEARNINGS.md)
    /// entry 19, which invalidated the first runs of research 0017 and 0018).
    ///
    /// Renamed on the wire from `fills` so a client cannot render it under a
    /// label the number does not support. Use
    /// [`last_transfer_slot`](Self::last_transfer_slot) for "did anything
    /// change hands".
    #[serde(rename = "price_reads")]
    pub fills: u64,
    /// The last slot a transfer was observed, or `None` if none ever was.
    ///
    /// A `max`, so unlike [`fills`](Self::fills) it cannot be inflated by
    /// re-reading the same window. This is the field that answers whether the
    /// token still trades.
    pub last_transfer_slot: Option<u64>,
    /// Prices, scaled by `PRICE_SCALE`. Null where not measured, never zero.
    pub first_price: Option<u64>,
    /// The last price observed.
    pub last_price: Option<u64>,
    /// The highest.
    pub peak_price: Option<u64>,
    /// The lowest.
    pub trough_price: Option<u64>,
    /// The highest fill price **within the most recent price window**.
    ///
    /// Exposed because [`peak_price`](Self::peak_price) is folded from launch
    /// and so can only widen -- it says nothing about *when* the peak happened,
    /// which is precisely why research 0020 could not answer whether an exit
    /// rule helps. This is the same measurement without the fold.
    ///
    /// **The window overlaps.** It is six hours and the pass runs hourly, so a
    /// peak set five hours ago appears in six consecutive measurements. It is a
    /// bounded recent lookback, not the movement since the previous checkpoint,
    /// and reading it as the latter overstates how fresh a move is.
    ///
    /// `None` on every row written before the column existed, which is most of
    /// the store.
    pub window_peak_price: Option<u64>,
    /// The lowest fill price within the most recent price window. Same caveats.
    pub window_trough_price: Option<u64>,
    /// Volume-weighted average price across every observed fill.
    pub vwap: Option<u64>,
    /// Whether the token graduated, and when.
    pub graduated_at: Option<u64>,
    /// Return from first to last, in basis points, where both are known.
    pub held_to_end_bps: Option<i64>,
}

/// Reads everything recorded about one mint.
///
/// # Errors
///
/// Returns [`radar_store::StoreError`] if the store cannot be read.
pub fn token_evidence(
    reader: &Reader,
    mint: &str,
    as_of: AsOf,
) -> Result<TokenEvidence, radar_store::StoreError> {
    let decisions = reader
        .read_decisions(as_of)?
        .into_iter()
        .filter(|d| d.mint.to_string() == mint)
        .collect();
    let measurements = reader
        .read_outcomes(as_of)?
        .into_iter()
        .filter(|o| o.mint.to_string() == mint)
        .map(|o| Measurement {
            measured_at: o.measured_at.get(),
            fills: o.fills,
            last_transfer_slot: o.last_transfer_slot.map(radar_types::Slot::get),
            first_price: o.first_price,
            last_price: o.last_price,
            peak_price: o.peak_price,
            trough_price: o.trough_price,
            window_peak_price: o.window_peak_price,
            window_trough_price: o.window_trough_price,
            vwap: o.vwap,
            graduated_at: o.graduated_at.map(radar_types::Slot::get),
            held_to_end_bps: o.held_to_end_gain_bps(),
        })
        .collect();

    Ok(TokenEvidence {
        mint: mint.to_owned(),
        decisions,
        measurements,
    })
}

/// Where a page of the decision record ended, so the next one can continue.
///
/// # Why this is a pair and not a slot
///
/// `decided_at` is **not unique**, and not nearly. It is the watermark a
/// `radar consider` run was taken at, and every decision in that run carries the
/// same one — so a single value covers a whole batch, which at the observed rate
/// is tens of rows and over a backlog pass is thousands.
///
/// A cursor of "the last slot I saw" would therefore return one page and then
/// skip **every remaining decision at that slot**, silently, with a perfectly
/// plausible-looking result. That is the convenient default that loses data, and
/// this repository has a rule about those.
///
/// So the cursor is the pair `(decided_at, mint)`, which is unique because a
/// strategy records one decision per mint per run, and the ordering is by both.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Cursor {
    /// The watermark of the last decision on the previous page.
    pub decided_at: u64,
    /// Its mint, which is what breaks the tie.
    pub mint: String,
}

impl Cursor {
    /// Parses `<slot>:<mint>`.
    ///
    /// `None` for anything malformed rather than a partial read: a cursor that
    /// parsed half of itself would page through a different sequence than the
    /// one the caller was given, which is worse than starting over.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        let (slot, mint) = raw.split_once(':')?;
        if mint.is_empty() {
            return None;
        }
        Some(Self {
            decided_at: slot.parse().ok()?,
            mint: mint.to_owned(),
        })
    }

    /// Renders `<slot>:<mint>`.
    #[must_use]
    pub fn render(&self) -> String {
        format!("{}:{}", self.decided_at, self.mint)
    }
}

/// What a caller asked the decision record for.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Query {
    /// Continue after this point. `None` starts at the newest.
    pub after: Option<Cursor>,
    /// Only decisions carrying this reason, from either list.
    pub reason: Option<String>,
    /// Only `proposed` or only `passed`.
    pub conclusion: Option<Conclusion>,
    /// How many to return.
    pub limit: usize,
}

/// The largest page anyone may ask for.
///
/// A cap rather than a suggestion. The store is read whole to answer this, so an
/// unbounded `limit` costs the *client* nothing and hands it every decision
/// Radar has ever taken in one response.
pub const MAX_LIMIT: usize = 200;

/// The page size when a caller does not choose one.
pub const DEFAULT_LIMIT: usize = 50;

/// A page of the decision record, newest first.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct Page {
    /// The watermark this was read at.
    pub as_of: u64,
    /// The decisions, newest first.
    pub decisions: Vec<Decision>,
    /// The cursor to pass as `after` for the following page.
    ///
    /// `None` means this is the last page. Distinct from an empty `decisions`
    /// list, which can also happen on a filter that matches nothing.
    pub next: Option<String>,
    /// How many decisions matched the filter in total, across every page.
    ///
    /// Costs nothing extra: answering this endpoint reads the whole table
    /// regardless, so the count is already in hand. It is what lets a reader see
    /// that a reason accounts for four thousand refusals rather than the fifty
    /// in front of them.
    pub matched: usize,
}

/// Whether a decision carries a reason, from either list.
///
/// Both lists, deliberately. The strategy's reasons and the kernel's are
/// different vocabularies at different stages, and a reader filtering by
/// `CapacityBelowFloor` does not know or care which layer emitted it.
fn carries(decision: &Decision, reason: &str) -> bool {
    decision.reasons.iter().any(|r| r == reason)
        || decision.kernel_reasons.iter().any(|r| r == reason)
}

/// Sorts, filters and cuts one page out of the decision record.
///
/// Pure, and separate from the handler for the reason everything else in this
/// module is: the cursor arithmetic is the part with a wrong version that looks
/// right, and it needs no store to be tested.
///
/// **Newest first**, ordered by `(decided_at, mint)` descending. The mint is not
/// decoration — see [`Cursor`].
#[must_use]
pub fn page(mut decisions: Vec<Decision>, query: &Query, as_of: u64) -> Page {
    decisions.sort_by(|a, b| {
        b.decided_at
            .get()
            .cmp(&a.decided_at.get())
            .then_with(|| b.mint.to_string().cmp(&a.mint.to_string()))
    });

    let mut matched: Vec<Decision> = decisions
        .into_iter()
        .filter(|d| query.conclusion.is_none_or(|wanted| d.conclusion == wanted))
        .filter(|d| {
            query
                .reason
                .as_deref()
                .is_none_or(|reason| carries(d, reason))
        })
        .collect();

    let total = matched.len();

    // The cursor is applied *after* counting, so `matched` describes the whole
    // filtered set rather than the tail of it. A count that shrank as a reader
    // paged would be reporting on the pagination.
    if let Some(after) = query.after.as_ref() {
        matched.retain(|d| {
            (d.decided_at.get(), d.mint.to_string()) < (after.decided_at, after.mint.clone())
        });
    }

    let limit = query.limit.clamp(1, MAX_LIMIT);
    // One past the limit, so "is there another page" is answered by looking
    // rather than by comparing the page size against the limit -- which reports
    // a further page whenever the total is an exact multiple.
    let has_more = matched.len() > limit;
    matched.truncate(limit);

    let next = has_more.then(|| matched.last()).flatten().map(|last| {
        Cursor {
            decided_at: last.decided_at.get(),
            mint: last.mint.to_string(),
        }
        .render()
    });

    Page {
        as_of,
        decisions: matched,
        next,
        matched: total,
    }
}

/// The exit-capacity bands, in micro-USD, matching research 0018.
///
/// Fixed rather than computed from the data, and deliberately the **same** bands
/// the published note used. A chart whose buckets move with the data cannot be
/// compared against the note it is illustrating, and the reader would have no
/// way to tell a change in the market from a change in the histogram.
const BANDS: &[(u64, Option<u64>)] = &[
    (0, Some(25_000_000)),
    (25_000_000, Some(30_000_000)),
    (30_000_000, Some(35_000_000)),
    (35_000_000, Some(60_000_000)),
    (60_000_000, None),
];

/// One band of the exit-capacity distribution.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct Band {
    /// Inclusive floor, micro-USD.
    pub floor: u64,
    /// Exclusive ceiling, micro-USD. `None` on the open-ended top band.
    pub ceiling: Option<u64>,
    /// How many decisions measured a capacity in this band.
    pub decisions: usize,
}

/// The capacity wall: what the venue actually offers, and what Radar sized into
/// it.
///
/// The single most important picture in the product, because it is the reason
/// the thing does not work yet. 80% of proposals sit in a ±13% band around $31,
/// because every pre-graduation pump.fun token rides the same bonding curve —
/// capacity is closer to a property of the venue than of the token, and the
/// median position it produces is $6.21 against an 850 bps round trip.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct Capacity {
    /// The watermark this was read at.
    pub as_of: u64,
    /// The distribution.
    pub bands: Vec<Band>,
    /// Decisions where a capacity was measured.
    pub measured: usize,
    /// Decisions where it was **not**.
    ///
    /// Reported rather than bucketed. AGENTS.md rule 9: a capacity that could
    /// not be measured is `None`, and `None` means "cannot exit", never "no
    /// limit found" — and certainly not zero. Folding these into the bottom band
    /// would draw a wall of thin tokens that nobody measured.
    pub unmeasured: usize,
    /// The median measured capacity, micro-USD. `None` when nothing was
    /// measured.
    pub median_capacity: Option<u64>,
    /// The median proposed notional, micro-USD, over proposals that carried one.
    pub median_notional: Option<u64>,
    /// What a round trip is assumed to cost, per ten thousand.
    pub round_trip_bps: u64,
}

/// The median of a sorted-in-place list, or `None` when it is empty.
///
/// `None` rather than zero, for the reason every other absent figure in this
/// interface is: a median of nothing is not a median of zero.
fn median_of(mut values: Vec<u64>) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    values.get(values.len() / 2).copied()
}

/// Builds the capacity distribution from recorded decisions.
///
/// Pure, and separate from the handler for the usual reason: the bucketing has a
/// wrong version that looks right — every boundary is a place where a token can
/// land in the wrong band or in none at all — and it needs no store to test.
#[must_use]
pub fn capacity(decisions: &[Decision], as_of: u64, round_trip_bps: u64) -> Capacity {
    let mut bands: Vec<Band> = BANDS
        .iter()
        .map(|&(floor, ceiling)| Band {
            floor,
            ceiling,
            decisions: 0,
        })
        .collect();

    let mut measured = 0usize;
    let mut unmeasured = 0usize;
    let mut capacities: Vec<u64> = Vec::new();

    for decision in decisions {
        let Some(capacity) = decision.exit_capacity_micro_usd else {
            unmeasured += 1;
            continue;
        };
        measured += 1;
        capacities.push(capacity);
        // Half-open `[floor, ceiling)`, so a value exactly on a boundary lands in
        // exactly one band. Inclusive on both ends would double-count it and the
        // totals would quietly exceed `measured`.
        if let Some(band) = bands
            .iter_mut()
            .find(|b| capacity >= b.floor && b.ceiling.is_none_or(|c| capacity < c))
        {
            band.decisions += 1;
        }
    }

    let notionals: Vec<u64> = decisions
        .iter()
        .filter_map(|d| d.notional_micro_usd)
        .collect();

    Capacity {
        as_of,
        bands,
        measured,
        unmeasured,
        median_capacity: median_of(capacities),
        median_notional: median_of(notionals),
        round_trip_bps,
    }
}

/// The return buckets, in basis points.
///
/// Wide, and deliberately so. A histogram of memecoin returns with fine buckets
/// is a spike at zero and a long flat nothing; these are chosen so the shape a
/// reader needs — most of the mass is below break-even — is visible at a glance.
///
/// The floor is open-ended downward because a token can go to nothing, and the
/// ceiling is open-ended upward because one occasionally does not.
const RETURN_BUCKETS: &[(Option<i64>, Option<i64>)] = &[
    (None, Some(-5_000)),
    (Some(-5_000), Some(-2_000)),
    (Some(-2_000), Some(-850)),
    (Some(-850), Some(0)),
    // Zero is its own bucket and not a boundary. See `exactly_zero`.
    (Some(1), Some(850)),
    (Some(850), Some(2_000)),
    (Some(2_000), None),
];

/// One bucket of the return distribution.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct Bucket {
    /// Inclusive floor in bps. `None` is open-ended downward.
    pub floor: Option<i64>,
    /// Exclusive ceiling in bps. `None` is open-ended upward.
    pub ceiling: Option<i64>,
    /// How many scored decisions landed here.
    pub scored: usize,
}

/// What the selection actually returned, as a distribution rather than a median.
///
/// # Why a median is not enough, and is the thing to resist
///
/// Research 0017's central caveat: **24–43% of both cohorts return exactly
/// zero**. A median over a point mass that large is a report about the point
/// mass, not about the market — and a bare median of 0 must not be read as "the
/// two cohorts performed identically".
///
/// A histogram is the only rendering that shows that, which is why this endpoint
/// exists at all when `/v1/scoreboard` already reports a median.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct Returns {
    /// The watermark this was read at.
    pub as_of: u64,
    /// The distribution, excluding the exact zeroes.
    pub buckets: Vec<Bucket>,
    /// How many scored decisions returned **exactly** zero.
    ///
    /// Its own figure rather than a bucket, because it is the caveat rather than
    /// a feature of the distribution. A quarter to two-fifths of this population
    /// ends exactly where it started — most tokens here trade a handful of times
    /// and stop — and a bucket that quietly absorbed them would hide the one
    /// thing 0017 says carries the whole result.
    pub exactly_zero: usize,
    /// Scored decisions in total, including the zeroes.
    pub scored: usize,
    /// Decisions that could not be scored at all.
    ///
    /// A decision with no entry price cannot be scored, and that is every
    /// refusal that never reached the exit probe. Counted, never treated as a
    /// return of nothing.
    pub unscored: usize,
    /// What a round trip is assumed to cost, per ten thousand.
    pub round_trip_bps: u64,
}

/// Builds the return distribution from decisions and their outcomes.
///
/// Pure. The bucketing is the part with a wrong version that looks right, and it
/// needs no store.
///
/// **Gross.** Nothing here is net of the round trip; the figure is carried so a
/// caller can draw the line rather than move the data under it.
#[must_use]
pub fn returns(
    decisions: &[Decision],
    outcomes: &[radar_store::Outcome],
    as_of: u64,
    round_trip_bps: u64,
) -> Returns {
    // The last price observed for each mint. A decision is scored against what
    // the outcome pass last saw, which is the same instrument the scoreboard
    // uses -- and the same one 0016 warns is not the instrument the entry came
    // from.
    let mut last: BTreeMap<String, u64> = BTreeMap::new();
    for outcome in outcomes {
        if let Some(price) = outcome.last_price {
            last.insert(outcome.mint.to_string(), price);
        }
    }

    let mut buckets: Vec<Bucket> = RETURN_BUCKETS
        .iter()
        .map(|&(floor, ceiling)| Bucket {
            floor,
            ceiling,
            scored: 0,
        })
        .collect();

    let mut exactly_zero = 0usize;
    let mut scored = 0usize;
    let mut unscored = 0usize;

    for decision in decisions {
        let bps = last
            .get(&decision.mint.to_string())
            .and_then(|price| decision.return_bps(*price));
        let Some(bps) = bps else {
            unscored += 1;
            continue;
        };
        scored += 1;
        if bps == 0 {
            exactly_zero += 1;
            continue;
        }
        if let Some(bucket) = buckets
            .iter_mut()
            .find(|b| b.floor.is_none_or(|f| bps >= f) && b.ceiling.is_none_or(|c| bps < c))
        {
            bucket.scored += 1;
        }
    }

    Returns {
        as_of,
        buckets,
        exactly_zero,
        scored,
        unscored,
        round_trip_bps,
    }
}

/// How many rows each table holds, for the health screen.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Default)]
pub struct StoreCounts {
    /// Launches recorded.
    pub launches: usize,
    /// Graduations recorded.
    pub graduations: usize,
    /// Outcome measurements.
    pub outcomes: usize,
    /// Decisions recorded.
    pub decisions: usize,
}

/// Counts the store's contents at a watermark.
///
/// # Errors
///
/// Returns [`radar_store::StoreError`] if the store cannot be read.
pub fn store_counts(reader: &Reader, as_of: AsOf) -> Result<StoreCounts, radar_store::StoreError> {
    // `count` rather than `read(..).len()`. Decoding 167,987 launch events to
    // learn that there are 167,987 of them took ten seconds against the live
    // store; the row count is in each file's Parquet footer.
    Ok(StoreCounts {
        launches: reader.count(Table::Launches, as_of)?,
        graduations: reader.count(Table::Graduations, as_of)?,
        outcomes: reader.count(Table::Outcomes, as_of)?,
        decisions: reader.count(Table::Decisions, as_of)?,
    })
}

/// Whether a decision proposed acting.
///
/// Exposed because a client rendering the funnel wants the same predicate the
/// funnel counted with, rather than reimplementing it and drifting.
#[must_use]
pub const fn is_proposal(decision: &Decision) -> bool {
    matches!(decision.conclusion, Conclusion::Proposed)
}

/// The round-trip cost the scoreboard assumes.
///
/// 850 bps, measured. Research 0009 found fewer than one token in ten ever
/// finishes above it, which is the single most important number on the page and
/// the reason the scoreboard exists at all.
pub const ASSUMED_COST_BPS: u64 = 850;

/// The honest scoreboard: what Radar's selection returned against its own
/// refusals.
///
/// The comparison is the **matched control**, not a constant. An earlier version
/// of this compared Radar's cohort to research 0009's population median, and
/// those are different quantities: 0009 enters at the token's first fill and
/// Radar enters forty minutes later, and the constant itself moved from −1,340
/// to −863 bps as the cohort grew. Refusals are priced in the same passes, the
/// same way, over the same universe — which is what makes a difference between
/// them attributable to the selection rather than to the measurement.
///
/// # Errors
///
/// Returns [`radar_store::StoreError`] if the store cannot be read.
pub fn scoreboard(
    reader: &Reader,
    as_of: AsOf,
    cost_bps: u64,
) -> Result<radar_research::selection::Report, radar_store::StoreError> {
    let decisions = reader.read_decisions(as_of)?;
    let outcomes = reader.read_outcomes(as_of)?;
    Ok(radar_research::selection::evaluate(
        &decisions, &outcomes, cost_bps,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use radar_types::{Address, Slot};

    fn decision(mint: u8, proposed: bool, reasons: &[&str], authorised: bool) -> Decision {
        Decision {
            mint: Address::new([mint; 32]),
            creator: Address::new([9u8; 32]),
            decided_at: Slot(10_000),
            launch_slot: Slot(4_000),
            strategy: "creator_edge".to_owned(),
            strategy_version: "0.1.0".to_owned(),
            conclusion: if proposed {
                Conclusion::Proposed
            } else {
                Conclusion::Passed
            },
            reasons: reasons.iter().map(|r| (*r).to_owned()).collect(),
            notional_micro_usd: proposed.then_some(6_300_000),
            exit_capacity_micro_usd: None,
            assumed_round_trip_bps: 850,
            coordination: None,
            authority_prevalence: None,
            kernel_outcome: proposed.then_some(if authorised {
                radar_store::KernelOutcome::Authorised
            } else {
                radar_store::KernelOutcome::Refused
            }),
            kernel_reasons: Vec::new(),
            entry_price: None,
            inputs_digest: "d".to_owned(),
        }
    }

    #[test]
    fn the_funnel_narrows_and_never_widens() {
        // A stage that reported more than the one before it would be a bug in
        // the counting, and it is the kind a reader would believe.
        let decisions = vec![
            decision(1, true, &[], false),
            decision(2, false, &["NoRoute"], false),
            decision(3, false, &["CapacityBelowFloor"], false),
        ];
        let f = funnel(&decisions, 1_000, 500, true);

        let counts: Vec<usize> = f.stages.iter().map(|s| s.count).collect();
        assert_eq!(counts, vec![1_000, 3, 1, 0]);
        for pair in counts.windows(2) {
            assert!(pair[1] <= pair[0], "the funnel widened: {counts:?}");
        }
    }

    #[test]
    fn a_proposal_the_kernel_refused_is_not_counted_as_authorised() {
        // The distinction that keeps the last stage honest. Every proposal in
        // production is refused by Policy::CLOSED, so a funnel counting
        // proposals as authorised would report activity that never happened.
        let refused = vec![decision(1, true, &[], false)];
        let authorised = vec![decision(1, true, &[], true)];

        assert_eq!(funnel(&refused, 10, 1, true).stages[3].count, 0);
        assert_eq!(funnel(&authorised, 10, 1, false).stages[3].count, 1);
    }

    #[test]
    fn reasons_are_ranked_by_frequency_with_a_total_order() {
        // A client rendering this twice must get the same list. Sorting by count
        // alone leaves ties in whatever order the map yielded.
        let decisions = vec![
            decision(1, false, &["NoRoute"], false),
            decision(2, false, &["NoRoute"], false),
            decision(3, false, &["CapacityBelowFloor"], false),
            decision(4, false, &["AaaSameCount"], false),
        ];
        let f = funnel(&decisions, 10, 1, true);
        assert_eq!(f.reasons[0].reason, "NoRoute");
        assert_eq!(f.reasons[0].count, 2);
        assert_eq!(
            f.reasons[1].reason, "AaaSameCount",
            "ties break by name, so the order is total"
        );
        assert_eq!(f.reasons[2].reason, "CapacityBelowFloor");
    }

    #[test]
    fn a_decision_carrying_several_reasons_counts_towards_each() {
        // A candidate that failed four ways is more informative than one that
        // failed once, and the funnel should say so rather than picking one.
        let decisions = vec![decision(
            1,
            false,
            &["NoRoute", "CapacityBelowFloor"],
            false,
        )];
        let f = funnel(&decisions, 10, 1, true);
        assert_eq!(f.reasons.len(), 2);
        assert!(f.reasons.iter().all(|r| r.count == 1));
    }

    #[test]
    fn an_empty_store_reports_zeroes_rather_than_refusing() {
        // A fresh instance has recorded nothing, and that is a state the screen
        // has to render. It is distinguishable from a broken one by the stage
        // details, which are present either way.
        let f = funnel(&[], 0, 0, true);
        assert!(f.stages.iter().all(|s| s.count == 0));
        assert!(f.reasons.is_empty());
        assert_eq!(f.stages.len(), 4, "the stages are always described");
    }

    #[test]
    fn the_examined_stage_says_it_is_not_the_whole_funnel() {
        // The free tier refuses tens of thousands a day and records none of
        // them. A reader seeing "examined: 900" under "recorded: 170,618" would
        // otherwise conclude the other 169,718 were never looked at.
        let f = funnel(&[], 170_618, 1, true);
        let examined = &f.stages[1];
        assert!(
            examined.detail.contains("re-derived"),
            "the stage must explain what is missing: {}",
            examined.detail
        );
    }

    #[test]
    fn the_closed_policy_is_reported_as_one_fact() {
        // The single most important thing on the page. A raw refusal list buries
        // it under six other reasons that are the same fact restated.
        assert!(funnel(&[], 0, 0, true).policy_closed);
        assert!(!funnel(&[], 0, 0, false).policy_closed);
    }

    /// A decision at a chosen watermark, so ties can be built deliberately.
    fn at(mint: u8, slot: u64) -> Decision {
        Decision {
            decided_at: Slot(slot),
            ..decision(mint, false, &["CapacityBelowFloor"], false)
        }
    }

    /// Every page of a query, walked to exhaustion.
    ///
    /// Returns the mints in the order a reader would actually see them, which is
    /// what the cursor is for and what a per-page assertion cannot check.
    fn walk(all: &[Decision], limit: usize) -> Vec<(u64, String)> {
        let mut seen = Vec::new();
        let mut after = None;
        // Bounded rather than `loop`. A cursor that fails to advance is an
        // infinite loop, and a test that hangs says less than one that fails.
        for _ in 0..100 {
            let query = Query {
                after: after.clone(),
                limit,
                ..Query::default()
            };
            let got = page(all.to_vec(), &query, 1);
            // The pair, not the mint. A mint is reconsidered on later runs, so
            // it recurs legitimately across watermarks -- and an identity that
            // collapsed those would report a cursor bug that was not there.
            seen.extend(
                got.decisions
                    .iter()
                    .map(|d| (d.decided_at.get(), d.mint.to_string())),
            );
            match got.next {
                None => return seen,
                Some(raw) => {
                    after = Some(Cursor::parse(&raw).expect("a cursor it just rendered"));
                }
            }
        }
        panic!("the cursor never reached the end");
    }

    #[test]
    fn paging_over_a_single_watermark_returns_every_row_exactly_once() {
        // The case this whole design exists for.
        //
        // `decided_at` is the watermark a `radar consider` run was taken at, and
        // every decision in that run shares it -- so a realistic page is all
        // ties. A cursor of "the last slot I saw" returns one page and then skips
        // every remaining row at that slot, silently, with a result that looks
        // entirely plausible.
        let all: Vec<Decision> = (1..=25u8).map(|m| at(m, 10_000)).collect();

        let seen = walk(&all, 10);
        assert_eq!(seen.len(), 25, "every row is returned");

        let unique: std::collections::BTreeSet<_> = seen.iter().collect();
        assert_eq!(unique.len(), 25, "and none of them twice");
    }

    #[test]
    fn paging_across_several_watermarks_holds_the_order() {
        // Ties and distinct slots together, which is what the real store looks
        // like: hourly batches, each internally tied.
        let mut all = Vec::new();
        for slot in [10_000u64, 10_100, 10_200] {
            for mint in 1..=7u8 {
                all.push(at(mint, slot));
            }
        }
        let seen = walk(&all, 4);
        assert_eq!(seen.len(), 21);
        assert_eq!(
            seen.iter().collect::<std::collections::BTreeSet<_>>().len(),
            21,
            "no repeats across pages"
        );
    }

    #[test]
    fn the_newest_decision_comes_first() {
        let all = vec![at(1, 10_000), at(2, 10_500), at(3, 9_000)];
        let got = page(
            all,
            &Query {
                limit: 10,
                ..Query::default()
            },
            1,
        );
        let slots: Vec<u64> = got.decisions.iter().map(|d| d.decided_at.get()).collect();
        assert_eq!(slots, vec![10_500, 10_000, 9_000]);
    }

    #[test]
    fn there_is_no_next_page_when_the_total_is_an_exact_multiple_of_the_limit() {
        // The off-by-one that hands a reader an empty final page. Comparing the
        // returned count against the limit reports "more" whenever they are
        // equal, which for an exact multiple is always.
        let all: Vec<Decision> = (1..=10u8).map(|m| at(m, 10_000)).collect();
        let got = page(
            all,
            &Query {
                limit: 10,
                ..Query::default()
            },
            1,
        );
        assert_eq!(got.decisions.len(), 10);
        assert!(got.next.is_none(), "ten of ten is the last page");
    }

    #[test]
    fn the_match_count_describes_the_filter_and_not_the_page() {
        // A count that shrank as a reader paged would be reporting on the
        // pagination. The point of this figure is to say that a reason accounts
        // for four thousand refusals rather than the fifty in front of you.
        let all: Vec<Decision> = (1..=30u8).map(|m| at(m, 10_000)).collect();
        let first = page(
            all.clone(),
            &Query {
                limit: 5,
                ..Query::default()
            },
            1,
        );
        assert_eq!(first.matched, 30);

        let after = Cursor::parse(&first.next.expect("more pages")).expect("parses");
        let second = page(
            all,
            &Query {
                after: Some(after),
                limit: 5,
                ..Query::default()
            },
            1,
        );
        assert_eq!(second.matched, 30, "unchanged by paging");
    }

    #[test]
    fn a_reason_filter_reads_both_lists() {
        // The strategy's reasons and the kernel's are different vocabularies at
        // different stages, and a reader filtering by a reason code does not know
        // or care which layer emitted it.
        let mut kernel = at(1, 10_000);
        kernel.reasons.clear();
        kernel.kernel_reasons = vec!["OverPositionLimit".to_owned()];

        let all = vec![kernel, at(2, 10_000)];

        let from_kernel = page(
            all.clone(),
            &Query {
                reason: Some("OverPositionLimit".to_owned()),
                limit: 10,
                ..Query::default()
            },
            1,
        );
        assert_eq!(from_kernel.matched, 1);

        let from_strategy = page(
            all,
            &Query {
                reason: Some("CapacityBelowFloor".to_owned()),
                limit: 10,
                ..Query::default()
            },
            1,
        );
        assert_eq!(from_strategy.matched, 1);
    }

    #[test]
    fn a_conclusion_filter_separates_the_two() {
        let all = vec![
            decision(1, true, &[], false),
            decision(2, false, &["CapacityBelowFloor"], false),
        ];
        let proposed = page(
            all.clone(),
            &Query {
                conclusion: Some(Conclusion::Proposed),
                limit: 10,
                ..Query::default()
            },
            1,
        );
        assert_eq!(proposed.matched, 1);
        assert!(proposed.decisions[0].proposed());

        let passed = page(
            all,
            &Query {
                conclusion: Some(Conclusion::Passed),
                limit: 10,
                ..Query::default()
            },
            1,
        );
        assert_eq!(passed.matched, 1);
        assert!(!passed.decisions[0].proposed());
    }

    #[test]
    fn a_limit_is_clamped_rather_than_honoured() {
        // The store is read whole to answer this, so an unbounded limit costs the
        // caller nothing and hands it every decision Radar has ever taken.
        let all: Vec<Decision> = (1..=250u8).map(|m| at(m, 10_000)).collect();
        let huge = page(
            all.clone(),
            &Query {
                limit: 100_000,
                ..Query::default()
            },
            1,
        );
        assert_eq!(huge.decisions.len(), MAX_LIMIT);

        // And zero is not a page size. Without the lower clamp it returns nothing
        // forever, with a cursor that never advances.
        let none = page(
            all,
            &Query {
                limit: 0,
                ..Query::default()
            },
            1,
        );
        assert_eq!(none.decisions.len(), 1);
    }

    #[test]
    fn a_cursor_round_trips_and_refuses_what_it_cannot_read() {
        let cursor = Cursor {
            decided_at: 441_734_987,
            mint: "So11111111111111111111111111111111111111112".to_owned(),
        };
        assert_eq!(Cursor::parse(&cursor.render()), Some(cursor));

        // Half-parsed is worse than not parsed: it pages through a different
        // sequence than the caller was handed.
        for bad in ["", "123", "notaslot:mint", "123:", ":mint"] {
            assert!(Cursor::parse(bad).is_none(), "{bad} must not parse");
        }
    }

    #[test]
    fn an_empty_store_is_an_empty_page_rather_than_a_missing_one() {
        let got = page(
            Vec::new(),
            &Query {
                limit: 10,
                ..Query::default()
            },
            7,
        );
        assert!(got.decisions.is_empty());
        assert_eq!(got.matched, 0);
        assert!(got.next.is_none());
        assert_eq!(got.as_of, 7, "the watermark is reported even with no rows");
    }

    /// A decision with a measured exit capacity.
    fn with_capacity(mint: u8, capacity: Option<u64>, notional: Option<u64>) -> Decision {
        Decision {
            exit_capacity_micro_usd: capacity,
            notional_micro_usd: notional,
            ..decision(mint, notional.is_some(), &[], false)
        }
    }

    #[test]
    fn an_unmeasured_capacity_is_counted_apart_rather_than_as_zero() {
        // Rule 9, and the version of it that would draw a false picture: a
        // capacity that could not be measured means "cannot exit", and folding
        // it into the bottom band draws a wall of thin tokens nobody measured.
        let all = vec![
            with_capacity(1, None, None),
            with_capacity(2, None, None),
            with_capacity(3, Some(31_000_000), None),
        ];
        let got = capacity(&all, 1, 850);

        assert_eq!(got.unmeasured, 2);
        assert_eq!(got.measured, 1);
        let bottom = got.bands.first().expect("a bottom band");
        assert_eq!(bottom.decisions, 0, "the unmeasured must not land here");
    }

    #[test]
    fn every_measured_decision_lands_in_exactly_one_band() {
        // The totals are the check. A boundary that is inclusive on both sides
        // double-counts, and one that is exclusive on both drops a value
        // silently -- and both produce a histogram that looks entirely normal.
        let all: Vec<Decision> = [
            0,
            24_999_999,
            25_000_000,
            29_999_999,
            30_000_000,
            34_999_999,
            35_000_000,
            59_999_999,
            60_000_000,
            618_400_000,
        ]
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            let mint = u8::try_from(i + 1).expect("small");
            with_capacity(mint, Some(c), None)
        })
        .collect();

        let got = capacity(&all, 1, 850);
        let binned: usize = got.bands.iter().map(|b| b.decisions).sum();
        assert_eq!(binned, got.measured, "every measured value is binned once");
        assert_eq!(got.measured, 10);
    }

    #[test]
    fn the_boundaries_fall_where_the_research_put_them() {
        // Swept individually, because "every value is binned once" holds just as
        // well when they are all binned into the *wrong* band.
        let at = |c: u64| {
            let got = capacity(&[with_capacity(1, Some(c), None)], 1, 850);
            got.bands
                .iter()
                .position(|b| b.decisions == 1)
                .expect("binned somewhere")
        };
        assert_eq!(at(24_999_999), 0);
        assert_eq!(at(25_000_000), 1, "the floor belongs to the band above");
        assert_eq!(at(29_999_999), 1);
        assert_eq!(at(30_000_000), 2);
        assert_eq!(at(34_999_999), 2);
        assert_eq!(at(35_000_000), 3);
        assert_eq!(at(59_999_999), 3);
        assert_eq!(at(60_000_000), 4, "the open-ended band");
        assert_eq!(at(u64::MAX), 4, "which really is open-ended");
    }

    #[test]
    fn the_medians_are_absent_rather_than_zero_when_nothing_was_measured() {
        let got = capacity(&[with_capacity(1, None, None)], 1, 850);
        assert_eq!(got.median_capacity, None);
        assert_eq!(got.median_notional, None);
    }

    #[test]
    fn the_notional_median_ignores_refusals_that_proposed_nothing() {
        // A refusal that never reached the exit probe has no notional, and
        // treating that as a proposal of zero drags the median to the floor --
        // which is the figure the whole chart exists to report.
        let all = vec![
            with_capacity(1, Some(31_000_000), None),
            with_capacity(2, Some(31_000_000), Some(6_210_000)),
            with_capacity(3, Some(31_000_000), Some(6_210_000)),
        ];
        let got = capacity(&all, 1, 850);
        assert_eq!(got.median_notional, Some(6_210_000));
    }

    #[test]
    fn an_empty_store_reports_a_shape_rather_than_nothing() {
        // The bands still exist with zero counts. A chart handed an empty list
        // should draw an empty chart, not fail to draw axes.
        let got = capacity(&[], 7, 850);
        assert_eq!(got.bands.len(), BANDS.len());
        assert_eq!(got.measured, 0);
        assert_eq!(got.unmeasured, 0);
        assert_eq!(got.as_of, 7);
        assert_eq!(got.round_trip_bps, 850);
    }

    fn outcome(mint: u8, last_price: Option<u64>) -> radar_store::Outcome {
        radar_store::Outcome {
            mint: Address::new([mint; 32]),
            measured_at: Slot(10_000),
            launch_slot: Slot(4_000),
            first_transfer_slot: None,
            last_transfer_slot: None,
            transfers: 0,
            unique_senders: 0,
            unique_receivers: 0,
            graduated_at: None,
            first_price: None,
            last_price,
            peak_price: None,
            trough_price: None,
            window_peak_price: None,
            window_trough_price: None,
            vwap: None,
            fills: 0,
        }
    }

    /// A decision priced at `entry`, so a later price produces a known return.
    fn priced(mint: u8, entry: u64) -> Decision {
        Decision {
            entry_price: Some(entry),
            ..decision(mint, true, &[], false)
        }
    }

    #[test]
    fn an_exact_zero_is_its_own_figure_and_not_a_bucket() {
        // 0017's central caveat. A quarter to two-fifths of this population ends
        // exactly where it started, and a bucket that absorbed them would hide
        // the one thing the note says carries the whole result -- a median over
        // that point mass is a report about the point mass.
        let all = vec![priced(1, 100), priced(2, 100), priced(3, 100)];
        let seen = vec![
            outcome(1, Some(100)), // 0 bps
            outcome(2, Some(100)), // 0 bps
            outcome(3, Some(50)),  // -5000 bps
        ];
        let got = returns(&all, &seen, 1, 850);

        assert_eq!(got.exactly_zero, 2);
        assert_eq!(got.scored, 3);
        let binned: usize = got.buckets.iter().map(|b| b.scored).sum();
        assert_eq!(binned, 1, "the zeroes are not in any bucket");
    }

    #[test]
    fn a_decision_with_no_entry_price_is_unscored_rather_than_flat() {
        // A decision that never reached the exit probe has no entry, so it
        // cannot be scored. Reporting it as zero would fold "not measurable"
        // into "broke even" -- which is the whole population's median dressed up
        // as a result.
        let all = vec![decision(1, false, &["CapacityBelowFloor"], false)];
        let got = returns(&all, &[outcome(1, Some(100))], 1, 850);
        assert_eq!(got.unscored, 1);
        assert_eq!(got.scored, 0);
        assert_eq!(got.exactly_zero, 0);
    }

    #[test]
    fn a_decision_whose_token_was_never_priced_is_unscored_too() {
        // Both halves are required. An entry with no exit is as unscoreable as
        // an exit with no entry, and only one of them is obvious.
        let got = returns(&[priced(1, 100)], &[outcome(1, None)], 1, 850);
        assert_eq!(got.unscored, 1);
        assert_eq!(got.scored, 0);
    }

    #[test]
    fn every_scored_return_lands_in_exactly_one_bucket() {
        // The totals are the check, as with the capacity bands: inclusive on
        // both sides double-counts, exclusive on both drops a value, and both
        // draw a histogram that looks entirely normal.
        //
        // Entry 100, so a last price of `p` is `(p - 100) * 100` bps.
        let prices = [1u64, 20, 50, 80, 95, 101, 110, 130, 200, 500];
        let all: Vec<Decision> = (1..=10u8).map(|m| priced(m, 100)).collect();
        let seen: Vec<radar_store::Outcome> = prices
            .iter()
            .enumerate()
            .map(|(i, &p)| {
                let mint = u8::try_from(i + 1).expect("small");
                outcome(mint, Some(p))
            })
            .collect();

        let got = returns(&all, &seen, 1, 850);
        let binned: usize = got.buckets.iter().map(|b| b.scored).sum();
        assert_eq!(binned + got.exactly_zero, got.scored);
        assert_eq!(got.scored, 10);
    }

    #[test]
    fn the_deepest_loss_and_the_largest_gain_are_both_open_ended() {
        // A token can go to nothing, and occasionally one does not. A bounded
        // bucket at either end silently drops the rows that matter most.
        let all = vec![priced(1, 1_000_000), priced(2, 1)];
        let seen = vec![outcome(1, Some(1)), outcome(2, Some(1_000_000))];
        let got = returns(&all, &seen, 1, 850);

        assert_eq!(got.buckets.first().expect("a floor bucket").scored, 1);
        assert_eq!(got.buckets.last().expect("a ceiling bucket").scored, 1);
        assert_eq!(got.scored, 2);
    }

    #[test]
    fn an_empty_store_still_reports_the_return_buckets() {
        let got = returns(&[], &[], 7, 850);
        assert_eq!(got.buckets.len(), RETURN_BUCKETS.len());
        assert_eq!(got.scored, 0);
        assert_eq!(got.exactly_zero, 0);
        assert_eq!(got.as_of, 7);
    }
}
