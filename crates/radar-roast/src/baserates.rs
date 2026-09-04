// SPDX-License-Identifier: Apache-2.0
//! Reading the published base-rate snapshot.
//!
//! The store's job in this product is **base rates, not lookup**. A per-mint
//! question is answered from the chain (see `radar-onchain`); the population it
//! is placed against is measured once, published as
//! `docs/research/data/0024-base-rates.json`, and read from disk in
//! microseconds.
//!
//! # Every figure carries the date it was measured
//!
//! This is the whole lesson of `0024`. `0008` measured these same quantities and
//! its headline was wrong by 2.7× nine days later, because the recipient count
//! is a **configuration** of whatever tool the launchers are running rather than
//! a law. A consumer that hard-codes any of these numbers is repeating that.
//!
//! So [`BaseRates::measured_on`] is not decoration, and
//! [`BaseRates::is_stale_at`] exists so a caller can refuse rather than quote a
//! month-old distribution as though it were current.
//!
//! # Rule 8: absent is a refusal to claim
//!
//! No snapshot means no population context — the reply says less, and says why.
//! It does **not** mean falling back on remembered numbers, which is how a
//! superseded figure gets published long after the note correcting it.

use serde::Deserialize;

/// Where the snapshot lives, relative to the repository root.
pub const DEFAULT_PATH: &str = "docs/research/data/0024-base-rates.json";

/// How old a snapshot may be before it should not be quoted.
///
/// Fourteen days. `0008`'s figures were wrong by 2.7× after nine, so this is
/// already generous — it is a backstop against quoting something ancient, not a
/// substitute for the scheduled re-run `0024` asks for.
pub const STALE_AFTER_DAYS: i64 = 14;

/// Why the snapshot could not be used.
#[derive(Debug, thiserror::Error)]
pub enum NotLoaded {
    /// The file was not there or could not be read.
    #[error("base rates not readable at {path}: {why}")]
    Unreadable {
        /// Where it was looked for.
        path: String,
        /// The underlying reason.
        why: String,
    },
    /// The file was there but is not the shape this expects.
    #[error("base rates malformed: {0}")]
    Malformed(String),
}

/// One band of the recipient distribution.
#[derive(Clone, Debug)]
pub struct Band {
    /// Its name, as the snapshot spells it.
    pub name: String,
    /// Lowest recipient count in the band.
    pub lo: u32,
    /// Highest recipient count in the band.
    pub hi: u32,
    /// Share of never-graduated launches in this band.
    pub never_graduated: f64,
    /// Share of organic graduations in this band.
    pub organic: f64,
    /// Share of instant graduations in this band.
    pub instant: f64,
    /// Probability a launch in this band graduates instantly.
    pub p_instant: f64,
    /// How many times the population rate that is.
    pub x_base_instant: f64,
}

/// One notional band of the measured round trip.
#[derive(Clone, Debug)]
pub struct CostBand {
    /// The notional range, as text.
    pub band: String,
    /// Round-trip cost in basis points.
    pub round_trip: f64,
}

/// The published snapshot.
#[derive(Clone, Debug)]
pub struct BaseRates {
    /// When it was measured, as `YYYY-MM-DD`.
    pub measured_on: String,
    /// Launches the distribution was measured over.
    pub launches: u64,
    /// Share of all launches that graduate at all.
    pub base_rate_graduates: f64,
    /// Share of all launches that graduate instantly.
    pub base_rate_instant: f64,
    /// The recipient bands.
    pub bands: Vec<Band>,
    /// The measured all-in round trip the kernel assumes.
    pub round_trip_kernel: f64,
    /// The bar a strategy must clear.
    pub round_trip_bar: f64,
    /// Round trip by notional.
    pub cost_bands: Vec<CostBand>,
}

#[derive(Deserialize)]
struct Raw {
    measured_on: String,
    launch_block: RawLaunchBlock,
    round_trip_bps: RawCost,
}

#[derive(Deserialize)]
struct RawLaunchBlock {
    launches: u64,
    base_rate_graduates: f64,
    base_rate_instant: f64,
    bands: Vec<RawBand>,
    populations: RawPopulations,
    histogram: serde_json::Value,
}

#[derive(Deserialize)]
struct RawPopulations {
    never_graduated: RawPopulation,
    organic: RawPopulation,
    instant: RawPopulation,
}

#[derive(Deserialize)]
struct RawPopulation {
    n: f64,
}

