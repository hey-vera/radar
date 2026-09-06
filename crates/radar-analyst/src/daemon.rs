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
use radar_roast::{BaseRates, Billed};
use radar_types::{Address, MicroUsd};

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
    /// The Telegram lane's own log. **Never read for the contest**: the
    /// leaderboard, the week-close job and the hunter tally read `log`, and a
    /// Telegram answer stays out of the record by being in a different file
    /// rather than by carrying a flag (design 0009 L5).
    pub telegram_log: String,
    /// The Telegram lane's `getUpdates` offset.
    pub telegram_cursor: String,
    /// Who the X gate refused, and when. The week-close job reads it: an
    /// account refused during the week does not win it (design 0007 §6.2).
    pub refusals: String,
    /// The account's own posts -- the weekly result, the daily "seven days
    /// later" -- recorded before they are said, like replies.
    pub posts: String,
    /// The contest's week records and the pool reading, which the public
    /// endpoints serve.
    pub contest_dir: String,
    /// The daily "seven days later" rows, written by `radar seven-days-later`
    /// on a timer and posted from here.
    pub daily_dir: String,
}

impl Paths {
    /// Under one directory, so an operator moves one thing.
    ///
    /// The contest directory is a sibling rather than a child: `radar-serve`
    /// reads it as `RADAR_CONTEST_DIR`, defaulting to `data/contest`, and the
    /// two defaults have to name the same place.
    #[must_use]
    pub fn under(dir: &str) -> Self {
        let contest_dir = std::path::Path::new(dir).parent().map_or_else(
            || "data/contest".to_owned(),
            |p| format!("{}/contest", p.display()),
        );
        Self {
            log: format!("{dir}/replies.jsonl"),
            cursor: format!("{dir}/cursor"),
            ledger: format!("{dir}/ledger.json"),
            telegram_log: format!("{dir}/telegram.jsonl"),
            telegram_cursor: format!("{dir}/telegram.cursor"),
            refusals: format!("{dir}/refusals.jsonl"),
            posts: format!("{dir}/posts.jsonl"),
            contest_dir,
            daily_dir: format!("{dir}/daily"),
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
/// Takes a getter rather than reading the environment, so the rule can be tested
/// without setting process-wide variables that parallel tests would fight over —
/// the same shape `Prices::from_vars` uses, and for the same reason.
///
/// Dollars in, micro-USD out, because an operator writing a daily cap thinks in
/// dollars and the meter counts in millionths. A value that will not parse is
/// **closed**, not ignored: a typo in a spending ceiling must not read as
/// permission.
pub fn budget_from(get: &impl Fn(&str) -> Option<String>) -> Budget {
    let daily = get("RADAR_ANALYST_DAILY_USD")
        .and_then(|v| v.trim().parse::<f64>().ok())
        .map(MicroUsd::from_dollars);
    let per_call = get("RADAR_ANALYST_PER_CALL_USD")
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

/// What to say when the budget refuses everything, or `None`.
///
/// A function rather than an `if` inside [`run`], because `run` never returns
/// and nothing inside it can be tested. The decision — *is this instance
/// funded* — is worth pinning: an operator who mistypes a ceiling gets a
/// service that answers nothing, and this line is the whole difference between
/// that and a mystery.
#[must_use]
pub fn unfunded_notice(budget: Budget) -> Option<&'static str> {
    (budget == Budget::CLOSED).then_some(
        "radar-analyst: unfunded -- RADAR_ANALYST_DAILY_USD and \
         RADAR_ANALYST_PER_CALL_USD are not both set, so every call is refused.",
    )
}

/// The admission limits, or ones that refuse everything.
///
/// Takes a getter, for the reason [`budget_from`] does.
///
/// Unset means zero, and zero means refuse. `Limits` has no `Default` in the
/// library on purpose — a default here would be a spending policy invented by
/// whoever typed it — so this function is where the absence is turned into a
/// refusal rather than into a number.
pub fn limits_from(get: &impl Fn(&str) -> Option<String>) -> Limits {
    let n = |key: &str| get(key).and_then(|v| v.trim().parse().ok()).unwrap_or(0);
    Limits {
        per_summoner_daily: n("RADAR_ANALYST_PER_SUMMONER_DAILY"),
        global_daily: n("RADAR_ANALYST_GLOBAL_DAILY"),
        // The one with a default, because a dedupe window is not a spending
        // decision -- it decides how long "already answered" lasts, and zero
        // would mean the same coin is answered again on the next poll. An hour
        // is the same figure the command uses.
        dedupe_seconds: get("RADAR_ANALYST_DEDUPE_SECONDS")
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(3_600),
    }
}

/// The analyst's own token, or `None` when no token is special.
///
/// ADR 0013 constraint 5. Takes a getter, for the reason [`budget_from`] does.
///
/// Three answers, and they are deliberately not two:
///
/// - **Unset or blank is `Ok(None)`**: no mint is special, and every coin is
///   answered on the same rule. This does not bend rule 8 -- there is no spend
///   and no permission in it, only a rule with nothing to apply to. The token
///   does not exist until ADR 0013's launch gate is met, and until then the
///   correct configuration is no configuration.
/// - **A value that parses is `Ok(Some(mint))`.**
/// - **A value that does not parse is `Err`, and the caller must not run.** A
///   misspelt mint would silently switch the rule off for the real token, which
///   is the one direction ADR 0013 exists to prevent: the analyst stating its
///   own price. Same shape as a price list that will not parse -- the instance
///   says what is wrong and answers nothing. The value is not echoed, because
///   the likeliest wrong value is some other variable's secret pasted on the
///   wrong line.
///
/// # Errors
///
/// When the variable is set to something that is not a base58 address.
pub fn self_mint_from(get: &impl Fn(&str) -> Option<String>) -> Result<Option<Address>, String> {
    let Some(raw) = get("RADAR_SELF_MINT") else {
        return Ok(None);
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    raw.parse::<Address>().map(Some).map_err(|_| {
        "RADAR_SELF_MINT is set and is not an address, so the analyst's own token cannot be \
         told apart and nothing is answered. Set it to the mint, or unset it if no token \
         exists yet."
            .to_owned()
    })
}

/// What the daemon says about which token, if any, is its own.
///
/// Said on every start, beside the publishing posture, so an operator reading
/// the journal after the token exists can see in one line whether the rule is
/// armed for the right mint.
#[must_use]
pub fn self_mint_notice(self_mint: Option<&Address>) -> String {
    match self_mint {
        None => "radar-analyst: no RADAR_SELF_MINT, so no token is the analyst's own and every \
                 coin is answered on the same rule."
            .to_owned(),
        Some(mint) => format!(
            "radar-analyst: RADAR_SELF_MINT={mint} -- its price and market capitalisation are \
             never stated; everything else about it is answered like any other coin."
        ),
    }
}

/// Whether this instance may actually say anything in public.
///
/// # Why this is not the credential
///
/// It was. The token was both the reader and the publisher, so pasting it into
/// `/etc/radar/analyst.env` turned a silent instance into a public account in
/// one step — and there was no way to read live mentions while answering
/// nobody.
///
/// That is the wrong shape for two reasons. The launch gate in design 0007 asks
/// for a hundred replies to be **read beside their fact sheets** before anybody
/// outside sees one, and with one switch that gate could only be satisfied by
/// publishing the hundred. And on 2026-09-04 two wrong figures were found in the
/// reply path in a single day — a cost 6.7× too high, and a charge signed as a
/// gain — both by looking at real output. The first hundred replies are exactly
/// where the next one gets found, and they should not be found in public.
///
/// So speaking is its own decision. Rule 8: the most consequential action here
/// requires somebody to type a word, not merely to have pasted a token.
///
/// Anything other than `on` is off, including `true`, `1` and `yes`. A ceiling
/// spelled wrongly must not read as permission, and neither must this — the
/// value is checked against exactly one word so that a typo fails closed and
/// the log says which state it is in.
#[must_use]
pub fn may_publish(get: &impl Fn(&str) -> Option<String>) -> bool {
    get("RADAR_X_PUBLISH").is_some_and(|v| v.trim().eq_ignore_ascii_case("on"))
}

/// What the daemon says about which state it is in.
///
/// Four, and each is a real situation somebody will be in:
///
/// 1. no bearer, so nothing is read and nothing is posted;
/// 2. reading, deliberately silent — the state the launch gate is read in;
/// 3. **switched on with no signing credential**, which is a misconfiguration;
/// 4. live.
///
/// The third exists because the platform needs two different credentials: a
/// bearer to read mentions, and four OAuth 1.0a values to post. Somebody who
/// sets the bearer, switches publishing on, and stops there has an instance that
/// answers every mention and delivers none of them — and states 2 and 3 look
/// identical from everywhere except the log, so the daemon names them apart on
/// every start.
#[must_use]
pub fn posture(has_credential: bool, can_post: bool, publishing: bool) -> &'static str {
    match (has_credential, can_post, publishing) {
        (false, _, _) => "radar-analyst: no credential, so nothing is read and nothing is posted.",
        (true, _, false) => {
            "radar-analyst: reading mentions and answering them to the log ONLY -- \
             set RADAR_X_PUBLISH=on to speak in public."
        }
        (true, false, true) => {
            "radar-analyst: RADAR_X_PUBLISH=on but there is no signing credential, so every \
             reply will be answered and none delivered. A bearer can read; posting needs \
             RADAR_X_API_KEY, RADAR_X_API_SECRET, RADAR_X_ACCESS_TOKEN and RADAR_X_ACCESS_SECRET."
        }
        (true, true, true) => "radar-analyst: LIVE -- replies are being posted publicly.",
    }
}

/// Which publisher the loop speaks through.
///
/// # Why this is a function
///
/// It was three lines inside [`run`], and `run` never returns, so nothing could
/// call it. Mutation testing said so precisely: deleting the arm that selects the
/// live client left every test passing, and the resulting daemon is one that
/// holds a valid credential, is switched on, and silently posts nothing.
///
/// That is the single most consequential line in this crate in the direction
/// nobody notices. A daemon that wrongly *posts* is caught within a minute by
/// anybody looking at the account; a daemon that wrongly *stays silent* looks
/// exactly like a quiet week, which the `analyst` check in `radar brief` is
/// deliberately built not to alarm about.
///
/// So the choice is out here where a test can make it, and `Publisher::name`
/// is what the test reads.
#[must_use]
pub fn publisher_for(x: Option<X>, publishing: bool) -> Box<dyn Publisher> {
    match (x, publishing) {
        (Some(client), true) => Box::new(client),
        _ => Box::new(DryRun),
    }
}

/// Who the gate never answers.
///
/// **The bot's own numeric id**, which is what a mention's `author_id` carries.
/// This was the literal string `radar` until 2026-09-06 -- a value no X account
/// id can equal, so the one entry the ignore list exists for was never in it,
/// and the account would have answered its own mention had anything produced
/// one. Research 0029, S21. The contest's own operator list has read
/// `x.user_id()` since #167; the gate did not.
///
/// With no credential there is no id and nothing to poll, so the list is empty
/// rather than carrying a placeholder that matches nobody.
#[must_use]
pub fn ignored(x: Option<&X>) -> Vec<String> {
    x.map(|x| vec![x.user_id().to_owned()]).unwrap_or_default()
}

/// Runs the loop. Never returns.
#[allow(
    clippy::too_many_lines,
    reason = "the daemon's start-up, read once top to bottom"
)]
pub fn run() -> ! {
    let dir = env("RADAR_ANALYST_DIR").unwrap_or_else(|| "data/analyst".to_owned());
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("radar-analyst: cannot use {dir}: {e}");
        std::process::exit(1);
    }
    let paths = Paths::under(&dir);

