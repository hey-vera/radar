// SPDX-License-Identifier: Apache-2.0
//! The check that the model did not introduce a number.
//!
//! # What this is, in one sentence
//!
//! Every numeric literal in the generated reply must appear in the fact sheet;
//! if one does not, the reply is discarded and the deterministic template ships
//! instead.
//!
//! # Why a check and not an instruction
//!
//! The system prompt tells the model not to invent figures, and that is worth
//! saying, but an instruction is a request and this is a public account. The
//! failure it guards against is not a malicious model — it is an ordinary one
//! being helpful: converting a fee to a dollar figure, adding two numbers,
//! rounding to something friendlier, recalling a statistic about pump.fun from
//! training. Each of those is a fabricated measurement published under a name
//! whose entire claim is that its numbers are measured.
//!
//! `radar-signer` makes the same move for the same reason. It does not trust
//! the caller's description of a transaction; it re-decodes the bytes and
//! checks them against the authorisation. **The signer re-reads the bytes it
//! signs; the roaster re-reads the numbers it posts.**
//!
//! # The tolerance rule, and why it is not zero
//!
//! A reply that may not round is a reply that must say "25.1%" where a person
//! would say "a quarter", and the result is unreadable. So a literal is accepted
//! when it matches an authorised value **at the precision the literal itself was
//! written to**: `25` matches `25.1` because `25.1` rounded to zero decimals is
//! `25`; `25.4` does not match `25.1` at one decimal.
//!
//! That rule is itself tested, because a tolerance nobody checked is a hole
//! nobody knows the size of.
//!
//! # What it cannot do
//!
//! It checks *numbers*, not claims. "The creator is a scammer" contains no
//! numeral and passes here — [`crate::forbidden`] is what refuses that, and the
//! two are separate because they fail for different reasons and a reader of
//! either should not have to hold both in mind.

/// Why a reply was rejected.
#[derive(Clone, Debug, PartialEq)]
pub struct Fabricated {
    /// The literal as it appeared in the reply.
    pub literal: String,
    /// Its value.
    pub value: f64,
}

/// Checks a reply against the numbers a fact sheet authorises.
///
/// Returns every literal that is not accounted for, in the order they appear.
/// An empty result means the reply may ship.
#[must_use]
pub fn check(reply: &str, authorised: &[f64]) -> Vec<Fabricated> {
    literals(reply)
        .into_iter()
        .filter(|(literal, value)| !accounted_for(*value, literal, authorised))
        .map(|(literal, value)| Fabricated { literal, value })
        .collect()
}

/// Whether one literal is explained by an authorised value.
fn accounted_for(value: f64, literal: &str, authorised: &[f64]) -> bool {
    // A year, a slot or an ordinal is still a number, and there is no safe
    // general exemption: "6 recipients" and "2026" are the same token to a
    // scanner. So nothing is exempt, and anything the reply is genuinely
    // allowed to say -- the slot included -- is put on the sheet instead.
    let decimals = decimals_in(literal);
    authorised.iter().any(|a| {
        // Exact, for the common case and for integers.
        if (a - value).abs() < 1e-9 {
            return true;
        }
        // Or the authorised value rounded to the precision the model wrote.
        //
        // An exact comparison is correct here and the lint is wrong about it:
        // both sides have just been rounded to the SAME number of decimals, so
        // they are the same grid points or they are different ones. A tolerance
        // on top would widen the rule by an unstated amount, which is the one
        // thing a fidelity check must not have.
        #[expect(
            clippy::float_cmp,
            reason = "both sides are rounded to the same precision immediately above; a \
                      margin here would widen the tolerance rule by an unstated amount"
        )]
        let same = round_to(*a, decimals) == round_to(value, decimals);
        same
    })
}

/// Rounds to a number of decimal places.
fn round_to(v: f64, decimals: u32) -> f64 {
    let factor = 10f64.powi(i32::try_from(decimals).unwrap_or(0));
    (v * factor).round() / factor
}

/// How many decimal places a literal was written to.
fn decimals_in(literal: &str) -> u32 {
    literal
        .split_once('.')
        .map_or(0, |(_, frac)| u32::try_from(frac.len()).unwrap_or(0))
}

/// The shortest run of base58 characters treated as an address rather than a
/// number.
///
/// A Solana address is 32–44 base58 characters. No unit suffix, currency symbol
/// or ordinary word comes anywhere near this length, so the rule cannot swallow
/// a real figure — "4200bps" is nine characters and is still scanned.
const ADDRESS_MIN_LEN: usize = 32;

