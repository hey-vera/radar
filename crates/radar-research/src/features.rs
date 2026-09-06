// SPDX-License-Identifier: Apache-2.0
//! E1 — one row per launch, every value observed at or before T.
//!
//! The walk-forward protocol ([`edge`](crate::edge)) needs a table it can fit
//! strata on. This builds it, and the only thing that makes it worth building
//! is the guarantee in the title: **a feature cannot carry information from
//! after the moment it claims to be measured at.** Design 0010 §6.2 says why a
//! fold design cannot catch a leak — a leaked feature does not fail on the
//! second fold, it wins every fold — so the guard has to sit earlier, at the
//! point the number enters the row.
//!
//! # The guard
//!
//! Every feature value arrives as an [`Observed<f64>`] carrying the slot of the
//! **latest input it was computed from**, and enters the row through
//! [`AsOf::accept`]. A value observed after T is a [`LookAhead`] error and never
//! a number. That is [`radar_asof`]'s two idle types getting their first real
//! caller, which design 0010 §8.1 row 3 made a condition of keeping them.
//!
//! The filters below already exclude late inputs on purpose — a trade after T is
//! skipped, an outcome measured after T is not counted. The `accept` is not a
//! second copy of that intent; it is the assertion that the intent held. A
//! filter that is wrong by one comparison is the whole failure, and it is
//! invisible in a diff. Here it is a build error naming the mint.
//!
//! # Labels are from the future, deliberately, and do not go through the guard
//!
//! A label is what happened next. It is read from outcomes measured *after* T
//! and it must be, or there is nothing to predict. So labels are set through a
//! separate path that takes no watermark, and they live in their own fields
//! rather than in [`Row::values`] — the two can never be confused by an index,
//! which is what would happen if a label were "just another column".
//!
//! # What is absent, and what is zero
//!
//! Rule 9. A creator with no prior launches has `creator_prior_launches = 0`,
//! because zero is the measurement. A launch whose dev buy the decoder could
//! not recover has `dev_buy_lamports = None`, because nobody measured it. A
//! stratum that names an absent feature does not include the row rather than
//! reading the absence as a small number.
//!
//! # Cost
//!
//! [`build`] reads the launches, trades, outcomes and decisions tables in full
//! at the watermark, the way [`creator_index`](crate::creator_index) does, and
//! holds the trades it needs in memory. The trades table is the large one, so
//! the window arguments are not a convenience: on a production store, run this
//! over the fold you are about to fit rather than over all of history at once.

use std::collections::{BTreeMap, BTreeSet};

use radar_asof::{AsOf, LookAhead, Observed};
use radar_store::{Event, GraduationMode, Outcome, Reader, Side, StoreError, Table, Trade};
use radar_types::{Address, Slot, SlotDelta};

/// Slots between a launch and T, the moment every feature is observed at.
///
/// The strategy's own `max_token_age`: the point past which
/// [`CreatorEdge`](radar_strategy::CreatorEdge) stops considering a token at
/// all. Measuring features at a later moment than the strategy would ever act
/// at would fit a rule nothing could trade;
/// `the_entry_offset_is_the_strategys_own_age_limit` holds the two together.
pub const ENTRY_OFFSET_SLOTS: u64 = 6_000;

/// Slots in an hour, at the 150-a-minute figure [`SlotDelta`] uses.
const SLOTS_PER_HOUR: u64 = 9_000;

/// The feature names, index-aligned with [`Row::values`].
///
/// A flat list rather than a struct with twenty-three fields because the
/// consumer enumerates thresholds over *all* features and never names one: a
/// struct would force a match arm per feature in the strata grammar, and adding
/// a feature would then be a change in two crates instead of one line here.
pub const FEATURES: &[&str] = &[
    // The launch block, from the store's own record of it.
    "launch_traders",
    "launch_transactions",
    "launch_contiguity",
    "dev_buy_lamports",
    // The creator's record as it stood at T, counted from measurements taken by
    // then — the pivot discipline `study` keeps, applied per row.
    "creator_prior_launches",
    "creator_prior_organic",
    "creator_prior_instant",
    "creator_prior_stillborn",
    "creator_launches_per_day",
    // Early activity, at three widths.
    "trades_25",
    "traders_25",
    "trades_300",
    "traders_300",
    "trades_6000",
    "traders_6000",
    // Liquidity velocity: how many trades it took to move the curve. The
    // strongest single predictor in arXiv 2602.14860 (design 0010 §7.2 d).
    "trades_to_10_sol",
    "trades_to_20_sol",
    "trades_to_30_sol",
    // The shape of a factory.
    "prior_same_name",
    "prior_same_symbol",
    "prior_same_uri_host",
    // What the decision lane recorded, where it decided this mint by T.
    "decision_launch_recipients",
    "decision_launch_transactions",
    "decision_authority_prevalence",
];

/// Index of a feature by name, for callers that want to name one.
///
/// # Panics
///
/// Never for a name in [`FEATURES`]; returns `None` otherwise.
#[must_use]
pub fn feature_index(name: &str) -> Option<usize> {
    FEATURES.iter().position(|f| *f == name)
}

/// One launch, observed at T, with what happened next.
#[derive(Clone, PartialEq, Debug)]
pub struct Row {
    /// The token.
    pub mint: Address,
    /// Its creator.
    pub creator: Address,
    /// The slot it launched in.
    pub launch_slot: Slot,
    /// T — the moment every value in [`values`](Self::values) was observed at
    /// or before.
    pub t: Slot,
    /// The features, index-aligned with [`FEATURES`]. `None` is "not measured",
    /// never zero.
    pub values: Vec<Option<f64>>,
    /// Gross return from the entry checkpoint to the six-hour checkpoint, in
    /// basis points. `None` when either checkpoint is missing a price.
    pub gross_6h_bps: Option<f64>,
    /// Gross return from the entry checkpoint to the twenty-four-hour
    /// checkpoint, in basis points.
    pub gross_24h_bps: Option<f64>,
    /// How the token graduated, as measured by the latest outcome at the
    /// watermark. A label: it is read from the future and is not a feature.
    pub mode: Option<GraduationMode>,
}

impl Row {
    /// The value of a feature by index, or `None` when it was not measured.
    #[must_use]
    pub fn value(&self, index: usize) -> Option<f64> {
        self.values.get(index).copied().flatten()
    }
}

