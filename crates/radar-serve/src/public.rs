// SPDX-License-Identifier: Apache-2.0
//! The three documents the public site reads.
//!
//! Design 0008 section 4. Each handler reads a **published file** and never
//! the store: a viral link must not reach the 3.2-second Parquet scan on the
//! two-core box. Each document carries the moment it was measured, and each
//! refuses rather than guesses when its file is absent -- rule 9 -- so the site
//! shows its committed fixture and says it is stale, which is older but true.
//!
//! # Paths from the environment, and an `_in` twin for every handler
//!
//! The same shape as `analyst_replies`: the route reads the environment, and a
//! function taking the paths does the work so the tests never touch a
//! process-wide variable that parallel tests would fight over.
//!
//! # Nothing is served cross-origin by default
//!
//! The site lives on another origin, so a browser needs
//! `Access-Control-Allow-Origin` to read these. It is set only when
//! `RADAR_SITE_ORIGIN` names that origin, and to that origin alone -- never
//! `*`. Unset means the header is absent and a browser elsewhere refuses the
//! response, which is rule 8 applied to who may read: the safe direction, and
//! the site's fallback makes it a visible one rather than a blank page.
//!
//! # Cached at the edge, briefly
//!
//! `Cache-Control: public, max-age=60`, so Cloudflare answers a spike itself.
//! Sixty seconds is the pool page's own refresh, and it is short enough that a
//! week closing is on the site within a minute.

use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use radar_analyst::log::Entry;
use radar_contest::ledger::Vault;
use radar_contest::{Record, Week};
use radar_roast::baserates::BaseRates;
use radar_roast::creator::Summary;
use radar_types::civil::{date_from_days, timestamp_from_seconds};
use serde_json::{Value, json};

/// Where the published files are, and who may read the answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    /// The contest's week records and the vault reading.
    pub contest_dir: String,
    /// The analyst's reply log.
    pub analyst_dir: String,
    /// The base-rate snapshot.
    pub base_rates: String,
    /// The population summary the creator-index job publishes.
    pub summary: String,
    /// The one origin allowed to read these from a browser, or none.
    pub site_origin: Option<String>,
}

impl Paths {
    /// From the environment, with the repository's default locations.
    ///
    /// Takes a getter, so the rule can be tested without setting process-wide
    /// variables. The defaults are the paths the other binaries write to, so
    /// an instance with nothing configured reads what the timers produce.
    pub fn from_vars(get: &impl Fn(&str) -> Option<String>) -> Self {
        let or = |key: &str, default: &str| get(key).unwrap_or_else(|| default.to_owned());
        Self {
            contest_dir: or("RADAR_CONTEST_DIR", "data/contest"),
            analyst_dir: or("RADAR_ANALYST_DIR", "data/analyst"),
            base_rates: or("RADAR_BASE_RATES", radar_roast::baserates::DEFAULT_PATH),
            summary: or("RADAR_POPULATION", radar_roast::creator::SUMMARY_PATH),
            site_origin: get("RADAR_SITE_ORIGIN")
                .map(|o| o.trim().to_owned())
                .filter(|o| !o.is_empty()),
        }
    }

    fn from_env() -> Self {
        Self::from_vars(&|k| std::env::var(k).ok())
    }
}

/// `GET /v1/public/stats`.
pub async fn stats() -> Response {
    let paths = Paths::from_env();
    match stats_in(&paths) {
        Some(doc) => respond(&paths, StatusCode::OK, doc),
        // Not an error and not zeroes. A figure that is not on disk cannot be
        // stated, and the site has a dated fixture for exactly this.
        None => respond(
            &paths,
            StatusCode::NOT_FOUND,
            json!({ "error": "not measured yet on this instance" }),
        ),
    }
}

/// `GET /v1/public/leaderboard`.
pub async fn leaderboard() -> Response {
    let paths = Paths::from_env();
    let doc = leaderboard_in(&paths, radar_analyst::daemon::now());
    respond(&paths, StatusCode::OK, doc)
}