    // The credential is the **source**. Speaking is a separate decision, and
    // `may_publish` says why. Absent the credential the loop reads nothing and
    // posts nothing; present but not switched on, it reads and answers into the
    // log, which is the state the launch gate is read in.
    let x = X::from_env();
    let publishing = may_publish(&env);
    let publisher = publisher_for(x.clone(), publishing);
    eprintln!(
        "{}",
        posture(x.is_some(), x.as_ref().is_some_and(X::can_post), publishing)
    );

    // The free lane, on its own token, its own switch, its own caps and its
    // own log (design 0009 L5). Same rule 8 shape as X: no token, nothing read.
    let telegram = crate::telegram::Telegram::from_env();
    let telegram_publishing = crate::telegram::may_publish(&env);
    let telegram_publisher = crate::telegram::publisher_for(telegram.clone(), telegram_publishing);
    eprintln!(
        "{}",
        crate::telegram::posture(telegram.is_some(), telegram_publishing)
    );
    let mut telegram_gate = Gate::new(crate::telegram::limits_from(&env), Vec::new());

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

    let budget = budget_from(&env);
    if let Some(notice) = unfunded_notice(budget) {
        eprintln!("{notice}");
    }

    let limits = limits_from(&env);
    let mut gate = Gate::new(limits, ignored(x.as_ref()));
    let mut spend = Spend::open(budget, prices, paths.ledger.clone(), day_of(now()));

