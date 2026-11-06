//! Fixtures shared by this crate's tests and the helper's: a signing key, real packages, installed copies.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

pub use crate::manifest::testing::{SHA, manifest_json, signed};
pub use crate::package::testing::{Pkg, build, build_with};
pub use crate::trust::testkeys::{sign, signing_key};
use crate::{Channel, SignedManifest, Trust, Version, package};

/// The trust that accepts what [`release`] signs.
pub fn trust() -> Trust {
    crate::trust::testkeys::trust(1, Channel::Stable)
}

/// The test trust on `channel` (a development build trusts the same throwaway key).
pub fn trust_for(channel: Channel) -> Trust {
    crate::trust::testkeys::trust(1, channel)
}

/// An installed managed copy of `files` at `version` in `root/app`, with the user's data and a stranger's file.
pub fn installed_copy(root: &Path, version: &str, files: &[(&str, &[u8])]) -> PathBuf {
    let app = root.join("app");
    std::fs::create_dir_all(&app).unwrap();
    let pkg = build(root, "install.zip", version, files);
    let v = package::verify_zip(&pkg.zip, &pkg.entry, Version::parse(version).unwrap(), "windows", "x64").unwrap();
    package::stage(&pkg.zip, &v, &app).unwrap();
    std::fs::remove_file(&pkg.zip).unwrap();
    std::fs::create_dir_all(app.join("BDFData").join("recovery")).unwrap();
    std::fs::write(app.join("BDFData").join("settings.json"), br#"{"theme_choice":"dark"}"#).unwrap();
    std::fs::write(app.join("BDFData").join("recovery").join("doc1.autosave"), b"unsaved work").unwrap();
    std::fs::write(app.join("notes.txt"), b"my own notes").unwrap();
    app
}

/// A signed release of `files` at `version`: its package (in `root/releases`) and the verified manifest.
pub fn release(root: &Path, version: &str, files: &[(&str, &[u8])]) -> (Pkg, SignedManifest) {
    let dir = root.join("releases");
    std::fs::create_dir_all(&dir).unwrap();
    let pkg = build(&dir, &format!("bdf-{version}.zip"), version, files);
    let m = signed(1, &manifest_json(version, "stable", &pkg.entry.sha256, pkg.entry.size), Channel::Stable).unwrap();
    (pkg, m)
}