/// One band as the snapshot spells it.
///
/// `fires_on` and `p_graduates` are deserialised but not published. They are
/// kept because their presence is what distinguishes this schema from an older
/// one: a snapshot missing them fails to parse rather than loading with two
/// fields quietly absent.
#[derive(Deserialize)]
struct RawBand {
    name: String,
    lo: u32,
    hi: u32,
    #[expect(
        dead_code,
        reason = "deserialised to validate the schema, not published"
    )]
    fires_on: f64,
    #[expect(
        dead_code,
        reason = "deserialised to validate the schema, not published"
    )]
    p_graduates: f64,
    p_instant: f64,
    x_base_instant: f64,
}

#[derive(Deserialize)]
struct RawCost {
    bar: f64,
    kernel_assumed: f64,
    by_notional: Vec<RawCostBand>,
}

#[derive(Deserialize)]
struct RawCostBand {
    band: String,
    round_trip: f64,
}

impl BaseRates {
    /// Reads the snapshot from a path.
    ///
    /// # Errors
    ///
    /// [`NotLoaded`] when the file cannot be read or is not the expected shape.
    pub fn load(path: &str) -> Result<Self, NotLoaded> {
        let text = std::fs::read_to_string(path).map_err(|e| NotLoaded::Unreadable {
            path: path.to_owned(),
            why: e.to_string(),
        })?;
        Self::parse(&text)
    }

    /// Parses the snapshot.
    ///
    /// # Errors
    ///
    /// [`NotLoaded::Malformed`] when the JSON is not the expected shape.
    pub fn parse(text: &str) -> Result<Self, NotLoaded> {
        let raw: Raw =
            serde_json::from_str(text).map_err(|e| NotLoaded::Malformed(e.to_string()))?;
        let lb = &raw.launch_block;

        let bands = lb
            .bands
            .iter()
            .map(|b| Band {
                name: b.name.clone(),
                lo: b.lo,
                hi: b.hi,
                never_graduated: share_in(
                    &lb.histogram,
                    b.lo,
                    b.hi,
                    0,
                    lb.populations.never_graduated.n,
                ),
                organic: share_in(&lb.histogram, b.lo, b.hi, 1, lb.populations.organic.n),
                instant: share_in(&lb.histogram, b.lo, b.hi, 2, lb.populations.instant.n),
                p_instant: b.p_instant,
                x_base_instant: b.x_base_instant,
            })
            .collect();

        Ok(Self {
            measured_on: raw.measured_on,
            launches: lb.launches,
            base_rate_graduates: lb.base_rate_graduates,
            base_rate_instant: lb.base_rate_instant,
            bands,
            round_trip_kernel: raw.round_trip_bps.kernel_assumed,
            round_trip_bar: raw.round_trip_bps.bar,
            cost_bands: raw
                .round_trip_bps
                .by_notional
                .iter()
                .map(|c| CostBand {
                    band: c.band.clone(),
                    round_trip: c.round_trip,
                })
                .collect(),
        })
    }

    /// The band a recipient count falls in, if any.
    ///
    /// The bands in the snapshot overlap deliberately — "exactly six" sits
    /// inside "five to seven" — so this returns the **narrowest** match, which
    /// is the most specific true statement available about that count.
    #[must_use]
    pub fn band_for(&self, recipients: u32) -> Option<&Band> {
        self.bands
            .iter()
            .filter(|b| recipients >= b.lo && recipients <= b.hi)
            .min_by_key(|b| b.hi - b.lo)
    }

    /// Whether the snapshot is too old to quote, given today's date.
    ///
    /// Dates are compared as `YYYY-MM-DD` text converted to a day count. A date
    /// that will not parse is treated as **stale**, not as fresh: an unreadable
    /// measurement date is exactly the case where quoting the numbers anyway is
    /// how a superseded figure survives.
    #[must_use]
    pub fn is_stale_at(&self, today: &str) -> bool {
        match (days(&self.measured_on), days(today)) {
            (Some(then), Some(now)) => now - then > STALE_AFTER_DAYS,
            _ => true,
        }
    }
}

/// The share of a population sitting in a recipient band.
fn share_in(histogram: &serde_json::Value, lo: u32, hi: u32, column: usize, total: f64) -> f64 {
    if total <= 0.0 {
        return 0.0;
    }
    let mut sum = 0.0;
    for r in lo..=hi {
        if let Some(row) = histogram.get(r.to_string()).and_then(|v| v.as_array())
            && let Some(n) = row.get(column).and_then(serde_json::Value::as_f64)
        {
            sum += n;
        }
    }
    sum / total
}

