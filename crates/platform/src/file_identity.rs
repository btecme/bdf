//! Which version of a file a save is allowed to replace (BUILD-006, finding F03).
//!
//! A document is opened from a file, edited in memory, and later saved back. Between the open and the save the
//! file may have been changed by another program, or by another session of this one. Saving over it without
//! looking silently destroys that newer version. The rule here is optimistic concurrency at the moment of the
//! commit: the save names the version it is allowed to replace, and is refused, with both versions left alone,
//! if the file on disk is not that version.
//!
//! * A **version** is the file's content fingerprint (SHA-256 of its bytes) and length. The content decides, not
//!   the timestamp: a file rewritten with different bytes of the same length and the same modification time is a
//!   different version, and one that was merely touched is not. The size, modification time and the platform's
//!   stamp are kept for display and for the cheap "did anything change" test elsewhere, never for the decision.
//! * Three **authorities** say what a save may replace ([`Authority`]): nothing (the file must not exist), the
//!   version this session last read or wrote (tracked), or a version the caller has inspected and names by its
//!   fingerprint (confirmed). There is no mode that replaces whatever is there.
//! * The check runs inside the save, after the new bytes are staged, flushed and closed and immediately before
//!   they replace the target ([`crate::persistence::Steps::guard`]). It is not a pre-flight check followed by
//!   a write long after. What it cannot do is close the last gap: a write that lands between the check and the
//!   replacement is not seen. Windows' `ReplaceFile` fails if another process has the target open without sharing,
//!   which covers the common case, and nothing in the standard library locks a file against writers; this is stated
//!   in `docs/bdf-evidence/BUILD-006.md` rather than hidden.

use std::fs::{File, Metadata};
use std::io::{self, Read};
use std::path::Path;
use std::time::SystemTime;

use sha2::{Digest, Sha256};

use crate::persistence::{Committed, PersistError, Steps, write_replace_with};
use crate::staging::{StagingName, staging_suffixes};

/// One version of a file: what its bytes are, and what the filesystem said about it when they were read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileVersion {
    pub len: u64,
    /// SHA-256 of the file's bytes.
    pub sha256: [u8; 32],
    /// The modification time the filesystem reported, for display. Not compared.
    pub modified: Option<SystemTime>,
}

impl FileVersion {
    /// The version of `bytes`, read from a file whose metadata was `meta` (if known).
    pub fn of_bytes(bytes: &[u8], meta: Option<&Metadata>) -> Self {
        Self { len: bytes.len() as u64, sha256: Sha256::digest(bytes).into(), modified: meta.and_then(|m| m.modified().ok()) }
    }

    /// The fingerprint as lowercase hex, the form an agent or a dialog passes back to confirm a version.
    pub fn fingerprint(&self) -> String {
        hex(&self.sha256)
    }

    /// Whether `other` has the same bytes. The length is implied by the fingerprint but checked anyway.
    pub fn same_content(&self, other: &FileVersion) -> bool {
        self.len == other.len && self.sha256 == other.sha256
    }
}

