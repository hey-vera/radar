// SPDX-License-Identifier: Apache-2.0
//! Loading the signing key.
//!
//! Kept in its own module so the surface that touches secret material is small
//! and readable in one sitting. Three rules hold here:
//!
//! - The key is read from a file path given in the environment, never from an
//!   environment variable itself. Environment variables leak into process
//!   listings, crash dumps, child processes and logs; a file has an owner and a
//!   mode. The x402 vendor's install script wants `SOLANA_PRIVATE_KEY` in the
//!   environment, which is exactly the shape being avoided.
//! - No `Debug`, `Display`, `Serialize` or `Clone` on anything holding secret
//!   bytes. The way a key ends up in a log is that something derived `Debug` on
//!   a struct three levels up.
//! - Missing configuration means refusing to sign, not signing without checks.

use std::path::Path;

use ed25519_dalek::{Signer as _, SigningKey};
use radar_types::{Address, Signature};

/// Why a key could not be loaded.
#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    /// The file could not be read.
    #[error("cannot read key file: {0}")]
    Unreadable(String),
    /// The file did not hold a Solana keypair.
    #[error("key file is not a 64-byte Solana keypair array")]
    Malformed,
    /// The public half does not match the secret half.
    ///
    /// A Solana keypair file stores both. If they disagree, the file has been
    /// edited or corrupted, and signing with it would produce signatures
    /// attributed to a wallet whose key we do not hold.
    #[error("key file's public half does not match its secret half")]
    Mismatched,
}

/// A loaded signing key.
///
/// Deliberately has no `Debug`, no `Clone` and no serde. The only thing it can
/// do is sign, and the only thing it will reveal is its public half.
pub struct Key {
    inner: SigningKey,
    public: Address,
}

impl Key {
    /// Loads a Solana keypair file: a JSON array of 64 bytes, secret then public.
    ///
    /// # Errors
    ///
    /// Returns [`KeyError`] if the file cannot be read, is not 64 bytes, or its
    /// two halves disagree.
    pub fn load(path: &Path) -> Result<Self, KeyError> {
        let text =
            std::fs::read_to_string(path).map_err(|e| KeyError::Unreadable(e.to_string()))?;
        let bytes: Vec<u8> = serde_json::from_str(&text).map_err(|_| KeyError::Malformed)?;
        Self::from_keypair_bytes(&bytes)
    }

    /// Builds a key from the 64 bytes of a Solana keypair file.
    ///
    /// # Errors
    ///
    /// Returns [`KeyError::Malformed`] on the wrong length, or
    /// [`KeyError::Mismatched`] if the stored public half is not the one the
    /// secret half derives.
    pub fn from_keypair_bytes(bytes: &[u8]) -> Result<Self, KeyError> {
        let secret: [u8; 32] = bytes
            .get(0..32)
            .and_then(|s| s.try_into().ok())
            .ok_or(KeyError::Malformed)?;
        let stated: [u8; 32] = bytes
            .get(32..64)
            .and_then(|s| s.try_into().ok())
            .ok_or(KeyError::Malformed)?;
        if bytes.len() != 64 {
            return Err(KeyError::Malformed);
        }

        let inner = SigningKey::from_bytes(&secret);
        let derived = inner.verifying_key().to_bytes();
        if derived != stated {
            return Err(KeyError::Mismatched);
        }

        Ok(Self {
            inner,
            public: Address::new(derived),
        })
    }

    /// The wallet this key signs for.
    #[must_use]
    pub const fn public(&self) -> Address {
        self.public
    }

    /// Signs verified message bytes.
    ///
    /// Takes a [`crate::Checked`] rather than a byte slice, so the type system
    /// records that verification happened. There is no method here that signs
    /// arbitrary bytes, because the moment one exists it is the one that gets
    /// called from the path that skipped a check.
    #[must_use]
    pub fn sign(&self, checked: &crate::Checked) -> Signature {
        Signature::new(self.inner.sign(checked.signable()).to_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic keypair file's contents.
    fn keypair_bytes(seed: u8) -> Vec<u8> {
        let signing = SigningKey::from_bytes(&[seed; 32]);
        let mut out = vec![seed; 32];
        out.extend_from_slice(&signing.verifying_key().to_bytes());
        out
    }

    #[test]
    fn a_wellformed_keypair_loads_and_reports_its_wallet() {
        let key = Key::from_keypair_bytes(&keypair_bytes(7)).expect("loads");
        let expected = SigningKey::from_bytes(&[7u8; 32])
            .verifying_key()
            .to_bytes();
        assert_eq!(key.public(), Address::new(expected));
    }

    #[test]
    fn a_keypair_whose_halves_disagree_is_refused() {
        // Signing with it would produce signatures attributed to a wallet whose
        // key we do not hold — which fails at submission, after the decision.
        let mut bytes = keypair_bytes(7);
        bytes[40] ^= 0xFF;
        assert!(matches!(
            Key::from_keypair_bytes(&bytes),
            Err(KeyError::Mismatched)
        ));
    }

    #[test]
    fn a_wrong_length_file_is_refused() {
        assert!(matches!(
            Key::from_keypair_bytes(&[0u8; 32]),
            Err(KeyError::Malformed)
        ));
        assert!(matches!(
            Key::from_keypair_bytes(&[0u8; 65]),
            Err(KeyError::Malformed)
        ));
    }

    #[test]
    fn signing_is_deterministic_and_verifiable() {
        // ed25519 signatures are deterministic, so a replay of a recorded
        // decision produces a byte-identical transaction. That is what lets the
        // research store compare a replay against a recording at all.
        use ed25519_dalek::{Verifier as _, VerifyingKey};

        let key = Key::from_keypair_bytes(&keypair_bytes(9)).expect("loads");
        let checked = crate::verify::tests_support::checked_fixture();

        let first = key.sign(&checked);
        assert_eq!(first.as_bytes(), key.sign(&checked).as_bytes());

        let verifying = VerifyingKey::from_bytes(key.public().as_bytes()).expect("valid key");
        verifying
            .verify(
                checked.signable(),
                &ed25519_dalek::Signature::from_bytes(first.as_bytes()),
            )
            .expect("the signature must verify against the wallet it claims");
    }
}
