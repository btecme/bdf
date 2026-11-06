//! The signed release description.
//!
//! A release publishes one small file next to the package, `bdf-update.json`, an envelope:
//!
//! ```json
//! { "payload": "<base64 of the manifest JSON below>", "signature": "<128 hex: ECDSA P-256 r||s over SHA-256(payload bytes)>" }
//! ```
//!
//! One file, so a feed that moves to a newer release between two requests can never pair a manifest with
//! another release's signature. [`SignedManifest::from_envelope`] is the only way to get a [`Manifest`] that
//! has been checked; everything downstream (the package's size and SHA-256, the download address, the version)
//! comes from those verified bytes. The manifest is:
//!
//! ```json
//! { "schema": 1, "product": "bdf", "channel": "stable", "version": "0.8.0",
//!   "published": "2026-11-06T12:00:00Z", "notes": "What changed.",
//!   "notes_url": "https://github.com/btecme/bdf/releases/tag/v0.8.0",
//!   "packages": [ { "os": "windows", "arch": "x64", "kind": "portable-zip",
//!                   "url": "https://github.com/btecme/bdf/releases/download/v0.8.0/bdf-0.8.0-windows-x64-portable.zip",
//!                   "size": 61234567, "sha256": "…64 hex…" } ] }
//! ```

use serde::{Deserialize, Serialize};

use crate::{Channel, Result, Trust, UpdateError, Version, unhex};

/// The release's update information (the envelope).
pub const MANIFEST_FILE: &str = "bdf-update.json";

/// Release information is tiny; anything bigger is not it.
pub const MAX_MANIFEST_BYTES: usize = 256 * 1024;
/// The envelope holds the manifest as base64, a third bigger.
pub const MAX_ENVELOPE_BYTES: usize = MAX_MANIFEST_BYTES / 3 * 4 + 1024;
/// The largest package accepted (the real one is well under a third of this).
pub const MAX_PACKAGE_BYTES: u64 = 512 * 1024 * 1024;
/// Where BDF releases are downloaded from. A package address elsewhere is refused, even in a signed manifest.
pub const RELEASE_DOWNLOAD_PREFIX: &str = "https://github.com/btecme/bdf/releases/download/";
/// Where the release notes page may point.
pub const RELEASE_PAGE_PREFIX: &str = "https://github.com/btecme/bdf/releases/";
const MAX_NOTES_CHARS: usize = 4000;

/// A package this release offers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageEntry {
    pub os: String,
    pub arch: String,
    /// `portable-zip` is the only kind.
    pub kind: String,
    pub url: String,
    pub size: u64,
    /// SHA-256 of the whole package file, lowercase hex.
    pub sha256: String,
}

/// The release information, as published.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: u32,
    pub product: String,
    pub channel: String,
    pub version: String,
    /// Informational (RFC 3339), shown nowhere that matters for trust.
    #[serde(default)]
    pub published: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub notes_url: Option<String>,
    /// The oldest version that can update to this one in place; older copies need the full package.
    #[serde(default)]
    pub min_from: Option<String>,
    pub packages: Vec<PackageEntry>,
}

/// A manifest whose signature was checked against the build's trust root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedManifest {
    pub manifest: Manifest,
    pub version: Version,
    /// The exact signed bytes (kept so the helper that applies the update can verify them again).
    pub raw: Vec<u8>,
    /// The signature over `raw`, hex.
    pub signature: String,
}

/// A release that applies to this copy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Available {
    pub version: Version,
    /// Plain text, control characters removed, at most 4,000 characters.
    pub notes: String,
    pub notes_url: Option<String>,
    pub published: String,
    pub package: PackageEntry,
}

/// What a verified manifest means for the running copy.
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)] // one short-lived value per check; boxing it would only add noise
pub enum Assessment {
    UpToDate,
    Available(Available),
}

#[derive(Serialize, Deserialize)]
struct Envelope {
    payload: String,
    signature: String,
}

impl SignedManifest {
    /// Read and verify an envelope (`bdf-update.json`).
    pub fn from_envelope(bytes: &[u8], trust: &Trust) -> Result<SignedManifest> {
        use base64::Engine as _;
        if bytes.len() > MAX_ENVELOPE_BYTES {
            return Err(UpdateError::Malformed("the release information is too large".into()));
        }
        let envelope: Envelope =
            serde_json::from_slice(bytes).map_err(|e| UpdateError::Malformed(format!("the release information is not valid ({e})")))?;
        let payload = base64::engine::general_purpose::STANDARD
            .decode(envelope.payload.trim())
            .map_err(|_| UpdateError::Malformed("the release payload is not base64".into()))?;
        SignedManifest::verify(&payload, &envelope.signature, trust)
    }

