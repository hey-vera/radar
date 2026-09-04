// SPDX-License-Identifier: Apache-2.0
//! The fact sheet: every number the analyst is allowed to say.
//!
//! # This type is the security boundary
//!
//! The model is given this and nothing else, and afterwards every numeric
//! literal in what it wrote is checked back against it. So the set of numbers
//! reachable from here **is** the set of numbers that can be published, and a
//! field added here is a claim authorised.
//!
//! That is the same shape as `radar-signer`'s `verify::check`, which re-decodes
//! the bytes to confirm they match the authorisation rather than trusting the
//! caller's description of them. *The signer re-reads the bytes it signs; the
//! roaster re-reads the numbers it posts.*
//!
//! # Why the numbers are enumerated rather than inferred
//!
//! [`FactSheet::authorised`] lists every value a reply may contain, in every
//! form it may take — a share appears both as its ratio and as its percentage,
//! because a model told "0.251" will reasonably write "25%". Enumerating is
//! deliberate: the alternative is a checker that tries to guess which
//! transformations of a fact are legitimate, and a checker that guesses is one
//! that can be argued into accepting a number nobody measured.

use radar_onchain::budget::Count;
use radar_onchain::{Dossier, LaunchBlock};
use radar_types::Slot;

use crate::baserates::BaseRates;
use std::fmt::Write as _;

/// Lamports in one SOL.
const LAMPORTS_PER_SOL: u64 = 1_000_000_000;

/// One publishable number, with the words that make it a claim.
#[derive(Clone, Debug, PartialEq)]
pub struct Fact {
    /// What it is, in the fact sheet the model reads.
    pub label: String,
    /// How it renders.
    pub rendered: String,
    /// Every numeric value this fact authorises.
    ///
    /// More than one because a single measurement has several honest
    /// renderings: 0.251, 25.1 and 25 are the same fact said three ways, and a
    /// model that picks a different one has not invented anything.
    pub values: Vec<f64>,
}

impl Fact {
    /// A fact whose only value is the one in its rendering.
    #[must_use]
    pub fn exact(label: impl Into<String>, value: f64, rendered: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            rendered: rendered.into(),
            values: vec![value],
        }
    }

    /// A share, authorised as a ratio, a percentage, and the percentage rounded.
    ///
    /// The rounded form is included because a reply that says "a quarter of
    /// them" or "25%" for 25.1% is being *readable*, not inventing. The check
    /// exists to stop fabrication, and a tolerance narrow enough to forbid
    /// ordinary rounding would push every reply to the deterministic template.
    #[must_use]
    pub fn share(label: impl Into<String>, ratio: f64) -> Self {
        let pct = ratio * 100.0;
        // Precision follows the magnitude, and this is not cosmetic. The
        // strongest finding in 0024 is that launches with one to three
        // recipients graduate instantly **0.02%** of the time; at one decimal
        // place that renders as "0.0%", which reads as *never* rather than as
        // *rare*. A reply that says a thing never happens when it happens two
        // times in twelve thousand is wrong in the direction that gets quoted
        // back at you.
        let rendered = if pct > 0.0 && pct < 0.1 {
            format!("{pct:.2}%")
        } else {
            format!("{pct:.1}%")
        };
        Self {
            label: label.into(),
            rendered,
            values: vec![
                ratio,
                pct,
                pct.round(),
                (pct * 10.0).round() / 10.0,
                (pct * 100.0).round() / 100.0,
            ],
        }
    }
}

/// Everything the analyst may assert about one token.
#[derive(Clone, Debug)]
pub struct FactSheet {
    /// The mint, as text. Not a number, and never checked as one.
    pub mint: String,
    /// The slot every figure was read at.
    pub read_at: Option<Slot>,
    /// The facts, in the order they are shown to the model.
    pub facts: Vec<Fact>,
    /// Creator-supplied strings, kept apart from the facts.
    ///
    /// **Never inlined into the fact list**, because the fact list is what the
    /// model is told is true. These are fenced separately as untrusted, and a
    /// number appearing inside one of them authorises nothing.
    pub untrusted: Vec<(String, String)>,
    /// What could not be read, so the reply can say so plainly.
    ///
    /// "Radar has no record" is a thing the analyst is expected to say, and it
    /// can only say it if the absence survives to here rather than becoming a
    /// default somewhere below.
    pub unknown: Vec<String>,
}

