// SPDX-License-Identifier: Apache-2.0
//! JSON canonicalisation, RFC 8785, for the subset Radar actually produces.
//!
//! Privy verifies an authorization signature by rebuilding the payload on its
//! side and canonicalising it the same way. So the bytes signed here must match
//! the bytes Privy derives, exactly, or every signing call fails.
//!
//! # Why a subset, and why it errors rather than approximating
//!
//! RFC 8785's hard part is numbers: it specifies ECMAScript's `Number::toString`,
//! which is a shortest-round-trip algorithm with several cases that are easy to
//! get subtly wrong and hard to notice.
//!
//! Radar builds these payloads itself. They contain a version integer, HTTP
//! method and URL strings, a header map, and a body whose values are strings —
//! and no floating-point number anywhere. So this handles that subset and
//! **errors** on anything outside it.
//!
//! Erring is the safe direction and the useful one. A canonicaliser that guessed
//! at a float would produce a signature Privy rejects, and the failure would
//! surface as an authentication error on some payloads and not others — the kind
//! of bug that gets blamed on the vendor for a week. Refusing to encode says
//! which value it could not encode.

use std::fmt::Write as _;

use serde_json::Value;

/// Why a payload could not be canonicalised.
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum NotCanonical {
    /// A floating-point number was present.
    ///
    /// RFC 8785 serialises numbers with ECMAScript's shortest-round-trip
    /// algorithm. Radar produces none, so rather than implement it and be
    /// approximately right, this refuses and names the value.
    #[error(
        "the payload contains the non-integer number {value}, which this \
         canonicaliser does not encode. Radar's payloads contain no \
         floating-point values; if one has appeared, the payload is wrong."
    )]
    NotAnInteger {
        /// The offending value, as serde saw it.
        value: String,
    },
}

/// Canonicalises a JSON value to the bytes RFC 8785 specifies.
///
/// # Errors
///
/// [`NotCanonical`] when the value contains something outside the supported
/// subset. There is no lossy path.
pub fn canonicalise(value: &Value) -> Result<String, NotCanonical> {
    let mut out = String::new();
    write_value(value, &mut out)?;
    Ok(out)
}

