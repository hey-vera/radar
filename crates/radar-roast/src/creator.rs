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

/// The whole population, at the same watermark as the records.
///
/// # Why this rides along rather than being measured separately
///
/// It is the sum of the records, and computing it in the same pass is what makes
/// it *consistent with* them: a population measured by a second scan could
/// disagree with the parts it is supposed to be the total of, and the reply
/// would quote both.
///
/// It also costs nothing. The pass that builds the index has already visited
/// every launch and every outcome; these are five additions per row.
///
/// # Why it is worth having at all
///
/// `docs/research/data/0024-base-rates.json` carries two figures of the same
/// shape, and they came from **outside**: a public RPC walking 45 slots and a
/// SQL endpoint that truncates silently at a thousand rows. Both are samples of
/// a window. This is the population Radar actually recorded — every succeeded
/// launch at the watermark — measured offline, from the store, deterministically.
///
/// That does not make the snapshot wrong or replaceable. The snapshot carries
/// the **recipient distribution**, which needs the launch block and which the
/// store did not record until ADR 0012. These two overlap on exactly two
/// figures, and on those two this is the better instrument.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Population {
    /// Succeeded launches recorded at or before the watermark.
    pub launches: u64,
    /// Of those, how many have had an outcome measured.
    pub measured: u64,
    /// Measured tokens whose curve filled over time.
    pub organic: u64,
    /// Measured tokens whose curve completed within three slots of launch.
    pub instant: u64,
    /// Measured tokens that showed almost no life.
    pub stillborn: u64,
}

impl Population {
    /// Share of measured tokens that filled their curve over time.
    ///
    /// `None` when nothing has been measured. **Not zero** — rule 9, and this is
    /// the direction that flatters: "0% of launches graduate" read off an empty
    /// denominator is a measurement of the outcome pass, published as a fact
    /// about the venue.
    #[must_use]
    pub fn organic_share(&self) -> Option<f64> {
        self.share(self.organic)
    }

    /// Share of measured tokens whose curve completed inside three slots.
    ///
    /// `None` when nothing has been measured; see [`Self::organic_share`].
    #[must_use]
    pub fn instant_share(&self) -> Option<f64> {
        self.share(self.instant)
    }

    /// Share of measured tokens that graduated at all, by either route.
    ///
    /// `None` when nothing has been measured; see [`Self::organic_share`].
    #[must_use]
    pub fn graduated_share(&self) -> Option<f64> {
        self.share(self.organic + self.instant)
    }

    /// Share of measured tokens that showed almost no life.
    ///
    /// `None` when nothing has been measured; see [`Self::organic_share`].
    #[must_use]
    pub fn stillborn_share(&self) -> Option<f64> {
        self.share(self.stillborn)
    }

    /// A share of the **measured** population, never of the recorded one.
    ///
    /// The denominator is `measured` rather than `launches` deliberately. The
    /// gap between them is how far behind the outcome pass is, and dividing by
    /// `launches` would fold Radar's own lag into a claim about the venue —
    /// understating every graduation rate by exactly the size of the backlog.
    fn share(&self, part: u64) -> Option<f64> {
        // Precision: `u64 as f64` is lossless below 2^53 and these are counts of
        // launches, six orders of magnitude short of it.
        #[expect(
            clippy::cast_precision_loss,
            reason = "counts of launches; 2^53 is six orders of magnitude away"
        )]
        (self.measured > 0).then(|| part as f64 / self.measured as f64)
    }
}