fn hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Parse a fingerprint written by [`FileVersion::fingerprint`] (64 hex digits, any case).
pub fn parse_fingerprint(text: &str) -> Option<[u8; 32]> {
    let text = text.trim();
    if text.len() != 64 || !text.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(text.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

/// Read the file at `path` and say which version those bytes are. The metadata comes from the same open handle,
/// so the version describes the bytes returned, not whatever the path names a moment later.
pub fn read_with_version(path: &Path) -> io::Result<(Vec<u8>, FileVersion)> {
    read_with_version_limited(path, u64::MAX)
}

/// [`read_with_version`], refusing a file of more than `limit` bytes. The limit is applied to the handle that is read
/// from, twice: to the length it reports before anything is allocated, and to what is actually read, so a file that
/// grows between the two is still stopped.
pub fn read_with_version_limited(path: &Path, limit: u64) -> io::Result<(Vec<u8>, FileVersion)> {
    let file = File::open(path)?;
    read_handle(file, limit)
}

fn read_handle(mut file: File, limit: u64) -> io::Result<(Vec<u8>, FileVersion)> {
    let meta = file.metadata()?;
    let too_large = |size: u64| io::Error::new(io::ErrorKind::FileTooLarge, format!("{size} bytes is more than the {limit} byte limit"));
    if meta.len() > limit {
        return Err(too_large(meta.len()));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(meta.len()).unwrap_or(0).min(1 << 28));
    // One byte more than the limit is enough to know it was exceeded.
    (&mut file).take(limit.saturating_add(1)).read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > limit {
        return Err(too_large(u64::try_from(bytes.len()).unwrap_or(u64::MAX)));
    }
    let version = FileVersion::of_bytes(&bytes, Some(&meta));
    Ok((bytes, version))
}

/// [`read_with_version_limited`] for a file that must itself be a regular file: a symbolic link is refused instead of
/// followed. The size limit applies to the handle that was checked, like everything else here.
///
/// A confined reader pins the folder the file is in, but the file name is a different matter: a link named like the
/// document, put there by a program that can write the folder, would carry the read outside it. The check is made on
/// the handle that is read from, so there is no gap between looking and opening.
///
/// * **Unix:** the file is opened with `O_NOFOLLOW`, which fails on a link, and the handle must be a regular file.
/// * **Windows:** it is opened with `FILE_FLAG_OPEN_REPARSE_POINT`, which opens a link itself rather than its target,
///   and the handle must be a regular file with no reparse point. A reparse point that is not a link (a cloud
///   placeholder, a deduplicated file) cannot be read through that handle, so it is opened again the ordinary way
///   once the handle has shown it is not a link; for those files only, a link swapped in during that instant is not
///   seen.
///
/// A hard link to a file outside the folder cannot be told from the file itself and is not covered.
pub fn read_regular_with_version(path: &Path, limit: u64) -> io::Result<(Vec<u8>, FileVersion)> {
    let refused = || io::Error::new(io::ErrorKind::InvalidInput, "a link, not a regular file");
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Not `FILE_FLAG`-style magic: the platform's own `O_NOFOLLOW`, whatever its value is on this system.
        options.custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let mut file = options.open(path).map_err(open_failure)?;
    let mut meta = file.metadata()?;
    if meta.file_type().is_symlink() || !(meta.is_file() || has_reparse_point(&meta)) {
        return Err(refused());
    }
    if has_reparse_point(&meta) {
        // Not a link (checked above) but a reparse point that must be opened the ordinary way to be read.
        file = File::open(path)?;
        meta = file.metadata()?;
        if !meta.is_file() {
            return Err(refused());
        }
    }
    read_handle(file, limit)
}

/// `O_NOFOLLOW` reports a link as "too many levels of symbolic links" (ELOOP; EMLINK on FreeBSD): say what it was.
fn open_failure(e: io::Error) -> io::Error {
    #[cfg(unix)]
    if matches!(rustix::io::Errno::from_io_error(&e), Some(rustix::io::Errno::LOOP | rustix::io::Errno::MLINK)) {
        return io::Error::new(io::ErrorKind::InvalidInput, "a link, not a regular file");
    }
    e
}

#[cfg(windows)]
fn has_reparse_point(meta: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn has_reparse_point(_: &Metadata) -> bool {
    false
}

/// The version of the file at `path` now: `None` if there is no regular file there. Streamed, so a large file is not
/// held in memory.
pub fn current(path: &Path) -> io::Result<Option<FileVersion>> {
    // Look before opening: opening a folder fails (on Windows with "access denied"), and a folder is simply not a
    // file version.
    match std::fs::metadata(path) {
        Ok(m) if m.is_file() => {}
        Ok(_) => return Ok(None),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    }
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let meta = file.metadata()?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    let mut len = 0u64;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        len += n as u64;
    }
    Ok(Some(FileVersion { len, sha256: hasher.finalize().into(), modified: meta.modified().ok() }))
}

/// What a save may replace.
#[derive(Clone, Copy, Debug)]
pub enum Authority<'a> {
    /// Nothing: the file must not exist.
    CreateNew,
    /// The version this session last read or wrote.
    ReplaceTracked(&'a FileVersion),
    /// A version the caller has inspected, named by its fingerprint.
    ReplaceConfirmed(&'a [u8; 32]),
}

/// Why a guarded save was refused. Both versions are intact: the file on disk was not touched and the caller still
/// holds the bytes it meant to write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Conflict {
    /// The save was to create the file, and something is there (`actual` is what).
    Exists { actual: FileVersion },
    /// The save was to replace a version that is no longer the one on disk.
    Changed { expected: [u8; 32], actual: FileVersion },
    /// The save was to replace a version, and the file is gone.
    Gone { expected: [u8; 32] },
}

impl std::fmt::Display for Conflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Conflict::Exists { actual } => write!(f, "a file already exists there (version {}); nothing was overwritten", actual.fingerprint()),
            Conflict::Changed { expected, actual } => write!(
                f,
                "the file changed since it was read: it was version {} and is now {}; nothing was overwritten",
                hex(expected),
                actual.fingerprint()
            ),
            Conflict::Gone { expected } => {
                write!(f, "the file was deleted since it was read (it was version {}); nothing was written", hex(expected))
            }
        }
    }
}

