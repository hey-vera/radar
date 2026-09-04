// SPDX-License-Identifier: Apache-2.0
//! Reading mentions from X, and replying to them.
//!
//! # The one impure module in this crate
//!
//! Everything else here is pure or file-backed and testable without a network.
//! This is the edge, and it is shaped so that as little as possible has to be
//! taken on trust: **building a request and reading a response are pure
//! functions**, and only [`X::get`] and [`X::post`] touch the wire. So the
//! things that can be wrong in a way a test would notice — the URL, the query
//! parameters, the JSON shape, what counts as retryable — are all exercised
//! without an account.
//!
//! That split is not decoration. The two facts this module cannot check for
//! itself are whether the endpoint accepts the request and what it bills, and
//! both are settled with a live credential rather than by reading more.
//!
//! # Rule 8
//!
//! [`X::from_env`] returns `None` when the credential is absent, and the binary
//! then uses [`DryRun`](crate::DryRun). An instance with no token **cannot
//! post**, rather than posting badly. That is the crate's resting state and
//! this module does not change it.
//!
//! # The author id, not the handle
//!
//! A mention carries `author_id`, a stable numeric id, and this module keeps
//! that as the summoner rather than resolving it to an @name. A handle can be
//! changed by its owner and reused by somebody else, so a per-summoner rate
//! limit keyed on one is a limit that can be shed by renaming. The id cannot be
//! changed and cannot be transferred.

use std::time::Duration;

use crate::log::Entry;
use crate::publish::{Publisher, Undeliverable};

/// One mention, from whatever source produced it.
///
/// Shared with the JSONL fixture reader on purpose: the file and the API are two
/// sources of the same thing, and a type each would let the fixture drift away
/// from what the account actually receives — which would make reading two
/// hundred dry-run replies a test of the fixture rather than of the bot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mention {
    /// The mention's id on the platform.
    pub id: String,
    /// The author's stable numeric id. Never the handle — see the module note.
    pub author: String,
    /// The text, exactly as written. Untrusted (rule 4).
    pub text: String,
    /// The post this one replies to, when it is a reply.
    ///
    /// Carried so a mention with no address in its own text can look one up in
    /// the post it was made under, which is how somebody actually asks: they
    /// reply "@radar what is this" to a tweet that names the coin.
    pub parent: Option<String>,
}

/// Why a call to the platform did not produce an answer.
#[derive(Debug, thiserror::Error)]
pub enum Unreachable {
    /// The request was refused, with the status and the body.
    ///
    /// Kept apart from [`Self::Transport`] because the two need opposite
    /// responses: a refusal is about the request and retrying it unchanged
    /// asks the same question again.
    #[error("x refused with {status}: {body}")]
    Refused {
        /// HTTP status.
        status: u16,
        /// Response body, truncated.
        body: String,
    },
    /// The request never got an answer.
    #[error("x unreachable: {0}")]
    Transport(String),
    /// The answer arrived and was not what this module can read.
    #[error("x sent something unreadable: {0}")]
    Unreadable(String),
}

/// How long to wait before trying again, or `None` to stop trying.
///
/// # Why a 4xx is never retried
///
/// A refusal that is not 429 is a statement about the request, and the request
/// does not change by being sent again. Retrying it spends money and rate limit
/// to receive the same answer, and — worse — it turns a bug that would have
/// been loud once into one that is quiet and continuous.
///
/// 429 is the exception and it is not an exception to the reasoning: it says
/// *this request, later*, which is a different request in the only dimension
/// that matters.
///
/// The ceiling is fifteen minutes. Past that the account is not rate-limited,
/// it is broken, and a poll loop that keeps quietly doubling is a bot that has
/// stopped answering without anybody being told.
#[must_use]
pub fn backoff(attempt: u32, status: Option<u16>) -> Option<Duration> {
    const CEILING: Duration = Duration::from_secs(900);
    const BASE: Duration = Duration::from_secs(5);

    // Written as "these retry, nothing else does" rather than as a rule for
    // 4xx plus a catch-all. An earlier version had both, and mutation testing
    // showed the 4xx arm was **behaviourally redundant**: with it deleted,
    // every status still produced the same answer, because the catch-all
    // already returns `None`. Two arms deciding one thing is one arm too many,
    // and the one that could be removed without changing an answer is the one
    // that was not carrying the rule.
    match status {
        // Rate limited, or the platform is having a bad minute, or no answer at
        // all. All three mean "later", and all three double.
        Some(429 | 500..=599) | None => {
            let doubled = BASE.saturating_mul(1_u32.checked_shl(attempt).unwrap_or(u32::MAX));
            Some(doubled.min(CEILING))
        }
        // Everything else waits for nothing. A 4xx is a statement about the
        // request, which does not change by being sent again; a 2xx or 3xx
        // reaching here is not a failure at all.
        Some(_) => None,
    }
}

