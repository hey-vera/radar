// SPDX-License-Identifier: Apache-2.0
//! The loop the daemon runs, and the configuration it reads.
//!
//! # Why this is a module and not the binary
//!
//! The binary is four lines. Everything it does is here, so that one poll --
//! read, answer, log, publish, advance the cursor, save the ledger -- can be
//! driven by a test against a fake platform instead of only by systemd against
//! the real one.
//!
//! That is not a testing convenience. The orderings this loop enforces are the
//! ones that cost money or credibility when they are wrong: a mention refused
//! after the chain was read has already been paid for, a reply posted before it
//! was logged is a public statement with no record, and a cursor advanced past a
//! mention that was never answered is a question silently dropped.

use std::time::Duration;

use radar_provider::Budget;
use radar_roast::BaseRates;
use radar_types::MicroUsd;

use crate::admission::{Gate, Limits};
use crate::answer::{Answered, Answering};
use crate::poll;
use crate::publish::{DryRun, Publisher};
use crate::spend::{Cost, Prices, Spend};
use crate::x::X;

/// Where the loop keeps its files.
pub struct Paths {
    /// The reply log.
    pub log: String,
    /// The last answered mention.
    pub cursor: String,
    /// The spend ledger.
    pub ledger: String,
}

impl Paths {
    /// Under one directory, so an operator moves one thing.
    #[must_use]
    pub fn under(dir: &str) -> Self {
        Self {
            log: format!("{dir}/replies.jsonl"),
            cursor: format!("{dir}/cursor"),
            ledger: format!("{dir}/ledger.json"),
        }
    }
}

/// Seconds since the epoch.
#[must_use]
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The accounting day a timestamp falls in.
///
/// Whole days since the epoch, UTC. The meter's window and the gate's are the
/// same day for the same reason a bill is: an operator reading "spent today"
/// and "replies today" should not have to ask which today.
#[must_use]
pub const fn day_of(secs: u64) -> u64 {
    secs / 86_400
}

fn env(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

/// The budget, or a closed one.
///
/// Dollars in, micro-USD out, because an operator writing a daily cap thinks in
/// dollars and the meter counts in millionths. A value that will not parse is
/// **closed**, not ignored: a typo in a spending ceiling must not read as
/// permission.
fn budget_from_env() -> Budget {
    let daily = env("RADAR_ANALYST_DAILY_USD")
        .and_then(|v| v.trim().parse::<f64>().ok())
        .map(MicroUsd::from_dollars);
    let per_call = env("RADAR_ANALYST_PER_CALL_USD")
        .and_then(|v| v.trim().parse::<f64>().ok())
        .map(MicroUsd::from_dollars);
    match (daily, per_call) {
        (Some(daily_max), Some(per_call_max)) => Budget {
            per_call_max,
            daily_max,
        },
        _ => Budget::CLOSED,
    }
}

/// The admission limits, or ones that refuse everything.
///
/// Unset means zero, and zero means refuse. `Limits` has no `Default` in the
/// library on purpose — a default here would be a spending policy invented by
/// whoever typed it — so this function is where the absence is turned into a
/// refusal rather than into a number.
fn limits_from_env() -> Limits {
    let n = |key: &str| env(key).and_then(|v| v.trim().parse().ok()).unwrap_or(0);
    Limits {
        per_summoner_daily: n("RADAR_ANALYST_PER_SUMMONER_DAILY"),
        global_daily: n("RADAR_ANALYST_GLOBAL_DAILY"),
        dedupe_seconds: env("RADAR_ANALYST_DEDUPE_SECONDS")
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(3_600),
    }
}

/// Runs the loop. Never returns.
pub fn run() -> ! {
    let dir = env("RADAR_ANALYST_DIR").unwrap_or_else(|| "data/analyst".to_owned());
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("radar-analyst: cannot use {dir}: {e}");
        std::process::exit(1);
    }
    let paths = Paths::under(&dir);

    // The source and the publisher are the same credential. Absent, the loop
    // reads nothing and posts nothing -- which is the resting state, and it is
    // reported rather than assumed.
    let x = X::from_env();
    let publisher: Box<dyn Publisher> = match &x {
        Some(client) => Box::new(client.clone()),
        None => Box::new(DryRun),
    };

    let Some(prices) = Prices::from_vars(&env) else {
        // Not an exit. A price list is a spending decision and its absence is a
        // configuration state, not a crash -- but nothing may be answered
        // without it, because an unpriced call cannot be metered and an
        // unmetered call is the open invoice this account cannot afford.
        eprintln!(
            "radar-analyst: no prices configured, so nothing can be metered and \
             nothing will be answered. Set RADAR_X_PRICE_MENTION_READ, \
             RADAR_X_PRICE_POST_READ, RADAR_X_PRICE_REPLY and \
             RADAR_MODEL_PER_CALL_USD_MICRO -- see deploy/analyst.env.example."
        );
        idle_forever();
    };

    let budget = budget_from_env();
    if budget == Budget::CLOSED {
        eprintln!(
            "radar-analyst: unfunded -- RADAR_ANALYST_DAILY_USD and \
             RADAR_ANALYST_PER_CALL_USD are not both set, so every call is refused."
        );
    }

    let limits = limits_from_env();
    let mut gate = Gate::new(limits, vec!["radar".to_owned()]);
    let mut spend = Spend::open(budget, prices, paths.ledger.clone(), day_of(now()));

    let client = radar_onchain::RpcClient::from_vars(&env);
    let rates = BaseRates::load(radar_roast::baserates::DEFAULT_PATH).ok();
    if rates.is_none() {
        eprintln!("radar-analyst: no base rates; replies will carry no population context");
    }
    let provider = radar_model::from_vars(&env).ok();

    eprintln!(
        "radar-analyst: publisher={} source={} dir={dir}",
        publisher.name(),
        if x.is_some() { "x" } else { "none" }
    );

    let mut wait = poll::BUSY;
    loop {
        let found = tick(
            x.as_ref(),
            publisher.as_ref(),
            &mut gate,
            &mut spend,
            &client,
            rates.as_ref(),
            provider.as_deref(),
            &paths,
        );
        wait = poll::interval(found, wait);
        std::thread::sleep(wait);
    }
}

