// SPDX-License-Identifier: Apache-2.0
//! Reading a mint account: authorities, and the Token-2022 extensions that
//! decide whether a token can be sold at all.
//!
//! This is the cheapest half of exit analysis and the half that catches the
//! worst outcomes. A transfer hook that reverts on sell, a permanent delegate
//! that can move tokens out from under a holder, a default-frozen account state
//! — none of these show up in a price quote. A quote will happily price a token
//! nobody can transfer.
//!
//! Verified against real mainnet accounts: pump.fun mints are Token-2022 with
//! both authorities revoked at creation and only metadata extensions, USDC is
//! classic SPL Token with both authorities live, and wrapped SOL is classic with
//! both revoked.

use radar_types::Address;
use serde::{Deserialize, Serialize};

/// The classic SPL Token program.
pub const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
/// The Token-2022 program.
pub const TOKEN_2022_PROGRAM: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";

/// The fixed part of a mint account, identical in both programs.
const BASE_LEN: usize = 82;
/// Where Token-2022 records what kind of account this is.
const ACCOUNT_TYPE_OFFSET: usize = 165;
/// Where the extension records begin.
const TLV_OFFSET: usize = 166;
/// `AccountType::Mint`.
const ACCOUNT_TYPE_MINT: u8 = 1;

/// A Token-2022 extension.
///
/// Named where the identifier is known and carried as [`Unknown`](Self::Unknown)
/// otherwise — the same discipline as the instruction decoder. An extension
/// nobody recognises is a reason to look, not a reason to assume it is harmless.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Extension {
    /// A fee is deducted from every transfer.
    TransferFeeConfig,
    /// The mint can be closed.
    MintCloseAuthority,
    /// Balances are hidden, and so is anything computed from them.
    ConfidentialTransferMint,
    /// New accounts start in a given state — including frozen.
    DefaultAccountState,
    /// Tokens cannot be transferred at all.
    NonTransferable,
    /// Balance grows over time.
    InterestBearingConfig,
    /// One address may move or burn anyone's tokens, without their signature.
    PermanentDelegate,
    /// Every transfer calls a program that may refuse it.
    TransferHook,
    /// Where the metadata lives.
    MetadataPointer,
    /// Metadata stored on the mint.
    TokenMetadata,
    /// Group pointer.
    GroupPointer,
    /// Group membership pointer.
    GroupMemberPointer,
    /// Amounts are scaled for display.
    ScaledUiAmount,
    /// Transfers can be paused.
    Pausable,
    /// Present but not recognised.
    Unknown {
        /// The type identifier.
        id: u16,
    },
}

impl Extension {
    /// Reads an extension from its identifier.
    #[must_use]
    pub const fn from_id(id: u16) -> Self {
        match id {
            1 => Self::TransferFeeConfig,
            3 => Self::MintCloseAuthority,
            4 => Self::ConfidentialTransferMint,
            6 => Self::DefaultAccountState,
            9 => Self::NonTransferable,
            10 => Self::InterestBearingConfig,
            12 => Self::PermanentDelegate,
            14 => Self::TransferHook,
            18 => Self::MetadataPointer,
            19 => Self::TokenMetadata,
            20 => Self::GroupPointer,
            22 => Self::GroupMemberPointer,
            25 => Self::ScaledUiAmount,
            26 => Self::Pausable,
            other => Self::Unknown { id: other },
        }
    }

    /// Whether this extension can stop or tax a holder getting out.
    ///
    /// `Unknown` counts. An extension nobody has looked at is not evidence of
    /// safety, and the whole point of exit analysis is to be wrong in the
    /// cautious direction.
    #[must_use]
    pub const fn threatens_exit(self) -> bool {
        matches!(
            self,
            Self::TransferFeeConfig
                | Self::ConfidentialTransferMint
                | Self::DefaultAccountState
                | Self::NonTransferable
                | Self::PermanentDelegate
                | Self::TransferHook
                | Self::Pausable
                | Self::Unknown { .. }
        )
    }
}

/// Why a mint account could not be read.
#[derive(Clone, Copy, PartialEq, Eq, Debug, thiserror::Error)]
pub enum MintError {
    /// Shorter than a mint account can be.
    #[error("mint account is {len} bytes, shorter than the {BASE_LEN}-byte minimum")]
    TooShort {
        /// Bytes present.
        len: usize,
    },
    /// The account exists but is not initialised.
    #[error("mint account is not initialised")]
    Uninitialised,
    /// The extension records do not parse.
    #[error("extension record at offset {at} claims {claimed} bytes, past the end")]
    BadExtension {
        /// Where the record starts.
        at: usize,
        /// The length it claimed.
        claimed: usize,
    },
}