impl FactSheet {
    /// Builds the sheet from a dossier and the published base rates.
    ///
    /// `rates` is `None` when the snapshot could not be loaded. That is not a
    /// reason to fall back on remembered numbers: without it the sheet simply
    /// carries no population context, and the reply says less. Rule 8 — a
    /// missing input is a refusal to claim, not a default.
    #[must_use]
    pub fn build(
        dossier: &Dossier,
        rates: Option<&BaseRates>,
        creators: Option<&crate::creator::CreatorIndex>,
    ) -> Self {
        let mut facts = Vec::new();
        let mut untrusted = Vec::new();
        let mut unknown = Vec::new();

        if let Some(launch) = &dossier.launch {
            push_launch(&mut facts, &mut untrusted, launch);
            if let Some(rates) = rates {
                push_population(&mut facts, launch.recipients, rates);
            }
            // **The fact that makes one reply differ from another.** Everything
            // above is about the block; three coins launched in the same minute
            // produce the same sentences, because the cost line is a constant
            // and most launches sit in the same recipient band. What this
            // creator did before is the part that is about *this* coin, and it
            // is the thing Radar has that nobody else does.
            if let Some(index) = creators {
                push_creator(&mut facts, &mut unknown, &launch.creator.to_string(), index);
            }
        } else {
            unknown.push("the launch block could not be read".to_owned());
        }

        if let Some(curve) = &dossier.curve {
            push_curve(&mut facts, curve);
        } else {
            unknown.push("the bonding curve could not be read".to_owned());
        }

        if let Some(count) = dossier.creator_transactions {
            facts.push(Fact::exact(
                "transactions by this creator's address (transactions, not launches)",
                f64::from(count.lower_bound()),
                format!("{count}"),
            ));
        }

        // Not inside the launch-block arm above, and deliberately: this is a
        // fact about the venue, not about the coin, so a mint whose launch block
        // could not be read still gets it. It is also what gives the creator's
        // counts a scale -- "none of 150 filled its curve" reads differently
        // once you know what share of everything does.
        if let Some(population) = creators.and_then(|c| c.population) {
            push_measured_population(&mut facts, &population);
        }

        if let Some(rates) = rates {
            push_cost(&mut facts, rates);
        }

        for miss in &dossier.unavailable {
            // **Radar's own phrase, never the raw reason.** `miss.why` is
            // diagnostic text -- "rpc transport: http status: 429", "no account
            // at <mint>" -- and two things are wrong with publishing it.
            //
            // It is an injection surface: a reason that echoes the mint would
            // put the attacker's own base58 into the trusted block, and
            // `authorised` reads numerals out of that block, so a mint chosen to
            // contain "68" would licence 68 as a publishable figure.
            //
            // And it is bad copy. A reader asking about a coin is owed "the
            // launch block could not be read", not an HTTP status. The raw
            // reason stays on the `Dossier` for the operator, where it belongs.
            unknown.push(phrase_for(miss.fact));
        }

        // **Said once, not twice.** Every unreadable fact reaches this list by
        // two routes: the field is `None`, and `Dossier::build` also recorded a
        // reason for it in `unavailable`. Both fire for the same failure, so a
        // reply about a token whose launch block could not be read told the
        // reader so twice, and a token where nothing could be read said four
        // lines that were two.
        //
        // Found by running the thing against a real mint, which is the only
        // place it shows: every fixture in this crate's tests supplies one route
        // or the other, never both, so the duplication was invisible to all of
        // them.
        //
        // Deduplicated rather than removing one route. Keeping both is what
        // guarantees an absent fact is always reported -- if `build` ever stops
        // recording a reason, the `None` branch still speaks, and rule 9 says an
        // absence must never pass silently. Order is preserved because it is the
        // order the reader meets the facts in.
        let mut seen = std::collections::BTreeSet::new();
        unknown.retain(|miss| seen.insert(miss.clone()));

        Self {
            mint: dossier.mint.to_string(),
            read_at: dossier.read_at,
            facts,
            untrusted,
            unknown,
        }
    }

