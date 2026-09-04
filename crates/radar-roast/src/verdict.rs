// SPDX-License-Identifier: Apache-2.0
//! The verdict, and the reply that ships when the model cannot be used.
//!
//! # The verdict is a rule, not a judgement
//!
//! The model decides what the headline is, what matters and how to say it. It
//! does **not** decide the verdict, for the same reason `radar-risk` is a pure
//! function: a verdict computed by a rule is replayable, and a refusal that can
//! be reproduced from a recording is one you can argue about with evidence.
//!
//! It is deliberately not a score. `GOAL.md` refuses a single safety score --
//! *"Radar has fourteen reason codes and a structural split. A green shield is
//! 'unknown rendered as safe'"* -- and a one-bit verdict word is a score with
//! one bit. So [`Verdict`] carries **reasons**, and a reply renders the reasons
//! rather than the label.
//!
//! # The template is not a fallback, it is the floor
//!
//! Every path that cannot produce a trustworthy model reply ships
//! [`template`] instead: no budget configured, no provider, the provider
//! unreachable, a fabricated number, a forbidden claim. That is rule 8 --
//! **deny by default when config is missing** -- applied to speech rather than
//! to money: an analyst that cannot verify what it is about to say falls back
//! to saying only what it measured, never to saying nothing and never to saying
//! more.

use crate::sheet::FactSheet;
use std::fmt::Write as _;

/// What the rule concluded, as reasons rather than a score.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Verdict {
    /// Things measured about this token that a reader should know.
    ///
    /// Each is a statement of fact with its number already in the sheet. None
    /// of them is a recommendation, and the order is the order they are worth
    /// reading rather than a ranking of severity.
    pub reasons: Vec<String>,
}

impl Verdict {
    /// Computes the verdict from the sheet.
    ///
    /// Pure, and a function of the sheet alone. Deliberately: given the same
    /// facts this returns the same reasons on any machine at any time, which is
    /// what makes a published reply reproducible from its recorded fact sheet.
    #[must_use]
    pub fn from(sheet: &FactSheet) -> Self {
        let mut reasons = Vec::new();
        for fact in &sheet.facts {
            // The reasons are the facts, restated. This is not a simplification
            // waiting to be replaced by scoring: the product is "here is what
            // was measured", and a rule that weighted these into a conclusion
            // would be the single safety score GOAL.md refuses.
            reasons.push(format!("{}: {}", fact.label, fact.rendered));
        }
        for miss in &sheet.unknown {
            reasons.push(format!("not known -- {miss}"));
        }
        Self { reasons }
    }
}

/// Facts the template leads with, in order, matched by a fragment of their
/// label.
///
/// **Order is the product decision, not a formatting one.** The cost line
/// leads: "six recipients" is insidery, while "a $50 position pays 4.6% to get
/// in and out" is comprehensible to anyone, is measured, and is said nowhere
/// else. The bundle line is second.
///
/// Matched on a label fragment rather than an index because the sheet's
/// contents vary with what could be read, and a positional template would
/// silently print the wrong fact for a token whose curve was unreadable.
const LEAD: &[&str] = &[
    "round trip for a position of $20-$200",
    // The creator's record, second. Running the command against three real
    // launches on 2026-09-04 produced three **identical** replies: the cost
    // line is a constant and most launches sit in the same recipient band, so
    // nothing above or below this was about the coin being asked about.
    //
    // This is. "Forty-seven launches, none of which reached an AMM by filling
    // over time" is specific, checkable, and the thing Radar has that nobody
    // else does.
    "tokens this creator has launched",
    "how many reached an AMM by filling over time",
    "distinct token accounts receiving",
    "share of INSTANT graduations",
    "share of launches that NEVER graduated",
    "SOL that can be bought before price moves",
    "SOL the creator spent",
];

/// How many facts the template will print.
///
/// A reply is read at a glance and screenshotted, or it is not read. The full
/// sheet is what `radar roast --sheet` is for; this is what gets posted, and a
/// twenty-line dump would be posted by nobody.
///
/// Five rather than four since 2026-09-04, and the extra one is the creator's
/// record. Four was enough while every fact was about the block; it is not
/// enough now that two of the lines are the only ones that differ between one
/// coin and the next.
const MAX_FACTS: usize = 5;

