// SPDX-License-Identifier: Apache-2.0
//! What Radar records.
//!
//! One schema serves both the historical backfill and the live recorder. That is
//! not tidiness — it is what makes the replay test meaningful. If history came in
//! through a different shape, replaying a recorded decision would compare two
//! pipelines rather than checking one.
//!
//! Two distinctions in here are easy to lose and expensive to lose:
//!
//! **Requested versus realised.** A `buy(tokens, max_sol_cost)` says what the
//! trader asked for, not what they paid. The realised figures come from balance
//! deltas and are [`Option`] because they are not always recoverable — a failed
//! transaction has none, and a live path that only saw the instruction has none
//! either. Defaulting them to zero would silently report every unresolved trade
//! as free.
//!
//! **Failed transactions are events.** A buy that reverted is real information
//! about a token — often the first sign that it cannot be traded — so it is
//! recorded with `success: Some(false)` rather than dropped.
//!
//! **And unresolved is a third state, not the first two.** Success, the
//! transaction's position in its block, and the account that traded are all
//! `Option` because the recorder does not always resolve them. Each of the
//! three was a sentinel until 2026-09-07 — `true`, `u32::MAX` and the system
//! program — and each sentinel reads as a measurement downstream: an outage
//! became successful activity, an unresolved position joined a contiguity run,
//! and one placeholder address made every backfilled launch look like it had
//! exactly one trader. [`Envelope::from_stored`] translates the two envelope
//! sentinels on read, so files written before the change say what they meant.

use radar_types::{Address, Signature, Slot};
use serde::{Deserialize, Serialize};

/// Which way a trade went.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    /// Acquiring tokens.
    Buy,
    /// Disposing of tokens.
    Sell,
}

impl Side {
    /// The name used in the stored column.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Buy => "buy",
            Self::Sell => "sell",
        }
    }
}

/// Where every event sits in the chain, and whether it took effect.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Envelope {
    /// The slot. Radar's only clock, and the point-in-time key.
    pub slot: Slot,
    /// The transaction this came from.
    pub signature: Signature,
    /// Position of the transaction within its block, when it is known.
    ///
    /// The input to same-slot clustering: coordination analysis needs to know
    /// which transactions landed together and in what order, and that ordering
    /// exists nowhere else once the block is discarded.
    ///
    /// `None` means the recorder never resolved a position. **It was `u32::MAX`
    /// until 2026-09-07**, which is a sentinel a reader has to know about to
    /// avoid treating it as a position four billion transactions into a block —
    /// and `longest_run` over a set containing it produced exactly that. Files
    /// written before the change still hold the sentinel and
    /// [`Envelope::from_stored`] translates it.
    pub tx_index: Option<u32>,
    /// Position of the instruction within its transaction.
    pub instruction_index: u32,
    /// The enclosing instruction, if this one was a cross-program invocation.
    pub parent_index: Option<u32>,
    /// Whether the transaction succeeded, when that is known.
    ///
    /// Three states, and each is a different sentence:
    ///
    /// - `Some(true)` — the chain accepted it.
    /// - `Some(false)` — the chain rejected it. A failed buy is information
    ///   about a token, not the absence of one, so it is recorded rather than
    ///   dropped.
    /// - `None` — **nobody looked**. The backfill's success comes from a join
    ///   against the transactions table, and when that join finds nothing there
    ///   is no `err` column to read. This was `true` until 2026-09-07: absent
    ///   was recorded as succeeded, which is rule 9 with the sign flipped and
    ///   the direction that invents activity.
    ///
    /// Read it through [`succeeded`](Self::succeeded) and
    /// [`failed`](Self::failed) rather than matching here, so that "unknown is
    /// not success" is one decision in one place.
    pub success: Option<bool>,
}

impl Envelope {
    /// Whether the transaction is **known** to have succeeded.
    ///
    /// Unknown is not success. Every population that means "what actually
    /// happened" — succeeded launches, distinct buyers, graduations, curve
    /// progress — filters on this, and an unresolved row is excluded rather
    /// than assumed. AGENTS.md rule 9.
    #[must_use]
    pub const fn succeeded(&self) -> bool {
        matches!(self.success, Some(true))
    }