/// Every creator's record at one watermark.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct CreatorIndex {
    /// The watermark this was computed at.
    pub watermark_slot: u64,
    /// When it was built, as seconds since the epoch.
    pub built_at: u64,
    /// The totals, over the same pass that built the records.
    ///
    /// `Option`, and defaulted, because an index written before this field
    /// existed is still a valid index and is sitting on the production box right
    /// now. Absent means **not measured**, which the consumer must say nothing
    /// about — a `Population::default()` here would be five zeroes claiming that
    /// nothing has ever graduated.
    #[serde(default)]
    pub population: Option<Population>,
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

    /// The totals, for a reader that needs five numbers and not 116,000
    /// records.
    ///
    /// `None` when the population was never measured: a summary of zeroes
    /// would be five claims that nothing ever graduated.
    #[must_use]
    pub fn summary(&self) -> Option<Summary> {
        self.population.map(|population| Summary {
            built_at: self.built_at,
            watermark_slot: self.watermark_slot,
            creators: u64::try_from(self.creators.len()).unwrap_or(u64::MAX),
            population,
        })
    }

    /// Writes the index where a consumer will find it, and the summary beside
    /// it.
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
        std::fs::rename(&temp, path)?;
        // The public site reads the totals without parsing the records, so
        // they are published as their own small file, from the same pass, at
        // the same moment. Only when there is something to say: an index with
        // no population writes no summary, and a reader finds nothing rather
        // than zeroes.
        if let Some(summary) = self.summary() {
            summary.write(&summary_path_beside(path))?;
        }
        Ok(())
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

/// Where the summary is published by default, beside the index.
pub const SUMMARY_PATH: &str = "docs/research/data/population.json";

/// The summary's path, given the index's: the same directory, its own name.
#[must_use]
pub fn summary_path_beside(index_path: &str) -> String {
    std::path::Path::new(index_path)
        .with_file_name("population.json")
        .to_string_lossy()
        .into_owned()
}

/// The population totals, published beside the index.
///
/// What the public site's stats document is built from. The index itself is
/// one record per creator -- 116,752 of them on 2026-09-04 -- and a public
/// endpoint that parsed it per request to read five totals would be the
/// three-second store scan in miniature, behind a viral link.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Summary {
    /// When the index was built, as seconds since the epoch.
    pub built_at: u64,
    /// The watermark it was built at.
    pub watermark_slot: u64,
    /// How many creators the index holds.
    pub creators: u64,
    /// The totals.
    pub population: Population,
}