/// The reply that ships when a model reply cannot be trusted or cannot be had.
///
/// Contains only figures from the sheet, so it passes
/// [`crate::fidelity::check`] against its own source by construction — and a
/// test asserts that rather than assuming it, because if the floor were itself
/// unpublishable there would be nothing left to fall back to.
#[must_use]
pub fn template(sheet: &FactSheet) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "Radar on {}:", sheet.mint);

    let mut shown = 0;
    for wanted in LEAD {
        if shown >= MAX_FACTS {
            break;
        }
        let Some(fact) = sheet
            .facts
            .iter()
            .find(|f| f.label.contains(wanted) && !f.rendered.is_empty())
        else {
            continue;
        };
        // The caveats live in the label and are restated in short form here
        // rather than dropped: "token accounts, not people" and "Radar's budget,
        // not the venue's ceiling" are the parts that keep the numbers honest,
        // and a reply that sheds them is a reply that says something else.
        let _ = writeln!(out, "- {}: {}", short(&fact.label), fact.rendered);
        shown += 1;
    }

    for miss in &sheet.unknown {
        // Said plainly, never rendered as reassurance and never by omission: a
        // reader not told something is missing assumes it was checked.
        let _ = writeln!(out, "- not known: {miss}");
    }
    if let Some(slot) = sheet.read_at {
        let _ = writeln!(out, "Read at slot {}.", slot.0);
    }
    out.push_str("Measured, not predicted. Not financial advice.\n");
    out
}