    /// Whether the transaction is **known** to have failed.
    ///
    /// Deliberately not `!succeeded()`: a count of failures that silently
    /// includes every unresolved row reports an outage as a chain event.
    #[must_use]
    pub const fn failed(&self) -> bool {
        matches!(self.success, Some(false))
    }

    /// Whether nobody established either way.
    #[must_use]
    pub const fn outcome_unknown(&self) -> bool {
        self.success.is_none()
    }

    /// A total order over positions within a slot that puts unknown **last**.
    ///
    /// `Option`'s own ordering puts `None` first, which would sort every
    /// unresolved trade ahead of the launch it belongs to and make a
    /// "first trade after the launch" read the least-known row in the block.
    /// The direction is arbitrary; having it written down once is not.
    #[must_use]
    pub const fn position_key(&self) -> (bool, u32, u32) {
        match self.tx_index {
            Some(tx) => (false, tx, self.instruction_index),
            None => (true, 0, self.instruction_index),
        }
    }

    /// The stored form of a row, with the pre-2026-09-07 sentinels translated.
    ///
    /// One function rather than a rule in the reader, because the two sentinels
    /// are **coupled**: `tx_index` and `success` both come from the backfill's
    /// join against the transactions table, so a row missing the position is a
    /// row whose `err` column was never read either. A reader that translated
    /// only the position would keep reporting those rows as successful, which
    /// is the half of the bug that mattered.
    ///
    /// `u32::MAX` is safe to read as a sentinel: no block reaches four billion
    /// transactions, and the writer that produced it said so in the comment
    /// beside it.
    #[must_use]
    pub fn from_stored(
        slot: Slot,
        signature: Signature,
        tx_index: Option<u32>,
        instruction_index: u32,
        parent_index: Option<u32>,
        success: Option<bool>,
    ) -> Self {
        let unresolved = tx_index.is_none_or(|tx| tx == LEGACY_UNKNOWN_TX_INDEX);
        Self {
            slot,
            signature,
            tx_index: if unresolved { None } else { tx_index },
            instruction_index,
            parent_index,
            success: if unresolved { None } else { success },
        }
    }
}

/// What the recorder wrote for an unresolved transaction position before
/// 2026-09-07.
///
/// Public because the backfill's own tests assert that it is no longer written,
/// and a magic number asserted in two crates is a magic number in two crates.
pub const LEGACY_UNKNOWN_TX_INDEX: u32 = u32::MAX;

/// Which program and instruction produced an event.
///
/// The instruction is stored by name rather than as an enum so the store stays
/// program-agnostic as decoders are added. An unrecognised instruction keeps its
/// discriminator as the name and sets `known` to false, which is what makes
/// "how much of the stream have we stopped understanding" a query rather than a
/// guess.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Origin {
    /// The program.
    pub program: Address,
    /// Anchor instruction name, or the discriminator in hex if unknown.
    pub instruction: String,
    /// Whether the decoder recognised the instruction.
    pub known: bool,
}

impl Origin {
    /// A recognised instruction.
    #[must_use]
    pub fn known(program: Address, instruction: impl Into<String>) -> Self {
        Self {
            program,
            instruction: instruction.into(),
            known: true,
        }
    }

    /// An instruction the decoder does not know, recorded by discriminator.
    #[must_use]
    pub fn unknown(program: Address, discriminator_hex: impl Into<String>) -> Self {
        Self {
            program,
            instruction: discriminator_hex.into(),
            known: false,
        }
    }
}

/// A token was created.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Launch {
    /// Where and when.
    pub envelope: Envelope,
    /// Which program and instruction.
    pub origin: Origin,
    /// The mint.
    pub mint: Address,
    /// The creator recorded in the instruction.
    pub creator: Address,
    /// Token name. **Untrusted** — arbitrary creator-controlled text. Store it,
    /// hash it, show it; never let it reach an instruction position.
    pub name: String,
    /// Token symbol. Untrusted, same as the name.
    pub symbol: String,
    /// Metadata URI. Untrusted, and never fetched automatically.
    pub uri: String,
    /// Lamports the creator spent buying their own token in the launch
    /// transaction, where that is recoverable.
    ///
    /// Present in roughly three launches in four. `None` means not recoverable,
    /// which is different from a dev buy of zero.
    pub dev_buy_lamports: Option<u64>,
}