/// Every row, with the watermark it was built at.
#[derive(Clone, PartialEq, Debug)]
pub struct FeatureTable {
    /// The store watermark this was built at. Part of the output's name, so two
    /// tables from different watermarks cannot be confused.
    pub watermark: Slot,
    /// The offset from launch to T, in slots.
    pub entry_offset: u64,
    /// The rows, in ascending launch-slot order. Ascending because the folds
    /// are contiguous windows by launch slot and shuffling is the one thing
    /// the protocol forbids.
    pub rows: Vec<Row>,
}

/// What stopped a table being built.
#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    /// The store could not be read.
    #[error("cannot read the store: {0}")]
    Store(#[from] StoreError),
    /// A feature carried a slot from after T. Always a bug in this module: a
    /// filter let a late input through.
    #[error("{mint} feature {feature}: {source}")]
    LookAhead {
        /// The token whose row was being built.
        mint: Address,
        /// The feature that carried the late value.
        feature: &'static str,
        /// The watermark and the observation that broke it.
        source: LookAhead,
    },
}

/// Accumulates one row's features, refusing anything observed after T.
///
/// Separate from [`Row`] so that the only way to set a value is through the
/// watermark. A public field would make the guard advisory.
struct RowBuilder {
    t: AsOf,
    values: Vec<Option<f64>>,
}

impl RowBuilder {
    fn new(t: Slot) -> Self {
        Self {
            t: AsOf::at(t),
            values: vec![None; FEATURES.len()],
        }
    }

    /// Records a feature, or refuses it as look-ahead.
    ///
    /// # Errors
    ///
    /// [`LookAhead`] when the observation is from after T.
    fn set(&mut self, index: usize, observed: Observed<f64>) -> Result<(), LookAhead> {
        let value = self.t.accept(observed)?;
        self.values[index] = Some(value);
        Ok(())
    }
}

/// Builds the feature table.
///
/// `as_of` is the store watermark. `from` and `to` bound the **launch slot** of
/// the rows produced, which is how a caller keeps one fold's worth of trades in
/// memory instead of all of history.
///
/// # Errors
///
/// [`BuildError::Store`] when a table cannot be read, [`BuildError::LookAhead`]
/// when a feature carried a slot from after its own T — which is a bug here,
/// not bad data.
pub fn build(
    reader: &Reader,
    as_of: AsOf,
    from: Slot,
    to: Slot,
) -> Result<FeatureTable, BuildError> {
    let launches = reader.read(Table::Launches, as_of)?;
    let outcomes = reader.read_outcomes(as_of)?;
    let decisions = reader.read_decisions(as_of)?;

    // Every succeeded launch, by mint. Failed launches are not launches
    // (`creator_index` says why), and a duplicate is the same launch recorded
    // twice, so the first wins.
    let mut by_mint: BTreeMap<Address, radar_store::Launch> = BTreeMap::new();
    for event in launches {
        let Event::Launch(launch) = event else {
            continue;
        };
        if !launch.envelope.succeeded {
            continue;
        }
        by_mint.entry(launch.mint).or_insert(*launch);
    }

    // Which mints this pass will produce a row for, and the trade window each
    // one needs. Computed before the trades are read so the filter below is a
    // lookup rather than a scan.
    let considered: BTreeSet<Address> = by_mint
        .values()
        .filter(|l| l.envelope.slot >= from && l.envelope.slot <= to)
        .map(|l| l.mint)
        .collect();

    let trades = trades_to_t(reader, as_of, &by_mint, &considered)?;
    let coverage = trade_coverage(reader)?;

    // Outcomes per mint, ascending by measurement. The row builder walks this
    // twice — once bounded by T for the creator's record, once unbounded for
    // the labels — and the bound is the only difference between them.
    let mut per_mint: BTreeMap<Address, Vec<&Outcome>> = BTreeMap::new();
    for outcome in &outcomes {
        per_mint.entry(outcome.mint).or_default().push(outcome);
    }
    for series in per_mint.values_mut() {
        series.sort_by_key(|o| o.measured_at);
    }

    // The creator's launches, ascending. A creator's record at T is counted
    // over these, from measurements taken by T.
    let mut by_creator: BTreeMap<Address, Vec<(Slot, Address)>> = BTreeMap::new();
    for launch in by_mint.values() {
        by_creator
            .entry(launch.creator)
            .or_default()
            .push((launch.envelope.slot, launch.mint));
    }
    for series in by_creator.values_mut() {
        series.sort_unstable();
    }

    let names = index_strings(by_mint.values().map(|l| (l.name.as_str(), l.envelope.slot)));
    let symbols = index_strings(
        by_mint
            .values()
            .map(|l| (l.symbol.as_str(), l.envelope.slot)),
    );
    let hosts = index_strings(
        by_mint
            .values()
            .filter_map(|l| uri_host(&l.uri).map(|h| (h, l.envelope.slot))),
    );

    // The latest decision per mint taken at or before that mint's T. A decision
    // taken later is not a fact about T, and absent is not zero.
    let mut decision_by_mint: BTreeMap<Address, &radar_store::Decision> = BTreeMap::new();
    for decision in &decisions {
        let Some(launch) = by_mint.get(&decision.mint) else {
            continue;
        };
        if decision.decided_at > launch.envelope.slot + SlotDelta(ENTRY_OFFSET_SLOTS) {
            continue;
        }
        decision_by_mint
            .entry(decision.mint)
            .and_modify(|held| {
                if decision.decided_at > held.decided_at {
                    *held = decision;
                }
            })
            .or_insert(decision);
    }

    let mut rows = Vec::new();
    for mint in &considered {
        let launch = &by_mint[mint];
        rows.push(build_row(&Inputs {
            launch,
            trades: trades.get(mint).map_or(&[][..], Vec::as_slice),
            trades_recorded: covered(
                &coverage,
                launch.envelope.slot,
                launch.envelope.slot + SlotDelta(ENTRY_OFFSET_SLOTS),
            ),
            per_mint: &per_mint,
            by_creator: &by_creator,
            names: &names,
            symbols: &symbols,
            hosts: &hosts,
            decision: decision_by_mint.get(mint).copied(),
        })?);
    }
    rows.sort_by_key(|r| (r.launch_slot, r.mint));

    Ok(FeatureTable {
        watermark: as_of.slot(),
        entry_offset: ENTRY_OFFSET_SLOTS,
        rows,
    })
}

