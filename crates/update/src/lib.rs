//! BDF's updater: what it trusts, how a release is verified, and how a verified release replaces the
//! running portable copy without ever leaving it half-updated (BUILD-018).
//!
//! The pieces, in the order an update meets them:
//!
//! * [`trust`]: the one public key this build accepts releases from, set at build time. A build without
//!   one refuses every update operation (**fail closed**); a `dev` build trusts a test key and says so.
//! * [`manifest`]: the signed release description (`bdf-update.json` plus a detached signature). The
//!   signature covers the exact bytes; the package's size and SHA-256 come from inside those bytes, so a
//!   checksum is never taken from an unauthenticated place.
//! * [`package`]: the portable ZIP. It is verified completely (every entry's path, size and hash against the
//!   package's own file list, which the signed hash covers) before a single file is written to the copy.
//! * [`layout`]: what is a managed copy (it carries `bdf-package.json`) and where update files live.
//! * [`apply`]: the journaled replacement. Files are renamed, never overwritten in place; the previous
//!   version is kept; an interrupted run is rolled back by [`apply::recover`].
//! * [`state`]: the cache and backoff of automatic checks, kept in the data folder.
//! * [`service`]: status, check, prepare, apply and rollback, the one object the app, CLI and tools share.
//! * [`feed`] (feature `net`): the HTTPS transport.
//!
//! Nothing here touches the user's data folder (`BDFData`): the replacement works from file lists and never
//! deletes a directory it does not own.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod apply;
pub mod layout;
pub mod manifest;
pub mod package;
pub mod service;
pub mod state;
pub mod trust;
pub mod version;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

#[cfg(feature = "net")]
pub mod feed;

pub use manifest::{Manifest, SignedManifest};
pub use trust::{Channel, Trust};
pub use version::Version;

/// What can go wrong. The messages are written for the person who sees them in the app, the CLI or an
/// agent: they say what was wrong and, where there is one, what to do.
#[derive(Debug)]
pub enum UpdateError {
    /// This build carries no release signing key, so it cannot tell a real release from a forgery.
    NoTrustRoot,
    /// The signature, the hash or the size of something did not match: it was changed or damaged.
    Tampered(String),
    /// A release that is genuine but not for this copy (older, other channel, other architecture).
    NotApplicable(String),
    /// Something unreadable: not JSON, an unknown schema, a path that escapes the folder.
    Malformed(String),
    /// The network said no (no connection, a bad status, a body over its limit).
    Network(String),
    /// A file operation failed. `what` says which step.
    Io { what: String, source: std::io::Error },
    /// The folder cannot be written (read-only medium, no permission, disk full).
    NotWritable(String),
    /// BDF, the command line or the worker is still running from this copy. Nothing was changed.
    InUse(String),
    /// This copy is not a managed portable package, so it cannot update itself.
    Unmanaged(String),
    /// The step was cancelled by the user.
    Cancelled,
    /// The replacement failed part-way and the previous version was put back, complete.
    RolledBack(String),
}

impl std::fmt::Display for UpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UpdateError::NoTrustRoot => {
                f.write_str("this build of BDF has no release signing key, so it cannot verify updates and will not install any")
            }
            UpdateError::Tampered(m) => write!(f, "the update failed verification and was discarded ({m})"),
            UpdateError::NotApplicable(m) => write!(f, "this release does not apply: {m}"),
            UpdateError::Malformed(m) => write!(f, "unreadable update information ({m})"),
            UpdateError::Network(m) => write!(f, "couldn't reach the update server ({m})"),
            UpdateError::Io { what, source } => write!(f, "{what}: {source}"),
            UpdateError::NotWritable(m) => write!(f, "{m}"),
            UpdateError::InUse(m) => write!(f, "{m}"),
            UpdateError::Unmanaged(m) => write!(f, "{m}"),
            UpdateError::Cancelled => f.write_str("the update was cancelled"),
            UpdateError::RolledBack(m) => write!(f, "the update could not be completed, so BDF was restored to the version it was ({m})"),
        }
    }
}

impl std::error::Error for UpdateError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            UpdateError::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// `Result` for this crate.
pub type Result<T, E = UpdateError> = std::result::Result<T, E>;

/// Attach a step name to an I/O failure; a full disk and a read-only folder get their own wording.
pub(crate) fn io_err(what: impl Into<String>) -> impl FnOnce(std::io::Error) -> UpdateError {
    let what = what.into();
    move |source| classify_io(what, source)
}

pub(crate) fn classify_io(what: String, source: std::io::Error) -> UpdateError {
    use std::io::ErrorKind;
    // 112 = ERROR_DISK_FULL, 39 = ERROR_HANDLE_DISK_FULL, 28 = ENOSPC.
    let full = matches!(source.raw_os_error(), Some(112 | 39 | 28)) || source.kind() == ErrorKind::StorageFull;
    if full {
        return UpdateError::NotWritable(format!("{what}: the disk is full; free some space and try again"));
    }
    if source.kind() == ErrorKind::PermissionDenied || source.kind() == ErrorKind::ReadOnlyFilesystem {
        return UpdateError::NotWritable(format!("{what}: this folder is read-only for you; move BDF to a folder you can write to ({source})"));
    }
    UpdateError::Io { what, source }
}

/// The platform words the manifest uses for this build's package (`windows`, `x64`, ...).
pub fn this_platform() -> (&'static str, &'static str) {
    let os = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    };
    let arch = if cfg!(target_arch = "x86_64") {
        "x64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "other"
    };
    (os, arch)
}

/// Current Unix time in seconds (0 when the clock is unusable).
pub fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Lowercase hex of `bytes`.
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        // Writing to a String cannot fail.
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// The bytes of `hex` (upper or lower case); `None` for an odd length or a non-hex character.
pub fn unhex(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    hex.as_bytes()
        .chunks(2)
        .map(|pair| {
            let digit = |c: u8| (c as char).to_digit(16);
            Some((digit(*pair.first()?)? * 16 + digit(*pair.get(1)?)?) as u8)
        })
        .collect()
}
