//! The portable package: a ZIP with one top-level folder whose files are listed, with sizes and SHA-256
//! hashes, in `bdf-package.json` inside it.
//!
//! The package as a whole is authenticated by the signed manifest (its size and SHA-256), and
//! [`verify_zip`] checks that hash **before** it opens the archive, so the ZIP reader only ever sees bytes the
//! release key vouched for. It then checks the contents against the package's own list: every file present
//! once with the listed size and hash, nothing unlisted, every path safe. Only after all of that does
//! [`stage`] write anything, and it checks each hash again as it writes.
//!
//! The same `bdf-package.json` is installed beside `bdf.exe`. It is what makes a copy *managed*, and its list
//! is what the replacement works from: BDF never touches a file it does not list, so the user's
//! `BDFData` folder and anything else they put beside the program are left alone.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zip::ZipArchive;

use crate::manifest::{PackageEntry, is_sha256_hex};
use crate::{Result, UpdateError, Version, hex, io_err};

/// The package list, installed beside the program.
pub const PACKAGE_FILE: &str = "bdf-package.json";
/// A package has a few dozen files; this is a ceiling against a hostile archive.
pub const MAX_FILES: usize = 2000;
pub const MAX_FILE_BYTES: u64 = 400 * 1024 * 1024;
pub const MAX_TOTAL_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_INFO_BYTES: u64 = 1024 * 1024;
/// Folders the package may never write into: the user's data and the updater's own working folders.
const RESERVED_TOP: [&str; 3] = ["bdfdata", "bdfupdate", "bdfprevious"];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    /// Relative, `/`-separated.
    pub path: String,
    pub size: u64,
    /// Lowercase hex SHA-256 of the file.
    pub sha256: String,
}

/// `bdf-package.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageInfo {
    pub schema: u32,
    pub product: String,
    pub version: String,
    pub os: String,
    pub arch: String,
    pub files: Vec<FileEntry>,
}

impl PackageInfo {
    /// Parse and validate a package list.
    pub fn parse(bytes: &[u8]) -> Result<PackageInfo> {
        let info: PackageInfo = serde_json::from_slice(bytes).map_err(|e| UpdateError::Malformed(format!("the package list is not valid ({e})")))?;
        info.validate()?;
        Ok(info)
    }

    fn validate(&self) -> Result<()> {
        if self.schema != 1 || self.product != "bdf" {
            return Err(UpdateError::Malformed("the package list is not a BDF package list".into()));
        }
        Version::parse(&self.version).ok_or_else(|| UpdateError::Malformed("the package list has no version".into()))?;
        if self.files.is_empty() || self.files.len() > MAX_FILES {
            return Err(UpdateError::Malformed("the package lists an impossible number of files".into()));
        }
        let mut seen = HashMap::new();
        let mut total = 0u64;
        for f in &self.files {
            if !safe_rel_path(&f.path) || f.path.eq_ignore_ascii_case(PACKAGE_FILE) {
                return Err(UpdateError::Malformed(format!("the package lists an unsafe path ({})", f.path.chars().take(60).collect::<String>())));
            }
            if f.size > MAX_FILE_BYTES || !is_sha256_hex(&f.sha256) {
                return Err(UpdateError::Malformed(format!("the package lists a bad size or hash for {}", f.path)));
            }
            total = total.saturating_add(f.size);
            if seen.insert(f.path.to_ascii_lowercase(), ()).is_some() {
                return Err(UpdateError::Malformed(format!("the package lists {} twice", f.path)));
            }
        }
        if total > MAX_TOTAL_BYTES {
            return Err(UpdateError::Malformed("the package is larger than any BDF release".into()));
        }
        // A file may not also be a folder of another entry (`a` and `a/b`).
        for f in &self.files {
            let mut prefix = String::new();
            for part in f.path.split('/').take(f.path.matches('/').count()) {
                if !prefix.is_empty() {
                    prefix.push('/');
                }
                prefix.push_str(part);
                if seen.contains_key(&prefix.to_ascii_lowercase()) {
                    return Err(UpdateError::Malformed(format!("the package lists {prefix} as both a file and a folder")));
                }
            }
        }
        Ok(())
    }

