// SPDX-License-Identifier: Apache-2.0
//! What was asked, what was measured, and what was said.
//!
//! # Non-negotiable, and the reason is specific
//!
//! This is the only thing that turns a public mistake into a **correction**
//! rather than an argument. When somebody screenshots a reply and says the
//! number is wrong, there are two possible conversations: one where Radar can
//! produce the fact sheet the reply was generated from and the slot it was read
//! at, and one where it cannot. The second one is unwinnable regardless of who
//! is right.
//!
//! This repository's house style is already this — `0016` corrects `0014`,
//! `0022` reverses its own recommendation, PR #108 opens *"I over-claimed in
//! #105."* The log is what makes that possible for something published
//! automatically, thousands of times, to people who did not ask for it to be
//! right.
//!
//! # Append-only, one JSON object per line
//!
//! The same shape the store uses and for the same reason: a crash mid-write
//! loses the last line rather than the file, and a partial line is detectably
//! partial. Deliberately **not** the Parquet store — a reply log is not a
//! point-in-time record of the chain, and writing it into a table a replay reads
//! would put live observations where recorded ones belong (rule 3).

use serde::{Deserialize, Serialize};

/// One answered — or refused — mention.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    /// When, as seconds since the epoch.
    pub at: u64,
    /// The mention's id on the platform.
    pub mention_id: String,
    /// Who asked.
    pub summoner: String,
    /// The mint, when one was resolved.
    pub mint: Option<String>,
    /// The slot the facts were read at.
    pub read_at_slot: Option<u64>,
    /// The fact sheet, rendered, exactly as the model was shown it.
    ///
    /// **The whole point of the log.** Storing the reply without the evidence
    /// records what Radar said and not whether it was entitled to say it, which
    /// is the half that settles an argument.
    pub fact_sheet: String,
    /// What was published, or would have been.
    pub reply: String,
    /// Whether the deterministic template shipped, and why.
    ///
    /// `None` means the model's reply was used. Anything else is the early
    /// warning that the voice pass is drifting.
    pub fellback: Option<String>,
    /// The published reply's id, when one was actually sent.
    ///
    /// `None` for a dry run or a failed post. Distinguishing "we decided this"
    /// from "we published this" matters: only the second is a public statement.
    pub reply_id: Option<String>,
}

/// Appends to a log file.
///
/// # Errors
///
/// Returns the underlying I/O error. **A failure here must stop the reply**, not
/// be logged and shrugged at: publishing something there is no record of is
/// exactly the situation this file exists to prevent, and it is worse than not
/// publishing.
pub fn append(path: &str, entry: &Entry) -> std::io::Result<()> {
    use std::io::Write as _;
    let line = serde_json::to_string(entry)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{line}")
}

/// Reads a log back.
///
/// Skips lines that will not parse rather than failing the read: a log with one
/// torn line at the end of a crashed write is still the record of everything
/// before it, and refusing to open it would lose the evidence to protect a
/// tidiness that nobody needs.
///
/// # Errors
///
/// Returns the underlying I/O error if the file cannot be read at all.
pub fn read(path: &str) -> std::io::Result<Vec<Entry>> {
    let text = std::fs::read_to_string(path)?;
    Ok(text
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> Entry {
        Entry {
            at: 1_788_000_000,
            mention_id: "m1".to_owned(),
            summoner: "alice".to_owned(),
            mint: Some("MintOne".to_owned()),
            read_at_slot: Some(444_007_820),
            fact_sheet: "recipients: 6\n".to_owned(),
            reply: "Six token accounts.".to_owned(),
            fellback: None,
            reply_id: Some("r1".to_owned()),
        }
    }

    #[test]
    fn an_entry_round_trips_through_the_log() {
        let dir = std::env::temp_dir().join(format!("radar-log-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temp dir");
        let path = dir.join("round-trip.jsonl");
        let path = path.to_str().expect("a path");
        let _ = std::fs::remove_file(path);

        append(path, &entry()).expect("append");
        append(path, &entry()).expect("append");
        let back = read(path).expect("read");

        assert_eq!(back.len(), 2);
        assert_eq!(back[0], entry());
        // The evidence is there, not just the words. A log of replies without
        // fact sheets records what Radar said and not whether it was entitled
        // to say it.
        assert_eq!(back[0].fact_sheet, "recipients: 6\n");
        assert_eq!(back[0].read_at_slot, Some(444_007_820));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_torn_final_line_does_not_lose_the_rest() {
        // A crash mid-write is the ordinary way this file ends. Refusing to
        // open it would lose every prior entry to protect a tidiness nobody
        // needs -- and those entries are the evidence.
        let dir = std::env::temp_dir().join(format!("radar-log-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temp dir");
        let path = dir.join("torn.jsonl");
        let path = path.to_str().expect("a path");
        let _ = std::fs::remove_file(path);

        append(path, &entry()).expect("append");
        std::fs::write(
            path,
            format!(
                "{}\n{{\"at\": 1788000001, \"mention_i",
                serde_json::to_string(&entry()).expect("json")
            ),
        )
        .expect("write");

        let back = read(path).expect("read");
        assert_eq!(back.len(), 1);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_dry_run_is_distinguishable_from_a_published_reply() {
        // "We decided this" and "we published this" are different facts, and
        // only the second is a public statement anyone can hold Radar to.
        let mut dry = entry();
        dry.reply_id = None;
        assert!(dry.reply_id.is_none());
        assert!(entry().reply_id.is_some());
    }
}
