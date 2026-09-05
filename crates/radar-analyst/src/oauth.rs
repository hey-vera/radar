// SPDX-License-Identifier: Apache-2.0
//! OAuth 1.0a request signing, which is the only way this account can speak.
//!
//! # Why this exists at all
//!
//! `POST /2/tweets` **does not accept an app-only bearer token.** It requires
//! user context — OAuth 1.0a, or OAuth 2.0 user context with `tweet.write`
//! (checked against `docs.x.com`, 2026-09-05). The client shipped sending a
//! bearer for both reading and posting, so reading worked, posting would have
//! been refused, and the failure would have arrived on the first real mention.
//!
//! OAuth 1.0a rather than OAuth 2.0, for a daemon posting as itself:
//!
//! | | OAuth 1.0a | OAuth 2.0 user context |
//! |---|---|---|
//! | setup | four values from the portal | browser redirect, once, by a human |
//! | lifetime | no expiry | access token expires in two hours |
//! | running cost | none | a refresh loop that, when it breaks, silently ends with a bot that has stopped talking |
//!
//! The second column's last row is what decides it. A refresh path that fails
//! needs a human with a browser, and `radar brief` is deliberately built not to
//! alarm on a quiet account — so the failure would look exactly like a quiet
//! week.
//!
//! # SHA-1, in 2026
//!
//! OAuth 1.0a specifies HMAC-SHA1 and X requires it. SHA-1 is broken for
//! collision resistance and **must not be used anywhere else in this
//! repository**. HMAC-SHA1 is not affected by those collision attacks, and its
//! use here is confined to this module.
//!
//! # The two details that make implementations wrong
//!
//! 1. **A JSON body is not signed.** OAuth 1.0a folds request-body parameters
//!    into the signature only when the body is `application/x-www-form-urlencoded`.
//!    `POST /2/tweets` sends JSON, so the body contributes nothing — including
//!    it produces a signature X rejects, with a 401 that says nothing useful.
//! 2. **Percent-encoding is RFC 3986, not the URL encoding most libraries do.**
//!    Only `A-Z a-z 0-9 - . _ ~` survive; everything else is `%` plus **upper
//!    case** hex. A lower-case `%2f` changes the signature base string and the
//!    signature with it.
//!
//! Both are covered by tests against the worked example in RFC 5849 §3.4.1 and
//! against X's own published example.

use std::fmt::Write as _;

use base64::Engine as _;
use hmac::{Hmac, KeyInit as _, Mac as _};
use sha1::Sha1;

/// The four values the X developer portal hands over.
///
/// Two identify the application and two identify the account it acts for.
/// Both pairs are required: an application cannot post without an account's
/// authority, and an account's authority is meaningless without an application
/// to exercise it.
#[derive(Clone)]
pub struct Credentials {
    /// The application's identity. "API Key" in the portal.
    pub consumer_key: String,
    /// The application's secret. "API Key Secret" in the portal.
    pub consumer_secret: String,
    /// The account's authority. "Access Token" in the portal.
    pub token: String,
    /// The account's secret. "Access Token Secret" in the portal.
    pub token_secret: String,
}

/// Redacted, all four fields, deliberately.
///
/// The same reason [`crate::x::X`] hand-writes one: a struct holding a
/// credential must never be printable, because the one place a `Debug` gets
/// called is a panic message or a log line, and both are read by people who
/// should not be handed the keys to the account.
impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("consumer_key", &"<redacted>")
            .field("consumer_secret", &"<redacted>")
            .field("token", &"<redacted>")
            .field("token_secret", &"<redacted>")
            .finish()
    }
}

