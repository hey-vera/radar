// SPDX-License-Identifier: Apache-2.0
//! Sending a reply, and the two reasons nothing here sends one yet.
//!
//! # Publishing is a separate capability from writing
//!
//! `radar-roast` writes a reply and cannot post it. This module posts and
//! cannot write. That split is the same one rule 1 draws between a proposal and
//! a signature, applied to speech instead of money: **the component that decides
//! what to say holds no credential, and the component holding the credential
//! decides nothing.**
//!
//! A single module that did both would be one bug away from publishing
//! something no check had seen.
//!
//! # Rule 8, and it is load-bearing here
//!
//! [`Publisher`] has exactly one implementation in this crate and it is
//! [`DryRun`], which writes to the log and returns no reply id. An instance with
//! no credential configured therefore **cannot post**, rather than posting
//! badly, and that is the deliberate resting state of this crate today.
//!
//! # Why the X client is not written yet
//!
//! Two numbers gate it, and AGENTS.md section 2 is explicit that a decision
//! turning on a price needs that price verified first — ADR 0011 is the standing
//! example of what happens otherwise, having chosen a wallet vendor partly on
//! two free tiers neither of which had been looked up.
//!
//! 1. Does `GET /2/users/:id/mentions` bill as an **owned read** ($0.001) or a
//!    standard read ($0.005)? That is 5× on the dominant read line, and
//!    docs.x.com lists owned reads as "posts, bookmarks, followers, likes, lists
//!    and more" without naming mentions.
//! 2. Is a summoned reply **containing a URL** $0.010 or $0.200? 20× on
//!    everything that links to a page.
//!
//! Both are settleable in the Developer Console with one live test post, and
//! neither is settleable by reading more. Writing the client first would mean
//! building a polling loop whose per-call cost is unknown to within a factor of
//! five, which is how a bill arrives that nobody predicted.
//!
//! The rest of the loop — parsing, admission, the log, the reply itself — does
//! not depend on either answer, which is why it is finished and this is not.

use crate::log::Entry;

/// Somewhere a reply can go.
#[derive(Debug, thiserror::Error)]
pub enum Undeliverable {
    /// No credential is configured.
    #[error("no publisher configured: this instance cannot post")]
    Unconfigured,
    /// The platform refused or could not be reached.
    #[error("publish failed: {0}")]
    Failed(String),
}

/// Something that can publish a reply.
///
/// Object-safe, so a binary holds one behind a `dyn` and does not care which it
/// got — the same shape `radar_model::Provider` uses, and for the same reason:
/// moving from a dry run to a live account should be a configuration change
/// rather than a rewrite.
pub trait Publisher: core::fmt::Debug {
    /// What this is, for logs and for an operator asking what is running.
    fn name(&self) -> &'static str;

    /// Publishes a reply to a mention, returning the new reply's id.
    ///
    /// # Errors
    ///
    /// [`Undeliverable`] when there is no credential or the platform refused.
    fn reply(&self, in_reply_to: &str, text: &str) -> Result<String, Undeliverable>;
}

/// A publisher that writes the reply down and posts nothing.
///
/// The default, and not a placeholder. Reading a few hundred of these beside
/// their fact sheets is how the account's voice gets judged before anybody
/// outside sees it, and the plan asks for exactly that before Phase 3 ships.
#[derive(Debug, Default)]
pub struct DryRun;

impl Publisher for DryRun {
    fn name(&self) -> &'static str {
        "dry-run"
    }

    fn reply(&self, _in_reply_to: &str, _text: &str) -> Result<String, Undeliverable> {
        // Not an error dressed as success: no reply id exists because no reply
        // exists. `Entry::reply_id` stays `None`, and the log therefore records
        // what Radar would have said rather than claiming it said it.
        Err(Undeliverable::Unconfigured)
    }
}