/// Sleeps rather than exiting, so a misconfigured unit is visible as a running
/// service that says what is missing rather than as a restart loop.
fn idle_forever() -> ! {
    loop {
        std::thread::sleep(Duration::from_secs(3_600));
    }
}

/// One poll, and everything it found. Returns how many mentions were answered.
///
/// Public so a test can drive exactly one against a fake platform. The loop in
/// [`run`] is this function and a sleep.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub fn tick(
    x: Option<&X>,
    publisher: &dyn Publisher,
    gate: &mut Gate,
    spend: &mut Spend,
    client: &radar_onchain::RpcClient,
    rates: Option<&BaseRates>,
    provider: Option<&dyn radar_model::Provider>,
    paths: &Paths,
) -> usize {
    let Some(x) = x else {
        return 0;
    };
    let at = now();
    let today = day_of(at);

    // The read is billable before it happens.
    let Ok(read) = spend.authorize(Cost::MentionRead, today) else {
        eprintln!("radar-analyst: budget spent; not polling");
        return 0;
    };

    let cursor = poll::read_cursor(&paths.cursor);
    let mentions = match x.mentions(cursor.as_deref()) {
        Ok(m) => {
            // Settled at what was reserved. The platform does not report a
            // per-call charge on the response, so the list price is the best
            // available actual -- and settling at anything less would quietly
            // hand the budget back. Settling at zero, which an earlier draft of
            // this did, makes the meter decorative.
            let charged = read.reserved();
            spend.settle(read, charged);
            m
        }
        Err(e) => {
            // Nothing was delivered, so nothing is charged.
            spend.release(read);
            eprintln!("radar-analyst: mentions poll failed: {e}");
            return 0;
        }
    };

    let ctx = Answering {
        client,
        rates,
        provider,
        now: at,
    };

    let mut answered = 0;
    for mention in &mentions {
        match crate::answer::answer(mention, gate, &ctx) {
            Answered::Reply(entry) => {
                let mint = entry.mint.clone().unwrap_or_default();
                let Ok(reply_cost) = spend.authorize(Cost::Reply, today) else {
                    eprintln!("radar-analyst: budget spent; {} not answered", mention.id);
                    continue;
                };
                match crate::publish::publish(publisher, &paths.log, *entry) {
                    Ok(written) => {
                        if let Some(id) = &written.reply_id {
                            let charged = reply_cost.reserved();
                            spend.settle(reply_cost, charged);
                            gate.record(&mention.author, &mint, id, at);
                            answered += 1;
                        } else {
                            // Nothing was published, so nothing is charged and
                            // the gate is not told: a broken publisher must not
                            // silence the account by spending an allowance it
                            // never used.
                            spend.release(reply_cost);
                        }
                    }
                    Err(e) => {
                        spend.release(reply_cost);
                        // An account that cannot record what it says must not
                        // carry on saying things.
                        eprintln!("radar-analyst: cannot write {}: {e}", paths.log);
                        break;
                    }
                }
            }
            other => {
                // Refusals, symbols and unreadable chains cost nothing and are
                // not published. They are still worth a line: an account that
                // answers nothing should say what it is seeing.
                eprintln!("radar-analyst: {} -> {other:?}", mention.id);
            }
        }
    }

    if let Some(next) = poll::next_cursor(mentions.iter().map(|m| m.id.as_str()), cursor.as_deref())
        && let Err(e) = poll::write_cursor(&paths.cursor, &next)
    {
        // Not fatal, and loud: the next start re-reads from the old cursor,
        // which the gate's dedupe absorbs.
        eprintln!("radar-analyst: cannot save the cursor: {e}");
    }
    if let Err(e) = spend.save() {
        eprintln!("radar-analyst: cannot save the ledger: {e}");
    }
    answered
}