impl Summary {
    /// Writes the summary, beside and renamed like the index.
    ///
    /// # Errors
    ///
    /// The I/O error, or a serialisation failure.
    pub fn write(&self, path: &str) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let temp = format!("{path}.new");
        std::fs::write(&temp, json)?;
        std::fs::rename(&temp, path)
    }

    /// Reads a summary from disk.
    ///
    /// # Errors
    ///
    /// The I/O error, or a parse failure. Absent means **not measured**, and
    /// the caller says so rather than filling in zeroes.
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
    fn the_summary_is_written_beside_the_index_and_only_when_there_is_one() {
        // The public site reads this file and not the index. Re-apply the bug
        // by dropping the summary write from `write` and the read below fails.
        let dir = tempfile::tempdir().expect("a temp dir");
        let index_path = dir.path().join("creator-index.json");
        let index_path = index_path.to_string_lossy().into_owned();
        let summary_path = summary_path_beside(&index_path);
        assert!(summary_path.ends_with("population.json"), "{summary_path}");

        let mut creators = BTreeMap::new();
        creators.insert("c1".to_owned(), Record::default());
        creators.insert("c2".to_owned(), Record::default());
        let index = CreatorIndex {
            watermark_slot: 444_374_676,
            built_at: 1_788_000_000,
            population: Some(Population {
                launches: 10,
                measured: 8,
                organic: 1,
                instant: 1,
                stillborn: 2,
            }),
            creators,
        };
        index.write(&index_path).expect("writes");
        let summary = Summary::read(&summary_path).expect("the summary is beside the index");
        assert_eq!(summary.creators, 2);
        assert_eq!(summary.watermark_slot, 444_374_676);
        assert_eq!(summary.built_at, 1_788_000_000);
        assert_eq!(summary.population.measured, 8);

        // An index with no population writes no summary: absent, not zeroes.
        // In its own directory, because the summary's name is fixed and the
        // first index's summary is already beside it.
        let older = CreatorIndex {
            population: None,
            ..index
        };
        let elsewhere = tempfile::tempdir().expect("a second temp dir");
        let older_path = elsewhere.path().join("creator-index.json");
        let older_path = older_path.to_string_lossy().into_owned();
        older.write(&older_path).expect("writes");
        assert!(
            Summary::read(&summary_path_beside(&older_path)).is_err(),
            "no population, so no summary file"
        );
    }

    #[test]
    fn a_creator_absent_from_the_index_is_unknown_rather_than_innocent() {
        // The store starts in August. A creator who launched before that is not
        // a creator who launched nothing, and a reply reading absence as
        // innocence would be rule 9 broken in the direction that flatters.
        let index = CreatorIndex {
            watermark_slot: 1,
            built_at: 0,
            population: None,
            creators: BTreeMap::new(),
        };
        assert_eq!(index.get("nobody"), None);
        assert!(index.is_empty());
    }

    #[test]
    fn the_size_of_the_index_is_the_number_of_creators_in_it() {
        // It is printed by `radar creator-index` and it is how an operator
        // knows the build worked: "117,680 creators at slot N" against "0
        // creators" is the difference between a good index and a silently
        // empty one, and a constant would report the same either way.
        let mut creators = BTreeMap::new();
        assert_eq!(
            CreatorIndex {
                watermark_slot: 1,
                built_at: 0,
                population: None,
                creators: creators.clone(),
            }
            .len(),
            0
        );

        for n in 0..3 {
            creators.insert(format!("creator-{n}"), Record::default());
        }
        let index = CreatorIndex {
            watermark_slot: 1,
            built_at: 0,
            population: None,
            creators,
        };
        assert_eq!(index.len(), 3);
        assert!(!index.is_empty(), "three is not none");
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
            population: None,
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

    #[test]
    fn a_share_is_out_of_what_was_measured_not_out_of_what_was_launched() {
        // Re-applying this bug is a one-character edit -- `launches` for
        // `measured` in `share` -- and it is the one that quietly understates
        // every graduation rate by the size of Radar's own outcome backlog.
        //
        // Here the backlog is half the population, so the wrong denominator
        // halves every figure: a 50% graduation rate published as 25%, which is
        // a measurement of the outcome pass presented as a fact about the venue.
        let p = Population {
            launches: 200,
            measured: 100,
            organic: 30,
            instant: 20,
            stillborn: 40,
        };
        assert!((p.graduated_share().expect("measured") - 0.50).abs() < 1e-12);
        assert!((p.organic_share().expect("measured") - 0.30).abs() < 1e-12);
        assert!((p.instant_share().expect("measured") - 0.20).abs() < 1e-12);
        assert!((p.stillborn_share().expect("measured") - 0.40).abs() < 1e-12);
    }

    #[test]
    fn nothing_measured_yields_no_share_at_all() {
        // Rule 9. `0.0` here would be "nothing on this venue ever graduates",
        // which is both false and the direction that sounds authoritative.
        let p = Population {
            launches: 5_000,
            measured: 0,
            organic: 0,
            instant: 0,
            stillborn: 0,
        };
        assert_eq!(p.graduated_share(), None);
        assert_eq!(p.organic_share(), None);
        assert_eq!(p.instant_share(), None);
        assert_eq!(p.stillborn_share(), None);
    }

    #[test]
    fn an_index_written_before_the_population_existed_still_loads() {
        // The one sitting on the production box right now has no `population`
        // key. Refusing it would take the creator facts away -- the facts that
        // stopped every reply being the same reply -- to gain a field.
        let old = r#"{"watermark_slot":444361818,"built_at":1788000000,
                      "creators":{"aaa":{"launches":3,"measured":2,"organic":1,
                      "instant":0,"stillborn":1}}}"#;
        let index: CreatorIndex = serde_json::from_str(old).expect("an older index loads");
        assert_eq!(index.len(), 1);
        assert_eq!(
            index.population, None,
            "absent means not measured, and the consumer must say nothing"
        );
    }
}