/// What a mint account says about itself.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct MintStructure {
    /// Decimals.
    pub decimals: u8,
    /// Total supply in base units.
    pub supply: u64,
    /// Who may mint more, if anyone.
    ///
    /// `None` means revoked, which is one-way — see [`radar_types::Latch`].
    pub mint_authority: Option<Address>,
    /// Who may freeze holder accounts, if anyone.
    ///
    /// The single most direct way to stop an exit: a live freeze authority can
    /// stop any individual holder selling, at any moment, with one transaction.
    pub freeze_authority: Option<Address>,
    /// Whether this is a Token-2022 mint.
    pub token_2022: bool,
    /// Extensions present.
    pub extensions: Vec<Extension>,
}

impl MintStructure {
    /// Extensions that could stop or tax an exit.
    #[must_use]
    pub fn exit_threats(&self) -> Vec<Extension> {
        self.extensions
            .iter()
            .copied()
            .filter(|e| e.threatens_exit())
            .collect()
    }

    /// Whether anything structural could stop a holder selling.
    ///
    /// A live freeze authority alone is enough. It does not have to have been
    /// used — the question is whether someone *can*, and a token whose issuer
    /// can freeze you at will is one you hold at their discretion.
    #[must_use]
    pub fn can_be_stopped(&self) -> bool {
        self.freeze_authority.is_some() || !self.exit_threats().is_empty()
    }

    /// Reads a mint account.
    ///
    /// # Errors
    ///
    /// Returns [`MintError`] if the account is too short, uninitialised, or its
    /// extension records do not parse.
    pub fn parse(data: &[u8], owner_program: &str) -> Result<Self, MintError> {
        if data.len() < BASE_LEN {
            return Err(MintError::TooShort { len: data.len() });
        }
        if data[45] != 1 {
            return Err(MintError::Uninitialised);
        }

        let option_address = |tag_at: usize| -> Option<Address> {
            let present = u32::from_le_bytes([
                data[tag_at],
                data[tag_at + 1],
                data[tag_at + 2],
                data[tag_at + 3],
            ]);
            (present == 1).then(|| {
                let mut bytes = [0u8; 32];
                bytes.copy_from_slice(&data[tag_at + 4..tag_at + 36]);
                Address::new(bytes)
            })
        };

        let token_2022 = owner_program == TOKEN_2022_PROGRAM;
        Ok(Self {
            decimals: data[44],
            supply: le_u64_at(data, 36),
            mint_authority: option_address(0),
            freeze_authority: option_address(46),
            token_2022,
            extensions: parse_extensions(data)?,
        })
    }
}

/// Reads eight little-endian bytes at a fixed offset.
///
/// Returns zero rather than panicking on a short buffer. The caller has already
/// checked [`BASE_LEN`], so a short read here is unreachable — but a mint parser
/// is fed bytes an attacker chose, and an unreachable panic is still a panic.
fn le_u64_at(data: &[u8], at: usize) -> u64 {
    data.get(at..at + 8)
        .and_then(|s| <[u8; 8]>::try_from(s).ok())
        .map_or(0, u64::from_le_bytes)
}

