//! The one way the desktop app and the automation tools put bytes into a file the user keeps
//! (BUILD-004, finding F02).
//!
//! A save is: look at the target, write the bytes to a new private file beside it, flush that file to
//! the disk, then replace the target with it. Every step can fail, and a failure at any step before the
//! replacement leaves the target exactly as it was; the failure says which step it was.
//!
//! * **Exclusive staging.** The file is created new, with an unpredictable name, so a name planted in
//!   advance is skipped rather than written through ([`crate::staging`]). It is private while it
//!   holds the document: `0600` on Unix, no sharing on Windows.
//! * **Flush.** The staging file is flushed with `sync_all` before it replaces anything, and a failed
//!   flush fails the save. (macOS asks for a full flush that SMB and FUSE volumes refuse; there a plain
//!   `fsync` is the strongest the volume offers.) On Unix the folder is flushed after the replacement
//!   too, so the new name itself survives a power loss; [`Committed::directory_flushed`] says whether
//!   that happened.
//! * **Evaluated replacement.** The target is inspected first: a folder or a device is refused, and an
//!   existing file's permissions are kept. On Unix the mode is copied to the staging file. On Windows
//!   the replacement is `ReplaceFile`, which keeps the replaced file's access list (a plain rename gives
//!   the new file the folder's inherited one, which can make a restricted document readable by more
//!   people) and refuses a read-only or locked target. `ReplaceFile` can fail part way, so it is given
//!   a backup name, and the original is put back if it is gone afterwards.
//!
//! What this does not do: it does not follow a symbolic link (the link itself is replaced, so a save
//! can't be redirected out of the folder it was aimed at), it does not preserve a file's owner, and on
//! Windows the staging file carries the folder's inherited permissions between being closed and
//! replacing the target, because setting an access list needs unsafe code the workspace forbids.
//! The exclusive handle keeps others out while the bytes are being written.

use std::fs::{File, Metadata};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::staging::{StagingName, create_staging, staging_suffixes};

/// The step of a save that failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Looking at the target: it is a folder or not a regular file.
    Inspect,
    /// Creating the staging file.
    Stage,
    /// Writing the bytes into it.
    Write,
    /// Flushing it to the disk.
    Flush,
    /// Replacing the target with it.
    Replace,
}

impl Stage {
    fn label(self) -> &'static str {
        match self {
            Stage::Inspect => "checking the file",
            Stage::Stage => "creating a temporary file",
            Stage::Write => "writing",
            Stage::Flush => "flushing to disk",
            Stage::Replace => "replacing the file",
        }
    }
}

/// Why a save failed. The target still has the bytes it had before, except in the one case `original_copy`
/// names.
#[derive(Debug)]
pub struct PersistError {
    pub stage: Stage,
    pub source: io::Error,
    /// Set only when `ReplaceFile` failed after taking the original away and putting it back also
    /// failed: the original bytes are in this file.
    pub original_copy: Option<PathBuf>,
}

impl PersistError {
    fn at(stage: Stage, source: io::Error) -> Self {
        Self { stage, source, original_copy: None }
    }

    /// The error as an `io::Error` of the same kind. The text is the underlying error's, so the
    /// messages users already see do not change; a lost original adds where it is.
    pub fn into_io(self) -> io::Error {
        match self.original_copy {
            None => self.source,
            Some(copy) => io::Error::new(self.source.kind(), format!("{} (the original is in {})", self.source, copy.display())),
        }
    }
}

impl std::fmt::Display for PersistError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} while {}", self.source, self.stage.label())?;
        match &self.original_copy {
            Some(copy) => write!(f, " (the original is in {})", copy.display()),
            None => Ok(()),
        }
    }
}

impl std::error::Error for PersistError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// A save that replaced the target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Committed {
    /// Whether the folder was flushed after the replacement (Unix only). `false` means the new name
    /// may not survive a power loss on this volume, though the bytes were flushed.
    pub directory_flushed: bool,
}

/// What was at the target when the save began.
pub enum Existing {
    /// Nothing: the file is new.
    Absent,
    /// A regular file, with the metadata read before anything was written.
    File(Metadata),
    /// A symbolic link; the link itself is replaced.
    Link,
}

