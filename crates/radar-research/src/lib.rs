// SPDX-License-Identifier: Apache-2.0
//! Replay: re-running a recorded decision at its original watermark.
//!
//! The plan states this as a one-line CI test — *replay a recorded live decision
//! at its original `as_of`; any divergence is a leak or a non-determinism bug*.
//! That is nearly right, and the missing case is the one that actually occurred.
//!
//! # Divergence has three causes, not two
//!
//! A replay compares two things independently: the **inputs** the decision was
//! made from, and the **decision** itself. Which of them moved says what
//! happened, and the three answers want different responses.
//!
//! **The decision changed while the inputs did not.** The strategy is not a pure
//! function of its inputs — it read a clock, an environment variable, iteration
//! order, or an address's hash. This is always a bug, it is always in the code,
//! and it is the one that belongs in CI. [`Verdict::NotDeterministic`].
//!
//! **The inputs changed.** The store returned different data for the same
//! watermark than it did when the decision was recorded. This is *not*
//! automatically a leak, and treating it as one would be a false alarm on a
//! correct system: an append-only store that is still backfilling legitimately
//! knows more about slot N tomorrow than it did today. On 2026-08-24 a repair
//! added 1,740 graduation events with historical slots to the live store
//! ([research 0006](../../docs/research/0006-the-graduation-table-was-empty-for-a-structural-reason.md)),
//! and every replay across that boundary would have screamed "leak" at an
//! operation that was fixing the data. [`Verdict::InputsChanged`] reports it as
//! what it is: a fact about the store's provenance that a human has to classify.
//!
//! **Neither changed.** [`Verdict::Identical`], the only passing result.
//!
//! # What a leak actually looks like
//!
//! The genuine leak — a read at watermark N returning something observed after N
//! — is caught by [`AsOf`] at the point of the read, not here. This harness
//! catches the failure one level up: that the *whole lane* is reproducible, so a
//! backtest over it means something. Both are necessary and neither substitutes
//! for the other.
//!
//! # Why digests rather than stored inputs
//!
//! A recording holds a hash of its inputs, not a copy of them. A copy would make
//! the recording the source of truth, and then a replay would be checking the
//! recording against itself rather than checking the store still says what it
//! said. The digest can only answer "the same or not", which is the only
//! question that keeps the store as the thing under test.

#![forbid(unsafe_code)]

pub mod basis;
pub mod selection;
pub mod study;

use radar_asof::AsOf;
use radar_strategy::{Candidate, Strategy};
use radar_types::{Address, Slot};
use serde::{Deserialize, Serialize};

/// A blake3 digest, rendered as hex.
///
/// Hex rather than bytes because a recording is meant to be read by a person
/// diffing two runs, and a base64 blob is not.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Digest(pub String);

impl Digest {
    /// Digests any serialisable value through its JSON encoding.
    ///
    /// JSON rather than a hand-written field-by-field encoding on purpose: a
    /// field added to [`Candidate`] enters the digest automatically. A manual
    /// encoding would silently keep hashing the old shape, so a new input could
    /// start moving decisions while every replay still reported them identical
    /// — a check that cannot see a whole class of change is worse than none,
    /// because it is trusted.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayError::Encoding`] if the value cannot be serialised.
    pub fn of<T: Serialize>(value: &T) -> Result<Self, ReplayError> {
        let bytes = serde_json::to_vec(value).map_err(|e| ReplayError::Encoding(e.to_string()))?;
        Ok(Self(blake3::hash(&bytes).to_hex().to_string()))
    }

    /// The first eight characters, for reports where the full hash is noise.
    #[must_use]
    pub fn short(&self) -> &str {
        let end = self
            .0
            .char_indices()
            .nth(8)
            .map_or(self.0.len(), |(i, _)| i);
        &self.0[..end]
    }
}

/// Something went wrong recording or replaying.
#[derive(Debug, thiserror::Error, PartialEq, Eq, Clone)]
pub enum ReplayError {
    /// A value could not be encoded for hashing.
    #[error("cannot encode for digest: {0}")]
    Encoding(String),
    /// The candidate this recording describes is no longer in the store.
    ///
    /// Distinct from a changed input. A launch that has vanished from an
    /// append-only store is a much louder fact than one whose record grew.
    #[error("mint {mint} is not in the store as of slot {as_of}")]
    CandidateGone {
        /// The token the recording is about.
        mint: Address,
        /// The watermark it was recorded at.
        as_of: Slot,
    },
}

/// One recorded decision, with everything needed to check it reproduces.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Recording {
    /// The strategy that decided.
    pub strategy: String,
    /// Its version at the time. A changed decision under a changed version is
    /// expected; under the same version it is a bug.
    pub strategy_version: String,
    /// The watermark the decision was made at.
    pub as_of: Slot,
    /// The token.
    pub mint: Address,
    /// Digest of the candidate the decision was made from.
    pub inputs_digest: Digest,
    /// Digest of the decision itself.
    pub decision_digest: Digest,
    /// The decision, rendered for a human reading a diff.
    ///
    /// Never compared — the digest is what is compared. This exists so that a
    /// divergence report can say *what* the two answers were rather than only
    /// that they differed.
    pub decision: serde_json::Value,
}