/// The partitions the trades table actually holds, by partition index.
///
/// # Why a trade feature is not a measurement everywhere
///
/// A launch with no recorded trades reads as zero traders, zero transactions
/// and a contiguity of zero — and that is a measurement **only if the store
/// records trades for that window**. On 2026-09-05 the production store's
/// trades directory was empty: created 2026-08-23 and never written to. Every
/// trade-derived feature would have been a confident zero about 520,000
/// launches, which is rule 9's exact failure and the one this module exists to
/// refuse.
///
/// So coverage is read from the table's own partition files. A launch whose
/// window is not wholly inside the partitions the store holds gets **absent**
/// trade features, and a stratum naming one drops the row rather than reading
/// the absence as a quiet market.
fn trade_coverage(reader: &Reader) -> Result<BTreeSet<u64>, StoreError> {
    Ok(reader
        .files(Table::Trades)?
        .iter()
        .filter_map(|path| Reader::partition_range(path))
        .map(|(start, _)| radar_store::partition_of(start))
        .collect())
}

/// Whether the trades table covers every partition from `from` to `to`.
///
/// The whole window or none of it. A window half-covered would produce a count
/// that is right about one half and silent about the other, which is worse than
/// absent because it looks like a number.
fn covered(coverage: &BTreeSet<u64>, from: Slot, to: Slot) -> bool {
    (radar_store::partition_of(from)..=radar_store::partition_of(to))
        .all(|partition| coverage.contains(&partition))
}

/// The trades of every considered mint, from its launch to its T, in order.
///
/// The window is the memory bound: the trades table is the large read, and
/// holding a mint's whole history when only the first 6,000 slots are ever
/// looked at is the difference between a pass that fits and one that does not.
/// A trade before the launch slot is a recording artefact, not history.
fn trades_to_t(
    reader: &Reader,
    as_of: AsOf,
    by_mint: &BTreeMap<Address, radar_store::Launch>,
    considered: &BTreeSet<Address>,
) -> Result<BTreeMap<Address, Vec<Trade>>, StoreError> {
    let mut trades: BTreeMap<Address, Vec<Trade>> = BTreeMap::new();
    for event in reader.read(Table::Trades, as_of)? {
        let Event::Trade(trade) = event else {
            continue;
        };
        if !considered.contains(&trade.mint) {
            continue;
        }
        let Some(launch) = by_mint.get(&trade.mint) else {
            continue;
        };
        let t = launch.envelope.slot + SlotDelta(ENTRY_OFFSET_SLOTS);
        if trade.envelope.slot < launch.envelope.slot || trade.envelope.slot > t {
            continue;
        }
        trades.entry(trade.mint).or_default().push(*trade);
    }
    for series in trades.values_mut() {
        series.sort_by_key(|t| {
            (
                t.envelope.slot,
                t.envelope.tx_index,
                t.envelope.instruction_index,
            )
        });
    }
    Ok(trades)
}

/// Ascending launch slots per distinct string, for the "how many before this
/// one shared it" counts.
fn index_strings<'a>(items: impl Iterator<Item = (&'a str, Slot)>) -> BTreeMap<String, Vec<Slot>> {
    let mut out: BTreeMap<String, Vec<Slot>> = BTreeMap::new();
    for (key, slot) in items {
        out.entry(key.to_owned()).or_default().push(slot);
    }
    for slots in out.values_mut() {
        slots.sort_unstable();
    }
    out
}

/// How many entries strictly precede `slot`.
fn prior_count(slots: &[Slot], slot: Slot) -> f64 {
    let cut = slots.partition_point(|s| *s < slot);
    // Exact to 2^53, and every count here is orders below that.
    #[expect(clippy::cast_precision_loss, reason = "counts, far below 2^53")]
    {
        cut as f64
    }
}

/// The host of a metadata URI, without fetching it.
///
/// Text handling only. Rule 4: a stranger's mint never chooses Radar's
/// outbound requests, and nothing here makes one.
fn uri_host(uri: &str) -> Option<&str> {
    let rest = uri.split_once("://")?.1;
    let host = rest.split(['/', '?', '#']).next()?;
    (!host.is_empty()).then_some(host)
}

/// The latest measurement at or before `bound`, if any.
fn latest_by<'a>(series: &[&'a Outcome], bound: Slot) -> Option<&'a Outcome> {
    series
        .iter()
        .take_while(|o| o.measured_at <= bound)
        .last()
        .copied()
}

/// The first measurement at or after `bound`, if any.
fn first_from<'a>(series: &[&'a Outcome], bound: Slot) -> Option<&'a Outcome> {
    series.iter().find(|o| o.measured_at >= bound).copied()
}

/// One row's inputs.
///
/// A struct of borrows rather than eight arguments: the eight are the same
/// either way, and this lets each group of features be its own function that a
/// mutation run can name.
struct Inputs<'a> {
    launch: &'a radar_store::Launch,
    trades: &'a [Trade],
    /// Whether the trades table covers this launch's whole window. When it does
    /// not, every trade-derived feature is absent rather than zero.
    trades_recorded: bool,
    per_mint: &'a BTreeMap<Address, Vec<&'a Outcome>>,
    by_creator: &'a BTreeMap<Address, Vec<(Slot, Address)>>,
    names: &'a BTreeMap<String, Vec<Slot>>,
    symbols: &'a BTreeMap<String, Vec<Slot>>,
    hosts: &'a BTreeMap<String, Vec<Slot>>,
    decision: Option<&'a radar_store::Decision>,
}

impl Inputs<'_> {
    /// The slot the token launched in.
    fn launch_slot(&self) -> Slot {
        self.launch.envelope.slot
    }

    /// T — the moment every feature is observed at or before.
    fn t(&self) -> Slot {
        self.launch_slot() + SlotDelta(ENTRY_OFFSET_SLOTS)
    }
}

/// Records one feature, turning a refusal into an error that names it.
fn set(
    builder: &mut RowBuilder,
    mint: Address,
    feature: &'static str,
    observed: Observed<f64>,
) -> Result<(), BuildError> {
    let index = feature_index(feature).expect("a name from FEATURES");
    builder
        .set(index, observed)
        .map_err(|source| BuildError::LookAhead {
            mint,
            feature,
            source,
        })
}

