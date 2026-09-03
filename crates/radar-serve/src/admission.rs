// SPDX-License-Identifier: Apache-2.0
//! Which verified customers this instance lets in.
//!
//! # Why a verified token is not enough
//!
//! [`customer::verify`](crate::customer::verify) proves that Privy issued a
//! token, for this application, unexpired. That is **authentication**, and the
//! module that does it says so in as many words. It is not a statement that the
//! holder is a customer of *this* instance, and while the product is private it
//! very often will not be: anyone can sign up to a Privy application.
//!
//! So there is a second question — *may this identity in?* — and until now
//! nothing asked it. The instance is meant to be private, and it was private
//! only because no customer authenticator was configured at all.
//!
//! # Why the allowlist holds DIDs and not the email you asked for
//!
//! Privy's access token carries `iss`, `aud`, `sub`, `sid` and `exp`. There is
//! no email in it. Email lives on the *user* object, behind a Privy API call —
//! which would put a network round trip on every request and make logging in
//! depend on Privy being reachable, on a path that otherwise does not need it.
//!
//! The DID is in the token, already signature-checked, and it is stable. So the
//! allowlist matches on `sub`. Same outcome, and it asks nothing of a vendor
//! that is not already proven.
//!
//! # Rule 8, and why going public is an edit rather than a deletion
//!
//! Unset means **closed**. It would be more convenient for "no allowlist" to
//! mean "everyone", and that is exactly the arrangement where a dropped
//! environment variable silently opens a private product. Going public is
//! `RADAR_CUSTOMER_ACCESS=open`, which someone has to type.

use std::collections::BTreeSet;

/// Which verified customers may reach the product.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Admission {
    /// Nobody. The shipped state, and what an absent configuration means.
    Closed,
    /// Only these Privy DIDs.
    Allowlist(BTreeSet<String>),
    /// Anyone whose token verifies.
    ///
    /// The public product. Entitlement — a paid subscription — is a separate
    /// question this type does not answer, and the two must not be conflated:
    /// this decides who may *reach* the product, and billing decides who may
    /// use it.
    Open,
}

/// The variable that configures this.
pub const VAR: &str = "RADAR_CUSTOMER_ACCESS";

impl Admission {
    /// Reads the policy from the environment.
    ///
    /// - unset or empty → [`Self::Closed`]
    /// - `open` → [`Self::Open`]
    /// - `allowlist:did:privy:a,did:privy:b` → [`Self::Allowlist`]
    ///
    /// # Errors
    ///
    /// Returns a message for anything else, including an `allowlist:` with no
    /// identities after it. Both are refusals rather than quiet closures: a
    /// misconfiguration that resolves to "closed" is indistinguishable from a
    /// deliberate one, and an operator would spend the outage looking at Privy.
    pub fn from_vars(get: &impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let Some(raw) = get(VAR) else {
            return Ok(Self::Closed);
        };
        let value = raw.trim();
        if value.is_empty() {
            return Ok(Self::Closed);
        }
        if value == "open" {
            return Ok(Self::Open);
        }
        if value == "closed" {
            return Ok(Self::Closed);
        }
        let Some(list) = value.strip_prefix("allowlist:") else {
            return Err(format!(
                "{VAR} must be `closed`, `open`, or `allowlist:<did>,<did>` — got {value:?}"
            ));
        };
        let dids: BTreeSet<String> = list
            .split(',')
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .map(str::to_owned)
            .collect();
        if dids.is_empty() {
            // An empty allowlist and `closed` mean the same thing, and that is
            // precisely why this is an error. Somebody who wrote `allowlist:`
            // meant to put an identity after it.
            return Err(format!("{VAR} names an allowlist with nobody in it"));
        }
        Ok(Self::Allowlist(dids))
    }

