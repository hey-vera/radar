// SPDX-License-Identifier: Apache-2.0
//! Turning CryptoHouse rows into store events.
//!
//! Split deliberately from the HTTP client: everything here is a pure function
//! from rows to events, so the whole conversion — including every way it can
//! refuse — is unit-testable without a network.
//!
//! The rule throughout is that an event with the wrong mint is worse than a
//! missing event. Where a mint cannot be resolved unambiguously the row is
//! skipped and counted, never guessed at. [`Stats`] makes those gaps visible;
//! silently dropping them would leave the store looking like a quiet market.

use radar_decode::pumpfun;
use radar_decode::{Decoded, Discriminator, decode_pumpfun};
use radar_store::{Envelope, Event, Graduation, Launch, Origin, Side, Trade};
use radar_types::{Address, Signature, Slot};
use serde::Deserialize;

/// Mints that are never the subject of a pump.fun trade — they are the other
/// side of it. Without excluding these, every trade would resolve to wrapped SOL.
pub const QUOTE_MINTS: &[&str] = &[
    "So11111111111111111111111111111111111111112",
    "So11111111111111111111111111111111111111111",
    "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
    "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB",
];

/// A row from the extraction query.
///
/// ClickHouse renders 64-bit integers as strings in `JSONEachRow`, so everything
/// numeric arrives as text and is parsed here.
#[derive(Debug, Clone, Deserialize)]
pub struct Row {
    /// Slot the transaction landed in.
    pub slot: String,
    /// Transaction signature.
    pub sig: String,
    /// Index of the instruction within its transaction.
    pub ix_index: String,
    /// Index of the enclosing instruction, or `-1` for a top-level instruction.
    pub parent_index: String,
    /// Raw instruction data, base58.
    pub data: String,
    /// Distinct candidate mints found for the transaction.
    #[serde(default)]
    pub mints: Vec<String>,
    /// Mints this transaction **created**, from their `MintTo` transfer rows.
    ///
    /// Load-bearing for graduations and meaningless for launches, because the
    /// two are opposites: a `create` mints its own subject token, while a
    /// `migrate` mints a *different* token — the AMM's LP mint — alongside the
    /// subject it merely moves. See [`resolve_graduation_mint`].
    #[serde(default)]
    pub minted: Vec<String>,
    /// Mints named in the transaction's post-token balances.
    ///
    /// A second source for the subject, used only when the transfer rows cannot
    /// answer. CryptoHouse's `token_transfers` has no row at all for roughly a
    /// fifth of successful migrations, and the balances carry the mint for those.
    #[serde(default)]
    pub balance_mints: Vec<String>,
    /// Position of the transaction within its block, if resolved.
    #[serde(default)]
    pub tx_index: Option<String>,
    /// Whether the transaction succeeded, as `0` or `1`.
    #[serde(default)]
    pub ok: Option<String>,
}

/// Why a row produced no event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Skipped {
    /// No candidate mint survived filtering.
    NoMint,
    /// More than one candidate mint, so attribution would be a guess.
    AmbiguousMint,
    /// A graduation whose subject token could not be told from the LP mint.
    ///
    /// Counted apart from [`Self::AmbiguousMint`] because the two have opposite
    /// weight. A skipped trade is one of a million a day; a skipped graduation
    /// is one of about sixty an hour, and it is the numerator of the only
    /// unambiguously good outcome the store records. Folding them together is
    /// how the graduation table sat at four rows for a day while reading as a
    /// market with no graduations in it.
    GraduationSubjectUnresolved,
    /// The instruction data was not valid base58.
    BadData,
    /// The decoder does not know this instruction.
    UnknownInstruction,
    /// The instruction is known but carries nothing worth storing.
    NotAnEvent,
    /// The arguments could not be read.
    BadArguments,
    /// A slot, index or address field could not be parsed.
    BadField,
}

/// What an extraction run produced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stats {
    /// Rows that became events.
    pub emitted: u64,
    /// Rows skipped, by reason. Kept rather than summed: "no mint" is a data
    /// limitation and "unknown instruction" is a program upgrade, and treating
    /// them alike would hide the one that needs acting on.
    pub skipped: std::collections::BTreeMap<Skipped, u64>,
}

