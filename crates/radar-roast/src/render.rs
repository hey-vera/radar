// SPDX-License-Identifier: Apache-2.0
//! Making a reply safe to publish, before anything decides whether it is true.
//!
//! # Why this runs first, and not last
//!
//! The obvious place for a "clean the text up" pass is at the end, just before
//! publishing. That is the wrong place, and the reason is specific.
//!
//! [`forbidden::check`](crate::forbidden::check) refuses a reply containing a
//! word this account may never publish. [`fidelity::check`](crate::fidelity::check)
//! refuses one containing a number the fact sheet does not authorise. Both read
//! the text as characters. A zero-width space is a character that renders as
//! nothing:
//!
//! ```text
//!   written by the model      what the checks see       what X renders
//!   s\u{200b}cam              "s", "cam"                scam
//!   1\u{200b}00%              "1" and "00"              100%
//! ```
//!
//! So a reply can carry a forbidden claim or an invented figure **past both
//! checks** and still say it to the reader. Cleaning afterwards would assemble
//! exactly the statement the checks refused.
//!
//! Sanitising first closes it: the checks read the same characters the reader
//! will. That ordering is the whole point of this module and it is what its
//! tests pin.
//!
//! # What is removed
//!
//! Only characters that carry no meaning a reader can see:
//!
//! - **Bidirectional overrides** — `U+202A`–`U+202E` and `U+2066`–`U+2069`.
//!   These reverse the rendering of everything after them, which turns a true
//!   sentence into a different one without changing a byte a checker reads.
//! - **Zero-width and format characters** — `U+200B`–`U+200F`, `U+FEFF`.
//! - **Control characters**, except the newlines a reply legitimately contains.
//!
//! Everything else survives. A token name in any script, emoji, punctuation and
//! ordinary Unicode are all left exactly as written: this is not a filter for
//! taste, and a reply that mangled a Japanese token's name would be wrong in a
//! way nobody asked for.
//!
//! # Where the creator's bytes come from
//!
//! The deterministic template does not embed them — it renders the mint, Radar's
//! own labels and Radar's own numbers. The model path can, because the token's
//! name and symbol are handed to it as fenced untrusted evidence and it may
//! quote them. So this matters for the model path and is applied to both, since
//! the cost of applying it to text that cannot need it is nothing.

/// The most characters a reply may carry.
///
/// X refuses a longer post outright, and a refusal is a 4xx, which
/// `radar-analyst`'s backoff never retries — so an over-long reply is not a
/// delayed reply, it is a lost one. Truncating deliberately is better than
/// discovering the limit at the platform.
///
/// Counted in **characters, not bytes**: an emoji is one character to a reader
/// and four bytes to a buffer, and truncating by bytes would also cut one in
/// half and produce invalid text.
pub const MAX_CHARS: usize = 280;

/// A reply, with everything a reader cannot see removed.
///
/// Idempotent: running it twice changes nothing the first pass left.
#[must_use]
pub fn for_publication(text: &str) -> String {
    let cleaned: String = text
        .chars()
        .filter(|c| !is_invisible(*c))
        // A newline is the one control character a reply legitimately carries.
        .filter(|c| !c.is_control() || *c == '\n')
        .collect();

    // Trimmed after cleaning rather than before: a reply that is entirely
    // invisible characters is empty, and it should be reported as empty rather
    // than as whitespace.
    let trimmed = cleaned.trim();
    if trimmed.chars().count() <= MAX_CHARS {
        return trimmed.to_owned();
    }
    trimmed
        .chars()
        .take(MAX_CHARS)
        .collect::<String>()
        .trim_end()
        .to_owned()
}

