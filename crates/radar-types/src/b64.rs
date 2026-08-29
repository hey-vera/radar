// SPDX-License-Identifier: Apache-2.0
//! Base64, because transactions arrive that way.
//!
//! Hand-rolled and local rather than shared with the identical routine in
//! `radar-sim`. That crate makes HTTP calls, and a dependency edge from the
//! signer to anything that can open a socket would weaken a property the deploy
//! notes state plainly: the signer has no network. Fifty lines of base64 is a
//! cheaper price than that edge.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// The URL-safe alphabet, which JSON Web Tokens use.
///
/// A separate alphabet rather than a translation step, so that `+` and `/` are
/// *refused* in a URL-safe input rather than quietly accepted. Two spellings of
/// the same token is one more thing an attacker can vary, and a verifier that
/// accepts both is a verifier whose input is not canonical.
const URL_ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Decodes standard base64, returning `None` on any character outside the
/// alphabet.
///
/// Refusing rather than skipping. A decoder that ignores stray bytes hands the
/// transaction parser a buffer that differs from what the sender wrote, and
/// every guarantee downstream is about the bytes that were read.
#[must_use]
pub fn decode(s: &str) -> Option<Vec<u8>> {
    decode_with(ALPHABET, s, true)
}

/// Decodes unpadded URL-safe base64, the encoding JSON Web Tokens use.
///
/// Stricter than [`decode`] in two ways, both because the caller is a signature
/// verifier rather than a transaction reader. Whitespace is refused rather than
/// skipped, and `=` padding is refused rather than terminating the input: a JWT
/// is defined as unpadded, so a padded one is not a JWT, and a verifier that
/// accepts several spellings of one token is a verifier whose input is not
/// canonical. That is the ground the interesting attacks are built on.
#[must_use]
pub fn decode_url(s: &str) -> Option<Vec<u8>> {
    decode_with(URL_ALPHABET, s, false)
}

/// The shared routine.
///
/// `lenient` is the pre-existing behaviour of [`decode`], kept exactly as it
/// was: whitespace skipped and `=` ending the input. Changing it would change
/// what the signer accepts, which is not a thing to do in passing.
fn decode_with(alphabet: &[u8; 64], s: &str, lenient: bool) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let mut buffer = 0u32;
    let mut bits = 0u32;

    for byte in s.bytes() {
        if lenient {
            if byte == b'=' {
                break;
            }
            if byte.is_ascii_whitespace() {
                continue;
            }
        }
        let value = alphabet.iter().position(|c| *c == byte)?;
        buffer = (buffer << 6) | u32::try_from(value).ok()?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((buffer >> bits) & 0xFF).unwrap_or(0));
        }
    }
    Some(out)
}

/// Encodes standard base64 with padding.
#[must_use]
pub fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                let index = usize::try_from((n >> (18 - i * 6)) & 0x3F).unwrap_or(0);
                out.push(char::from(ALPHABET[index]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_byte_survives_a_round_trip() {
        // A transaction is read at fixed offsets, so a decoder that is subtly
        // wrong shifts every field rather than failing visibly.
        let original: Vec<u8> = (0u8..=255).collect();
        assert_eq!(decode(&encode(&original)).as_deref(), Some(&original[..]));
    }

    #[test]
    fn every_length_class_round_trips() {
        // The three padding cases, which is where hand-rolled base64 goes wrong.
        for len in 0..16usize {
            let data: Vec<u8> = (0..len).map(|i| u8::try_from(i).unwrap_or(0)).collect();
            assert_eq!(
                decode(&encode(&data)).as_deref(),
                Some(&data[..]),
                "len {len}"
            );
        }
    }

    #[test]
    fn known_vectors_match() {
        assert_eq!(encode(b"A"), "QQ==");
        assert_eq!(encode(b"AB"), "QUI=");
        assert_eq!(encode(b"ABC"), "QUJD");
        assert_eq!(decode("QUJD").as_deref(), Some(&b"ABC"[..]));
    }

    #[test]
    fn characters_outside_the_alphabet_are_refused() {
        assert_eq!(decode("QU!D"), None);
        assert_eq!(decode("****"), None);
    }
}
