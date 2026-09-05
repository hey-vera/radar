// SPDX-License-Identifier: Apache-2.0
//! "Seven days later": the bot's own calls, aged in public.
//!
//! Design 0009 §3 L6 and §5 M4, plan 0006 item 6. Once a day, at a fixed hour,
//! the account posts what the chain did to the coins it was summoned about
//! seven days earlier: how many graduated and how, how many have not moved
//! since the reply, and how the priced ones ended against their first fill.
//! It is the record loop with a daily beat -- a fact nobody else can post,
//! and the only honest form of a "hit rate": the bot has no takes, so what
//! ages is the measurement.
//!
//! # Two halves, one file
//!
//! The analyst does not read the store (the unit says so). So the join --
//! replies from the log, outcomes from the store -- runs as a timer job in the
//! `radar` binary on the box, the way the creator index is built, and writes
//! `daily/<date>.json`. This module reads that file, renders it, checks it
//! and posts it. Rendering is pure and the file is the evidence beside the
//! post.
//!
//! # Day one has nothing to say
//!
//! No replies seven days ago is [`Rendered::NothingYet`], and nothing is
//! posted. The file still records that the job ran, so a quiet day and a job
//! that did not run are different things on disk.

use std::fmt::Write as _;

use radar_contest::Vault;
use radar_types::civil::{date_from_days, timestamp_from_seconds};
use serde::{Deserialize, Serialize};

use crate::publish::Publisher;
use crate::weekly::Post;

/// The hour, UTC, the post goes out. Printed on the site once the site has a
/// place for it (design 0009 §7).
pub const POST_HOUR_UTC: u64 = 12;

/// How a coin graduated, as the store measured it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Graduation {
    /// The curve completed within a few slots of the launch.
    Instant,
    /// The curve filled over time.
    Organic,
}

/// One coin the bot was asked about, seven days on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Row {
    /// The coin.
    pub mint: String,
    /// When the bot answered, seconds since the epoch.
    pub asked_at: u64,
    /// The published reply, when there was one.
    pub reply_id: Option<String>,
    /// How it graduated, or `None` for not (as far as the store has seen).
    pub graduation: Option<Graduation>,
    /// Whether no transfer has been seen since the slot the reply was read
    /// at. `None` when the store has no transfer slot for it at all.
    pub quiet_since_reply: Option<bool>,
    /// Held from first fill to last observed price, in basis points, when
    /// both were measured.
    pub held_bps: Option<i64>,
}

/// The day's rows, as the job wrote them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rows {
    /// The day the replies were made, `YYYY-MM-DD`.
    pub asked_on: String,
    /// When the job ran, seconds since the epoch.
    pub built_at: u64,
    /// The store's watermark the outcomes were read at.
    pub watermark_slot: u64,
    /// The coins.
    pub rows: Vec<Row>,
}

impl Rows {
    /// Reads a day's file.
    ///
    /// # Errors
    ///
    /// The I/O or parse error.
    pub fn read(path: &str) -> std::io::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        serde_json::from_str(&text).map_err(std::io::Error::other)
    }

    /// Writes a day's file, via a sibling and a rename.
    ///
    /// # Errors
    ///
    /// The I/O error.
    pub fn write(&self, path: &str) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        let tmp = format!("{path}.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)
    }
}

/// What rendering produced.
#[derive(Clone, Debug, PartialEq)]
pub enum Rendered {
    /// A post, with its authorised numbers.
    Post(Post),
    /// Nothing to say, and why.
    NothingYet(String),
}

/// The day's post, from its rows. Pure.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    reason = "counts and lamports, all far below 2^53"
)]
pub fn render(rows: &Rows, vault: Option<&Vault>) -> Rendered {
    if rows.rows.is_empty() {
        return Rendered::NothingYet(format!(
            "no replies on {}, so nothing to look back on",
            rows.asked_on
        ));
    }
    let mut authorised = Vec::new();
    authorised.extend(
        rows.asked_on
            .split('-')
            .filter_map(|p| p.parse::<f64>().ok()),
    );

    let n = rows.rows.len();
    let graduated = rows.rows.iter().filter(|r| r.graduation.is_some()).count();
    let instant = rows
        .rows
        .iter()
        .filter(|r| r.graduation == Some(Graduation::Instant))
        .count();
    let quiet = rows
        .rows
        .iter()
        .filter(|r| r.quiet_since_reply == Some(true))
        .count();
    let mut held: Vec<i64> = rows.rows.iter().filter_map(|r| r.held_bps).collect();
    held.sort_unstable();
    for v in [n, graduated, instant, quiet, held.len()] {
        authorised.push(v as f64);
    }

    let mut text = format!(
        "Seven days later: {n} {} we were asked about on {}. {graduated} graduated, {instant} of \
         them inside the launch block; {quiet} {} had no transfer since we answered.",
        if n == 1 { "coin" } else { "coins" },
        rows.asked_on,
        if quiet == 1 { "has" } else { "have" },
    );
    if !held.is_empty() {
        let median = held[held.len() / 2];
        // The scanner reads digits, so a negative figure authorises its
        // magnitude; the sign is prose.
        authorised.push(median.unsigned_abs() as f64);
        let _ = write!(
            text,
            " Of {} priced, the median held from first fill to last price is {median} bps.",
            held.len()
        );
    }
    match vault {
        Some(v) => {
            let sol = v.lamports as f64 / 1_000_000_000.0;
            let rendered = format!("{sol:.3}");
            authorised.push(sol);
            if let Ok(r) = rendered.parse::<f64>() {
                authorised.push(r);
            }
            let at = timestamp_from_seconds(v.measured_at);
            authorised.extend(at[..10].split('-').filter_map(|p| p.parse::<f64>().ok()));
            authorised.extend(at[11..19].split(':').filter_map(|p| p.parse::<f64>().ok()));
            let _ = write!(text, " Pool: {rendered} SOL at {at}.");
        }
        None => text.push_str(" Pool: no token yet."),
    }

    Rendered::Post(Post {
        text,
        authorised,
        source: serde_json::to_string(rows).unwrap_or_default(),
    })
}

