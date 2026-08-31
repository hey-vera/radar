// SPDX-License-Identifier: Apache-2.0
//! Durable meter state, so a budget survives a restart.
//!
//! # Why this exists
//!
//! Rule 8 says `radar-provider` implements "a [`Ledger`](radar_provider::Ledger)
//! that survives a restart, because a budget that forgets is not a budget and
//! `radar-serve` runs under `Restart=always`."
//!
//! The type could. Nothing made it. `Agent::restore` had exactly one caller and
//! it was a unit test; `main.rs` built the agent with `Agent::new` and a fresh
//! zero every time. So the daily model budget reset on every restart, and under
//! `Restart=always` a crash loop would have handed out a fresh day's allowance
//! per crash.
//!
//! It had cost nothing when it was found — `radar-serve` had zero unplanned
//! restarts — which is precisely why it was worth finding then rather than after
//! the first one. It is the third occurrence of the pattern
//! [`LEARNINGS`](https://github.com/hey-vera/radar/blob/main/LEARNINGS.md)
//! entries 1 and 9 record: a property documented more strongly than it is
//! enforced.
//!
//! # Why it refuses when it cannot write
//!
//! Rule 8's direction, applied to itself. A meter that cannot record what it
//! spent cannot enforce a ceiling across a restart, so a spender with no durable
//! ledger is an unmetered spender wearing a meter's clothes. With no state
//! directory configured, this refuses — the same shape as a missing
//! `RADAR_MODEL_DAILY_USD` meaning no agent rather than an unmetered one.

use std::path::{Path, PathBuf};

/// Why the ledger is unusable.
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum Unusable {
    /// No state directory is configured.
    #[error(
        "no durable ledger: set RADAR_STATE_DIR to a writable directory. A spend \
         meter that cannot record what it spent cannot enforce a ceiling across a \
         restart, so this refuses rather than spending unmetered."
    )]
    NotConfigured,
    /// The directory could not be created or written to.
    #[error("the state directory {path} is not writable: {why}")]
    NotWritable {
        /// Where it tried.
        path: String,
        /// What the filesystem said.
        why: String,
    },
}

/// The environment variable naming the durable state directory.
pub const STATE_DIR: &str = "RADAR_STATE_DIR";