/// A label short enough to post, with its caveat intact.
fn short(label: &str) -> &str {
    match label {
        l if l.contains("distinct token accounts receiving") => {
            "token accounts in the launch block (accounts, not people)"
        }
        l if l.contains("share of INSTANT graduations") => {
            "share of instantly-graduating launches in that band"
        }
        l if l.contains("share of launches that NEVER graduated") => {
            "share of never-graduated launches in that band"
        }
        l if l.contains("SOL that can be bought before price moves") => {
            "SOL before 1% impact (Radar's budget, not the venue's ceiling)"
        }
        l if l.contains("SOL the creator spent") => "the creator's own buy",
        l if l.contains("round trip for a position of") => "round trip on a $20-$200 position",
        // The two lines that make one reply differ from the next, so they are
        // the two whose wording matters most. The sheet's labels are written to
        // be unambiguous to a model reading twenty of them; these are written to
        // be read once, by somebody deciding whether to buy.
        l if l.contains("tokens this creator has launched") => "tokens this creator has launched",
        l if l.contains("how many reached an AMM by filling over time") => {
            "of those, how many ever filled their curve over time"
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sheet::Fact;

    fn sheet() -> FactSheet {
        FactSheet {
            mint: "MintOne".to_owned(),
            read_at: Some(radar_types::Slot(444_007_820)),
            facts: vec![
                Fact::exact("recipients", 11.0, "11"),
                Fact::share("share of never-graduated in that band", 0.005),
            ],
            untrusted: vec![("token name".to_owned(), "Gay Pepe".to_owned())],
            unknown: vec!["the creator's launch count".to_owned()],
        }
    }

    #[test]
    fn a_fact_with_nothing_rendered_is_not_quoted() {
        // The selector is `label matches AND something was rendered`. With OR, a
        // fact whose value could not be read is printed with an empty value --
        // "- recipients: " -- which reads as a measurement of nothing rather
        // than as an absence. That is LEARNINGS 5's shape in a published reply.
        // The label has to be one `LEAD` looks for, or the selector never
        // reaches the second operand and the mutation is untested -- which is
        // how the first version of this passed while the mutant lived.
        let wanted = LEAD[0];
        let mut s = sheet();
        s.facts = vec![Fact {
            label: wanted.to_owned(),
            rendered: String::new(),
            values: vec![],
        }];
        s.unknown.clear();
        let out = template(&s);
        // Counted, not searched for by label: the template prints
        // `short(&fact.label)`, so looking for the full LEAD string in the
        // output cannot find the line even when it is there. The first version
        // of this test searched for it and passed while the mutant lived.
        let quoted = out.lines().filter(|l| l.starts_with("- ")).count();
        assert_eq!(quoted, 0, "an unrendered fact was quoted anyway:\n{out}");
    }

    #[test]
    fn the_reply_stops_at_the_fact_ceiling() {
        // `shown += 1` counts toward MAX_FACTS. Mutated to `*=` it stays at zero
        // for ever and the ceiling never binds, so a reply grows without limit
        // -- and the one place that shows up is a post that will not send.
        // The labels have to be ones `LEAD` actually looks for, or the loop
        // matches nothing and prints nothing -- which is how the first version
        // of this test passed while the mutant lived. LEAD carries six patterns
        // against a ceiling of four, so a sheet answering all six is the only
        // case where the ceiling is what stops a six-line post.
        assert!(
            LEAD.len() > MAX_FACTS,
            "the ceiling is unreachable if LEAD is no longer than it, and this test would prove nothing"
        );
        let mut s = sheet();
        s.facts = LEAD
            .iter()
            .map(|wanted| Fact::exact((*wanted).to_owned(), 11.0, "11"))
            .collect();
        // The unknowns are printed as `- ` lines too, and they are not what the
        // ceiling governs. Counting them made the first run of this read five
        // against four and look like a defect in the ceiling rather than in the
        // count.
        s.unknown.clear();
        let out = template(&s);
        let quoted = out.lines().filter(|l| l.starts_with("- ")).count();
        assert_eq!(
            quoted, MAX_FACTS,
            "{quoted} facts quoted against a ceiling of {MAX_FACTS}:\n{out}"
        );
    }

    #[test]
    fn the_template_passes_its_own_fidelity_check() {
        // The property that makes it a safe floor. If the template could
        // contain a number the sheet does not authorise, then the thing that
        // ships when a reply is rejected would itself be unpublishable -- and
        // there would be nothing left to fall back to.
        let sheet = sheet();
        let text = template(&sheet);
        let caught = crate::fidelity::check(&text, &sheet.authorised());
        assert!(caught.is_empty(), "{caught:?}");
    }

    #[test]
    fn the_template_passes_the_forbidden_check() {
        let text = template(&sheet());
        assert!(crate::forbidden::check(&text).is_empty());
    }

    #[test]
    fn the_template_says_what_is_unknown_rather_than_omitting_it() {
        // "Radar has no record" said plainly, never as reassurance and never by
        // silence -- a reader who is not told something is missing assumes it
        // was checked.
        let text = template(&sheet());
        assert!(text.contains("not known: the creator's launch count"));
    }

    #[test]
    fn the_template_carries_the_slot_every_figure_was_read_at() {
        assert!(template(&sheet()).contains("444007820"));
    }

    #[test]
    fn an_untrusted_name_never_reaches_the_template() {
        // The template is the trusted floor; a creator-controlled string in it
        // would be a creator writing part of Radar's reply.
        assert!(!template(&sheet()).contains("Gay Pepe"));
    }

    #[test]
    fn the_verdict_is_a_function_of_the_sheet_alone() {
        // Replayability: the same facts give the same reasons, which is what
        // lets a published reply be reproduced from its recorded fact sheet.
        assert_eq!(Verdict::from(&sheet()), Verdict::from(&sheet()));
        assert!(
            Verdict::from(&sheet())
                .reasons
                .iter()
                .any(|r| r.contains("11"))
        );
    }

    #[test]
    fn an_empty_sheet_still_produces_a_publishable_reply() {
        // The case a stranger can force: a mint nothing could be read about.
        // It must produce a reply that says so, not an empty string and not a
        // reply implying everything was fine.
        let empty = FactSheet {
            mint: "MintTwo".to_owned(),
            read_at: None,
            facts: Vec::new(),
            untrusted: Vec::new(),
            unknown: vec!["the launch block could not be read".to_owned()],
        };
        let text = template(&empty);
        assert!(text.contains("not known"));
        assert!(crate::forbidden::check(&text).is_empty());
        assert!(crate::fidelity::check(&text, &empty.authorised()).is_empty());
    }
}