/// The `YYYY-MM-DD` of a moment.
#[must_use]
pub fn date_of(secs: u64) -> String {
    date_from_days(i64::try_from(secs / 86_400).unwrap_or(i64::MAX))
}

/// Where a day's rows live, and where its posted marker goes.
#[must_use]
pub fn paths_for(daily_dir: &str, date: &str) -> (String, String) {
    (
        format!("{daily_dir}/{date}.json"),
        format!("{daily_dir}/{date}.posted"),
    )
}

/// Whether today's post is due: past the hour, the file exists, not yet
/// posted. Returns the date when it is.
#[must_use]
pub fn due(now: u64, daily_dir: &str) -> Option<String> {
    if (now % 86_400) / 3_600 < POST_HOUR_UTC {
        return None;
    }
    let date = date_of(now);
    let (rows, posted) = paths_for(daily_dir, &date);
    (std::path::Path::new(&rows).exists() && !std::path::Path::new(&posted).exists())
        .then_some(date)
}

/// Posts today's rows if they are due. Returns how many posts were sent on
/// the first publisher.
///
/// The marker is written whether or not anything was sent -- a quiet day, a
/// dry run and a refused post are all "done for today" -- so one bad file does
/// not become a post attempt on every tick. The log says which it was.
///
/// # Errors
///
/// The I/O error from the log or the marker.
pub fn post_if_due(
    now: u64,
    daily_dir: &str,
    vault: Option<&Vault>,
    publisher: &dyn Publisher,
    posts_log: &str,
    telegram: &dyn Publisher,
    telegram_log: &str,
) -> std::io::Result<usize> {
    let Some(date) = due(now, daily_dir) else {
        return Ok(0);
    };
    let (rows_path, marker) = paths_for(daily_dir, &date);
    let rows = Rows::read(&rows_path)?;
    let sent = match render(&rows, vault) {
        Rendered::NothingYet(why) => {
            eprintln!("radar-analyst: seven days later, {date}: {why}");
            0
        }
        Rendered::Post(post) => {
            let label = format!("daily:{date}");
            let sent = crate::weekly::publish(
                publisher,
                posts_log,
                &label,
                std::slice::from_ref(&post),
                now,
            )?;
            crate::weekly::publish(telegram, telegram_log, &label, &[post], now)?;
            sent
        }
    };
    std::fs::write(&marker, timestamp_from_seconds(now))?;
    Ok(sent)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(
        mint: &str,
        graduation: Option<Graduation>,
        quiet: Option<bool>,
        held: Option<i64>,
    ) -> Row {
        Row {
            mint: mint.to_owned(),
            asked_at: 1_788_000_000,
            reply_id: Some("r".to_owned()),
            graduation,
            quiet_since_reply: quiet,
            held_bps: held,
        }
    }

    fn rows(rows: Vec<Row>) -> Rows {
        Rows {
            asked_on: "2026-08-29".to_owned(),
            built_at: 1_788_600_000,
            watermark_slot: 444_505_805,
            rows,
        }
    }

    #[test]
    fn the_post_counts_what_happened_and_passes_both_checks() {
        let day = rows(vec![
            row("A", Some(Graduation::Organic), Some(false), Some(-3_228)),
            row("B", Some(Graduation::Instant), Some(false), Some(-5_981)),
            row("C", None, Some(true), None),
            row("D", None, None, Some(120)),
        ]);
        let vault = Vault {
            address: "V".to_owned(),
            lamports: 500_000_000,
            measured_at: 1_788_600_000,
        };
        let Rendered::Post(post) = render(&day, Some(&vault)) else {
            panic!("a post");
        };
        assert!(
            post.text
                .starts_with("Seven days later: 4 coins we were asked about on 2026-08-29."),
            "{}",
            post.text
        );
        assert!(
            post.text
                .contains("2 graduated, 1 of them inside the launch block"),
            "{}",
            post.text
        );
        assert!(
            post.text
                .contains("1 has had no transfer since we answered"),
            "{}",
            post.text
        );
        // Three priced: -5981, -3228, 120 -> the middle one.
        assert!(
            post.text.contains(
                "Of 3 priced, the median held from first fill to last price is -3228 bps."
            ),
            "{}",
            post.text
        );
        assert!(post.text.contains("Pool: 0.500 SOL at"), "{}", post.text);
        assert!(
            post.text.chars().count() <= 280,
            "{}: {}",
            post.text.chars().count(),
            post.text
        );
        assert_eq!(crate::weekly::check(&post), Ok(()), "{}", post.text);
        assert!(post.source.contains("\"asked_on\":\"2026-08-29\""));
    }

    #[test]
    fn a_day_with_nothing_asked_says_so_and_is_not_a_post() {
        let day = rows(Vec::new());
        assert!(
            matches!(render(&day, None), Rendered::NothingYet(ref why) if why.contains("2026-08-29"))
        );
        // And one coin, unpriced, unmeasured: a post with no median and no
        // pool, still under the limit and still checked.
        let day = rows(vec![row("A", None, None, None)]);
        let Rendered::Post(post) = render(&day, None) else {
            panic!("a post");
        };
        assert!(
            post.text.contains("1 coin we were asked about"),
            "{}",
            post.text
        );
        assert!(!post.text.contains("median"), "{}", post.text);
        assert!(post.text.ends_with("Pool: no token yet."), "{}", post.text);
        assert_eq!(crate::weekly::check(&post), Ok(()), "{}", post.text);
    }

    #[test]
    fn the_median_is_authorised_as_its_magnitude_and_a_stray_figure_is_not() {
        // Re-applied by dropping `authorised.push(median.unsigned_abs() as
        // f64)`: the median is flagged and the first assertion fails.
        let day = rows(vec![row("A", None, None, Some(-3_228))]);
        let Rendered::Post(mut post) = render(&day, None) else {
            panic!("a post");
        };
        assert_eq!(crate::weekly::check(&post), Ok(()));
        post.text.push_str(" One did 9000%.");
        assert!(crate::weekly::check(&post).is_err());
    }

    #[test]
    fn the_file_round_trips_and_due_needs_the_hour_the_file_and_no_marker() {
        let dir = std::env::temp_dir().join(format!("radar-daily-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let dir = dir.to_string_lossy().into_owned();
        // 2026-09-05 at 11:59:59 and at 12:00:00 UTC.
        let day = 20_701u64 * 86_400;
        let before = day + 11 * 3_600 + 59 * 60 + 59;
        let at = day + 12 * 3_600;
        assert_eq!(date_of(at), "2026-09-05");
        let (path, marker) = paths_for(&dir, "2026-09-05");

        assert_eq!(due(at, &dir), None, "no file yet");
        let written = rows(vec![row("A", None, None, None)]);
        written.write(&path).expect("write");
        assert_eq!(Rows::read(&path).expect("read"), written);
        assert_eq!(due(before, &dir), None, "before the hour");
        assert_eq!(due(at, &dir).as_deref(), Some("2026-09-05"));
        std::fs::write(&marker, "x").expect("marker");
        assert_eq!(due(at, &dir), None, "already posted");
    }

    #[test]
    fn posting_writes_the_marker_whether_or_not_anything_went_out() {
        // A dry run: the post is recorded, nothing is sent, and the marker is
        // written so the next tick does not try again. Re-applied by writing
        // the marker only when `sent > 0`: the second call posts again and the
        // log doubles.
        let dir = std::env::temp_dir().join(format!("radar-daily-post-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let dir = dir.to_string_lossy().into_owned();
        let at = 20_701u64 * 86_400 + 13 * 3_600;
        let (path, marker) = paths_for(&dir, "2026-09-05");
        rows(vec![row(
            "A",
            Some(Graduation::Organic),
            Some(true),
            Some(-100),
        )])
        .write(&path)
        .expect("write");
        let posts = format!("{dir}/posts.jsonl");
        let telegram = format!("{dir}/telegram.jsonl");

        let sent = post_if_due(
            at,
            &dir,
            None,
            &crate::publish::DryRun,
            &posts,
            &crate::publish::DryRun,
            &telegram,
        )
        .expect("io");
        assert_eq!(sent, 0);
        assert!(std::path::Path::new(&marker).exists());
        let logged = crate::log::latest(&posts).expect("read");
        assert_eq!(logged.len(), 1);
        assert_eq!(logged[0].mention_id, "daily:2026-09-05:0");
        assert!(logged[0].reply_id.is_none());

        let again = post_if_due(
            at,
            &dir,
            None,
            &crate::publish::DryRun,
            &posts,
            &crate::publish::DryRun,
            &telegram,
        )
        .expect("io");
        assert_eq!(again, 0);
        assert_eq!(
            crate::log::latest(&posts).expect("read").len(),
            1,
            "not posted twice"
        );
    }
}