impl Stats {
    fn skip(&mut self, why: Skipped) {
        *self.skipped.entry(why).or_default() += 1;
    }

    /// Total rows skipped.
    #[must_use]
    pub fn total_skipped(&self) -> u64 {
        self.skipped.values().sum()
    }

    /// Share of rows that produced an event, or `None` if there were no rows.
    #[must_use]
    pub fn yield_rate(&self) -> Option<f64> {
        let total = self.emitted + self.total_skipped();
        #[expect(
            clippy::cast_precision_loss,
            reason = "a display ratio, not accounting"
        )]
        (total > 0).then(|| self.emitted as f64 / total as f64)
    }
}

/// The single mint a row is about, if that is unambiguous.
///
/// Quote assets are excluded first: a pump.fun trade always moves wrapped SOL or
/// a stablecoin as well as the token, and without this every trade in the store
/// would be attributed to wrapped SOL.
///
/// # Errors
///
/// Returns [`Skipped::NoMint`] if nothing survives the quote filter, or
/// [`Skipped::AmbiguousMint`] if more than one candidate does.
pub fn resolve_mint(candidates: &[String]) -> Result<&str, Skipped> {
    let mut subjects = candidates
        .iter()
        .filter(|m| !QUOTE_MINTS.contains(&m.as_str()));
    let Some(first) = subjects.next() else {
        return Err(Skipped::NoMint);
    };
    if subjects.next().is_some() {
        return Err(Skipped::AmbiguousMint);
    }
    Ok(first)
}

/// The token a **graduation** is about, which is not the one it creates.
///
/// A migration transaction moves two non-quote tokens, and the naive rule —
/// "the single mint that is not a quote asset" — sees two candidates and refuses
/// every one of them. Measured over one hour of chain: of 66 migrations, 49 had
/// two non-quote mints and 17 had none. Not one had exactly one, so not one
/// could be recorded.
///
/// The two are distinguishable structurally, and the distinction is the right
/// way round rather than a heuristic: the **LP mint is created by the migration**
/// — it appears as `MintTo` then `Burn` — while the **subject token predates it**
/// and only moves. So the subject is the non-quote mint this transaction did not
/// mint.
///
/// The amount is *not* usable for this. The subject's transfers summed to
/// `413,800,000,000,000` in one sample and `206,900,000,000,000` in another, so a
/// constant that looked like the bonding-curve remainder matched under half of
/// them.
///
/// Where the transfer rows are absent entirely, the post-token balances answer
/// instead. Measured over the same hour, the two together resolved **62 of 62**
/// successful migrations to exactly one mint.
///
/// # Errors
///
/// Returns [`Skipped::GraduationSubjectUnresolved`] if neither source leaves
/// exactly one candidate. Counted rather than guessed: a graduation attributed
/// to the LP mint is a graduation recorded against a token that never launched.
pub fn resolve_graduation_mint(row: &Row) -> Result<&str, Skipped> {
    let not_quote = |m: &&String| !QUOTE_MINTS.contains(&m.as_str());

    // The subject moved but was not minted here. Applying this to a launch would
    // be exactly wrong -- a `create` mints its subject, and 300 of 303 mints in
    // sampled launch transactions were minted in the transaction itself -- which
    // is why this is a graduation-only rule rather than a general one.
    let mut moved = row
        .mints
        .iter()
        .filter(not_quote)
        .filter(|m| !row.minted.contains(m));
    if let (Some(only), None) = (moved.next(), moved.next()) {
        return Ok(only);
    }

    let mut from_balances = row.balance_mints.iter().filter(not_quote);
    match (from_balances.next(), from_balances.next()) {
        (Some(only), None) => Ok(only),
        _ => Err(Skipped::GraduationSubjectUnresolved),
    }
}

fn parse_u64(s: &str) -> Result<u64, Skipped> {
    s.parse().map_err(|_| Skipped::BadField)
}