    pub fn parsed_version(&self) -> Result<Version> {
        Version::parse(&self.version).ok_or_else(|| UpdateError::Malformed("the package list has no version".into()))
    }

    pub fn get(&self, path: &str) -> Option<&FileEntry> {
        self.files.iter().find(|f| f.path.eq_ignore_ascii_case(path))
    }
}

/// Whether `path` is a relative `/`-separated path that stays inside the folder it is joined to and is a valid
/// name on Windows, macOS and Linux alike.
pub fn safe_rel_path(path: &str) -> bool {
    if path.is_empty() || path.len() > 200 {
        return false;
    }
    let mut parts = path.split('/').peekable();
    let first = parts.peek().copied().unwrap_or("");
    if RESERVED_TOP.contains(&first.to_ascii_lowercase().as_str()) {
        return false;
    }
    parts.all(|part| {
        if part.is_empty() || part == "." || part == ".." || part.len() > 100 || part.ends_with(['.', ' ']) || part.starts_with(' ') {
            return false;
        }
        if part.chars().any(|c| c.is_control() || matches!(c, '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')) {
            return false;
        }
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        !(matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || ((stem.starts_with("COM") || stem.starts_with("LPT")) && stem.len() == 4 && stem.ends_with(|c: char| c.is_ascii_digit())))
    })
}

/// Stream `reader` into a SHA-256, refusing more than `limit` bytes. Returns the length and the hex hash.
pub fn hash_reader(mut reader: impl Read, limit: u64) -> std::io::Result<(u64, String)> {
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut len = 0u64;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        len += n as u64;
        if len > limit {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "more data than declared"));
        }
        hasher.update(buf.get(..n).unwrap_or(&[]));
    }
    Ok((len, hex(&hasher.finalize())))
}

/// Lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// Size and SHA-256 of a file.
pub fn hash_file(path: &Path) -> std::io::Result<(u64, String)> {
    hash_reader(BufReader::new(File::open(path)?), u64::MAX)
}

/// A package that passed [`verify_zip`].
#[derive(Debug)]
pub struct Verified {
    pub info: PackageInfo,
    /// The exact bytes of `bdf-package.json` (installed beside the program).
    pub info_bytes: Vec<u8>,
    /// The archive's single top-level folder.
    pub top: String,
}

