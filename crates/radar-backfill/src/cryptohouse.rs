// SPDX-License-Identifier: Apache-2.0
//! A small read-only client for CryptoHouse.
//!
//! CryptoHouse is a free public ClickHouse holding the whole Solana chain
//! (ADR 0002). Radar uses it for **bulk extraction into its own store, once** —
//! never on a hot path and never as a live provider lane. After extraction the
//! data is ours and the service going away costs nothing.
//!
//! Being a guest here is a real constraint. The credentials ship in the public
//! web client, so this is a public read endpoint, but that is an implicit
//! invitation rather than an explicit one. Queries are windowed so each one stays
//! well inside the server's sixty-second cap, and the extractor paces itself
//! between them.

use std::time::Duration;

use serde::de::DeserializeOwned;

/// The public endpoint.
pub const ENDPOINT: &str = "https://crypto-clickhouse.clickhouse.com/";
/// The read-only user the public web client uses.
pub const USER: &str = "crypto";

/// A CryptoHouse query failed.
#[derive(Debug, thiserror::Error)]
pub enum QueryError {
    /// The request did not complete.
    #[error("cryptohouse transport: {0}")]
    Transport(String),
    /// The server rejected or could not finish the query.
    ///
    /// Two of these mean "the window was too wide" rather than "something is
    /// broken", and the extractor narrows and retries: a sixty-second execution
    /// timeout, and the thousand-row result cap. Both are fixed on the public
    /// endpoint and cannot be raised — the user is `readonly=1`.
    #[error("cryptohouse: {0}")]
    Server(String),
    /// A row did not match the expected shape.
    #[error("cryptohouse row: {0}")]
    Row(#[from] serde_json::Error),
}

impl QueryError {
    /// Whether narrowing the window and retrying is worth trying.
    ///
    /// True for the two limits a wide window runs into. Anything else — a bad
    /// identifier, a transport failure — will fail identically on a narrower
    /// window, and retrying would only hammer a public endpoint we are a guest on.
    #[must_use]
    pub fn should_narrow(&self) -> bool {
        matches!(
            self,
            Self::Server(m)
                if m.contains("TIMEOUT_EXCEEDED") || m.contains("TOO_MANY_ROWS_OR_BYTES")
        )
    }
}

/// A read-only CryptoHouse client.
pub struct Client {
    endpoint: String,
    agent: ureq::Agent,
}

impl Default for Client {
    fn default() -> Self {
        Self::new(ENDPOINT)
    }
}

impl Client {
    /// A client for the given endpoint.
    #[must_use]
    pub fn new(endpoint: impl Into<String>) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(180)))
            // **Do not turn a non-2xx into an error before its body is read.**
            //
            // ClickHouse puts the whole explanation in the body and nothing
            // useful in the status: the thousand-row cap arrives as a bare
            // HTTP 500 whose body names `TOO_MANY_ROWS_OR_BYTES`. With ureq's
            // default, that became `Error::StatusCode(500)` with the body
            // dropped, so [`QueryError::should_narrow`] — which looks for
            // exactly that text — returned false, and a window that needed
            // halving failed outright instead.
            //
            // That is why the `trades` table was empty. `--scope trades` over
            // any window wide enough to be worth running returns far more than
            // a thousand rows, every attempt hit the cap, and every attempt
            // gave up at the first response rather than narrowing. The
            // narrowing code was correct and never ran. Found 2026-09-11.
            .http_status_as_error(false)
            .build();
        Self {
            endpoint: endpoint.into(),
            agent: config.into(),
        }
    }

    /// Runs a query and deserialises each row.
    ///
    /// Rows come back as `JSONEachRow`, one JSON object per line, which streams
    /// without the server buffering a whole result set.
    ///
    /// # Errors
    ///
    /// Returns [`QueryError`] if the request fails, the server rejects the query,
    /// or a row does not deserialise.
    pub fn query<T: DeserializeOwned>(&self, sql: &str) -> Result<Vec<T>, QueryError> {
        // POST with the SQL in the body rather than GET with it in the URL.
        // An outcome batch names four hundred mints, which is roughly 18 KB of
        // query -- well past the URL length most proxies accept, and the failure
        // arrives as a bare 404 that looks like a missing endpoint rather than an
        // oversized request.
        let mut response = match self
            .agent
            .post(&self.endpoint)
            .query("user", USER)
            .content_type("text/plain; charset=utf-8")
            .send(format!("{sql} FORMAT JSONEachRow"))
        {
            Ok(r) => r,
            Err(e) => return Err(QueryError::Transport(e.to_string())),
        };

        // The status is read but never used to decide the outcome on its own:
        // `http_status_as_error(false)` is set precisely so the body arrives
        // whatever the status, because that is where ClickHouse says what
        // happened. It is carried into the message only so an operator can
        // see it.
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|e| QueryError::Transport(e.to_string()))?;

        if is_error_status(status) {
            return Err(server_error(status, &body, sql));
        }

        parse_rows(&body)
    }
}

