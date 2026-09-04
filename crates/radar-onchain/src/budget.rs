// SPDX-License-Identifier: Apache-2.0
//! The bounds a dossier is built inside.
//!
//! # Why this is a type and not three arguments
//!
//! Every read this crate performs is triggered by a stranger. The public
//! analyst answers an `@`-mention, and the mint in that mention is chosen by
//! whoever sent it — including someone who picked the most expensive token on
//! the chain on purpose. A creator with four hundred launches, a token with ten
//! thousand transactions in its launch slot, and an endpoint that has begun to
//! rate-limit are all the *normal* case for a public account, not the edge one.
//!
//! So the cost of answering has to be bounded before the question is read,
//! rather than discovered while answering it. [`Budget`] is that bound, it is
//! passed by value into the read path, and it is decremented by the client
//! itself rather than by its callers — a caller that forgets is the failure this
//! shape prevents.
//!
//! # Exhaustion is a fact, not an error
//!
//! Running out of budget does **not** fail the dossier. It truncates it, and the
//! truncation is reported: a recipient count that hit the cap comes back as
//! [`Count::AtLeast`] rather than as a number. AGENTS.md rule 9 — absent is not
//! zero, and unknown is not safe — is the whole of the reasoning. "Six
//! recipients" and "at least six recipients, we stopped counting" are different
//! claims, and publishing the first when only the second was measured is exactly
//! the kind of confident wrongness the account exists not to be.

use std::time::{Duration, Instant};

/// How many RPC calls one dossier may make.
///
/// Sized from the shape of the work rather than picked round: one signature
/// page, one transaction per signature in the launch slot, the curve account,
/// the fee config, and a creator lookup. Sixty is comfortably above a normal
/// token and well below anything that could be used as an amplifier.
pub const DEFAULT_MAX_CALLS: u32 = 60;

/// How many pages of `getSignaturesForAddress` one dossier may walk.
///
/// Each page is a thousand signatures. Three is enough to reach the launch of
/// any token that is still on the bonding curve; a token with more than three
/// thousand signatures has graduated or is being spammed, and in both cases the
/// answer is to say so rather than to keep paging.
pub const DEFAULT_MAX_PAGES: u32 = 3;

/// How long one dossier may take.
///
/// A reply that arrives after the thread is dead is not worth its cost — this is
/// the product constraint, not a safety one, and it is the reason the whole path
/// reads the chain rather than the store.
pub const DEFAULT_DEADLINE: Duration = Duration::from_secs(20);

/// What a bounded read ran out of.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Exhausted {
    /// The call allowance was spent.
    Calls,
    /// The page allowance was spent.
    Pages,
    /// The wall clock ran out.
    Deadline,
}

/// The bounds one dossier is built inside.
///
/// Not `Clone`, deliberately. A cloned budget is two budgets, which is a
/// doubled bill and an amplifier a stranger controls.
#[derive(Debug)]
pub struct Budget {
    calls_left: u32,
    pages_left: u32,
    started: Instant,
    deadline: Duration,
    calls_made: u32,
}

impl Default for Budget {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_CALLS, DEFAULT_MAX_PAGES, DEFAULT_DEADLINE)
    }
}

impl Budget {
    /// A budget with the given allowances.
    #[must_use]
    pub fn new(calls: u32, pages: u32, deadline: Duration) -> Self {
        Self {
            calls_left: calls,
            pages_left: pages,
            started: Instant::now(),
            deadline,
            calls_made: 0,
        }
    }

    /// Takes one call, or says why it cannot.
    ///
    /// The deadline is checked first: a budget with calls left but no time left
    /// should report the reason it actually stopped, because "we ran out of
    /// time" and "we ran out of allowance" call for different fixes.
    ///
    /// # Errors
    ///
    /// [`Exhausted`] when the deadline has passed or no call allowance remains.
    pub fn take_call(&mut self) -> Result<(), Exhausted> {
        if self.started.elapsed() >= self.deadline {
            return Err(Exhausted::Deadline);
        }
        if self.calls_left == 0 {
            return Err(Exhausted::Calls);
        }
        self.calls_left -= 1;
        self.calls_made += 1;
        Ok(())
    }

    /// Takes one page.
    ///
    /// Separate from [`Budget::take_call`] because a page is also a call, and
    /// the caller takes both: paging is the one operation whose cost is
    /// unbounded in the *number* of calls rather than in their size, so it
    /// carries its own ceiling as well as drawing on the shared one.
    ///
    /// # Errors
    ///
    /// [`Exhausted::Pages`] when the page allowance is spent.
    pub fn take_page(&mut self) -> Result<(), Exhausted> {
        if self.pages_left == 0 {
            return Err(Exhausted::Pages);
        }
        self.pages_left -= 1;
        Ok(())
    }

