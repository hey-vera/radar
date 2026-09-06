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

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use radar_contest::Metrics;

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

/// What one entrant's account says about itself, as read at week close.
///
/// Both fields come from a single `/2/users` lookup. `created_at` decides the
/// age rule; `username` is what the leaderboard prints instead of a numeric id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    /// When the account was created, seconds since the epoch.
    pub created_at: u64,
    /// Its handle, when the platform returned one.
    pub username: Option<String>,
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
    /// The four values that let this account **speak**, or `None`.
    ///
    /// Separate from the bearer because the platform treats them separately:
    /// reading mentions is an app-only operation and the bearer is the cheapest
    /// way to do it, while `POST /2/tweets` refuses an app-only token outright
    /// and requires user context.
    ///
    /// `None` is a client that can read and cannot post, which is a state worth
    /// having rather than an error: it is what an instance looks like before
    /// somebody has finished the credential, and [`Publisher::reply`] says so in
    /// those words instead of failing at the platform.
    oauth: Option<crate::oauth::Credentials>,
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
            // Present or absent, never the values. Whether this client can post
            // is the thing somebody reading a log line actually wants, and it is
            // not a secret; the four strings behind it are.
            .field(
                "oauth",
                &if self.oauth.is_some() {
                    "<redacted, can post>"
                } else {
                    "none, cannot post"
                },
            )
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
            // Read separately, and absent is not an error. The bearer is what
            // makes this instance able to read; these are what make it able to
            // speak, and an operator who has set up only the first has an
            // instance that answers into the log -- which is the state the
            // launch gate is read in.
            oauth: crate::oauth::Credentials::from_env(),
        })
    }

    /// A client pointed at a different root, for tests.
    ///
    /// Carries no OAuth credential, so a test using it can read and cannot post
    /// — the same shape as a half-configured instance. Use [`Self::signing`] for
    /// a client that can.
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
            oauth: None,
        }
    }

    /// The same, with a credential that can sign a post.
    #[must_use]
    pub fn signing(
        base: impl Into<String>,
        bearer: impl Into<String>,
        user_id: impl Into<String>,
        oauth: crate::oauth::Credentials,
    ) -> Self {
        Self {
            bearer: bearer.into(),
            user_id: user_id.into(),
            base: base.into(),
            oauth: Some(oauth),
        }
    }

    /// Whether this client can post, as opposed to only read.
    #[must_use]
    pub const fn can_post(&self) -> bool {
        self.oauth.is_some()
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
    /// Posts a JSON body, signed with OAuth 1.0a.
    ///
    /// **Not the bearer.** `POST /2/tweets` refuses an app-only token and
    /// requires user context — the client sent a bearer here until 2026-09-05,
    /// which read correctly and would have been refused on the first real reply.
    ///
    /// The body is **not** part of the signature. OAuth 1.0a folds body
    /// parameters into the base string only for `application/x-www-form-urlencoded`,
    /// and this is JSON; signing it produces a signature the platform rejects.
    /// The same goes for the query, which is empty on this endpoint and is
    /// passed explicitly so the reason is visible rather than implied.
    fn post(&self, url: &str, body: &str) -> Result<String, Unreachable> {
        let Some(credentials) = &self.oauth else {
            return Err(Unreachable::Transport(
                "no OAuth credential, so this instance can read but cannot post -- set \
                 RADAR_X_API_KEY, RADAR_X_API_SECRET, RADAR_X_ACCESS_TOKEN and \
                 RADAR_X_ACCESS_SECRET"
                    .to_owned(),
            ));
        };
        let authorization = crate::oauth::authorization(
            credentials,
            "POST",
            url,
            &[],
            crate::daemon::now(),
            &crate::oauth::nonce(),
        );
        let response = ureq::post(url)
            .header("Authorization", &authorization)
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

/// The body of a top-level post.
#[must_use]
pub fn post_body(text: &str) -> String {
    serde_json::json!({ "text": text }).to_string()
}

/// Most ids one lookup may carry. The platform's own limit on both endpoints.
const LOOKUP: usize = 100;

/// How many user resources one engager page may return.
///
/// **One page per endpoint per entry, and no pagination.** The walk below is
/// bounded by arithmetic rather than by patience, and X bills these per user
/// returned at ten times a post read -- so a second page is a second bill for a
/// reply that a hundred distinct accounts already engaged with, which is not a
/// reply whose ranking is in doubt. A hundred is the platform's own maximum.
const ENGAGERS_PER_PAGE: usize = 100;

/// Who engaged with one reply, as distinct accounts with their ages.
///
/// Accounts, not counts. That is the whole difference between this and
/// [`Metrics`]: `quote_count` is ten when one account quotes ten times, and
/// `quoted` has one entry. Research 0029, S16.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Engagers {
    /// Accounts that reposted it.
    pub reposted: BTreeMap<String, Account>,
    /// Accounts that quoted it, however many times each.
    pub quoted: BTreeMap<String, Account>,
    /// Accounts that liked it.
    pub liked: BTreeMap<String, Account>,
}

impl Engagers {
    /// Every distinct account seen across all three reads.
    ///
    /// A person who reposted *and* liked is one engager, which is what makes
    /// this a count of people rather than of actions.
    #[must_use]
    pub fn distinct(&self) -> BTreeSet<&str> {
        self.reposted
            .keys()
            .chain(self.quoted.keys())
            .chain(self.liked.keys())
            .map(String::as_str)
            .collect()
    }

    /// How many user resources these reads billed for.
    ///
    /// The sum of the three pages, **not** the distinct count: X charges per
    /// user returned, and an account that reposted and liked was returned
    /// twice. Charging the distinct count would under-count the bill by
    /// exactly the overlap, which is largest on the replies that engaged most
    /// people.
    #[must_use]
    pub fn billed_resources(&self) -> usize {
        self.reposted.len() + self.quoted.len() + self.liked.len()
    }
}

impl X {
    /// The account's own id: the contest's operator, who never wins.
    #[must_use]
    pub fn user_id(&self) -> &str {
        &self.user_id
    }

    /// The URL that reads the public metrics of the account's own replies.
    ///
    /// Pure. Only ids, only digits: an id came from the platform and is read
    /// back out of a log file, and this is the one place it becomes a query.
    #[must_use]
    pub fn metrics_url(&self, ids: &[String]) -> String {
        format!(
            "{}/2/tweets?ids={}&tweet.fields=public_metrics",
            self.base,
            digits_joined(ids)
        )
    }

    /// The URL that reads when the entrants' accounts were created.
    #[must_use]
    pub fn accounts_url(&self, ids: &[String]) -> String {
        format!(
            "{}/2/users?ids={}&user.fields=created_at,username",
            self.base,
            digits_joined(ids)
        )
    }

    /// The public metrics of the given replies, keyed by reply id.
    ///
    /// Read at week close (design 0007 C2), a hundred ids to the call. A reply
    /// the platform did not return -- deleted, hidden -- is simply absent, and
    /// the caller treats absent as unscored rather than as zero.
    ///
    /// # Errors
    ///
    /// [`Unreachable`] on the first page that fails; nothing partial is kept.
    pub fn metrics(&self, ids: &[String]) -> Result<BTreeMap<String, Metrics>, Unreachable> {
        let mut out = BTreeMap::new();
        for page in ids.chunks(LOOKUP) {
            let body = self.get(&self.metrics_url(page))?;
            out.extend(parse_metrics(&body)?);
        }
        Ok(out)
    }

    /// What is known about each entrant's account, keyed by id.
    ///
    /// The contest's age rule needs the creation time (design 0007 §6.2). An
    /// account the platform did not return is absent, and the caller treats
    /// that as an age it could not read -- excluded as unknown, never as old
    /// enough.
    ///
    /// The handle rides along on the same call, the same page and the same
    /// price: `user.fields` takes a list, and asking for `username` beside
    /// `created_at` costs nothing extra. Design 0008 §5.2 asked for handles on
    /// the leaderboard, and the alternative was a second metered call to learn
    /// something this one was already entitled to return.
    ///
    /// # Errors
    ///
    /// [`Unreachable`] on the first page that fails.
    pub fn accounts(&self, ids: &[String]) -> Result<BTreeMap<String, Account>, Unreachable> {
        let mut out = BTreeMap::new();
        for page in ids.chunks(LOOKUP) {
            let body = self.get(&self.accounts_url(page))?;
            out.extend(parse_accounts(&body)?);
        }
        Ok(out)
    }

    /// The three URLs one engager scan reads.
    ///
    /// Built here rather than inline so the query can be checked without a
    /// call, the way `metrics_url` and `accounts_url` are. `created_at` is the
    /// field the age floor is applied to; without it every engager would be of
    /// unknown age and rule 9 would exclude the lot.
    #[must_use]
    pub fn engager_urls(&self, reply_id: &str) -> [String; 3] {
        // Digits only, for the reason `metrics_url` gives: the id came off the
        // network before it came out of our own record.
        let id: String = reply_id.chars().filter(char::is_ascii_digit).collect();
        let fields = "user.fields=created_at,username";
        [
            format!(
                "{}/2/tweets/{id}/retweeted_by?{fields}&max_results={ENGAGERS_PER_PAGE}",
                self.base
            ),
            format!(
                "{}/2/tweets/{id}/liking_users?{fields}&max_results={ENGAGERS_PER_PAGE}",
                self.base
            ),
            // Quotes come back as posts, so the authors arrive under
            // `includes.users` rather than `data`. That is why this one is
            // parsed differently below and not because the shape was guessed.
            format!(
                "{}/2/tweets/{id}/quote_tweets?expansions=author_id&{fields}&max_results={ENGAGERS_PER_PAGE}",
                self.base
            ),
        ]
    }

    /// Who engaged with one reply.
    ///
    /// Three reads, one page each. Returns accounts rather than counts, which
    /// is the entire reason the scan exists: `quote_count` is ten when one
    /// account quotes ten times.
    ///
    /// # Errors
    ///
    /// [`Unreachable`] from any of the three. All or nothing on purpose -- a
    /// partial scan would produce a verified score that is missing a category,
    /// and a reply whose reposters were read but whose likers were not would
    /// rank below one that was read completely. The caller falls back to the
    /// raw score for the week rather than ranking on half a measurement.
    pub fn engagers(&self, reply_id: &str) -> Result<Engagers, Unreachable> {
        let [reposts, likes, quotes] = self.engager_urls(reply_id);
        Ok(Engagers {
            reposted: parse_accounts(&self.get(&reposts)?)?,
            liked: parse_accounts(&self.get(&likes)?)?,
            quoted: parse_quote_authors(&self.get(&quotes)?)?,
        })
    }
}

fn digits_joined(ids: &[String]) -> String {
    ids.iter()
        .map(|id| id.chars().filter(char::is_ascii_digit).collect::<String>())
        .filter(|id| !id.is_empty())
        .collect::<Vec<_>>()
        .join(",")
}

/// Reads a tweet-lookup response into metrics by id.
///
/// Missing metrics are a missing entry, not zeroes: a reply the platform
/// returned without `public_metrics` is one it would not let us score.
///
/// # Errors
///
/// [`Unreachable::Unreadable`] when the body is not JSON or reports an error
/// with no data.
pub fn parse_metrics(body: &str) -> Result<BTreeMap<String, Metrics>, Unreachable> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| Unreachable::Unreadable(format!("not json: {e}")))?;
    let mut out = BTreeMap::new();
    let Some(items) = value.get("data").and_then(serde_json::Value::as_array) else {
        if let Some(errors) = value.get("errors") {
            return Err(Unreachable::Unreadable(format!("errors: {errors}")));
        }
        return Ok(out);
    };
    for item in items {
        let (Some(id), Some(pm)) = (
            item.get("id").and_then(serde_json::Value::as_str),
            item.get("public_metrics"),
        ) else {
            continue;
        };
        let n = |key: &str| pm.get(key).and_then(serde_json::Value::as_u64);
        let (Some(reposts), Some(quotes), Some(likes), Some(replies)) = (
            n("retweet_count"),
            n("quote_count"),
            n("like_count"),
            n("reply_count"),
        ) else {
            continue;
        };
        out.insert(
            id.to_owned(),
            Metrics {
                reposts,
                quotes,
                likes,
                replies,
                // The platform reports counts, never who. The engager scan is
                // a separate set of reads at week close, and until it runs
                // this entry is unscanned rather than scanned-and-empty.
                verified: None,
            },
        );
    }
    Ok(out)
}