fn envelope(row: &Row) -> Result<Envelope, Skipped> {
    let parent = row
        .parent_index
        .parse::<i64>()
        .map_err(|_| Skipped::BadField)?;
    Ok(Envelope {
        slot: Slot(parse_u64(&row.slot)?),
        signature: row
            .sig
            .parse::<Signature>()
            .map_err(|_| Skipped::BadField)?,
        // Absent when the transactions join found nothing. Zero is a real block
        // position, so it must not stand in for "unknown" -- but the store needs
        // a value, and u32::MAX is not a position any block reaches.
        tx_index: row.tx_index.as_deref().map_or(Ok(u32::MAX), |v| {
            v.parse::<u32>().map_err(|_| Skipped::BadField)
        })?,
        instruction_index: u32::try_from(parse_u64(&row.ix_index)?)
            .map_err(|_| Skipped::BadField)?,
        parent_index: (parent >= 0).then(|| u32::try_from(parent).unwrap_or(u32::MAX)),
        // Absent means unknown; treating unknown as failed would discard real
        // activity, and treating it as succeeded would invent it. The join
        // supplies this for every row it resolves.
        succeeded: row.ok.as_deref() != Some("0"),
    })
}

/// Converts one row into an event.
///
/// # Errors
///
/// Returns [`Skipped`] describing why, which the caller counts rather than
/// discards.
pub fn event_from_row(row: &Row) -> Result<Event, Skipped> {
    let data = bs58::decode(&row.data)
        .into_vec()
        .map_err(|_| Skipped::BadData)?;

    let instruction = match decode_pumpfun(&data) {
        Decoded::Known(ix) => ix,
        Decoded::Unknown { .. } | Decoded::Malformed { .. } => {
            return Err(Skipped::UnknownInstruction);
        }
    };

    let envelope = envelope(row)?;
    // Which mint a row is about depends on what the instruction did to it. A
    // launch and a trade have one subject and the quote filter finds it; a
    // graduation has two non-quote tokens and needs to know which one it made.
    let mint: Address = if instruction.is_graduation() {
        resolve_graduation_mint(row)?
    } else {
        resolve_mint(&row.mints)?
    }
    .parse()
    .map_err(|_| Skipped::BadField)?;
    let origin = Origin::known(pumpfun::PROGRAM_ID, instruction.anchor_name());

    if instruction.is_launch() {
        let l = radar_decode::args::launch(&data).map_err(|_| Skipped::BadArguments)?;
        return Ok(Event::Launch(Box::new(Launch {
            envelope,
            origin,
            mint,
            creator: l.creator,
            name: l.name.to_owned(),
            symbol: l.symbol.to_owned(),
            uri: l.uri.to_owned(),
            // Recoverable only from balance deltas, which this query does not
            // fetch. None rather than zero: a creator who bought nothing and a
            // dev buy we did not measure are different facts.
            dev_buy_lamports: None,
        })));
    }

    if instruction.is_graduation() {
        return Ok(Event::Graduation(Box::new(Graduation {
            envelope,
            origin,
            mint,
        })));
    }

    if instruction.is_trade() {
        let (side, layout) = (
            instruction.side().ok_or(Skipped::NotAnEvent)?,
            instruction.layout().ok_or(Skipped::NotAnEvent)?,
        );
        let t =
            radar_decode::args::trade(&data, side, layout).map_err(|_| Skipped::BadArguments)?;
        return Ok(Event::Trade(Box::new(Trade {
            envelope,
            origin,
            mint,
            // The trader is an account key, which this query does not fetch.
            // Left as the mint's zero placeholder would be a lie, so the trade
            // records the program until the account join is added.
            trader: Address::SYSTEM_PROGRAM,
            side: match side {
                radar_decode::Side::Buy => Side::Buy,
                radar_decode::Side::Sell => Side::Sell,
            },
            realised_lamports: None,
            realised_tokens: None,
            requested_amount: t.exact.raw(),
            requested_is_lamports: t.exact.lamports().is_some(),
            limit_amount: t.limit.raw(),
            accepted_any_price: t.accepted_any_price(),
        })));
    }

    Err(Skipped::NotAnEvent)
}