    /// The envelope for these verified bytes (what is cached and handed to the helper that applies the update).
    pub fn envelope(&self) -> Vec<u8> {
        make_envelope(&self.raw, &self.signature)
    }

    /// Verify `signature` over `raw` with `trust`, then read and validate the manifest.
    ///
    /// The signature is checked **before** the bytes are parsed, so unauthenticated input never reaches the
    /// JSON parser. The manifest must be for this product and this build's channel.
    pub fn verify(raw: &[u8], signature: &str, trust: &Trust) -> Result<SignedManifest> {
        if raw.len() > MAX_MANIFEST_BYTES {
            return Err(UpdateError::Malformed("the release information is too large".into()));
        }
        trust.verify(raw, signature)?;
        let manifest: Manifest =
            serde_json::from_slice(raw).map_err(|e| UpdateError::Malformed(format!("the release information is not valid ({e})")))?;
        let version = manifest.validate(trust.channel())?;
        Ok(SignedManifest { manifest, version, raw: raw.to_vec(), signature: signature.trim().to_string() })
    }

    /// Decide what this release means for a copy at `current`.
    ///
    /// * `floor`: the highest release version this copy has ever verified. A release below it is a replay of
    ///   an old, genuinely signed manifest and is refused.
    /// * An older or equal version than `current` is never offered.
    pub fn assess(&self, current: Version, floor: Option<Version>, os: &str, arch: &str, channel: Channel) -> Result<Assessment> {
        if let Some(floor) = floor
            && self.version < floor
        {
            return Err(UpdateError::NotApplicable(format!(
                "version {} is older than version {floor}, which was already seen; an old release is not offered again",
                self.version
            )));
        }
        if self.version < current {
            return Err(UpdateError::NotApplicable(format!("version {} is older than the version you have ({current})", self.version)));
        }
        if self.version == current {
            return Ok(Assessment::UpToDate);
        }
        if let Some(min) = self.manifest.min_from.as_deref().and_then(Version::parse)
            && current < min
        {
            return Err(UpdateError::NotApplicable(format!(
                "version {} can only be installed over version {min} or newer; install the full portable package instead",
                self.version
            )));
        }
        let package = self
            .manifest
            .packages
            .iter()
            .find(|p| p.os == os && p.arch == arch && p.kind == "portable-zip")
            .ok_or_else(|| UpdateError::NotApplicable(format!("version {} has no package for {os} {arch}", self.version)))?;
        if !package_url_allowed(&package.url, channel) {
            return Err(UpdateError::Malformed("the package address is not a BDF release address".into()));
        }
        Ok(Assessment::Available(Available {
            version: self.version,
            notes: clean_notes(&self.manifest.notes),
            notes_url: self.manifest.notes_url.clone().filter(|u| page_url_allowed(u)),
            published: self.manifest.published.chars().filter(|c| !c.is_control()).take(40).collect(),
            package: package.clone(),
        }))
    }
}

impl Manifest {
    /// Structural checks that do not depend on the running copy. Returns the parsed version.
    pub fn validate(&self, channel: Channel) -> Result<Version> {
        if self.schema != 1 {
            return Err(UpdateError::Malformed(format!("unknown release information version {}", self.schema)));
        }
        if self.product != "bdf" {
            return Err(UpdateError::NotApplicable("this release information is for a different product".into()));
        }
        if self.channel != channel.name() {
            return Err(UpdateError::NotApplicable(format!(
                "this release is on the {} channel, this build follows {}",
                self.channel,
                channel.name()
            )));
        }
        let version = Version::parse(&self.version)
            .ok_or_else(|| UpdateError::Malformed(format!("`{}` is not a release version", self.version.chars().take(40).collect::<String>())))?;
        if self.packages.is_empty() || self.packages.len() > 16 {
            return Err(UpdateError::Malformed("the release lists no usable packages".into()));
        }
        for p in &self.packages {
            if p.size == 0 || p.size > MAX_PACKAGE_BYTES {
                return Err(UpdateError::Malformed("a package has an impossible size".into()));
            }
            if !is_sha256_hex(&p.sha256) {
                return Err(UpdateError::Malformed("a package hash is not a SHA-256".into()));
            }
        }
        Ok(version)
    }
}