    /// How many calls have been made.
    ///
    /// Reported on the dossier so the cost of an answer is visible next to the
    /// answer. A figure nobody can see is a figure nobody notices doubling.
    #[must_use]
    pub const fn calls_made(&self) -> u32 {
        self.calls_made
    }

    /// How long the read has been running.
    #[must_use]
    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }
}

/// A count that may have been cut short.
///
/// The reason this is not a `u32` is AGENTS.md rule 9. A count that stopped at
/// its cap and a count that finished are different facts, and the type is what
/// stops the first being published as the second. `radar-graph` refuses on a
/// recipient count; a truncated count fed to a threshold is a refusal decided by
/// a budget rather than by the chain.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Count {
    /// Everything was counted.
    Exactly(u32),
    /// Counting stopped at the bound. The real number is this or larger.
    AtLeast(u32),
}

impl Count {
    /// The number, whether or not it is complete.
    ///
    /// Named to make a caller say what it is doing. Reaching for the number
    /// while discarding whether it was complete is the mistake this enum
    /// exists to make visible, so it is available but never implicit.
    #[must_use]
    pub const fn lower_bound(self) -> u32 {
        match self {
            Self::Exactly(n) | Self::AtLeast(n) => n,
        }
    }

    /// The number, only if it is known to be the whole of it.
    #[must_use]
    pub const fn exact(self) -> Option<u32> {
        match self {
            Self::Exactly(n) => Some(n),
            Self::AtLeast(_) => None,
        }
    }

    /// Whether counting was cut short.
    #[must_use]
    pub const fn is_truncated(self) -> bool {
        matches!(self, Self::AtLeast(_))
    }
}

impl std::fmt::Display for Count {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Exactly(n) => write!(f, "{n}"),
            Self::AtLeast(n) => write!(f, "at least {n}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_budget_stops_at_its_call_allowance() {
        let mut budget = Budget::new(2, 1, Duration::from_secs(60));
        assert!(budget.take_call().is_ok());
        assert!(budget.take_call().is_ok());
        assert_eq!(budget.take_call(), Err(Exhausted::Calls));
        // And it stays refused rather than recovering on the next attempt.
        assert_eq!(budget.take_call(), Err(Exhausted::Calls));
        assert_eq!(budget.calls_made(), 2);
    }

    #[test]
    fn a_budget_stops_at_its_page_allowance() {
        let mut budget = Budget::new(60, 2, Duration::from_secs(60));
        assert!(budget.take_page().is_ok());
        assert!(budget.take_page().is_ok());
        assert_eq!(budget.take_page(), Err(Exhausted::Pages));
    }

    #[test]
    fn a_spent_deadline_refuses_before_the_call_allowance_is_consulted() {
        // The order matters for the report, not just the refusal: a budget with
        // fifty calls left that stopped on time must not say it ran out of
        // calls, or the fix applied will be the wrong one.
        let mut budget = Budget::new(60, 3, Duration::ZERO);
        assert_eq!(budget.take_call(), Err(Exhausted::Deadline));
        assert_eq!(budget.calls_made(), 0);
    }

    #[test]
    fn elapsed_reports_real_time_rather_than_zero() {
        // `Budget::elapsed` -> Default::default() survived: nothing asserted the
        // value, and it is reported on every dossier as the cost of an answer.
        // A figure nobody checks is a figure nobody notices doubling.
        let budget = Budget::new(10, 3, Duration::from_secs(60));
        std::thread::sleep(Duration::from_millis(5));
        assert!(
            budget.elapsed() >= Duration::from_millis(5),
            "elapsed must measure something: {:?}",
            budget.elapsed()
        );
    }

    #[test]
    fn lower_bound_returns_the_count_it_was_given() {
        // Replacing this with 0 or 1 survived. It is what a threshold reads, so
        // a constant here would make every refusal a refusal about nothing.
        assert_eq!(Count::Exactly(6).lower_bound(), 6);
        assert_eq!(Count::AtLeast(11).lower_bound(), 11);
        assert_eq!(Count::Exactly(0).lower_bound(), 0);
        assert_eq!(Count::AtLeast(1).lower_bound(), 1);
    }

    #[test]
    fn a_truncated_count_never_presents_itself_as_exact() {
        // The whole reason `Count` exists. `lower_bound` is equal for both, so
        // a consumer reading only that cannot tell them apart -- which is why
        // `exact` is the one a threshold has to go through.
        let cut = Count::AtLeast(6);
        let whole = Count::Exactly(6);
        assert_eq!(cut.lower_bound(), whole.lower_bound());
        assert_eq!(cut.exact(), None);
        assert_eq!(whole.exact(), Some(6));
        assert!(cut.is_truncated());
        assert!(!whole.is_truncated());
        assert_eq!(cut.to_string(), "at least 6");
        assert_eq!(whole.to_string(), "6");
    }
}