/// Converts a batch of rows, counting what did not convert.
#[must_use]
pub fn events_from_rows(rows: &[Row]) -> (Vec<Event>, Stats) {
    let mut out = Vec::with_capacity(rows.len());
    let mut stats = Stats::default();
    for row in rows {
        match event_from_row(row) {
            Ok(e) => {
                out.push(e);
                stats.emitted += 1;
            }
            Err(why) => stats.skip(why),
        }
    }
    (out, stats)
}

/// What an extraction run asks for.
///
/// The public endpoint caps a result at a thousand rows and will not let a
/// readonly user raise it, so what is extractable is decided by row count rather
/// than by preference.
///
/// Launches run at roughly 24,000 a day — about an hour of chain per thousand
/// rows — so six months is a few thousand queries and a few hours. Trades run at
/// over a million a day, which is a thousand rows per twenty seconds of chain and
/// some 780,000 queries for the same period. That is not a slow extraction, it is
/// a different plan: per-mint aggregates instead, which is the granularity
/// outcome labels need anyway.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Scope {
    /// Launches and graduations — the token universe and its endings. Viable to
    /// extract in full.
    #[default]
    Lifecycle,
    /// Every individual trade. Viable only over narrow windows, for investigating
    /// specific periods rather than backfilling history.
    Trades,
    /// Graduations alone.
    ///
    /// For repairing a store whose launches are already correct. Re-running
    /// [`Self::Lifecycle`] over a window that has already been recorded would
    /// append a second copy of every launch in it, and the readers that
    /// deduplicate by mint would hide that from every count that does not.
    Graduations,
}

impl Scope {
    /// The discriminators this scope asks CryptoHouse for.
    #[must_use]
    pub fn discriminators(self) -> Vec<String> {
        pumpfun::KNOWN
            .iter()
            .map(|(ix, _, _)| *ix)
            .filter(|ix| match self {
                Self::Lifecycle => ix.is_launch() || ix.is_graduation(),
                Self::Trades => ix.is_trade(),
                Self::Graduations => ix.is_graduation(),
            })
            .map(|ix| Discriminator::to_string(&ix.discriminator()))
            .collect()
    }
}