/// The envelope file for a manifest and its signature (used by the release tooling and the tests).
pub fn make_envelope(manifest_json: &[u8], signature_hex: &str) -> Vec<u8> {
    use base64::Engine as _;
    let envelope = Envelope { payload: base64::engine::general_purpose::STANDARD.encode(manifest_json), signature: signature_hex.trim().to_string() };
    // Two strings always serialize.
    serde_json::to_vec_pretty(&envelope).unwrap_or_default()
}

/// 64 lowercase hex characters.
pub fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) && unhex(s).is_some()
}

/// Whether `url` may be downloaded as a package: a BDF release asset over HTTPS, or (dev channel only) a
/// loopback test server. Query strings, fragments, backslashes and `..` are refused.
pub fn package_url_allowed(url: &str, channel: Channel) -> bool {
    let clean = |rest: &str| !rest.is_empty() && !rest.contains(['?', '#', '\\', '@', ' ']) && !rest.split('/').any(|c| c == ".." || c == ".");
    if let Some(rest) = url.strip_prefix(RELEASE_DOWNLOAD_PREFIX) {
        return clean(rest) && rest.contains('/');
    }
    if channel == Channel::Dev {
        for prefix in ["http://127.0.0.1:", "http://localhost:", "http://[::1]:"] {
            if let Some(rest) = url.strip_prefix(prefix) {
                let (port, path) = rest.split_once('/').unwrap_or((rest, ""));
                return !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) && clean(path);
            }
        }
    }
    false
}

fn page_url_allowed(url: &str) -> bool {
    url.strip_prefix(RELEASE_PAGE_PREFIX).is_some_and(|rest| !rest.contains(['?', '#', '\\', ' ']) && !rest.split('/').any(|c| c == ".."))
}

/// Release notes for display: no control characters (newlines and tabs stay), a hard length cap.
pub fn clean_notes(notes: &str) -> String {
    let kept: String =
        notes.chars().filter(|c| !c.is_control() || matches!(c, '\n' | '\t')).filter(|c| !is_bidi_control(*c)).take(MAX_NOTES_CHARS).collect();
    kept.trim().to_string()
}

fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}')
}

