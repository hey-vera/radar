// SPDX-License-Identifier: Apache-2.0
//! Telegram: the free lane, and not the record.
//!
//! Design 0009 §3 L5 and §5 M5. X is the public record and the contest; a reply
//! there is a dated public statement anyone can check. Telegram is where the
//! same question costs nothing to answer, so somebody who wants to check twenty
//! coins a day does it here instead of at the X gate. **Same parser, same gate
//! shape, same fact path, same two checks on the text.** Only the transport
//! differs, so a Telegram message is untrusted input on exactly the rule an X
//! mention is (rule 4), and an instruction in one travels nowhere for the same
//! reason: only the address survives [`crate::mention::read`].
//!
//! # What keeps it out of the contest
//!
//! Not a flag. Telegram answers are written to **their own log file**, and the
//! public leaderboard, the week-close job and the hunter tally read the X log
//! and nothing else. A Telegram answer cannot become a contest entry because no
//! code path reads the file it is in for that purpose -- which is the cheapest
//! level of the enforcement ladder, and why there is no `is_telegram` field to
//! forget to check.
//!
//! # Rule 8, twice
//!
//! No `RADAR_TELEGRAM_BOT_TOKEN` means nothing is read and nothing is sent, and
//! the daemon says so. A token alone reads messages and answers them **into the
//! log only**; `RADAR_TELEGRAM_PUBLISH=on` is what makes it speak, for the
//! reason the X switch exists: the first replies are meant to be read beside
//! their fact sheets before a stranger sees one. The caps are separate from X's
//! and unset means zero, which means refuse.
//!
//! # A different bot from the alert one
//!
//! The deploy guide's alert channel is a bot that talks *to the operator*. This
//! one talks to strangers. A token shared between them would let a stranger's
//! message land in the alert channel's chat, so they are two bots by design and
//! the env example says so.
//!
//! # Ids
//!
//! A Telegram message id is only unique within its chat, so the mention id this
//! module records is `chat_id:message_id` -- that pair *is* the message's
//! identity on the platform, and it is also exactly what a reply needs. The
//! summoner is `tg:<user id>`: numeric like an X author id, prefixed so the two
//! can never be confused in a log or a cap.

use crate::admission::{Gate, Limits};
use crate::answer::{Answered, Answering};
use crate::publish::{DryRun, Publisher, Undeliverable};
use crate::x::{Mention, Unreachable};

/// Where the Bot API lives.
pub const API: &str = "https://api.telegram.org";

/// A Telegram bot: reads messages addressed to it and can reply.
#[derive(Clone)]
pub struct Telegram {
    token: String,
    base: String,
    /// The chat the weekly and daily posts go to, or `None`.
    ///
    /// `RADAR_TELEGRAM_CHANNEL`. Replies need no channel -- they go where the
    /// question was -- so a bot with a token and no channel answers people and
    /// posts nothing on its own, and [`Publisher::post`] says so.
    channel: Option<String>,
}

impl core::fmt::Debug for Telegram {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // The token is the credential and it sits in every URL this module
        // builds. A `Debug` that printed it would put it in the journal the
        // first time anything logged the client.
        f.debug_struct("Telegram")
            .field("token", &"<redacted>")
            .field("base", &self.base)
            .field("channel", &self.channel)
            .finish()
    }
}

/// One page of updates: the mentions it held, and where to ask from next.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Page {
    /// Messages with text, as mentions.
    pub mentions: Vec<Mention>,
    /// The `offset` for the next call: one past the largest update id seen.
    ///
    /// `None` when the page was empty, so the caller keeps the cursor it had.
    /// Telegram re-sends every update until it is acknowledged by an offset
    /// past it, so a cursor that failed to advance is a page answered twice --
    /// which the gate's dedupe absorbs, and the log then shows.
    pub next_offset: Option<String>,
}