/// `GET /v1/public/pool`.
pub async fn pool() -> Response {
    let paths = Paths::from_env();
    let doc = pool_in(&paths);
    respond(&paths, StatusCode::OK, doc)
}

/// The population figures, in the site's shape, or `None` when a figure it
/// needs is not on disk.
///
/// Two files: the population summary the creator-index job writes beside the
/// index, and the base-rate snapshot. The summary exists so that this does not
/// parse 116,000 creator records per request to read five totals.
#[must_use]
pub fn stats_in(paths: &Paths) -> Option<Value> {
    let summary = Summary::read(&paths.summary).ok()?;
    let rates = BaseRates::load(&paths.base_rates).ok()?;
    // Research 0011's figure travels in the snapshot with its own date. A
    // snapshot without it is an older one, and the page must not fill the gap
    // with a remembered number.
    let aftermath = rates.aftermath.as_ref()?;
    let population = summary.population;
    Some(json!({
        "measured_at": timestamp_from_seconds(summary.built_at),
        "watermark_slot": summary.watermark_slot,
        "watched": {
            "launches": population.launches,
            "creators": summary.creators,
            "measured": population.measured,
            "organic": population.organic,
            "instant": population.instant,
            "stillborn": population.stillborn,
        },
        "bands": {
            "measured_on": rates.measured_on,
            "launches": rates.launches,
            "base_rate_instant": rates.base_rate_instant,
            "rows": rates.bands.iter().map(|b| json!({
                "name": b.name,
                "lo": b.lo,
                "hi": b.hi,
                "share_of_launches": b.fires_on,
                "p_instant": b.p_instant,
                "x_base_instant": b.x_base_instant,
            })).collect::<Vec<_>>(),
        },
        "cost": {
            "band": "$20-$200",
            "round_trip_bps": rates.round_trip_bar,
        },
        "aftermath": {
            "measured_on": aftermath.measured_on,
            "organic_median_bps": aftermath.organic_median_bps,
        },
    }))
}

/// The leaderboard, in the site's shape.
///
/// The latest **closed** week's record when one exists, ranked and scored.
/// Before any week has closed, the current week from the reply log with every
/// score `null` -- the honest partial page design 0008 section 11 describes,
/// rather than an empty one. Before the bot has answered anyone at all, the
/// honest empty: `week` is `null` and the page says no week has run.
#[must_use]
pub fn leaderboard_in(paths: &Paths, now: u64) -> Value {
    let entries = replies(paths);

    if let Some(record) = latest_record(&paths.contest_dir) {
        let (answered, published) = counts_in(&entries, record.week);
        let ranked: Vec<Value> = record
            .ranking
            .ranked
            .iter()
            .enumerate()
            .map(|(i, r)| {
                json!({
                    "rank": i + 1,
                    "summoner": r.entry.summoner,
                    "mint": r.entry.mint,
                    "reply_url": reply_url(&r.entry.reply_id),
                    "score": r.score,
                })
            })
            .collect();
        return json!({
            "week": monday_of(record.week),
            "measured_at": timestamp_from_seconds(record.closed_at),
            "entries": ranked,
            "answered": answered,
            "published": published,
        });
    }

    let week = Week::of(now);
    let mut this_week: Vec<&Entry> = entries
        .iter()
        .filter(|e| week.contains(e.at) && e.mint.is_some())
        .collect();
    if this_week.is_empty() {
        return json!({
            "week": Value::Null,
            "measured_at": Value::Null,
            "entries": [],
            "answered": 0,
            "published": 0,
        });
    }
    this_week.sort_by_key(|e| e.at);
    let (answered, published) = counts_in(&entries, week);
    let unscored: Vec<Value> = this_week
        .iter()
        .enumerate()
        .map(|(i, e)| {
            json!({
                "rank": i + 1,
                "summoner": e.summoner,
                "mint": e.mint,
                "reply_url": e.reply_id.as_deref().map(reply_url),
                // Engagement is read at week close and not before. `null`,
                // never `0`: a zero would say nobody engaged.
                "score": Value::Null,
            })
        })
        .collect();
    json!({
        "week": monday_of(week),
        "measured_at": timestamp_from_seconds(now),
        "entries": unscored,
        "answered": answered,
        "published": published,
    })
}

