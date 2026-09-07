// SPDX-License-Identifier: Apache-2.0
//! Domain vocabulary for Radar.
//!
//! This crate holds types and nothing else: no I/O, no network, no clock. Every
//! other crate in the workspace depends on it, so anything that can fail at
//! runtime belongs somewhere further out.
//!
//! Three ideas here carry more weight than the rest and are worth reading before
//! the code that uses them:
//!
//! - [`Slot`] is the only clock Radar has. Wall-clock time is never the basis of
//!   a decision, because a chain does not advance on wall-clock time and a
//!   replay of a decision must land on the same inputs the live run saw.
//! - [`Mutability`] is declared per fact, not per provider. It is what makes
//!   "never fetch this twice" a property the cache can enforce rather than a
//!   convention a caller has to remember.
//! - [`MicroUsd`] is integer money. Costs are summed across millions of calls and
//!   compared against hard budget caps; floating point has no place in that path.

#![forbid(unsafe_code)]

/// Base64, because three crates need it and one of them must not depend on the
/// signer to get it.
///
/// It lived in `radar-signer` and was used from `radar-serve`, which meant the
/// crate holding `Key::sign` was compiled into the internet-facing process. Rule
/// 1's "no network, no listener" was true of the signer *binary* and not of the
/// signer *crate*, and the only thing the web server ever wanted from it was
/// this.
pub mod b64;
pub mod civil;

mod address;
mod money;
mod mutability;
mod provenance;
mod slot;

/// The commit this binary was built from, or `None`.
///
/// # The trap this closes, which caught us twice
///
/// A binary on the box and a binary in the repository look identical from the
/// outside. Twice a change was made, believed deployed, and debugged for an
/// hour against a running process that predated it -- once because the install
/// was never run and once because the unit was never restarted. Neither state
/// is visible from `systemctl status`, which happily reports a five-day-old
/// process as active.
///
/// `option_env!` rather than `env!`: **an ordinary `cargo build` has no
/// `RADAR_BUILD_SHA`**, and a workspace that refused to compile without one set
/// would be a check that fires on every developer, every time -- the exact
/// shape §5 says to delete rather than tune. `None` here means "built outside
/// CI", which is true and is what the callers print.
///
/// Release CI sets it from `github.sha`. Nothing else does, deliberately: a
/// value a local build could invent would be a claim about provenance that
/// provenance did not make.
#[must_use]
pub const fn build_sha() -> Option<&'static str> {
    option_env!("RADAR_BUILD_SHA")
}

/// The same, rendered for a line of output.
///
/// `"unknown"` rather than an empty string or a plausible-looking hash: an
/// operator reading this needs to be able to tell "I cannot say" apart from a
/// commit, and a blank reads as neither.
#[must_use]
pub fn build_sha_or_unknown() -> &'static str {
    build_sha().unwrap_or("unknown")
}

pub use address::{Address, AddressParseError, Signature};
pub use money::MicroUsd;
pub use mutability::{Latch, LatchReopened, Mutability, Revalidation};
pub use provenance::{EvidenceTier, Provenance, SourceId, Trust};
pub use slot::{Slot, SlotDelta};

#[cfg(test)]
mod build_tests {
    use super::{build_sha, build_sha_or_unknown};

    #[test]
    fn a_build_outside_release_ci_says_unknown_rather_than_a_blank_or_a_hash() {
        // Every test build is a build outside release CI: nothing sets
        // `RADAR_BUILD_SHA` except the one workflow step that produces the
        // artifact. So this is the value an operator sees on a hand-built
        // binary, and it has to be a word rather than an empty string -- a
        // blank reads as neither "I cannot say" nor a commit.
        //
        // CI turned `build_sha` into `Some("")` and `Some("xyzzy")` and
        // `build_sha_or_unknown` into `""` and `"xyzzy"`, and all four
        // survived. Re-apply any of them: this fails.
        assert_eq!(build_sha(), None, "a test build has no RADAR_BUILD_SHA");
        assert_eq!(build_sha_or_unknown(), "unknown");
        assert!(!build_sha_or_unknown().is_empty());
    }
}