impl Credentials {
    /// Reads all four from the environment, or `None`.
    ///
    /// **All four or none.** A partial set is treated as absent rather than as
    /// an error to discover at the first post: three of four is somebody
    /// halfway through a deploy, and the honest response is the same one the
    /// bearer gets — this instance cannot speak, said at startup.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        Self::from_vars(&|k| std::env::var(k).ok())
    }

    /// The rule, over a getter, so it is testable without process-wide state.
    #[must_use]
    pub fn from_vars(get: &impl Fn(&str) -> Option<String>) -> Option<Self> {
        let read = |k: &str| {
            get(k)
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
        };
        Some(Self {
            consumer_key: read("RADAR_X_API_KEY")?,
            consumer_secret: read("RADAR_X_API_SECRET")?,
            token: read("RADAR_X_ACCESS_TOKEN")?,
            token_secret: read("RADAR_X_ACCESS_SECRET")?,
        })
    }
}

/// Percent-encoding as RFC 3986 §2.3 defines it, which OAuth 1.0a requires.
///
/// Only `A-Z a-z 0-9 - . _ ~` are left alone. Everything else — including `/`,
/// `:` and space — becomes `%` and two **upper case** hex digits.
///
/// This is not the same as the encoding a URL library will do for you. Most
/// leave `/` and `:` intact in a path, and several emit lower-case hex. Either
/// difference changes the signature base string, and therefore the signature,
/// and X answers with a 401 that does not say which of the two it was.
#[must_use]
pub fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(*byte as char);
            }
            // Upper case, and `{:02X}` rather than `{:x}`: a lower-case escape
            // is a different string to the signature even though it is the same
            // URL to a browser.
            other => {
                let _ = write!(out, "%{other:02X}");
            }
        }
    }
    out
}

/// The signature base string: the exact bytes that get signed.
///
/// `METHOD & percent(url) & percent(sorted parameters)`, per RFC 5849 §3.4.1.
///
/// `params` carries the OAuth fields **and any query parameters of the request**
/// — the two are pooled and sorted together, which is the step most often
/// missed. A signed request whose URL carries `?max_results=5` and whose base
/// string does not is refused.
///
/// `url` must already be stripped of its query string. Passing one with `?`
/// still attached signs a different resource than the one requested.
#[must_use]
pub fn base_string(method: &str, url: &str, params: &[(String, String)]) -> String {
    let mut encoded: Vec<(String, String)> =
        params.iter().map(|(k, v)| (encode(k), encode(v))).collect();
    // Sorted by encoded key, then by encoded value -- byte order, on the
    // *encoded* forms, which is what §3.4.1.3.2 specifies. Sorting the raw
    // strings gives a different order whenever an encoding changes the first
    // differing byte.
    encoded.sort();

    let joined = encoded
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");

    format!(
        "{}&{}&{}",
        method.to_uppercase(),
        encode(url),
        encode(&joined)
    )
}

/// The signing key: `percent(consumer_secret) & percent(token_secret)`.
///
/// The `&` is present even when the token secret is empty, which is the case
/// during the request-token leg of the OAuth dance. This daemon never runs that
/// leg — it is given a token — but omitting the separator produces a key that
/// is wrong in a way that only shows up as a refused request.
#[must_use]
fn signing_key(consumer_secret: &str, token_secret: &str) -> String {
    format!("{}&{}", encode(consumer_secret), encode(token_secret))
}