/// Builds the extraction query for a time window.
///
/// Windowed on `block_timestamp` because that is what the table is partitioned
/// by; slot ranges would scan everything. The window is the caller's lever
/// against the server's sixty-second cap.
#[must_use]
pub fn query_for_window(from: &str, to: &str, scope: Scope) -> String {
    let discs = scope
        .discriminators()
        .iter()
        .map(|d| format!("'{d}'"))
        .collect::<Vec<_>>()
        .join(",");
    let program = pumpfun::PROGRAM_ID.to_string();
    let quotes = QUOTE_MINTS
        .iter()
        .map(|m| format!("'{m}'"))
        .collect::<Vec<_>>()
        .join(",");

    format!(
        "WITH ix AS (\
           SELECT tx_signature, block_slot, index AS ix_index, parent_index, data \
           FROM solana.instructions \
           WHERE program_id='{program}' \
             AND block_timestamp >= '{from}' AND block_timestamp < '{to}' \
             AND lower(hex(substring(base58Decode(data),1,8))) IN ({discs})\
         ), mints AS (\
           SELECT tx_signature, \
                  groupUniqArrayIf(mint, mint NOT IN ({quotes})) AS mints, \
                  groupUniqArrayIf(mint, transfer_type='MintTo') AS minted \
           FROM solana.token_transfers \
           WHERE block_timestamp >= '{from}' AND block_timestamp < '{to}' \
           GROUP BY tx_signature\
         ), txs AS (\
           SELECT signature, index AS tx_index, err, \
                  arrayDistinct(arrayMap(x -> x.2, post_token_balances)) AS balance_mints \
           FROM solana.transactions \
           WHERE block_timestamp >= '{from}' AND block_timestamp < '{to}'\
         ) \
         SELECT toString(ix.block_slot) AS slot, ix.tx_signature AS sig, \
                toString(ix.ix_index) AS ix_index, toString(ix.parent_index) AS parent_index, \
                ix.data AS data, mints.mints AS mints, mints.minted AS minted, \
                txs.balance_mints AS balance_mints, \
                toString(txs.tx_index) AS tx_index, toString(txs.err = '') AS ok \
         FROM ix LEFT JOIN mints ON ix.tx_signature = mints.tx_signature \
                 LEFT JOIN txs ON ix.tx_signature = txs.signature"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(data_b58: &str, mints: &[&str]) -> Row {
        Row {
            slot: "440624677".into(),
            sig: Signature::new([3u8; 64]).to_string(),
            ix_index: "2".into(),
            parent_index: "-1".into(),
            data: data_b58.into(),
            mints: mints.iter().map(|s| (*s).to_owned()).collect(),
            minted: Vec::new(),
            balance_mints: Vec::new(),
            tx_index: Some("117".into()),
            ok: Some("1".into()),
        }
    }

    /// A real `migrate_v2` row, captured from mainnet slot 441251921.
    ///
    /// Kept verbatim rather than synthesised, because the shape it holds is the
    /// whole finding: two non-quote mints, one of which this transaction created.
    fn real_migration_row() -> Row {
        Row {
            slot: "441251921".into(),
            sig: "mZpcwJN6kTd7BxdNd5dDQ7EJpBR2JRBZb8BsZSSXWr2wKb6Rn8PNxE75Guji2BkFsRwWPAFQPmDkwJa7CPbJcX6".into(),
            ix_index: "2".into(),
            parent_index: "-1".into(),
            data: "YQq8B6nbicx".into(),
            mints: vec![
                MIGRATED.to_owned(),
                LP_MINT.to_owned(),
            ],
            minted: vec![LP_MINT.to_owned()],
            balance_mints: vec![
                MIGRATED.to_owned(),
                QUOTE_MINTS[0].to_owned(),
            ],
            tx_index: Some("110".into()),
            ok: Some("1".into()),
        }
    }

    /// The token that graduated in the captured transaction.
    const MIGRATED: &str = "2Rt18SqHXcgzUU1P94Qr71A9URcpmkwB99cD5SGXpump";
    /// The AMM LP mint the same transaction created.
    const LP_MINT: &str = "Dck97H5qwyztdKm9hmfXhQjbT7uZGLsFFGo5du3ToE4U";

    #[test]
    fn a_graduation_resolves_to_the_token_it_moved_not_the_one_it_minted() {
        let row = real_migration_row();
        // The naive rule sees two candidates and refuses. This is the whole bug:
        // every graduation on chain took that path, so the table held four rows
        // where roughly 1,480 events had happened.
        assert_eq!(resolve_mint(&row.mints), Err(Skipped::AmbiguousMint));
        assert_eq!(resolve_graduation_mint(&row), Ok(MIGRATED));
    }

    #[test]
    fn the_whole_row_becomes_a_graduation_for_the_right_mint() {
        let event = event_from_row(&real_migration_row()).expect("a graduation");
        assert!(matches!(event, Event::Graduation(_)));
        assert_eq!(event.mint().to_string(), MIGRATED);
    }

    #[test]
    fn a_graduation_falls_back_to_balances_when_there_are_no_transfer_rows() {
        // CryptoHouse has no `token_transfers` row at all for roughly a fifth of
        // successful migrations. Without this fallback those are lost silently,
        // which is the same failure one layer down.
        let mut row = real_migration_row();
        row.mints.clear();
        row.minted.clear();
        assert_eq!(resolve_graduation_mint(&row), Ok(MIGRATED));
    }

    #[test]
    fn a_graduation_that_cannot_be_told_apart_is_refused_with_its_own_reason() {
        // Not AmbiguousMint. A skipped trade is one of a million a day; a skipped
        // graduation is one of about sixty an hour and is the numerator of the
        // only good outcome recorded, so the two must not share a counter.
        let mut row = real_migration_row();
        row.minted.clear();
        row.balance_mints = vec![MIGRATED.to_owned(), LP_MINT.to_owned()];
        assert_eq!(
            resolve_graduation_mint(&row),
            Err(Skipped::GraduationSubjectUnresolved)
        );
    }

    #[test]
    fn a_launch_still_resolves_to_the_mint_it_created() {
        // The graduation rule inverted would destroy the half that works: a
        // `create` mints its own subject, and 300 of 303 mints in sampled launch
        // transactions were minted in the transaction itself. So the rule must
        // stay keyed on the instruction, never applied to every row.
        let mut row = row(&launch_data(), &[PUMP]);
        row.minted = vec![PUMP.to_owned()];
        let event = event_from_row(&row).expect("a launch");
        assert!(matches!(event, Event::Launch(_)));
        assert_eq!(event.mint().to_string(), PUMP);
    }

    fn launch_data() -> String {
        let mut d = pumpfun::Instruction::CreateV2
            .discriminator()
            .as_bytes()
            .to_vec();
        for s in ["Coin", "CN", "https://example.invalid/m.json"] {
            d.extend_from_slice(&u32::try_from(s.len()).expect("short").to_le_bytes());
            d.extend_from_slice(s.as_bytes());
        }
        d.extend_from_slice(&[7u8; 32]);
        bs58::encode(d).into_string()
    }

    fn trade_data() -> String {
        let mut d = pumpfun::Instruction::Buy
            .discriminator()
            .as_bytes()
            .to_vec();
        d.extend_from_slice(&1_000_000u64.to_le_bytes());
        d.extend_from_slice(&150_000_000u64.to_le_bytes());
        bs58::encode(d).into_string()
    }

    const PUMP: &str = "5NfV2sy8DqXamLvYEE4LcTWzGqZc5Emv4bqqhVDWpump";

    #[test]
    fn a_quote_mint_is_never_the_subject_of_a_trade() {
        // Every pump.fun trade also moves wrapped SOL. Without this filter the
        // entire store would be attributed to wrapped SOL.
        assert_eq!(
            resolve_mint(&["So11111111111111111111111111111111111111112".into()]),
            Err(Skipped::NoMint)
        );
        assert_eq!(
            resolve_mint(&[
                "So11111111111111111111111111111111111111112".into(),
                PUMP.into()
            ]),
            Ok(PUMP)
        );
    }

    #[test]
    fn two_candidate_mints_are_refused_rather_than_guessed() {
        // An event with the wrong mint is worse than a missing event: it
        // attributes real activity to a token that never saw it.
        let two = vec![
            PUMP.to_owned(),
            "AnotherMintAddressThatIsDifferent1111111111".to_owned(),
        ];
        assert_eq!(resolve_mint(&two), Err(Skipped::AmbiguousMint));
    }

    #[test]
    fn a_launch_row_becomes_a_launch_event() {
        let e = event_from_row(&row(&launch_data(), &[PUMP])).expect("converts");
        let Event::Launch(l) = e else {
            panic!("expected a launch")
        };
        assert_eq!(l.name, "Coin");
        assert_eq!(l.symbol, "CN");
        assert_eq!(l.creator, Address::new([7u8; 32]));
        assert_eq!(l.mint.to_string(), PUMP);
        assert_eq!(l.envelope.slot, Slot(440_624_677));
        assert_eq!(l.envelope.tx_index, 117);
        // Top-level instruction: parent_index of -1 must become None, not 4294967295.
        assert_eq!(l.envelope.parent_index, None);
        // Not measured by this query, and not faked as zero.
        assert_eq!(l.dev_buy_lamports, None);
    }

    #[test]
    fn a_trade_row_keeps_the_unit_of_what_the_trader_pinned() {
        let e = event_from_row(&row(&trade_data(), &[PUMP])).expect("converts");
        let Event::Trade(t) = e else {
            panic!("expected a trade")
        };
        assert_eq!(t.side, Side::Buy);
        // `buy` pins tokens and bounds SOL.
        assert!(!t.requested_is_lamports);
        assert_eq!(t.requested_amount, 1_000_000);
        assert_eq!(t.limit_amount, 150_000_000);
        assert!(!t.accepted_any_price);
    }

    #[test]
    fn a_nested_instruction_keeps_its_parent() {
        let mut r = row(&trade_data(), &[PUMP]);
        r.parent_index = "3".into();
        let e = event_from_row(&r).expect("converts");
        assert_eq!(e.envelope().parent_index, Some(3));
    }

    #[test]
    fn a_failed_transaction_is_recorded_as_failed_rather_than_skipped() {
        let mut r = row(&trade_data(), &[PUMP]);
        r.ok = Some("0".into());
        let e = event_from_row(&r).expect("converts");
        assert!(!e.envelope().succeeded);
    }

    #[test]
    fn an_unresolvable_transaction_position_is_not_reported_as_the_first_in_its_block() {
        // Zero is a real block position. Using it for "unknown" would make every
        // unresolved transaction look like it led its slot, which is exactly the
        // signal coordination analysis reads.
        let mut r = row(&trade_data(), &[PUMP]);
        r.tx_index = None;
        let e = event_from_row(&r).expect("converts");
        assert_eq!(e.envelope().tx_index, u32::MAX);
    }

    #[test]
    fn rows_with_no_mint_are_counted_rather_than_dropped_silently() {
        let rows = vec![row(&launch_data(), &[PUMP]), row(&launch_data(), &[])];
        let (events, stats) = events_from_rows(&rows);
        assert_eq!(events.len(), 1);
        assert_eq!(stats.emitted, 1);
        assert_eq!(stats.skipped.get(&Skipped::NoMint), Some(&1));
        assert_eq!(stats.yield_rate(), Some(0.5));
    }

    #[test]
    fn an_unknown_instruction_is_skipped_under_its_own_reason() {
        // Distinct from a data limitation: a rising unknown count is a program
        // upgrade and needs acting on, where a rising no-mint count does not.
        let unknown = bs58::encode([0xAAu8; 24]).into_string();
        let (_, stats) = events_from_rows(&[row(&unknown, &[PUMP])]);
        assert_eq!(stats.skipped.get(&Skipped::UnknownInstruction), Some(&1));
    }

    #[test]
    fn malformed_base58_is_skipped_rather_than_panicking() {
        let (_, stats) = events_from_rows(&[row("0OIl not base58", &[PUMP])]);
        assert_eq!(stats.skipped.get(&Skipped::BadData), Some(&1));
    }

    #[test]
    fn the_lifecycle_scope_leaves_out_trades() {
        // Trades are three orders of magnitude more numerous and would blow the
        // thousand-row cap in twenty seconds of chain.
        let discs = Scope::Lifecycle.discriminators();
        assert_eq!(
            discs.len(),
            4,
            "two launch paths, two graduation paths: {discs:?}"
        );
        assert_eq!(
            Scope::Trades.discriminators().len(),
            6,
            "four buys, two sells"
        );
        assert_eq!(Scope::default(), Scope::Lifecycle);

        // The repair scope asks for graduations and nothing else, so re-running
        // it over an already-recorded window cannot append a second copy of
        // every launch in it.
        let repair = Scope::Graduations.discriminators();
        assert_eq!(repair.len(), 2, "migrate and migrate_v2: {repair:?}");
        for d in &repair {
            assert!(
                discs.contains(d),
                "graduation scope must be a subset of lifecycle: {d}"
            );
        }
        assert!(
            Scope::Graduations
                .discriminators()
                .iter()
                .all(|d| !Scope::Trades.discriminators().contains(d)),
            "a graduation is not a trade"
        );

        let sql = query_for_window(
            "2026-08-21 06:00:00",
            "2026-08-21 06:02:00",
            Scope::Lifecycle,
        );
        assert!(sql.contains(&pumpfun::PROGRAM_ID.to_string()));
        // The two columns graduation resolution depends on. Without `minted` the
        // subject cannot be told from the LP mint, and without the balance
        // fallback the migrations that have no transfer rows are lost — so a
        // query missing either silently reinstates the bug.
        assert!(
            sql.contains("transfer_type='MintTo'"),
            "missing minted column"
        );
        assert!(
            sql.contains("post_token_balances"),
            "missing balance fallback"
        );
        // Quote mints excluded server-side keeps the payload small.
        assert!(sql.contains("So11111111111111111111111111111111111111112"));
        for d in &discs {
            assert!(sql.contains(d.as_str()), "query is missing {d}");
        }
    }
}