impl Telegram {
    /// From the environment, or `None` when there is no token.
    ///
    /// Blank is unset. `RADAR_TELEGRAM_API_BASE` exists so the loop can be
    /// driven against a fake end to end; it defaults to the real thing because
    /// a dropped variable must point at Telegram rather than at nothing.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        Self::from_vars(&|k| std::env::var(k).ok())
    }

    /// [`Self::from_env`] with a getter, so the rule can be tested.
    #[must_use]
    pub fn from_vars(get: &impl Fn(&str) -> Option<String>) -> Option<Self> {
        let token = get("RADAR_TELEGRAM_BOT_TOKEN")?;
        if token.trim().is_empty() {
            return None;
        }
        Some(Self {
            token: token.trim().to_owned(),
            base: get("RADAR_TELEGRAM_API_BASE").unwrap_or_else(|| API.to_owned()),
            channel: get("RADAR_TELEGRAM_CHANNEL")
                .map(|c| c.trim().to_owned())
                .filter(|c| !c.is_empty()),
        })
    }

    /// A bot pointed at a different root, for tests. No channel.
    #[must_use]
    pub fn at(base: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            base: base.into(),
            channel: None,
        }
    }

    /// The same bot, posting its own posts into `channel`.
    #[must_use]
    pub fn with_channel(mut self, channel: impl Into<String>) -> Self {
        self.channel = Some(channel.into());
        self
    }

    /// The `getUpdates` URL for everything after `offset`.
    ///
    /// Pure, so the query can be checked without a call. Only messages are
    /// asked for: edits, channel posts and the rest are more untrusted text to
    /// carry, and none of it reaches a decision. `timeout=0` because the loop
    /// already sleeps between polls and a long poll held open here would hold
    /// the X poll with it.
    #[must_use]
    pub fn updates_url(&self, offset: Option<&str>) -> String {
        let mut url = format!(
            "{}/bot{}/getUpdates?timeout=0&allowed_updates=%5B%22message%22%5D",
            self.base, self.token
        );
        if let Some(offset) = offset {
            // Digits only, for the reason `X::mentions_url` gives: the cursor
            // came from the network before it came from our file.
            let clean: String = offset.chars().filter(char::is_ascii_digit).collect();
            if !clean.is_empty() {
                url.push_str("&offset=");
                url.push_str(&clean);
            }
        }
        url
    }

    /// Reads the updates after `offset`.
    ///
    /// # Errors
    ///
    /// [`Unreachable`] when the platform refuses, cannot be reached, or answers
    /// with something this module cannot read.
    pub fn updates(&self, offset: Option<&str>) -> Result<Page, Unreachable> {
        // The same two calls `X` makes, unsigned: the token is in the path, so
        // there is no header to add and nothing to redact from one.
        let response = ureq::get(&self.updates_url(offset)).call();
        parse_updates(&body_of(response)?)
    }

    fn send(&self, body: &str) -> Result<String, Unreachable> {
        let response = ureq::post(&format!("{}/bot{}/sendMessage", self.base, self.token))
            .header("Content-Type", "application/json")
            .send(body);
        body_of(response)
    }
}

/// Turns a ureq result into a body or a typed failure.
fn body_of(
    response: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
) -> Result<String, Unreachable> {
    match response {
        Ok(mut ok) => ok
            .body_mut()
            .read_to_string()
            .map_err(|e| Unreachable::Transport(e.to_string())),
        Err(ureq::Error::StatusCode(status)) => Err(Unreachable::Refused {
            status,
            body: String::new(),
        }),
        Err(other) => Err(Unreachable::Transport(other.to_string())),
    }
}

