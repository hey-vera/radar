// SPDX-License-Identifier: Apache-2.0
//! `radar analyst` — the whole summoned-reply loop, over mentions from a file.
//!
//! The caller `radar-analyst` was built for, and the way the loop is exercised
//! without an X account. Mentions come from a JSONL file rather than a poller,
//! which makes the interesting half — parsing, admission, the reply, the log —
//! runnable, repeatable and diffable.
//!
//! **It cannot post.** The only [`Publisher`](radar_analyst::Publisher) wired
//! here is the dry run, which holds no credential and returns no reply id, so
//! every entry it writes records what Radar *would* have said. Swapping in a
//! live publisher is a configuration change against a trait, and the X client
//! behind it is not written — see `radar_analyst::publish` for the two billing
//! questions that gate it.
//!
//! # The fixture is the point
//!
//! A file of mentions is what lets somebody read two hundred replies beside
//! their fact sheets before anything is public, and disagree with some. That
//! was going to be the last thing anyone did before launch; doing it first is
//! cheaper.

use std::time::Duration;

use radar_analyst::{Admitted, Asked, Entry, Gate, Limits, Refused};
use radar_onchain::{Budget, RpcClient};
use radar_roast::BaseRates;
use radar_types::Address;

use crate::dossier::safe;
use crate::flag;

/// One line of the mentions file.
struct Mention {
    id: String,
    author: String,
    text: String,
}

/// Reads one line, or skips it.
///
/// Hand-parsed from a `Value` rather than derived, because `radar-cli` does not
/// depend on `serde` and a fixture reader is not a reason to give it one — this
/// binary's dependency tree is deliberately small.
///
/// A line missing a field is **skipped**, not defaulted. A mention with no
/// author would otherwise be attributed to `""` and share one summoner's
/// allowance with every other malformed line.
fn parse_mention(line: &str) -> Option<Mention> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    Some(Mention {
        id: value.get("id")?.as_str()?.to_owned(),
        author: value.get("author")?.as_str()?.to_owned(),
        text: value.get("text")?.as_str()?.to_owned(),
    })
}

/// Runs the command.
///
/// # Errors
///
/// A message when the mentions file is missing or unreadable.
pub fn run(args: &[String]) -> Result<(), String> {
    let path = flag(args, "--mentions").ok_or_else(|| {
        "usage: radar analyst --mentions <file.jsonl> [--log <file>] [--rpc URL] \
         [--per-summoner N] [--global N]\n\
         \n\
         Each line: {\"id\":\"...\",\"author\":\"...\",\"text\":\"...\"}\n\
         Dry run only -- this command holds no credential and cannot post."
            .to_owned()
    })?;

    let text = std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))?;
    let mentions: Vec<Mention> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(parse_mention)
        .collect();

    let log_path = flag(args, "--log").unwrap_or_else(|| "analyst-log.jsonl".to_owned());

    // Limits are stated rather than defaulted. `Limits` has no `Default` on
    // purpose: a default here would be a spending policy invented by whoever
    // typed it, and this is the file where it would go unnoticed.
    let limits = Limits {
        per_summoner_daily: flag(args, "--per-summoner")
            .and_then(|v| v.parse().ok())
            .unwrap_or(3),
        global_daily: flag(args, "--global")
            .and_then(|v| v.parse().ok())
            .unwrap_or(50),
        dedupe_seconds: 3600,
    };
    let mut gate = Gate::new(limits, vec!["radar".to_owned()]);

    let client = flag(args, "--rpc").map_or_else(RpcClient::from_env, RpcClient::new);
    let rates = BaseRates::load(radar_roast::baserates::DEFAULT_PATH).ok();
    if rates.is_none() {
        eprintln!("no base rates; replies will carry no population context");
    }
    let provider = radar_model::from_vars(&|k| std::env::var(k).ok()).ok();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());

    let ctx = Answering {
        client: &client,
        rates: rates.as_ref(),
        provider: provider.as_deref(),
        log_path: &log_path,
        now,
    };
    for mention in &mentions {
        answer(mention, &mut gate, &ctx)?;
    }

    println!(
        "\n{} mention(s), {} reply(ies) sent, log at {log_path}",
        mentions.len(),
        gate.sent_today()
    );
    println!("(dry run: this command holds no credential and posted nothing)");
    Ok(())
}