/// The client.
///
/// Holds the credential and nothing else. It does not decide what to say, and
/// the module that decides what to say holds no credential — the same split
/// [`crate::publish`] draws, kept as the code is filled in rather than only in
/// the plan.
#[derive(Clone)]
pub struct X {
    bearer: String,
    user_id: String,
    /// The API root, so tests can point at something that is not the platform.
    base: String,
}

// Written by hand so the token cannot reach a log through a derive. A struct
// this small does not need the derive, and a credential in a debug line is the
// kind of leak that is discovered in somebody else's log aggregator.
impl std::fmt::Debug for X {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("X")
            .field("user_id", &self.user_id)
            .field("base", &self.base)
            .field("bearer", &"<redacted>")
            .finish()
    }
}

/// The real API root.
const API: &str = "https://api.x.com";

/// Whether a pair of environment values is a usable credential.
///
/// Separated from [`X::from_env`] so the rule can be tested without setting
/// process-wide environment variables, which two tests running at once would
/// fight over. The rule itself is rule 8: **both or neither.** A token with no
/// user id has nothing to poll and a user id with no token cannot poll it, so a
/// half-configured instance is unconfigured rather than an error saved up for
/// the first request.
///
/// Whitespace counts as absent. An operator who clears a variable by leaving
/// `RADAR_X_BEARER=` in the file has removed the credential, and the difference
/// between that and a missing line is not one this should have an opinion about.
fn configured(bearer: Option<String>, user_id: Option<String>) -> Option<(String, String)> {
    let bearer = bearer?;
    let user_id = user_id?;
    if bearer.trim().is_empty() || user_id.trim().is_empty() {
        return None;
    }
    Some((bearer, user_id))
}

/// Most mentions to ask for in one page.
///
/// The endpoint's own maximum is 100. Asking for the maximum is right here: a
/// page is one billable read whatever it contains, so a smaller page costs the
/// same and covers less.
const PAGE: u32 = 100;

impl X {
    /// Builds a client from the environment, or `None` when it is not
    /// configured.
    ///
    /// Both variables are required. A token with no user id has nothing to poll
    /// and a user id with no token cannot poll it, so a half-configured
    /// instance is treated as unconfigured rather than as an error to report
    /// later — rule 8, and it means the binary falls back to the dry run.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        configured(
            std::env::var("RADAR_X_BEARER").ok(),
            std::env::var("RADAR_X_USER_ID").ok(),
        )
        .map(|(bearer, user_id)| Self {
            bearer,
            user_id,
            // Overridable, and defaulted to the real thing. This exists so the
            // daemon's loop can be driven against a fake platform end to end
            // rather than only in pieces, and so a staging account can point
            // elsewhere without a rebuild.
            //
            // It is the one setting here that is safe to default on absence,
            // because what it defaults to is production: a dropped variable
            // points at X rather than at nothing. Every other switch in this
            // crate defaults to refusing, and for the same underlying reason —
            // the default must be the outcome that cannot surprise anybody.
            base: std::env::var("RADAR_X_API_BASE").unwrap_or_else(|_| API.to_owned()),
        })
    }

    /// A client pointed at a different root, for tests.
    #[must_use]
    pub fn at(
        base: impl Into<String>,
        bearer: impl Into<String>,
        user_id: impl Into<String>,
    ) -> Self {
        Self {
            bearer: bearer.into(),
            user_id: user_id.into(),
            base: base.into(),
        }
    }

    /// The URL a mentions poll requests.
    ///
    /// Pure, so the query can be checked without spending a call. The fields
    /// are the ones the reply pipeline needs and no others: every extra field
    /// is more untrusted text to carry and none of it reaches a decision.
    #[must_use]
    pub fn mentions_url(&self, since_id: Option<&str>) -> String {
        let mut url = format!(
            "{}/2/users/{}/mentions?max_results={PAGE}\
             &tweet.fields=author_id,referenced_tweets",
            self.base, self.user_id
        );
        if let Some(since) = since_id {
            // Only the characters an id can contain. A cursor is read back from
            // a file this process wrote, but it is a value that came from the
            // network first, and a query parameter built from it is the one
            // place it could become something other than an id.
            let clean: String = since.chars().filter(char::is_ascii_digit).collect();
            if !clean.is_empty() {
                url.push_str("&since_id=");
                url.push_str(&clean);
            }
        }
        url
    }

    /// Reads mentions newer than `since_id`.
    ///
    /// # Errors
    ///
    /// [`Unreachable`] when the platform refuses, cannot be reached, or answers
    /// with something this module cannot read.
    pub fn mentions(&self, since_id: Option<&str>) -> Result<Vec<Mention>, Unreachable> {
        let body = self.get(&self.mentions_url(since_id))?;
        parse_mentions(&body)
    }

    /// GETs a URL with the bearer token.
    fn get(&self, url: &str) -> Result<String, Unreachable> {
        let response = ureq::get(url)
            .header("Authorization", &format!("Bearer {}", self.bearer))
            .call();
        Self::body_of(response)
    }

    /// POSTs a JSON body with the bearer token.
    fn post(&self, url: &str, body: &str) -> Result<String, Unreachable> {
        let response = ureq::post(url)
            .header("Authorization", &format!("Bearer {}", self.bearer))
            .header("Content-Type", "application/json")
            .send(body);
        Self::body_of(response)
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
}

