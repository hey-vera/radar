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

/// What a fact is a claim about, because one kind is withheld for one mint.
///
/// ADR 0013 constraint 5: the analyst never states its own token's price or
/// market capitalisation. That is enforced here rather than requested of the
/// model — a fact tagged [`About::Price`] is dropped from the sheet for the
/// configured mint **before the model sees it**, so the number is never in the
/// set the fidelity check would authorise.
///
/// **Nothing on the sheet is a price fact today.** Every figure the builder
/// emits is structure, history, depth, cost or population, so the rule has
/// nothing to drop yet. The variant exists so that the first price or
/// market-cap fact anyone adds is withheld for the analyst's own token by
/// construction, rather than by a reviewer remembering the ADR. The residual
/// is stated plainly: [`Fact::exact`] and [`Fact::share`] tag a measurement, so
/// an author adding a market-cap line through them and not through a literal
/// still has to choose the tag. There is no way to make the compiler ask.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum About {
    /// Structure, history, depth, cost or population -- what the analyst
    /// exists to state, about any token including its own.
    Measurement,
    /// The token's price or market capitalisation, in any unit and any form.
    Price,
}

/// One publishable number, with the words that make it a claim.
#[derive(Clone, Debug, PartialEq)]
pub struct Fact {
    /// What kind of claim this is. Decides whether the self-mint rule drops it.
    pub about: About,
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
    /// A measured fact whose only value is the one in its rendering.
    ///
    /// A **measurement**, never a price: a price or market-cap fact is built as
    /// a literal with [`About::Price`], so that the choice is written down where
    /// the self-mint rule can read it.
    #[must_use]
    pub fn exact(label: impl Into<String>, value: f64, rendered: impl Into<String>) -> Self {
        Self {
            about: About::Measurement,
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
            about: About::Measurement,
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
    ///
    /// `self_mint` is the analyst's own token, from `RADAR_SELF_MINT`, or
    /// `None` when no token is special. When the dossier is about that mint,
    /// every [`About::Price`] fact is dropped and the sheet says so
    /// ([`withhold_price`]). Everything else about the token is stated on the
    /// same rule as any other coin — ADR 0013 constraint 6 — which is why this
    /// is one filter and not a separate path.
    #[must_use]
    pub fn build(
        dossier: &Dossier,
        rates: Option<&BaseRates>,
        creators: Option<&crate::creator::CreatorIndex>,
        self_mint: Option<&radar_types::Address>,
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

        // Last, after every push, so a price fact added anywhere above is
        // caught. Compared on the parsed address, not on text: the mint a
        // stranger typed has already been parsed by the time a dossier exists,
        // and two spellings of one address must not be two tokens here.
        if self_mint == Some(&dossier.mint) {
            withhold_price(&mut facts);
        }

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

/// ADR 0013 constraint 5, applied to one sheet.
///
/// Drops every [`About::Price`] fact and says so in the trusted block, so the
/// model is told *why* the figure is absent rather than left to supply one --
/// which it could not do anyway, because a number that is not on the sheet is
/// one the fidelity check refuses. The note carries no digit for that reason:
/// [`FactSheet::authorised`] reads numerals out of the rendered block, and a
/// note citing the ADR by number would authorise that number.
///
/// A function of the facts alone, and separate from [`FactSheet::build`], so
/// it can be tested against a sheet that carries a price fact -- which no
/// dossier produces today.
fn withhold_price(facts: &mut Vec<Fact>) {
    facts.retain(|f| f.about != About::Price);
    facts.push(Fact {
        about: About::Measurement,
        label: "this token".to_owned(),
        rendered: "the analyst's own. Its price and market capitalisation are never stated, \
                   whoever asks and whatever they are. Say so if it comes up; say nothing \
                   about what it is worth."
            .to_owned(),
        values: Vec::new(),
    });
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
            about: About::Measurement,
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
            about: About::Measurement,
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
            about: About::Measurement,
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
        about: About::Measurement,
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
            about: About::Measurement,
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
            about: About::Measurement,
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
            about: About::Measurement,
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

    /// A market-cap fact, which nothing builds today and which is exactly what
    /// the rule exists to catch when something does.
    fn a_price_fact() -> Fact {
        Fact {
            about: About::Price,
            label: "market capitalisation when read".to_owned(),
            rendered: "69000 USD".to_owned(),
            values: vec![69_000.0, 69.0],
        }
    }

    #[test]
    fn a_price_fact_is_withheld_and_the_sheet_says_why() {
        // ADR 0013 constraint 5. The figure must leave the authorised set, not
        // merely the rendering: a price the model may not see but may still
        // cite is a price the fidelity check would let through.
        //
        // Re-apply the bug by deleting the `retain` in `withhold_price`: the
        // 69000 stays authorised and this fails on the first assertion.
        let mut facts = vec![Fact::exact("recipients", 6.0, "6"), a_price_fact()];
        withhold_price(&mut facts);
        let sheet = FactSheet {
            mint: "M".to_owned(),
            read_at: None,
            facts,
            untrusted: Vec::new(),
            unknown: Vec::new(),
        };

        let authorised = sheet.authorised();
        assert!(
            !authorised.iter().any(|v| (*v - 69_000.0).abs() < 1e-9),
            "the market cap survived withholding: {authorised:?}"
        );
        assert!(
            !authorised.iter().any(|v| (*v - 69.0).abs() < 1e-9),
            "a rendering of the market cap survived: {authorised:?}"
        );
        assert!(
            sheet.facts.iter().all(|f| f.about != About::Price),
            "a price fact is still on the sheet: {:?}",
            sheet.facts
        );

        // Withholding must not become silence, and must not cost the answer.
        // Rule 9: an absence that goes unmentioned reads as reassurance, or
        // here as coyness -- the model is told why the figure is not there.
        let rendered = sheet.render();
        assert!(rendered.contains("never stated"), "{rendered}");
        assert!(
            authorised.iter().any(|v| (*v - 6.0).abs() < 1e-9),
            "the measured fact was lost with the price: {authorised:?}"
        );

        // And the note itself authorises nothing. `authorised` reads numerals
        // out of the rendered block, so a note that cited the ADR by number
        // would licence that number. Only the recipient count remains.
        assert!(
            authorised.iter().all(|v| (*v - 6.0).abs() < 1e-9),
            "the note put a number into the authorised set: {authorised:?}"
        );
    }

    /// A dossier about one mint and nothing else, so what the sheet says is
    /// decided by the mint alone.
    fn dossier_for(mint: [u8; 32]) -> Dossier {
        Dossier {
            mint: radar_types::Address::new(mint),
            read_at: None,
            launch: None,
            curve: None,
            creator_transactions: None,
            unavailable: Vec::new(),
            calls: 0,
            elapsed_ms: 0,
        }
    }

    #[test]
    fn the_curve_facts_reach_the_sheet() {
        // `push_curve` replaced with nothing survived mutation testing on
        // 2026-09-05: no test asserted that a curve the dossier read appears on
        // the sheet. A sheet that silently says less is LEARNINGS 5 in a
        // published reply -- an absence that reads as fine.
        let mut dossier = dossier_for([3u8; 32]);
        dossier.curve = Some(radar_onchain::CurveFacts {
            complete: false,
            real_sol_reserves: 6_186_150_833,
            capacity_lamports: Some(303_000_000),
            fees: None,
        });
        let rendered = FactSheet::build(&dossier, None, None, None).render();
        assert!(
            rendered.contains("has the token graduated off the bonding curve: no"),
            "{rendered}"
        );
        assert!(rendered.contains("0.3030 SOL"), "{rendered}");

        // Graduated, and no depth at all: both are statements, not blanks.
        let mut done = dossier_for([3u8; 32]);
        done.curve = Some(radar_onchain::CurveFacts {
            complete: true,
            real_sol_reserves: 0,
            capacity_lamports: None,
            fees: None,
        });
        let rendered = FactSheet::build(&done, None, None, None).render();
        assert!(
            rendered.contains("has the token graduated off the bonding curve: yes"),
            "{rendered}"
        );
        assert!(rendered.contains("cannot size into this"), "{rendered}");
    }

    const SNAPSHOT: &str = include_str!("../../../docs/research/data/0024-base-rates.json");

    #[test]
    fn the_cost_facts_reach_the_sheet() {
        // `push_cost` replaced with nothing survived the same run. The cost line
        // is the fact GOAL.md says leads every reply, and nothing pinned that it
        // was there at all.
        let rates = BaseRates::parse(SNAPSHOT).expect("the published snapshot");
        let sheet = FactSheet::build(&dossier_for([3u8; 32]), Some(&rates), None, None);
        let rendered = sheet.render();
        assert!(
            rendered.contains(
                "expected edge a strategy must clear before one trade is worth making: 456 bps"
            ),
            "{rendered}"
        );
        assert!(
            rendered.contains("round trip Radar's kernel assumes, on fresh launches: 850 bps"),
            "{rendered}"
        );
        assert!(
            rendered.contains("round trip for a position of $20-$200: 456 bps (4.6%)"),
            "{rendered}"
        );
        // Authorised in both renderings, so a reply quoting 4.6% is not refused
        // as a fabrication of a figure the sheet stated.
        let authorised = sheet.authorised();
        assert!(authorised.iter().any(|v| (*v - 456.0).abs() < 1e-9));
        assert!(authorised.iter().any(|v| (*v - 4.56).abs() < 1e-9));
    }

    #[test]
    fn the_measured_population_reaches_the_sheet() {
        // `push_measured_population` replaced with nothing survived the same
        // run. This is the denominator every creator count is read against;
        // without it "none of 150 filled its curve" has no scale.
        let index = crate::creator::CreatorIndex {
            watermark_slot: 444_374_676,
            built_at: 1_788_000_000,
            population: Some(crate::creator::Population {
                launches: 508_814,
                measured: 506_991,
                organic: 9_060,
                instant: 5_222,
                stillborn: 116_608,
            }),
            creators: std::collections::BTreeMap::new(),
        };
        let rendered = FactSheet::build(&dossier_for([3u8; 32]), None, Some(&index), None).render();
        assert!(
            rendered.contains(
                "launches Radar has recorded and measured, which every share below is out of: 506991"
            ),
            "{rendered}"
        );
        assert!(
            rendered.contains(
                "of every measured launch, how many showed almost no activity at all: 23.0%"
            ),
            "{rendered}"
        );
        assert!(
            rendered.contains("of every measured launch, how many graduated at all: "),
            "{rendered}"
        );

        // Nothing measured is not a population of zeroes -- rule 9. The figure
        // is said to be missing, in Radar's words, rather than published as 0%.
        let empty = crate::creator::CreatorIndex {
            population: Some(crate::creator::Population::default()),
            ..index
        };
        let rendered = FactSheet::build(&dossier_for([3u8; 32]), None, Some(&empty), None).render();
        assert!(
            rendered.contains("NOT AVAILABLE -- no outcome has been measured yet"),
            "{rendered}"
        );
        assert!(!rendered.contains("0.0%"), "{rendered}");
    }

    #[test]
    fn the_rule_applies_to_the_configured_mint_and_to_no_other() {
        // Three sheets, one dossier. The comparison in `build` is the whole of
        // "is this the analyst's own token", and it is one character from
        // applying to every coin or to none.
        let own = radar_types::Address::new([3u8; 32]);
        let other = radar_types::Address::new([4u8; 32]);
        let dossier = dossier_for([3u8; 32]);

        let withheld = FactSheet::build(&dossier, None, None, Some(&own));
        assert!(
            withheld.render().contains("never stated"),
            "the configured mint must be told apart: {}",
            withheld.render()
        );

        // Another coin is answered like any other, with no mention of the rule.
        // A note on every sheet would make every reply about the analyst's own
        // token, which is the opposite of constraint 6.
        let stranger = FactSheet::build(&dossier, None, None, Some(&other));
        assert!(
            !stranger.render().contains("never stated"),
            "{}",
            stranger.render()
        );

        // No token configured: no token is special. Rule 8 is not touched --
        // absence means the rule has nothing to apply to, not that a default
        // mint is assumed.
        let unconfigured = FactSheet::build(&dossier, None, None, None);
        assert!(
            !unconfigured.render().contains("never stated"),
            "{}",
            unconfigured.render()
        );
    }
}