/// Reads the Token-2022 extension records.
///
/// Classic mints are exactly 82 bytes and have none. A Token-2022 mint pads to
/// 165, marks the account type, and lists type-length-value records from 166.
fn parse_extensions(data: &[u8]) -> Result<Vec<Extension>, MintError> {
    if data.len() <= ACCOUNT_TYPE_OFFSET || data[ACCOUNT_TYPE_OFFSET] != ACCOUNT_TYPE_MINT {
        return Ok(Vec::new());
    }

    let mut out = Vec::new();
    let mut at = TLV_OFFSET;
    while at + 4 <= data.len() {
        let kind = u16::from_le_bytes([data[at], data[at + 1]]);
        let len = usize::from(u16::from_le_bytes([data[at + 2], data[at + 3]]));
        // A zero type with zero length is the end padding, not a record.
        if kind == 0 && len == 0 {
            break;
        }
        let end = at.checked_add(4).and_then(|s| s.checked_add(len));
        match end {
            Some(end) if end <= data.len() => out.push(Extension::from_id(kind)),
            // A record claiming more than the account holds is corrupt or
            // hostile. Stopping with an error beats returning a partial list
            // that a caller would read as "these are all the extensions".
            _ => return Err(MintError::BadExtension { at, claimed: len }),
        }
        at += 4 + len;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real pump.fun mint `5NfV2sy8DqXamLvYEE4LcTWzGqZc5Emv4bqqhVDWpump`,
    /// truncated to the parts this parses.
    fn pumpfun_mint() -> Vec<u8> {
        let mut data = vec![0u8; 389];
        data[44] = 6; // decimals
        data[45] = 1; // initialised
        data[36..44].copy_from_slice(&998_581_408_301_213u64.to_le_bytes());
        data[ACCOUNT_TYPE_OFFSET] = ACCOUNT_TYPE_MINT;
        // MetadataPointer(64) then TokenMetadata(151), as observed on chain.
        data[166..168].copy_from_slice(&18u16.to_le_bytes());
        data[168..170].copy_from_slice(&64u16.to_le_bytes());
        data[234..236].copy_from_slice(&19u16.to_le_bytes());
        data[236..238].copy_from_slice(&151u16.to_le_bytes());
        data
    }

    fn classic_mint(mint_auth: bool, freeze_auth: bool) -> Vec<u8> {
        let mut data = vec![0u8; BASE_LEN];
        data[44] = 6;
        data[45] = 1;
        if mint_auth {
            data[0..4].copy_from_slice(&1u32.to_le_bytes());
            data[4..36].copy_from_slice(&[7u8; 32]);
        }
        if freeze_auth {
            data[46..50].copy_from_slice(&1u32.to_le_bytes());
            data[50..82].copy_from_slice(&[8u8; 32]);
        }
        data
    }

    #[test]
    fn a_real_pumpfun_mint_parses_as_token_2022_with_both_authorities_revoked() {
        let m = MintStructure::parse(&pumpfun_mint(), TOKEN_2022_PROGRAM).expect("parses");
        assert_eq!(m.decimals, 6);
        assert_eq!(m.supply, 998_581_408_301_213);
        assert_eq!(m.mint_authority, None);
        assert_eq!(m.freeze_authority, None);
        assert!(m.token_2022);
        assert_eq!(
            m.extensions,
            vec![Extension::MetadataPointer, Extension::TokenMetadata]
        );
        // Metadata extensions cannot stop a sale.
        assert!(m.exit_threats().is_empty());
        assert!(!m.can_be_stopped());
    }

    #[test]
    fn a_live_freeze_authority_alone_means_a_holder_can_be_stopped() {
        // It does not have to have been used. The question is whether someone
        // can, and a token whose issuer can freeze you is one you hold at their
        // discretion.
        let m = MintStructure::parse(&classic_mint(false, true), TOKEN_PROGRAM).expect("parses");
        assert!(m.freeze_authority.is_some());
        assert!(m.can_be_stopped());
        assert!(
            m.exit_threats().is_empty(),
            "the threat is the authority, not an extension"
        );
    }

    #[test]
    fn a_classic_mint_with_both_revoked_has_nothing_structural_against_it() {
        let m = MintStructure::parse(&classic_mint(false, false), TOKEN_PROGRAM).expect("parses");
        assert!(!m.token_2022);
        assert!(m.extensions.is_empty());
        assert!(!m.can_be_stopped());
    }

    #[test]
    fn the_dangerous_extensions_are_all_flagged() {
        for ext in [
            Extension::TransferHook,
            Extension::PermanentDelegate,
            Extension::NonTransferable,
            Extension::DefaultAccountState,
            Extension::TransferFeeConfig,
            Extension::Pausable,
            Extension::ConfidentialTransferMint,
        ] {
            assert!(
                ext.threatens_exit(),
                "{ext:?} must be treated as an exit threat"
            );
        }
    }

    #[test]
    fn an_unrecognised_extension_counts_as_a_threat() {
        // An extension nobody has looked at is not evidence of safety, and exit
        // analysis should be wrong in the cautious direction.
        let unknown = Extension::from_id(9_999);
        assert_eq!(unknown, Extension::Unknown { id: 9_999 });
        assert!(unknown.threatens_exit());
    }

    #[test]
    fn metadata_extensions_are_not_treated_as_threats() {
        // Every pump.fun token carries these. Flagging them would make the
        // signal fire on the entire population and mean nothing.
        assert!(!Extension::MetadataPointer.threatens_exit());
        assert!(!Extension::TokenMetadata.threatens_exit());
    }

    #[test]
    fn a_truncated_account_errors_rather_than_reading_past_the_end() {
        assert!(matches!(
            MintStructure::parse(&[0u8; 40], TOKEN_PROGRAM),
            Err(MintError::TooShort { len: 40 })
        ));
    }

    #[test]
    fn an_uninitialised_mint_is_refused() {
        let mut data = classic_mint(false, false);
        data[45] = 0;
        assert_eq!(
            MintStructure::parse(&data, TOKEN_PROGRAM),
            Err(MintError::Uninitialised)
        );
    }

    #[test]
    fn a_lying_extension_length_errors_rather_than_returning_a_partial_list() {
        // A partial list reads as "these are all the extensions", which for exit
        // analysis is the difference between "nothing can stop you" and "we
        // stopped looking".
        let mut data = vec![0u8; 200];
        data[44] = 6;
        data[45] = 1;
        data[ACCOUNT_TYPE_OFFSET] = ACCOUNT_TYPE_MINT;
        data[166..168].copy_from_slice(&14u16.to_le_bytes());
        data[168..170].copy_from_slice(&u16::MAX.to_le_bytes());
        assert!(matches!(
            MintStructure::parse(&data, TOKEN_2022_PROGRAM),
            Err(MintError::BadExtension { .. })
        ));
    }
}
