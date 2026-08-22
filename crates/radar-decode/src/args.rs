// SPDX-License-Identifier: Apache-2.0
//! Instruction arguments, with the unit carried in the type.
//!
//! pump.fun's trade instructions all take two `u64`s, and **the two fields swap
//! meaning between variants**:
//!
//! | Instruction | first `u64` | second `u64` |
//! |---|---|---|
//! | `buy`, `buy_v2` | token amount | max SOL to spend |
//! | `sell`, `sell_v2` | token amount | min SOL to accept |
//! | `buy_exact_sol_in` | **SOL to spend** | min tokens to accept |
//! | `buy_exact_quote_in_v2` | **quote amount in** | min tokens to accept |
//!
//! A decoder that assumed one layout for all of them would read a token amount
//! as lamports. Measured on real payloads that is not a small error: the median
//! `buy_v2` first field is 3.6 trillion, which read as lamports is 3,614 SOL
//! against a true spend of about 0.007. Every downstream size, cost and P&L
//! figure would be wrong by six orders of magnitude, and none of them would look
//! obviously broken.
//!
//! So the unit lives in [`Amount`] rather than in a comment, and there is no way
//! to read lamports out of a token field.

use core::fmt;

use radar_types::Address;

/// A quantity, tagged with what it counts.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum Amount {
    /// Base units of the token. pump.fun mints use six decimals.
    Tokens(u64),
    /// Lamports.
    Lamports(u64),
}

impl Amount {
    /// The raw count, whatever it counts. Prefer [`tokens`](Self::tokens) or
    /// [`lamports`](Self::lamports), which cannot be confused.
    #[must_use]
    pub const fn raw(self) -> u64 {
        match self {
            Self::Tokens(v) | Self::Lamports(v) => v,
        }
    }

    /// The value if it is a token quantity.
    #[must_use]
    pub const fn tokens(self) -> Option<u64> {
        match self {
            Self::Tokens(v) => Some(v),
            Self::Lamports(_) => None,
        }
    }

    /// The value if it is lamports.
    #[must_use]
    pub const fn lamports(self) -> Option<u64> {
        match self {
            Self::Lamports(v) => Some(v),
            Self::Tokens(_) => None,
        }
    }

    /// Whether this is the "no limit" sentinel. `u64::MAX` appears as a max-cost
    /// bound and zero as a min-output bound; both mean the trader accepted any
    /// price, which is a meaningful behavioural signal rather than missing data.
    #[must_use]
    pub const fn is_unbounded(self) -> bool {
        matches!(self.raw(), 0 | u64::MAX)
    }
}

impl fmt::Display for Amount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tokens(v) => write!(f, "{v} tokens"),
            Self::Lamports(v) => write!(f, "{v} lamports"),
        }
    }
}

/// Which way a trade goes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Side {
    /// Acquiring tokens.
    Buy,
    /// Disposing of tokens.
    Sell,
}

/// A decoded trade instruction.
///
/// `exact` is the side the trader pinned; `limit` is the bound they accepted on
/// the other side. Which is which depends on the instruction variant, which is
/// exactly why both carry their unit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Trade {
    /// Buy or sell.
    pub side: Side,
    /// The quantity the trader specified exactly.
    pub exact: Amount,
    /// The bound accepted on the other side.
    pub limit: Amount,
}

impl Trade {
    /// SOL involved, if the instruction pinned it exactly.
    ///
    /// `None` for a token-exact trade, where the SOL figure is a bound rather
    /// than an outcome and the realised amount can only be read from the
    /// transaction's balance changes or its emitted event.
    #[must_use]
    pub const fn exact_lamports(self) -> Option<u64> {
        self.exact.lamports()
    }

    /// Tokens involved, if the instruction pinned them exactly.
    #[must_use]
    pub const fn exact_tokens(self) -> Option<u64> {
        self.exact.tokens()
    }

    /// Whether the trader accepted any price at all — an unbounded max cost on a
    /// buy or a zero minimum output on a sell. Common in panic exits and in
    /// automated flow that does not care about slippage.
    #[must_use]
    pub const fn accepted_any_price(self) -> bool {
        self.limit.is_unbounded()
    }
}

