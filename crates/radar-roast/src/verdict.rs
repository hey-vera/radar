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
    // The creator's record first. Running the command against three real
    // launches on 2026-09-04 produced three **identical** replies: the cost
    // line is a constant and most launches sit in the same recipient band, so
    // nothing above or below this was about the coin being asked about.
    //
    // This is. "One hundred and fifty launches, none of which reached an AMM by
    // filling over time" is specific, checkable, and the thing Radar has that
    // nobody else does.
    "tokens this creator has launched",
    "how many reached an AMM by filling over time",
    // **Immediately after it, and this is the point of the ordering.** A count
    // with no denominator is a number the reader cannot weigh: "none of 150"
    // sounds damning to somebody who assumes half of them should have, and
    // unremarkable to somebody who assumes none ever do. Neither reader is
    // informed. The population says which.
    //
    // Measured 2026-09-04 over 506,991 outcomes: 2.81%.
    // **Before the population lines, and this cost a revision to get right.**
    // Those lines are constants too -- 2.8% and 23.0% in every reply -- so
    // leading with them just swaps one repeated opener for another. This is the
    // one fact about *this coin* that survives when the creator is unknown,
    // which for a fresh launch is the common case.
    //
    // The honest shape of the limitation: for a brand-new coin by a creator
    // Radar has never seen, the launch block is nearly all it has. That is a
    // fact about the product, not about the wording, and the template should
    // show it rather than pad around it.
    "distinct token accounts receiving",
    "of every measured launch, how many graduated at all",
    // The most quotable figure in the set, and one the published snapshot does
    // not carry at all: 23.0% of measured launches show almost no life.
    "of every measured launch, how many showed almost no activity at all",
    "share of INSTANT graduations",
    "share of launches that NEVER graduated",
    "SOL that can be bought before price moves",
    "SOL the creator spent",
    // **The round trip is deliberately NOT here.** It led every reply until
    // 2026-09-05, and it is the same 456 bps every time, so every reply opened
    // with the same sentence -- an account that reads as a bot repeating itself
    // rather than as something that looked at the coin.
    //
    // It is too useful to drop, so it is printed as a closing line instead: last,
    // always, and out of competition for the five slots above.
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