/// Signed manifests for tests (and for the helper's tests, with the `testing` feature).
#[cfg(any(test, feature = "testing"))]
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub mod testing {
    use super::*;
    use crate::trust::testkeys;

    pub const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    pub fn manifest_json(version: &str, channel: &str, sha: &str, size: u64) -> String {
        format!(
            r#"{{"schema":1,"product":"bdf","channel":"{channel}","version":"{version}","published":"2026-11-06T00:00:00Z","notes":"Fixes.","notes_url":"https://github.com/btecme/bdf/releases/tag/v{version}","packages":[{{"os":"windows","arch":"x64","kind":"portable-zip","url":"https://github.com/btecme/bdf/releases/download/v{version}/bdf-{version}-windows-x64-portable.zip","size":{size},"sha256":"{sha}"}}]}}"#
        )
    }

    pub fn signed(seed: u8, json: &str, channel: Channel) -> Result<SignedManifest> {
        SignedManifest::verify(json.as_bytes(), &testkeys::sign(seed, json.as_bytes()), &testkeys::trust(seed, channel))
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;
    use crate::trust::testkeys;

    fn assess(m: &SignedManifest, current: Version, floor: Option<Version>) -> Result<Assessment> {
        m.assess(current, floor, "windows", "x64", Channel::Stable)
    }

    #[test]
    fn a_signed_newer_release_is_offered_with_its_package() {
        let m = signed(1, &manifest_json("0.8.1", "stable", SHA, 1000), Channel::Stable).unwrap();
        match assess(&m, Version(0, 8, 0), None).unwrap() {
            Assessment::Available(a) => {
                assert_eq!(a.version, Version(0, 8, 1));
                assert_eq!(a.package.sha256, SHA);
                assert_eq!(a.notes, "Fixes.");
                assert!(a.notes_url.unwrap().ends_with("/v0.8.1"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn same_older_and_replayed_releases_are_not_offered() {
        let m = signed(1, &manifest_json("0.8.1", "stable", SHA, 1000), Channel::Stable).unwrap();
        assert_eq!(assess(&m, Version(0, 8, 1), None).unwrap(), Assessment::UpToDate);
        assert!(matches!(assess(&m, Version(0, 9, 0), None), Err(UpdateError::NotApplicable(_))));
        // Genuinely signed but older than one this copy already saw: a replay.
        assert!(matches!(assess(&m, Version(0, 8, 0), Some(Version(0, 8, 2))), Err(UpdateError::NotApplicable(_))));
        // The floor equal to the release is fine.
        assert!(matches!(assess(&m, Version(0, 8, 0), Some(Version(0, 8, 1))).unwrap(), Assessment::Available(_)));
    }

    #[test]
    fn a_tampered_manifest_or_signature_never_reaches_the_parser() {
        let json = manifest_json("0.8.1", "stable", SHA, 1000);
        let sig = testkeys::sign(1, json.as_bytes());
        let trust = testkeys::trust(1, Channel::Stable);
        let changed = json.replace("1000", "1001");
        assert!(matches!(SignedManifest::verify(changed.as_bytes(), &sig, &trust), Err(UpdateError::Tampered(_))));
        let other_key = testkeys::sign(2, json.as_bytes());
        assert!(matches!(SignedManifest::verify(json.as_bytes(), &other_key, &trust), Err(UpdateError::Tampered(_))));
        // Garbage that is not JSON but is "signed" is refused as unreadable, not accepted.
        let junk = "not json";
        assert!(matches!(SignedManifest::verify(junk.as_bytes(), &testkeys::sign(1, junk.as_bytes()), &trust), Err(UpdateError::Malformed(_))));
    }

    #[test]
    fn no_trust_root_means_no_manifest() {
        let json = manifest_json("0.8.1", "stable", SHA, 1000);
        let r = SignedManifest::verify(json.as_bytes(), &testkeys::sign(1, json.as_bytes()), &Trust::none(Channel::Stable));
        assert!(matches!(r, Err(UpdateError::NoTrustRoot)));
    }

    #[test]
    fn the_channel_and_product_must_match() {
        let dev = manifest_json("0.8.1", "dev", SHA, 1000);
        assert!(matches!(signed(1, &dev, Channel::Stable), Err(UpdateError::NotApplicable(_))));
        assert!(signed(1, &dev, Channel::Dev).is_ok());
        let other = manifest_json("0.8.1", "stable", SHA, 1000).replace(r#""product":"bdf""#, r#""product":"other""#);
        assert!(matches!(signed(1, &other, Channel::Stable), Err(UpdateError::NotApplicable(_))));
    }

    #[test]
    fn broken_fields_are_refused() {
        for (from, to) in [
            (r#""schema":1"#, r#""schema":2"#),
            (r#""version":"0.8.1""#, r#""version":"0.8.1-rc1""#),
            (r#""size":1000"#, r#""size":0"#),
            (r#""size":1000"#, r#""size":99999999999"#),
            (SHA, "ABCDEF"),
            (r#""packages":[{"#, r#""packages":[],"x":[{"#),
        ] {
            let json = manifest_json("0.8.1", "stable", SHA, 1000).replacen(from, to, 1);
            assert!(signed(1, &json, Channel::Stable).is_err(), "{to}");
        }
        let upper = manifest_json("0.8.1", "stable", &SHA.to_uppercase(), 1000);
        assert!(signed(1, &upper, Channel::Stable).is_err(), "uppercase hash");
    }

    #[test]
    fn a_missing_platform_package_is_not_applicable() {
        let m = signed(1, &manifest_json("0.8.1", "stable", SHA, 1000), Channel::Stable).unwrap();
        let r = m.assess(Version(0, 8, 0), None, "windows", "arm64", Channel::Stable);
        assert!(matches!(r, Err(UpdateError::NotApplicable(_))));
        let r = m.assess(Version(0, 8, 0), None, "linux", "x64", Channel::Stable);
        assert!(matches!(r, Err(UpdateError::NotApplicable(_))));
    }

    #[test]
    fn only_bdf_release_addresses_are_downloaded() {
        let ok = "https://github.com/btecme/bdf/releases/download/v0.8.1/bdf-0.8.1-windows-x64-portable.zip";
        assert!(package_url_allowed(ok, Channel::Stable));
        for bad in [
            "https://github.com/other/bdf/releases/download/v1/x.zip",
            "https://github.com/btecme/bdf/releases/download/",
            "https://github.com/btecme/bdf/releases/download/v1/../../x.zip",
            "https://github.com/btecme/bdf/releases/download/v1/x.zip?token=1",
            "https://github.com/btecme/bdf/releases/download/v1/x.zip#a",
            "https://github.com/btecme/bdf/releases/download/v1\\x.zip",
            "https://github.com.evil.example/btecme/bdf/releases/download/v1/x.zip",
            "http://github.com/btecme/bdf/releases/download/v1/x.zip",
            "https://example.com/bdf.zip",
            "file:///C:/x.zip",
            "http://127.0.0.1:8080/x.zip",
        ] {
            assert!(!package_url_allowed(bad, Channel::Stable), "{bad}");
        }
        // Only a dev build may fetch from a loopback test server.
        assert!(package_url_allowed("http://127.0.0.1:8080/x.zip", Channel::Dev));
        assert!(package_url_allowed("http://localhost:1/a/b.zip", Channel::Dev));
        for bad in [
            "http://127.0.0.1/x.zip",
            "http://127.0.0.1:80@evil.example/x.zip",
            "http://127.0.0.1:8080/../x.zip",
            "http://127.0.0.1.evil.example:8080/x.zip",
            "ftp://127.0.0.1:8080/x",
        ] {
            assert!(!package_url_allowed(bad, Channel::Dev), "{bad}");
        }
    }

    #[test]
    fn a_signed_manifest_cannot_point_elsewhere() {
        let json = manifest_json("0.8.1", "stable", SHA, 1000).replace("https://github.com/btecme/bdf/releases/download/", "https://example.com/");
        let m = signed(1, &json, Channel::Stable).unwrap();
        assert!(matches!(assess(&m, Version(0, 8, 0), None), Err(UpdateError::Malformed(_))));
    }

    #[test]
    fn notes_are_displayable_text_only() {
        assert_eq!(clean_notes("  a\u{0}b\u{1b}[31mc\nd\te\u{202e}f  "), "ab[31mc\nd\tef");
        assert_eq!(clean_notes(&"x".repeat(10_000)).chars().count(), 4000);
        let m = signed(
            1,
            &manifest_json("0.8.1", "stable", SHA, 1000).replace("https://github.com/btecme/bdf/releases/tag/", "https://evil.example/"),
            Channel::Stable,
        )
        .unwrap();
        match assess(&m, Version(0, 8, 0), None).unwrap() {
            Assessment::Available(a) => assert_eq!(a.notes_url, None, "a notes link outside the BDF releases is dropped"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_envelope_round_trips_and_any_tampering_is_caught() {
        let json = manifest_json("0.8.1", "stable", SHA, 1000);
        let trust = testkeys::trust(1, Channel::Stable);
        let env = make_envelope(json.as_bytes(), &testkeys::sign(1, json.as_bytes()));
        let m = SignedManifest::from_envelope(&env, &trust).unwrap();
        assert_eq!(m.version, Version(0, 8, 1));
        assert_eq!(SignedManifest::from_envelope(&m.envelope(), &trust).unwrap(), m);
        // A different payload under the same signature.
        let other = manifest_json("0.9.0", "stable", SHA, 1000);
        let swapped = make_envelope(other.as_bytes(), &testkeys::sign(1, json.as_bytes()));
        assert!(matches!(SignedManifest::from_envelope(&swapped, &trust), Err(UpdateError::Tampered(_))));
        for bad in ["", "{}", "[]", "null", r#"{"payload":"!!!","signature":"00"}"#, r#"{"payload":5,"signature":"00"}"#] {
            assert!(SignedManifest::from_envelope(bad.as_bytes(), &trust).is_err(), "{bad:?}");
        }
        assert!(matches!(SignedManifest::from_envelope(&vec![b' '; MAX_ENVELOPE_BYTES + 1], &trust), Err(UpdateError::Malformed(_))));
    }

    #[test]
    fn oversized_release_information_is_refused_unparsed() {
        let big = " ".repeat(MAX_MANIFEST_BYTES + 1);
        let trust = testkeys::trust(1, Channel::Stable);
        assert!(matches!(SignedManifest::verify(big.as_bytes(), &testkeys::sign(1, big.as_bytes()), &trust), Err(UpdateError::Malformed(_))));
    }
}