fn build_row(inputs: &Inputs<'_>) -> Result<Row, BuildError> {
    let mut builder = RowBuilder::new(inputs.t());
    launch_block_features(&mut builder, inputs)?;
    creator_record_features(&mut builder, inputs)?;
    activity_features(&mut builder, inputs)?;
    velocity_features(&mut builder, inputs)?;
    factory_features(&mut builder, inputs)?;
    decision_features(&mut builder, inputs)?;

    let (gross_6h_bps, gross_24h_bps, mode) = labels(inputs);
    Ok(Row {
        mint: inputs.launch.mint,
        creator: inputs.launch.creator,
        launch_slot: inputs.launch_slot(),
        t: inputs.t(),
        values: builder.values,
        gross_6h_bps,
        gross_24h_bps,
        mode,
    })
}

/// Who was in the launch block, and in what arrangement.
fn launch_block_features(builder: &mut RowBuilder, inputs: &Inputs<'_>) -> Result<(), BuildError> {
    let mint = inputs.launch.mint;
    let at = inputs.launch_slot();
    if !inputs.trades_recorded {
        // The dev buy comes off the launch row rather than the trades table, so
        // it survives; the rest of this function is about trades.
        if let Some(lamports) = inputs.launch.dev_buy_lamports {
            set(
                builder,
                mint,
                "dev_buy_lamports",
                Observed::new(lamports_as_f64(lamports), at),
            )?;
        }
        return Ok(());
    }
    let in_block: Vec<&Trade> = inputs
        .trades
        .iter()
        .filter(|t| t.envelope.slot == at)
        .collect();
    let accounts: BTreeSet<Address> = in_block.iter().map(|t| t.trader).collect();
    let positions: BTreeSet<u32> = in_block.iter().map(|t| t.envelope.tx_index).collect();

    set(
        builder,
        mint,
        "launch_traders",
        Observed::new(count(accounts.len()), at),
    )?;
    set(
        builder,
        mint,
        "launch_transactions",
        Observed::new(count(positions.len()), at),
    )?;
    set(
        builder,
        mint,
        "launch_contiguity",
        Observed::new(count(longest_run(&positions)), at),
    )?;
    if let Some(lamports) = inputs.launch.dev_buy_lamports {
        set(
            builder,
            mint,
            "dev_buy_lamports",
            Observed::new(lamports_as_f64(lamports), at),
        )?;
    }
    Ok(())
}

/// The creator's record **as it stood at T**.
///
/// Counted from measurements taken by T, which is the pivot discipline
/// [`study`](crate::study) keeps across a population, applied per row. A
/// measurement of a sibling token taken after T is exactly the leak this table
/// exists to make impossible, and it is skipped here and refused by the guard.
fn creator_record_features(
    builder: &mut RowBuilder,
    inputs: &Inputs<'_>,
) -> Result<(), BuildError> {
    let mint = inputs.launch.mint;
    let launch_slot = inputs.launch_slot();
    let t = inputs.t();
    let empty = Vec::new();
    let siblings = inputs
        .by_creator
        .get(&inputs.launch.creator)
        .unwrap_or(&empty);

    let mut prior_launches = 0u64;
    let mut organic = 0u64;
    let mut instant = 0u64;
    let mut stillborn = 0u64;
    let mut first_prior: Option<Slot> = None;
    // The latest slot any of this rests on, which is what the value is observed
    // at. Starts at the launch itself: a count of zero is known then.
    let mut at = launch_slot;

    for (slot, other) in siblings {
        if *slot >= launch_slot {
            continue;
        }
        prior_launches += 1;
        first_prior.get_or_insert(*slot);
        at = at.max(*slot);
        let Some(series) = inputs.per_mint.get(other) else {
            continue;
        };
        let Some(outcome) = latest_by(series, t) else {
            continue;
        };
        at = at.max(outcome.measured_at);
        if outcome.appears_stillborn() {
            stillborn += 1;
        }
        match outcome.graduation_mode() {
            Some(GraduationMode::Organic) => organic += 1,
            Some(GraduationMode::Instant) => instant += 1,
            None => {}
        }
    }

    for (name, value) in [
        ("creator_prior_launches", prior_launches),
        ("creator_prior_organic", organic),
        ("creator_prior_instant", instant),
        ("creator_prior_stillborn", stillborn),
    ] {
        set(builder, mint, name, Observed::new(whole(value), at))?;
    }

    if let Some(first) = first_prior {
        let span = launch_slot.saturating_since(first).get();
        // A creator whose prior launches all landed in one slot has no rate: the
        // denominator is a span, and a span of zero is not a small one.
        if span > 0 {
            let days = whole(span) / whole(SLOTS_PER_HOUR * 24);
            set(
                builder,
                mint,
                "creator_launches_per_day",
                Observed::new(whole(prior_launches) / days, at),
            )?;
        }
    }
    Ok(())
}

/// How much happened, at three widths.
fn activity_features(builder: &mut RowBuilder, inputs: &Inputs<'_>) -> Result<(), BuildError> {
    if !inputs.trades_recorded {
        return Ok(());
    }
    let mint = inputs.launch.mint;
    for (width, trades_name, traders_name) in [
        (25u64, "trades_25", "traders_25"),
        (300, "trades_300", "traders_300"),
        (ENTRY_OFFSET_SLOTS, "trades_6000", "traders_6000"),
    ] {
        let edge = inputs.launch_slot() + SlotDelta(width);
        let seen: Vec<&Trade> = inputs
            .trades
            .iter()
            .filter(|t| t.envelope.slot <= edge)
            .collect();
        let distinct: BTreeSet<Address> = seen.iter().map(|t| t.trader).collect();
        // Observed at the window's edge rather than at the last trade inside it:
        // "nothing traded by slot X" is a statement about X, and it is only true
        // once X has passed. Bounded by T, which it already is by construction.
        let at = edge.min(inputs.t());
        set(
            builder,
            mint,
            trades_name,
            Observed::new(count(seen.len()), at),
        )?;
        set(
            builder,
            mint,
            traders_name,
            Observed::new(count(distinct.len()), at),
        )?;
    }
    Ok(())
}