    let client = radar_onchain::RpcClient::from_vars(&env);
    let rates = BaseRates::load(radar_roast::baserates::DEFAULT_PATH).ok();
    if rates.is_none() {
        eprintln!("radar-analyst: no base rates; replies will carry no population context");
    }
    // The fact that makes one reply differ from another. Absent, every reply
    // about a fresh launch says the same thing, so its absence is reported
    // rather than left to be noticed in the output.
    let creators = radar_roast::CreatorIndex::read(radar_roast::creator::DEFAULT_PATH).ok();
    if creators.is_none() {
        eprintln!(
            "radar-analyst: no creator index; replies will say nothing about who launched              the token. Build one with `radar creator-index`."
        );
    }
    let provider = radar_model::from_vars(&env).ok();

    // ADR 0013 constraint 5. A value that will not parse idles the instance
    // rather than running with the rule off: `self_mint_from` says why.
    let self_mint = match self_mint_from(&env) {
        Ok(mint) => mint,
        Err(e) => {
            eprintln!("radar-analyst: {e}");
            idle_forever();
        }
    };
    eprintln!("{}", self_mint_notice(self_mint.as_ref()));

    eprintln!(
        "radar-analyst: publisher={} source={} telegram={} dir={dir}",
        publisher.name(),
        if x.is_some() { "x" } else { "none" },
        if telegram.is_some() {
            telegram_publisher.name()
        } else {
            "off"
        }
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
            creators.as_ref(),
            provider.as_deref(),
            self_mint.as_ref(),
            &paths,
        );
        let found_telegram = crate::telegram::tick(
            telegram.as_ref(),
            telegram_publisher.as_ref(),
            &mut telegram_gate,
            &mut spend,
            &client,
            rates.as_ref(),
            creators.as_ref(),
            provider.as_deref(),
            self_mint.as_ref(),
            &paths,
        );
        // The week closes on the tick after Monday 00:00 UTC, once. The
        // record is written first; the posts are written from the record.
        // Every account the operator controls, not just the bot's own.
        //
        // The bot posts as itself and is managed from a person's own account.
        // Only the bot's id was excluded before 2026-09-06, so the managing
        // account could have entered its own contest and won -- the operator
        // paying themselves out of a pool the public is told is theirs.
        //
        // `RADAR_CONTEST_OPERATORS` is a comma-separated list of numeric ids;
        // the bot's own id is always in the set whether or not it is listed, so
        // forgetting the variable cannot make the bot eligible.
        let rules = radar_contest::Rules::published(operator_ids(x.as_ref()));
        match crate::contest::close_if_due(
            x.as_ref(),
            &paths,
            now(),
            &rules,
            limits.per_summoner_daily,
        ) {
            Ok(Some(record)) => announce_week(
                &record,
                publisher.as_ref(),
                telegram_publisher.as_ref(),
                &mut spend,
                &client,
                rates.as_ref(),
                creators.as_ref(),
                provider.as_deref(),
                self_mint.as_ref(),
                &paths,
            ),
            Ok(None) => {}
            Err(e) => eprintln!("radar-analyst: cannot write the week's record: {e}"),
        }
        // Every tick, not only at close: see the function's note.
        prompt_claim_if_due(publisher.as_ref(), &mut spend, &paths);
        // The daily post, from the rows the timer job wrote, once past the
        // hour. Priced as one top-level post when it goes out on X.
        announce_day(
            publisher.as_ref(),
            telegram_publisher.as_ref(),
            &mut spend,
            &paths,
        );
        wait = next_wait(found, found_telegram, wait);
        std::thread::sleep(wait);
    }
}