/// Whether a character renders as nothing.
const fn is_invisible(c: char) -> bool {
    matches!(c,
        // Bidirectional overrides and isolates.
        '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
        // Zero-width space, joiners, and the directional marks.
        | '\u{200b}'..='\u{200f}'
        // Byte-order mark, which is also a zero-width no-break space.
        | '\u{feff}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zero_width_space_cannot_hide_a_word_from_a_check() {
        // The attack this module exists for: the checks read characters, and a
        // zero-width space splits a word into two the checker does not
        // recognise while the reader sees one word.
        let hidden = "this is a s\u{200b}cam";
        assert_eq!(for_publication(hidden), "this is a scam");
    }

    #[test]
    fn a_zero_width_space_cannot_hide_a_number_either() {
        // Same attack against the fidelity check: "1", "00" and "%" are not a
        // figure; "100%" is, and it is the one the reader gets.
        assert_eq!(for_publication("up 1\u{200b}00%"), "up 100%");
    }

    #[test]
    fn bidirectional_overrides_are_removed() {
        // An override reverses everything after it, so a true sentence renders
        // as a different one without a checker seeing a different character.
        let attacked = "the round trip is 30%\u{202e} not";
        let out = for_publication(attacked);
        assert!(!out.contains('\u{202e}'), "{out:?}");
        assert_eq!(out, "the round trip is 30% not");
        for c in [
            '\u{202a}', '\u{202b}', '\u{202c}', '\u{202d}', '\u{2066}', '\u{2069}',
        ] {
            assert!(
                !for_publication(&format!("a{c}b")).contains(c),
                "{c:?} must not survive"
            );
        }
    }

    #[test]
    fn control_characters_go_but_newlines_stay() {
        // A reply legitimately spans lines. A carriage return or an escape does
        // not, and an escape is how a terminal reading the log gets driven.
        assert_eq!(for_publication("a\u{1b}[31mb"), "a[31mb");
        assert_eq!(for_publication("line one\nline two"), "line one\nline two");
        assert_eq!(for_publication("a\rb"), "ab");
        assert_eq!(for_publication("a\u{0}b"), "ab");
    }

    #[test]
    fn ordinary_text_in_any_script_is_left_alone() {
        // Not a filter for taste. A reply that mangled a token's name would be
        // wrong in a way nobody asked for.
        for text in [
            "The round trip on a $50 position here is about 30%.",
            "トークン名はそのまま",
            "Le coût est de 4,6 %",
            "emoji survive 🚀 unchanged",
            "punctuation: -- \"quoted\" (parenthetical) 100% & more",
        ] {
            assert_eq!(for_publication(text), text, "{text:?} must survive intact");
        }
    }

    #[test]
    fn an_over_long_reply_is_truncated_rather_than_lost() {
        // X refuses a longer post, a refusal is a 4xx, and a 4xx is never
        // retried -- so this is the difference between a shortened reply and no
        // reply at all.
        let long = "a".repeat(MAX_CHARS + 50);
        let out = for_publication(&long);
        assert_eq!(out.chars().count(), MAX_CHARS);
    }

    #[test]
    fn truncation_counts_characters_rather_than_bytes() {
        // Truncating an emoji by bytes cuts it in half and produces text that
        // is not valid to send.
        let long = "🚀".repeat(MAX_CHARS + 10);
        let out = for_publication(&long);
        assert_eq!(out.chars().count(), MAX_CHARS);
        assert!(out.chars().all(|c| c == '🚀'));
    }

    #[test]
    fn a_reply_of_exactly_the_limit_is_not_touched() {
        let exact = "b".repeat(MAX_CHARS);
        assert_eq!(for_publication(&exact), exact);
    }

    #[test]
    fn a_reply_that_is_entirely_invisible_becomes_empty() {
        // Reported as empty, which the caller already treats as "the model
        // returned nothing usable" and answers with the template.
        assert_eq!(for_publication("\u{200b}\u{202e}\u{feff}"), "");
        assert_eq!(for_publication("   \n  "), "");
    }

    #[test]
    fn cleaning_is_idempotent() {
        // A second pass must not keep eating the reply, because the text is
        // cleaned once before the checks and the checks may run on it again.
        let once = for_publication("a\u{200b}b\u{202e} c");
        assert_eq!(for_publication(&once), once);
    }
}