/// Reads a mentions response.
///
/// Pure, and deliberately forgiving in one direction only: a mention missing a
/// field this crate needs is **skipped**, never defaulted. An entry with no
/// author would otherwise be attributed to the empty string and share one
/// summoner's daily allowance with every other malformed entry — which is the
/// same reasoning the JSONL reader already applies, kept identical because they
/// are two readers of one thing.
///
/// An absent `data` array is not an error. It is what the endpoint returns when
/// nothing has mentioned the account, which is most polls.
///
/// # Errors
///
/// [`Unreachable::Unreadable`] when the body is not JSON at all, or when the
/// platform reported an error instead of a page.
pub fn parse_mentions(body: &str) -> Result<Vec<Mention>, Unreachable> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| Unreachable::Unreadable(format!("not json: {e}")))?;

    // An error document is a 200 with `errors` in it for some failures, so a
    // status check alone does not catch this. Reported rather than read as an
    // empty page: "nobody mentioned us" and "we were not allowed to look" must
    // not be the same outcome, or the bot goes quiet and looks healthy.
    if value.get("data").is_none() {
        if let Some(errors) = value.get("errors") {
            return Err(Unreachable::Unreadable(format!("errors: {errors}")));
        }
        return Ok(Vec::new());
    }

    let Some(items) = value.get("data").and_then(serde_json::Value::as_array) else {
        return Err(Unreachable::Unreadable("data is not an array".to_owned()));
    };

    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let (Some(id), Some(author), Some(text)) = (
            item.get("id").and_then(serde_json::Value::as_str),
            item.get("author_id").and_then(serde_json::Value::as_str),
            item.get("text").and_then(serde_json::Value::as_str),
        ) else {
            continue;
        };
        out.push(Mention {
            id: id.to_owned(),
            author: author.to_owned(),
            text: text.to_owned(),
            parent: parent_of(item),
        });
    }
    Ok(out)
}

/// The post a mention replies to, if any.
///
/// `referenced_tweets` also carries quotes and retweets, and only `replied_to`
/// is the thread the asker is standing in. Taking any reference would sometimes
/// read the wrong post and answer about the wrong coin.
fn parent_of(item: &serde_json::Value) -> Option<String> {
    item.get("referenced_tweets")?
        .as_array()?
        .iter()
        .find(|r| r.get("type").and_then(serde_json::Value::as_str) == Some("replied_to"))?
        .get("id")?
        .as_str()
        .map(str::to_owned)
}

/// Reads the id out of a posted-reply response.
///
/// # Errors
///
/// [`Unreachable::Unreadable`] when the body carries no id. **This is treated
/// as a failure to publish**, which is the safe direction: the log then records
/// no reply id, and the worst case is a reply that exists and is recorded as
/// not existing. The opposite default — assuming success — would record a
/// reply id for something that was never posted.
pub fn parse_posted_id(body: &str) -> Result<String, Unreachable> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| Unreachable::Unreadable(format!("not json: {e}")))?;
    value
        .get("data")
        .and_then(|d| d.get("id"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| Unreachable::Unreadable(format!("no id in {value}")))
}