/// How long to sleep after a tick that found `found` X mentions and
/// `found_telegram` Telegram messages.
///
/// Either lane finding something keeps the loop busy: the two counts are
/// added and handed to [`poll::interval`]. A function rather than a line
/// inside [`run`] because `run` never returns, and CI's mutants replaced the
/// `+` with `*` and `-` with nothing failing -- a loop that went idle while
/// one lane was busy, or one that panicked on underflow, and no test could
/// see either.
#[must_use]
pub fn next_wait(found: usize, found_telegram: usize, previous: Duration) -> Duration {
    poll::interval(found + found_telegram, previous)
}

/// Settles a post's reservation when something was sent and releases it when
/// nothing was.
///
/// A reservation for a post that never left -- a dry run, a refused text, a
/// publisher that failed -- must go back, or the day's budget is spent on
/// posts nobody received; one that did leave is charged at what was reserved,
/// because the platform reports no per-call price. A function because CI's
/// mutants turned this `>` into `==` inside two functions nothing could call
/// from a test, and a meter that settles the empty case and releases the
/// real one is a meter that runs out on quiet weeks and never on busy ones.
fn settle_if_sent(spend: &mut Spend, reservation: radar_provider::Commitment, sent: usize) {
    if sent > 0 {
        let charged = reservation.reserved();
        spend.settle(reservation, charged);
    } else {
        spend.release(reservation);
    }
}

/// Posts today's "seven days later" if it is due, metering the X post.
fn announce_day(
    publisher: &dyn Publisher,
    telegram: &dyn Publisher,
    spend: &mut Spend,
    paths: &Paths,
) {
    let at = now();
    if crate::daily::due(at, &paths.daily_dir).is_none() {
        return;
    }
    let vault = std::fs::read_to_string(format!("{}/pool.json", paths.contest_dir))
        .ok()
        .and_then(|text| radar_contest::Vault::from_json(&text).ok());
    let Ok(reservation) = spend.authorize(Cost::Post, day_of(at)) else {
        eprintln!("radar-analyst: budget spent; today's post is not published");
        return;
    };
    match crate::daily::post_if_due(
        at,
        &paths.daily_dir,
        vault.as_ref(),
        publisher,
        &paths.posts,
        telegram,
        &paths.telegram_log,
    ) {
        Ok(sent) => settle_if_sent(spend, reservation, sent),
        Err(e) => {
            spend.release(reservation);
            eprintln!("radar-analyst: cannot post the day: {e}");
        }
    }
}

/// Posts a closed week: the summary, then the winner's coin torn down as a
/// reply to it, on X and -- when a channel is configured -- on Telegram.
///
/// The teardown reads the chain once for the winning mint, the way a summoned
/// reply would, and is written by the same roaster under the same checks. A
/// week with no winner posts the summary alone.
#[allow(clippy::too_many_arguments)]
fn announce_week(
    record: &radar_contest::Record,
    publisher: &dyn Publisher,
    telegram: &dyn Publisher,
    spend: &mut Spend,
    client: &radar_onchain::RpcClient,
    rates: Option<&BaseRates>,
    creators: Option<&radar_roast::CreatorIndex>,
    provider: Option<&dyn radar_model::Provider>,
    self_mint: Option<&radar_types::Address>,
    paths: &Paths,
) {
    let at = now();
    let vault = std::fs::read_to_string(format!("{}/pool.json", paths.contest_dir))
        .ok()
        .and_then(|text| radar_contest::Vault::from_json(&text).ok());
    let mut posts = vec![crate::weekly::summary(record, vault.as_ref())];

    if let Some(winner) = record.ranking.winner() {
        match winner.entry.mint.parse::<radar_types::Address>() {
            Ok(mint) => {
                let mut budget = radar_onchain::budget::Budget::default();
                match radar_onchain::build(client, &mut budget, &mint) {
                    Ok(dossier) => {
                        let (sheet, reply) =
                            radar_roast::roast(&dossier, rates, creators, provider, self_mint);
                        posts.push(crate::weekly::teardown(&sheet, &reply));
                    }
                    Err(e) => {
                        eprintln!("radar-analyst: no teardown, the chain could not be read: {e}");
                    }
                }
            }
            Err(_) => eprintln!("radar-analyst: no teardown, the winning mint is not an address"),
        }
    }

    // One top-level post and up to one reply, priced as such. Refused by the
    // meter means recorded and not said, like everything else here.
    let today = day_of(at);
    let Ok(reservation) = spend.authorize(Cost::Post, today) else {
        eprintln!("radar-analyst: budget spent; the week's post is not published");
        return;
    };
    match crate::weekly::publish(
        publisher,
        &paths.posts,
        &format!("weekly:{}", record.week.0),
        &posts,
        at,
    ) {
        Ok(sent) => settle_if_sent(spend, reservation, sent),
        Err(e) => {
            spend.release(reservation);
            eprintln!("radar-analyst: cannot write {}: {e}", paths.posts);
            return;
        }
    }
    // Free, and recorded in the same file under the same id so a reader sees
    // both lanes side by side.
    if let Err(e) = crate::weekly::publish(
        telegram,
        &paths.telegram_log,
        &format!("weekly:{}", record.week.0),
        &posts,
        at,
    ) {
        eprintln!("radar-analyst: cannot write {}: {e}", paths.telegram_log);
    }
}