/// Verify the whole package at `zip_path` against the signed `expected` entry. Writes nothing.
pub fn verify_zip(zip_path: &Path, expected: &PackageEntry, version: Version, os: &str, arch: &str) -> Result<Verified> {
    let file = File::open(zip_path).map_err(io_err(format!("couldn't open {}", zip_path.display())))?;
    let on_disk = file.metadata().map_err(io_err("couldn't read the package size"))?.len();
    if on_disk != expected.size {
        return Err(UpdateError::Tampered(format!("the package is {on_disk} bytes, the signed release says {}", expected.size)));
    }
    let (_, digest) = hash_reader(BufReader::new(&file), expected.size).map_err(io_err("couldn't read the package"))?;
    if digest != expected.sha256 {
        return Err(UpdateError::Tampered("the package does not match the signed release (hash differs)".into()));
    }
    let mut archive = ZipArchive::new(BufReader::new(File::open(zip_path).map_err(io_err("couldn't reopen the package"))?))
        .map_err(|e| UpdateError::Malformed(format!("the package is not a valid ZIP ({e})")))?;
    if archive.len() > MAX_FILES + 256 {
        return Err(UpdateError::Malformed("the package has too many entries".into()));
    }

    // Names first, from the central directory only: one top-level folder, every file unique and safe.
    let mut top: Option<String> = None;
    let mut files: HashMap<String, usize> = HashMap::new();
    let mut declared: HashMap<String, u64> = HashMap::new();
    for i in 0..archive.len() {
        let entry = archive.by_index_raw(i).map_err(|e| UpdateError::Malformed(format!("the package is damaged ({e})")))?;
        let name = entry.name().to_string();
        if entry.encrypted() || entry.is_symlink() {
            return Err(UpdateError::Tampered(format!(
                "the package contains an encrypted or linked entry ({})",
                name.chars().take(60).collect::<String>()
            )));
        }
        let trimmed = name.strip_suffix('/').unwrap_or(&name);
        let (first, rest) = match trimmed.split_once('/') {
            Some((a, b)) => (a, Some(b)),
            None => (trimmed, None),
        };
        if first.is_empty() || !safe_rel_path(first) {
            return Err(UpdateError::Tampered(format!("the package contains an unsafe path ({})", name.chars().take(60).collect::<String>())));
        }
        match &top {
            None => top = Some(first.to_string()),
            Some(t) if t == first => {}
            Some(_) => return Err(UpdateError::Tampered("the package has more than one top-level folder".into())),
        }
        if entry.is_dir() {
            continue;
        }
        let Some(rest) = rest else {
            return Err(UpdateError::Tampered("the package has a file outside its folder".into()));
        };
        if !(safe_rel_path(rest) || rest == PACKAGE_FILE) {
            return Err(UpdateError::Tampered(format!("the package contains an unsafe path ({})", name.chars().take(60).collect::<String>())));
        }
        if files.insert(rest.to_ascii_lowercase(), i).is_some() {
            return Err(UpdateError::Tampered(format!("the package contains {rest} twice")));
        }
        declared.insert(rest.to_ascii_lowercase(), entry.size());
    }
    let top = top.ok_or_else(|| UpdateError::Malformed("the package is empty".into()))?;

    // The list.
    let info_index =
        *files.get(&PACKAGE_FILE.to_ascii_lowercase()).ok_or_else(|| UpdateError::Malformed("the package has no bdf-package.json".into()))?;
    let mut info_bytes = Vec::new();
    {
        let entry = archive.by_index(info_index).map_err(|e| UpdateError::Malformed(format!("couldn't read the package list ({e})")))?;
        entry.take(MAX_INFO_BYTES + 1).read_to_end(&mut info_bytes).map_err(io_err("couldn't read the package list"))?;
    }
    if info_bytes.len() as u64 > MAX_INFO_BYTES {
        return Err(UpdateError::Malformed("the package list is too large".into()));
    }
    let info = PackageInfo::parse(&info_bytes)?;
    if info.parsed_version()? != version {
        return Err(UpdateError::Tampered(format!("the package is version {}, the signed release is {version}", info.version)));
    }
    if info.os != os || info.arch != arch {
        return Err(UpdateError::NotApplicable(format!("the package is for {} {}, this copy is {os} {arch}", info.os, info.arch)));
    }

    // Contents against the list.
    if files.len() != info.files.len() + 1 {
        return Err(UpdateError::Tampered("the package contains files its list does not name".into()));
    }
    for f in &info.files {
        let key = f.path.to_ascii_lowercase();
        let index = *files.get(&key).ok_or_else(|| UpdateError::Tampered(format!("the package is missing {}", f.path)))?;
        if declared.get(&key) != Some(&f.size) {
            return Err(UpdateError::Tampered(format!("{} has a different size than the package list says", f.path)));
        }
        let entry = archive.by_index(index).map_err(|e| UpdateError::Malformed(format!("couldn't read {} ({e})", f.path)))?;
        let (len, digest) =
            hash_reader(entry, f.size).map_err(|e| UpdateError::Tampered(format!("{} does not match the package list ({e})", f.path)))?;
        if len != f.size || digest != f.sha256 {
            return Err(UpdateError::Tampered(format!("{} does not match the package list (hash differs)", f.path)));
        }
    }
    Ok(Verified { info, info_bytes, top })
}

