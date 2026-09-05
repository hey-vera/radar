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

use radar_analyst::{Answered, Answering, Gate, Limits, Mention};
use radar_onchain::RpcClient;
use radar_roast::BaseRates;

use crate::dossier::safe;
use crate::flag;

/// Reads one line, or skips it.
///
/// Hand-parsed from a `Value` rather than derived, because `radar-cli` does not
/// depend on `serde` and a fixture reader is not a reason to give it one — this
/// binary's dependency tree is deliberately small.
///
/// A line missing a field is **skipped**, not defaulted. A mention with no
/// author would otherwise be attributed to `""` and share one summoner's
/// allowance with every other malformed line.
///
/// The type is [`radar_analyst::Mention`], the same one the X client produces.
/// A fixture type of its own would be free to drift away from what the account
/// actually receives, which would make reading two hundred dry-run replies a
/// test of the fixture rather than of the bot.
///
/// `parent` is read here too, under the platform's own field name, so a fixture
/// can exercise the reply-chain path without an account.
fn parse_mention(line: &str) -> Option<Mention> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    Some(Mention {
        id: value.get("id")?.as_str()?.to_owned(),
        author: value.get("author")?.as_str()?.to_owned(),
        text: value.get("text")?.as_str()?.to_owned(),
        parent: value
            .get("parent")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
    })
}

/// The mentions in a JSONL file's text.
///
/// Blank lines are skipped, and a line that does not parse is dropped rather
/// than failing the run: one malformed line in a feed is not a reason to answer
/// nobody.
///
/// Its own function so the blank-line filter can be tested. Inline, deleting
/// the `!` kept only the blank lines and parsed nothing, and no test could see
/// that -- CI reported it as a survivor.
fn mentions_in(text: &str) -> Vec<Mention> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(parse_mention)
        .collect()
}

/// Whether a reply needs a newline before the shell prompt returns.
fn needs_newline(text: &str) -> bool {
    !text.ends_with('\n')
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
    let mentions = mentions_in(&text);

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

    let client = flag(args, "--rpc").map_or_else(
        || RpcClient::from_vars(&|k| std::env::var(k).ok()),
        RpcClient::new,
    );
    let rates = BaseRates::load(radar_roast::baserates::DEFAULT_PATH).ok();
    if rates.is_none() {
        eprintln!("no base rates; replies will carry no population context");
    }
    let provider = radar_model::from_vars(&|k| std::env::var(k).ok()).ok();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());

    let creators = radar_roast::CreatorIndex::read(radar_roast::creator::DEFAULT_PATH).ok();
    if creators.is_none() {
        eprintln!("no creator index; replies will say nothing about who launched the token");
    }

    // ADR 0013 constraint 5, read the way the daemon reads it. A dry run that
    // ignored it would have the hundred replies Josh reads before launch differ
    // from the ones the daemon posts, on the one rule that is about the token.
    let self_mint = radar_analyst::daemon::self_mint_from(&|k| std::env::var(k).ok())?;

    let ctx = Answering {
        client: &client,
        rates: rates.as_ref(),
        creators: creators.as_ref(),
        provider: provider.as_deref(),
        self_mint: self_mint.as_ref(),
        now,
    };
    for mention in &mentions {
        answer(mention, &mut gate, &ctx, &log_path)?;
    }

    println!(
        "\n{} mention(s), {} reply(ies) sent, log at {log_path}",
        mentions.len(),
        gate.sent_today()
    );
    println!("(dry run: this command holds no credential and posted nothing)");
    Ok(())
}

/// Answers one mention and prints what happened.
///
/// The pipeline itself is [`radar_analyst::answer`], shared with the daemon:
/// two copies of *what the account says* is the arrangement where the version
/// somebody reviews two hundred replies from is not the version that posts
/// them. This function is the difference between the two callers, and it is
/// entirely presentation.
///
/// # Errors
///
/// Only when the reply log cannot be written. That stops the whole run rather
/// than skipping the mention, and deliberately: an account that cannot record
/// what it says must not carry on saying things.
fn answer(
    mention: &Mention,
    gate: &mut Gate,
    ctx: &Answering<'_>,
    log_path: &str,
) -> Result<(), String> {
    println!(
        "
=== {} from @{}",
        mention.id,
        safe(&mention.author, 32)
    );
    // The mention's own text is echoed escaped, and is used for nothing else:
    // everything after this point sees only what the parser kept, which is a
    // mint or a symbol.
    println!("    {}", safe(&mention.text, 120));

    let entry = match radar_analyst::answer(mention, gate, ctx) {
        Answered::Reply(entry) => *entry,
        Answered::Ticker(reply) => {
            println!("--> {reply}");
            return Ok(());
        }
        Answered::Nothing => {
            println!("--> (no mint or ticker found; nothing to answer)");
            return Ok(());
        }
        Answered::Refused(why) => {
            println!("--> refused: {}", radar_analyst::describe(&why));
            return Ok(());
        }
        Answered::NotAnAddress => {
            println!("--> (not a valid address)");
            return Ok(());
        }
        Answered::Unreadable(e) => {
            println!("--> (could not read the chain: {e})");
            return Ok(());
        }
    };

    let mint_text = entry.mint.clone().unwrap_or_default();
    let was_template = entry.fellback.is_some();

    // The log is written before anything is said, and a failure to write stops
    // the reply.
    let written = radar_analyst::publish::publish(&radar_analyst::DryRun, log_path, entry)
        .map_err(|e| format!("could not write the reply log at {log_path}: {e}"))?;

    print!("--> {}", written.reply);
    if needs_newline(&written.reply) {
        println!();
    }
    if was_template {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_lines_are_skipped_and_real_ones_are_kept() {
        // The filter is `not empty`. Deleting the `!` keeps only the blank
        // lines, so a full mentions file parses to nothing and the run answers
        // nobody -- while exiting zero, which looks exactly like a quiet day.
        // That is LEARNINGS 5, and it lived inline where no test could see it.
        let file = concat!(
            "\n",
            "   \n",
            r#"{"id":"1","author":"alice","text":"@radar what about $ABC"}"#,
            "\n",
            "\t\n",
            r#"{"id":"2","author":"bob","text":"@radar and $DEF"}"#,
            "\n",
        );
        let found = mentions_in(file);
        assert_eq!(found.len(), 2, "two real lines among four blank ones");

        // Blank input is not an error, and it is not two mentions either.
        assert!(mentions_in("").is_empty());
        assert!(mentions_in("\n  \n\t\n").is_empty());

        // A line that does not parse is dropped, not fatal.
        let with_junk = format!("not json\n{}", r#"{"id":"3","author":"c","text":"$GHI"}"#);
        assert_eq!(mentions_in(&with_junk).len(), 1);
    }

    #[test]
    fn a_reply_gets_a_newline_only_when_it_lacks_one() {
        assert!(needs_newline("no trailing newline"));
        assert!(!needs_newline("has one\n"));
        assert!(needs_newline(""));
    }
}
