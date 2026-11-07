//! Writing the portable package (release tooling; `cargo xtask portable-package` uses it).
//!
//! The package is a ZIP with one top-level folder, `bdf-<version>-<os>-<arch>/`, holding the program files
//! and `bdf-package.json`, the list of every other file with its size and SHA-256. Entries are written in a
//! fixed order with a fixed timestamp, so the same inputs give the same bytes.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use zip::write::SimpleFileOptions;

use crate::manifest::PackageEntry;
use crate::package::{self, FileEntry, PACKAGE_FILE, PackageInfo, hash_file, safe_rel_path};
use crate::{Result, UpdateError, Version, io_err};

/// One file to put in the package.
pub struct Input {
    /// Where it goes, relative and `/`-separated.
    pub rel: String,
    pub source: PathBuf,
}

/// Write `out` and return its manifest entry (size and hash; the caller fills in the address).
///
/// The result is verified (`verify_zip`) before it is returned, so a package that would not install is never
/// produced.
pub fn write_package(out: &Path, version: &str, os: &str, arch: &str, inputs: &[Input]) -> Result<PackageEntry> {
    let parsed = Version::parse(version).ok_or_else(|| UpdateError::Malformed(format!("`{version}` is not a release version")))?;
    let mut inputs: Vec<&Input> = inputs.iter().collect();
    inputs.sort_by(|a, b| a.rel.cmp(&b.rel));
    let mut files = Vec::new();
    for i in &inputs {
        if !safe_rel_path(&i.rel) || i.rel == PACKAGE_FILE {
            return Err(UpdateError::Malformed(format!("`{}` cannot be in a package", i.rel)));
        }
        let (size, sha256) = hash_file(&i.source).map_err(io_err(format!("couldn't read {}", i.source.display())))?;
        files.push(FileEntry { path: i.rel.clone(), size, sha256 });
    }
    let info = PackageInfo { schema: 1, product: "bdf".into(), version: parsed.to_string(), os: os.into(), arch: arch.into(), files };
    let info_bytes = serde_json::to_vec_pretty(&info).map_err(|e| UpdateError::Malformed(e.to_string()))?;
    let top = format!("bdf-{parsed}-{os}-{arch}");

    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).map_err(io_err("couldn't create the output folder"))?;
    }
    let mut zip = zip::ZipWriter::new(File::create(out).map_err(io_err(format!("couldn't create {}", out.display())))?);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).compression_level(Some(9));
    let zip_err = |e: zip::result::ZipError| UpdateError::Malformed(format!("couldn't write the package ({e})"));
    for i in &inputs {
        zip.start_file(format!("{top}/{}", i.rel), options).map_err(zip_err)?;
        let mut src = File::open(&i.source).map_err(io_err(format!("couldn't read {}", i.source.display())))?;
        std::io::copy(&mut src, &mut zip).map_err(io_err("couldn't write the package"))?;
    }
    zip.start_file(format!("{top}/{PACKAGE_FILE}"), options).map_err(zip_err)?;
    zip.write_all(&info_bytes).map_err(io_err("couldn't write the package"))?;
    zip.finish().map_err(zip_err)?;

    let (size, sha256) = hash_file(out).map_err(io_err("couldn't read the finished package"))?;
    let entry = PackageEntry { os: os.into(), arch: arch.into(), kind: "portable-zip".into(), url: String::new(), size, sha256 };
    package::verify_zip(out, &entry, parsed, os, arch)?;
    Ok(entry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_built_package_verifies_installs_and_is_reproducible() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("a.exe"), b"program").unwrap();
        std::fs::write(src.join("b.txt"), b"licence").unwrap();
        let inputs = [Input { rel: "b.txt".into(), source: src.join("b.txt") }, Input { rel: "sub/a.exe".into(), source: src.join("a.exe") }];
        let one = write_package(&dir.path().join("one.zip"), "0.8.1", "windows", "x64", &inputs).unwrap();
        let two = write_package(&dir.path().join("two.zip"), "0.8.1", "windows", "x64", &inputs).unwrap();
        assert_eq!(one, two, "the same inputs make the same package");
        let verified = package::verify_zip(&dir.path().join("one.zip"), &one, Version(0, 8, 1), "windows", "x64").unwrap();
        assert_eq!(verified.top, "bdf-0.8.1-windows-x64");
        assert_eq!(verified.info.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["b.txt", "sub/a.exe"]);
    }

    #[test]
    fn unsafe_names_and_missing_files_are_refused_at_build_time() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("f");
        std::fs::write(&f, b"x").unwrap();
        for rel in ["../x", "BDFData/x", "bdf-package.json", "a\\b", ""] {
            let r = write_package(&dir.path().join("o.zip"), "0.8.1", "windows", "x64", &[Input { rel: rel.into(), source: f.clone() }]);
            assert!(r.is_err(), "{rel:?}");
        }
        assert!(
            write_package(&dir.path().join("o.zip"), "0.8.1", "windows", "x64", &[Input { rel: "x".into(), source: dir.path().join("missing") }])
                .is_err()
        );
        assert!(write_package(&dir.path().join("o.zip"), "latest", "windows", "x64", &[]).is_err());
    }
}