/// A directory holding meter state.
#[derive(Clone, Debug)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    /// Opens the store named by the environment, proving it is writable.
    ///
    /// Writability is checked **now**, at startup, rather than discovered at the
    /// first write. A ledger that turns out to be unwritable an hour in has
    /// already let an hour of spend go unrecorded, and the operator finds out
    /// from a log line nobody was reading.
    ///
    /// # Errors
    ///
    /// [`Unusable`] when unconfigured or unwritable. There is no variant meaning
    /// "carry on without recording".
    pub fn open(get: &impl Fn(&str) -> Option<String>) -> Result<Self, Unusable> {
        let dir = get(STATE_DIR)
            .filter(|v| !v.trim().is_empty())
            .ok_or(Unusable::NotConfigured)?;
        Self::at(Path::new(dir.trim()))
    }

    /// Opens a store at a path, proving it is writable.
    ///
    /// # Errors
    ///
    /// [`Unusable::NotWritable`] when the directory cannot be created or written.
    pub fn at(dir: &Path) -> Result<Self, Unusable> {
        let unwritable = |why: std::io::Error| Unusable::NotWritable {
            path: dir.display().to_string(),
            why: why.to_string(),
        };
        std::fs::create_dir_all(dir).map_err(unwritable)?;

        // Actually write, rather than checking a permission bit. A read-only
        // mount, a full disk and a wrong owner all present differently to a
        // metadata check and identically to a failed write.
        //
        // The probe name is unique per call. A fixed one races: two openers of
        // the same directory each create it, and the first to finish deletes the
        // second's probe, so the second reports the directory unwritable when it
        // is fine. Found by the test suite running in parallel, which is the
        // cheap version of finding it from two `radar-serve` processes during a
        // deploy overlap.
        let probe = dir.join(format!(
            ".writable-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::write(&probe, b"radar").map_err(unwritable)?;
        std::fs::remove_file(&probe).map_err(unwritable)?;

        Ok(Self {
            dir: dir.to_path_buf(),
        })
    }

    /// Reads a named record, or `None` when it has never been written.
    ///
    /// A record that exists but cannot be parsed is **also** `None`, and that is
    /// deliberate: the alternative is refusing to start because of a corrupt
    /// counter, which trades a small overspend for a total outage. The
    /// conservative direction here is the opposite of rule 8's usual one, and it
    /// is worth saying why — an unreadable *budget* would be dangerous, but this
    /// is the record of what was already spent, and a service that will not boot
    /// spends nothing while also doing nothing.
    #[must_use]
    pub fn read<T: serde::de::DeserializeOwned>(&self, name: &str) -> Option<T> {
        let raw = std::fs::read_to_string(self.path(name)).ok()?;
        serde_json::from_str(&raw).ok()
    }

    /// Writes a named record, replacing any previous one.
    ///
    /// Written to a temporary file and renamed, so a process killed mid-write
    /// leaves the previous record intact rather than a truncated one. A
    /// half-written ledger that parses to a smaller number is an overspend that
    /// looks like a fresh start.
    ///
    /// # Errors
    ///
    /// [`Unusable::NotWritable`] when the write or the rename fails.
    pub fn write<T: serde::Serialize>(&self, name: &str, value: &T) -> Result<(), Unusable> {
        let unwritable = |why: String| Unusable::NotWritable {
            path: self.dir.display().to_string(),
            why,
        };
        let encoded = serde_json::to_vec(value).map_err(|e| unwritable(e.to_string()))?;
        let temporary = self.path(&format!("{name}.tmp"));
        std::fs::write(&temporary, &encoded).map_err(|e| unwritable(e.to_string()))?;
        std::fs::rename(&temporary, self.path(name)).map_err(|e| unwritable(e.to_string()))
    }

    /// Where a named record lives.
    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.json"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Serialize, serde::Deserialize, PartialEq, Eq, Debug)]
    struct Counted {
        day: u64,
        spent: u64,
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("radar-ledger-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn an_unconfigured_state_directory_refuses_rather_than_spending_unmetered() {
        // Rule 8, applied to the meter itself. This is the whole point of the
        // module: a spender with no durable ledger is an unmetered spender, and
        // "nobody set the variable" must not be the way to become one.
        let none = |_: &str| None;
        assert_eq!(Store::open(&none).err(), Some(Unusable::NotConfigured));

        let blank = |k: &str| (k == STATE_DIR).then(|| "   ".to_owned());
        assert_eq!(
            Store::open(&blank).err(),
            Some(Unusable::NotConfigured),
            "a blank value is not a configured directory"
        );
    }

    #[test]
    fn what_was_written_is_what_comes_back() {
        let dir = scratch("round-trip");
        let store = Store::at(&dir).expect("a writable scratch directory");
        let record = Counted {
            day: 20_331,
            spent: 1_450_000,
        };

        assert_eq!(
            store.read::<Counted>("model"),
            None,
            "nothing has been written yet"
        );
        store.write("model", &record).expect("writes");
        assert_eq!(store.read::<Counted>("model"), Some(record));
    }

    #[test]
    fn a_second_write_replaces_the_first() {
        // The property a restart depends on. A ledger that appended, or that
        // kept the first value, would restore yesterday's number forever.
        let dir = scratch("replace");
        let store = Store::at(&dir).expect("a writable scratch directory");
        store
            .write("model", &Counted { day: 1, spent: 10 })
            .expect("writes");
        store
            .write("model", &Counted { day: 1, spent: 99 })
            .expect("writes");
        assert_eq!(
            store.read::<Counted>("model"),
            Some(Counted { day: 1, spent: 99 })
        );
    }

    #[test]
    fn a_corrupt_record_reads_as_absent_rather_than_stopping_the_service() {
        // The one place the usual direction is inverted, and it is deliberate.
        // An unreadable *budget* would be dangerous and must refuse; this is the
        // record of what was already spent, and refusing to boot over a corrupt
        // counter trades a bounded overspend for a total outage.
        let dir = scratch("corrupt");
        let store = Store::at(&dir).expect("a writable scratch directory");
        std::fs::write(dir.join("model.json"), b"{ this is not json").expect("writes garbage");
        assert_eq!(store.read::<Counted>("model"), None);
    }

    #[test]
    fn a_directory_that_cannot_be_written_is_refused_at_startup() {
        // Discovered now rather than at the first write an hour in, by which
        // point an hour of spend has gone unrecorded and the operator learns
        // about it from a log line nobody was reading.
        //
        // A path whose parent is a *file* cannot be a directory on any platform,
        // which makes this the one unwritable case that behaves the same on
        // Windows and Linux -- permission bits do not.
        let dir = scratch("not-a-directory");
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        let blocker = dir.join("i-am-a-file");
        std::fs::write(&blocker, b"not a directory").expect("writes");

        assert!(
            matches!(
                Store::at(&blocker.join("under-a-file")),
                Err(Unusable::NotWritable { .. })
            ),
            "a directory under a file must be refused"
        );
    }

    #[test]
    fn two_openers_of_the_same_directory_do_not_report_it_unwritable() {
        // The race the probe name exists to avoid. A fixed probe filename means
        // the first opener to finish deletes the second's probe, and the second
        // concludes the directory is unwritable when it is fine.
        //
        // Found by the test suite running in parallel. The production version is
        // two `radar-serve` processes overlapping during a deploy, where the
        // consequence is the new one refusing to start with a message blaming
        // the filesystem.
        let dir = scratch("concurrent");
        std::fs::create_dir_all(&dir).expect("a scratch directory");

        let handles: Vec<_> = (0..8)
            .map(|_| {
                let dir = dir.clone();
                std::thread::spawn(move || Store::at(&dir).is_ok())
            })
            .collect();
        for handle in handles {
            assert!(
                handle.join().expect("the thread finished"),
                "a concurrent opener reported the directory unwritable"
            );
        }
    }

    #[test]
    fn separate_records_do_not_overwrite_each_other() {
        // The signature meter and the model ledger share a directory, and a
        // naming collision would have one silently restore the other's numbers.
        let dir = scratch("separate");
        let store = Store::at(&dir).expect("a writable scratch directory");
        store
            .write("model", &Counted { day: 1, spent: 10 })
            .expect("writes");
        store
            .write("signatures", &Counted { day: 1, spent: 77 })
            .expect("writes");

        assert_eq!(
            store.read::<Counted>("model"),
            Some(Counted { day: 1, spent: 10 })
        );
        assert_eq!(
            store.read::<Counted>("signatures"),
            Some(Counted { day: 1, spent: 77 })
        );
    }
}