/// Liquidity velocity: the trades it took to move the curve to each depth.
///
/// Buys only, and only those whose moved lamports the decoder recovered. A sell
/// is not inflow, and a buy with no realised figure is not a buy of zero.
fn velocity_features(builder: &mut RowBuilder, inputs: &Inputs<'_>) -> Result<(), BuildError> {
    if !inputs.trades_recorded {
        return Ok(());
    }
    let mint = inputs.launch.mint;
    let mut cumulative = 0u128;
    let mut taken = 0u64;
    let mut depths = [
        (10u128, "trades_to_10_sol", None),
        (20, "trades_to_20_sol", None),
        (30, "trades_to_30_sol", None),
    ];

    for trade in inputs.trades {
        let Some(lamports) = trade.realised_lamports else {
            continue;
        };
        if trade.side != Side::Buy {
            continue;
        }
        taken += 1;
        cumulative += u128::from(lamports);
        for (sol, _, reached) in &mut depths {
            if reached.is_none() && cumulative >= *sol * 1_000_000_000 {
                *reached = Some((taken, trade.envelope.slot));
            }
        }
    }

    for (_, name, reached) in depths {
        if let Some((trades, at)) = reached {
            set(builder, mint, name, Observed::new(whole(trades), at))?;
        }
    }
    Ok(())
}

/// How many earlier launches wore the same name, symbol and metadata host.
fn factory_features(builder: &mut RowBuilder, inputs: &Inputs<'_>) -> Result<(), BuildError> {
    let mint = inputs.launch.mint;
    let at = inputs.launch_slot();
    set(
        builder,
        mint,
        "prior_same_name",
        Observed::new(
            inputs
                .names
                .get(&inputs.launch.name)
                .map_or(0.0, |s| prior_count(s, at)),
            at,
        ),
    )?;
    set(
        builder,
        mint,
        "prior_same_symbol",
        Observed::new(
            inputs
                .symbols
                .get(&inputs.launch.symbol)
                .map_or(0.0, |s| prior_count(s, at)),
            at,
        ),
    )?;
    if let Some(host) = uri_host(&inputs.launch.uri) {
        set(
            builder,
            mint,
            "prior_same_uri_host",
            Observed::new(
                inputs.hosts.get(host).map_or(0.0, |s| prior_count(s, at)),
                at,
            ),
        )?;
    }
    Ok(())
}

/// What the decision lane recorded, where it decided this mint by T.
fn decision_features(builder: &mut RowBuilder, inputs: &Inputs<'_>) -> Result<(), BuildError> {
    let mint = inputs.launch.mint;
    let Some(decision) = inputs.decision else {
        return Ok(());
    };
    let at = decision.decided_at;

    if let Some(recipients) = decision.launch_recipients {
        set(
            builder,
            mint,
            "decision_launch_recipients",
            Observed::new(f64::from(recipients), at),
        )?;
    }
    if let Some(transactions) = decision.launch_transactions {
        set(
            builder,
            mint,
            "decision_launch_transactions",
            Observed::new(f64::from(transactions), at),
        )?;
    }
    if let Some(ordinal) = decision
        .authority_prevalence
        .as_deref()
        .and_then(prevalence_ordinal)
    {
        set(
            builder,
            mint,
            "decision_authority_prevalence",
            Observed::new(ordinal, at),
        )?;
    }
    Ok(())
}

/// What happened next: the two gross returns and the graduation mode.
///
/// **Read from after T on purpose.** These take no watermark and never touch
/// [`RowBuilder`], so no index confusion can put one in a feature column.
///
/// The entry is the first checkpoint at or after T; the exit is the last
/// checkpoint at or before the horizon, and it has to be a later measurement
/// than the entry or the "return" is one reading divided by itself.
fn labels(inputs: &Inputs<'_>) -> (Option<f64>, Option<f64>, Option<GraduationMode>) {
    let series = inputs
        .per_mint
        .get(&inputs.launch.mint)
        .cloned()
        .unwrap_or_default();
    let t = inputs.t();
    let entry = first_from(&series, t);
    let gross = |hours: u64| -> Option<f64> {
        let entry = entry?;
        let horizon = inputs.launch_slot() + SlotDelta(hours * SLOTS_PER_HOUR);
        let exit = latest_by(&series, horizon)?;
        // Strictly later than the entry, not merely at or after T. The first
        // live run made this exact mistake: most mints in the production store
        // are measured once after T, so the latest measurement before the
        // horizon *was* the entry, and the label came back as one reading
        // divided by itself. That is a return of exactly zero, reported for
        // 357,077 rows, and every median in every stratum was 0.0 bps -- a null
        // that looked like a measurement and was an identity.
        if exit.measured_at <= entry.measured_at {
            return None;
        }
        // And both prices have to be **fresh**, which is the same mistake two
        // levels down. `last_price` is the price of the last observed fill, so
        // a token that stops trading reports the identical number at every
        // later measurement: two measurements, one observation, and a return of
        // exactly zero. That is not a price that held steady -- it is a price
        // nobody quoted, on a position that could not have been exited at all.
        //
        // Three runs found this three times. `last_transfer_slot` was the
        // second attempt and it was the wrong field: a transfer is a token
        // moving between wallets, which happens to dead coins constantly and is
        // not a trade.
        //
        // `window_peak_price` is the right one and it was here all along. It is
        // taken from **that measurement's own window** and is `None` when the
        // window held no fills, so `is_some()` says "fills happened near this
        // reading" -- which is precisely the question. Both ends need it: a
        // stale entry price is a quote nobody could have bought at, and a stale
        // exit price is one nobody could have sold at.
        //
        // It is absent on every row written before 2026-08-31, when the column
        // was added, so labels before then are refused rather than guessed.
        if entry.window_peak_price.is_none() || exit.window_peak_price.is_none() {
            return None;
        }
        bps_between(entry.last_price?, exit.last_price?)
    };
    (
        gross(6),
        gross(24),
        series.last().and_then(|o| o.graduation_mode()),
    )
}

/// The ordinal of a prevalence label, or `None` for one this does not know.
///
/// The strings are what [`radar_graph::prevalence::Prevalence::label`] writes.
/// Ordered, because the bands are ordered by appearance count and a threshold
/// over them is meaningful; unknown rather than a fallback number, because a
/// label this does not recognise is a schema change and must not be quietly
/// bucketed as "ordinary".
fn prevalence_ordinal(label: &str) -> Option<f64> {
    match label {
        "ordinary" => Some(0.0),
        "repeat launcher" => Some(1.0),
        "infrastructure" => Some(2.0),
        _ => None,
    }
}