/// Everything one answer needs that does not change between mentions.
struct Answering<'a> {
    client: &'a RpcClient,
    rates: Option<&'a BaseRates>,
    provider: Option<&'a dyn radar_model::Provider>,
    log_path: &'a str,
    now: u64,
}

/// Answers one mention.
///
/// # Errors
///
/// Only when the reply log cannot be written. That stops the whole run rather
/// than skipping the mention, and deliberately: an account that cannot record
/// what it says must not carry on saying things.
fn answer(mention: &Mention, gate: &mut Gate, ctx: &Answering<'_>) -> Result<(), String> {
    println!("\n=== {} from @{}", mention.id, safe(&mention.author, 32));
    // The mention's own text is echoed escaped, and is used for nothing else:
    // everything after this point sees only what the parser kept, which is a
    // mint or a symbol.
    println!("    {}", safe(&mention.text, 120));

    let mint_text = match radar_analyst::read(&mention.text) {
        Asked::Mint(m) => m,
        Asked::Ticker(t) => {
            // The honest answer, and the best content available: a symbol
            // identifies nothing, and guessing which token was meant is how
            // measurements get published about the wrong project.
            println!("--> {}", radar_analyst::ticker_reply(&t));
            return Ok(());
        }
        Asked::Nothing => {
            println!("--> (no mint or ticker found; nothing to answer)");
            return Ok(());
        }
    };

    if let Admitted::No(why) = gate.admit(&mention.author, &mint_text, ctx.now) {
        println!("--> refused: {}", describe(&why));
        return Ok(());
    }

    let Ok(mint) = mint_text.parse::<Address>() else {
        println!("--> (not a valid address)");
        return Ok(());
    };

    let mut budget = Budget::new(
        radar_onchain::budget::DEFAULT_MAX_CALLS,
        radar_onchain::budget::DEFAULT_MAX_PAGES,
        Duration::from_secs(20),
    );
    let dossier = match radar_onchain::build(ctx.client, &mut budget, &mint) {
        Ok(d) => d,
        Err(e) => {
            println!("--> (could not read the chain: {e})");
            return Ok(());
        }
    };

    let (sheet, reply) = radar_roast::roast(&dossier, ctx.rates, ctx.provider);

    let entry = Entry {
        at: ctx.now,
        mention_id: mention.id.clone(),
        summoner: mention.author.clone(),
        mint: Some(mint_text.clone()),
        read_at_slot: dossier.read_at.map(|s| s.0),
        // The evidence, not only the words. A log of replies without fact
        // sheets records what Radar said and not whether it was entitled to say
        // it, and the second is the half that settles an argument.
        fact_sheet: sheet.render(),
        reply: reply.text.clone(),
        fellback: reply.fellback.as_ref().map(|f| format!("{f:?}")),
        reply_id: None,
    };

    // The log is written before anything is treated as sent, and a failure to
    // write stops the reply.
    let written = radar_analyst::publish::publish(&radar_analyst::DryRun, ctx.log_path, entry)
        .map_err(|e| format!("could not write the reply log at {}: {e}", ctx.log_path))?;

    print!("--> {}", written.reply);
    if !written.reply.ends_with('\n') {
        println!();
    }
    if reply.is_template() {
        println!("    (deterministic template)");
    }
    // Recorded only once it has actually been through the publisher. A mention
    // admitted and then not sent has cost nothing on X, and charging it would
    // let a broken publisher silence the account by spending an allowance it
    // never used.
    if let Some(id) = &written.reply_id {
        gate.record(&mention.author, &mint_text, id, ctx.now);
    }
    Ok(())
}

/// A refusal, in words worth telling somebody.
fn describe(why: &Refused) -> String {
    match why {
        Refused::Unconfigured => "no limits configured, so nothing is answered".to_owned(),
        Refused::SummonerDaily { cap } => format!("this account has had its {cap} replies today"),
        Refused::GlobalDaily { cap } => format!("the daily cap of {cap} replies is spent"),
        Refused::AlreadyAnswered { reply_id } => {
            format!("already answered for this mint, see {reply_id}")
        }
        Refused::SelfOrIgnored => "Radar does not answer itself".to_owned(),
    }
}
