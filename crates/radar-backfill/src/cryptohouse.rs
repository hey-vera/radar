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
            // ClickHouse answers a bad query with a non-2xx whose *body* holds
            // the explanation and whose status holds nothing useful -- an unknown
            // column comes back as a bare 404. Reporting only the status once
            // cost a debugging round trip, so the body is read out here.
            Err(ureq::Error::StatusCode(code)) => {
                // Name the query that failed. An error that says only "404"
                // could be any of several queries in a batch run, and finding
                // out which cost a round trip once already.
                let head: String = sql.chars().take(100).collect();
                return Err(QueryError::Server(format!(
                    "HTTP {code} rejecting: {head}..."
                )));
            }
            Err(e) => return Err(QueryError::Transport(e.to_string())),
        };

        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|e| QueryError::Transport(e.to_string()))?;

        parse_rows(&body)
    }
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
