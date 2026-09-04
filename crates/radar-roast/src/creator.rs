// SPDX-License-Identifier: Apache-2.0
//! Every creator's record, precomputed once so a reply can look one up.
//!
//! # Why this exists
//!
//! The public analyst answers about a coin while the thread it was asked in is
//! still alive. It reads the chain on demand for the things the chain can
//! answer — the launch block, the curve — and that works because those are one
//! block and one account.
//!
//! *"How did this creator's other tokens turn out"* is not answerable that way.
//! It is a question about 529,000 recorded launches joined to 1.4 million
//! outcome measurements, and `creator_track_record` answers it by decoding both
//! tables in full: ten seconds and climbing, on a shared two-core box, for one
//! mention.
//!
//! So it is precomputed. The same shape `docs/research/data/0024-base-rates.json`
//! already uses: a file with the date it was measured, read in microseconds,
//! refused rather than guessed at when it is absent.
//!
//! # What it is for
//!
//! Without it, every reply is the same reply. Three different coins measured on
//! 2026-09-04 produced identical text — the cost line, the recipient count, the
//! band — because nothing in the fact sheet was about *that* coin. A creator's
//! record is the fact that differs, and it is the one Radar has that nobody else
//! does: 117,390 creators, watched since August.
//!
//! # Where the halves live
//!
//! This is the **reading** half: the type, the lookup, and the file. Building it
//! needs the store, which the analyst deliberately does not have on its path, so
//! `radar_research::creator_index` owns that and writes this shape.
//!
//! The same split `BaseRates` uses, for the same reason: the consumer owns the
//! type it depends on, and the producer is free to be as heavy as it needs.
//!
//! # What it deliberately does not carry
//!
//! No rates and no verdicts, only counts. A rate computed here would be a rate
//! computed twice — `creator_track_record` already has one, with a minimum
//! sample and a `sample_note` explaining itself — and two of them would drift.
//! The consumer decides what a count means, and refuses to say anything when the
//! sample is too small.
//!
//! And **graduation is split**. A curve bought out within three slots of launch
//! was bought by capital committed before the token existed, so it is evidence
//! of coordination rather than demand. A creator ranked on the undifferentiated
//! count is ranked partly on how well they bundle.

use std::collections::BTreeMap;

/// Where the index lives by default.
///
/// Beside the base rates, and read the same way: a published measurement with
/// the slot it was taken at, refused rather than guessed at when absent.
pub const DEFAULT_PATH: &str = "docs/research/data/creator-index.json";

/// One creator's record, as counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Record {
    /// Launches recorded at or before the watermark.
    pub launches: u32,
    /// Launches for which an outcome has been measured.
    ///
    /// At most `launches`. A gap means the outcome pass has not caught up, not
    /// that those tokens did nothing — which is why both numbers are kept and a
    /// consumer must quote the second when it quotes a share.
    pub measured: u32,
    /// Measured tokens whose curve filled over time rather than in a block.
    pub organic: u32,
    /// Measured tokens whose curve completed within three slots of launch.
    pub instant: u32,
    /// Measured tokens that showed almost no life.
    pub stillborn: u32,
}

/// Every creator's record at one watermark.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct CreatorIndex {
    /// The watermark this was computed at.
    pub watermark_slot: u64,
    /// When it was built, as seconds since the epoch.
    pub built_at: u64,
    /// Creator address, base58, to their record.
    pub creators: BTreeMap<String, Record>,
}

impl CreatorIndex {
    /// One creator's record, or `None` if this index has never seen them.
    ///
    /// `None` means **not in the record**, never "launched nothing". A creator
    /// absent from a store that starts in August is a creator who launched
    /// before Radar was watching, and a reply that read absence as innocence
    /// would be rule 9 broken in the direction that flatters.
    #[must_use]
    pub fn get(&self, creator: &str) -> Option<&Record> {
        self.creators.get(creator)
    }

    /// How many creators are in it.
    #[must_use]
    pub fn len(&self) -> usize {
        self.creators.len()
    }

    /// Whether it holds nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.creators.is_empty()
    }

    /// Writes the index where a consumer will find it.
    ///
    /// # Errors
    ///
    /// The I/O error, or a serialisation failure.
    pub fn write(&self, path: &str) -> std::io::Result<()> {
        let json = serde_json::to_string(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        // Beside and renamed, for the reason every other file this project
        // writes is: a half-written index still parses as JSON right up until
        // the truncation, and a creator missing from it reads as a creator with
        // no record.
        let temp = format!("{path}.new");
        std::fs::write(&temp, json)?;
        std::fs::rename(&temp, path)
    }

    /// Reads an index from disk.
    ///
    /// # Errors
    ///
    /// The I/O error, or a parse failure.
    pub fn read(path: &str) -> std::io::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        serde_json::from_str(&text)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_creator_absent_from_the_index_is_unknown_rather_than_innocent() {
        // The store starts in August. A creator who launched before that is not
        // a creator who launched nothing, and a reply reading absence as
        // innocence would be rule 9 broken in the direction that flatters.
        let index = CreatorIndex {
            watermark_slot: 1,
            built_at: 0,
            creators: BTreeMap::new(),
        };
        assert_eq!(index.get("nobody"), None);
        assert!(index.is_empty());
    }

    #[test]
    fn an_index_round_trips_through_a_file() {
        let dir = std::env::temp_dir().join(format!("radar-cidx-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temp dir");
        let path = dir.join("index.json");
        let path = path.to_str().expect("a path").to_owned();

        let mut creators = BTreeMap::new();
        creators.insert(
            "abc".to_owned(),
            Record {
                launches: 47,
                measured: 40,
                organic: 0,
                instant: 2,
                stillborn: 31,
            },
        );
        let index = CreatorIndex {
            watermark_slot: 444_339_860,
            built_at: 1_788_000_000,
            creators,
        };
        index.write(&path).expect("written");

        let back = CreatorIndex::read(&path).expect("read");
        assert_eq!(back.watermark_slot, 444_339_860);
        let record = back.get("abc").expect("the creator");
        assert_eq!(record.launches, 47);
        assert_eq!(record.measured, 40);
        assert_eq!(record.organic, 0);
        assert_eq!(record.instant, 2);
        assert_eq!(record.stillborn, 31);

        assert!(
            !std::path::Path::new(&format!("{path}.new")).exists(),
            "the temporary file must not survive"
        );
    }

    #[test]
    fn measured_is_kept_apart_from_launches() {
        // A gap between them means the outcome pass has not caught up, not that
        // those tokens did nothing. A consumer quoting a share must quote the
        // denominator it is a share of, and it can only do that if both numbers
        // are here.
        let record = Record {
            launches: 47,
            measured: 40,
            organic: 0,
            instant: 2,
            stillborn: 31,
        };
        assert!(record.measured <= record.launches);
        assert!(record.organic + record.instant + record.stillborn <= record.measured);
    }
}
