// SPDX-License-Identifier: Apache-2.0
//! The public site's fee ladder is the one this crate decodes, row for row.
//!
//! # Why the check is here and not in the site's own suite
//!
//! `site/src/fixtures/fee-ladder.json` is published to strangers on the
//! tokenomics page: it is where the prize comes from, and a wrong row there is
//! a wrong claim about somebody's money. The obvious place to check it is the
//! site's test suite — and that is the wrong place, because the capture it
//! would have to be checked against
//! ([`fixtures/pumpswap_fees.json`](fixtures/pumpswap_fees.json)) is stored as
//! hex, and reading it from TypeScript means a second implementation of this
//! crate's fee decoder in a language that cannot be asked to agree with the
//! first. `stats.json`'s own comment refuses exactly that duplication for the
//! share arithmetic, for the same reason.
//!
//! So the decoder disposes and the fixture only publishes. The site renders the
//! rows; this asserts they are the rows on chain. If somebody edits a number on
//! the marketing page to make the prize look better, this fails.
//!
//! Research
//! [0028](../../../docs/research/0028-the-fee-after-graduation-is-a-ladder.md)
//! is the note that read the account.

use radar_pumpfun::fees::FeeConfig;

const CAPTURE: &str = include_str!("fixtures/pumpswap_fees.json");
const PUBLISHED: &str = include_str!("../../../site/src/fixtures/fee-ladder.json");

/// Lamports in one SOL. The published file is in SOL because a reader counts in
/// SOL; the account is in lamports because the program does.
const SOL: u128 = 1_000_000_000;

fn decoded() -> FeeConfig {
    let capture: serde_json::Value = serde_json::from_str(CAPTURE).expect("the capture is JSON");
    let hex = capture["accounts"]["fee_config"]["data_hex"]
        .as_str()
        .expect("the capture carries the fee config");
    assert!(hex.len().is_multiple_of(2), "hex is whole bytes");
    let bytes: Vec<u8> = (0..hex.len() / 2)
        .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).expect("the capture is valid hex"))
        .collect();
    FeeConfig::parse(&bytes).expect("PumpSwap's fee config parses")
}

fn published() -> serde_json::Value {
    serde_json::from_str(PUBLISHED).expect("the site's fixture is valid JSON")
}

#[test]
fn every_row_the_site_publishes_is_a_row_this_crate_decodes() {
    let decoded = decoded();
    let published = published();
    let rows = published["after_graduation"]["rows"]
        .as_array()
        .expect("the site publishes an array of rows");

    assert_eq!(
        rows.len(),
        decoded.tiers.len(),
        "the site publishes {} rows and the account has {}",
        rows.len(),
        decoded.tiers.len()
    );

    for (i, (row, tier)) in rows.iter().zip(&decoded.tiers).enumerate() {
        let field = |name: &str| -> u128 {
            u128::from(
                row[name]
                    .as_u64()
                    .unwrap_or_else(|| panic!("row {i} has a numeric {name}")),
            )
        };
        // The threshold is the one field that is converted rather than copied,
        // so it is the one that can be wrong by a factor of a billion without
        // looking wrong. Asserted in lamports, which is the unit the program
        // uses -- multiplying the published SOL back up, rather than dividing
        // the account down, because division would hide a row whose threshold
        // is not a whole number of SOL.
        assert_eq!(
            field("from_sol") * SOL,
            tier.threshold_lamports,
            "row {i}: the site says {} SOL and the account says {} lamports",
            field("from_sol"),
            tier.threshold_lamports
        );
        assert_eq!(field("lp_bps"), u128::from(tier.fees.lp_bps), "row {i} lp");
        assert_eq!(
            field("protocol_bps"),
            u128::from(tier.fees.protocol_bps),
            "row {i} protocol"
        );
        assert_eq!(
            field("creator_bps"),
            u128::from(tier.fees.creator_bps),
            "row {i} creator -- this is the fee that becomes the prize"
        );
    }
}

#[test]
fn the_curve_fee_the_site_quotes_is_the_first_row_of_the_ladder() {
    // The pool page and the tokenomics page both say "30 bps of volume" for a
    // token still on its curve. That number is row zero's creator share, and
    // research 0028 §"the pool page and the prize arithmetic" is the reason it
    // has to keep agreeing: a token that graduates pays 95, not 30, and the
    // page that quotes the smaller number must at least quote it correctly.
    let published = published();
    let quoted = published["curve"]["creator_bps"]
        .as_u64()
        .expect("the site quotes a curve fee");
    assert_eq!(
        u128::from(quoted),
        u128::from(decoded().tiers[0].fees.creator_bps),
        "the site's curve fee is not the ladder's first row"
    );
}

#[test]
fn the_published_ladder_falls_and_never_rises_after_the_step_up() {
    // Not a claim about the chain -- a claim about the *shape* the tokenomics
    // page draws, which is "95 immediately after graduation, sliding down to 5".
    // A reordered fixture would still pass the row-for-row check above if the
    // account were reordered with it; this fails if the story the page tells
    // stops being true of the numbers under it.
    let published = published();
    let rows = published["after_graduation"]["rows"]
        .as_array()
        .expect("rows");
    let creator: Vec<u64> = rows
        .iter()
        .map(|r| r["creator_bps"].as_u64().expect("numeric"))
        .collect();
    let thresholds: Vec<u64> = rows
        .iter()
        .map(|r| r["from_sol"].as_u64().expect("numeric"))
        .collect();

    assert!(
        thresholds.windows(2).all(|w| w[0] < w[1]),
        "the ladder's thresholds do not strictly increase: {thresholds:?}"
    );
    assert_eq!(creator[0], 30, "the ladder starts at the curve's fee");
    assert_eq!(creator[1], 95, "the step up on graduation");
    assert!(
        creator[1..].windows(2).all(|w| w[0] >= w[1]),
        "the creator's share rises somewhere after the step up: {creator:?}"
    );
    assert_eq!(
        *creator.last().expect("rows"),
        5,
        "the top of the ladder is 5 bps"
    );
}