/// A decoded launch instruction.
///
/// Borrows from the instruction data rather than allocating: the recorder decodes
/// millions of these and most are discarded by the Tier-0 filter before anything
/// is kept.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Launch<'a> {
    /// Token name, as supplied by the creator. **Untrusted**: arbitrary
    /// creator-controlled text that must never reach an instruction position.
    pub name: &'a str,
    /// Token symbol. Untrusted, same as the name.
    pub symbol: &'a str,
    /// Metadata URI. Untrusted, and never fetched automatically.
    pub uri: &'a str,
    /// The creator address recorded in the instruction.
    pub creator: Address,
}

/// Why an instruction's arguments could not be read.
#[derive(Clone, Copy, PartialEq, Eq, Debug, thiserror::Error)]
pub enum ArgError {
    /// The payload ended before a field did.
    #[error("payload ended at {at} while reading {field}, need {need} more bytes")]
    Truncated {
        /// Field being read.
        field: &'static str,
        /// Offset reached.
        at: usize,
        /// Bytes still required.
        need: usize,
    },
    /// A length-prefixed string claimed more bytes than the payload holds.
    #[error("{field} claims {claimed} bytes but only {available} remain")]
    ImplausibleLength {
        /// Field being read.
        field: &'static str,
        /// Length the payload claimed.
        claimed: usize,
        /// Bytes actually left.
        available: usize,
    },
    /// A string field was not valid UTF-8.
    #[error("{field} is not valid UTF-8")]
    NotUtf8 {
        /// Field being read.
        field: &'static str,
    },
}