impl std::error::Error for Conflict {}

/// Check `authority` against the file at `path` as it is now.
pub fn check(path: &Path, authority: Authority) -> io::Result<()> {
    let now = current(path)?;
    let conflict = match (authority, now) {
        (Authority::CreateNew, None) => return Ok(()),
        (Authority::CreateNew, Some(actual)) => Conflict::Exists { actual },
        (Authority::ReplaceTracked(base), Some(actual)) if base.same_content(&actual) => return Ok(()),
        (Authority::ReplaceTracked(base), Some(actual)) => Conflict::Changed { expected: base.sha256, actual },
        (Authority::ReplaceTracked(base), None) => Conflict::Gone { expected: base.sha256 },
        (Authority::ReplaceConfirmed(hash), Some(actual)) if actual.sha256 == *hash => return Ok(()),
        (Authority::ReplaceConfirmed(hash), Some(actual)) => Conflict::Changed { expected: *hash, actual },
        (Authority::ReplaceConfirmed(hash), None) => Conflict::Gone { expected: *hash },
    };
    Err(io::Error::new(io::ErrorKind::AlreadyExists, conflict))
}

/// The conflict inside a failed save, if that is why it failed.
pub fn conflict_of(error: &PersistError) -> Option<&Conflict> {
    error.source.get_ref().and_then(|inner| inner.downcast_ref::<Conflict>())
}

/// Save `bytes` over `path`, but only if `authority` still holds when the new bytes are about to replace the file.
pub fn write_guarded(path: &Path, bytes: &[u8], form: StagingName, authority: Authority) -> Result<Committed, PersistError> {
    write_guarded_with(path, bytes, form, authority, staging_suffixes(), &Steps::real())
}