fn write_value(value: &Value, out: &mut String) -> Result<(), NotCanonical> {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(n) => {
            // Integers only, and both signednesses, because `serde_json` stores
            // them separately and a value above `i64::MAX` is a `u64`.
            if let Some(i) = n.as_i64() {
                let _ = write!(out, "{i}");
            } else if let Some(u) = n.as_u64() {
                let _ = write!(out, "{u}");
            } else {
                return Err(NotCanonical::NotAnInteger {
                    value: n.to_string(),
                });
            }
        }
        Value::String(s) => write_string(s, out),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(item, out)?;
            }
            out.push(']');
        }
        Value::Object(map) => {
            // Sorted by UTF-16 code unit, which is what RFC 8785 specifies --
            // *not* by UTF-8 bytes. The two agree for everything below the BMP
            // and disagree for supplementary-plane characters, so a byte sort
            // would be right for every key Radar sends today and wrong for the
            // first one that is not ASCII. Cheap to do correctly now.
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_by_key(|k| k.encode_utf16().collect::<Vec<u16>>());

            out.push('{');
            for (i, key) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_string(key, out);
                out.push(':');
                write_value(&map[key], out)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

/// Writes a JSON string with RFC 8785's escaping.
///
/// The two-character escapes where they exist, `\u00xx` for the remaining
/// control characters, and everything else literal. Notably `/` is **not**
/// escaped and non-ASCII characters are **not** escaped — both are places a
/// hand-rolled encoder tends to differ from the specification, and either would
/// change the signed bytes.
fn write_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn keys_are_sorted_and_there_is_no_whitespace() {
        // The two properties the whole thing turns on. Privy rebuilds this
        // payload on its side, and a different key order or a single space is a
        // different signature.
        let value = json!({"version": 1, "method": "POST", "url": "https://api.privy.io/v1"});
        assert_eq!(
            canonicalise(&value).expect("canonicalises"),
            r#"{"method":"POST","url":"https://api.privy.io/v1","version":1}"#
        );
    }

    #[test]
    fn nested_objects_are_sorted_at_every_level() {
        // A sort applied only at the top would pass a flat test and produce the
        // wrong bytes for every real request, because the body is nested.
        let value = json!({
            "body": {"params": {"encoding": "base64", "transaction": "AQAB"}, "method": "signTransaction"},
            "headers": {"privy-app-id": "app"},
        });
        assert_eq!(
            canonicalise(&value).expect("canonicalises"),
            r#"{"body":{"method":"signTransaction","params":{"encoding":"base64","transaction":"AQAB"}},"headers":{"privy-app-id":"app"}}"#
        );
    }

    #[test]
    fn a_float_is_refused_rather_than_approximated() {
        // The deliberate hole in the subset. RFC 8785 numbers are ECMAScript's
        // shortest-round-trip serialisation; guessing at it would produce a
        // signature Privy rejects on some payloads and not others, which is the
        // kind of failure that gets blamed on the vendor for a week.
        let value = json!({"amount": 1.5});
        assert!(matches!(
            canonicalise(&value),
            Err(NotCanonical::NotAnInteger { .. })
        ));
    }

    #[test]
    fn integers_of_both_signednesses_survive() {
        // `serde_json` stores signed and unsigned separately, and a value above
        // `i64::MAX` arrives as a `u64`. Reading only `as_i64` would refuse it
        // as a non-integer, which it plainly is not.
        let value = json!({"big": u64::MAX, "negative": -1, "zero": 0});
        assert_eq!(
            canonicalise(&value).expect("canonicalises"),
            format!(r#"{{"big":{},"negative":-1,"zero":0}}"#, u64::MAX)
        );
    }

    #[test]
    fn strings_are_escaped_the_way_the_specification_says_and_no_further() {
        // Both directions matter. Escaping too little changes the bytes; so does
        // escaping too much, and `/` and non-ASCII are exactly where a
        // hand-rolled encoder tends to over-escape.
        let value = json!({"s": "a\"b\\c\nd\te\u{1}f/g\u{e9}"});
        assert_eq!(
            canonicalise(&value).expect("canonicalises"),
            "{\"s\":\"a\\\"b\\\\c\\nd\\te\\u0001f/g\u{e9}\"}"
        );
    }

    #[test]
    fn the_specifications_own_string_example_matches() {
        // The escaping case taken from RFC 8785 itself rather than invented,
        // because an encoder tested only against its own output is tested
        // against nothing. Every awkward case is here at once: a supplementary
        // character, a raw control character, a newline, an apostrophe, a
        // quote, a backslash and a solidus.
        //
        // The specification's numeric example is deliberately not included. Its
        // point is float serialisation, which this canonicaliser refuses by
        // design, and casting those floats to integers to make them fit would be
        // a test that looks like the specification's and checks something else.
        let value = json!({
            "string": "\u{20ac}$\u{000f}\u{000a}A'\u{0042}\u{0022}\u{005c}\\\"/",
            "literals": [null, true, false],
        });
        assert_eq!(
            canonicalise(&value).expect("canonicalises"),
            "{\"literals\":[null,true,false],\"string\":\"\u{20ac}$\\u000f\\nA'B\\\"\\\\\\\\\\\"/\"}"
        );
    }

    #[test]
    fn arrays_keep_their_order() {
        // Sorting is for object keys only. An array is ordered data and sorting
        // one would change the meaning, not merely the encoding.
        let value = json!({"a": [3, 1, 2]});
        assert_eq!(
            canonicalise(&value).expect("canonicalises"),
            r#"{"a":[3,1,2]}"#
        );
    }
}