/// Reads a `getUpdates` response.
///
/// Forgiving in one direction only, like [`crate::x::parse_mentions`]: an update
/// that is not a text message from an identifiable sender in an identifiable
/// chat is **skipped**, never defaulted -- it still advances the offset, so a
/// sticker is acknowledged and not re-read forever. `ok: false` is reported,
/// not read as an empty page: "nobody wrote" and "the token was revoked" must
/// not be the same outcome.
///
/// # Errors
///
/// [`Unreachable::Unreadable`] when the body is not JSON, is not `ok`, or has no
/// result array.
pub fn parse_updates(body: &str) -> Result<Page, Unreachable> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| Unreachable::Unreadable(format!("not json: {e}")))?;
    if value.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
        let why = value
            .get("description")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("no description");
        return Err(Unreachable::Unreadable(format!("not ok: {why}")));
    }
    let Some(items) = value.get("result").and_then(serde_json::Value::as_array) else {
        return Err(Unreachable::Unreadable("result is not an array".to_owned()));
    };

    let mut page = Page::default();
    let mut newest: Option<u64> = None;
    for item in items {
        if let Some(id) = item.get("update_id").and_then(serde_json::Value::as_u64) {
            newest = Some(newest.map_or(id, |n| n.max(id)));
        }
        let Some(message) = item.get("message") else {
            continue;
        };
        let (Some(message_id), Some(chat), Some(from), Some(text)) = (
            message
                .get("message_id")
                .and_then(serde_json::Value::as_i64),
            message
                .get("chat")
                .and_then(|c| c.get("id"))
                .and_then(serde_json::Value::as_i64),
            message
                .get("from")
                .and_then(|f| f.get("id"))
                .and_then(serde_json::Value::as_i64),
            message.get("text").and_then(serde_json::Value::as_str),
        ) else {
            continue;
        };
        let parent = message
            .get("reply_to_message")
            .and_then(|r| r.get("message_id"))
            .and_then(serde_json::Value::as_i64)
            .map(|id| format!("{chat}:{id}"));
        page.mentions.push(Mention {
            id: format!("{chat}:{message_id}"),
            author: format!("tg:{from}"),
            text: text.to_owned(),
            parent,
        });
    }
    page.next_offset = newest.map(|n| (n + 1).to_string());
    Ok(page)
}

/// The `sendMessage` body for a reply to `chat_id:message_id`.
///
/// `None` when the id is not that shape: a reply with no chat to go to is not
/// sent to a guessed one.
#[must_use]
pub fn send_body(in_reply_to: &str, text: &str) -> Option<String> {
    let (chat, message) = in_reply_to.split_once(':')?;
    let chat: i64 = chat.parse().ok()?;
    let message: i64 = message.parse().ok()?;
    Some(
        serde_json::json!({
            "chat_id": chat,
            "text": text,
            "reply_parameters": { "message_id": message },
            "disable_web_page_preview": true,
        })
        .to_string(),
    )
}

/// Reads the id of a sent message out of a `sendMessage` response.
///
/// Treated as a failure to publish when it cannot be read, for the reason
/// [`crate::x::parse_posted_id`] gives: the safe mistake is a reply that exists
/// and is recorded as not existing.
///
/// # Errors
///
/// [`Unreachable::Unreadable`] when the body is not `ok` or carries no id.
pub fn parse_sent_id(body: &str) -> Result<String, Unreachable> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| Unreachable::Unreadable(format!("not json: {e}")))?;
    if value.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
        return Err(Unreachable::Unreadable("not ok".to_owned()));
    }
    let result = value
        .get("result")
        .ok_or_else(|| Unreachable::Unreadable("no result".to_owned()))?;
    let (Some(chat), Some(id)) = (
        result
            .get("chat")
            .and_then(|c| c.get("id"))
            .and_then(serde_json::Value::as_i64),
        result.get("message_id").and_then(serde_json::Value::as_i64),
    ) else {
        return Err(Unreachable::Unreadable("no message id".to_owned()));
    };
    Ok(format!("{chat}:{id}"))
}

impl Publisher for Telegram {
    fn name(&self) -> &'static str {
        "telegram"
    }

    fn reply(&self, in_reply_to: &str, text: &str) -> Result<String, Undeliverable> {
        let body = send_body(in_reply_to, text).ok_or_else(|| {
            Undeliverable::Failed(format!("not a chat:message id: {in_reply_to}"))
        })?;
        let response = self
            .send(&body)
            .map_err(|e| Undeliverable::Failed(e.to_string()))?;
        parse_sent_id(&response).map_err(|e| Undeliverable::Failed(e.to_string()))
    }

    fn post(&self, text: &str) -> Result<String, Undeliverable> {
        let Some(channel) = &self.channel else {
            return Err(Undeliverable::Unconfigured);
        };
        let body = serde_json::json!({
            "chat_id": channel,
            "text": text,
            "disable_web_page_preview": true,
        })
        .to_string();
        let response = self
            .send(&body)
            .map_err(|e| Undeliverable::Failed(e.to_string()))?;
        parse_sent_id(&response).map_err(|e| Undeliverable::Failed(e.to_string()))
    }
}