/// The body a reply posts.
///
/// Pure, and the reason it is a function is the escaping: the text contains a
/// creator-controlled token name, so it is built by a JSON serialiser rather
/// than by formatting a string. A reply is the one place in this system where
/// somebody else's bytes end up inside a document this process writes.
#[must_use]
pub fn reply_body(in_reply_to: &str, text: &str) -> String {
    serde_json::json!({
        "text": text,
        "reply": { "in_reply_to_tweet_id": in_reply_to },
    })
    .to_string()
}

impl Publisher for X {
    fn name(&self) -> &'static str {
        "x"
    }

    fn reply(&self, in_reply_to: &str, text: &str) -> Result<String, Undeliverable> {
        let url = format!("{}/2/tweets", self.base);
        let body = self
            .post(&url, &reply_body(in_reply_to, text))
            .map_err(|e| Undeliverable::Failed(e.to_string()))?;
        parse_posted_id(&body).map_err(|e| Undeliverable::Failed(e.to_string()))
    }
}

/// Turns a mention into the log entry a reply is recorded under.
///
/// Here rather than in the loop because it is the only place that knows both
/// shapes, and a caller assembling it by hand is a caller that can forget the
/// author.
#[must_use]
pub fn entry_for(mention: &Mention, at: u64) -> Entry {
    Entry {
        at,
        mention_id: mention.id.clone(),
        summoner: mention.author.clone(),
        mint: None,
        read_at_slot: None,
        fact_sheet: String::new(),
        reply: String::new(),
        fellback: None,
        reply_id: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A page shaped the way the endpoint documents it.
    const PAGE_BODY: &str = r#"{
      "data": [
        {"id":"1","author_id":"a1","text":"@radar what is So11111111111111111111111111111111111111112"},
        {"id":"2","author_id":"a2","text":"@radar thoughts?",
         "referenced_tweets":[{"type":"replied_to","id":"parent-9"}]}
      ],
      "meta": {"result_count": 2, "newest_id": "2"}
    }"#;

    #[test]
    fn a_page_reads_into_mentions_with_the_parent_kept() {
        let got = parse_mentions(PAGE_BODY).expect("a page");
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].id, "1");
        assert_eq!(got[0].author, "a1");
        assert!(got[0].text.contains("So1111111"));
        assert_eq!(got[0].parent, None);
        assert_eq!(got[1].parent.as_deref(), Some("parent-9"));
    }

    #[test]
    fn an_empty_page_is_not_an_error_because_it_is_most_polls() {
        let got = parse_mentions(r#"{"meta":{"result_count":0}}"#).expect("empty page");
        assert!(got.is_empty());
    }

    #[test]
    fn an_error_document_is_not_read_as_an_empty_page() {
        // The failure this prevents: the account loses access, every poll
        // returns an error document, the bot answers nothing, and every check
        // says it is healthy because "no mentions" looks exactly like this.
        let body = r#"{"errors":[{"title":"Unauthorized","status":401}]}"#;
        let err = parse_mentions(body).expect_err("must not read as empty");
        assert!(matches!(err, Unreachable::Unreadable(_)), "{err:?}");
    }

    #[test]
    fn a_mention_missing_a_field_is_skipped_not_defaulted() {
        // An entry with no author would be attributed to "" and would share one
        // summoner's allowance with every other malformed entry.
        let body = r#"{"data":[
          {"id":"1","text":"no author"},
          {"id":"2","author_id":"a2","text":"complete"},
          {"author_id":"a3","text":"no id"}
        ]}"#;
        let got = parse_mentions(body).expect("a page");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].id, "2");
    }

    #[test]
    fn a_quote_is_not_treated_as_the_thread_it_is_standing_in() {
        // Only `replied_to` is the post the asker is under. Reading a quoted
        // post instead would answer about a different coin, confidently.
        let body = r#"{"data":[{"id":"1","author_id":"a1","text":"hi",
          "referenced_tweets":[{"type":"quoted","id":"other-post"}]}]}"#;
        let got = parse_mentions(body).expect("a page");
        assert_eq!(got[0].parent, None);
    }

    #[test]
    fn the_replied_to_reference_is_found_among_others() {
        let body = r#"{"data":[{"id":"1","author_id":"a1","text":"hi",
          "referenced_tweets":[{"type":"quoted","id":"q"},{"type":"replied_to","id":"p"}]}]}"#;
        let got = parse_mentions(body).expect("a page");
        assert_eq!(got[0].parent.as_deref(), Some("p"));
    }

    #[test]
    fn nonsense_is_reported_rather_than_read_as_nothing() {
        let err = parse_mentions("<html>502 Bad Gateway</html>").expect_err("not json");
        assert!(matches!(err, Unreachable::Unreadable(_)), "{err:?}");
    }

    #[test]
    fn the_mentions_url_carries_the_fields_the_pipeline_needs() {
        let x = X::at("https://example.test", "tok", "u42");
        let url = x.mentions_url(None);
        assert!(
            url.starts_with("https://example.test/2/users/u42/mentions?"),
            "{url}"
        );
        assert!(url.contains("max_results=100"), "{url}");
        assert!(url.contains("author_id"), "{url}");
        assert!(url.contains("referenced_tweets"), "{url}");
        assert!(!url.contains("since_id"), "{url}");
    }

    #[test]
    fn a_cursor_is_added_and_kept_to_digits() {
        // The cursor is read back from a file, but it came from the network
        // first. This is the one place it becomes part of a request.
        let x = X::at("https://example.test", "tok", "u42");
        assert!(x.mentions_url(Some("1789")).contains("&since_id=1789"));
        assert!(
            x.mentions_url(Some("17&max_results=1"))
                .contains("&since_id=171"),
            "the ampersand and letters must not survive"
        );
        assert!(
            !x.mentions_url(Some("../../evil")).contains("since_id"),
            "a cursor with no digits adds no parameter at all"
        );
    }

    #[test]
    fn the_reply_body_escapes_a_creator_controlled_name() {
        // A token name is arbitrary bytes chosen by somebody else, and this is
        // where they land inside a document this process writes. Built by a
        // serialiser, not by formatting.
        let body = reply_body("m1", "the \"symbol\" is \\ and \n a newline");
        let back: serde_json::Value = serde_json::from_str(&body).expect("valid json");
        assert_eq!(
            back["text"].as_str().expect("text"),
            "the \"symbol\" is \\ and \n a newline"
        );
        assert_eq!(back["reply"]["in_reply_to_tweet_id"], "m1");
    }

    #[test]
    fn a_posted_reply_yields_its_id() {
        assert_eq!(
            parse_posted_id(r#"{"data":{"id":"99","text":"hi"}}"#).expect("an id"),
            "99"
        );
    }

    #[test]
    fn a_response_with_no_id_is_a_failure_to_publish() {
        // The safe direction: the log records no reply id. Assuming success
        // would record an id for something that was never posted.
        assert!(parse_posted_id(r#"{"data":{}}"#).is_err());
        assert!(parse_posted_id(r#"{"errors":[{"title":"duplicate"}]}"#).is_err());
    }

    #[test]
    fn a_refusal_that_is_not_rate_limiting_is_never_retried() {
        // The request does not change by being sent again. Retrying turns a bug
        // that would have been loud once into one that is quiet and continuous.
        for status in [400, 401, 403, 404, 422] {
            assert!(
                backoff(0, Some(status)).is_none(),
                "{status} must not be retried"
            );
        }
    }

    #[test]
    fn rate_limiting_and_server_errors_are_retried_with_doubling() {
        for status in [429, 500, 503] {
            let first = backoff(0, Some(status)).expect("retryable");
            let second = backoff(1, Some(status)).expect("retryable");
            assert_eq!(first, Duration::from_secs(5));
            assert_eq!(second, Duration::from_secs(10));
        }
        // A transport failure has no status and is the same kind of "later".
        assert_eq!(backoff(0, None), Some(Duration::from_secs(5)));
    }

    #[test]
    fn the_wait_stops_doubling_at_fifteen_minutes() {
        // Past the ceiling the account is not rate-limited, it is broken, and a
        // loop that keeps doubling is a bot that stopped answering with nobody
        // told. Checked at a shift that would overflow, too.
        assert_eq!(backoff(20, Some(429)), Some(Duration::from_secs(900)));
        assert_eq!(backoff(64, Some(429)), Some(Duration::from_secs(900)));
        assert_eq!(backoff(u32::MAX, Some(500)), Some(Duration::from_secs(900)));
    }

    #[test]
    fn a_success_reaching_the_backoff_waits_for_nothing() {
        assert!(backoff(0, Some(200)).is_none());
    }

    #[test]
    fn the_credential_is_absent_from_the_debug_rendering() {
        // A token in a debug line is discovered in somebody else's log
        // aggregator, which is the worst place to discover it.
        let rendered = format!("{:?}", X::at("https://example.test", "super-secret", "u1"));
        assert!(!rendered.contains("super-secret"), "{rendered}");
        assert!(rendered.contains("<redacted>"), "{rendered}");
        assert!(rendered.contains("u1"), "{rendered}");
    }

    /// A one-request HTTP server, for exercising the wire.
    ///
    /// # Why this is worth sixty lines
    ///
    /// Everything above this point is a pure function, and mutation testing
    /// said so loudly: `get`, `post`, `body_of`, `mentions` and the `Publisher`
    /// impl could each be replaced with `Ok(...)` and every test still passed.
    /// Those are exactly the functions that carry the credential, choose the
    /// method and decide what a refusal means, so exempting them as plumbing
    /// would exempt the half of this module that can lose money or leak a token.
    ///
    /// A real listener on a real socket is cheaper than it looks and needs no
    /// dependency: `TcpListener` binds port 0, the OS picks a free port, and the
    /// thread answers exactly one request and stops. It also checks the thing no
    /// unit test can — that the **Authorization header is actually sent** —
    /// which is otherwise assumed until the first live call fails.
    fn serve_once(status: &str, body: &str) -> (String, std::sync::mpsc::Receiver<String>) {
        use std::io::{Read as _, Write as _};

        /// The end of an HTTP header block.
        const CRLF_CRLF: [u8; 4] = [13, 10, 13, 10];

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
        let port = listener.local_addr().expect("an address").port();
        let (tx, rx) = std::sync::mpsc::channel();

        let status = status.to_owned();
        let body = body.to_owned();
        std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            // Read until the request is complete, not until the socket first
            // yields. A POST arrives as headers and then a body in a second
            // segment, and a single read saw only the headers -- which failed
            // the body assertion while the code was correct.
            //
            // Byte windows rather than string escapes: an earlier version
            // searched for a CRLF written as an escape sequence, the escape did
            // not survive being written to the file, and the loop blocked
            // forever on a match that could never happen. Bytes cannot be
            // mangled that way.
            //
            // The read timeout is the belt to that pair of braces: whatever
            // else goes wrong, this thread ends and the test fails instead of
            // hanging a runner.
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let mut buf = vec![0_u8; 8192];
            let mut n = 0;
            while n < buf.len() {
                let Ok(read) = stream.read(&mut buf[n..]) else {
                    break;
                };
                if read == 0 {
                    break;
                }
                n += read;
                let Some(head_end) = buf[..n].windows(4).position(|w| w == CRLF_CRLF) else {
                    continue;
                };
                let head = String::from_utf8_lossy(&buf[..head_end]).to_lowercase();
                let want: usize = head
                    .split("content-length:")
                    .nth(1)
                    .and_then(|rest| rest.split_whitespace().next())
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                if n >= head_end + 4 + want {
                    break;
                }
            }
            let request = String::from_utf8_lossy(&buf[..n]).into_owned();
            let _ = tx.send(request);

            // Built from bytes for the same reason the reader above is: a CRLF
            // written as an escape sequence did not survive being written to
            // this file, and HTTP is one of the few things that genuinely means
            // carriage-return-line-feed rather than "a newline".
            let crlf = String::from_utf8(vec![13, 10]).expect("ascii is utf-8");
            let head = format!(
                "HTTP/1.1 {status}{crlf}Content-Length: {len}{crlf}Content-Type: application/json{crlf}Connection: close{crlf}{crlf}",
                len = body.len()
            );
            let response = format!("{head}{body}");
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        });

        (format!("http://127.0.0.1:{port}"), rx)
    }

    #[test]
    fn a_mentions_poll_sends_the_token_and_reads_the_page() {
        let (base, seen) = serve_once("200 OK", PAGE_BODY);
        let x = X::at(base, "secret-token", "u42");

        let got = x.mentions(Some("1700")).expect("a page");
        assert_eq!(got.len(), 2, "the page must be read, not invented");
        assert_eq!(got[0].id, "1");

        let request = seen.recv().expect("the server saw a request");
        assert!(
            request.starts_with("GET /2/users/u42/mentions"),
            "{request}"
        );
        assert!(
            request.contains("since_id=1700"),
            "the cursor must reach the wire: {request}"
        );
        assert!(
            request
                .to_lowercase()
                .contains("authorization: bearer secret-token"),
            "an unauthenticated poll returns nothing and looks like a quiet account: {request}"
        );
    }

    #[test]
    fn a_reply_posts_the_body_to_the_tweets_endpoint_and_returns_the_id() {
        let (base, seen) = serve_once("201 Created", r#"{"data":{"id":"posted-7"}}"#);
        let x = X::at(base, "secret-token", "u42");

        let id = x
            .reply("m1", "the round trip here is about 30%")
            .expect("posted");
        assert_eq!(id, "posted-7");

        let request = seen.recv().expect("the server saw a request");
        assert!(request.starts_with("POST /2/tweets"), "{request}");
        assert!(
            request
                .to_lowercase()
                .contains("authorization: bearer secret-token"),
            "{request}"
        );
        assert!(
            request.contains(r#""in_reply_to_tweet_id":"m1""#),
            "a reply that names no parent is a top-level post, which is a              different product and a different price: {request}"
        );
        assert!(request.contains("round trip"), "{request}");
    }

    #[test]
    fn a_refusal_carries_its_status_rather_than_becoming_an_empty_page() {
        // The failure this prevents is the quiet one: a 401 read as "no
        // mentions" leaves every check green while the bot answers nobody.
        let (base, _seen) = serve_once("401 Unauthorized", r#"{"title":"Unauthorized"}"#);
        let x = X::at(base, "stale-token", "u42");

        let err = x.mentions(None).expect_err("a refusal");
        match err {
            Unreachable::Refused { status, .. } => assert_eq!(status, 401),
            other => panic!("a 401 must not be read as anything else: {other:?}"),
        }
    }

    #[test]
    fn a_reply_the_platform_refuses_is_undeliverable_rather_than_silently_fine() {
        let (base, _seen) = serve_once("403 Forbidden", r#"{"title":"Forbidden"}"#);
        let x = X::at(base, "tok", "u42");
        let err = x.reply("m1", "text").expect_err("a refusal");
        assert!(matches!(err, Undeliverable::Failed(_)), "{err:?}");
    }

    #[test]
    fn a_two_hundred_carrying_no_id_is_still_a_failure_to_publish() {
        // X answers some failures with a 200 and an `errors` document. Reading
        // that as success would record a reply id for a reply that never
        // existed, which is the direction that cannot be corrected later.
        let (base, _seen) = serve_once("200 OK", r#"{"errors":[{"title":"duplicate content"}]}"#);
        let x = X::at(base, "tok", "u42");
        assert!(x.reply("m1", "text").is_err());
    }

    #[test]
    fn a_credential_needs_both_halves_or_it_is_not_a_credential() {
        // Rule 8, and the reason it is both-or-neither: a token with no user id
        // has nothing to poll, and a user id with no token cannot poll it.
        let some = |s: &str| Some(s.to_owned());
        assert_eq!(
            configured(some("tok"), some("u1")),
            Some(("tok".to_owned(), "u1".to_owned()))
        );
        assert_eq!(configured(None, some("u1")), None);
        assert_eq!(configured(some("tok"), None), None);
        assert_eq!(configured(None, None), None);
        // Whitespace is absence: clearing a variable by leaving `VAR=` in the
        // file removes the credential.
        assert_eq!(configured(some("  "), some("u1")), None);
        assert_eq!(configured(some("tok"), some("	")), None);
    }

    #[test]
    fn the_publisher_says_which_publisher_it_is() {
        // It goes into the reply log, which is the durable record of what this
        // account said and through what. A run recorded under the wrong
        // publisher is a run somebody later reads as a dry run.
        assert_eq!(X::at("https://example.test", "t", "u").name(), "x");
    }

    #[test]
    fn an_entry_carries_the_author_as_the_summoner() {
        let m = Mention {
            id: "m1".to_owned(),
            author: "a1".to_owned(),
            text: "hi".to_owned(),
            parent: None,
        };
        let e = entry_for(&m, 1_788_000_000);
        assert_eq!(e.mention_id, "m1");
        assert_eq!(e.summoner, "a1");
        assert!(e.reply_id.is_none());
    }
}
