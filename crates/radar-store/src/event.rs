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
//! recorded with `succeeded: false` rather than dropped.

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
    /// Position of the transaction within its block.
    ///
    /// The input to same-slot clustering: coordination analysis needs to know
    /// which transactions landed together and in what order, and that ordering
    /// exists nowhere else once the block is discarded.
    pub tx_index: u32,
    /// Position of the instruction within its transaction.
    pub instruction_index: u32,
    /// The enclosing instruction, if this one was a cross-program invocation.
    pub parent_index: Option<u32>,
    /// Whether the transaction succeeded.
    ///
    /// A failed buy is information about a token, not the absence of one.
    pub succeeded: bool,
}

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
    /// The account that traded.
    pub trader: Address,
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
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
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
}

impl Table {
    /// Every table, for iteration.
    /// Every table, including outcomes. For directory setup and file listings —
    /// **not** for read loops. See [`EVENT_TABLES`](Self::EVENT_TABLES).
    pub const ALL: &'static [Self] = &[
        Self::Launches,
        Self::Trades,
        Self::Graduations,
        Self::Outcomes,
        Self::Decisions,
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
            Self::Launches | Self::Trades | Self::Graduations => "slot",
            Self::Outcomes => "measured_at",
            Self::Decisions => "decided_at",
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(slot: u64) -> Envelope {
        Envelope {
            slot: Slot(slot),
            signature: Signature::new([1u8; 64]),
            tx_index: 7,
            instruction_index: 2,
            parent_index: None,
            succeeded: true,
        }
    }

    fn trade(slot: u64) -> Event {
        Event::Trade(Box::new(Trade {
            envelope: envelope(slot),
            origin: Origin::known(Address::SYSTEM_PROGRAM, "buy"),
            mint: Address::new([9u8; 32]),
            trader: Address::new([8u8; 32]),
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
        e.succeeded = false;
        // Nothing about the type refuses it. A failed buy is often the first
        // sign a token cannot be traded.
        assert!(!e.succeeded);
    }

    #[test]
    fn events_round_trip_through_json() {
        let e = trade(42);
        let s = serde_json::to_string(&e).expect("serialize");
        assert_eq!(serde_json::from_str::<Event>(&s).expect("deserialize"), e);
        assert!(s.contains("\"kind\":\"trade\""));
    }
}