/// Whether the operator has switched Telegram replies on.
///
/// Exactly `on`, for the reason [`crate::daemon::may_publish`] gives.
#[must_use]
pub fn may_publish(get: &impl Fn(&str) -> Option<String>) -> bool {
    get("RADAR_TELEGRAM_PUBLISH").is_some_and(|v| v.trim().eq_ignore_ascii_case("on"))
}

/// Which publisher Telegram replies go through.
#[must_use]
pub fn publisher_for(telegram: Option<Telegram>, publishing: bool) -> Box<dyn Publisher> {
    match (telegram, publishing) {
        (Some(bot), true) => Box::new(bot),
        _ => Box::new(DryRun),
    }
}

/// What the daemon says about the Telegram lane on start.
#[must_use]
pub fn posture(configured: bool, publishing: bool) -> &'static str {
    match (configured, publishing) {
        (false, _) => {
            "radar-analyst: telegram off -- no RADAR_TELEGRAM_BOT_TOKEN, so nothing is read there and nothing is sent."
        }
        (true, false) => {
            "radar-analyst: telegram reading messages and answering them to telegram.jsonl ONLY -- \
             set RADAR_TELEGRAM_PUBLISH=on to reply in the chat."
        }
        (true, true) => "radar-analyst: telegram LIVE -- replies are being sent in the chat.",
    }
}

/// The Telegram lane's caps. Separate from X's: a different bill, a different
/// room, and a global cap here is a cost ceiling on model calls, not on posts.
#[must_use]
pub fn limits_from(get: &impl Fn(&str) -> Option<String>) -> Limits {
    let n = |key: &str| get(key).and_then(|v| v.trim().parse().ok()).unwrap_or(0);
    Limits {
        per_summoner_daily: n("RADAR_TELEGRAM_PER_SUMMONER_DAILY"),
        global_daily: n("RADAR_TELEGRAM_GLOBAL_DAILY"),
        dedupe_seconds: get("RADAR_TELEGRAM_DEDUPE_SECONDS")
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(3_600),
    }
}