/// The prize pool, in the site's shape.
///
/// `vault` and `lamports` are `null` until the vault reading exists, which is
/// until a token exists. That is a different state from a balance of zero and
/// the page renders it differently.
#[must_use]
pub fn pool_in(paths: &Paths) -> Value {
    let vault = std::fs::read_to_string(format!("{}/pool.json", paths.contest_dir))
        .ok()
        .and_then(|text| Vault::from_json(&text).ok());
    let mut winners: Vec<(u64, Value)> = records(&paths.contest_dir)
        .into_iter()
        .filter_map(|record| {
            let payout = record.payout.as_ref()?;
            let winner = record.winner.as_ref()?;
            Some((
                record.week.0,
                json!({
                    "week": monday_of(record.week),
                    "summoner": winner.summoner,
                    "lamports": payout.lamports,
                    "signature": payout.signature,
                }),
            ))
        })
        .collect();
    // Newest first; a reader wants the last winner, not the first.
    winners.sort_by_key(|(week, _)| std::cmp::Reverse(*week));
    json!({
        "vault": vault.as_ref().map(|v| v.address.clone()),
        "lamports": vault.as_ref().map(|v| v.lamports),
        "measured_at": vault.as_ref().map(|v| timestamp_from_seconds(v.measured_at)),
        "winners": winners.into_iter().map(|(_, w)| w).collect::<Vec<_>>(),
    })
}

/// The reply log, folded to one row per reply, or nothing when there is none.
fn replies(paths: &Paths) -> Vec<Entry> {
    radar_analyst::log::latest(&format!("{}/replies.jsonl", paths.analyst_dir)).unwrap_or_default()
}

/// Replies decided in the week, and how many of them reached the platform.
fn counts_in(entries: &[Entry], week: Week) -> (usize, usize) {
    let in_week: Vec<&Entry> = entries
        .iter()
        .filter(|e| week.contains(e.at) && e.mint.is_some())
        .collect();
    let published = in_week.iter().filter(|e| e.reply_id.is_some()).count();
    (in_week.len(), published)
}

/// Every week record in the directory, in no particular order.
///
/// A file that does not parse is skipped rather than failing the page: a torn
/// write is not evidence, and the weeks either side of it still are.
fn records(dir: &str) -> Vec<Record> {
    let Ok(listing) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    listing
        .flatten()
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.ends_with(".json") && name.trim_end_matches(".json").parse::<u64>().is_ok()
        })
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .filter_map(|text| Record::from_json(&text).ok())
        .collect()
}

/// The most recently closed week's record, if any.
fn latest_record(dir: &str) -> Option<Record> {
    records(dir).into_iter().max_by_key(|r| r.week)
}

/// The Monday a week opens on, as `YYYY-MM-DD`.
fn monday_of(week: Week) -> String {
    date_from_days(i64::try_from(week.opens_at() / 86_400).unwrap_or(i64::MAX))
}

/// Where a reply lives on the platform.
///
/// The `i/web/status` form resolves without the author's handle, which the
/// ledger does not carry and should not have to.
fn reply_url(reply_id: &str) -> String {
    format!("https://x.com/i/web/status/{reply_id}")
}