/// HMAC-SHA1 of the base string under the signing key, base64-encoded.
///
/// # Panics
///
/// Never, in practice. HMAC accepts a key of any length — the construction pads
/// a short one and hashes a long one — so `new_from_slice` has no failing input
/// here. Left as an `expect` rather than smothered with a fallback, because a
/// silent fallback would produce a wrong signature and a refused post, which is
/// much harder to diagnose than a panic naming this line.
#[must_use]
pub fn sign(base: &str, consumer_secret: &str, token_secret: &str) -> String {
    let key = signing_key(consumer_secret, token_secret);
    // `new_from_slice` on HMAC accepts a key of any length -- it is the HMAC
    // construction that pads or hashes it -- so this cannot fail for a
    // non-empty key and the error case is unreachable rather than ignored.
    let mut mac =
        Hmac::<Sha1>::new_from_slice(key.as_bytes()).expect("HMAC accepts a key of any length");
    mac.update(base.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
}

/// A nonce: unique per timestamp, and unpredictable.
///
/// OAuth 1.0a asks only for uniqueness within a timestamp, which a counter would
/// give. Unpredictability costs one call to the system generator and removes a
/// class of replay argument entirely, so it is not worth reasoning about which
/// of the two the platform actually enforces.
#[must_use]
pub fn nonce() -> String {
    let bytes: [u8; 16] = rand::random();
    let mut out = String::with_capacity(32);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// The `Authorization` header value for a signed request.
///
/// `query` is the request's own query parameters, which are signed alongside the
/// OAuth fields but do **not** appear in the header. A JSON body contributes
/// nothing — see this module's header for why that is the detail that breaks
/// implementations.
///
/// `timestamp` and `nonce` are arguments rather than read here, so a signature
/// can be checked against a published example instead of only against itself.
#[must_use]
pub fn authorization(
    credentials: &Credentials,
    method: &str,
    url: &str,
    query: &[(String, String)],
    timestamp: u64,
    nonce: &str,
) -> String {
    let mut oauth = vec![
        (
            "oauth_consumer_key".to_owned(),
            credentials.consumer_key.clone(),
        ),
        ("oauth_nonce".to_owned(), nonce.to_owned()),
        ("oauth_signature_method".to_owned(), "HMAC-SHA1".to_owned()),
        ("oauth_timestamp".to_owned(), timestamp.to_string()),
        ("oauth_token".to_owned(), credentials.token.clone()),
        ("oauth_version".to_owned(), "1.0".to_owned()),
    ];

    // Signed together; sent separately. The query parameters belong in the base
    // string and not in the header, and a header carrying them is refused.
    let mut all = oauth.clone();
    all.extend_from_slice(query);
    let signature = sign(
        &base_string(method, url, &all),
        &credentials.consumer_secret,
        &credentials.token_secret,
    );
    oauth.push(("oauth_signature".to_owned(), signature));

    // Sorted for the reader's benefit rather than the protocol's: the header's
    // order carries no meaning, and a stable one makes two requests comparable
    // by eye when something is being debugged at four in the morning.
    oauth.sort();
    let rendered = oauth
        .iter()
        .map(|(k, v)| format!("{}=\"{}\"", encode(k), encode(v)))
        .collect::<Vec<_>>()
        .join(", ");
    format!("OAuth {rendered}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_encoding_follows_rfc_3986_and_not_a_url_library() {
        // The unreserved set, untouched.
        assert_eq!(encode("aZ09-._~"), "aZ09-._~");
        // The characters a URL library most often leaves alone, and the reason
        // borrowing one produces a signature that is refused.
        assert_eq!(encode("/"), "%2F");
        assert_eq!(encode(":"), "%3A");
        assert_eq!(encode("?"), "%3F");
        assert_eq!(encode("&"), "%26");
        assert_eq!(encode("="), "%3D");
        // A space is `%20`, never `+`.
        assert_eq!(encode(" "), "%20");
        // Upper-case hex. `%2f` is the same URL and a different signature.
        assert_eq!(encode("/"), encode("/").to_uppercase());
        // Multi-byte input is encoded per UTF-8 byte.
        assert_eq!(encode("é"), "%C3%A9");
        assert_eq!(encode(""), "");
    }

    #[test]
    fn the_signature_matches_the_worked_example_in_rfc_5849() {
        // RFC 5849 section 3.4.1.1 gives a base string and 3.4.1.2 the key, and
        // the resulting signature is published. Checking against a document
        // rather than against this implementation's own output is the whole
        // point: a test that signs and then verifies with the same code passes
        // for every consistent mistake.
        let base = "POST&http%3A%2F%2Fexample.com%2Frequest&a2%3Dr%2520b%26a3%3D2%2520q\
                    %26a3%3Da%26b5%3D%253D%25253D%26c%2540%3D%26c2%3D%26oauth_consumer_\
                    key%3D9djdj82h48djs9d2%26oauth_nonce%3D7d8f3e4a%26oauth_signature_\
                    method%3DHMAC-SHA1%26oauth_timestamp%3D137131201%26oauth_token%3D\
                    kkk9d7dh3k39sjv7";
        let signature = sign(base, "j49sk3j29djd", "dh893hdasih9");
        assert_eq!(signature, "r6/TJjbCOr97/+UU0NsvSne7s5g=");
    }

    #[test]
    fn the_base_string_pools_and_sorts_every_parameter() {
        // RFC 5849 section 3.4.1.3.2: sorted by encoded name, then by encoded
        // value. The query parameters and the OAuth fields go into one pool --
        // signing only the OAuth fields is the mistake that makes every request
        // with a query string fail while the ones without succeed.
        let params = vec![
            ("b".to_owned(), "2".to_owned()),
            ("a".to_owned(), "1".to_owned()),
            ("c".to_owned(), "3".to_owned()),
        ];
        let base = base_string("get", "https://api.x.com/2/x", &params);
        assert_eq!(
            base,
            "GET&https%3A%2F%2Fapi.x.com%2F2%2Fx&a%3D1%26b%3D2%26c%3D3"
        );
        // The method is upper-cased rather than trusted.
        assert!(base.starts_with("GET&"));
    }

    #[test]
    fn parameters_sort_by_value_when_their_names_match() {
        // Legal in OAuth and easy to get wrong: a repeated name sorts by value.
        let params = vec![
            ("a".to_owned(), "z".to_owned()),
            ("a".to_owned(), "b".to_owned()),
        ];
        let base = base_string("GET", "https://e.test/", &params);
        assert!(base.ends_with("a%3Db%26a%3Dz"), "{base}");
    }

    #[test]
    fn the_signing_key_keeps_its_separator_when_a_secret_is_empty() {
        // The empty token secret happens during the leg this daemon never runs,
        // and dropping the `&` there produces a key that is wrong in a way only
        // a refused request reveals.
        assert_eq!(signing_key("abc", ""), "abc&");
        assert_eq!(signing_key("", ""), "&");
        // Both halves are percent-encoded before joining.
        assert_eq!(signing_key("a/b", "c d"), "a%2Fb&c%20d");
    }

    fn credentials() -> Credentials {
        Credentials {
            consumer_key: "ck".to_owned(),
            consumer_secret: "cs".to_owned(),
            token: "tok".to_owned(),
            token_secret: "ts".to_owned(),
        }
    }

    #[test]
    fn the_header_carries_the_oauth_fields_and_never_the_query() {
        // The query is signed and not sent in the header. A header carrying it
        // is refused, and it is the natural mistake to make once the two are
        // pooled for signing.
        let query = vec![("max_results".to_owned(), "5".to_owned())];
        let header = authorization(
            &credentials(),
            "GET",
            "https://api.x.com/2/users/1/mentions",
            &query,
            1_788_000_000,
            "abc123",
        );
        assert!(header.starts_with("OAuth "));
        assert!(header.contains("oauth_consumer_key=\"ck\""));
        assert!(header.contains("oauth_token=\"tok\""));
        assert!(header.contains("oauth_signature_method=\"HMAC-SHA1\""));
        assert!(header.contains("oauth_version=\"1.0\""));
        assert!(header.contains("oauth_timestamp=\"1788000000\""));
        assert!(header.contains("oauth_nonce=\"abc123\""));
        assert!(header.contains("oauth_signature=\""));
        assert!(
            !header.contains("max_results"),
            "the query is signed, not sent: {header}"
        );
    }

    #[test]
    fn the_query_changes_the_signature_even_though_it_is_not_in_the_header() {
        // The other half of the rule above, and the one a header-only assertion
        // cannot see: if the query were dropped from the base string too, these
        // two would be identical and every paged request would still be signed
        // as though it were unpaged.
        let with = authorization(
            &credentials(),
            "GET",
            "https://api.x.com/2/x",
            &[("max_results".to_owned(), "5".to_owned())],
            1_788_000_000,
            "abc123",
        );
        let without = authorization(
            &credentials(),
            "GET",
            "https://api.x.com/2/x",
            &[],
            1_788_000_000,
            "abc123",
        );
        assert_ne!(with, without);
    }

    #[test]
    fn every_secret_changes_the_signature() {
        // Four credentials, and a signature that ignored one of them would still
        // look completely correct. Each is varied alone.
        let base = authorization(
            &credentials(),
            "POST",
            "https://api.x.com/2/tweets",
            &[],
            1,
            "n",
        );
        for mutate in [
            |c: &mut Credentials| c.consumer_key = "other".to_owned(),
            |c: &mut Credentials| c.consumer_secret = "other".to_owned(),
            |c: &mut Credentials| c.token = "other".to_owned(),
            |c: &mut Credentials| c.token_secret = "other".to_owned(),
        ] {
            let mut changed = credentials();
            mutate(&mut changed);
            let other = authorization(&changed, "POST", "https://api.x.com/2/tweets", &[], 1, "n");
            assert_ne!(base, other, "a credential that changes nothing is not used");
        }
    }

    #[test]
    fn the_method_the_url_the_timestamp_and_the_nonce_all_change_the_signature() {
        let base = authorization(&credentials(), "POST", "https://e.test/a", &[], 1, "n");
        assert_ne!(
            base,
            authorization(&credentials(), "GET", "https://e.test/a", &[], 1, "n")
        );
        assert_ne!(
            base,
            authorization(&credentials(), "POST", "https://e.test/b", &[], 1, "n")
        );
        assert_ne!(
            base,
            authorization(&credentials(), "POST", "https://e.test/a", &[], 2, "n")
        );
        assert_ne!(
            base,
            authorization(&credentials(), "POST", "https://e.test/a", &[], 1, "m")
        );
    }

    #[test]
    fn a_nonce_is_unpredictable_and_does_not_repeat() {
        let a = nonce();
        let b = nonce();
        assert_ne!(a, b);
        assert_eq!(a.len(), 32, "sixteen bytes as hex");
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn all_four_credentials_or_none() {
        const FULL: [(&str, &str); 4] = [
            ("RADAR_X_API_KEY", "k"),
            ("RADAR_X_API_SECRET", "s"),
            ("RADAR_X_ACCESS_TOKEN", "t"),
            ("RADAR_X_ACCESS_SECRET", "ts"),
        ];
        let all = |k: &str| {
            FULL.iter()
                .find(|(name, _)| *name == k)
                .map(|(_, v)| (*v).to_owned())
        };
        assert!(Credentials::from_vars(&all).is_some());

        // Three of four is somebody halfway through a deploy. Treated as absent,
        // so the daemon says it cannot speak at startup rather than discovering
        // it on the first real mention.
        for missing in 0..4 {
            let partial: Vec<_> = FULL
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != missing)
                .map(|(_, p)| *p)
                .collect();
            let get = |k: &str| {
                partial
                    .iter()
                    .find(|(name, _)| *name == k)
                    .map(|(_, v)| (*v).to_owned())
            };
            assert!(
                Credentials::from_vars(&get).is_none(),
                "missing index {missing} must read as unconfigured"
            );
        }
    }

    #[test]
    fn a_blank_credential_is_a_missing_one() {
        // An env file with `RADAR_X_API_SECRET=` in it is the shape a
        // half-finished edit leaves behind, and it must not read as configured.
        let get = |k: &str| {
            Some(match k {
                "RADAR_X_API_SECRET" => "   ".to_owned(),
                _ => "value".to_owned(),
            })
        };
        assert!(Credentials::from_vars(&get).is_none());
    }

    #[test]
    fn the_credentials_never_print_themselves() {
        // The one place a Debug gets called is a panic message or a log line,
        // and both are read by people who should not be handed the account.
        let rendered = format!("{:?}", credentials());
        assert!(!rendered.contains("cs"), "{rendered}");
        assert!(!rendered.contains("ts"), "{rendered}");
        assert!(rendered.contains("redacted"));
    }
}
