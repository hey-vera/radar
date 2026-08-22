// SPDX-License-Identifier: Apache-2.0
//! Where a fact came from, and how much weight it can carry.

use core::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::slot::Slot;

/// Identifies the source that produced a value: a provider lane, a local
/// decoder, or a purchased archive.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct SourceId(pub String);

impl SourceId {
    /// Values Radar decoded itself from a block it fetched.
    #[must_use]
    pub fn local_decode() -> Self {
        Self("local-decode".to_owned())
    }

    /// The underlying string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// How far a value is allowed to travel toward an action.
///
/// This is the prompt-injection boundary, expressed as a type rather than a
/// convention. External text — social posts, token metadata, website copy,
/// transaction memos — is [`Trust::Untrusted`] no matter how authoritative it
/// sounds, and no amount of it may become an instruction.
#[derive(
    Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, JsonSchema, PartialOrd, Ord,
)]
#[serde(rename_all = "snake_case")]
pub enum Trust {
    /// Derived by Radar from chain data it fetched and decoded itself. The only
    /// class that may drive a decision unaccompanied.
    Chain,
    /// A provider's assertion that Radar has not independently verified. Usable,
    /// but a disagreement with `Chain` is resolved in favour of `Chain`.
    Vendor,
    /// Content authored by someone outside the system. Storable, hashable,
    /// displayable, analysable as data. Never an instruction, and never on its
    /// own sufficient for an action.
    Untrusted,
}

/// How strongly the evidence behind a claim supports it.
///
/// Kept deliberately separate from [`Trust`]: a perfectly trustworthy source can
/// supply weak evidence, and an untrusted source can supply a fact that is
/// directly verifiable. Conflating the two is how a confident-sounding
/// conclusion gets built on nothing.
#[derive(
    Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, JsonSchema, PartialOrd, Ord,
)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceTier {
    /// Observed directly on chain: a tip transfer to a known Jito tip account in
    /// the same slot, a buy inside the create transaction.
    Direct,
    /// A strong inference from observed facts: common non-exchange funder within
    /// a short window, identical funding amounts across many wallets.
    Strong,
    /// A weak inference: timing correlation with no funding link, similar sizing.
    Weak,
    /// Looked for and not found. Distinct from "not looked for", which is absence
    /// of a claim rather than a claim of absence.
    Unknown,
    /// Two sources disagree. Never silently resolved — it surfaces.
    Conflicting,
}

impl EvidenceTier {
    /// Whether this tier may on its own justify sizing a position.
    ///
    /// Only [`Direct`](Self::Direct) and [`Strong`](Self::Strong) qualify, and
    /// even then only once the research store has shown the underlying signal
    /// predicts outcomes.
    #[must_use]
    pub const fn is_actionable(self) -> bool {
        matches!(self, Self::Direct | Self::Strong)
    }
}

/// The full provenance of a value: who said it, how much it can be trusted, how
/// strong the evidence is, and the slot it was true at.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Provenance {
    /// Which source produced the value.
    pub source: SourceId,
    /// How far the value may travel toward an action.
    pub trust: Trust,
    /// How strong the supporting evidence is.
    pub tier: EvidenceTier,
    /// The slot the value was true at. Every cached or replayed value carries
    /// this, which is what makes a replay comparable to the live run.
    pub as_of: Slot,
}

impl Provenance {
    /// Provenance for a value Radar decoded itself from chain data.
    #[must_use]
    pub fn decoded(as_of: Slot) -> Self {
        Self {
            source: SourceId::local_decode(),
            trust: Trust::Chain,
            tier: EvidenceTier::Direct,
            as_of,
        }
    }

    /// Provenance for external text. Always untrusted, never actionable alone.
    #[must_use]
    pub fn untrusted(source: SourceId, as_of: Slot) -> Self {
        Self {
            source,
            trust: Trust::Untrusted,
            tier: EvidenceTier::Weak,
            as_of,
        }
    }
}

impl fmt::Display for SourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_text_is_never_actionable_on_its_own() {
        let p = Provenance::untrusted(SourceId("x-api".to_owned()), Slot(100));
        assert_eq!(p.trust, Trust::Untrusted);
        assert!(!p.tier.is_actionable());
    }

    #[test]
    fn locally_decoded_chain_data_is_the_strongest_class() {
        let p = Provenance::decoded(Slot(100));
        assert_eq!(p.trust, Trust::Chain);
        assert!(p.tier.is_actionable());
    }

    #[test]
    fn trust_orders_from_most_to_least_trustworthy() {
        // Ord is derived, so the declaration order is load-bearing: code that
        // picks the best of several sources relies on Chain sorting first.
        assert!(Trust::Chain < Trust::Vendor);
        assert!(Trust::Vendor < Trust::Untrusted);
    }

    #[test]
    fn weak_and_unknown_and_conflicting_are_all_unactionable() {
        for t in [
            EvidenceTier::Weak,
            EvidenceTier::Unknown,
            EvidenceTier::Conflicting,
        ] {
            assert!(!t.is_actionable(), "{t:?} must not be actionable");
        }
    }
}