/// A JSON response with the edge-cache and, when configured, the CORS header.
fn respond(paths: &Paths, status: StatusCode, doc: Value) -> Response {
    let mut response = (status, Json(doc)).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=60"),
    );
    if let Some(origin) = paths.site_origin.as_deref()
        && let Ok(value) = HeaderValue::from_str(origin)
    {
        headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, value);
        headers.insert(header::VARY, HeaderValue::from_static("Origin"));
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use radar_contest::ledger::{Claim, Payout};
    use radar_contest::score::{Entry as ContestEntry, Metrics, Ranked, Ranking};

    const SNAPSHOT: &str = include_str!("../../../docs/research/data/0024-base-rates.json");

    /// Week 2958 opens on Monday 2026-09-07.
    const WEEK: Week = Week(2958);

    fn paths_in(dir: &std::path::Path) -> Paths {
        Paths {
            contest_dir: dir.join("contest").to_string_lossy().into_owned(),
            analyst_dir: dir.join("analyst").to_string_lossy().into_owned(),
            base_rates: dir.join("rates.json").to_string_lossy().into_owned(),
            summary: dir.join("population.json").to_string_lossy().into_owned(),
            site_origin: None,
        }
    }

    fn a_summary() -> Summary {
        Summary {
            built_at: 20_700 * 86_400 + 23 * 3600 + 55 * 60,
            watermark_slot: 444_374_676,
            creators: 116_752,
            population: radar_roast::creator::Population {
                launches: 508_814,
                measured: 506_991,
                organic: 8_999,
                instant: 5_230,
                stillborn: 116_427,
            },
        }
    }

    fn a_reply(at: u64, reply_id: Option<&str>) -> Entry {
        Entry {
            at,
            mention_id: format!("m{at}"),
            summoner: "alice".to_owned(),
            mint: Some("So11111111111111111111111111111111111111112".to_owned()),
            read_at_slot: Some(1),
            fact_sheet: String::new(),
            reply: "measured".to_owned(),
            fellback: None,
            reply_id: reply_id.map(str::to_owned),
        }
    }

    fn a_record(week: Week) -> Record {
        Record::close(
            week,
            Ranking {
                ranked: vec![Ranked {
                    entry: ContestEntry {
                        reply_id: "r1".to_owned(),
                        summoner: "alice".to_owned(),
                        mint: "M".to_owned(),
                        at: week.opens_at() + 10,
                        metrics: Metrics {
                            likes: 4,
                            ..Metrics::default()
                        },
                    },
                    score: 4,
                }],
                excluded: Vec::new(),
            },
        )
    }

    #[test]
    fn the_paths_default_to_where_the_timers_write_and_the_origin_is_optional() {
        let none = Paths::from_vars(&|_| None);
        assert_eq!(none.contest_dir, "data/contest");
        assert_eq!(none.analyst_dir, "data/analyst");
        assert_eq!(none.base_rates, radar_roast::baserates::DEFAULT_PATH);
        assert_eq!(none.summary, radar_roast::creator::SUMMARY_PATH);
        assert_eq!(none.site_origin, None);

        let set = Paths::from_vars(&|k| match k {
            "RADAR_CONTEST_DIR" => Some("/var/lib/radar/contest".to_owned()),
            "RADAR_SITE_ORIGIN" => Some("  https://cabalhunter.org ".to_owned()),
            _ => None,
        });
        assert_eq!(set.contest_dir, "/var/lib/radar/contest");
        assert_eq!(set.site_origin.as_deref(), Some("https://cabalhunter.org"));
        // Blank is unset, not an empty origin that would produce an empty
        // header.
        let blank = Paths::from_vars(&|k| (k == "RADAR_SITE_ORIGIN").then(|| "  ".to_owned()));
        assert_eq!(blank.site_origin, None);
    }

    #[test]
    fn stats_needs_both_files_and_refuses_rather_than_guessing_without_them() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let paths = paths_in(dir.path());

        // Nothing on disk: nothing stated.
        assert_eq!(stats_in(&paths), None);

        // The snapshot alone is not enough: the counts are not in it.
        std::fs::write(&paths.base_rates, SNAPSHOT).expect("write");
        assert_eq!(stats_in(&paths), None);

        a_summary().write(&paths.summary).expect("write");
        let doc = stats_in(&paths).expect("both files present");
        assert_eq!(doc["measured_at"], "2026-09-04T23:55:00Z");
        assert_eq!(doc["watermark_slot"], 444_374_676);
        assert_eq!(doc["watched"]["launches"], 508_814);
        assert_eq!(doc["watched"]["creators"], 116_752);
        assert_eq!(doc["watched"]["measured"], 506_991);
        assert_eq!(doc["bands"]["measured_on"], "2026-09-03");
        assert_eq!(doc["bands"]["launches"], 17_497);
        let rows = doc["bands"]["rows"].as_array().expect("rows");
        assert!(rows.iter().any(|r| r["name"] == "one to three"
            && (r["share_of_launches"].as_f64().expect("share") - 0.705).abs() < 1e-9));
        assert_eq!(doc["cost"]["round_trip_bps"], 456.0);
        assert_eq!(doc["aftermath"]["organic_median_bps"], -3228.0);
    }

    #[test]
    fn a_snapshot_without_the_aftermath_figure_is_refused_rather_than_padded() {
        // Re-apply the bug by making `aftermath` default to a remembered
        // number in `stats_in`, and this passes a document the disk did not
        // carry.
        let dir = tempfile::tempdir().expect("a temp dir");
        let paths = paths_in(dir.path());
        a_summary().write(&paths.summary).expect("write");
        let mut snapshot: Value = serde_json::from_str(SNAPSHOT).expect("json");
        snapshot
            .as_object_mut()
            .expect("object")
            .remove("aftermath");
        std::fs::write(&paths.base_rates, snapshot.to_string()).expect("write");
        assert_eq!(stats_in(&paths), None);
    }

    #[test]
    fn before_the_bot_has_answered_anyone_the_leaderboard_is_the_honest_empty() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let doc = leaderboard_in(&paths_in(dir.path()), WEEK.opens_at() + 100);
        assert_eq!(doc["week"], Value::Null);
        assert_eq!(doc["measured_at"], Value::Null);
        assert_eq!(doc["entries"].as_array().expect("entries").len(), 0);
        assert_eq!(doc["answered"], 0);
        assert_eq!(doc["published"], 0);
    }

    #[test]
    fn an_open_week_lists_its_summoners_with_no_score_rather_than_zero() {
        // Design 0008 section 11: the partial page, not the empty one. The
        // score is `null` because engagement is read at week close; a `0`
        // would say nobody engaged. Re-apply the bug by writing `0` and the
        // assertion on `Value::Null` fails.
        let dir = tempfile::tempdir().expect("a temp dir");
        let paths = paths_in(dir.path());
        std::fs::create_dir_all(&paths.analyst_dir).expect("mkdir");
        let log = format!("{}/replies.jsonl", paths.analyst_dir);
        let now = WEEK.opens_at() + 3_600;
        radar_analyst::log::append(&log, &a_reply(now - 60, Some("r1"))).expect("append");
        radar_analyst::log::append(&log, &a_reply(now - 30, None)).expect("append");
        // Last week's reply does not belong to this week's page.
        radar_analyst::log::append(&log, &a_reply(WEEK.opens_at() - 60, Some("r0")))
            .expect("append");

        let doc = leaderboard_in(&paths, now);
        assert_eq!(doc["week"], "2026-09-07");
        let entries = doc["entries"].as_array().expect("entries");
        assert_eq!(entries.len(), 2, "{doc}");
        assert_eq!(entries[0]["rank"], 1);
        assert_eq!(entries[0]["score"], Value::Null);
        assert_eq!(entries[0]["reply_url"], "https://x.com/i/web/status/r1");
        assert_eq!(
            entries[1]["reply_url"],
            Value::Null,
            "an unpublished reply has no url"
        );
        assert_eq!(doc["answered"], 2);
        assert_eq!(doc["published"], 1);
    }

    #[test]
    fn a_closed_week_is_served_from_its_record_ranked_and_scored() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let paths = paths_in(dir.path());
        std::fs::create_dir_all(&paths.contest_dir).expect("mkdir");
        // Two records; the newer one is the page. A torn file beside them is
        // skipped rather than failing the page.
        for week in [Week(2957), WEEK] {
            let record = a_record(week);
            std::fs::write(
                format!("{}/{}.json", paths.contest_dir, week.0),
                record.to_json().expect("json"),
            )
            .expect("write");
        }
        std::fs::write(format!("{}/2959.json", paths.contest_dir), "{ torn").expect("write");

        let doc = leaderboard_in(&paths, WEEK.closes_at() + 10);
        assert_eq!(doc["week"], "2026-09-07");
        assert_eq!(doc["measured_at"], timestamp_from_seconds(WEEK.closes_at()));
        let entries = doc["entries"].as_array().expect("entries");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["score"], 4);
        assert_eq!(entries[0]["summoner"], "alice");
        assert_eq!(entries[0]["reply_url"], "https://x.com/i/web/status/r1");
    }

    #[test]
    fn the_pool_is_null_until_a_token_exists_and_never_zero() {
        // The assertion the site's own test makes, from the other side: the
        // document must not carry a zero balance when there is no vault.
        let dir = tempfile::tempdir().expect("a temp dir");
        let paths = paths_in(dir.path());
        let doc = pool_in(&paths);
        assert_eq!(doc["vault"], Value::Null);
        assert_eq!(doc["lamports"], Value::Null);
        assert_eq!(doc["measured_at"], Value::Null);
        assert_eq!(doc["winners"].as_array().expect("winners").len(), 0);
        assert!(!doc.to_string().contains("\"lamports\":0"), "{doc}");
    }

    #[test]
    fn a_vault_reading_and_past_payouts_reach_the_pool_newest_first() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let paths = paths_in(dir.path());
        std::fs::create_dir_all(&paths.contest_dir).expect("mkdir");
        let vault = Vault {
            address: "VAULT".to_owned(),
            lamports: 123_456_789,
            measured_at: WEEK.closes_at(),
        };
        std::fs::write(
            format!("{}/pool.json", paths.contest_dir),
            vault.to_json().expect("json"),
        )
        .expect("write");
        for week in [Week(2957), WEEK] {
            let mut record = a_record(week);
            record.claim = Some(Claim {
                address: "ADDR".to_owned(),
                reply_id: "c".to_owned(),
                at: week.closes_at() + 1,
            });
            record.payout = Some(Payout {
                recipient: "ADDR".to_owned(),
                lamports: 1_000 + week.0,
                signature: format!("SIG{}", week.0),
                at: week.closes_at() + 2,
            });
            std::fs::write(
                format!("{}/{}.json", paths.contest_dir, week.0),
                record.to_json().expect("json"),
            )
            .expect("write");
        }
        // An unpaid week is not a winner on the pool page.
        std::fs::write(
            format!("{}/2959.json", paths.contest_dir),
            a_record(Week(2959)).to_json().expect("json"),
        )
        .expect("write");

        let doc = pool_in(&paths);
        assert_eq!(doc["vault"], "VAULT");
        assert_eq!(doc["lamports"], 123_456_789);
        assert_eq!(doc["measured_at"], timestamp_from_seconds(WEEK.closes_at()));
        let winners = doc["winners"].as_array().expect("winners");
        assert_eq!(winners.len(), 2);
        assert_eq!(winners[0]["week"], "2026-09-07");
        assert_eq!(winners[0]["signature"], "SIG2958");
        assert_eq!(winners[1]["week"], "2026-08-31");
    }

    #[test]
    fn the_cors_header_names_the_one_origin_and_is_absent_when_none_is_configured() {
        let mut paths = paths_in(std::path::Path::new("."));
        let closed = respond(&paths, StatusCode::OK, json!({}));
        assert!(
            closed
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .is_none()
        );
        assert_eq!(
            closed
                .headers()
                .get(header::CACHE_CONTROL)
                .expect("cached")
                .to_str()
                .expect("ascii"),
            "public, max-age=60"
        );

        paths.site_origin = Some("https://cabalhunter.org".to_owned());
        let open = respond(&paths, StatusCode::OK, json!({}));
        assert_eq!(
            open.headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .expect("the header")
                .to_str()
                .expect("ascii"),
            "https://cabalhunter.org"
        );
        assert_eq!(
            open.headers()
                .get(header::VARY)
                .expect("vary")
                .to_str()
                .expect("ascii"),
            "Origin"
        );
    }
}