    /// Every numeric value a reply may contain.
    ///
    /// Three sources, and the boundary between them is the point:
    ///
    /// 1. **Each fact's declared values**, which carry the honest re-renderings
    ///    a measurement has — 0.251, 25.1 and 25 are one fact said three ways.
    /// 2. **Every numeral in the trusted rendering**, labels included. A label
    ///    says things like "research 0022" and "$20-$200", and those numerals
    ///    were written *by Radar* and shown to the model as true. A model citing
    ///    the band it was given has invented nothing, and a check that caught it
    ///    would reject the most careful replies while passing vaguer ones.
    /// 3. **The slot**, because a reply citing the slot it was read at is doing
    ///    the thing this account exists to do.
    ///
    /// What is **not** a source is [`FactSheet::untrusted`]. That is the whole
    /// boundary: a creator who names their token "99.9% of holders profited"
    /// must not thereby licence 99.9 as a publishable figure. The untrusted
    /// strings are fenced separately and never rendered into this block.
    #[must_use]
    pub fn authorised(&self) -> Vec<f64> {
        let mut values: Vec<f64> = self.facts.iter().flat_map(|f| f.values.clone()).collect();
        values.extend(
            crate::fidelity::literals(&self.render())
                .into_iter()
                .map(|(_, v)| v),
        );
        if let Some(slot) = self.read_at {
            #[expect(
                clippy::cast_precision_loss,
                reason = "a slot is well inside f64's exact integer range and this is a \
                          comparison against a literal the model wrote, not arithmetic"
            )]
            values.push(slot.0 as f64);
        }
        values
    }

    /// The sheet as the model sees it.
    ///
    /// Facts only. The mint, the slot, and the untrusted strings are fenced
    /// separately by [`crate::voice`] so that nothing in this block is
    /// creator-controlled.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        for fact in &self.facts {
            let _ = writeln!(out, "{}: {}", fact.label, fact.rendered);
        }
        for miss in &self.unknown {
            let _ = writeln!(out, "NOT KNOWN: {miss}");
        }
        out
    }
}

/// Radar's own words for a fact it could not read.
///
/// A closed set, so nothing outside this file can put text into the trusted
/// block. An unrecognised fact name gets a generic phrase rather than its raw
/// reason -- the fallback has to be the safe one, because the case it covers is
/// a fact added later by someone who did not read this comment.
fn phrase_for(fact: &str) -> String {
    match fact {
        "launch block" => "the launch block could not be read",
        "curve" => "the bonding curve could not be read",
        "creator history" => "the creator's history could not be read",
        _ => "part of this could not be read",
    }
    .to_owned()
}

fn push_launch(facts: &mut Vec<Fact>, untrusted: &mut Vec<(String, String)>, launch: &LaunchBlock) {
    facts.push(Fact::exact(
        "distinct token accounts receiving the token in its own launch block \
         (token accounts, NOT owners, NOT people)",
        f64::from(launch.recipients.lower_bound()),
        format!("{}", launch.recipients),
    ));
    facts.push(Fact::exact(
        "transactions in the launch block",
        f64::from(launch.transactions.lower_bound()),
        format!("{}", launch.transactions),
    ));
    match launch.dev_buy_lamports {
        Some(l) => facts.push(Fact::exact(
            "SOL the creator spent buying their own token in the launch block",
            lamports_as_sol(l),
            format!("{} SOL", render_sol(l)),
        )),
        // Rule 9, and this one is a statement about a person: "did not buy" and
        // "we could not see a buy" are different accusations.
        None => facts.push(Fact {
            label: "creator's own buy in the launch block".to_owned(),
            rendered: "not found -- absent, NOT zero. Do not say the creator bought nothing."
                .to_owned(),
            values: Vec::new(),
        }),
    }
    untrusted.push(("token name".to_owned(), launch.metadata.name.clone()));
    untrusted.push(("token symbol".to_owned(), launch.metadata.symbol.clone()));
}