/// Whether an HTTP status means the body is an explanation rather than rows.
///
/// Pulled out beside [`server_error`] and for the same reason: inside
/// [`Client::query`] the only way to exercise the boundary was a live request,
/// so nothing checked which side of 400 each status fell on. A `<` here rather
/// than a `>=` would feed ClickHouse's error text to `parse_rows` and report
/// the failure as a malformed row -- a fact about this build, for something
/// that is a fact about the query.
#[must_use]
const fn is_error_status(status: u16) -> bool {
    status >= 400
}

/// Builds the error for a non-2xx response, from its status and its body.
///
/// **Pure, and separate from [`Client::query`], because the bug it fixes is
/// only visible in what this returns.** The thousand-row cap arrives as a bare
/// HTTP 500 whose body names `TOO_MANY_ROWS_OR_BYTES`, and
/// [`QueryError::should_narrow`] decides whether to halve the window by looking
/// for exactly that text. An earlier version formatted the status and the first
/// hundred characters of the *query* and discarded the body — so the marker was
/// never present, `should_narrow` returned false, and a window that needed
/// halving failed outright. Every `--scope trades` run hit this, which is why
/// the `trades` table was empty. Found 2026-09-11.
///
/// Living inside `query` meant the only way to exercise it was a live request
/// against a public endpoint. Out here it takes a status and a body, so the
/// wrong behaviour can be reapplied in a test.
///
/// The body leads and is truncated at 600 characters: both markers appear near
/// the front of a ClickHouse exception, which can otherwise carry a long stack.
/// The query's first hundred characters follow, because an error that says only
/// "404" could be any of several queries in a batch run.
#[must_use]
fn server_error(status: u16, body: &str, sql: &str) -> QueryError {
    let detail: String = body.trim().chars().take(600).collect();
    let head: String = sql.chars().take(100).collect();
    QueryError::Server(format!("HTTP {status}: {detail} -- rejecting: {head}..."))
}