/// What [`Steps::replace`] is asked to do.
pub struct Replacement<'a> {
    pub staging: &'a Path,
    pub target: &'a Path,
    pub existing: &'a Existing,
    /// Where `ReplaceFile` may leave the original while it works.
    pub backup: &'a Path,
}

/// The operations a save performs, so a test can make any one of them fail. [`Steps::real`] is the only
/// set the product uses.
pub struct Steps<'a> {
    pub write: &'a dyn Fn(&mut File, &[u8]) -> io::Result<()>,
    pub flush: &'a dyn Fn(&File) -> io::Result<()>,
    pub replace: &'a dyn Fn(&Replacement) -> io::Result<()>,
}

impl Steps<'static> {
    pub fn real() -> Self {
        Steps { write: &real_write, flush: &real_flush, replace: &real_replace }
    }
}

fn real_write(file: &mut File, bytes: &[u8]) -> io::Result<()> {
    file.write_all(bytes)
}

/// Flush `file` to the disk. On macOS `sync_all` asks for a full flush (`F_FULLFSYNC`), which SMB and FUSE
/// volumes, such as a mounted Cryptomator vault, refuse as unsupported: there a plain `fsync` is the
/// strongest flush the volume offers. Any other failure fails the save.
pub fn real_flush(file: &File) -> io::Result<()> {
    flush_with(file, File::sync_all)
}

/// [`real_flush`] with the full flush given, so the macOS fallback can be exercised everywhere.
pub fn flush_with(file: &File, sync_all: impl FnOnce(&File) -> io::Result<()>) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        use rustix::io::Errno;
        match sync_all(file) {
            Err(e) if matches!(Errno::from_io_error(&e), Some(Errno::NOTSUP | Errno::NOTTY)) => Ok(rustix::fs::fsync(file)?),
            other => other,
        }
    }
    #[cfg(not(target_os = "macos"))]
    sync_all(file)
}

#[cfg(windows)]
fn real_replace(r: &Replacement) -> io::Result<()> {
    match r.existing {
        // Nothing to keep the permissions of, and nothing to lose: a rename.
        Existing::Absent | Existing::Link => std::fs::rename(r.staging, r.target),
        Existing::File(_) => {
            let (Some(target), Some(staging), Some(backup)) = (r.target.to_str(), r.staging.to_str(), r.backup.to_str()) else {
                // The Windows API takes text; a name that isn't valid Unicode can't be passed on, and a rename
                // would silently drop the file's permissions, so the save is refused instead.
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "the file name has characters that can't be saved without changing its permissions",
                ));
            };
            winsafe::ReplaceFile(target, staging, Some(backup), winsafe::co::REPLACEFILE::default())
                .map_err(|e| io::Error::from_raw_os_error(e.raw() as i32))
        }
    }
}

#[cfg(not(windows))]
fn real_replace(r: &Replacement) -> io::Result<()> {
    std::fs::rename(r.staging, r.target)
}

/// Save `bytes` over `path` (or create it), with the real operations. `form` is the staging-name shape
/// the caller has always used.
pub fn write_replace(path: &Path, bytes: &[u8], form: StagingName) -> Result<Committed, PersistError> {
    write_replace_with(path, bytes, form, staging_suffixes(), &Steps::real())
}