/// Write the verified package's files into `dest` (created; it must be empty or absent), each hashed again
/// as it is written, plus `bdf-package.json`. Nothing outside `dest` is touched.
pub fn stage(zip_path: &Path, verified: &Verified, dest: &Path) -> Result<()> {
    if dest.exists() && std::fs::read_dir(dest).map_err(io_err("couldn't inspect the staging folder"))?.next().is_some() {
        return Err(UpdateError::Io { what: "the staging folder is not empty".into(), source: std::io::Error::other("not empty") });
    }
    std::fs::create_dir_all(dest).map_err(io_err("couldn't create the staging folder"))?;
    let mut archive = ZipArchive::new(BufReader::new(File::open(zip_path).map_err(io_err("couldn't reopen the package"))?))
        .map_err(|e| UpdateError::Malformed(format!("the package is not a valid ZIP ({e})")))?;
    for f in &verified.info.files {
        let name = format!("{}/{}", verified.top, f.path);
        let entry = archive.by_name(&name).map_err(|e| UpdateError::Tampered(format!("the package is missing {} ({e})", f.path)))?;
        let target = join_rel(dest, &f.path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(io_err("couldn't create a staging folder"))?;
        }
        write_hashed(entry, &target, f)?;
    }
    let info_path = dest.join(PACKAGE_FILE);
    std::fs::write(&info_path, &verified.info_bytes).map_err(io_err("couldn't stage the package list"))?;
    Ok(())
}

fn write_hashed(mut entry: impl Read, target: &Path, f: &FileEntry) -> Result<()> {
    let mut out = File::create(target).map_err(io_err(format!("couldn't write {}", f.path)))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut len = 0u64;
    loop {
        let n = entry.read(&mut buf).map_err(io_err(format!("couldn't read {} from the package", f.path)))?;
        if n == 0 {
            break;
        }
        len += n as u64;
        if len > f.size {
            return Err(UpdateError::Tampered(format!("{} is larger than the package list says", f.path)));
        }
        let chunk = buf.get(..n).unwrap_or(&[]);
        hasher.update(chunk);
        out.write_all(chunk).map_err(io_err(format!("couldn't write {}", f.path)))?;
    }
    out.sync_all().map_err(io_err(format!("couldn't flush {}", f.path)))?;
    if len != f.size || hex(&hasher.finalize()) != f.sha256 {
        return Err(UpdateError::Tampered(format!("{} changed while it was staged", f.path)));
    }
    Ok(())
}

/// `root` joined with a validated `/`-separated relative path.
pub fn join_rel(root: &Path, rel: &str) -> PathBuf {
    let mut p = root.to_path_buf();
    for part in rel.split('/') {
        p.push(part);
    }
    p
}