/// The authors of quote posts, from a `quote_tweets` response.
///
/// **The authors, not the posts.** Ten quotes from one account are one entry
/// here, and that collapse is the fix S16 describes: `quote_count` counts
/// posts, and posts are unlimited per account.
///
/// The users arrive under `includes.users` because the `data` array holds
/// posts; `expansions=author_id` is what puts them there. A response with
/// quotes but no expansion is **not** zero quoters -- it is a request that
/// asked for the wrong thing, and it fails rather than reporting nobody.
///
/// # Errors
///
/// [`Unreachable::Unreadable`] when the body is not JSON, when the platform
/// returned errors, or when there are quote posts and no authors to attribute
/// them to.
pub fn parse_quote_authors(body: &str) -> Result<BTreeMap<String, Account>, Unreachable> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| Unreachable::Unreadable(format!("not json: {e}")))?;
    if let Some(errors) = value.get("errors") {
        return Err(Unreachable::Unreadable(format!("errors: {errors}")));
    }
    let posts = value
        .get("data")
        .and_then(serde_json::Value::as_array)
        .map_or(0, Vec::len);
    let users = value.get("includes").and_then(|i| i.get("users"));
    match users {
        Some(users) => parse_accounts(&serde_json::json!({ "data": users }).to_string()),
        // No quotes at all is genuinely nobody, and the platform omits
        // `includes` entirely in that case.
        None if posts == 0 => Ok(BTreeMap::new()),
        None => Err(Unreachable::Unreadable(format!(
            "{posts} quote post(s) and no includes.users to attribute them to"
        ))),
    }
}

