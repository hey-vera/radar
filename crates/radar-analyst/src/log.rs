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
    /// The refusal signals the sheet carried, for the hunter rank (design 0009
    /// M3), in the sheet's fixed order.
    ///
    /// `None` on a line written before the count existed (2026-09-05), which
    /// is unknown and not empty: an old reply cannot be scored, and a rule that
    /// read it as zero would rank its summoner below one whose coin was clean.
    /// `Some(vec![])` is a sheet that was counted and carried nothing. An
    /// absent field reads as `None` because that is what serde does with an
    /// `Option`; the test below pins that this schema relies on it.
    pub signals: Option<Vec<radar_roast::sheet::Signal>>,
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

/// The log, with each mention's last word.
///
/// # Why the raw read is not enough
///
/// [`crate::publish`] appends **twice** for one reply: once before it says
/// anything, so a statement is never made without a record, and once after, to
/// record the platform's id for it or why there is none. Both lines are true and
/// the second supersedes the first.
///
/// So a reader counting lines counts intents, not replies. Anything answering
/// *"what did this account say, and what happened to it"* wants this function.
/// [`read`] stays raw because the ordering itself is evidence: an intent with no
/// outcome beside it is a reply that was interrupted, and folding that away
/// would hide the one case worth noticing.
///
/// Ordered by first appearance, so the shape of a run is preserved rather than
/// sorted into something tidier than it was.
///
/// # Errors
///
/// The underlying I/O error if the file cannot be read at all.
pub fn latest(path: &str) -> std::io::Result<Vec<Entry>> {
    let all = read(path)?;
    let mut order: Vec<String> = Vec::new();
    let mut newest: std::collections::HashMap<String, Entry> = std::collections::HashMap::new();
    for entry in all {
        if !newest.contains_key(&entry.mention_id) {
            order.push(entry.mention_id.clone());
        }
        newest.insert(entry.mention_id.clone(), entry);
    }
    Ok(order
        .into_iter()
        .filter_map(|id| newest.remove(&id))
        .collect())
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_line_written_before_the_count_existed_reads_as_unknown_and_not_as_nothing() {
        // Rule 9 on the log's own history. Lines from before 2026-09-05 have
        // no `signals` field; they must load, and they must load as `None`,
        // because "not counted" ranks nobody and "counted zero" ranks the
        // summoner below everyone whose coin carried one signal. The type is
        // what makes this true -- a missing `Option` is `None` to serde -- so
        // the mutant here is a `Vec<Signal>` with a default, and it was made:
        // the three assertions below then refuse to compile, because a `Vec`
        // is not an `Option`. That is the type doing the work, and this test
        // is here so the schema cannot drift to the `Vec` without a reader
        // meeting the reason.
        let old = r#"{"at":1,"mention_id":"m","summoner":"s","mint":null,"read_at_slot":null,"fact_sheet":"","reply":"","fellback":null,"reply_id":null}"#;
        let entry: super::Entry = serde_json::from_str(old).expect("an old line still loads");
        assert_eq!(entry.signals, None);

        let counted = r#"{"at":1,"mention_id":"m","summoner":"s","mint":null,"read_at_slot":null,"fact_sheet":"","reply":"","fellback":null,"reply_id":null,"signals":["creator_bought_own_launch","launch_block_in_strongest_band"]}"#;
        let entry: super::Entry = serde_json::from_str(counted).expect("a counted line loads");
        assert_eq!(
            entry.signals,
            Some(vec![
                radar_roast::sheet::Signal::CreatorBoughtOwnLaunch,
                radar_roast::sheet::Signal::LaunchBlockInStrongestBand,
            ])
        );
        // And a counted sheet that carried nothing is `Some([])`, not `None`.
        let clean = r#"{"at":1,"mention_id":"m","summoner":"s","mint":null,"read_at_slot":null,"fact_sheet":"","reply":"","fellback":null,"reply_id":null,"signals":[]}"#;
        let entry: super::Entry = serde_json::from_str(clean).expect("loads");
        assert_eq!(entry.signals, Some(Vec::new()));
    }

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
            signals: None,
            reply_id: Some("r1".to_owned()),
        }
    }

    #[test]
    fn an_interrupted_reply_is_still_the_account_saying_something() {
        // The case the two-append ordering creates and the one that matters
        // most: the intent was recorded and the process died before the
        // outcome. `latest` must still report it -- a mention with one record
        // is a reply that may well have gone out, and dropping it from the
        // folded view would hide exactly the entry somebody is looking for.
        //
        // This is the mutant `delete ! in latest` survived on: with every
        // mention carrying two records, keeping only the ids already seen
        // produces the same answer, and only a single-record mention tells the
        // two apart.
        let dir = std::env::temp_dir().join(format!("radar-log-fold-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temp dir");
        let path = dir.join("interrupted.jsonl");
        let path = path.to_str().expect("a path").to_owned();
        let _ = std::fs::remove_file(&path);

        // One mention answered fully, one interrupted after its intent.
        let mut finished_intent = entry();
        finished_intent.mention_id = "done".to_owned();
        finished_intent.reply_id = None;
        let mut finished_outcome = finished_intent.clone();
        finished_outcome.reply_id = Some("r-done".to_owned());

        let mut interrupted = entry();
        interrupted.mention_id = "interrupted".to_owned();
        interrupted.reply_id = None;

        append(&path, &finished_intent).expect("append");
        append(&path, &finished_outcome).expect("append");
        append(&path, &interrupted).expect("append");

        let folded = latest(&path).expect("read");
        assert_eq!(folded.len(), 2, "both mentions must appear: {folded:?}");
        assert_eq!(folded[0].mention_id, "done", "order of first appearance");
        assert_eq!(folded[0].reply_id.as_deref(), Some("r-done"));
        assert_eq!(folded[1].mention_id, "interrupted");
        assert!(
            folded[1].reply_id.is_none(),
            "an interrupted reply has no id, and that is the signal"
        );
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