/// Blanks out address-shaped tokens before scanning.
///
/// A digit inside an identifier is not a claim. Without this, the mint in
/// "Radar on 82U9hMTJP9Wz…" contributes half a dozen numbers that no fact sheet
/// authorises, and every reply naming the token it is about would be rejected.
///
/// It is a **security** rule and not only a cosmetic one. pump.fun addresses are
/// ground so that they end in `pump`, which is public proof that grinding an
/// address to contain a chosen substring is cheap. If digits inside an address
/// were authorised, an attacker could mint a token whose address contains the
/// figure they want published and then have it quoted back as measured.
fn blank_addresses(text: &str) -> String {
    const B58: &str = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<char> = Vec::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        let start = i;
        while i < chars.len() && B58.contains(chars[i]) {
            i += 1;
        }
        let run = i - start;
        if run >= ADDRESS_MIN_LEN {
            out.extend(std::iter::repeat_n(' ', run));
        } else {
            out.extend_from_slice(&chars[start..i]);
        }
        if i < chars.len() {
            out.push(chars[i]);
            i += 1;
        }
    }
    out.into_iter().collect()
}

/// Every numeric literal in a piece of text, with its value.
///
/// Address-shaped tokens are blanked first — see [`blank_addresses`]. Commas
/// inside a number are group separators and are dropped, because a model writing
/// `17,497` means the same as `17497` and a scanner that split them would report
/// `17` and `497`: two fabricated numbers where there were none, which would
/// send every reply to the template.
///
/// A `%` or a `$` around the number is punctuation and is not part of it.
#[must_use]
pub fn literals(text: &str) -> Vec<(String, f64)> {
    let text = blank_addresses(text);
    let bytes: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let start = i;
        let mut seen_dot = false;
        while i < bytes.len() {
            let c = bytes[i];
            if c.is_ascii_digit() {
                i += 1;
            } else if c == ',' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit() {
                // A separator only when digits continue on the other side, so
                // "6, 7 and 8" is three numbers rather than one.
                i += 1;
            } else if c == '.' && !seen_dot && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit()
            {
                // A decimal point only when a digit follows: a number ending a
                // sentence must not swallow the full stop.
                seen_dot = true;
                i += 1;
            } else {
                break;
            }
        }
        let literal: String = bytes[start..i].iter().collect();
        let cleaned: String = literal.chars().filter(|c| *c != ',').collect();
        if let Ok(value) = cleaned.parse::<f64>() {
            out.push((cleaned, value));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fabricated_figure_is_caught() {
        // The verification standard: inject a number nobody measured and
        // confirm the check catches it.
        let caught = check("The round trip here is 4200 bps.", &[850.0, 456.0]);
        assert_eq!(caught.len(), 1);
        assert_eq!(caught[0].literal, "4200");
    }

    #[test]
    fn an_authorised_figure_passes() {
        assert!(check("The round trip is 850 bps.", &[850.0, 456.0]).is_empty());
    }

    #[test]
    fn ordinary_rounding_is_allowed_and_misstatement_is_not() {
        // The tolerance rule, stated as a test because a tolerance nobody
        // checked is a hole nobody knows the size of.
        assert!(
            check("about 25%", &[25.1]).is_empty(),
            "25 rounds from 25.1"
        );
        assert!(check("25.1%", &[25.1]).is_empty());
        assert!(
            !check("25.4%", &[25.1]).is_empty(),
            "not a rounding of 25.1"
        );
        assert!(!check("68%", &[25.1]).is_empty(), "0008's dead number");
        // Rounding does not licence a different order of magnitude.
        assert!(!check("250", &[25.1]).is_empty());
    }

    #[test]
    fn a_number_at_the_very_end_of_the_text_does_not_read_past_it() {
        // The decimal-point branch guards with `i + 1 < bytes.len()` before
        // looking at the next byte. Mutated to `i - 1`, the guard is true at the
        // last index and the read runs off the end. The existing full-stop test
        // did not catch it because its full stop was not the final byte.
        //
        // Both of these end exactly at the character after the digits.
        assert!(check("the figure is 25.", &[25.0]).is_empty());
        assert_eq!(literals("ends on 6").len(), 1);

        // And the *text* of the literal, not only how many there are. The guard
        // has a second `i + 1`, inside `bytes[i + 1]`, and mutating that one to
        // `i - 1` looks at the digit already consumed instead of the character
        // ahead -- so the full stop is swallowed into the number. "6." and "6"
        // parse to the same f64, which is why a count cannot tell them apart;
        // what changes is the precision the literal is taken to be written to,
        // and that is what the rounding tolerance is derived from.
        // The full stop must not be the final character, or the guard before it
        // (`i + 1 < bytes.len()`) short-circuits and the byte in question is
        // never read -- which is how the first version of this test missed the
        // mutation entirely while looking straight at it.
        let ends = literals("ends on 6. Next");
        assert_eq!(ends.len(), 1);
        assert_eq!(
            ends[0].0, "6",
            "the full stop is punctuation, not a decimal point"
        );

        // The same character *is* a decimal point when a digit follows it.
        let inner = literals("it is 6.5 here");
        assert_eq!(inner.len(), 1);
        assert_eq!(inner[0].0, "6.5");
    }

    #[test]
    fn the_exact_match_is_a_difference_and_not_a_sum_or_a_ratio() {
        // Two values that agree far past any precision a model would write, but
        // round to different grid points at the precision it *did* write. Only
        // the `(a - value).abs()` branch can pass this, so it is the branch
        // being tested rather than the rounding one underneath it.
        //
        // Every mutation of that expression breaks it: a sum is ~1.0, a ratio is
        // ~1.0, and neither is under the epsilon -- and with the rounding branch
        // disagreeing, the literal is reported as fabricated.
        assert!(
            check("0.5000000002", &[0.500_000_000_1]).is_empty(),
            "a difference of 1e-10 is the same number to any reader"
        );
    }

    #[test]
    fn the_epsilon_is_exclusive_and_the_boundary_is_where_it_says() {
        // A difference of exactly the epsilon is *not* a match. `<` and `<=` are
        // one character apart and disagree only here, and a tolerance rule that
        // silently widened by one representable step is the thing this whole
        // module exists to prevent.
        //
        // 1e-9 minus zero is exactly 1e-9 in binary floating point, so this is a
        // real boundary rather than an approximation of one.
        assert!(
            !check("0.000000001", &[0.0]).is_empty(),
            "exactly the epsilon is outside it"
        );
        // And a step under it is inside.
        assert!(check("0.000000001", &[0.000_000_000_1]).is_empty());
    }

    #[test]
    fn group_separators_do_not_split_a_number_into_two_inventions() {
        // Without this, every reply quoting the population would be sent to the
        // template -- and a check that fires on everything gets switched off.
        assert_eq!(
            literals("17,497 launches"),
            vec![("17497".to_owned(), 17_497.0)]
        );
        assert!(check("17,497 launches", &[17_497.0]).is_empty());
    }

    #[test]
    fn a_comma_between_numbers_is_still_a_list() {
        // The other direction: "6, 7 and 8" must not become 678.
        let found = literals("6, 7 and 8");
        assert_eq!(found.len(), 3);
        assert_eq!(found[0].0, "6");
        assert_eq!(found[2].0, "8");
    }

    #[test]
    fn a_number_ending_a_sentence_does_not_swallow_the_full_stop() {
        assert_eq!(literals("It is 6."), vec![("6".to_owned(), 6.0)]);
        assert_eq!(literals("It is 6.5."), vec![("6.5".to_owned(), 6.5)]);
    }

    #[test]
    fn currency_and_percent_signs_are_punctuation() {
        assert_eq!(
            literals("$50 and 30%"),
            vec![("50".to_owned(), 50.0), ("30".to_owned(), 30.0),]
        );
    }

    #[test]
    fn digits_inside_an_address_are_not_claims() {
        // Without this, every reply naming the token it is about is rejected,
        // because a base58 address is full of digits. And because pump.fun
        // addresses are ground to end in `pump`, an attacker could otherwise
        // grind one containing the figure they wanted published.
        let mint = "82U9hMTJP9WzBAG5852mRoQ4Qbwa48nWudPyGEpHpump";
        assert!(literals(mint).is_empty(), "{:?}", literals(mint));
        assert!(check(&format!("Radar on {mint}: 6 recipients."), &[6.0]).is_empty());
    }

    #[test]
    fn a_short_token_with_letters_is_still_scanned() {
        // The rule must not become a way to smuggle a number past the check by
        // gluing a unit to it. "4200bps" is nowhere near address length.
        let found = literals("the round trip is 4200bps");
        assert_eq!(found, vec![("4200".to_owned(), 4200.0)]);
        assert!(!check("4200bps", &[850.0]).is_empty());
    }

    #[test]
    fn a_reply_with_no_numbers_passes_trivially() {
        assert!(check("Radar has no record of this token.", &[]).is_empty());
    }

    #[test]
    fn every_fabricated_literal_is_reported_not_just_the_first() {
        // The log is how a public mistake becomes a correction rather than an
        // argument, so it has to say everything that was wrong.
        let caught = check("11 recipients, 99% of them, 4200 bps", &[11.0]);
        assert_eq!(caught.len(), 2);
        assert_eq!(caught[0].literal, "99");
        assert_eq!(caught[1].literal, "4200");
    }

    #[test]
    fn a_model_doing_arithmetic_on_real_facts_is_still_caught() {
        // The realistic failure. Both inputs are authorised; the sum is not a
        // measurement, and it is exactly the kind of helpfulness that would
        // otherwise put an unmeasured figure under a name that promises
        // measurement.
        assert!(!check("850 plus 456 is 1306 bps", &[850.0, 456.0]).is_empty());
    }
}