/// What a replay found.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum Verdict {
    /// Same inputs, same decision. The only passing result.
    Identical,
    /// The store answers differently for the same watermark than it used to.
    ///
    /// Legitimate while a backfill is still filling in history, and a serious
    /// problem once a slot range is supposed to be settled. The harness cannot
    /// tell which — that depends on what was being done to the store — so it
    /// reports the fact and refuses to classify it.
    InputsChanged {
        /// What the inputs hashed to when recorded.
        was: Digest,
        /// What they hash to now.
        now: Digest,
        /// Whether the decision moved as well.
        ///
        /// `false` is the reassuring case: the store gained data that did not
        /// change the answer.
        decision_moved: bool,
    },
    /// Same inputs, different decision. Always a bug in the code.
    NotDeterministic {
        /// What was decided when recorded.
        was: serde_json::Value,
        /// What is decided now.
        now: serde_json::Value,
    },
}

impl Verdict {
    /// Whether this verdict should fail a build.
    ///
    /// Only [`Self::NotDeterministic`]. An input change is a fact about the
    /// data, and failing CI on it would mean every backfill broke the build —
    /// which trains people to ignore the check, so it would be worse than not
    /// having one.
    #[must_use]
    pub const fn is_failure(&self) -> bool {
        matches!(self, Self::NotDeterministic { .. })
    }

    /// Whether this verdict needs a human to look at it.
    #[must_use]
    pub const fn needs_review(&self) -> bool {
        !matches!(self, Self::Identical)
    }
}

/// Records what a strategy decided about a candidate.
///
/// # Errors
///
/// Returns [`ReplayError::Encoding`] if the candidate or decision cannot be
/// serialised for hashing.
pub fn record<S: Strategy>(strategy: &S, candidate: &Candidate) -> Result<Recording, ReplayError> {
    let decision = strategy.consider(candidate);
    let decision_value =
        serde_json::to_value(&decision).map_err(|e| ReplayError::Encoding(e.to_string()))?;

    Ok(Recording {
        strategy: strategy.name().to_owned(),
        strategy_version: strategy.version().to_owned(),
        as_of: candidate.as_of.slot(),
        mint: candidate.mint,
        inputs_digest: Digest::of(candidate)?,
        decision_digest: Digest::of(&decision)?,
        decision: decision_value,
    })
}

/// Re-runs a recording against a freshly assembled candidate.
///
/// The caller assembles the candidate from the store at
/// [`Recording::as_of`] — this function deliberately does not do that itself, so
/// that the assembly under test is the same code the live lane uses rather than
/// a copy of it that could drift.
///
/// # Errors
///
/// Returns [`ReplayError::Encoding`] if the fresh candidate or decision cannot
/// be serialised.
pub fn replay<S: Strategy>(
    recording: &Recording,
    strategy: &S,
    candidate: &Candidate,
) -> Result<Verdict, ReplayError> {
    debug_assert_eq!(
        candidate.as_of.slot(),
        recording.as_of,
        "a replay at a different watermark is not a replay"
    );

    let inputs_now = Digest::of(candidate)?;
    let decision_now = strategy.consider(candidate);
    let decision_digest_now = Digest::of(&decision_now)?;
    let decision_moved = decision_digest_now != recording.decision_digest;

    if inputs_now != recording.inputs_digest {
        return Ok(Verdict::InputsChanged {
            was: recording.inputs_digest.clone(),
            now: inputs_now,
            decision_moved,
        });
    }

    if decision_moved {
        return Ok(Verdict::NotDeterministic {
            was: recording.decision.clone(),
            now: serde_json::to_value(&decision_now)
                .map_err(|e| ReplayError::Encoding(e.to_string()))?,
        });
    }

    Ok(Verdict::Identical)
}

/// The watermark a recording must be replayed at.
///
/// A convenience so callers cannot accidentally replay at the store's *current*
/// watermark, which would compare a decision against a different question and
/// report the difference as non-determinism.
#[must_use]
pub const fn watermark_for(recording: &Recording) -> AsOf {
    AsOf::at(recording.as_of)
}

/// A summary over many replays.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize)]
pub struct Summary {
    /// Recordings that reproduced exactly.
    pub identical: u64,
    /// Recordings whose inputs moved.
    pub inputs_changed: u64,
    /// Recordings whose inputs held but whose decision moved.
    pub not_deterministic: u64,
    /// Recordings whose candidate is no longer in the store.
    pub candidate_gone: u64,
}

impl Summary {
    /// Counts one verdict.
    pub const fn count(&mut self, verdict: &Verdict) {
        match verdict {
            Verdict::Identical => self.identical += 1,
            Verdict::InputsChanged { .. } => self.inputs_changed += 1,
            Verdict::NotDeterministic { .. } => self.not_deterministic += 1,
        }
    }

    /// Counts a recording whose candidate could not be rebuilt.
    pub const fn count_gone(&mut self) {
        self.candidate_gone += 1;
    }

    /// Total recordings replayed.
    #[must_use]
    pub const fn total(&self) -> u64 {
        self.identical + self.inputs_changed + self.not_deterministic + self.candidate_gone
    }

    /// Whether anything here should fail a build.
    ///
    /// A missing candidate counts. An append-only store losing a launch it
    /// already had is not a provenance nuance, it is data loss.
    #[must_use]
    pub const fn is_failure(&self) -> bool {
        self.not_deterministic > 0 || self.candidate_gone > 0
    }
}