/// Posts the claim prompt for any week whose winner has not been told yet.
///
/// Runs on every tick, not only at close. `try_claim` requires a claim to be a
/// reply to this post, so a week with no prompt on its record accepts no claim
/// at all -- and a prompt that failed to post once would otherwise cost the
/// winner the whole seven days. Retrying is bounded by the claim window.
///
/// The prompt goes under the account's own winning reply, so it arrives in the
/// thread the winner is already in. Its id is written back into the record;
/// until that write happens no claim is possible, which is the safe direction.
///
/// In a dry run the post is recorded and not published, no id comes back,
/// `claim_prompt` stays `None`, and no claim can land -- correct, because no
/// winning reply was published for anyone to have seen either.
fn prompt_claim_if_due(publisher: &dyn Publisher, spend: &mut Spend, paths: &Paths) {
    let at = now();
    let Some(record) = crate::contest::prompt_due(&paths.contest_dir, at) else {
        return;
    };
    let Some(winner) = record.winner.as_ref() else {
        return;
    };
    let Some(post) = crate::weekly::claim_prompt(&record) else {
        return;
    };

    let Ok(reservation) = spend.authorize(Cost::Reply, day_of(at)) else {
        eprintln!(
            "radar-analyst: budget spent; week {} claim prompt not posted, retrying next tick",
            record.week.0
        );
        return;
    };
    match crate::weekly::publish_under(
        publisher,
        &paths.posts,
        &format!("claim:{}", record.week.0),
        Some(&winner.reply_id),
        std::slice::from_ref(&post),
        at,
    ) {
        Ok((sent, first)) => {
            settle_if_sent(spend, reservation, sent);
            if let Some(id) = first {
                let mut updated = record;
                updated.claim_prompt = Some(id);
                if let Err(e) = crate::contest::write_record(&paths.contest_dir, &updated) {
                    // The post went out and the record does not know it. The
                    // next tick posts a second prompt, which is noisy and
                    // recoverable; a claim replying to either one is refused
                    // until a write succeeds, which is the safe failure.
                    eprintln!(
                        "radar-analyst: week {} claim prompt posted but not recorded: {e}",
                        updated.week.0
                    );
                }
            }
        }
        Err(e) => {
            spend.release(reservation);
            eprintln!("radar-analyst: cannot write {}: {e}", paths.posts);
        }
    }
}

/// Every account the operator controls, for the contest's exclusion rule.
///
/// The bot's own id is always included, so an unset or mistyped
/// `RADAR_CONTEST_OPERATORS` can never make the bot itself eligible -- the
/// failure this ordering exists to prevent. Everything else is additive.
///
/// Ids only: anything that is not a run of digits is dropped, because an X
/// account id is a number and a handle pasted here would silently never match
/// the `summoner` field, which carries an id.
fn operator_ids(x: Option<&X>) -> Vec<String> {
    let own = x.map_or_else(|| "radar".to_owned(), |x| x.user_id().to_owned());
    operator_ids_from(
        &own,
        std::env::var("RADAR_CONTEST_OPERATORS").ok().as_deref(),
    )
}