    /// Whether this identity may reach the product.
    ///
    /// Compared whole and case-sensitively. A DID is an opaque identifier from a
    /// vendor, not a name, and normalising one is inventing a rule about a
    /// format Radar does not own.
    #[must_use]
    pub fn admits(&self, did: &str) -> bool {
        match self {
            Self::Closed => false,
            Self::Open => true,
            Self::Allowlist(dids) => dids.contains(did),
        }
    }

    /// How an operator should see this at start.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Closed => format!("closed — no customer may reach the product (set {VAR})"),
            Self::Open => "open — any verified Privy identity".to_owned(),
            Self::Allowlist(dids) => format!("allowlist of {}", dids.len()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn var(value: &str) -> impl Fn(&str) -> Option<String> + '_ {
        move |k: &str| (k == VAR).then(|| value.to_owned())
    }

    #[test]
    fn an_absent_setting_admits_nobody() {
        // The whole point. If absent meant "everyone", a dropped environment
        // variable on one deploy would silently open a private product, and
        // nothing would look wrong.
        let none = |_: &str| None;
        assert_eq!(Admission::from_vars(&none), Ok(Admission::Closed));
        assert!(!Admission::Closed.admits("did:privy:anyone"));

        assert_eq!(Admission::from_vars(&var("   ")), Ok(Admission::Closed));
    }

    #[test]
    fn going_public_is_something_someone_has_to_type() {
        assert_eq!(Admission::from_vars(&var("open")), Ok(Admission::Open));
        assert!(Admission::Open.admits("did:privy:a-stranger"));
    }

    #[test]
    fn an_allowlist_admits_exactly_who_it_names() {
        let policy = Admission::from_vars(&var("allowlist:did:privy:josh,did:privy:second"))
            .expect("parses");
        assert!(policy.admits("did:privy:josh"));
        assert!(policy.admits("did:privy:second"));
        assert!(!policy.admits("did:privy:someone-else"));
        // Not a prefix match, and not a substring one. Either would admit an
        // identity nobody listed.
        assert!(!policy.admits("did:privy:josh2"));
        assert!(!policy.admits("josh"));
    }

    #[test]
    fn whitespace_around_an_identity_is_forgiven_but_case_is_not() {
        let policy =
            Admission::from_vars(&var("allowlist: did:privy:josh , did:privy:b ")).expect("parses");
        assert!(policy.admits("did:privy:josh"));
        // A DID is an opaque vendor identifier. Case-folding it invents a rule
        // about a format Radar does not own, and the failure would be silent in
        // the permissive direction.
        assert!(!policy.admits("DID:PRIVY:JOSH"));
    }

    #[test]
    fn an_empty_allowlist_is_an_error_rather_than_a_quiet_closure() {
        // It would behave identically to `closed`, and that is exactly why it
        // must not be accepted: somebody who typed `allowlist:` meant to put an
        // identity after it, and a silent closure sends them to Privy's
        // dashboard to debug their own env file.
        assert!(Admission::from_vars(&var("allowlist:")).is_err());
        assert!(Admission::from_vars(&var("allowlist: , ")).is_err());
    }

    #[test]
    fn an_unrecognised_setting_is_refused_rather_than_guessed_at() {
        for bad in ["everyone", "true", "1", "allow", "OPEN", "did:privy:josh"] {
            assert!(Admission::from_vars(&var(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn closed_can_also_be_said_out_loud() {
        // So an operator turning the product off writes what they mean rather
        // than deleting a line and trusting the default.
        assert_eq!(Admission::from_vars(&var("closed")), Ok(Admission::Closed));
    }

    #[test]
    fn the_description_says_which_state_it_is_in() {
        // `radar brief` and the start-up banner both read this, and a line that
        // cannot distinguish open from closed is the failure LEARNINGS records
        // repeatedly.
        assert!(Admission::Closed.describe().contains("closed"));
        assert!(Admission::Open.describe().contains("open"));
        let list = Admission::from_vars(&var("allowlist:did:privy:a")).expect("parses");
        assert!(list.describe().contains('1'));
    }
}