/// Parses a `JSONEachRow` body, surfacing a server exception as an error.
///
/// ClickHouse reports failures as a normal-looking row containing `exception`,
/// so a parser that only looked at the HTTP status would treat a timeout as an
/// empty result — and an empty result from a backfill is a silent gap.
///
/// # Errors
///
/// Returns [`QueryError::Server`] if the body carries an exception, or
/// [`QueryError::Row`] if a line does not deserialise.
pub fn parse_rows<T: DeserializeOwned>(body: &str) -> Result<Vec<T>, QueryError> {
    let mut out = Vec::new();
    for line in body.lines().filter(|l| !l.trim().is_empty()) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line)
            && let Some(e) = v.get("exception").and_then(serde_json::Value::as_str)
        {
            return Err(QueryError::Server(e.to_owned()));
        }
        out.push(serde_json::from_str(line)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Deserialize, PartialEq, Eq, Debug)]
    struct Row {
        n: String,
    }

    #[test]
    fn rows_parse_one_per_line() {
        let rows: Vec<Row> = parse_rows("{\"n\":\"1\"}\n{\"n\":\"2\"}\n").expect("parses");
        assert_eq!(rows, vec![Row { n: "1".into() }, Row { n: "2".into() }]);
    }

    #[test]
    fn the_row_cap_narrows_the_window_like_a_timeout_does() {
        // The public endpoint caps results at a thousand rows and will not let a
        // readonly user raise it, so overflow is a routine signal to ask for less
        // rather than a failure.
        let body = "{\"exception\": \"Code: 396. DB::Exception: Limit for result exceeded,                     max rows: 1.00 thousand (TOO_MANY_ROWS_OR_BYTES)\"}";
        assert!(
            parse_rows::<Row>(body)
                .expect_err("must error")
                .should_narrow()
        );
    }

    /// 400 is the boundary, and both sides of it are pinned.
    ///
    /// ClickHouse answers a rejected query with a non-2xx whose body carries
    /// the explanation, and a success with rows. Reading 400 as success feeds
    /// the explanation to `parse_rows`, which then reports a fact about this
    /// build for something that is a fact about the query; reading 399 as a
    /// failure throws away rows that arrived.
    #[test]
    fn four_hundred_is_where_a_body_stops_being_rows() {
        assert!(!is_error_status(200));
        assert!(!is_error_status(204));
        assert!(!is_error_status(399), "399 is not an error status");
        assert!(is_error_status(400), "400 is");
        assert!(is_error_status(500));
    }

    /// The row cap as it actually arrives: a bare HTTP 500 with the marker in
    /// the body.
    ///
    /// Reapply the bug by having `server_error` ignore `body` — format the
    /// status and the query alone, as it did before 2026-09-11 — and this
    /// fails while
    /// `the_row_cap_narrows_the_window_like_a_timeout_does` still passes,
    /// because that one feeds the body in directly and never goes near a
    /// status code. That gap is the whole reason `--scope trades` never
    /// worked.
    #[test]
    fn a_row_cap_reported_as_http_500_still_narrows() {
        let body = "Code: 396. DB::Exception: Limit for result exceeded, max rows: 1.00 thousand (TOO_MANY_ROWS_OR_BYTES) (version 26.4.1.2212)";
        let err = server_error(500, body, "SELECT mint FROM solana.token_transfers");
        assert!(
            err.should_narrow(),
            "a 500 carrying TOO_MANY_ROWS_OR_BYTES must narrow, not fail: {err}"
        );
    }

    /// A timeout reported the same way, for the same reason.
    #[test]
    fn a_timeout_reported_as_http_500_still_narrows() {
        let body = "Code: 159. DB::Exception: Timeout exceeded (TIMEOUT_EXCEEDED)";
        let err = server_error(500, body, "SELECT 1");
        assert!(err.should_narrow(), "{err}");
    }

    /// And a genuine mistake still does not, because narrowing it would only
    /// hammer a public endpoint with the same broken query.
    #[test]
    fn a_bad_identifier_does_not_narrow_however_it_is_reported() {
        let body =
            "Code: 47. DB::Exception: Unknown expression identifier `mnit` (UNKNOWN_IDENTIFIER)";
        let err = server_error(404, body, "SELECT mnit FROM solana.token_transfers");
        assert!(!err.should_narrow(), "{err}");
    }

    /// The status reaches the operator, and so does which query failed.
    #[test]
    fn the_error_names_the_status_and_the_query_that_failed() {
        let err = server_error(
            500,
            "Code: 396 ...",
            "SELECT mint FROM solana.token_transfers",
        );
        let text = err.to_string();
        assert!(text.contains("500"), "{text}");
        assert!(text.contains("solana.token_transfers"), "{text}");
    }

    #[test]
    fn a_server_exception_is_an_error_rather_than_an_empty_result() {
        // ClickHouse returns failures as a normal-looking row. Treating that as
        // zero rows would write a silent gap into the store, and a gap in a
        // backfill is indistinguishable from a quiet market.
        let body =
            "{\"exception\": \"Code: 159. DB::Exception: Timeout exceeded (TIMEOUT_EXCEEDED)\"}";
        let err = parse_rows::<Row>(body).expect_err("must error");
        assert!(err.should_narrow(), "{err}");
    }

    #[test]
    fn a_query_error_is_not_retried_by_narrowing() {
        // A bad identifier fails identically on a narrower window; retrying would
        // only hammer an endpoint we are a guest on.
        let body = "{\"exception\": \"Code: 47. DB::Exception: Unknown identifier\"}";
        assert!(
            !parse_rows::<Row>(body)
                .expect_err("must error")
                .should_narrow()
        );
    }

    #[test]
    fn an_empty_body_is_zero_rows_not_an_error() {
        let rows: Vec<Row> = parse_rows("").expect("parses");
        assert!(rows.is_empty());
    }
}