/// [`write_replace`], trying the staging names `suffixes` give and performing the operations `steps`
/// give.
pub fn write_replace_with(
    path: &Path,
    bytes: &[u8],
    form: StagingName,
    suffixes: impl IntoIterator<Item = u64>,
    steps: &Steps,
) -> Result<Committed, PersistError> {
    let existing = inspect(path).map_err(|e| PersistError::at(Stage::Inspect, e))?;
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("save");
    let (staging, file) = create_staging(dir, name, form, suffixes).map_err(|e| PersistError::at(Stage::Stage, e))?;
    let cleanup = |e: PersistError| {
        let _ = std::fs::remove_file(&staging);
        e
    };
    // The handle is closed when this ends, success or failure, before the file is removed or replaced
    // (Windows will not delete or replace a file that is still open without sharing).
    let staged = (|| {
        let mut file = file;
        (steps.write)(&mut file, bytes).map_err(|e| PersistError::at(Stage::Write, e))?;
        (steps.flush)(&file).map_err(|e| PersistError::at(Stage::Flush, e))
    })();
    staged.map_err(cleanup)?;
    keep_permissions(&staging, &existing, dir).map_err(|e| cleanup(PersistError::at(Stage::Replace, e)))?;
    let backup = backup_path(&staging);
    let replacement = Replacement { staging: &staging, target: path, existing: &existing, backup: &backup };
    match (steps.replace)(&replacement) {
        Ok(()) => {
            let _ = std::fs::remove_file(&backup);
            Ok(Committed { directory_flushed: flush_directory(dir) })
        }
        Err(source) => {
            let original_copy = restore_original(path, &backup);
            let _ = std::fs::remove_file(&staging);
            Err(PersistError { stage: Stage::Replace, source, original_copy })
        }
    }
}

/// Save `bytes` to the file a command line names (`--out`), the way an editor would: the same service as
/// [`write_replace`], except that a path that is not an ordinary file is not replaced.
///
/// * A symbolic link is followed, so the file it points at is replaced and the link stays (a command
///   line user who points `--out` at a link means the file behind it).
/// * Something that is neither a file nor a folder, such as `/dev/stdout` or a pipe, has nothing to
///   replace; it is written in place.
pub fn write_output(path: &Path, bytes: &[u8], form: StagingName) -> io::Result<()> {
    let target = match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()),
        _ => path.to_path_buf(),
    };
    if let Ok(m) = std::fs::metadata(&target)
        && !m.is_file()
        && !m.is_dir()
    {
        return std::fs::write(&target, bytes);
    }
    write_replace(&target, bytes, form).map(|_| ()).map_err(PersistError::into_io)
}

/// Look at what is at `path`. A folder, or anything that is not a regular file or a link, is refused.
fn inspect(path: &Path) -> io::Result<Existing> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => Ok(Existing::Link),
        Ok(m) if m.is_dir() => Err(io::Error::new(io::ErrorKind::InvalidInput, "is a folder, not a file")),
        Ok(m) if m.is_file() => Ok(Existing::File(m)),
        Ok(_) => Err(io::Error::new(io::ErrorKind::InvalidInput, "is not a regular file")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Existing::Absent),
        Err(e) => Err(e),
    }
}

