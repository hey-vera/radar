// SPDX-License-Identifier: Apache-2.0
//! Solana addresses and transaction signatures.

use core::fmt;
use core::str::FromStr;

use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A 32-byte Solana account address.
///
/// Stored as raw bytes and rendered as base58, which is the only form a human or
/// an explorer will recognise. Comparison and hashing are on the bytes, so a
/// value that round-trips through base58 is still the same key.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Address([u8; 32]);

/// A 64-byte transaction signature, rendered as base58.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Signature([u8; 64]);

/// Why a base58 string could not be read as an address or signature.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AddressParseError {
    /// The string was not valid base58.
    #[error("not valid base58: {0}")]
    NotBase58(String),
    /// The decoded byte length was wrong for the target type.
    #[error("expected {expected} bytes, decoded {actual}")]
    WrongLength {
        /// Byte length the target type requires.
        expected: usize,
        /// Byte length actually decoded.
        actual: usize,
    },
}

impl Address {
    /// The all-zero address, which on Solana is the System Program.
    pub const SYSTEM_PROGRAM: Self = Self([0u8; 32]);

    /// Wraps raw bytes without validation. Any 32 bytes are a syntactically
    /// valid address, so this cannot fail.
    #[must_use]
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// The raw bytes.
    #[must_use]
    pub const fn to_bytes(self) -> [u8; 32] {
        self.0
    }

    /// The raw bytes, borrowed.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl Signature {
    /// Wraps raw bytes without validation.
    #[must_use]
    pub const fn new(bytes: [u8; 64]) -> Self {
        Self(bytes)
    }

    /// The raw bytes, borrowed.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 64] {
        &self.0
    }
}

fn decode_base58<const N: usize>(s: &str) -> Result<[u8; N], AddressParseError> {
    let mut out = [0u8; N];
    let written = bs58::decode(s)
        .onto(&mut out[..])
        .map_err(|_| AddressParseError::NotBase58(s.to_owned()))?;
    if written == N {
        Ok(out)
    } else {
        Err(AddressParseError::WrongLength {
            expected: N,
            actual: written,
        })
    }
}

impl FromStr for Address {
    type Err = AddressParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        decode_base58::<32>(s).map(Self)
    }
}

impl FromStr for Signature {
    type Err = AddressParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        decode_base58::<64>(s).map(Self)
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&bs58::encode(self.0).into_string())
    }
}

impl fmt::Display for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&bs58::encode(self.0.as_slice()).into_string())
    }
}

// Base58, not a byte array: a debug line naming an address is something a human
// pastes into an explorer, and 32 decimal numbers cannot be pasted anywhere.
impl fmt::Debug for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Address({self})")
    }
}

impl fmt::Debug for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Signature({self})")
    }
}

macro_rules! string_serde {
    ($t:ty, $name:literal) => {
        impl Serialize for $t {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.collect_str(self)
            }
        }
        impl<'de> Deserialize<'de> for $t {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = <std::borrow::Cow<'de, str>>::deserialize(d)?;
                s.parse().map_err(D::Error::custom)
            }
        }
        impl JsonSchema for $t {
            fn schema_name() -> std::borrow::Cow<'static, str> {
                $name.into()
            }
            fn json_schema(g: &mut SchemaGenerator) -> Schema {
                String::json_schema(g)
            }
        }
    };
}

string_serde!(Address, "Address");
string_serde!(Signature, "Signature");

#[cfg(test)]
mod tests {
    use super::*;

    const WSOL: &str = "So11111111111111111111111111111111111111112";

    #[test]
    fn address_round_trips_through_base58() {
        let a: Address = WSOL.parse().expect("valid address");
        assert_eq!(a.to_string(), WSOL);
    }

    #[test]
    fn address_round_trips_through_json() {
        let a: Address = WSOL.parse().expect("valid address");
        let json = serde_json::to_string(&a).expect("serialize");
        assert_eq!(json, format!("\"{WSOL}\""));
        let back: Address = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(a, back);
    }

    #[test]
    fn a_too_short_string_is_rejected_rather_than_padded() {
        // Silently left-padding a short decode is how a truncated address becomes
        // a valid-looking one that points somewhere else entirely.
        let err = "abc".parse::<Address>().expect_err("must reject");
        let AddressParseError::WrongLength { expected, actual } = err else {
            panic!("expected a length error, got {err:?}");
        };
        assert_eq!(expected, 32);
        assert!(
            actual < 32,
            "decoded {actual} bytes, which should not be a valid address"
        );
    }

    #[test]
    fn non_base58_is_rejected() {
        // `0`, `O`, `I` and `l` are excluded from the base58 alphabet precisely
        // because they are visually ambiguous.
        assert!(matches!(
            "0OIl".parse::<Address>(),
            Err(AddressParseError::NotBase58(_))
        ));
    }

    #[test]
    fn system_program_is_all_zeroes() {
        assert_eq!(
            Address::SYSTEM_PROGRAM.to_string(),
            "11111111111111111111111111111111"
        );
    }
}