/// A token was bought or sold.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Trade {
    /// Where and when.
    pub envelope: Envelope,
    /// Which program and instruction.
    pub origin: Origin,
    /// The mint traded.
    pub mint: Address,
    /// The account that traded, when it is known.
    ///
    /// `None` means the recorder never resolved one. The backfill's query does
    /// not fetch account keys, and until 2026-09-07 it wrote
    /// [`Address::SYSTEM_PROGRAM`] as a placeholder — so every trade in the
    /// store names the same "trader", and every count of distinct traders over
    /// backfilled data is 1. A placeholder is not an actor, and a feature that
    /// counts one is worse than a feature that is absent, because it looks like
    /// a measurement.
    pub trader: Option<Address>,
    /// Direction.
    pub side: Side,
    /// Lamports that actually moved, from balance deltas. `None` when not
    /// recoverable — never zero as a stand-in.
    pub realised_lamports: Option<u64>,
    /// Token base units that actually moved. `None` when not recoverable.
    pub realised_tokens: Option<u64>,
    /// The quantity the trader pinned in the instruction, in whichever unit the
    /// variant pins. Kept alongside the realised figures because the gap between
    /// them *is* the slippage.
    pub requested_amount: u64,
    /// Whether `requested_amount` counts lamports rather than token base units.
    pub requested_is_lamports: bool,
    /// The bound the trader accepted on the other side.
    pub limit_amount: u64,
    /// Whether the trader accepted any price at all — an unbounded max cost or a
    /// zero minimum output. Roughly 58% of sells do. A behavioural signal, not
    /// missing data.
    pub accepted_any_price: bool,
}

/// A token graduated from its bonding curve to an AMM.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Graduation {
    /// Where and when.
    pub envelope: Envelope,
    /// Which program and instruction.
    pub origin: Origin,
    /// The mint that graduated.
    pub mint: Address,
}

/// Anything Radar records.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    /// A token was created.
    Launch(Box<Launch>),
    /// A token was bought or sold.
    Trade(Box<Trade>),
    /// A token graduated to an AMM.
    Graduation(Box<Graduation>),
}

impl Event {
    /// The envelope, whatever the kind.
    #[must_use]
    pub const fn envelope(&self) -> &Envelope {
        match self {
            Self::Launch(e) => &e.envelope,
            Self::Trade(e) => &e.envelope,
            Self::Graduation(e) => &e.envelope,
        }
    }

    /// The mint this event concerns.
    #[must_use]
    pub const fn mint(&self) -> Address {
        match self {
            Self::Launch(e) => e.mint,
            Self::Trade(e) => e.mint,
            Self::Graduation(e) => e.mint,
        }
    }

    /// The slot. Shorthand for the envelope's.
    #[must_use]
    pub const fn slot(&self) -> Slot {
        self.envelope().slot
    }

    /// The partition table this event belongs to.
    #[must_use]
    pub const fn table(&self) -> Table {
        match self {
            Self::Launch(_) => Table::Launches,
            Self::Trade(_) => Table::Trades,
            Self::Graduation(_) => Table::Graduations,
        }
    }
}