/// Build real ZIP packages for tests (and for the helper's tests, with the `testing` feature).
#[cfg(any(test, feature = "testing"))]
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub mod testing {

    use zip::write::SimpleFileOptions;

    use super::*;

    pub struct Pkg {
        pub zip: PathBuf,
        pub entry: PackageEntry,
    }

    /// A package of `(path, contents)` files at `version`, written to `dir`.
    pub fn build(dir: &Path, name: &str, version: &str, files: &[(&str, &[u8])]) -> Pkg {
        build_with(dir, name, version, files, |_| {})
    }

    /// Like [`build`], letting a test change the list or add raw entries before it is written.
    pub fn build_with(dir: &Path, name: &str, version: &str, files: &[(&str, &[u8])], tweak: impl FnOnce(&mut Spec)) -> Pkg {
        let mut spec = Spec {
            info: PackageInfo {
                schema: 1,
                product: "bdf".into(),
                version: version.into(),
                os: "windows".into(),
                arch: "x64".into(),
                files: files.iter().map(|(p, c)| FileEntry { path: (*p).into(), size: c.len() as u64, sha256: hex(&Sha256::digest(c)) }).collect(),
            },
            entries: files.iter().map(|(p, c)| (format!("top/{p}"), c.to_vec())).collect(),
            info_bytes: None,
        };
        tweak(&mut spec);
        let path = dir.join(name);
        let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
        let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (n, c) in &spec.entries {
            zip.start_file(n, opts).unwrap();
            zip.write_all(c).unwrap();
        }
        zip.start_file(format!("top/{PACKAGE_FILE}"), opts).unwrap();
        zip.write_all(&spec.info_bytes.clone().unwrap_or_else(|| serde_json::to_vec_pretty(&spec.info).unwrap())).unwrap();
        zip.finish().unwrap();
        let (size, sha256) = hash_file(&path).unwrap();
        Pkg {
            entry: PackageEntry { os: "windows".into(), arch: "x64".into(), kind: "portable-zip".into(), url: String::new(), size, sha256 },
            zip: path,
        }
    }

    pub struct Spec {
        pub info: PackageInfo,
        pub entries: Vec<(String, Vec<u8>)>,
        pub info_bytes: Option<Vec<u8>>,
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    const V: Version = Version(0, 8, 1);

    fn verify(p: &Pkg) -> Result<Verified> {
        verify_zip(&p.zip, &p.entry, V, "windows", "x64")
    }

    #[test]
    fn a_complete_package_verifies_and_stages_exactly() {
        let dir = tempfile::tempdir().unwrap();
        let p = build(dir.path(), "a.zip", "0.8.1", &[("bdf.exe", b"exe-bytes"), ("portable.txt", b"p"), ("licenses/MIT.txt", b"license")]);
        let v = verify(&p).unwrap();
        assert_eq!(v.top, "top");
        let stage_dir = dir.path().join("stage");
        stage(&p.zip, &v, &stage_dir).unwrap();
        assert_eq!(std::fs::read(stage_dir.join("bdf.exe")).unwrap(), b"exe-bytes");
        assert_eq!(std::fs::read(stage_dir.join("licenses").join("MIT.txt")).unwrap(), b"license");
        assert_eq!(std::fs::read(stage_dir.join(PACKAGE_FILE)).unwrap(), v.info_bytes);
        // The staging folder is not reused.
        assert!(stage(&p.zip, &v, &stage_dir).is_err());
    }

    #[test]
    fn a_package_that_is_not_the_signed_one_is_rejected_before_it_is_opened() {
        let dir = tempfile::tempdir().unwrap();
        let p = build(dir.path(), "a.zip", "0.8.1", &[("bdf.exe", b"exe")]);
        let mut wrong_hash = p.entry.clone();
        wrong_hash.sha256 = "0".repeat(64);
        assert!(matches!(verify_zip(&p.zip, &wrong_hash, V, "windows", "x64"), Err(UpdateError::Tampered(_))));
        let mut wrong_size = p.entry.clone();
        wrong_size.size += 1;
        assert!(matches!(verify_zip(&p.zip, &wrong_size, V, "windows", "x64"), Err(UpdateError::Tampered(_))));
        // One flipped byte anywhere is caught by the whole-file hash.
        let mut bytes = std::fs::read(&p.zip).unwrap();
        let mid = bytes.len() / 2;
        bytes[mid] ^= 1;
        std::fs::write(&p.zip, &bytes).unwrap();
        assert!(matches!(verify(&p), Err(UpdateError::Tampered(_))));
    }

    #[test]
    fn a_truncated_package_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let p = build(dir.path(), "a.zip", "0.8.1", &[("bdf.exe", b"exe-bytes-exe-bytes")]);
        let bytes = std::fs::read(&p.zip).unwrap();
        std::fs::write(&p.zip, &bytes[..bytes.len() / 2]).unwrap();
        assert!(matches!(verify(&p), Err(UpdateError::Tampered(_))));
    }

    /// A genuine signed hash over a package whose content is wrong (a bad build, not a transfer error).
    #[test]
    fn content_that_disagrees_with_the_package_list_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        type Tweak = Box<dyn FnOnce(&mut Spec)>;
        let cases: Vec<(&str, Tweak)> = vec![
            ("changed content", Box::new(|s| s.entries[0].1 = b"EVIL".to_vec())),
            ("unlisted file", Box::new(|s| s.entries.push(("top/extra.exe".into(), b"x".to_vec())))),
            ("missing file", Box::new(|s| s.info.files.push(FileEntry { path: "gone.dll".into(), size: 1, sha256: "a".repeat(64) }))),
            ("wrong version", Box::new(|s| s.info.version = "0.8.2".into())),
            ("wrong arch", Box::new(|s| s.info.arch = "arm64".into())),
            ("duplicate entry by letter case", Box::new(|s| s.entries.push(("top/BDF.EXE".into(), b"exe".to_vec())))),
            ("two top folders", Box::new(|s| s.entries.push(("other/x".into(), b"x".to_vec())))),
            ("file at the root", Box::new(|s| s.entries.push(("x.exe".into(), b"x".to_vec())))),
            ("traversal in an entry", Box::new(|s| s.entries.push(("top/../evil.exe".into(), b"x".to_vec())))),
            ("data folder in an entry", Box::new(|s| s.entries.push(("top/BDFData/settings.json".into(), b"x".to_vec())))),
            ("backslash entry", Box::new(|s| s.entries.push(("top/a\\b.exe".into(), b"x".to_vec())))),
            ("list names data folder", Box::new(|s| s.info.files.push(FileEntry { path: "BDFData/x".into(), size: 1, sha256: "a".repeat(64) }))),
            ("not json", Box::new(|s| s.info_bytes = Some(b"nope".to_vec()))),
        ];
        for (n, (label, tweak)) in cases.into_iter().enumerate() {
            let p = build_with(dir.path(), &format!("c{n}.zip"), "0.8.1", &[("bdf.exe", b"exe")], tweak);
            assert!(verify(&p).is_err(), "{label} must be rejected");
        }
    }

    #[test]
    fn unsafe_paths_are_refused_everywhere() {
        for ok in ["bdf.exe", "licenses/OFL-Inter.txt", "a/b/c.txt", "x y.txt", "name.with.dots"] {
            assert!(safe_rel_path(ok), "{ok}");
        }
        for bad in [
            "",
            "/abs",
            "a//b",
            "a/../b",
            "..",
            ".",
            "./a",
            "a/.",
            "C:/x",
            "C:x",
            "a\\b",
            "a:b",
            "bdfdata/x",
            "BDFData",
            "BDFUpdate/x",
            "bdfprevious/y",
            "nul",
            "CON.txt",
            "com1",
            "LPT9.log",
            "a/aux/b",
            "trailing.",
            "trailing ",
            " leading",
            "a?b",
            "a*b",
            "a<b",
            "a|b",
            "a\"b",
            "a\u{0}b",
            "a\nb",
            &"x".repeat(101),
            &"a/".repeat(101),
        ] {
            assert!(!safe_rel_path(bad), "{bad:?}");
        }
    }

    #[test]
    fn a_file_may_not_also_be_a_folder() {
        let info = PackageInfo {
            schema: 1,
            product: "bdf".into(),
            version: "0.8.1".into(),
            os: "windows".into(),
            arch: "x64".into(),
            files: vec![
                FileEntry { path: "a".into(), size: 1, sha256: "a".repeat(64) },
                FileEntry { path: "A/b".into(), size: 1, sha256: "b".repeat(64) },
            ],
        };
        assert!(PackageInfo::parse(&serde_json::to_vec(&info).unwrap()).is_err());
    }

    #[test]
    fn a_non_zip_file_with_the_right_hash_is_malformed_not_a_crash() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("junk.zip");
        std::fs::write(&path, b"this is not a zip file at all").unwrap();
        let (size, sha256) = hash_file(&path).unwrap();
        let entry = PackageEntry { os: "windows".into(), arch: "x64".into(), kind: "portable-zip".into(), url: String::new(), size, sha256 };
        assert!(matches!(verify_zip(&path, &entry, V, "windows", "x64"), Err(UpdateError::Malformed(_))));
    }

    #[test]
    fn hashing_stops_at_its_limit() {
        let data = [1u8; 100];
        assert!(hash_reader(&data[..], 99).is_err());
        let (len, _) = hash_reader(&data[..], 100).unwrap();
        assert_eq!(len, 100);
    }
}