/// Give the staging file the permissions the saved file should have: an existing file's own, or for a new
/// one what an ordinary new file in that folder gets. Only Unix has modes to copy; on Windows the
/// replacement itself keeps an existing file's permissions.
#[cfg(unix)]
fn keep_permissions(staging: &Path, existing: &Existing, dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = match existing {
        Existing::File(m) => m.permissions().mode() & 0o7777,
        Existing::Link | Existing::Absent => default_new_file_mode(dir)?,
    };
    std::fs::set_permissions(staging, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn keep_permissions(_: &Path, _: &Existing, _: &Path) -> io::Result<()> {
    Ok(())
}

/// The mode an ordinary new file gets in `dir` (0666 less the process's umask), read from a probe file
/// that is created and removed at once; the umask can't be read without changing it.
#[cfg(unix)]
fn default_new_file_mode(dir: &Path) -> io::Result<u32> {
    use std::os::unix::fs::PermissionsExt;
    // An ordinary open, not the staging helper: that one is private (0600), which is the thing being avoided.
    let mut last = io::Error::new(io::ErrorKind::AlreadyExists, "every probe name tried is taken");
    for suffix in staging_suffixes().take(crate::staging::STAGING_ATTEMPTS) {
        let probe = dir.join(format!(".pdfcraft-mode-probe-{suffix:016x}.tmp"));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&probe) {
            Ok(file) => {
                drop(file);
                let mode = std::fs::metadata(&probe).map(|m| m.permissions().mode() & 0o7777);
                let _ = std::fs::remove_file(&probe);
                return mode;
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => last = e,
            Err(e) => return Err(e),
        }
    }
    Err(last)
}

/// The name `ReplaceFile` may park the original under: the staging file's own unpredictable name with
/// `.bak` added, so it is beside the target and nobody can have taken it in advance.
fn backup_path(staging: &Path) -> PathBuf {
    let mut name = staging.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".bak");
    staging.with_file_name(name)
}

/// After a failed replacement: if the target is gone and the backup is there, put it back. Returns the
/// backup's path only when the original is left in it because that failed too.
fn restore_original(target: &Path, backup: &Path) -> Option<PathBuf> {
    if target.symlink_metadata().is_ok() {
        // The target is still there: the original was never taken away (or the failure came before). A
        // leftover backup is a copy of it.
        let _ = std::fs::remove_file(backup);
        return None;
    }
    if backup.symlink_metadata().is_err() {
        return None;
    }
    match std::fs::rename(backup, target) {
        Ok(()) => None,
        Err(_) => Some(backup.to_path_buf()),
    }
}

/// Flush the folder that holds the new name (Unix). Windows has no such call in the standard library.
#[cfg(unix)]
fn flush_directory(dir: &Path) -> bool {
    File::open(dir).and_then(|d| d.sync_all()).is_ok()
}

#[cfg(not(unix))]
fn flush_directory(_: &Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn test_dir(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pdfcraft-persistence-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn entries(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    }

    fn save(path: &Path, bytes: &[u8]) -> Result<Committed, PersistError> {
        write_replace_with(path, bytes, StagingName::SuffixThenTag, [1, 2, 3], &Steps::real())
    }

    fn save_with(path: &Path, bytes: &[u8], steps: &Steps) -> Result<Committed, PersistError> {
        write_replace_with(path, bytes, StagingName::SuffixThenTag, [1, 2, 3], steps)
    }

    #[test]
    fn a_new_file_is_created_and_an_existing_one_replaced_leaving_nothing_behind() {
        let dir = test_dir("basic");
        let target = dir.join("out.pdf");
        save(&target, b"ONE").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"ONE");
        save(&target, b"TWO!").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"TWO!");
        assert_eq!(entries(&dir), ["out.pdf"], "no staging or backup file is left");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A write that stops part way (a full disk) must not touch the original.
    #[test]
    fn a_short_write_keeps_the_original_and_leaves_no_temporary_file() {
        let dir = test_dir("short");
        let target = dir.join("out.pdf");
        std::fs::write(&target, b"ORIGINAL").unwrap();
        let steps = Steps {
            write: &|f, bytes| {
                f.write_all(&bytes[..bytes.len() / 2])?;
                Err(io::Error::other("disk full"))
            },
            ..Steps::real()
        };
        let e = save_with(&target, b"NEW CONTENT", &steps).unwrap_err();
        assert_eq!(e.stage, Stage::Write);
        assert!(e.to_string().contains("disk full") && e.original_copy.is_none());
        assert_eq!(std::fs::read(&target).unwrap(), b"ORIGINAL");
        assert_eq!(entries(&dir), ["out.pdf"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F02: a failed flush must fail the save, not be skipped.
    #[test]
    fn a_failed_flush_keeps_the_original_and_fails_the_save() {
        let dir = test_dir("flush");
        let target = dir.join("out.pdf");
        std::fs::write(&target, b"ORIGINAL").unwrap();
        let steps = Steps { flush: &|_| Err(io::Error::other("disk gone")), ..Steps::real() };
        let e = save_with(&target, b"NEW", &steps).unwrap_err();
        assert_eq!(e.stage, Stage::Flush);
        assert_eq!(std::fs::read(&target).unwrap(), b"ORIGINAL");
        assert_eq!(entries(&dir), ["out.pdf"]);
        // The flush ran before the replacement: it saw the staged bytes and the original unchanged.
        let seen = Cell::new(false);
        let steps = Steps {
            flush: &|f| {
                seen.set(true);
                assert_eq!(f.metadata()?.len(), 3, "the bytes are written before the flush");
                assert_eq!(std::fs::read(&target)?, b"ORIGINAL", "the target is untouched until the flush has succeeded");
                Ok(())
            },
            ..Steps::real()
        };
        save_with(&target, b"NEW", &steps).unwrap();
        assert!(seen.get());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_replacement_keeps_the_original_and_removes_the_staging_file() {
        let dir = test_dir("replace");
        let target = dir.join("out.pdf");
        std::fs::write(&target, b"ORIGINAL").unwrap();
        let steps = Steps { replace: &|_| Err(io::Error::new(io::ErrorKind::PermissionDenied, "locked")), ..Steps::real() };
        let e = save_with(&target, b"NEW", &steps).unwrap_err();
        assert_eq!((e.stage, e.source.kind()), (Stage::Replace, io::ErrorKind::PermissionDenied));
        assert_eq!(e.into_io().kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(std::fs::read(&target).unwrap(), b"ORIGINAL");
        assert_eq!(entries(&dir), ["out.pdf"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_folder_or_a_taken_staging_name_is_refused_before_anything_is_written() {
        let dir = test_dir("refuse");
        let folder = dir.join("folder.pdf");
        std::fs::create_dir(&folder).unwrap();
        assert_eq!(save(&folder, b"X").unwrap_err().stage, Stage::Inspect);
        assert!(entries(&folder).is_empty());
        let target = dir.join("out.pdf");
        for s in 1..=3u64 {
            std::fs::write(dir.join(format!(".out.pdf.{s:016x}.pdfcraft-tmp")), "PLANTED").unwrap();
        }
        assert_eq!(save(&target, b"X").unwrap_err().stage, Stage::Stage);
        assert!(!target.exists());
        let planted = dir.join(".out.pdf.0000000000000001.pdfcraft-tmp");
        assert_eq!(std::fs::read(planted).unwrap(), b"PLANTED", "a planted file is never written through");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The target is put back if a replacement that fails has taken it away.
    #[test]
    fn restoring_puts_the_original_back_only_when_it_is_gone() {
        let dir = test_dir("restore");
        let (target, backup) = (dir.join("t.pdf"), dir.join("t.bak"));
        // Gone and parked in the backup: restored.
        std::fs::write(&backup, b"ORIGINAL").unwrap();
        assert_eq!(restore_original(&target, &backup), None);
        assert_eq!(std::fs::read(&target).unwrap(), b"ORIGINAL");
        assert!(!backup.exists());
        // Still there: left alone, and a leftover backup is dropped.
        std::fs::write(&backup, b"STALE").unwrap();
        assert_eq!(restore_original(&target, &backup), None);
        assert_eq!(std::fs::read(&target).unwrap(), b"ORIGINAL");
        assert!(!backup.exists());
        // Gone and no backup: nothing to do, and nothing is invented.
        std::fs::remove_file(&target).unwrap();
        assert_eq!(restore_original(&target, &backup), None);
        assert!(!target.exists());
        // Gone, a backup, and it can't be moved back: the error names the file that holds the original.
        std::fs::write(&backup, b"ORIGINAL").unwrap();
        let blocked = dir.join("no-such-folder").join("t.pdf");
        assert_eq!(restore_original(&blocked, &backup), Some(backup.clone()));
        assert_eq!(std::fs::read(&backup).unwrap(), b"ORIGINAL");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_command_line_output_is_saved_through_the_service_and_follows_a_link() {
        let dir = test_dir("output");
        let out = dir.join("out.pdf");
        write_output(&out, b"ONE", StagingName::SuffixThenTag).unwrap();
        write_output(&out, b"TWO", StagingName::SuffixThenTag).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"TWO");
        assert_eq!(entries(&dir), ["out.pdf"]);
        // A folder is refused, and says so, as before.
        assert!(write_output(&dir, b"X", StagingName::SuffixThenTag).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_command_line_output_follows_a_link_and_writes_a_device_in_place() {
        let dir = test_dir("output-link");
        let real = dir.join("real.pdf");
        std::fs::write(&real, b"OLD").unwrap();
        let link = dir.join("latest.pdf");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        write_output(&link, b"NEW", StagingName::SuffixThenTag).unwrap();
        assert_eq!(std::fs::read(&real).unwrap(), b"NEW", "the file behind the link is replaced");
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink(), "and the link stays");
        write_output(Path::new("/dev/null"), b"discarded", StagingName::SuffixThenTag).unwrap();
        assert_eq!(entries(&dir), ["latest.pdf", "real.pdf"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_flush_helper_passes_a_failure_through() {
        let dir = test_dir("flush-helper");
        let f = File::create(dir.join("f")).unwrap();
        assert!(flush_with(&f, |_| Ok(())).is_ok());
        assert_eq!(flush_with(&f, |_| Err(io::Error::other("nope"))).unwrap_err().to_string(), "nope");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    mod windows {
        use super::*;
        use std::os::windows::fs::OpenOptionsExt;
        use std::process::Command;

        /// What `icacls` reports for `path`, without the path itself.
        fn acl(path: &Path) -> Option<String> {
            let out = Command::new("icacls").arg(path).output().ok()?;
            if !out.status.success() {
                return None;
            }
            let text = String::from_utf8_lossy(&out.stdout).into_owned();
            let prefix = path.to_string_lossy().into_owned();
            Some(text.replace(&prefix, "").lines().filter(|l| !l.starts_with("Successfully")).map(str::trim).collect::<Vec<_>>().join("|"))
        }

        /// Give the file an access list of its own: inheritance removed, the current user, and read access for the
        /// built-in Users group as an explicit entry. That entry is the point: a folder's inherited list shows it as
        /// inherited, so a file that was renamed into place can never have it as an explicit one, whatever the
        /// folder (a hosted runner's temp folder grants the same accounts as a plain restriction would).
        fn restrict(path: &Path) -> bool {
            let user = format!("{}:(F)", std::env::var("USERNAME").unwrap_or_default());
            let run = |args: &[&str]| Command::new("icacls").arg(path).args(args).output().map(|o| o.status.success()).unwrap_or(false);
            run(&["/inheritance:r", "/grant:r", &user]) && run(&["/grant", "*S-1-5-32-545:(R)"])
        }

        /// BUILD-004: a rename gives the saved file the folder's inherited access list; the replacement must
        /// not. The second half is the baseline: the rename the old code used does change it.
        #[test]
        fn a_restricted_targets_access_list_survives_the_save() {
            let dir = test_dir("acl");
            let target = dir.join("secret.pdf");
            std::fs::write(&target, b"OLD").unwrap();
            assert!(restrict(&target), "icacls must be available to run this test");
            let before = acl(&target).unwrap();
            save(&target, b"NEW").unwrap();
            assert_eq!(std::fs::read(&target).unwrap(), b"NEW");
            assert_eq!(acl(&target).unwrap(), before, "the restricted access list must be kept");
            // The old behaviour, for contrast: renaming over the target loses it. If the folder's inherited list
            // happened to equal the restricted one this would be vacuous, so it fails loudly instead.
            let renaming = Steps { replace: &|r| std::fs::rename(r.staging, r.target), ..Steps::real() };
            save_with(&target, b"NEWER", &renaming).unwrap();
            assert_ne!(acl(&target).unwrap(), before, "a plain rename was expected to change the access list");
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn a_read_only_target_is_refused_and_keeps_its_bytes() {
            let dir = test_dir("readonly");
            let target = dir.join("ro.pdf");
            std::fs::write(&target, b"ORIGINAL").unwrap();
            let mut perm = std::fs::metadata(&target).unwrap().permissions();
            perm.set_readonly(true);
            std::fs::set_permissions(&target, perm).unwrap();
            let e = save(&target, b"NEW").unwrap_err();
            assert_eq!(e.stage, Stage::Replace);
            assert_eq!(std::fs::read(&target).unwrap(), b"ORIGINAL");
            assert_eq!(entries(&dir), ["ro.pdf"], "no staging or backup file is left");
            let mut perm = std::fs::metadata(&target).unwrap().permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            perm.set_readonly(false);
            let _ = std::fs::set_permissions(&target, perm);
            let _ = std::fs::remove_dir_all(&dir);
        }

        /// Another process has the document open without sharing: the save fails and the file is unchanged,
        /// and succeeds once it lets go.
        #[test]
        fn a_target_another_program_holds_open_is_kept_intact() {
            let dir = test_dir("locked");
            let target = dir.join("held.pdf");
            std::fs::write(&target, b"ORIGINAL").unwrap();
            let held = std::fs::OpenOptions::new().read(true).share_mode(0).open(&target).unwrap();
            let e = save(&target, b"NEW").unwrap_err();
            assert_eq!(e.stage, Stage::Replace);
            drop(held);
            assert_eq!(std::fs::read(&target).unwrap(), b"ORIGINAL");
            assert_eq!(entries(&dir), ["held.pdf"]);
            save(&target, b"NEW").unwrap();
            assert_eq!(std::fs::read(&target).unwrap(), b"NEW");
            let _ = std::fs::remove_dir_all(&dir);
        }

        /// The staging file cannot be read by anyone while the bytes are being written.
        #[test]
        fn the_staging_file_is_not_shared_while_it_is_written() {
            let dir = test_dir("exclusive");
            let target = dir.join("out.pdf");
            let steps = Steps {
                write: &|f, bytes| {
                    f.write_all(bytes)?;
                    let staging = std::fs::read_dir(&dir)?.filter_map(Result::ok).map(|e| e.path()).find(|p| p != &target);
                    let staging = staging.ok_or_else(|| io::Error::other("no staging file"))?;
                    // Another opener is refused for as long as the writing handle is open.
                    match File::open(&staging) {
                        Err(e) if e.raw_os_error() == Some(32) => Ok(()),
                        other => Err(io::Error::other(format!("the staging file was readable by another handle: {other:?}"))),
                    }
                },
                ..Steps::real()
            };
            save_with(&target, b"SECRET", &steps).unwrap();
            assert_eq!(std::fs::read(&target).unwrap(), b"SECRET");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[cfg(unix)]
    mod unix {
        use super::*;
        use std::os::unix::fs::PermissionsExt;

        fn mode(p: &Path) -> u32 {
            std::fs::metadata(p).unwrap().permissions().mode() & 0o7777
        }

        #[test]
        fn the_staging_file_is_private_while_it_holds_the_document() {
            let dir = test_dir("private");
            let target = dir.join("out.pdf");
            let seen = Cell::new(0u32);
            let steps = Steps {
                flush: &|f| {
                    seen.set(f.metadata()?.permissions().mode() & 0o7777);
                    Ok(())
                },
                ..Steps::real()
            };
            save_with(&target, b"SECRET", &steps).unwrap();
            assert_eq!(seen.get(), 0o600, "the staging file is private before it is flushed");
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn an_existing_files_mode_is_kept_and_a_new_file_gets_the_ordinary_one() {
            let dir = test_dir("mode");
            let target = dir.join("out.pdf");
            std::fs::write(&target, b"OLD").unwrap();
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o640)).unwrap();
            save(&target, b"NEW").unwrap();
            assert_eq!(mode(&target), 0o640);
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o400)).unwrap();
            save(&target, b"NEWER").unwrap();
            assert_eq!(mode(&target), 0o400);
            // A new file gets what `File::create` would give in this folder, not the staging file's 0600.
            let fresh = dir.join("fresh.pdf");
            save(&fresh, b"X").unwrap();
            let ordinary = dir.join("ordinary");
            std::fs::write(&ordinary, b"").unwrap();
            assert_eq!(mode(&fresh), mode(&ordinary));
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn the_folder_is_flushed_after_the_replacement() {
            let dir = test_dir("dirsync");
            let committed = save(&dir.join("out.pdf"), b"X").unwrap();
            assert!(committed.directory_flushed);
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn a_link_is_replaced_not_followed() {
            let dir = test_dir("link");
            let outside = dir.join("outside.pdf");
            std::fs::write(&outside, b"OUTSIDE").unwrap();
            let link = dir.join("link.pdf");
            std::os::unix::fs::symlink(&outside, &link).unwrap();
            save(&link, b"NEW").unwrap();
            assert_eq!(std::fs::read(&outside).unwrap(), b"OUTSIDE", "the file the link pointed at is untouched");
            assert_eq!(std::fs::read(&link).unwrap(), b"NEW");
            assert!(!std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