/// One stored table per event kind.
///
/// Separate rather than a single wide table with nullable columns: the schemas
/// have little in common, and a launch row carrying eight null trade columns
/// compresses worse and reads worse than two narrow tables.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Table {
    /// Token creations.
    Launches,
    /// Buys and sells.
    Trades,
    /// Bonding-curve graduations.
    Graduations,
    /// Outcome measurements. Not chain events but derived observations, each
    /// stamped with the slot it was taken at.
    Outcomes,
    /// What the decision lane concluded about a candidate, at the watermark it
    /// concluded it. Not a chain event and not a measurement of the chain — a
    /// record of what Radar did, which is the only thing that can later be
    /// joined against prices to ask whether the selection was worth making.
    Decisions,
    /// What Radar holds and what it held.
    ///
    /// Append-only like the rest: opening writes a row and closing writes
    /// another with the same `opened_at`, so "what did Radar hold on Tuesday"
    /// is answerable at any watermark. A mutable row would answer it with
    /// today's state.
    Positions,
    /// What the recorder actually collected, and over what.
    ///
    /// Not a chain event and not a measurement of the chain — a record of which
    /// ingestion ranges were run and which of them finished. Coverage was
    /// inferred from partition filenames until 2026-09-07, and a file exists as
    /// soon as its first row lands: a run that died a quarter of the way
    /// through a window produced one, and the rest of the window read as a
    /// quiet market. See [`crate::coverage`].
    Coverage,
    /// Trades on the venue-agnostic market tape, as `radar-backfill`'s
    /// market-tape collector records them.
    ///
    /// Not a chain event in the sense this table's siblings are: there is no
    /// decoded instruction and no envelope, because a swap collected this way
    /// is a token leg and a quote leg outer-joined from raw transfers rather
    /// than a `buy`/`sell` this build decoded. Recorded like
    /// [`Positions`](Self::Positions) and [`Coverage`](Self::Coverage) —
    /// stamped with a real, per-row chain slot the way an event is, but through
    /// its own writer and reader rather than the shared envelope machinery,
    /// because it carries no signature position, no parent instruction and no
    /// success flag for that machinery to fill in. See [`crate::market_trade`].
    MarketTrades,
}