/// What this creator's other tokens did.
///
/// # Counts, never a rate
///
/// "Nine of forty-one" and "22%" say the same thing to an arithmetician and
/// different things to a reader: the share hides the denominator, and the
/// denominator is the part that decides whether the number means anything.
/// `creator_track_record` computes rates with a minimum sample and a note
/// explaining itself; this publishes what was counted and lets the reader do
/// the division.
///
/// # Absent is not innocent
///
/// A creator the index has never seen launched before Radar was watching. That
/// is said plainly, because a reply that omitted the line would read as a clean
/// record — rule 9 in the direction that flatters, which is the one that gets
/// somebody hurt.
///
/// # Never presented as a good sign
///
/// Research 0011: graduation predicts **volatility, not profit**. Organic
/// graduations end at a median −3,228 bps against −853 for tokens that never
/// graduate. So the graduation count is published as a measurement and the
/// label never suggests it is encouraging.
fn push_creator(
    facts: &mut Vec<Fact>,
    unknown: &mut Vec<String>,
    creator: &str,
    index: &crate::creator::CreatorIndex,
) {
    let Some(record) = index.get(creator) else {
        // One line, no continuation. A `\` continuation in a Rust string keeps
        // the *leading* whitespace of the next line, so this rendered with a
        // run of fourteen spaces in the middle of a published sentence -- which
        // is the sort of thing that looks like a broken bot rather than a
        // careful one.
        unknown.push(
            "this creator has no record here: Radar has been watching since August, so they launched before that, or have not launched again"
                .to_owned(),
        );
        return;
    };

    facts.push(Fact::exact(
        "tokens this creator has launched, in Radar's record",
        f64::from(record.launches),
        record.launches.to_string(),
    ));

    // The denominator, always beside the numerator. A gap between launches and
    // measured means the outcome pass has not caught up -- not that those
    // tokens did nothing -- and a share quoted without it would be a share of
    // an unstated population.
    if record.measured == 0 {
        unknown.push("how those launches turned out: none has been measured yet".to_owned());
        return;
    }
    facts.push(Fact::exact(
        "of those, how many have been measured",
        f64::from(record.measured),
        record.measured.to_string(),
    ));
    facts.push(Fact::exact(
        "of the measured, how many reached an AMM by filling over time",
        f64::from(record.organic),
        record.organic.to_string(),
    ));
    facts.push(Fact::exact(
        "of the measured, how many filled their curve within three slots (capital committed before the token existed, not demand)",
        f64::from(record.instant),
        record.instant.to_string(),
    ));
    facts.push(Fact::exact(
        "of the measured, how many showed almost no activity at all",
        f64::from(record.stillborn),
        record.stillborn.to_string(),
    ));
}

/// The population as **Radar itself measured it**, from the store.
///
/// # Why this is beside the snapshot rather than instead of it
///
/// `push_population` places one coin's recipient count in a distribution that
/// came from outside: a public RPC walking 45 slots, and a SQL endpoint that
/// truncates at a thousand rows. That distribution is the only one available,
/// because the store did not record a launch-block recipient count until
/// ADR 0012 and only rows written after 2026-09-03 carry one.
///
/// The graduation rates are different. Radar has every succeeded launch it ever
/// recorded and every outcome it ever measured, so it can count them rather than
/// sample them — and the creator-index timer already does, every six hours, in
/// the same pass. On these figures the store is the better instrument, and using
/// the sampled ones when the counted ones are on disk would be a choice to be
/// less accurate.
///
/// # The denominator is stated, always
///
/// Every share here is over `measured`, and `measured` is printed beside them.
/// The gap between what was launched and what was measured is Radar's own
/// backlog, and a share quoted without its denominator invites the reader to
/// treat a lag as a finding.
fn push_measured_population(facts: &mut Vec<Fact>, population: &crate::creator::Population) {
    // Rule 9 in one branch: nothing measured is not a population of zeroes. Say
    // that the figure is missing, in Radar's own words, rather than publishing
    // "0% of launches graduate" off an empty denominator.
    let Some(graduated) = population.graduated_share() else {
        facts.push(Fact {
            label: "how the venue as a whole turns out".to_owned(),
            rendered: "NOT AVAILABLE -- no outcome has been measured yet".to_owned(),
            values: Vec::new(),
        });
        return;
    };
    facts.push(Fact::exact(
        "launches Radar has recorded and measured, which every share below is out of",
        // Lossless below 2^53; these are counts of launches.
        #[expect(
            clippy::cast_precision_loss,
            reason = "counts of launches; 2^53 is six orders of magnitude away"
        )]
        {
            population.measured as f64
        },
        population.measured.to_string(),
    ));
    facts.push(Fact::share(
        "of every measured launch, how many graduated at all",
        graduated,
    ));
    if let Some(organic) = population.organic_share() {
        facts.push(Fact::share(
            "of every measured launch, how many filled their curve over time",
            organic,
        ));
    }
    if let Some(instant) = population.instant_share() {
        facts.push(Fact::share(
            "of every measured launch, how many filled inside their own launch block",
            instant,
        ));
    }
    if let Some(stillborn) = population.stillborn_share() {
        facts.push(Fact::share(
            "of every measured launch, how many showed almost no activity at all",
            stillborn,
        ));
    }
}

