// SPDX-License-Identifier: Apache-2.0
//! The signature meter.
//!
//! The only customer state Radar persists, per
//! [ADR 0006](https://github.com/hey-vera/radar/blob/main/docs/adr/0006-radar-records-only-what-it-cannot-recover.md).
//! It exists to answer one question that cannot be answered retroactively: how
//! many signatures does a customer actually consume in a month?
//!
//! That number decides whether Privy's pricing stays acceptable — 50,000
//! signatures are included and each one above costs a cent — and
//! [ADR 0005](https://github.com/hey-vera/radar/blob/main/docs/adr/0005-customers-keep-custody-and-grant-radar-a-bounded-signer.md)
//! made counting it a precondition for exactly that reason. Privy reports an
//! aggregate on a dashboard. It does not hand back a per-customer history for a
//! month in which nobody was counting.
//!
//! # Why it is also a cap, and not only a count
//!
//! Because a counter that only observes is a counter that watches an unbounded
//! signer run. The meter refuses past a daily ceiling, which makes a runaway
//! loop cost a bounded number of signatures instead of a bill — and rule 8's
//! direction applies: with no ceiling configured it refuses everything rather
//! than permitting everything.

use serde::{Deserialize, Serialize};

/// A customer, as the meter knows them.
///
/// A salted hash of the Privy DID, never the DID itself.
///
/// # Why hash something Radar sees anyway
///
/// Worth stating precisely, because it would otherwise look like theatre. Radar
/// *does* see the DID — it is in the verified token on every request, and the
/// hash hides nothing from the running system.
///
/// What it does is keep the identifier out of the **durable** artefact. The
/// meter's rows outlive the request by years and will be copied for research, and
/// a copied file that holds counts cannot be joined against a customer list,
/// while one that holds DIDs can. The threat is every future copy of the store,
/// not the live path.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct Subject(String);

/// Why a subject could not be formed.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum SubjectError {
    /// No salt is configured.
    ///
    /// Rule 8. An unsalted hash of a DID is a stable identifier that anyone
    /// holding a DID can recompute and look up, which is most of what the hash
    /// was for. Falling back to recording the raw DID would be worse than
    /// refusing, and falling back to an unsalted hash would be worse still — it
    /// *looks* protected.
    NoSalt,
    /// The salt is too short to be worth having.
    ///
    /// A short salt is brute-forceable against a known DID, so this refuses
    /// rather than accepting a value that would pass a glance.
    SaltTooShort {
        /// How many bytes were configured.
        given: usize,
        /// How many are required.
        needed: usize,
    },
    /// The identifier was empty.
    Empty,
}

/// The shortest salt accepted.
///
/// Thirty-two bytes, matching the digest it feeds. Shorter salts are refused
/// rather than stretched: stretching a weak salt produces a value that looks like
/// a strong one, and this is a place where looking right is the failure.
pub const MIN_SALT_BYTES: usize = 32;

impl Subject {
    /// Derives a subject from a DID and an instance salt.
    ///
    /// # Errors
    ///
    /// Returns [`SubjectError`] when there is no usable salt or no identifier.
    /// There is no variant that produces a subject anyway.
    pub fn derive(did: &str, salt: &[u8]) -> Result<Self, SubjectError> {
        if did.trim().is_empty() {
            return Err(SubjectError::Empty);
        }
        if salt.is_empty() {
            return Err(SubjectError::NoSalt);
        }
        if salt.len() < MIN_SALT_BYTES {
            return Err(SubjectError::SaltTooShort {
                given: salt.len(),
                needed: MIN_SALT_BYTES,
            });
        }
        // Keyed rather than `hash(salt || did)`. A keyed digest is the
        // construction meant for this, and concatenation is where length-
        // extension bugs come from even when the hash in use does not have one.
        let key = *blake3::hash(salt).as_bytes();
        Ok(Self(
            blake3::keyed_hash(&key, did.trim().as_bytes())
                .to_hex()
                .to_string(),
        ))
    }

    /// The hex digest, for writing.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What the meter would need to be rebuilt.
///
/// The durable shape. Deliberately three numbers and a subject: anything richer
/// is customer state, which ADR 0006 says Radar does not keep.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Reading {
    /// Who.
    pub subject: Subject,
    /// The accounting day this covers.
    pub day: u64,
    /// Signatures made on this subject's behalf on that day.
    pub today: u32,
    /// Signatures refused for want of allowance on that day.
    ///
    /// Recorded separately because a meter reading zero refusals and one reading
    /// nine hundred describe very different systems, and a count that folds them
    /// together cannot tell them apart.
    pub refused: u32,
}

/// The daily signature allowance for one customer.
///
/// No default, and none available. Rule 8: a meter with no ceiling loaded refuses
/// everything, because an unbounded signer is the thing invariant 1 exists to
/// prevent and "unconfigured" must not be the way to get one.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
pub struct Allowance(u32);

impl Allowance {
    /// Refuses everything. The correct value for an allowance that failed to
    /// load.
    pub const CLOSED: Self = Self(0);

    /// An allowance of `per_day` signatures.
    #[must_use]
    pub const fn per_day(per_day: u32) -> Self {
        Self(per_day)
    }

    /// The ceiling.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// One customer's signature meter for one day.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Meter {
    subject: Subject,
    allowance: Allowance,
    day: u64,
    today: u32,
    refused: u32,
}

impl Meter {
    /// A fresh meter for a subject on a day.
    #[must_use]
    pub const fn new(subject: Subject, allowance: Allowance, day: u64) -> Self {
        Self {
            subject,
            allowance,
            day,
            today: 0,
            refused: 0,
        }
    }

    /// Rebuilds a meter from a saved reading.
    ///
    /// A reading from an earlier day is **not** carried forward — the allowance
    /// is daily, so yesterday's consumption has no claim on today's. Restoring it
    /// as though it were today's would refuse everything until midnight, which is
    /// a different bug wearing this one's clothes.
    #[must_use]
    pub fn restore(reading: &Reading, allowance: Allowance, day: u64) -> Self {
        if reading.day != day {
            return Self::new(reading.subject.clone(), allowance, day);
        }
        Self {
            subject: reading.subject.clone(),
            allowance,
            day,
            today: reading.today,
            refused: reading.refused,
        }
    }

    /// Records a signature about to be made, or refuses it.
    ///
    /// Counts **before** the signature rather than after. A process that dies
    /// mid-call cannot know whether the signature happened, and a meter that
    /// counts afterwards undercounts exactly the calls that went wrong — which
    /// are the ones a runaway loop consists of.
    ///
    /// # Errors
    ///
    /// Returns the allowance when it is spent. The refusal is itself counted, so
    /// a meter that is refusing says so rather than merely sitting at its
    /// ceiling.
    pub fn charge(&mut self) -> Result<(), Allowance> {
        if self.today >= self.allowance.get() {
            self.refused = self.refused.saturating_add(1);
            return Err(self.allowance);
        }
        self.today = self.today.saturating_add(1);
        Ok(())
    }

    /// What has been consumed today.
    #[must_use]
    pub const fn today(&self) -> u32 {
        self.today
    }

    /// How many refusals today.
    #[must_use]
    pub const fn refused(&self) -> u32 {
        self.refused
    }

    /// What is left today.
    #[must_use]
    pub const fn remaining(&self) -> u32 {
        self.allowance.get().saturating_sub(self.today)
    }

    /// What this meter would need to be rebuilt.
    #[must_use]
    pub fn reading(&self) -> Reading {
        Reading {
            subject: self.subject.clone(),
            day: self.day,
            today: self.today,
            refused: self.refused,
        }
    }
}