/// A cursor that reads Borsh-ish little-endian fields without panicking.
struct Reader<'a> {
    buf: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    const fn new(buf: &'a [u8], at: usize) -> Self {
        Self { buf, at }
    }

    fn take(&mut self, n: usize, field: &'static str) -> Result<&'a [u8], ArgError> {
        let end = self.at.checked_add(n).ok_or(ArgError::Truncated {
            field,
            at: self.at,
            need: n,
        })?;
        if end > self.buf.len() {
            return Err(ArgError::Truncated {
                field,
                at: self.at,
                need: end - self.buf.len(),
            });
        }
        let out = &self.buf[self.at..end];
        self.at = end;
        Ok(out)
    }

    fn u32(&mut self, field: &'static str) -> Result<u32, ArgError> {
        let b = self.take(4, field)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self, field: &'static str) -> Result<u64, ArgError> {
        let b = self.take(8, field)?;
        Ok(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    fn string(&mut self, field: &'static str) -> Result<&'a str, ArgError> {
        let len = self.u32(field)? as usize;
        let available = self.buf.len().saturating_sub(self.at);
        if len > available {
            return Err(ArgError::ImplausibleLength {
                field,
                claimed: len,
                available,
            });
        }
        let bytes = self.take(len, field)?;
        core::str::from_utf8(bytes).map_err(|_| ArgError::NotUtf8 { field })
    }

    fn address(&mut self, field: &'static str) -> Result<Address, ArgError> {
        let b = self.take(32, field)?;
        let mut out = [0u8; 32];
        out.copy_from_slice(b);
        Ok(Address::new(out))
    }
}

/// How a trade instruction orders its two `u64` arguments.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Layout {
    /// First field is a token amount, second is a SOL bound.
    TokensThenSolBound,
    /// First field is a SOL amount, second is a token bound.
    SolThenTokenBound,
}

/// Reads a trade instruction's arguments.
///
/// # Errors
///
/// Returns [`ArgError::Truncated`] if the payload is shorter than two `u64`s
/// after the discriminator.
pub fn trade(data: &[u8], side: Side, layout: Layout) -> Result<Trade, ArgError> {
    let mut r = Reader::new(data, 8);
    let first = r.u64("first argument")?;
    let second = r.u64("second argument")?;
    let (exact, limit) = match layout {
        Layout::TokensThenSolBound => (Amount::Tokens(first), Amount::Lamports(second)),
        Layout::SolThenTokenBound => (Amount::Lamports(first), Amount::Tokens(second)),
    };
    Ok(Trade { side, exact, limit })
}

/// Reads a launch instruction's arguments: three Borsh strings then the creator.
///
/// # Errors
///
/// Returns [`ArgError`] if the payload is truncated, a string length is
/// implausible, or a string is not valid UTF-8. Creator-supplied text is
/// arbitrary bytes and a malformed launch must not take the recorder down.
pub fn launch(data: &[u8]) -> Result<Launch<'_>, ArgError> {
    let mut r = Reader::new(data, 8);
    let name = r.string("name")?;
    let symbol = r.string("symbol")?;
    let uri = r.string("uri")?;
    let creator = r.address("creator")?;
    Ok(Launch {
        name,
        symbol,
        uri,
        creator,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_amount_cannot_be_read_as_lamports() {
        // The whole point of the type. Median buy_v2 first field is ~3.6e12,
        // which read as lamports is 3,614 SOL against a true spend near 0.007.
        let a = Amount::Tokens(3_614_520_997_424);
        assert_eq!(a.lamports(), None);
        assert_eq!(a.tokens(), Some(3_614_520_997_424));
    }

    #[test]
    fn layout_decides_which_field_is_money() {
        let mut data = vec![0u8; 8];
        data.extend_from_slice(&1_000u64.to_le_bytes());
        data.extend_from_slice(&2_000u64.to_le_bytes());

        let t = trade(&data, Side::Buy, Layout::TokensThenSolBound).expect("decodes");
        assert_eq!(t.exact, Amount::Tokens(1_000));
        assert_eq!(t.limit, Amount::Lamports(2_000));
        assert_eq!(t.exact_lamports(), None);

        let t = trade(&data, Side::Buy, Layout::SolThenTokenBound).expect("decodes");
        assert_eq!(t.exact, Amount::Lamports(1_000));
        assert_eq!(t.limit, Amount::Tokens(2_000));
        assert_eq!(t.exact_lamports(), Some(1_000));
    }

    #[test]
    fn unbounded_limits_are_recognised_at_both_ends() {
        // Zero minimum output on a sell and u64::MAX max cost on a buy both mean
        // "any price", and both occur constantly in real flow.
        assert!(Amount::Lamports(0).is_unbounded());
        assert!(Amount::Lamports(u64::MAX).is_unbounded());
        assert!(!Amount::Lamports(1).is_unbounded());
    }

    #[test]
    fn a_truncated_trade_errors_rather_than_panicking() {
        assert!(matches!(
            trade(&[0u8; 12], Side::Buy, Layout::TokensThenSolBound),
            Err(ArgError::Truncated { .. })
        ));
        assert!(matches!(
            trade(&[], Side::Buy, Layout::TokensThenSolBound),
            Err(ArgError::Truncated { .. })
        ));
    }

    #[test]
    fn a_lying_string_length_errors_rather_than_reading_past_the_buffer() {
        // Creator-supplied bytes. A length prefix of 4 billion must not become an
        // out-of-bounds read or a multi-gigabyte allocation.
        let mut data = vec![0u8; 8];
        data.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(
            launch(&data),
            Err(ArgError::ImplausibleLength { .. })
        ));
    }

    #[test]
    fn non_utf8_names_error_rather_than_being_mangled() {
        let mut data = vec![0u8; 8];
        data.extend_from_slice(&2u32.to_le_bytes());
        data.extend_from_slice(&[0xff, 0xfe]);
        assert!(matches!(
            launch(&data),
            Err(ArgError::NotUtf8 { field: "name" })
        ));
    }

    #[test]
    fn a_well_formed_launch_round_trips() {
        let mut data = vec![0u8; 8];
        for s in ["Coin", "CN", "https://example.invalid/m.json"] {
            data.extend_from_slice(&u32::try_from(s.len()).expect("short").to_le_bytes());
            data.extend_from_slice(s.as_bytes());
        }
        data.extend_from_slice(&[7u8; 32]);

        let l = launch(&data).expect("decodes");
        assert_eq!(l.name, "Coin");
        assert_eq!(l.symbol, "CN");
        assert_eq!(l.uri, "https://example.invalid/m.json");
        assert_eq!(l.creator, Address::new([7u8; 32]));
    }
}