/// The longest run of consecutive positions.
///
/// A Jito bundle is sequential, atomic and inside one slot, so its transactions
/// occupy consecutive positions in the block. This counts the longest such run
/// among the launch slot's trades — design 0010 §7.2 c, and §11 item 3 is the
/// caveat that block order equalling execution order is an external claim a
/// capture has yet to dispose.
fn longest_run(positions: &BTreeSet<u32>) -> usize {
    let mut best = 0usize;
    let mut run = 0usize;
    let mut previous: Option<u32> = None;
    for position in positions {
        run = match previous {
            Some(p) if p + 1 == *position => run + 1,
            _ => 1,
        };
        best = best.max(run);
        previous = Some(*position);
    }
    best
}

/// Return in basis points from `entry` to `exit`, both in [`PRICE_SCALE`] units.
///
/// `None` when the entry price is zero: a return measured against nothing is
/// infinite rather than large, and rule 9 says an unmeasurable figure is absent.
///
/// [`PRICE_SCALE`]: radar_store::PRICE_SCALE
fn bps_between(entry: u64, exit: u64) -> Option<f64> {
    if entry == 0 {
        return None;
    }
    let entry = lamports_as_f64(entry);
    let exit = lamports_as_f64(exit);
    Some((exit / entry - 1.0) * 10_000.0)
}

/// A count as a feature value.
#[expect(clippy::cast_precision_loss, reason = "counts, far below 2^53")]
fn count(n: usize) -> f64 {
    n as f64
}

/// A whole number as a feature value.
#[expect(clippy::cast_precision_loss, reason = "counts, far below 2^53")]
fn whole(n: u64) -> f64 {
    n as f64
}

/// A price or a lamport figure as a feature value.
///
/// Above 2^53 this loses precision, and every figure it is handed is far below:
/// a dev buy is lamports of SOL and a price is scaled by `PRICE_SCALE`, whose
/// products are ratios rather than sums.
#[expect(clippy::cast_precision_loss, reason = "ratios, not exact arithmetic")]
fn lamports_as_f64(n: u64) -> f64 {
    n as f64
}

// --- the file ------------------------------------------------------------
//
// Parquet, and the same shape the store writes: zstd, dictionary-encoded, one
// file. Not *in* the store — a feature table is derived and recoverable, which
// is ADR 0006's rule for what the store must not hold. It is an artifact of a
// research pass, named by the watermark it was built at so two of them from
// different points in history cannot be confused for each other.
//
// The reader exists because the writer would otherwise be a layer with no
// caller (LEARNINGS 1, 9 and 10), and because a round trip is the only way to
// know the file says what the table said.