/// Reads a user-lookup response into creation times by id.
///
/// # Errors
///
/// [`Unreachable::Unreadable`] when the body is not JSON or reports an error
/// with no data.
pub fn parse_accounts(body: &str) -> Result<BTreeMap<String, Account>, Unreachable> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| Unreachable::Unreadable(format!("not json: {e}")))?;
    let mut out = BTreeMap::new();
    let Some(items) = value.get("data").and_then(serde_json::Value::as_array) else {
        if let Some(errors) = value.get("errors") {
            return Err(Unreachable::Unreadable(format!("errors: {errors}")));
        }
        return Ok(out);
    };
    for item in items {
        let (Some(id), Some(created)) = (
            item.get("id").and_then(serde_json::Value::as_str),
            item.get("created_at")
                .and_then(serde_json::Value::as_str)
                .and_then(radar_types::civil::seconds_from_timestamp),
        ) else {
            continue;
        };
        out.insert(
            id.to_owned(),
            Account {
                created_at: created,
                // Absent rather than empty when the platform does not return
                // one. A handle is used to build a link, and an empty string
                // would build a link to the platform's own front page.
                username: item
                    .get("username")
                    .and_then(serde_json::Value::as_str)
                    .filter(|u| !u.is_empty())
                    .map(str::to_owned),
            },
        );
    }
    Ok(out)
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

    fn post(&self, text: &str) -> Result<String, Undeliverable> {
        let url = format!("{}/2/tweets", self.base);
        let body = self
            .post(&url, &post_body(text))
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
        signals: None,
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
    fn an_account_without_a_usable_handle_has_none_rather_than_an_empty_one() {
        // The site builds a link out of this. An empty string would build
        // `https://x.com/` -- a link to the platform's front page, rendered as
        // though it were the entrant's profile -- and an absent handle instead
        // makes the site fall back to the id-based link, which resolves.
        //
        // CI reported the `!` in the `is_empty` filter as a surviving mutant:
        // nothing exercised the empty case.
        let body = r#"{"data":[
            {"id":"1","created_at":"2020-01-01T00:00:00.000Z","username":"real"},
            {"id":"2","created_at":"2020-01-01T00:00:00.000Z","username":""},
            {"id":"3","created_at":"2020-01-01T00:00:00.000Z"}
        ]}"#;
        let accounts = parse_accounts(body).expect("valid");
        assert_eq!(
            accounts["1"].username.as_deref(),
            Some("real"),
            "a real handle survives"
        );
        assert_eq!(
            accounts["2"].username, None,
            "an empty handle is absent, not empty -- deleting the `!` fails here"
        );
        assert_eq!(accounts["3"].username, None, "a missing handle is absent");
        // And the age still reads on all three: the handle is a passenger on
        // this call and must not be able to fail the thing the call is for.
        assert_eq!(accounts.len(), 3);
        for id in ["1", "2", "3"] {
            assert_eq!(accounts[id].created_at, 1_577_836_800, "{id}");
        }
    }

    #[test]
    fn the_lookup_urls_carry_only_digit_ids_and_the_operator_is_the_configured_user() {
        // CI's mutants: `user_id` replaced by "" or "xyzzy" -- the contest's
        // operator becomes nobody, and the account could win its own contest;
        // and the `!` dropped in `digits_joined`, which keeps an id with no
        // digits as an empty entry and puts a stray comma in the query.
        let x = X::at("https://api.test", "bearer", "u42");
        assert_eq!(x.user_id(), "u42");
        let ids = vec!["123".to_owned(), "abc".to_owned(), "4x5".to_owned()];
        assert_eq!(
            x.metrics_url(&ids),
            "https://api.test/2/tweets?ids=123,45&tweet.fields=public_metrics"
        );
        assert_eq!(
            x.accounts_url(&ids),
            "https://api.test/2/users?ids=123,45&user.fields=created_at,username"
        );
    }

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

    /// A credential that signs. The values are arbitrary; only their effect on
    /// the signature matters, and `oauth.rs` is where that is checked.
    fn credentials() -> crate::oauth::Credentials {
        crate::oauth::Credentials {
            consumer_key: "ck".to_owned(),
            consumer_secret: "cs".to_owned(),
            token: "tok".to_owned(),
            token_secret: "ts".to_owned(),
        }
    }

    #[test]
    fn a_client_with_no_oauth_credential_refuses_to_post_rather_than_being_refused() {
        // The half-configured instance: a bearer, so it reads, and no signing
        // credential, so it cannot speak. Saying so locally is better than
        // spending a call to be told -- and the message names the four
        // variables, because "401" does not.
        let x = X::at("https://example.test", "secret-token", "u42");
        assert!(!x.can_post());
        // And the other side of it. Asserting only the `false` case leaves
        // `can_post` replaceable with a constant `false`, which CI found: the
        // daemon would then report "no signing credential" on a fully
        // configured instance and never post, for ever, quietly.
        assert!(
            X::signing("https://example.test", "secret-token", "u42", credentials()).can_post()
        );
        let err = x.reply("m1", "text").expect_err("must refuse");
        let rendered = err.to_string();
        assert!(rendered.contains("cannot post"), "{rendered}");
        assert!(rendered.contains("RADAR_X_API_KEY"), "{rendered}");
    }

    #[test]
    fn a_reply_posts_the_body_to_the_tweets_endpoint_and_returns_the_id() {
        let (base, seen) = serve_once("201 Created", r#"{"data":{"id":"posted-7"}}"#);
        let x = X::signing(base, "secret-token", "u42", credentials());

        let id = x
            .reply("m1", "the round trip here is about 30%")
            .expect("posted");
        assert_eq!(id, "posted-7");

        let request = seen.recv().expect("the server saw a request");
        assert!(request.starts_with("POST /2/tweets"), "{request}");
        // **OAuth, never the bearer.** The platform refuses an app-only token on
        // this endpoint, so a request carrying one reads correctly here and is
        // refused there -- which is exactly what shipped until 2026-09-05 and
        // would have surfaced on the first real reply.
        let lower = request.to_lowercase();
        assert!(
            lower.contains("authorization: oauth "),
            "the post must be signed, not bearered: {request}"
        );
        assert!(
            !lower.contains("bearer secret-token"),
            "the bearer must not reach this endpoint: {request}"
        );
        for field in [
            "oauth_consumer_key",
            "oauth_nonce",
            "oauth_signature",
            "oauth_signature_method",
            "oauth_timestamp",
            "oauth_token",
            "oauth_version",
        ] {
            assert!(lower.contains(field), "missing {field}: {request}");
        }
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