/// [`write_guarded`], trying the staging names `suffixes` give and performing the operations `steps` give (its
/// `guard` is replaced by this function's).
pub fn write_guarded_with(
    path: &Path,
    bytes: &[u8],
    form: StagingName,
    authority: Authority,
    suffixes: impl IntoIterator<Item = u64>,
    steps: &Steps,
) -> Result<Committed, PersistError> {
    let guard = |target: &Path| check(target, authority);
    let steps = Steps { guard: &guard, ..*steps };
    write_replace_with(path, bytes, form, suffixes, &steps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn dir(test: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pdfcraft-identity-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn save(path: &Path, bytes: &[u8], authority: Authority) -> Result<Committed, PersistError> {
        write_guarded(path, bytes, StagingName::SuffixThenTag, authority)
    }

    fn names(d: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(d).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    }

    #[test]
    fn a_fingerprint_round_trips_and_malformed_ones_are_refused() {
        let v = FileVersion::of_bytes(b"hello", None);
        let text = v.fingerprint();
        assert_eq!(text.len(), 64);
        assert_eq!(parse_fingerprint(&text), Some(v.sha256));
        assert_eq!(parse_fingerprint(&text.to_uppercase()), Some(v.sha256));
        let not_hex = "g".repeat(64);
        for bad in ["", "zz", &text[..63], &format!("{text}0"), &not_hex] {
            assert_eq!(parse_fingerprint(bad), None, "{bad:?}");
        }
        assert_eq!(parse_fingerprint("é".repeat(32).as_str()), None, "non-ASCII input is refused without slicing it");
    }

    /// F03: the file changed after it was read; the save must be refused and both versions kept.
    #[test]
    fn a_save_over_a_file_changed_since_it_was_read_is_refused_and_keeps_both_versions() {
        let d = dir("changed");
        let path = d.join("doc.pdf");
        std::fs::write(&path, b"VERSION ONE").unwrap();
        let (_, opened) = read_with_version(&path).unwrap();
        std::fs::write(&path, b"EXTERNAL EDIT").unwrap();
        let e = save(&path, b"MY EDIT", Authority::ReplaceTracked(&opened)).unwrap_err();
        let conflict = conflict_of(&e).expect("a structured conflict");
        let Conflict::Changed { expected, actual } = conflict else { panic!("{conflict:?}") };
        assert_eq!(*expected, opened.sha256);
        assert_eq!(actual.sha256, FileVersion::of_bytes(b"EXTERNAL EDIT", None).sha256);
        assert_eq!(std::fs::read(&path).unwrap(), b"EXTERNAL EDIT", "the newer version on disk is untouched");
        assert_eq!(names(&d), ["doc.pdf"], "no staging file is left");
        assert!(e.to_string().contains("nothing was overwritten") && e.to_string().contains(&actual.fingerprint()));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The decision is the content: same length and same modification time, different bytes, is a conflict; the
    /// same bytes after a touch is not.
    #[test]
    fn identical_size_and_timestamp_but_different_bytes_is_detected_and_a_mere_touch_is_not() {
        let d = dir("same-stamp");
        let path = d.join("doc.pdf");
        std::fs::write(&path, b"AAAAAAAAAA").unwrap();
        let (_, opened) = read_with_version(&path).unwrap();
        let mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
        // Different bytes, same length, the modification time put back.
        std::fs::write(&path, b"BBBBBBBBBB").unwrap();
        std::fs::File::options().write(true).open(&path).unwrap().set_modified(mtime).unwrap();
        let after = std::fs::metadata(&path).unwrap();
        assert_eq!((after.len(), after.modified().unwrap()), (10, mtime), "the file now looks exactly as it did");
        let e = save(&path, b"MINE", Authority::ReplaceTracked(&opened)).unwrap_err();
        assert!(matches!(conflict_of(&e), Some(Conflict::Changed { .. })), "{e}");
        assert_eq!(std::fs::read(&path).unwrap(), b"BBBBBBBBBB");
        // The reverse: same bytes, a later modification time. Not a change.
        std::fs::write(&path, b"AAAAAAAAAA").unwrap();
        std::fs::File::options().write(true).open(&path).unwrap().set_modified(mtime + std::time::Duration::from_secs(3600)).unwrap();
        save(&path, b"MINE", Authority::ReplaceTracked(&opened)).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"MINE");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn creating_never_replaces_and_a_deleted_file_is_not_silently_recreated() {
        let d = dir("create");
        let path = d.join("new.pdf");
        save(&path, b"FIRST", Authority::CreateNew).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"FIRST");
        let e = save(&path, b"SECOND", Authority::CreateNew).unwrap_err();
        assert!(matches!(conflict_of(&e), Some(Conflict::Exists { .. })), "{e}");
        assert_eq!(std::fs::read(&path).unwrap(), b"FIRST");
        // A tracked file deleted by someone else: refused, not recreated.
        let (_, v) = read_with_version(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        let e = save(&path, b"THIRD", Authority::ReplaceTracked(&v)).unwrap_err();
        assert!(matches!(conflict_of(&e), Some(Conflict::Gone { .. })), "{e}");
        assert!(!path.exists());
        assert_eq!(names(&d), Vec::<String>::new());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A version the caller inspected can be replaced; any other cannot.
    #[test]
    fn a_confirmed_version_is_replaced_and_a_stale_confirmation_is_refused() {
        let d = dir("confirmed");
        let path = d.join("other.pdf");
        std::fs::write(&path, b"SOMEONE ELSE'S").unwrap();
        let inspected = current(&path).unwrap().unwrap();
        // The file changes after it was inspected: the confirmation no longer covers it.
        std::fs::write(&path, b"CHANGED AFTER INSPECTION").unwrap();
        let e = save(&path, b"MINE", Authority::ReplaceConfirmed(&inspected.sha256)).unwrap_err();
        assert!(matches!(conflict_of(&e), Some(Conflict::Changed { .. })), "{e}");
        assert_eq!(std::fs::read(&path).unwrap(), b"CHANGED AFTER INSPECTION");
        let now = current(&path).unwrap().unwrap();
        save(&path, b"MINE", Authority::ReplaceConfirmed(&now.sha256)).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"MINE");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The check is made after the new bytes are staged and flushed, immediately before the replacement: a change made
    /// while the bytes were being prepared is seen.
    #[test]
    fn the_check_is_made_after_staging_not_before() {
        let d = dir("late");
        let path = d.join("doc.pdf");
        std::fs::write(&path, b"ONE").unwrap();
        let (_, v) = read_with_version(&path).unwrap();
        let steps = Steps {
            // The "other program" writes while this one is writing its staging file.
            write: &|file, bytes| {
                std::fs::write(&path, b"WRITTEN WHILE STAGING").unwrap();
                io::Write::write_all(file, bytes)
            },
            ..Steps::real()
        };
        let e =
            write_guarded_with(&path, b"MINE", StagingName::SuffixThenTag, Authority::ReplaceTracked(&v), staging_suffixes(), &steps).unwrap_err();
        assert!(matches!(conflict_of(&e), Some(Conflict::Changed { .. })), "{e}");
        assert_eq!(std::fs::read(&path).unwrap(), b"WRITTEN WHILE STAGING");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn reading_returns_the_version_of_the_bytes_it_returned() {
        let d = dir("read");
        let path = d.join("doc.pdf");
        std::fs::write(&path, b"CONTENT").unwrap();
        let (bytes, v) = read_with_version(&path).unwrap();
        assert_eq!(bytes, b"CONTENT");
        assert!(v.same_content(&FileVersion::of_bytes(b"CONTENT", None)));
        assert_eq!(current(&path).unwrap().unwrap().sha256, v.sha256);
        assert_eq!(current(&d.join("missing")).unwrap(), None);
        assert_eq!(current(&d).unwrap(), None, "a folder is not a file version");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A file link `at` -> `to`; `false` if this machine will not make one.
    fn link_file(to: &Path, at: &Path) -> bool {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(to, at).is_ok()
        }
        #[cfg(windows)]
        {
            std::os::windows::fs::symlink_file(to, at).is_ok()
        }
    }

    #[test]
    fn a_strict_read_returns_a_regular_file_and_refuses_a_link_or_a_folder() {
        let d = dir("strict-read");
        let (real, linked) = (d.join("real.bin"), d.join("linked.bin"));
        std::fs::write(&real, b"inside").unwrap();
        let (bytes, version) = read_regular_with_version(&real, u64::MAX).unwrap();
        assert_eq!(bytes, b"inside");
        assert!(version.same_content(&FileVersion::of_bytes(b"inside", None)));
        assert!(read_regular_with_version(&d, u64::MAX).is_err(), "a folder is not a file");
        assert!(read_regular_with_version(&d.join("missing"), u64::MAX).is_err());

        if link_file(&real, &linked) {
            assert!(read_with_version(&linked).is_ok(), "the ordinary read follows a link, as people expect");
            let error = read_regular_with_version(&linked, u64::MAX).expect_err("the strict read does not");
            assert!(error.to_string().contains("link"), "{error}");
        } else {
            eprintln!("SKIPPED the link half: this machine cannot create a file symbolic link");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_read_stops_at_its_limit_on_the_handle_it_reads() {
        let d = dir("read-limit");
        let path = d.join("ten.bin");
        std::fs::write(&path, b"0123456789").unwrap();
        assert_eq!(read_with_version_limited(&path, 10).unwrap().0, b"0123456789", "exactly the limit is fine");
        for limit in [9, 0] {
            let error = read_with_version_limited(&path, limit).expect_err("over the limit");
            assert_eq!(error.kind(), io::ErrorKind::FileTooLarge, "{limit}");
            assert_eq!(read_regular_with_version(&path, limit).expect_err("over the limit").kind(), io::ErrorKind::FileTooLarge, "{limit}");
        }
        assert_eq!(read_regular_with_version(&path, 10).unwrap().0.len(), 10);
        let _ = std::fs::remove_dir_all(&d);
    }
}