fn push_population(facts: &mut Vec<Fact>, recipients: Count, rates: &BaseRates) {
    // A truncated count must not be looked up in a distribution: the band it
    // lands in would be decided by Radar's call budget rather than by the chain.
    let Some(exact) = recipients.exact() else {
        facts.push(Fact {
            label: "population context for the recipient count".to_owned(),
            rendered: "NOT AVAILABLE -- the count was cut short, so it cannot be \
                       placed in a distribution"
                .to_owned(),
            values: Vec::new(),
        });
        return;
    };
    let Some(band) = rates.band_for(exact) else {
        return;
    };
    facts.push(Fact::share(
        format!(
            "share of launches that NEVER graduated whose block had {} recipients ({})",
            exact, band.name
        ),
        band.never_graduated,
    ));
    facts.push(Fact::share(
        format!("share of ORGANIC graduations in that band ({})", band.name),
        band.organic,
    ));
    facts.push(Fact::share(
        format!("share of INSTANT graduations in that band ({})", band.name),
        band.instant,
    ));
    facts.push(Fact::share(
        format!(
            "probability a launch in that band ({}) graduates instantly",
            band.name
        ),
        band.p_instant,
    ));
    facts.push(Fact::exact(
        format!("how many times the base rate that is ({} band)", band.name),
        band.x_base_instant,
        format!("{:.1}x", band.x_base_instant),
    ));
    facts.push(Fact::share(
        "population rate: share of all launches that graduate instantly",
        rates.base_rate_instant,
    ));
    facts.push(Fact::share(
        "population rate: share of all launches that graduate at all",
        rates.base_rate_graduates,
    ));
}

fn push_curve(facts: &mut Vec<Fact>, curve: &radar_onchain::CurveFacts) {
    facts.push(Fact {
        label: "has the token graduated off the bonding curve".to_owned(),
        rendered: if curve.complete { "yes" } else { "no" }.to_owned(),
        values: Vec::new(),
    });
    match curve.capacity_lamports {
        Some(l) => facts.push(Fact::exact(
            "SOL that can be bought before price moves 1% -- this is RADAR'S OWN \
             impact budget, NOT a ceiling the venue imposes (research 0022)",
            lamports_as_sol(l),
            format!("{} SOL", render_sol(l)),
        )),
        None => facts.push(Fact {
            label: "exit capacity".to_owned(),
            rendered: "none -- cannot size into this at all. NOT 'no limit found'.".to_owned(),
            values: Vec::new(),
        }),
    }
    if let Some(fees) = &curve.fees {
        #[expect(
            clippy::cast_precision_loss,
            reason = "a basis-point figure is a small integer; this is a comparison value"
        )]
        let rt = fees.round_trip_bps() as f64;
        facts.push(Fact {
            label: "venue fee, round trip, read from the on-chain schedule".to_owned(),
            rendered: format!(
                "{rt} bps -- THE VENUE FEE ONLY. The measured all-in round trip is 850 bps. \
                 Never present the fee as the cost of trading."
            ),
            values: vec![rt, rt / 100.0],
        });
    }
}

fn push_cost(facts: &mut Vec<Fact>, rates: &BaseRates) {
    facts.push(Fact::exact(
        "measured all-in round trip Radar's kernel assumes, on fresh launches",
        rates.round_trip_kernel,
        format!("{} bps", rates.round_trip_kernel),
    ));
    facts.push(Fact::exact(
        "expected edge a strategy must clear before one trade is worth making",
        rates.round_trip_bar,
        format!("{} bps", rates.round_trip_bar),
    ));
    for band in &rates.cost_bands {
        facts.push(Fact {
            label: format!("round trip for a position of {}", band.band),
            rendered: format!("{} bps ({:.1}%)", band.round_trip, band.round_trip / 100.0),
            values: vec![band.round_trip, band.round_trip / 100.0],
        });
    }
}

#[expect(
    clippy::cast_precision_loss,
    reason = "lamports are converted only for a comparison against a literal the model \
              wrote; the rendered figure comes from integer arithmetic in `render_sol`"
)]
fn lamports_as_sol(lamports: u64) -> f64 {
    lamports as f64 / LAMPORTS_PER_SOL as f64
}