/// Publishes an entry, or records that it was not published.
///
/// The log is written **either way**, and it is written *before* the reply is
/// treated as sent. Publishing something there is no record of is the one
/// outcome [`crate::log`] exists to prevent, and it is worse than not
/// publishing at all.
///
/// # Errors
///
/// The I/O error when the log cannot be written. **This stops the reply**: an
/// account that cannot record what it says must not say it.
pub fn publish(
    publisher: &dyn Publisher,
    log_path: &str,
    mut entry: Entry,
) -> std::io::Result<Entry> {
    match publisher.reply(&entry.mention_id, &entry.reply) {
        Ok(id) => entry.reply_id = Some(id),
        Err(why) => {
            entry.reply_id = None;
            // Recorded on the entry rather than only on a terminal: a run whose
            // publisher was down all night should be readable from the log
            // afterwards, not reconstructed from whoever was watching.
            entry.fellback = Some(match entry.fellback {
                Some(existing) => format!("{existing}; not published: {why}"),
                None => format!("not published: {why}"),
            });
        }
    }
    crate::log::append(log_path, &entry)?;
    Ok(entry)
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
            reply_id: None,
        }
    }

    fn temp(name: &str) -> String {
        let dir = std::env::temp_dir().join(format!("radar-pub-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temp dir");
        let p = dir.join(name);
        let p = p.to_str().expect("a path").to_owned();
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn the_default_publisher_cannot_post() {
        // Rule 8. An instance with no credential does not post badly; it does
        // not post. This is the resting state of the crate.
        let err = DryRun.reply("m1", "text").expect_err("must refuse");
        assert!(matches!(err, Undeliverable::Unconfigured));
    }

    #[test]
    fn a_dry_run_is_logged_with_no_reply_id() {
        // "We decided this" is recorded; "we published this" is not claimed.
        let path = temp("dry.jsonl");
        let out = publish(&DryRun, &path, entry()).expect("logged");
        assert!(out.reply_id.is_none());
        assert!(out.fellback.expect("a reason").contains("not published"));

        let back = crate::log::read(&path).expect("read");
        assert_eq!(back.len(), 1);
        assert!(back[0].reply_id.is_none());
        assert_eq!(back[0].fact_sheet, "recipients: 6\n");
    }

    #[derive(Debug)]
    struct Posts;

    impl Publisher for Posts {
        fn name(&self) -> &'static str {
            "posts"
        }
        fn reply(&self, _: &str, _: &str) -> Result<String, Undeliverable> {
            Ok("reply-1".to_owned())
        }
    }

    #[test]
    fn a_published_reply_is_logged_with_its_id() {
        let path = temp("live.jsonl");
        let out = publish(&Posts, &path, entry()).expect("logged");
        assert_eq!(out.reply_id.as_deref(), Some("reply-1"));
        assert!(out.fellback.is_none());
        assert_eq!(
            crate::log::read(&path).expect("read")[0]
                .reply_id
                .as_deref(),
            Some("reply-1")
        );
    }

    #[test]
    fn a_log_that_cannot_be_written_fails_the_reply() {
        // An account that cannot record what it says must not say it. Verified
        // against a path that cannot exist, because the alternative -- logging
        // the failure and posting anyway -- is the exact situation the log
        // exists to prevent.
        let bad = "/this/path/does/not/exist/and/cannot/be/created/log.jsonl";
        assert!(publish(&Posts, bad, entry()).is_err());
    }

    #[test]
    fn a_pre_existing_fallback_reason_is_kept_alongside_the_publish_failure() {
        // Two different things went wrong and both matter: the model wrote
        // something it should not have, AND the reply never went out. Losing
        // the first would hide the drift signal.
        let path = temp("both.jsonl");
        let mut e = entry();
        e.fellback = Some("Fabricated".to_owned());
        let out = publish(&DryRun, &path, e).expect("logged");
        let reason = out.fellback.expect("a reason");
        assert!(reason.contains("Fabricated"), "{reason}");
        assert!(reason.contains("not published"), "{reason}");
    }
}
