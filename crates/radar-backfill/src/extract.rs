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

use std::time::Duration;

use radar_decode::pumpfun;
use radar_decode::{Decoded, Discriminator, Instruction, Program, decode};
use radar_store::{Envelope, Event, Graduation, Launch, Origin, Side, Table, Trade, from_epoch};
use radar_types::{Address, Signature, Slot};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::cryptohouse::{Client, QueryError};

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
        // Absent when the transactions join found nothing. The store now
        // carries that absence rather than a sentinel: this was `u32::MAX`
        // until 2026-09-07, and `longest_run` over a set containing it counted
        // it as a block position.
        tx_index: row
            .tx_index
            .as_deref()
            .map(|v| v.parse::<u32>().map_err(|_| Skipped::BadField))
            .transpose()?,
        instruction_index: u32::try_from(parse_u64(&row.ix_index)?)
            .map_err(|_| Skipped::BadField)?,
        parent_index: (parent >= 0).then(|| u32::try_from(parent).unwrap_or(u32::MAX)),
        // Absent means unknown, and it is now recorded as unknown. Treating it
        // as failed would discard real activity; treating it as succeeded --
        // which `row.ok.as_deref() != Some("0")` did until 2026-09-07 --
        // invents it, and that is the direction that puts an outage into a
        // feature as measured activity. The join supplies this for every row it
        // resolves.
        success: row.ok.as_deref().map(|v| v != "0"),
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

    // The program is named here rather than inferred from the bytes, because
    // the bytes cannot name it: pump.fun and PumpSwap share seven discriminators
    // exactly, so `buy` from either is the same eight bytes.
    //
    // `Program::PumpFun` is true of these rows because the CryptoHouse query
    // filters on that program in SQL, and `Row` carries no program column to
    // read it back from. That makes this the one place the two must agree, and
    // agreeing by construction beats agreeing by comment: **when the query
    // learns a second venue, this argument has to come from the row**, or every
    // PumpSwap trade it returns will be recorded as a bonding-curve one.
    let instruction = match decode(Program::PumpFun, &data) {
        Decoded::Known(Instruction::PumpFun(ix)) => ix,
        // `decode` answers in the program it was given, so a PumpSwap arm here
        // would mean the decoder ignored its own argument. Folded into the same
        // refusal as unknown bytes rather than asserted: a backfill row is not
        // worth a panic, and the row is skipped either way.
        Decoded::Known(_) | Decoded::Unknown { .. } | Decoded::Malformed { .. } => {
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
            // Recorded as unknown until the account join is added: the system
            // program stood in for it until 2026-09-07, which made every
            // distinct-trader count over backfilled data equal to one.
            trader: None,
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
    /// The store tables a window under this scope collects into.
    ///
    /// Beside [`discriminators`](Self::discriminators) and matching the same
    /// way: the two together are the whole of what a scope means, and a scope
    /// that asked for an instruction whose events land in a table nobody
    /// recorded coverage for would report a gap as a quiet market. Exhaustive,
    /// so a fourth scope stops compiling here rather than silently covering
    /// nothing.
    #[must_use]
    pub const fn tables(self) -> &'static [Table] {
        match self {
            Self::Lifecycle => &[Table::Launches, Table::Graduations],
            Self::Trades => &[Table::Trades],
            Self::Graduations => &[Table::Graduations],
        }
    }

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

/// Fetches one time window, halving it whenever the server says it was too
/// wide, and stitching the two halves back together in order.
///
/// Pulled out of the backfill runner (where it lived as a private `fn` fitted
/// to [`Row`] and [`Scope`]) and made generic, so a second caller with a
/// different `SELECT` — the public market endpoints in `radar-serve`, among
/// them — gets the same halving discipline rather than a second copy of it.
/// `query` builds the SQL text for a `from .. to` pair; everything about *what*
/// is being asked for lives in the closure, and everything about *how wide a
/// bite the endpoint will take* lives here.
///
/// `depth` bottoms out at 10, matching what the extractor already accepted:
/// past that, a window this narrow still failing means something other than
/// "too wide", and retrying deeper would only hammer a public endpoint we are
/// a guest on (ADR 0002).
///
/// # Errors
///
/// Returns the server's [`QueryError`] once the window cannot be narrowed any
/// further, or once the halving depth is exhausted.
pub fn fetch_windowed<T: DeserializeOwned>(
    client: &Client,
    from: i64,
    to: i64,
    depth: u32,
    min_window_seconds: i64,
    pause_between: Duration,
    query: &dyn Fn(&str, &str) -> String,
) -> Result<Vec<T>, QueryError> {
    narrowing_fetch(
        &|sql| client.query::<T>(sql),
        from,
        to,
        depth,
        min_window_seconds,
        pause_between,
        query,
    )
}

/// The deepest a window may be halved before the failure is reported instead.
///
/// Ten halvings takes a five-minute window below a third of a second, so a
/// window still failing here is failing for a reason narrowing cannot fix.
const MAX_NARROWING_DEPTH: u32 = 10;

/// [`fetch_windowed`] with the transport supplied by the caller.
///
/// **Split out on 2026-09-11 because the decision this makes had no test and
/// could not have one.** Every branch here — whether an error narrows, whether
/// the window is still wide enough to halve, whether the depth is exhausted,
/// and that both halves are fetched and joined in order — was fused to a live
/// HTTP client, so exercising it meant querying a public endpoint. It was
/// therefore never exercised, and a bug in `Client::query` that stopped
/// `should_narrow` ever returning true went unnoticed for as long as the
/// `trades` table stayed empty.
///
/// `run` takes the SQL and returns rows or the server's error. That is the
/// whole seam: a test supplies one that fails on a wide window and succeeds on
/// a narrow one, and every branch below becomes reachable without a network.
///
/// **A caller may also use `run` to bound what a narrowing fetch costs.** The
/// halving below is unbounded in queries — a window over the row cap becomes
/// two queries, and each of those may become two more — so a caller with a
/// quota must count them somewhere, and `run` is the only place every one of
/// them passes through. A `run` that starts refusing with an error whose
/// [`QueryError::should_narrow`] is false stops the recursion at once and
/// surfaces as an ordinary failure, which the caller then records as the
/// partial coverage it is. `radar_backfill::market_tape::Budget` does exactly
/// this.
///
/// # Errors
///
/// Returns whatever `run` last returned: once the window cannot be narrowed
/// further, once the halving depth is spent, or at once for an error
/// narrowing cannot fix.
pub fn narrowing_fetch<T>(
    run: &dyn Fn(&str) -> Result<Vec<T>, QueryError>,
    from: i64,
    to: i64,
    depth: u32,
    min_window_seconds: i64,
    pause_between: Duration,
    query: &dyn Fn(&str, &str) -> String,
) -> Result<Vec<T>, QueryError> {
    let sql = query(&from_epoch(from), &from_epoch(to));
    match run(&sql) {
        Ok(rows) => Ok(rows),
        Err(e)
            if e.should_narrow()
                && (to - from) > min_window_seconds
                && depth < MAX_NARROWING_DEPTH =>
        {
            let mid = from + (to - from) / 2;
            eprintln!(
                "    window too wide ({}), halving: {} .. {}",
                if e.to_string().contains("TOO_MANY_ROWS") {
                    "row cap"
                } else {
                    "timeout"
                },
                from_epoch(from),
                from_epoch(to)
            );
            // The earlier half first, and its rows kept, so a partial success
            // is still progress in chronological order. A failure in either
            // half propagates: half a window reported as a whole one is the
            // silent gap this project is most organised against.
            let mut rows = narrowing_fetch(
                run,
                from,
                mid,
                depth + 1,
                min_window_seconds,
                pause_between,
                query,
            )?;
            std::thread::sleep(pause_between);
            rows.extend(narrowing_fetch(
                run,
                mid,
                to,
                depth + 1,
                min_window_seconds,
                pause_between,
                query,
            )?);
            Ok(rows)
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod narrowing {
    use std::cell::RefCell;
    use std::time::Duration;

    use super::{QueryError, narrowing_fetch};

    /// The query builder every test here uses: the window, verbatim, so an
    /// assertion can read back exactly which spans were asked for.
    fn spans(from: &str, to: &str) -> String {
        format!("{from}..{to}")
    }

    /// A fake transport that records every window it was asked for and fails
    /// any wider than `widest_ok` seconds with the row-cap error.
    ///
    /// The row-cap text matters: [`QueryError::should_narrow`] matches on it,
    /// and a fake that returned some other server error would exercise the
    /// give-up path while looking like it exercised narrowing.
    struct Endpoint {
        widest_ok: i64,
        asked: RefCell<Vec<String>>,
    }

    impl Endpoint {
        fn new(widest_ok: i64) -> Self {
            Self {
                widest_ok,
                asked: RefCell::new(Vec::new()),
            }
        }

        /// One row per successful window, carrying the span, so the caller's
        /// concatenation order is observable.
        fn run(&self, sql: &str) -> Result<Vec<String>, QueryError> {
            self.asked.borrow_mut().push(sql.to_owned());
            let (from, to) = sql
                .split_once("..")
                .expect("the fake query builder's shape");
            let width = width_seconds(from, to);
            if width > self.widest_ok {
                Err(QueryError::Server(
                    "Code: 396 ... (TOO_MANY_ROWS_OR_BYTES)".to_owned(),
                ))
            } else {
                Ok(vec![sql.to_owned()])
            }
        }
    }

    /// Seconds between two `YYYY-MM-DD HH:MM:SS` stamps, for the fake only.
    fn width_seconds(from: &str, to: &str) -> i64 {
        let secs = |s: &str| {
            let time = s.split(' ').nth(1).expect("a time part");
            let mut parts = time.split(':').map(|p| p.parse::<i64>().expect("numeric"));
            let h = parts.next().expect("hours");
            let m = parts.next().expect("minutes");
            let sec = parts.next().expect("seconds");
            h * 3600 + m * 60 + sec
        };
        secs(to) - secs(from)
    }

    fn fetch(
        endpoint: &Endpoint,
        from: i64,
        to: i64,
        min_window_seconds: i64,
    ) -> Result<Vec<String>, QueryError> {
        narrowing_fetch(
            &|sql| endpoint.run(sql),
            from,
            to,
            0,
            min_window_seconds,
            Duration::ZERO,
            &spans,
        )
    }

    /// A window inside the cap is fetched once and never halved.
    #[test]
    fn a_window_that_fits_is_asked_for_exactly_once() {
        let endpoint = Endpoint::new(600);
        let rows = fetch(&endpoint, 0, 600, 4).expect("fits");
        assert_eq!(rows.len(), 1);
        assert_eq!(endpoint.asked.borrow().len(), 1, "no halving was needed");
    }

    /// A window over the cap is halved until each piece fits, and the pieces
    /// come back joined in chronological order.
    ///
    /// This is the behaviour the `trades` table needed and never got. It kills
    /// the mutants that flip the narrowing guard to a constant, that swap the
    /// `&&`s for `||`s, and that change `from + (to - from) / 2` into anything
    /// that is not the midpoint -- a wrong midpoint either loses a span or
    /// repeats one, and both are visible in the joined result.
    #[test]
    fn a_window_over_the_cap_is_halved_until_each_piece_fits() {
        let endpoint = Endpoint::new(300);
        let rows = fetch(&endpoint, 0, 1200, 4).expect("narrows");
        assert_eq!(
            rows,
            vec![
                "1970-01-01 00:00:00..1970-01-01 00:05:00",
                "1970-01-01 00:05:00..1970-01-01 00:10:00",
                "1970-01-01 00:10:00..1970-01-01 00:15:00",
                "1970-01-01 00:15:00..1970-01-01 00:20:00",
            ],
            "four contiguous five-minute pieces, earliest first, none repeated"
        );
    }

    /// Narrowing stops at the floor and reports the failure rather than
    /// halving forever.
    ///
    /// Kills the mutants that relax `(to - from) > min_window_seconds` to
    /// `>=`, `<`, or `==`: each changes which window is the last one tried.
    #[test]
    fn a_window_that_cannot_fit_stops_at_the_floor_and_fails() {
        let endpoint = Endpoint::new(0);
        let err = fetch(&endpoint, 0, 64, 8).expect_err("nothing fits");
        assert!(err.should_narrow(), "the failure reported is the row cap");
        // 64 -> 32 -> 16 -> 8, and 8 is not > 8, so the last windows tried are
        // eight seconds wide. Anything narrower means the floor was ignored.
        let asked = endpoint.asked.borrow();
        let narrowest = asked
            .iter()
            .map(|sql| {
                let (f, t) = sql.split_once("..").expect("shape");
                width_seconds(f, t)
            })
            .min()
            .expect("at least one window");
        assert_eq!(narrowest, 8, "the floor is the narrowest window attempted");
    }

    /// The halving stops at the depth limit rather than recursing forever.
    ///
    /// Kills the mutants that turn `depth + 1` into `depth * 1` -- which
    /// leaves the depth at zero so the limit is never reached -- and the one
    /// that relaxes `depth < MAX_NARROWING_DEPTH` to `<=`. With a floor of one
    /// second and a wide window, the depth limit is what ends the recursion,
    /// and ten halvings of a wide span is a bounded, countable number of
    /// queries.
    #[test]
    fn the_halving_stops_at_the_depth_limit_not_at_the_floor() {
        let calls = RefCell::new(0u32);
        let err = narrowing_fetch(
            &|_sql| {
                *calls.borrow_mut() += 1;
                Err::<Vec<String>, _>(QueryError::Server(
                    "Code: 396 (TOO_MANY_ROWS_OR_BYTES)".to_owned(),
                ))
            },
            0,
            1 << 20,
            0,
            1,
            Duration::ZERO,
            &spans,
        )
        .expect_err("nothing ever fits");
        assert!(err.should_narrow());
        // Eleven: the original window and ten halvings, straight down the
        // left spine. Not a full tree, because the first half's failure
        // propagates with `?` and the right half is never asked for -- which
        // is the property worth having. Half a window returned as a whole one
        // is the silent gap this project is most organised against, so a
        // failure anywhere aborts the lot rather than reporting what it got.
        //
        // With `depth + 1` mutated to `depth * 1` the depth never advances and
        // this runs to the four-second floor instead, which is far more than
        // eleven; with `<` relaxed to `<=` it is twelve.
        let issued = *calls.borrow();
        assert_eq!(
            issued, 11,
            "the first window plus ten halvings, and no right halves after a failure"
        );
    }

    /// The midpoint is the middle, and both halves are asked for.
    ///
    /// Kills the mutant that turns `from + (to - from) / 2` into an addition
    /// of the two bounds: with a non-zero `from` that lands outside the window
    /// entirely, so one half is never asked for and the other is asked for
    /// twice. A window starting at zero would hide it, which is why this one
    /// does not start at zero.
    #[test]
    fn the_midpoint_splits_the_window_it_was_given_not_the_one_at_zero() {
        let endpoint = Endpoint::new(300);
        let rows = fetch(&endpoint, 3_600, 4_200, 4).expect("narrows once");
        assert_eq!(
            rows,
            vec![
                "1970-01-01 01:00:00..1970-01-01 01:05:00",
                "1970-01-01 01:05:00..1970-01-01 01:10:00",
            ],
            "two contiguous halves of the window actually asked about"
        );
    }

    /// The floor compares the window's *width*, not the sum of its bounds.
    ///
    /// Every other test here starts at zero, where `to - from` and `to + from`
    /// are the same number, so the mutant replacing one with the other
    /// survives all of them. This window starts an hour in: its width is
    /// sixteen seconds and its bounds sum to over seven thousand, so a floor
    /// of thirty-two stops it immediately under subtraction and never under
    /// addition.
    #[test]
    fn the_floor_measures_the_window_not_the_sum_of_its_bounds() {
        let endpoint = Endpoint::new(0);
        let calls_before = endpoint.asked.borrow().len();
        let err = fetch(&endpoint, 3_600, 3_616, 32).expect_err("nothing fits");
        assert!(err.should_narrow(), "the row cap is what was reported");
        assert_eq!(
            endpoint.asked.borrow().len() - calls_before,
            1,
            "a sixteen-second window is already under a thirty-two second \
             floor and must not be halved at all"
        );
    }

    /// Both halves carry the depth forward, not just the first.
    ///
    /// The depth test above walks the left spine only, because a failure there
    /// propagates before the right half is ever asked for. This one succeeds
    /// on the left and fails on the right, so the second recursive call's
    /// `depth + 1` is the one being exercised -- the mutant turning it into
    /// `depth * 1` leaves that branch's depth at zero and lets it halve far
    /// past the limit.
    #[test]
    fn the_second_half_carries_the_depth_forward_too() {
        // Fails **only** for a window ending at the very top, so every left
        // half succeeds and the recursion walks down the right spine. That is
        // what makes the second recursive call's `depth + 1` the one under
        // test: with a fake that failed on the left, the failure propagates
        // before the right call is reached at all, and the mutant survives.
        struct Lopsided {
            calls: RefCell<u32>,
        }
        let endpoint = Lopsided {
            calls: RefCell::new(0),
        };
        let err = narrowing_fetch(
            &|sql: &str| {
                *endpoint.calls.borrow_mut() += 1;
                // Keyed on where the window *ends*, not where it starts. An
                // earlier version read `from`, so the first window began at
                // zero, succeeded outright, and the halving under test never
                // ran at all.
                let (_from, to) = sql.split_once("..").expect("the fake's shape");
                let secs = to
                    .split(' ')
                    .nth(1)
                    .and_then(|t| {
                        let mut p = t.split(':').map(|x| x.parse::<i64>().ok());
                        Some(p.next()?? * 3600 + p.next()?? * 60 + p.next()??)
                    })
                    .expect("a time");
                if secs == 86_000 {
                    Err(QueryError::Server(
                        "Code: 396 (TOO_MANY_ROWS_OR_BYTES)".to_owned(),
                    ))
                } else {
                    Ok(vec![sql.to_owned()])
                }
            },
            0,
            86_000,
            0,
            1,
            Duration::ZERO,
            &spans,
        )
        .expect_err("the upper half never fits");
        assert!(err.should_narrow());
        // The span is deliberately far wider than the floor: 86,000 seconds
        // halves about seventeen times before reaching one second, but the
        // depth limit is ten. So the two stop at clearly different points, and
        // a right half whose depth never advances runs deeper than one whose
        // does. An earlier version used 1,024 seconds, where the floor and the
        // depth limit coincide — the mutant survived it. It is also under a
        // day, because this fake's clock parser reads only the time part of a
        // stamp and a multi-day window would read as a few hours.
        let calls = *endpoint.calls.borrow();
        assert!(
            calls <= 21,
            "the depth limit bounds the right half too, got {calls} calls"
        );
    }

    /// An error that narrowing cannot fix is reported at once, not retried on
    /// a narrower window.
    ///
    /// Retrying a bad identifier would hammer a public endpoint with the same
    /// broken query, which is what `should_narrow` exists to prevent. Kills
    /// the mutant that replaces the whole guard with `true`.
    #[test]
    fn an_error_that_narrowing_cannot_fix_is_not_retried() {
        fn broken(_sql: &str) -> Result<Vec<String>, QueryError> {
            Err(QueryError::Server(
                "Code: 47 ... (UNKNOWN_IDENTIFIER)".to_owned(),
            ))
        }
        let calls = RefCell::new(0u32);
        let err = narrowing_fetch(
            &|sql| {
                *calls.borrow_mut() += 1;
                broken(sql)
            },
            0,
            1200,
            0,
            4,
            Duration::ZERO,
            &spans,
        )
        .expect_err("a bad query stays bad");
        assert!(!err.should_narrow());
        assert_eq!(*calls.borrow(), 1, "asked once, not narrowed");
    }
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
        assert_eq!(l.envelope.tx_index, Some(117));
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
        assert!(e.envelope().failed());
    }

    #[test]
    fn an_unresolvable_transaction_position_is_not_reported_as_the_first_in_its_block() {
        // Zero is a real block position. Using it for "unknown" would make every
        // unresolved transaction look like it led its slot, which is exactly the
        // signal coordination analysis reads. It was `u32::MAX` until
        // 2026-09-07, which reads as a position rather than as an absence to
        // anything that does arithmetic on it -- `longest_run` did.
        let mut r = row(&trade_data(), &[PUMP]);
        r.tx_index = None;
        let e = event_from_row(&r).expect("converts");
        assert_eq!(e.envelope().tx_index, None);
    }

    #[test]
    fn an_unresolved_transaction_is_not_recorded_as_a_successful_one() {
        // Re-apply by restoring `row.ok.as_deref() != Some("0")`: with no `ok`
        // column that expression is `true`, so an outage in the transactions
        // join became successful on-chain activity in every feature built from
        // these rows.
        let mut r = row(&trade_data(), &[PUMP]);
        r.ok = None;
        r.tx_index = None;
        let e = event_from_row(&r).expect("converts");
        assert!(e.envelope().outcome_unknown());
        assert!(!e.envelope().succeeded());
        assert!(!e.envelope().failed());
    }

    #[test]
    fn a_backfilled_trade_has_no_trader_rather_than_a_placeholder_one() {
        // The query does not fetch account keys. Writing the system program
        // there made every distinct-trader count over backfilled data equal to
        // one, which is a measurement-shaped answer to a question nothing
        // asked.
        let e = event_from_row(&row(&trade_data(), &[PUMP])).expect("converts");
        let Event::Trade(t) = e else {
            panic!("a trade row is a trade")
        };
        assert_eq!(t.trader, None);
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