/// Renders lamports as SOL by integer arithmetic.
///
/// `radar-types` keeps money integral on purpose, and a printed figure that has
/// silently rounded through a float is exactly what this account must not
/// publish.
fn render_sol(lamports: u64) -> String {
    format!(
        "{}.{:04}",
        lamports / LAMPORTS_PER_SOL,
        (lamports % LAMPORTS_PER_SOL) / 100_000
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_precision_switch_is_exactly_where_0024_needs_it() {
        // The rule is `> 0.0 && < 0.1` renders two decimals, everything else
        // one. Both comparisons are one character from being wrong and every
        // mutation of them survived, because nothing tested either edge.
        //
        // What is at stake is 0024's strongest finding: one to three recipients
        // graduate instantly 0.02% of the time. At one decimal that is "0.0%",
        // which reads as *never* rather than *rare* -- wrong in the direction
        // that gets quoted back at you.
        assert_eq!(Fact::share("x", 0.000_2).rendered, "0.02%");
        assert_eq!(Fact::share("x", 0.000_5).rendered, "0.05%");

        // Zero is not "0.00%". It is genuinely zero, and the two-decimal form is
        // for small-but-real, so the lower bound is exclusive.
        assert_eq!(Fact::share("x", 0.0).rendered, "0.0%");

        // And a tenth of a percent is the upper bound, also exclusive: 0.1% has
        // no hidden precision to show.
        assert_eq!(Fact::share("x", 0.001).rendered, "0.1%");

        // Ordinary magnitudes are unaffected.
        assert_eq!(Fact::share("x", 0.251).rendered, "25.1%");
    }

    #[test]
    fn every_unreadable_fact_names_which_fact_it_was() {
        // Each arm is a different sentence in a published reply. Deleting any of
        // them falls through to the generic phrase, which is safe but says less,
        // and nothing noticed -- three arms, three survivors.
        assert_eq!(
            phrase_for("launch block"),
            "the launch block could not be read"
        );
        assert_eq!(phrase_for("curve"), "the bonding curve could not be read");
        assert_eq!(
            phrase_for("creator history"),
            "the creator's history could not be read"
        );
        // The fallback is deliberately the safe one, for a fact added later by
        // someone who did not read the comment above it.
        assert_eq!(
            phrase_for("something new"),
            "part of this could not be read"
        );
    }

    #[test]
    fn lamports_convert_to_sol_at_the_documented_rate() {
        // Used only to compare against a figure a model wrote, which is why it
        // is a float at all -- but a wrong conversion there authorises a wrong
        // number in a public reply.
        assert!((lamports_as_sol(LAMPORTS_PER_SOL) - 1.0).abs() < 1e-9);
        assert!((lamports_as_sol(LAMPORTS_PER_SOL / 2) - 0.5).abs() < 1e-9);
        assert!((lamports_as_sol(3 * LAMPORTS_PER_SOL) - 3.0).abs() < 1e-9);
        assert!((lamports_as_sol(0) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn a_share_authorises_its_ordinary_roundings_and_nothing_else() {
        let f = Fact::share("x", 0.251);
        assert!(f.values.iter().any(|v| (*v - 0.251).abs() < 1e-9));
        assert!(f.values.iter().any(|v| (*v - 25.1).abs() < 1e-9));
        assert!(f.values.iter().any(|v| (*v - 25.0).abs() < 1e-9));
        // Not a number a reader would call a rounding of 25.1%.
        assert!(!f.values.iter().any(|v| (*v - 68.0).abs() < 1e-9));
        assert_eq!(f.rendered, "25.1%");
    }

    #[test]
    fn sol_renders_by_integer_arithmetic() {
        assert_eq!(render_sol(1_000_000_000), "1.0000");
        assert_eq!(render_sol(303_000_000), "0.3030");
        assert_eq!(render_sol(0), "0.0000");
    }

    #[test]
    fn an_untrusted_name_is_never_a_fact() {
        // The separation this type exists for: a number inside a token's *name*
        // must authorise nothing, or a creator could licence their own figures
        // by putting them in the name.
        let sheet = FactSheet {
            mint: "M".to_owned(),
            read_at: None,
            facts: Vec::new(),
            untrusted: vec![("token name".to_owned(), "99999 percent safe".to_owned())],
            unknown: Vec::new(),
        };
        assert!(sheet.authorised().is_empty());
        assert!(!sheet.render().contains("99999"));
    }
}