/// The same, with the listed value supplied rather than read.
///
/// Split out so the rule can be tested without setting a process-wide variable
/// — the pattern `Paths::from_vars` already uses, for the same reason.
#[must_use]
fn operator_ids_from(own: &str, listed: Option<&str>) -> Vec<String> {
    let mut ids = vec![own.to_owned()];
    let Some(listed) = listed else {
        return ids;
    };
    ids.extend(
        listed
            .split(',')
            .map(|id| {
                id.trim()
                    .chars()
                    .filter(char::is_ascii_digit)
                    .collect::<String>()
            })
            // An empty id is what a stray comma, a blank entry or a pasted
            // handle collapses to, and an empty string in the set would make
            // `is_operator("")` true. No summoner is empty today, so this is
            // belt and braces — but the belt costs one `!`, and the failure it
            // prevents is an entrant silently excluded from a prize.
            .filter(|id| !id.is_empty()),
    );
    ids
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
    creators: Option<&radar_roast::CreatorIndex>,
    provider: Option<&dyn radar_model::Provider>,
    self_mint: Option<&Address>,
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

    let mut answered = 0;
    for mention in &mentions {
        // A winner naming an address inside the claim window is claiming, not
        // summoning. Checked first, and at the cost of a directory listing
        // only: the claim is written into the record and the mention is not
        // answered, because a wallet is not a coin.
        if let Some(week) = crate::contest::try_claim(mention, &paths.contest_dir, at) {
            eprintln!(
                "radar-analyst: {} -> claim recorded for week {}",
                mention.id, week.0
            );
            continue;
        }

        // The model call is the one thing a stranger can make this account
        // spend that nothing charged for until now. Reserved **before**
        // `answer`, because `answer` makes the call internally: by the time a
        // reply comes back the money is gone, and a ceiling checked after that
        // is not a ceiling.
        //
        // A refusal here does not refuse the mention. The day's model budget
        // being spent means this one reply is the deterministic template --
        // which is exactly what the account ships with no provider configured
        // at all, so it is rule 8 rather than an outage.
        let reserved = provider.and_then(|_| {
            spend
                .authorize(Cost::ModelCall, today)
                .inspect_err(|_| {
                    eprintln!(
                        "radar-analyst: model budget spent; {} answered by the template",
                        mention.id
                    );
                })
                .ok()
        });
        let ctx = Answering {
            client,
            rates,
            creators,
            // Gated on the reservation, so a refused meter means no call was
            // made rather than one that was made unmetered.
            provider: if reserved.is_some() { provider } else { None },
            self_mint,
            now: at,
        };

        let outcome = crate::answer::answer(mention, gate, &ctx);
        // Settled here rather than inside the arms below. The money is already
        // spent by this point, and the reply's own reservation is a separate
        // ceiling that can be refused -- one must not hold the other open.
        if let Some(commitment) = reserved {
            match outcome.billed() {
                Billed::NoCall => spend.release(commitment),
                Billed::Reported(actual) => spend.settle(commitment, actual),
                // Rule 9. What was reserved is the honest charge for a cost
                // nobody reported. Zero is not.
                Billed::Unreported => {
                    let charged = commitment.reserved();
                    spend.settle(commitment, charged);
                }
            }
        }

        match outcome {
            Answered::Reply { entry, .. } => {
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
                // A gate refusal is also a fact the contest needs: an account
                // refused during the week does not win it. Appended, never
                // fatal -- a refusal that could not be written is a line on
                // the terminal, and the reply loop carries on.
                if let Answered::Refused(why) = &other
                    && let Err(e) = crate::contest::append_refusal(
                        &paths.refusals,
                        &crate::contest::RefusalLine {
                            at,
                            summoner: mention.author.clone(),
                            why: crate::answer::describe(why),
                            kind: Some(crate::contest::RefusalKind::of(why)),
                        },
                    )
                {
                    eprintln!("radar-analyst: cannot write {}: {e}", paths.refusals);
                }
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

#[cfg(test)]
mod tests {
    #[test]
    fn the_operator_set_always_holds_the_bots_own_id_and_drops_empty_ones() {
        use super::operator_ids_from;

        // Unset: the bot is still excluded. This ordering is the point -- a
        // missing variable must never make the bot eligible for its own prize.
        assert_eq!(operator_ids_from("111", None), vec!["111".to_owned()]);

        // Listed ids are additive.
        assert_eq!(
            operator_ids_from("111", Some("222,333")),
            vec!["111".to_owned(), "222".to_owned(), "333".to_owned()]
        );

        // Whitespace and a handle pasted where an id belongs. `summoner` is a
        // numeric id, so a handle would silently never match; it collapses to
        // empty and is dropped rather than sitting in the set as "".
        //
        // Re-apply by deleting the `!` in the filter and the last assertion
        // fails: "" enters the set and `is_operator("")` becomes true.
        assert_eq!(
            operator_ids_from("111", Some(" 222 , , @thecabalhunter , 333 ")),
            vec!["111".to_owned(), "222".to_owned(), "333".to_owned()]
        );
        assert!(
            !operator_ids_from("111", Some(",,")).contains(&String::new()),
            "an empty id must never enter the set"
        );
    }

    use super::*;

    #[test]
    fn a_post_that_left_is_charged_and_one_that_did_not_is_given_back() {
        // Re-applied as CI did: `>` to `==` charges the dry run and refunds the
        // real post, and both assertions below fail.
        let dir = std::env::temp_dir().join(format!("radar-daemon-settle-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temp dir");
        let ledger = dir.join("ledger.json").to_string_lossy().into_owned();
        let _ = std::fs::remove_file(&ledger);
        let prices = Prices {
            mention_read: MicroUsd(1_000),
            post_read: MicroUsd(5_000),
            reply: MicroUsd(10_000),
            post: MicroUsd(15_000),
            model_call: MicroUsd(2_000),
        };
        let mut spend = Spend::open(
            Budget {
                per_call_max: MicroUsd(50_000),
                daily_max: MicroUsd(1_000_000),
            },
            prices,
            ledger,
            1,
        );
        let reservation = spend.authorize(Cost::Post, 1).expect("authorised");
        settle_if_sent(&mut spend, reservation, 0);
        assert_eq!(
            spend.spent_today(),
            MicroUsd::ZERO,
            "nothing left, nothing charged"
        );

        let reservation = spend.authorize(Cost::Post, 1).expect("authorised");
        settle_if_sent(&mut spend, reservation, 2);
        assert_eq!(
            spend.spent_today(),
            MicroUsd(15_000),
            "a thread that left is one post's price"
        );
    }

    #[test]
    fn either_lane_finding_something_keeps_the_loop_busy() {
        // Re-applied as CI did: `+` to `*` makes (0, 1) idle and the second
        // assertion fails; `+` to `-` panics on (0, 1) and the test fails there.
        assert_eq!(next_wait(1, 0, poll::IDLE), poll::BUSY);
        assert_eq!(next_wait(0, 1, poll::IDLE), poll::BUSY);
        assert_eq!(next_wait(2, 3, poll::IDLE), poll::BUSY);
        // Nothing found on either lane: the wait doubles from where it was.
        assert_eq!(next_wait(0, 0, poll::BUSY), poll::BUSY * 2);
    }

    /// A getter over a fixed table, so the rules can be tested without touching
    /// process-wide environment variables.
    fn from(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let owned: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |key: &str| owned.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    }

    #[test]
    fn a_day_is_whole_days_since_the_epoch() {
        // The meter's window and the gate's are the same day, so this arithmetic
        // decides when both reset. Getting it wrong by an operator means a
        // budget that resets hourly or never.
        assert_eq!(day_of(0), 0);
        assert_eq!(day_of(86_399), 0, "one second before the boundary");
        assert_eq!(day_of(86_400), 1, "the boundary itself");
        assert_eq!(day_of(86_401), 1);
        assert_eq!(day_of(1_788_000_000), 20_694);
    }

    #[test]
    fn a_budget_needs_both_halves_or_it_is_closed() {
        // Deny by default: a per-call ceiling with no daily cap is not a
        // budget, it is a ceiling on how fast an unbounded bill accumulates.
        let both = from(&[
            ("RADAR_ANALYST_DAILY_USD", "5.00"),
            ("RADAR_ANALYST_PER_CALL_USD", "0.25"),
        ]);
        let budget = budget_from(&both);
        assert_eq!(budget.daily_max, MicroUsd(5_000_000));
        assert_eq!(budget.per_call_max, MicroUsd(250_000));

        for partial in [
            vec![("RADAR_ANALYST_DAILY_USD", "5.00")],
            vec![("RADAR_ANALYST_PER_CALL_USD", "0.25")],
            vec![],
        ] {
            assert_eq!(
                budget_from(&from(&partial)),
                Budget::CLOSED,
                "{partial:?} must not be a budget"
            );
        }
    }

    #[test]
    fn a_ceiling_that_will_not_parse_is_closed_rather_than_ignored() {
        // A typo in a spending ceiling must not read as permission.
        let typo = from(&[
            ("RADAR_ANALYST_DAILY_USD", "five dollars"),
            ("RADAR_ANALYST_PER_CALL_USD", "0.25"),
        ]);
        assert_eq!(budget_from(&typo), Budget::CLOSED);
    }

    #[test]
    fn an_unfunded_instance_is_told_why_it_is_answering_nothing() {
        // An operator who mistypes a ceiling gets a service that answers
        // nothing, and this line is the difference between that and a mystery.
        let notice = unfunded_notice(Budget::CLOSED).expect("a closed budget says so");
        assert!(notice.contains("unfunded"), "{notice}");
        assert!(notice.contains("RADAR_ANALYST_DAILY_USD"), "{notice}");

        // A funded one says nothing: a warning that fires when everything is
        // fine is a warning nobody reads.
        assert_eq!(
            unfunded_notice(Budget {
                per_call_max: MicroUsd(250_000),
                daily_max: MicroUsd(5_000_000),
            }),
            None
        );
    }

    #[test]
    fn absent_limits_refuse_everything() {
        // Zero is the refusing value in `Gate`, so unset means nobody is
        // answered. A default here would be a policy invented by whoever typed
        // it.
        let limits = limits_from(&from(&[]));
        assert_eq!(limits.per_summoner_daily, 0);
        assert_eq!(limits.global_daily, 0);
        // Except the dedupe window, which is not a spending decision: zero
        // would answer the same coin again on the very next poll.
        assert_eq!(limits.dedupe_seconds, 3_600);
    }

    #[test]
    fn limits_are_read_when_they_are_set() {
        let set = from(&[
            ("RADAR_ANALYST_PER_SUMMONER_DAILY", "3"),
            ("RADAR_ANALYST_GLOBAL_DAILY", "50"),
            ("RADAR_ANALYST_DEDUPE_SECONDS", "900"),
        ]);
        let limits = limits_from(&set);
        assert_eq!(limits.per_summoner_daily, 3);
        assert_eq!(limits.global_daily, 50);
        assert_eq!(limits.dedupe_seconds, 900);
    }

    #[test]
    fn the_self_mint_is_none_when_unset_or_blank_and_read_when_it_is_an_address() {
        // Unset and blank both mean no token is special. The token does not
        // exist until the launch gate is met, so for now the correct
        // configuration is none, and it must not be reported as an error.
        assert!(matches!(self_mint_from(&from(&[])), Ok(None)));
        assert!(matches!(
            self_mint_from(&from(&[("RADAR_SELF_MINT", "   ")])),
            Ok(None)
        ));

        // A real address, with the whitespace an env file leaves around it.
        let mint = Address::new([3u8; 32]);
        let padded = format!("  {mint}  ");
        let read = self_mint_from(&from(&[("RADAR_SELF_MINT", padded.as_str())]));
        assert!(
            matches!(read, Ok(Some(m)) if m == mint),
            "the mint must round-trip"
        );
    }

    #[test]
    fn a_self_mint_that_is_not_an_address_is_an_error_and_not_none() {
        // The direction that matters. `None` means the rule is off, so a typo
        // that read as `None` would have the analyst state its own price while
        // every log line said the rule was configured. Re-apply the bug by
        // mapping the parse failure to `Ok(None)` and this fails.
        match self_mint_from(&from(&[("RADAR_SELF_MINT", "not-a-mint")])) {
            Err(e) => {
                assert!(e.contains("RADAR_SELF_MINT"), "{e}");
                assert!(e.contains("nothing is answered"), "{e}");
                // Not echoed: the likeliest wrong value is another variable's
                // secret on the wrong line.
                assert!(!e.contains("not-a-mint"), "{e}");
            }
            Ok(m) => panic!("an unparseable mint must not be accepted as {m:?}"),
        }
    }

    #[test]
    fn the_self_mint_notice_names_the_mint_or_says_there_is_none() {
        // One line in the journal that says whether the rule is armed, and for
        // which token. Each state says something only it could say.
        let none = self_mint_notice(None);
        assert!(none.contains("no RADAR_SELF_MINT"), "{none}");
        assert!(none.contains("same rule"), "{none}");

        let mint = Address::new([3u8; 32]);
        let some = self_mint_notice(Some(&mint));
        assert!(some.contains(&mint.to_string()), "{some}");
        assert!(some.contains("never stated"), "{some}");
        assert!(!some.contains("no RADAR_SELF_MINT"), "{some}");
    }

    #[test]
    fn the_paths_are_all_under_one_directory() {
        // An operator moves one thing, and the unit grants write access to one
        // path. A file that escaped this directory would be a file the service
        // is not permitted to write.
        let paths = Paths::under("/var/lib/radar/analyst");
        for path in [&paths.log, &paths.cursor, &paths.ledger] {
            assert!(
                path.starts_with("/var/lib/radar/analyst/"),
                "{path} escapes the directory"
            );
        }
        // And they are three different files, not one name used three times.
        assert_ne!(paths.log, paths.cursor);
        assert_ne!(paths.cursor, paths.ledger);
        assert_ne!(paths.log, paths.ledger);
    }

    /// A getter over a fixed list, so nothing touches the process environment.
    fn vars<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k: &str| {
            pairs
                .iter()
                .find(|(name, _)| *name == k)
                .map(|(_, v)| (*v).to_owned())
        }
    }

    #[test]
    fn speaking_in_public_needs_its_own_word() {
        // The switch this exists for. A credential makes the account *readable*;
        // it must not make it *audible*, because the launch gate asks for a
        // hundred replies to be read beside their fact sheets first and there
        // was no way to do that without publishing them.
        assert!(may_publish(&vars(&[("RADAR_X_PUBLISH", "on")])));
        assert!(may_publish(&vars(&[("RADAR_X_PUBLISH", "ON")])));
        assert!(may_publish(&vars(&[("RADAR_X_PUBLISH", "  on  ")])));
    }

    #[test]
    fn anything_that_is_not_that_word_is_silence() {
        // Including every value an operator might reasonably expect to work.
        // A ceiling spelled wrongly must not read as permission, and this is a
        // ceiling on speech.
        //
        // `true`, `1` and `yes` are here deliberately: each one is somebody
        // confidently enabling the account and getting silence, which is the
        // safe direction and is reported by `posture` rather than left a
        // mystery.
        for value in ["", " ", "off", "true", "1", "yes", "no", "onn", "n"] {
            assert!(
                !may_publish(&vars(&[("RADAR_X_PUBLISH", value)])),
                "{value:?} must not enable publishing"
            );
        }
        assert!(!may_publish(&vars(&[])), "absent is silence");
    }

    /// A client that is never called — only [`Publisher::name`] is read.
    fn a_client() -> X {
        X::at("https://example.test", "bearer", "u42")
    }

    #[test]
    fn only_a_credential_that_is_switched_on_speaks() {
        // CI found this by deleting the arm that selects the live client and
        // watching every test pass. The daemon that leaves behind holds a valid
        // credential, is switched on, and silently posts nothing.
        //
        // It is the failure direction nobody notices: a daemon that wrongly
        // posts is caught within a minute by anybody looking at the account, and
        // one that wrongly stays silent looks exactly like a quiet week — which
        // `radar brief`'s analyst check is deliberately built not to alarm on.
        assert_eq!(publisher_for(Some(a_client()), true).name(), "x");

        // Every other combination is silence, and each is a real state: no
        // credential yet; a credential being read beside its fact sheets before
        // anybody outside sees a reply; and the switch on with nothing to speak
        // through, which must not be mistaken for the first case.
        assert_eq!(publisher_for(Some(a_client()), false).name(), "dry-run");
        assert_eq!(publisher_for(None, true).name(), "dry-run");
        assert_eq!(publisher_for(None, false).name(), "dry-run");
    }

    #[test]
    fn the_three_states_are_told_apart_in_words() {
        // Reading-but-silent and live look identical from everywhere except the
        // reply log, so the daemon says which it is on every start.
        assert!(posture(false, false, false).contains("no credential"));
        assert!(posture(true, true, false).contains("log ONLY"));
        assert!(posture(true, true, false).contains("RADAR_X_PUBLISH=on"));
        assert!(posture(true, true, true).contains("LIVE"));

        // A bearer is required to speak, so "publishing without one" is not a
        // state that can exist -- and if it ever did, it must not be reported
        // as live.
        assert!(posture(false, true, true).contains("no credential"));

        // The misconfiguration the second credential introduced: switched on,
        // able to read, unable to sign. Answers everything, delivers nothing.
        // It must not read as either of its neighbours.
        let unsigned = posture(true, false, true);
        assert!(unsigned.contains("no signing credential"), "{unsigned}");
        assert!(unsigned.contains("RADAR_X_API_KEY"), "{unsigned}");
        assert!(!unsigned.contains("LIVE"), "{unsigned}");
        assert!(!unsigned.contains("log ONLY"), "{unsigned}");
    }

    #[test]
    fn the_gate_ignores_the_bot_by_its_own_id_and_not_by_a_name() {
        // Research 0029, S21. The list held the literal `radar` until
        // 2026-09-06. A mention carries `author_id`, which is a run of digits,
        // so no account could ever match it -- the one entry the ignore list
        // exists for was the one entry it did not contain.
        //
        // Re-apply by returning `vec!["radar".to_owned()]` and the first
        // assertion fails.
        let x = X::at("http://127.0.0.1:1", "test-token", "1739482910");
        assert_eq!(ignored(Some(&x)), vec!["1739482910".to_owned()]);

        // No credential is nothing to poll, so there is nobody to ignore. A
        // placeholder here would be a list that matches nobody, which is what
        // the bug was.
        assert!(ignored(None).is_empty());
    }
}