/// One poll of the Telegram lane. Returns how many messages were answered.
///
/// The same sequence as the X loop -- admit before the chain is read, record
/// before anything is said, tell the gate only what was actually sent -- and
/// none of the X costs, because the platform bills nothing. The one thing it
/// spends is the model call inside the reply, which the X loop does not meter
/// separately either.
///
/// The offset is written whenever the page named one, answered or not: an
/// unacknowledged update is re-sent on every poll, so a refused message that
/// did not move the cursor would be refused again for ever and cost a log line
/// each time.
#[allow(clippy::too_many_arguments)]
pub fn tick(
    telegram: Option<&Telegram>,
    publisher: &dyn Publisher,
    gate: &mut Gate,
    client: &radar_onchain::RpcClient,
    rates: Option<&radar_roast::BaseRates>,
    creators: Option<&radar_roast::CreatorIndex>,
    provider: Option<&dyn radar_model::Provider>,
    self_mint: Option<&radar_types::Address>,
    paths: &crate::daemon::Paths,
) -> usize {
    let Some(bot) = telegram else {
        return 0;
    };
    let at = crate::daemon::now();
    let cursor = crate::poll::read_cursor(&paths.telegram_cursor);
    let page = match bot.updates(cursor.as_deref()) {
        Ok(page) => page,
        Err(e) => {
            eprintln!("radar-analyst: telegram poll failed: {e}");
            return 0;
        }
    };

    let ctx = Answering {
        client,
        rates,
        creators,
        provider,
        self_mint,
        now: at,
    };
    let mut answered = 0;
    for mention in &page.mentions {
        match crate::answer::answer(mention, gate, &ctx) {
            Answered::Reply(entry) => {
                let mint = entry.mint.clone().unwrap_or_default();
                match crate::publish::publish(publisher, &paths.telegram_log, *entry) {
                    Ok(written) => {
                        if let Some(id) = &written.reply_id {
                            gate.record(&mention.author, &mint, id, at);
                            answered += 1;
                        }
                    }
                    Err(e) => {
                        eprintln!("radar-analyst: cannot write {}: {e}", paths.telegram_log);
                        break;
                    }
                }
            }
            other => eprintln!("radar-analyst: telegram {} -> {other:?}", mention.id),
        }
    }

    if let Some(next) = page.next_offset
        && let Err(e) = crate::poll::write_cursor(&paths.telegram_cursor, &next)
    {
        eprintln!("radar-analyst: cannot save the telegram cursor: {e}");
    }
    answered
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let owned: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |k: &str| {
            owned
                .iter()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.clone())
        }
    }

    const PAGE: &str = r#"{"ok":true,"result":[
      {"update_id":700,"message":{"message_id":41,"from":{"id":9001,"is_bot":false},"chat":{"id":-100777,"type":"supergroup"},"text":"@radar_bot So11111111111111111111111111111111111111112"}},
      {"update_id":701,"message":{"message_id":42,"from":{"id":9002},"chat":{"id":-100777},"sticker":{"file_id":"x"}}},
      {"update_id":699,"message":{"message_id":40,"from":{"id":9003},"chat":{"id":5},"text":"what is this","reply_to_message":{"message_id":38,"text":"..."}}},
      {"update_id":702,"edited_message":{"message_id":41,"text":"edited"}}
    ]}"#;

    #[test]
    fn a_page_reads_into_mentions_with_chat_scoped_ids_and_the_next_offset() {
        let page = parse_updates(PAGE).expect("a page");
        assert_eq!(
            page.mentions.len(),
            2,
            "the sticker and the edit are skipped: {page:?}"
        );
        assert_eq!(page.mentions[0].id, "-100777:41");
        assert_eq!(page.mentions[0].author, "tg:9001");
        assert_eq!(page.mentions[0].parent, None);
        assert_eq!(page.mentions[1].id, "5:40");
        assert_eq!(page.mentions[1].parent.as_deref(), Some("5:38"));
        // One past the LARGEST id, not the last: 702 is the edit, which was
        // skipped as a mention and still has to be acknowledged, or Telegram
        // re-sends it on every poll.
        assert_eq!(page.next_offset.as_deref(), Some("703"));
    }

    #[test]
    fn an_empty_page_keeps_the_cursor_and_a_refusal_is_not_an_empty_page() {
        let page = parse_updates(r#"{"ok":true,"result":[]}"#).expect("empty");
        assert!(page.mentions.is_empty());
        assert_eq!(page.next_offset, None);

        let refused =
            parse_updates(r#"{"ok":false,"error_code":401,"description":"Unauthorized"}"#);
        assert!(
            matches!(refused, Err(Unreachable::Unreadable(ref why)) if why.contains("Unauthorized")),
            "{refused:?}"
        );
        assert!(parse_updates("not json").is_err());
        assert!(parse_updates(r#"{"ok":true,"result":{}}"#).is_err());
    }

    #[test]
    fn a_message_missing_its_sender_or_chat_is_skipped_not_attributed_to_nobody() {
        // Re-applied by defaulting `from` to 0: the message is attributed to
        // `tg:0`, one summoner's allowance is shared by every malformed update,
        // and this fails.
        let body = r#"{"ok":true,"result":[
          {"update_id":1,"message":{"message_id":1,"chat":{"id":5},"text":"So11111111111111111111111111111111111111112"}},
          {"update_id":2,"message":{"message_id":2,"from":{"id":7},"text":"So11111111111111111111111111111111111111112"}}
        ]}"#;
        let page = parse_updates(body).expect("a page");
        assert!(page.mentions.is_empty(), "{page:?}");
        assert_eq!(page.next_offset.as_deref(), Some("3"));
    }

    #[test]
    fn the_url_carries_the_token_asks_for_messages_only_and_cleans_the_offset() {
        let bot = Telegram::at("https://tg.test", "123:ABC");
        assert_eq!(
            bot.updates_url(None),
            "https://tg.test/bot123:ABC/getUpdates?timeout=0&allowed_updates=%5B%22message%22%5D"
        );
        assert!(bot.updates_url(Some("703")).ends_with("&offset=703"));
        // A cursor that is not digits contributes nothing rather than a
        // malformed query.
        assert!(!bot.updates_url(Some("7&x=1")).contains("x=1"));
        assert!(bot.updates_url(Some("abc")).ends_with("message%22%5D"));
    }

    #[test]
    fn the_token_is_not_in_the_debug_output() {
        let bot = Telegram::at("https://tg.test", "123:SECRET");
        let shown = format!("{bot:?}");
        assert!(!shown.contains("SECRET"), "{shown}");
        assert!(shown.contains("tg.test"));
    }

    #[test]
    fn a_reply_goes_to_the_chat_the_question_was_in_and_nowhere_else() {
        let body = send_body("-100777:41", "Six token accounts.").expect("a body");
        let value: serde_json::Value = serde_json::from_str(&body).expect("json");
        assert_eq!(value["chat_id"], -100_777);
        assert_eq!(value["reply_parameters"]["message_id"], 41);
        assert_eq!(value["text"], "Six token accounts.");
        // No chat, no guess.
        assert_eq!(send_body("41", "x"), None);
        assert_eq!(send_body("abc:41", "x"), None);
        assert_eq!(send_body("5:forty", "x"), None);
    }

    #[test]
    fn a_sent_message_id_is_read_back_in_the_same_shape_or_not_at_all() {
        let ok = r#"{"ok":true,"result":{"message_id":43,"chat":{"id":-100777},"text":"..."}}"#;
        assert_eq!(parse_sent_id(ok).expect("an id"), "-100777:43");
        assert!(parse_sent_id(r#"{"ok":false,"description":"Bad Request"}"#).is_err());
        assert!(parse_sent_id(r#"{"ok":true,"result":{"text":"no id"}}"#).is_err());
        assert!(parse_sent_id("nope").is_err());
    }

    #[test]
    fn no_token_is_no_bot_and_a_blank_token_is_no_token() {
        assert!(Telegram::from_vars(&vars(&[])).is_none());
        assert!(Telegram::from_vars(&vars(&[("RADAR_TELEGRAM_BOT_TOKEN", "  ")])).is_none());
        let bot =
            Telegram::from_vars(&vars(&[("RADAR_TELEGRAM_BOT_TOKEN", " 1:a ")])).expect("a bot");
        assert_eq!(bot.base, API);
        assert!(
            bot.updates_url(None)
                .starts_with("https://api.telegram.org/bot1:a/")
        );
        let elsewhere = Telegram::from_vars(&vars(&[
            ("RADAR_TELEGRAM_BOT_TOKEN", "1:a"),
            ("RADAR_TELEGRAM_API_BASE", "http://127.0.0.1:9"),
        ]))
        .expect("a bot");
        assert!(
            elsewhere
                .updates_url(None)
                .starts_with("http://127.0.0.1:9/bot1:a/")
        );
    }

    #[test]
    fn only_a_token_that_is_switched_on_speaks_and_the_switch_is_exactly_on() {
        let bot = || Telegram::at("https://tg.test", "1:a");
        assert_eq!(publisher_for(Some(bot()), true).name(), "telegram");
        assert_eq!(publisher_for(Some(bot()), false).name(), "dry-run");
        assert_eq!(publisher_for(None, true).name(), "dry-run");
        assert!(may_publish(&vars(&[("RADAR_TELEGRAM_PUBLISH", " ON ")])));
        for value in ["", "true", "1", "yes", "onn"] {
            assert!(
                !may_publish(&vars(&[("RADAR_TELEGRAM_PUBLISH", value)])),
                "{value:?}"
            );
        }
        assert!(!may_publish(&vars(&[])));
    }

    #[test]
    fn the_three_states_are_told_apart_and_the_caps_are_zero_unless_set() {
        assert!(posture(false, false).contains("telegram off"));
        assert!(posture(false, true).contains("telegram off"));
        assert!(posture(true, false).contains("ONLY"));
        assert!(posture(true, false).contains("RADAR_TELEGRAM_PUBLISH=on"));
        assert!(posture(true, true).contains("LIVE"));

        let closed = limits_from(&vars(&[]));
        assert_eq!((closed.per_summoner_daily, closed.global_daily), (0, 0));
        assert_eq!(closed.dedupe_seconds, 3_600);
        let open = limits_from(&vars(&[
            ("RADAR_TELEGRAM_PER_SUMMONER_DAILY", "20"),
            ("RADAR_TELEGRAM_GLOBAL_DAILY", "500"),
            ("RADAR_TELEGRAM_DEDUPE_SECONDS", "60"),
        ]));
        assert_eq!(
            (
                open.per_summoner_daily,
                open.global_daily,
                open.dedupe_seconds
            ),
            (20, 500, 60)
        );
    }
}