/// The one sentence Radar prints if the model says nothing useful.
///
/// Built from the sheet's own facts and nothing else, so it passes
/// [`crate::fidelity::check`] against `sheet.authorised()` by construction --
/// which is what lets it be handed to the model as a starting line rather than
/// as a suggestion the checks would later refuse.
///
/// It exists because three real launches on 2026-09-04 produced three
/// **identical** replies. The cost line is a constant and most launches sit in
/// the same recipient band, so the model had no anchor that was about the coin
/// in front of it. This is that anchor, and it is chosen the same way [`LEAD`]
/// orders facts: the creator's record if there is one, otherwise the launch
/// block.
///
/// `None` when the sheet has neither. **An unknown creator is not a creator
/// with zero launches** -- rule 9 -- so there is nothing to lead with and the
/// model is given no headline rather than a misleading one.
///
/// Under a hundred characters, because the first sentence is what gets
/// screenshotted without the rest.
#[must_use]
pub fn headline(sheet: &FactSheet) -> Option<String> {
    let rendered = |wanted: &str| -> Option<&str> {
        sheet
            .facts
            .iter()
            .find(|f| f.label.contains(wanted) && !f.rendered.is_empty())
            .map(|f| f.rendered.as_str())
    };

    // The creator's record: a count beside the count it should be weighed
    // against, which is the whole shape the voice is asked for.
    if let (Some(launched), Some(filled)) = (
        rendered("tokens this creator has launched"),
        rendered("how many reached an AMM by filling over time"),
    ) {
        let line = format!("{launched} launches by this creator. {filled} ever filled a curve.");
        if line.chars().count() <= 100 {
            return Some(line);
        }
    }

    // Otherwise the launch block, which is about this coin even when the
    // creator is new.
    // Matched on the label `sheet.rs` actually emits -- "distinct token accounts
    // receiving the token in its own launch block (token accounts, NOT owners,
    // NOT people)". A first version matched a phrase that appears in `short`'s
    // rendering rather than in the label, so this branch silently never fired
    // and the fallback test caught it.
    if let Some(recipients) = rendered("receiving the token in its own launch block") {
        let line = format!("{recipients} token accounts were paid in the launch block.");
        if line.chars().count() <= 100 {
            return Some(line);
        }
    }

    None
}

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
    // The headline, when there is one: the same anchor the model is offered,
    // so the floor and the voice pass lead on the same fact rather than on
    // whichever fact happened to sort first.
    if let Some(headline) = headline(sheet) {
        let _ = writeln!(out, "{headline}");
    }

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
    // The cost, always, and always last. It is the same number in every reply --
    // that is why it is not competing for a slot above -- but it is also the one
    // figure that applies to the reader no matter what the rest of the reply
    // said, so dropping it to make room would be dropping the only line that is
    // about them rather than about the coin.
    //
    // Drawn from the sheet rather than written as a constant here, so
    // `fidelity::check` still holds: a number in the reply that is not on the
    // sheet is exactly what that check refuses, and hard-coding 456 would make
    // this function the one place allowed to invent one.
    //
    // **The band qualifier is load-bearing.** The sheet carries one of these per
    // notional band and the cheapest one is first, so matching on "round trip
    // for a position of" alone finds `$0.20-$2` -- 3042 bps -- and publishes a
    // cost 6.7x the real one on every reply. Written without it, run against
    // three live coins, and caught by reading the output.
    if let Some(cost) = sheet.facts.iter().find(|f| {
        f.label.contains("round trip for a position of $20-$200") && !f.rendered.is_empty()
    }) {
        let _ = writeln!(
            out,
            "Entering and leaving a $20-$200 position: {}.",
            cost.rendered
        );
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
        // The population lines. Written as a comparison rather than as a
        // statistic, because the reader is holding the creator's count two lines
        // above and the sentence has to connect the two for them.
        l if l.contains("how many graduated at all") => {
            "across every launch Radar has measured, how many graduated at all"
        }
        l if l.contains("how many showed almost no activity at all") => {
            "and how many showed almost no activity at all"
        }
        l if l.contains("how many filled their curve over time") => {
            "across every launch Radar has measured, how many filled over time"
        }
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
    use crate::sheet::{About, Fact};

    /// A sheet shaped like the real ones the box produced on 2026-09-04: a
    /// creator with a long record, the population beside it, the launch block,
    /// and the cost.
    fn a_real_shaped_sheet() -> FactSheet {
        FactSheet {
            mint: "ECQdbWN1jBAQ9GXGFxX9gqvoa6NT3weWe4SCpAaapump".to_owned(),
            read_at: Some(radar_types::Slot(444_388_986)),
            facts: vec![
                // **Every band, in the order the snapshot lists them.** A
                // fixture carrying only the wanted one cannot catch a lookup
                // that finds the wrong band -- and that is exactly what
                // happened: written with one band, it passed, and against a
                // real sheet it published $0.20-$2's 3042 bps.
                Fact {
                    about: About::Measurement,
                    label: "round trip for a position of $0.20-$2".to_owned(),
                    rendered: "3042 bps (30.4%)".to_owned(),
                    values: vec![3042.0, 30.4],
                },
                Fact {
                    about: About::Measurement,
                    label: "round trip for a position of $2-$20".to_owned(),
                    rendered: "250 bps (2.5%)".to_owned(),
                    values: vec![250.0, 2.5],
                },
                Fact {
                    about: About::Measurement,
                    label: "round trip for a position of $20-$200".to_owned(),
                    rendered: "456 bps (4.6%)".to_owned(),
                    values: vec![456.0, 4.6],
                },
                Fact::exact("tokens this creator has launched", 150.0, "150"),
                Fact::exact("how many reached an AMM by filling over time", 0.0, "0"),
                Fact::share(
                    "of every measured launch, how many graduated at all",
                    0.0281,
                ),
                Fact::exact(
                    "distinct token accounts receiving the token in its own launch block",
                    4.0,
                    "4",
                ),
                Fact::share(
                    "of every measured launch, how many showed almost no activity at all",
                    0.230,
                ),
            ],
            untrusted: vec![("token name".to_owned(), "GOAT".to_owned())],
            unknown: Vec::new(),
            signals: Vec::new(),
        }
    }

    #[test]
    fn the_headline_is_publishable_by_construction() {
        // It is handed to the model as a starting line and printed by the
        // floor, so it has to pass the same two checks every reply passes --
        // and it passes them by being built only out of the sheet, not by
        // being careful.
        let sheet = a_real_shaped_sheet();
        let headline = headline(&sheet).expect("a sheet with a creator record has one");
        assert!(
            crate::fidelity::check(&headline, &sheet.authorised()).is_empty(),
            "{:?}",
            crate::fidelity::check(&headline, &sheet.authorised())
        );
        assert!(
            crate::forbidden::check(&headline).is_empty(),
            "{:?}",
            crate::forbidden::check(&headline)
        );
        assert!(
            headline.chars().count() <= 100,
            "{} chars: {headline}",
            headline.chars().count()
        );
    }

    #[test]
    fn an_unknown_creator_gets_no_headline_rather_than_a_zero() {
        // Rule 9, and the direction that matters. A creator Radar has never
        // seen has NO record, not a record of zero launches -- and "0 launches
        // by this creator" would be a damning sentence invented out of an
        // absence. With no creator record and no launch block there is nothing
        // about this coin to lead with, so nothing is offered.
        let mut sheet = a_real_shaped_sheet();
        sheet.facts.retain(|f| {
            !f.label.contains("tokens this creator has launched")
                && !f
                    .label
                    .contains("how many reached an AMM by filling over time")
                && !f
                    .label
                    .contains("receiving the token in its own launch block")
        });
        assert_eq!(headline(&sheet), None);
        // And the floor still renders, without an empty line where the
        // headline would have been.
        let out = template(&sheet);
        assert!(
            !out.contains(
                "

"
            ),
            "a blank line was left behind: {out:?}"
        );
    }

    #[test]
    fn the_headline_falls_back_to_the_launch_block_when_the_creator_is_new() {
        let mut sheet = a_real_shaped_sheet();
        sheet.facts.retain(|f| {
            !f.label.contains("tokens this creator has launched")
                && !f
                    .label
                    .contains("how many reached an AMM by filling over time")
        });
        let headline = headline(&sheet).expect("the launch block is still about this coin");
        assert!(headline.contains("launch block"), "{headline}");
        assert!(crate::fidelity::check(&headline, &sheet.authorised()).is_empty());
    }

    #[test]
    fn the_reply_does_not_open_with_the_line_that_is_the_same_every_time() {
        // The defect this ordering fixes. Until 2026-09-05 the round trip led
        // every reply, and it is 456 bps in all of them -- so three different
        // coins opened with the same sentence, which reads as a bot repeating
        // itself rather than as something that looked at the coin.
        let out = template(&a_real_shaped_sheet());
        let first = out.lines().nth(1).expect("a first line after the header");
        // Line 1 is now the headline rather than the first bullet, and it is
        // still about the creator's record -- which is the property this test
        // was always about. The shape changed; the claim did not.
        assert!(
            first.contains("launches by this creator"),
            "the first line must be about this coin, got: {first}"
        );
        assert!(
            !first.contains("round trip"),
            "the constant must not lead: {first}"
        );
    }

    #[test]
    fn the_creators_record_is_followed_by_what_it_should_be_weighed_against() {
        // A count with no denominator is a number the reader cannot use. "None
        // of 150" sounds damning to somebody who assumes half should have, and
        // unremarkable to somebody who assumes none ever do -- neither of them
        // is informed, and the population is what decides.
        //
        // This was measured and then not published: the figures landed in the
        // sheet on 2026-09-04 and never reached a reply, because LEAD is a fixed
        // whitelist and they were not on it.
        let out = template(&a_real_shaped_sheet());
        let launched = out.find("creator has launched").expect("the count");
        let population = out
            .find("across every launch Radar has measured")
            .expect("the denominator must be published");
        assert!(
            population > launched,
            "the population must come after the count it explains:
{out}"
        );
        // And in the same short list, not pushed off the end of it by the five
        // slot cap -- a denominator the reader never sees is one that was not
        // published.
        assert!(
            out.lines()
                .take(6)
                .any(|l| l.contains("across every launch")),
            "it must survive the cap:
{out}"
        );
    }

    #[test]
    fn the_cost_is_always_said_and_always_last() {
        // Out of competition for the five slots, because it is the same in every
        // reply -- but never dropped, because it is the only line that is about
        // the reader rather than about the coin.
        let out = template(&a_real_shaped_sheet());
        let cost = out
            .find("Entering and leaving")
            .expect("the cost line must survive being demoted");
        let last_fact = out.rfind("- ").expect("fact lines");
        assert!(
            cost > last_fact,
            "it must come after the facts:
{out}"
        );
        assert!(
            out.contains("456 bps"),
            "with its figure:
{out}"
        );
        assert!(
            !out.contains("3042"),
            "the cheapest band is listed first and must not be the one found:
{out}"
        );
    }

    #[test]
    fn a_sheet_with_no_cost_simply_has_no_cost_line() {
        // Rule 8. The snapshot may be absent, and inventing 456 here would make
        // this function the one place in the reply path allowed to publish a
        // number that is not on the sheet -- which is precisely what
        // `fidelity::check` refuses everywhere else.
        let mut sheet = a_real_shaped_sheet();
        sheet.facts.retain(|f| !f.label.contains("round trip"));
        let out = template(&sheet);
        assert!(!out.contains("Entering and leaving"), "{out}");
        assert!(!out.contains("456"), "{out}");
    }

    #[test]
    fn the_token_name_never_reaches_the_reply() {
        // It is the most obvious "improvement" to make -- "Radar on GOAT:" reads
        // far better than a base58 string -- and it is attacker-controlled text
        // (rule 4). A coin named "Radar says BUY" would have the bot publish
        // that. The mint cannot be spoofed; the name can, so it stays fenced in
        // `untrusted` and out of everything that gets posted.
        let out = template(&a_real_shaped_sheet());
        assert!(!out.contains("GOAT"), "{out}");
    }

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
            signals: Vec::new(),
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
            about: About::Measurement,
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
            signals: Vec::new(),
        };
        let text = template(&empty);
        assert!(text.contains("not known"));
        assert!(crate::forbidden::check(&text).is_empty());
        assert!(crate::fidelity::check(&text, &empty.authorised()).is_empty());
    }
}