/// What stopped a feature table being written or read.
#[derive(Debug, thiserror::Error)]
pub enum FileError {
    /// The file could not be opened or created.
    #[error("cannot open {path}: {source}")]
    Io {
        /// The path being read or written.
        path: String,
        /// What the filesystem said.
        source: std::io::Error,
    },
    /// Parquet or arrow refused.
    #[error("parquet: {0}")]
    Parquet(#[from] parquet::errors::ParquetError),
    /// Arrow refused to build or read a batch.
    #[error("arrow: {0}")]
    Arrow(#[from] arrow::error::ArrowError),
    /// The file is not a feature table this version can read.
    #[error("{path} is not a feature table: {why}")]
    Malformed {
        /// The path.
        path: String,
        /// What was wrong.
        why: String,
    },
}

/// The metadata key carrying the watermark the table was built at.
const WATERMARK_KEY: &str = "radar.watermark_slot";
/// The metadata key carrying the offset from launch to T.
const ENTRY_OFFSET_KEY: &str = "radar.entry_offset_slots";

/// The file name for a table built at `watermark`.
///
/// Zero-padded so a directory listing sorts chronologically, which is the order
/// the folds run in.
#[must_use]
pub fn file_name(watermark: Slot) -> String {
    format!("features_slot_{:012}.parquet", watermark.get())
}

/// The arrow schema. One nullable float per feature, in [`FEATURES`] order.
fn schema() -> arrow::datatypes::Schema {
    use arrow::datatypes::{DataType, Field};

    let mut fields = vec![
        Field::new("mint", DataType::Utf8, false),
        Field::new("creator", DataType::Utf8, false),
        Field::new("launch_slot", DataType::UInt64, false),
        Field::new("t", DataType::UInt64, false),
    ];
    for name in FEATURES {
        fields.push(Field::new(*name, DataType::Float64, true));
    }
    fields.push(Field::new("gross_6h_bps", DataType::Float64, true));
    fields.push(Field::new("gross_24h_bps", DataType::Float64, true));
    fields.push(Field::new("graduation_mode", DataType::Utf8, true));
    arrow::datatypes::Schema::new(fields)
}

/// The spelling of a graduation mode in the file.
///
/// The same two words the store's own `serde` derive writes, so a reader of
/// either file sees one vocabulary.
const fn mode_label(mode: GraduationMode) -> &'static str {
    match mode {
        GraduationMode::Instant => "instant",
        GraduationMode::Organic => "organic",
    }
}

/// The inverse, refusing anything else rather than guessing.
fn mode_from(label: &str) -> Option<GraduationMode> {
    match label {
        "instant" => Some(GraduationMode::Instant),
        "organic" => Some(GraduationMode::Organic),
        _ => None,
    }
}

/// Writes the table.
///
/// Deterministic: the same table written twice produces the same bytes, which
/// `radar features` run twice depends on and
/// `the_same_table_writes_the_same_bytes` holds.
///
/// # Errors
///
/// [`FileError`] when the path cannot be created or arrow refuses the batch.
pub fn write(table: &FeatureTable, path: &std::path::Path) -> Result<(), FileError> {
    use arrow::array::{ArrayRef, Float64Array, StringArray, UInt64Array};
    use parquet::arrow::ArrowWriter;
    use parquet::basic::{Compression, ZstdLevel};
    use parquet::file::properties::WriterProperties;
    use std::sync::Arc;

    let schema = Arc::new(schema());
    let mut columns: Vec<ArrayRef> = vec![
        Arc::new(
            table
                .rows
                .iter()
                .map(|r| Some(r.mint.to_string()))
                .collect::<StringArray>(),
        ),
        Arc::new(
            table
                .rows
                .iter()
                .map(|r| Some(r.creator.to_string()))
                .collect::<StringArray>(),
        ),
        Arc::new(
            table
                .rows
                .iter()
                .map(|r| Some(r.launch_slot.get()))
                .collect::<UInt64Array>(),
        ),
        Arc::new(
            table
                .rows
                .iter()
                .map(|r| Some(r.t.get()))
                .collect::<UInt64Array>(),
        ),
    ];
    for index in 0..FEATURES.len() {
        columns.push(Arc::new(
            table
                .rows
                .iter()
                .map(|r| r.value(index))
                .collect::<Float64Array>(),
        ));
    }
    columns.push(Arc::new(
        table
            .rows
            .iter()
            .map(|r| r.gross_6h_bps)
            .collect::<Float64Array>(),
    ));
    columns.push(Arc::new(
        table
            .rows
            .iter()
            .map(|r| r.gross_24h_bps)
            .collect::<Float64Array>(),
    ));
    columns.push(Arc::new(
        table
            .rows
            .iter()
            .map(|r| r.mode.map(mode_label))
            .collect::<StringArray>(),
    ));

    let batch = arrow::record_batch::RecordBatch::try_new(Arc::clone(&schema), columns)?;
    let props = WriterProperties::builder()
        .set_compression(Compression::ZSTD(ZstdLevel::try_new(3).unwrap_or_default()))
        .set_dictionary_enabled(true)
        // The watermark and the offset are properties of the whole table, not
        // of a row. Repeating them per row would make an empty table lose them,
        // and an empty fold is a legitimate result.
        .set_key_value_metadata(Some(vec![
            parquet::file::metadata::KeyValue::new(
                WATERMARK_KEY.to_owned(),
                table.watermark.get().to_string(),
            ),
            parquet::file::metadata::KeyValue::new(
                ENTRY_OFFSET_KEY.to_owned(),
                table.entry_offset.to_string(),
            ),
        ]))
        .build();

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| FileError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    let file = std::fs::File::create(path).map_err(|source| FileError::Io {
        path: path.display().to_string(),
        source,
    })?;
    let mut writer = ArrowWriter::try_new(file, schema, Some(props))?;
    writer.write(&batch)?;
    writer.close()?;
    Ok(())
}

/// Reads a table back.
///
/// # Errors
///
/// [`FileError`] when the file cannot be read, is not parquet, or does not
/// carry the columns and metadata a feature table has.
pub fn read(path: &std::path::Path) -> Result<FeatureTable, FileError> {
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

    let display = path.display().to_string();
    let file = std::fs::File::open(path).map_err(|source| FileError::Io {
        path: display.clone(),
        source,
    })?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;

    let metadata = |key: &str| -> Option<u64> {
        builder
            .metadata()
            .file_metadata()
            .key_value_metadata()?
            .iter()
            .find(|kv| kv.key == key)
            .and_then(|kv| kv.value.as_ref())
            .and_then(|v| v.parse().ok())
    };
    // Absent metadata is a different file, not a table with a watermark of
    // zero: a fold boundary computed against slot zero would silently include
    // everything.
    let watermark = metadata(WATERMARK_KEY).ok_or_else(|| FileError::Malformed {
        path: display.clone(),
        why: format!("no {WATERMARK_KEY} in the file metadata"),
    })?;
    let entry_offset = metadata(ENTRY_OFFSET_KEY).ok_or_else(|| FileError::Malformed {
        path: display.clone(),
        why: format!("no {ENTRY_OFFSET_KEY} in the file metadata"),
    })?;

    let mut rows = Vec::new();
    for batch in builder.build()? {
        rows.extend(rows_from(&batch?, &display)?);
    }

    Ok(FeatureTable {
        watermark: Slot(watermark),
        entry_offset,
        rows,
    })
}

/// Decodes one batch into rows.
///
/// Split out of [`read`] so that the file-level concerns — opening it, finding
/// the metadata a table cannot do without — stay separate from the column
/// decoding, which is the part that grows by one line per feature.
fn rows_from(batch: &arrow::record_batch::RecordBatch, path: &str) -> Result<Vec<Row>, FileError> {
    use arrow::array::{Array, Float64Array, StringArray, UInt64Array};

    let column = |name: &str| -> Result<&arrow::array::ArrayRef, FileError> {
        batch
            .column_by_name(name)
            .ok_or_else(|| FileError::Malformed {
                path: path.to_owned(),
                why: format!("no {name} column"),
            })
    };
    let text = |name: &str| -> Result<StringArray, FileError> {
        column(name)?
            .as_any()
            .downcast_ref::<StringArray>()
            .cloned()
            .ok_or_else(|| FileError::Malformed {
                path: path.to_owned(),
                why: format!("{name} is not text"),
            })
    };
    let number = |name: &str| -> Result<UInt64Array, FileError> {
        column(name)?
            .as_any()
            .downcast_ref::<UInt64Array>()
            .cloned()
            .ok_or_else(|| FileError::Malformed {
                path: path.to_owned(),
                why: format!("{name} is not a slot"),
            })
    };
    let real = |name: &str| -> Result<Float64Array, FileError> {
        column(name)?
            .as_any()
            .downcast_ref::<Float64Array>()
            .cloned()
            .ok_or_else(|| FileError::Malformed {
                path: path.to_owned(),
                why: format!("{name} is not a number"),
            })
    };

    let mint = text("mint")?;
    let creator = text("creator")?;
    let launch_slot = number("launch_slot")?;
    let t = number("t")?;
    let features: Vec<Float64Array> = FEATURES
        .iter()
        .map(|name| real(name))
        .collect::<Result<_, _>>()?;
    let six = real("gross_6h_bps")?;
    let day = real("gross_24h_bps")?;
    let mode = text("graduation_mode")?;

    let parse = |value: &str, what: &str| -> Result<Address, FileError> {
        value.parse().map_err(|_| FileError::Malformed {
            path: path.to_owned(),
            why: format!("{what} {value} is not an address"),
        })
    };

    let mut rows = Vec::with_capacity(batch.num_rows());
    for i in 0..batch.num_rows() {
        rows.push(Row {
            mint: parse(mint.value(i), "mint")?,
            creator: parse(creator.value(i), "creator")?,
            launch_slot: Slot(launch_slot.value(i)),
            t: Slot(t.value(i)),
            values: features
                .iter()
                .map(|c| c.is_valid(i).then(|| c.value(i)))
                .collect(),
            gross_6h_bps: six.is_valid(i).then(|| six.value(i)),
            gross_24h_bps: day.is_valid(i).then(|| day.value(i)),
            // An unknown label reads as absent rather than as a mode. A new
            // spelling is a schema change, and guessing one would put tokens in
            // a cohort nobody measured.
            mode: mode.is_valid(i).then(|| mode_from(mode.value(i))).flatten(),
        });
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(n: u64) -> Slot {
        Slot(n)
    }

    #[test]
    fn a_value_observed_after_t_is_refused_and_never_becomes_a_number() {
        // The planted leak. This is the whole reason the module exists: a
        // feature computed from something that had not happened yet is a
        // build error naming the feature, not a plausible-looking column.
        let mut builder = RowBuilder::new(slot(1_000));
        let refused = builder.set(0, Observed::new(1.0, slot(1_001)));

        let error = refused.expect_err("a value from after T must be refused");
        assert_eq!(error.watermark, slot(1_000));
        assert_eq!(error.observed, slot(1_001));
        assert_eq!(
            builder.values[0], None,
            "a refused value must leave the column absent, not zero"
        );
    }

    #[test]
    fn a_value_observed_at_t_is_admitted() {
        // The other edge, because a guard that refuses everything would pass
        // the test above and produce an empty table.
        let mut builder = RowBuilder::new(slot(1_000));
        builder
            .set(0, Observed::new(7.0, slot(1_000)))
            .expect("T itself is not the future");
        assert_eq!(builder.values[0], Some(7.0));
    }

    #[test]
    fn the_entry_offset_is_the_strategys_own_age_limit() {
        // Two constants that must agree, in two crates. If the strategy stops
        // looking at tokens older than some other age, a table measured at
        // 6,000 slots is fitting a rule nothing could act on.
        assert_eq!(
            ENTRY_OFFSET_SLOTS,
            radar_strategy::creator_edge::Thresholds::DEFAULT.max_token_age
        );
    }

    #[test]
    fn every_feature_name_is_unique_and_resolvable() {
        // A duplicate name would silently make one column unreachable through
        // `feature_index` and leave the other permanently absent.
        let distinct: BTreeSet<&&str> = FEATURES.iter().collect();
        assert_eq!(distinct.len(), FEATURES.len(), "{FEATURES:?}");
        for (index, name) in FEATURES.iter().enumerate() {
            assert_eq!(feature_index(name), Some(index));
        }
        assert_eq!(feature_index("not a feature"), None);
    }

    #[test]
    fn contiguity_counts_the_longest_consecutive_run() {
        // The bundle's shape. A gap breaks the run; a repeat does not extend it,
        // because positions are a set.
        assert_eq!(longest_run(&BTreeSet::new()), 0);
        assert_eq!(longest_run(&[4u32].into_iter().collect()), 1);
        assert_eq!(longest_run(&[1u32, 2, 3].into_iter().collect()), 3);
        assert_eq!(longest_run(&[1u32, 2, 4, 5, 6].into_iter().collect()), 3);
        assert_eq!(
            longest_run(&[9u32, 1, 2].into_iter().collect()),
            2,
            "a set is walked in order, so the input's order cannot change the answer"
        );
    }

    #[test]
    fn a_uri_host_is_read_as_text_and_never_fetched() {
        assert_eq!(uri_host("https://ipfs.io/ipfs/Qm123"), Some("ipfs.io"));
        assert_eq!(uri_host("https://ipfs.io"), Some("ipfs.io"));
        assert_eq!(uri_host("https://host/a?b#c"), Some("host"));
        assert_eq!(uri_host(""), None);
        assert_eq!(uri_host("not a uri"), None);
        assert_eq!(uri_host("https://"), None, "an empty host is not a host");
    }

    #[test]
    fn prior_counts_are_strictly_before_the_launch_being_described() {
        // Off by one here is a leak: counting the row's own launch would tell
        // every launch that one prior launch shared its name.
        let slots = vec![slot(10), slot(20), slot(20), slot(30)];
        assert!((prior_count(&slots, slot(10)) - 0.0).abs() < f64::EPSILON);
        assert!((prior_count(&slots, slot(20)) - 1.0).abs() < f64::EPSILON);
        assert!((prior_count(&slots, slot(21)) - 3.0).abs() < f64::EPSILON);
        assert!((prior_count(&slots, slot(5)) - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_return_against_a_zero_entry_is_absent_rather_than_infinite() {
        assert_eq!(bps_between(0, 100), None);
        assert_eq!(bps_between(100, 100), Some(0.0));
        assert_eq!(bps_between(100, 200), Some(10_000.0));
        assert_eq!(bps_between(100, 50), Some(-5_000.0));
    }

    #[test]
    fn a_graduation_mode_survives_the_file_in_the_stores_own_vocabulary() {
        // Both directions and both variants. A dropped arm here reads every
        // instant graduation back as "not measured", which would quietly move
        // the whole instant cohort out of every stratum that names it -- and the
        // file would still parse.
        for mode in [GraduationMode::Instant, GraduationMode::Organic] {
            assert_eq!(mode_from(mode_label(mode)), Some(mode), "{mode:?}");
        }
        assert_eq!(mode_label(GraduationMode::Instant), "instant");
        assert_eq!(mode_label(GraduationMode::Organic), "organic");
        assert_eq!(
            mode_from("bundled"),
            None,
            "a spelling this does not know is absent, not a guess: a new label              is a schema change and guessing one puts tokens in a cohort nobody              measured"
        );
    }

    #[test]
    fn an_unrecognised_prevalence_label_is_absent_rather_than_ordinary() {
        assert_eq!(prevalence_ordinal("ordinary"), Some(0.0));
        assert_eq!(prevalence_ordinal("repeat launcher"), Some(1.0));
        assert_eq!(prevalence_ordinal("infrastructure"), Some(2.0));
        assert_eq!(prevalence_ordinal("something new"), None);
        assert_eq!(prevalence_ordinal(""), None);
    }
}