/// A day number from a `YYYY-MM-DD` date.
///
/// Rough — it treats every month as 31 days — which is fine for "is this more
/// than a fortnight old" and would not be fine for anything else. Written out
/// rather than pulling a date crate into a tree that has none.
fn days(date: &str) -> Option<i64> {
    let mut parts = date.split('-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let d: i64 = parts.next()?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some(y * 372 + m * 31 + d)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SNAPSHOT: &str = include_str!("../../../docs/research/data/0024-base-rates.json");

    #[test]
    fn the_published_snapshot_parses() {
        // The file in the repository, not a fixture. A snapshot the code cannot
        // read is the LEARNINGS 1 shape: a document that outlived the thing it
        // describes.
        let rates = BaseRates::parse(SNAPSHOT).expect("the published snapshot");
        assert_eq!(rates.measured_on, "2026-09-03");
        assert_eq!(rates.launches, 17_497);
        assert!((rates.base_rate_instant - 0.011_830).abs() < 1e-6);
        // The three reconciled round-trip numbers, as docs/STATE.md carries
        // them. If the snapshot and that table ever disagree, the analyst and
        // the research notes publish different costs for the same trade.
        assert!((rates.round_trip_kernel - 850.0).abs() < 1e-9);
        assert!((rates.round_trip_bar - 456.0).abs() < 1e-9);
        assert!(!rates.bands.is_empty());
    }

    #[test]
    fn a_recipient_count_lands_in_the_narrowest_band_that_holds_it() {
        let rates = BaseRates::parse(SNAPSHOT).expect("the published snapshot");
        // Six is inside both "exactly six" and "five to seven"; the specific
        // one is the more informative true statement.
        assert_eq!(rates.band_for(6).expect("a band").name, "exactly six");
        assert_eq!(rates.band_for(5).expect("a band").name, "five to seven");
        assert_eq!(rates.band_for(2).expect("a band").name, "one to three");
        assert_eq!(rates.band_for(11).expect("a band").name, "ten to thirteen");
        // A count in no band gets no claim rather than the nearest one.
        assert!(rates.band_for(40).is_none());
    }

    /// Two bands holding the same count, where the wide one is listed first.
    ///
    /// The published snapshot happens to list its narrow bands early, so
    /// `min_by_key` returned the right answer even when the key was wrong --
    /// which is why every mutation of `b.hi - b.lo` survived. Order is not the
    /// rule; width is.
    fn overlapping(lo_wide: u32, hi_wide: u32, lo_narrow: u32, hi_narrow: u32) -> BaseRates {
        let band = |name: &str, lo, hi| Band {
            name: name.to_owned(),
            lo,
            hi,
            never_graduated: 0.0,
            organic: 0.0,
            instant: 0.0,
            p_instant: 0.0,
            x_base_instant: 0.0,
        };
        BaseRates {
            measured_on: "2026-09-03".to_owned(),
            launches: 1,
            base_rate_graduates: 0.0,
            base_rate_instant: 0.0,
            bands: vec![
                band("wide", lo_wide, hi_wide),
                band("narrow", lo_narrow, hi_narrow),
            ],
            round_trip_kernel: 0.0,
            round_trip_bar: 0.0,
            cost_bands: Vec::new(),
        }
    }

    #[test]
    fn the_narrowest_band_wins_even_when_the_wide_one_is_listed_first() {
        // 4..=9 is width 5; 6..=6 is width 0. Listed wide-first, so returning
        // the first match rather than the narrowest gives the wrong answer.
        let rates = overlapping(4, 9, 6, 6);
        assert_eq!(rates.band_for(6).expect("a band").name, "narrow");

        // And the width has to be `hi - lo`, not `hi + lo`: here the wide band
        // 1..=9 sums to 10 and the narrow 5..=5 sums to 10 as well, so a sum
        // ties and yields the first. The difference does not tie.
        let rates = overlapping(1, 9, 5, 5);
        assert_eq!(rates.band_for(5).expect("a band").name, "narrow");

        // Nor `hi / lo`: 2..=8 divides to 4 and 6..=6 divides to 1, which is
        // the right order by accident. 1..=3 divides to 3 and 2..=2 to 1 --
        // also right. This pair is the one that inverts: 3..=4 divides to 1,
        // and 4..=4 divides to 1, a tie the wide band wins on order.
        let rates = overlapping(3, 4, 4, 4);
        assert_eq!(rates.band_for(4).expect("a band").name, "narrow");
    }

    #[test]
    fn the_bands_carry_the_shares_0024_measured() {
        let rates = BaseRates::parse(SNAPSHOT).expect("the published snapshot");
        let six = rates.band_for(6).expect("a band");
        // 0024's corrected table: 52/207 instant, 930/16,972 never-graduated.
        assert!((six.instant - 52.0 / 207.0).abs() < 1e-6, "{}", six.instant);
        assert!((six.never_graduated - 930.0 / 16_972.0).abs() < 1e-6);
        // And the figure 0008 got wrong is nowhere near what this now says.
        assert!(six.instant < 0.30, "0008's 68% must not reappear");
    }

    #[test]
    fn a_snapshot_whose_date_will_not_parse_is_stale_rather_than_fresh() {
        // The direction matters: treating an unreadable date as fresh is how a
        // superseded figure keeps getting published.
        let mut rates = BaseRates::parse(SNAPSHOT).expect("the published snapshot");
        rates.measured_on = "not a date".to_owned();
        assert!(rates.is_stale_at("2026-09-03"));
        rates.measured_on = "2026-09-03".to_owned();
        assert!(!rates.is_stale_at("2026-09-10"));
        assert!(rates.is_stale_at("2026-10-03"));
    }

    #[test]
    fn the_staleness_boundary_is_where_the_constant_says_it_is() {
        // `> STALE_AFTER_DAYS` rather than `>=`, and the day either side of it.
        // Without both, the comparison can move by one and nothing notices --
        // which is a fortnight rule that is quietly a fortnight and a day.
        let mut rates = BaseRates::parse(SNAPSHOT).expect("the published snapshot");
        rates.measured_on = "2026-09-01".to_owned();
        assert_eq!(STALE_AFTER_DAYS, 14, "the dates below are chosen for this");

        // Exactly fourteen days on: still fresh.
        assert!(!rates.is_stale_at("2026-09-15"));
        // Fifteen: stale.
        assert!(rates.is_stale_at("2026-09-16"));
    }

    #[test]
    fn a_date_that_is_not_a_date_is_rejected_on_both_fields() {
        // The guard is `month out of range OR day out of range`. With `AND`, a
        // date has to be wrong in *both* to be refused -- so "2026-13-05" would
        // parse, and a snapshot dated in a thirteenth month would read as fresh.
        let mut rates = BaseRates::parse(SNAPSHOT).expect("the published snapshot");
        rates.measured_on = "2026-09-01".to_owned();

        // Bad month, good day.
        assert!(rates.is_stale_at("2026-13-05"));
        assert!(rates.is_stale_at("2026-00-05"));
        // Good month, bad day.
        assert!(rates.is_stale_at("2026-09-32"));
        assert!(rates.is_stale_at("2026-09-00"));
        // And a real date is still readable, so the guard is not simply always
        // refusing.
        assert!(!rates.is_stale_at("2026-09-02"));
    }

    #[test]
    fn the_day_count_orders_dates_across_years_months_and_days() {
        // `y * 372 + m * 31 + d`. Every coefficient here has to be big enough
        // that the field below it cannot overtake it: a year must outrank any
        // month, and a month must outrank any day. The mutations that survived
        // were `*` to `+` and `+` to `*` on exactly these, which is the same
        // rule stated as arithmetic.
        let mut rates = BaseRates::parse(SNAPSHOT).expect("the published snapshot");
        rates.measured_on = "2026-01-01".to_owned();

        // A year later is stale, however early in the year it falls -- so the
        // year term dominates twelve months plus thirty-one days.
        assert!(rates.is_stale_at("2027-01-01"));
        // A month later is stale, however early in the month -- so the month
        // term dominates thirty-one days.
        assert!(rates.is_stale_at("2026-02-01"));
        // And the December-to-January step is one month, not a jump backwards.
        rates.measured_on = "2026-12-20".to_owned();
        assert!(!rates.is_stale_at("2026-12-30"));
        assert!(rates.is_stale_at("2027-01-20"));
    }

    #[test]
    fn a_malformed_snapshot_is_refused_rather_than_defaulted() {
        assert!(BaseRates::parse("{}").is_err());
        assert!(BaseRates::parse("not json").is_err());
    }
}
