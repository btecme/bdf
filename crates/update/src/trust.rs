//! What this build trusts: one release signing key, fixed at build time.
//!
//! The key is an ECDSA P-256 public key (an uncompressed SEC1 point, 130 hex characters) read from
//! `BDF_UPDATE_PUBKEY` by `build.rs`. Signatures are the fixed-size `r || s` form over the SHA-256 of
//! the exact manifest bytes, which any platform's standard library can check, including the
//! PowerShell deployment script (`packaging/windows/deploy-bdf.ps1`).
//!
//! * **No key, no updates.** [`Trust::build`] returns an unconfigured trust and every verification
//!   fails with [`UpdateError::NoTrustRoot`]. Nothing falls back to "trust the download".
//! * A **`dev`** build is made with a test key and `BDF_UPDATE_CHANNEL=dev`. It checks exactly as hard as
//!   a release build; only the key differs. The interface labels it, and only it may read its feed from
//!   `BDF_UPDATE_FEED`. A release build ignores that variable.

use p256::ecdsa::{Signature, VerifyingKey, signature::hazmat::PrehashVerifier};
use sha2::{Digest, Sha256};

use crate::{Result, UpdateError, unhex};

/// Which line of releases a build follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    /// Real releases, signed by the owner's release key.
    Stable,
    /// Test releases signed by a throwaway key; clearly labelled.
    Dev,
}

impl Channel {
    pub fn name(self) -> &'static str {
        match self {
            Channel::Stable => "stable",
            Channel::Dev => "dev",
        }
    }
    pub fn parse(name: &str) -> Option<Channel> {
        match name {
            "stable" => Some(Channel::Stable),
            "dev" => Some(Channel::Dev),
            _ => None,
        }
    }
}

/// The public key a build accepts releases from, and the channel it follows.
#[derive(Clone, Debug)]
pub struct Trust {
    key: Option<VerifyingKey>,
    channel: Channel,
    /// First 16 hex characters of the SHA-256 of the key bytes: enough to tell keys apart in a status line.
    key_id: Option<String>,
}

impl Trust {
    /// The trust compiled into this build (`BDF_UPDATE_PUBKEY`, `BDF_UPDATE_CHANNEL`).
    pub fn build() -> Trust {
        let channel = Channel::parse(env!("BDF_TRUST_CHANNEL")).unwrap_or(Channel::Stable);
        match Trust::from_hex(env!("BDF_TRUST_KEY"), channel) {
            Ok(t) => t,
            Err(e) => {
                if !env!("BDF_TRUST_KEY").is_empty() {
                    log::warn!("the update signing key compiled into this build is unusable ({e}); updates are off");
                }
                Trust { key: None, channel, key_id: None }
            }
        }
    }

    /// Trust the key given as hex of an uncompressed SEC1 point.
    pub fn from_hex(hex: &str, channel: Channel) -> Result<Trust> {
        let bytes = unhex(hex.trim()).ok_or_else(|| UpdateError::Malformed("the signing key is not hex".into()))?;
        if bytes.len() != 65 || bytes.first() != Some(&4) {
            return Err(UpdateError::Malformed("the signing key must be an uncompressed P-256 point (65 bytes)".into()));
        }
        let key = VerifyingKey::from_sec1_bytes(&bytes).map_err(|_| UpdateError::Malformed("the signing key is not a valid P-256 point".into()))?;
        let id = crate::hex(&Sha256::digest(&bytes));
        Ok(Trust { key: Some(key), channel, key_id: id.get(..16).map(str::to_string) })
    }

    /// A trust with no key (what a build without `BDF_UPDATE_PUBKEY` has).
    pub fn none(channel: Channel) -> Trust {
        Trust { key: None, channel, key_id: None }
    }

    pub fn channel(&self) -> Channel {
        self.channel
    }

    /// Whether this build can verify anything at all.
    pub fn is_configured(&self) -> bool {
        self.key.is_some()
    }

    pub fn key_id(&self) -> Option<&str> {
        self.key_id.as_deref()
    }

    /// Check `signature` (128 hex characters, `r || s`) over the SHA-256 of `message`.
    pub fn verify(&self, message: &[u8], signature_hex: &str) -> Result<()> {
        let key = self.key.as_ref().ok_or(UpdateError::NoTrustRoot)?;
        let raw = unhex(signature_hex.trim())
            .filter(|r| r.len() == 64)
            .ok_or_else(|| UpdateError::Tampered("the signature is not 64 bytes of hex".into()))?;
        let signature = Signature::from_slice(&raw).map_err(|_| UpdateError::Tampered("the signature is malformed".into()))?;
        key.verify_prehash(&Sha256::digest(message), &signature)
            .map_err(|_| UpdateError::Tampered("the signature does not match the release information".into()))
    }
}

/// A fixed throwaway key pair for tests (never used by a build).
#[cfg(any(test, feature = "testing"))]
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub mod testkeys {
    use p256::ecdsa::{SigningKey, signature::hazmat::PrehashSigner};
    use sha2::{Digest, Sha256};

    use super::*;

    pub fn signing_key(seed: u8) -> SigningKey {
        let mut bytes = [seed; 32];
        bytes[0] = 1;
        SigningKey::from_slice(&bytes).expect("a fixed non-zero scalar below the group order")
    }

    pub fn trust(seed: u8, channel: Channel) -> Trust {
        let point = signing_key(seed).verifying_key().to_sec1_point(false);
        Trust::from_hex(&crate::hex(point.as_bytes()), channel).expect("a valid key")
    }

    pub fn sign(seed: u8, message: &[u8]) -> String {
        let signature: Signature = signing_key(seed).sign_prehash(&Sha256::digest(message)).expect("signing a digest");
        crate::hex(&signature.to_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::testkeys::*;
    use super::*;

    #[test]
    fn a_good_signature_verifies_and_any_change_does_not() {
        let trust = trust(7, Channel::Dev);
        let sig = sign(7, b"manifest");
        trust.verify(b"manifest", &sig).unwrap();
        assert!(matches!(trust.verify(b"manifest!", &sig), Err(UpdateError::Tampered(_))));
        let mut flipped = sig.clone().into_bytes();
        flipped[10] = if flipped[10] == b'0' { b'1' } else { b'0' };
        assert!(matches!(trust.verify(b"manifest", &String::from_utf8(flipped).unwrap()), Err(UpdateError::Tampered(_))));
        // A signature by another key is not accepted.
        assert!(matches!(trust.verify(b"manifest", &sign(8, b"manifest")), Err(UpdateError::Tampered(_))));
    }

    #[test]
    fn malformed_signatures_are_refused_without_panicking() {
        let trust = trust(7, Channel::Dev);
        for bad in ["", "zz", "00", &"0".repeat(128), &"f".repeat(128), &"0".repeat(130), "\u{e9}"] {
            assert!(trust.verify(b"x", bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_build_without_a_key_refuses_everything() {
        let none = Trust::none(Channel::Stable);
        assert!(!none.is_configured());
        assert!(matches!(none.verify(b"x", &sign(7, b"x")), Err(UpdateError::NoTrustRoot)));
    }

    #[test]
    fn unusable_keys_are_refused() {
        for bad in ["", "04", "zz", &"04".repeat(65), &format!("02{}", "11".repeat(32)), &format!("04{}", "00".repeat(64))] {
            assert!(Trust::from_hex(bad, Channel::Stable).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn this_build_has_a_defined_trust() {
        // Whatever the environment set, building the trust never panics; without a key it is unconfigured.
        let t = Trust::build();
        assert_eq!(t.is_configured(), !env!("BDF_TRUST_KEY").is_empty());
    }
}