impl Table {
    /// Every table, including outcomes, decisions and positions. For directory setup and file listings —
    /// **not** for read loops. See [`EVENT_TABLES`](Self::EVENT_TABLES).
    pub const ALL: &'static [Self] = &[
        Self::Launches,
        Self::Trades,
        Self::Graduations,
        Self::Outcomes,
        Self::Decisions,
        Self::Positions,
        Self::Coverage,
        Self::MarketTrades,
    ];

    /// The tables that hold chain events, which is what
    /// [`Reader::read`](crate::Reader::read) understands.
    ///
    /// Outcomes are deliberately absent: they are measurements with no signature
    /// and no transaction position, and they are read by
    /// [`Reader::read_outcomes`](crate::Reader::read_outcomes). Iterating `ALL`
    /// and calling `read` on each compiles and then fails at runtime — it broke
    /// the CLI once — so the distinction is a constant rather than a comment.
    pub const EVENT_TABLES: &'static [Self] = &[Self::Launches, Self::Trades, Self::Graduations];

    /// Whether this table holds chain events rather than measurements.
    #[must_use]
    pub const fn holds_events(self) -> bool {
        matches!(self, Self::Launches | Self::Trades | Self::Graduations)
    }

    /// The column holding the slot this row is ordered and watermarked by.
    ///
    /// Not every table calls it `slot`, and the difference is meaningful:
    /// an event happened *at* a slot, while a measurement or a decision was
    /// *taken as of* one. Naming them alike would hide that.
    ///
    /// This exists as a method because the alternative had already appeared
    /// four times as `if table == Table::Outcomes { .. } else { .. }`, scattered
    /// across the reader, the writer and the schema. A third table makes every
    /// one of those silently wrong rather than loudly broken, which is
    /// [LEARNINGS] entry 6 exactly: widening a constant is an API change to
    /// every place that matches on it, and the compiler cannot see it. A method
    /// makes the match exhaustive, so adding a table stops compiling until each
    /// site is considered.
    ///
    /// [LEARNINGS]: https://github.com/hey-vera/radar/blob/main/LEARNINGS.md
    #[must_use]
    pub const fn slot_column(self) -> &'static str {
        match self {
            // A market trade carries a genuine per-row chain slot the same way
            // an event does, even though it is recorded rather than decoded --
            // see `Table::MarketTrades`'s own doc comment for why.
            Self::Launches | Self::Trades | Self::Graduations | Self::MarketTrades => "slot",
            Self::Outcomes => "measured_at",
            Self::Decisions => "decided_at",
            Self::Positions => "opened_at",
            // The collection watermark: the moment the range was established
            // complete. A coverage record written today must not make a
            // decision taken last week look better-informed than it was.
            Self::Coverage => "recorded_at",
        }
    }

    /// The directory name under the store root.
    #[must_use]
    pub const fn dir(self) -> &'static str {
        match self {
            Self::Launches => "launches",
            Self::Trades => "trades",
            Self::Graduations => "graduations",
            Self::Outcomes => "outcomes",
            Self::Decisions => "decisions",
            Self::Positions => "positions",
            Self::Coverage => "coverage",
            Self::MarketTrades => "market_trades",
        }
    }

    /// The table a directory name belongs to.
    ///
    /// The inverse of [`dir`](Self::dir), for the stored `table` column on a
    /// coverage row. Exhaustive over the same match, so a new table stops
    /// compiling here rather than reading back as `None`.
    #[must_use]
    pub fn from_dir(dir: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|t| t.dir() == dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(slot: u64) -> Envelope {
        Envelope {
            slot: Slot(slot),
            signature: Signature::new([1u8; 64]),
            tx_index: Some(7),
            instruction_index: 2,
            parent_index: None,
            success: Some(true),
        }
    }

    fn trade(slot: u64) -> Event {
        Event::Trade(Box::new(Trade {
            envelope: envelope(slot),
            origin: Origin::known(Address::SYSTEM_PROGRAM, "buy"),
            mint: Address::new([9u8; 32]),
            trader: Some(Address::new([8u8; 32])),
            side: Side::Buy,
            realised_lamports: None,
            realised_tokens: None,
            requested_amount: 1_000,
            requested_is_lamports: false,
            limit_amount: 2_000,
            accepted_any_price: false,
        }))
    }

    #[test]
    fn an_unresolved_amount_is_none_rather_than_zero() {
        // Zero would report every trade whose deltas could not be recovered as
        // free, which would flatter execution cost exactly where it is least
        // known.
        let Event::Trade(t) = trade(100) else {
            panic!()
        };
        assert_eq!(t.realised_lamports, None);
        assert_ne!(t.realised_lamports, Some(0));
    }

    #[test]
    fn events_route_to_their_own_table() {
        assert_eq!(trade(1).table(), Table::Trades);
        assert_eq!(Table::Trades.dir(), "trades");
        // Every table appears in ALL. Derived rather than a literal, because a
        // literal has to be bumped by whoever adds a table and is therefore
        // exactly as reliable as their remembering to.
        assert_eq!(
            Table::ALL.len(),
            Table::EVENT_TABLES.len() + Table::ALL.iter().filter(|t| !t.holds_events()).count()
        );
    }

    #[test]
    fn the_event_tables_are_exactly_the_ones_read_as_events() {
        // Iterating ALL and calling read() on each compiles and fails at
        // runtime, because a measurement has no slot column. The separate
        // constant is what stops that being a comment nobody reads.
        //
        // Stated as "EVENT_TABLES is exactly the tables that hold events",
        // rather than as a count with a named exception. The first version said
        // `EVENT_TABLES.len() == ALL.len() - 1` and listed Outcomes by name,
        // which broke the moment a second non-event table arrived -- and would
        // have passed had the new table been wrongly added to EVENT_TABLES.
        for t in Table::ALL {
            assert_eq!(
                Table::EVENT_TABLES.contains(t),
                t.holds_events(),
                "{t:?} disagrees about whether it holds events"
            );
        }
        assert!(!Table::Outcomes.holds_events());
        assert!(!Table::Decisions.holds_events());
        assert!(
            Table::ALL.iter().any(|t| !t.holds_events()),
            "a check over an empty set of exceptions would pass vacuously"
        );
    }

    #[test]
    fn every_table_has_a_distinct_directory() {
        let mut dirs: Vec<&str> = Table::ALL.iter().map(|t| t.dir()).collect();
        dirs.sort_unstable();
        let before = dirs.len();
        dirs.dedup();
        assert_eq!(dirs.len(), before);
    }

    #[test]
    fn an_unknown_instruction_keeps_its_discriminator_and_is_flagged() {
        // The unknown rate has to stay queryable: a decoder that has silently
        // stopped understanding a program looks exactly like a quiet program.
        let o = Origin::unknown(Address::SYSTEM_PROGRAM, "577c34bf3426d6e8");
        assert!(!o.known);
        assert_eq!(o.instruction, "577c34bf3426d6e8");
    }

    #[test]
    fn a_failed_transaction_is_still_an_event() {
        let mut e = envelope(5);
        e.success = Some(false);
        // Nothing about the type refuses it. A failed buy is often the first
        // sign a token cannot be traded.
        assert!(e.failed());
        assert!(!e.succeeded());
        // A rejection is an answer. Without this the mutant that makes
        // `outcome_unknown` always true survives, and every caller that gates
        // on it -- the whole feature pass -- goes silent on data it can read.
        assert!(!e.outcome_unknown());
    }

    #[test]
    fn unknown_is_neither_succeeded_nor_failed() {
        // The two questions are asked separately because the answers differ.
        // `!succeeded()` would call an unresolved row a failure, and a count of
        // on-chain failures that rises during an outage is a number about the
        // recorder wearing the label of a number about the chain.
        let mut e = envelope(5);
        e.success = None;
        assert!(!e.succeeded());
        assert!(!e.failed());
        assert!(e.outcome_unknown());
    }

    #[test]
    fn the_legacy_sentinels_read_back_as_unknown_together() {
        // Re-apply the bug by passing `Some(true)` beside the sentinel, which
        // is exactly the pair every row written before 2026-09-07 holds: the
        // backfill's join supplied both columns or neither, and the old writer
        // turned "neither" into a position and a success.
        let e = Envelope::from_stored(
            Slot(5),
            Signature::new([1u8; 64]),
            Some(LEGACY_UNKNOWN_TX_INDEX),
            2,
            None,
            Some(true),
        );
        assert_eq!(e.tx_index, None);
        assert!(
            e.outcome_unknown(),
            "a row with no resolved position had no `err` column read either"
        );

        // And a resolved row is untouched, so the translation cannot quietly
        // erase real history.
        let ok = Envelope::from_stored(
            Slot(5),
            Signature::new([1u8; 64]),
            Some(117),
            2,
            None,
            Some(true),
        );
        assert_eq!(ok.tx_index, Some(117));
        assert!(ok.succeeded());
        assert!(!ok.outcome_unknown());
    }

    #[test]
    fn a_directory_name_maps_back_to_its_own_table_and_nothing_else() {
        // The inverse of `dir`, and the coverage table's `table` column depends
        // on it. Re-apply by turning the `==` into `!=`: every name resolves to
        // whichever table is not it, which for a stored coverage row means a
        // range about trades reads back as a range about launches.
        for t in Table::ALL {
            assert_eq!(Table::from_dir(t.dir()), Some(*t), "{t:?}");
        }
        assert_eq!(Table::from_dir("not_a_table"), None);
        assert_eq!(Table::from_dir(""), None);
    }

    #[test]
    fn an_unknown_position_sorts_last_within_its_slot() {
        // `Option`'s own ordering puts `None` first, which would sort every
        // unresolved trade ahead of the launch it belongs to. Re-apply by
        // returning `(self.tx_index.is_some(), ..)`.
        let mut known = envelope(5);
        known.tx_index = Some(9);
        let mut unknown = envelope(5);
        unknown.tx_index = None;
        assert!(known.position_key() < unknown.position_key());
    }

    #[test]
    fn events_round_trip_through_json() {
        let e = trade(42);
        let s = serde_json::to_string(&e).expect("serialize");
        assert_eq!(serde_json::from_str::<Event>(&s).expect("deserialize"), e);
        assert!(s.contains("\"kind\":\"trade\""));
    }
}
