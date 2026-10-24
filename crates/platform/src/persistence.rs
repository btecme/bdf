//! The one way the desktop app and the automation tools put bytes into a file the user keeps
//! (BUILD-004, finding F02).
//!
//! A save is: look at the target, write the bytes to a new private file beside it, flush that file to
//! the disk, then replace the target with it. Every step can fail, and a failure at any step before the
//! replacement leaves the target exactly as it was; the failure says which step it was.
//!
//! * **Exclusive, private staging.** The file is created new, with an unpredictable name, so a name planted in
//!   advance is skipped rather than written through ([`crate::staging`]). It is private from before the first
//!   byte is written to after it has replaced the target: `0600` on Unix; on Windows it is given the target's
//!   own access list before any byte goes in (see below), and is opened with no sharing while it is written.
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
//! * **Windows staging takes the target's access list first.** No safe API sets an access list, but
//!   `ReplaceFile` gives the file it moves into place the access list of the file it replaces, and it will do
//!   that for a hard link of the target. So an empty staging file replaces a hard link of the target, which
//!   hands it the target's access list while it is still empty, and only then are the document's bytes
//!   written into it. At every later point (while written, flushed, closed, after a failure, after a crash
//!   that leaves the file behind) the staged bytes are exactly as readable as the target. Where that
//!   cannot be set up (no hard links) the save is refused before any byte is written, unless the volume has no
//!   access lists at all (FAT, exFAT), where there is nothing to keep private.
//!
//! What this does not do: it does not follow a symbolic link (the link itself is replaced, so a save
//! can't be redirected out of the folder it was aimed at), and it does not preserve a file's owner. A new file
//! has no target to take an access list from; it gets the folder's inherited one, as the saved file will.

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
    /// The caller's check that the target is still what the save may replace (see [`crate::file_identity`]).
    Guard,
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
            Stage::Guard => "checking the file is still the one being replaced",
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
#[derive(Clone, Copy)]
pub struct Steps<'a> {
    pub write: &'a dyn Fn(&mut File, &[u8]) -> io::Result<()>,
    pub flush: &'a dyn Fn(&File) -> io::Result<()>,
    pub replace: &'a dyn Fn(&Replacement) -> io::Result<()>,
    /// Windows: make a hard link of the target, the first step of giving the staging file the target's access list.
    pub link: &'a dyn Fn(&Path, &Path) -> io::Result<()>,
    /// Windows: move the empty staging file over that hard link, which is what hands it the access list.
    pub adopt_acl: &'a dyn Fn(&Replacement) -> io::Result<()>,
    /// Windows: whether the target's volume keeps access lists at all.
    pub has_acls: &'a dyn Fn(&Path) -> io::Result<bool>,
    /// Called with the target's path after the new bytes are staged, flushed and closed, immediately before they
    /// replace it. An error refuses the save: the staging file is removed and the target is untouched. This is where
    /// [`crate::file_identity`] checks that the file is still the version the save may replace.
    pub guard: &'a dyn Fn(&Path) -> io::Result<()>,
}

impl Steps<'static> {
    pub fn real() -> Self {
        Steps {
            write: &real_write,
            flush: &real_flush,
            replace: &real_replace,
            link: &real_link,
            adopt_acl: &real_replace,
            has_acls: &volume_has_acls,
            guard: &|_| Ok(()),
        }
    }
}

fn real_link(target: &Path, link: &Path) -> io::Result<()> {
    std::fs::hard_link(target, link)
}

/// Whether the volume `path` is on keeps access lists (NTFS and ReFS do; FAT and exFAT do not).
#[cfg(windows)]
fn volume_has_acls(path: &Path) -> io::Result<bool> {
    let os = |e: winsafe::co::ERROR| io::Error::from_raw_os_error(e.raw() as i32);
    let text = path.to_str().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the file name is not valid Unicode"))?;
    let root = winsafe::GetVolumePathName(text).map_err(os)?;
    let mut flags = winsafe::co::FILE_VOL::default();
    winsafe::GetVolumeInformation(Some(&root), None, None, None, Some(&mut flags), None).map_err(os)?;
    Ok(flags.has(winsafe::co::FILE_VOL::PERSISTENT_ACLS))
}

#[cfg(not(windows))]
fn volume_has_acls(_: &Path) -> io::Result<bool> {
    Ok(true)
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
        Existing::Link => std::fs::rename(r.staging, r.target),
        Existing::Absent => create_without_replacing(r),
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
    match r.existing {
        Existing::Absent => create_without_replacing(r),
        _ => std::fs::rename(r.staging, r.target),
    }
}

/// Put the staged file at a name nothing had when the save began, without replacing anything that has appeared
/// there since: a hard link fails if the name is taken (a rename would silently replace it), and the staging name is
/// dropped afterwards. A filesystem without hard links (FAT, exFAT) falls back to the rename, whose window is the
/// instant between the check and the call.
fn create_without_replacing(r: &Replacement) -> io::Result<()> {
    match std::fs::hard_link(r.staging, r.target) {
        Ok(()) => {
            // The file is in place under its final name; dropping the staging name is housekeeping.
            let _ = std::fs::remove_file(r.staging);
            Ok(())
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(e),
        Err(_) => std::fs::rename(r.staging, r.target),
    }
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
    let (staging, file) = make_staging(dir, name, form, suffixes, path, &existing, steps)?;
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
    // The guard comes first: it is where a confined save notices that its folder was swapped for a link (after which the
    // staging path may lead somewhere else), and where the version being replaced is checked, as late as the work
    // before the replacement allows.
    (steps.guard)(path).map_err(|e| cleanup(PersistError::at(Stage::Guard, e)))?;
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

/// Create the staging file for saving over `target`, private from the start (see the module doc).
fn make_staging(
    dir: &Path,
    name: &str,
    form: StagingName,
    suffixes: impl IntoIterator<Item = u64>,
    target: &Path,
    existing: &Existing,
    steps: &Steps,
) -> Result<(PathBuf, File), PersistError> {
    let plain = create_staging(dir, name, form, suffixes).map_err(|e| PersistError::at(Stage::Stage, e))?;
    #[cfg(windows)]
    if matches!(existing, Existing::File(_)) {
        return private_staging(plain, target, existing, steps);
    }
    let _ = (target, existing, steps);
    Ok(plain)
}

/// Windows: turn the empty staging file `plain` into one that has `target`'s access list, before any byte is written.
///
/// 1. Hard-link the target under a new name (the link and the target are one file with one access list).
/// 2. Replace that link with the empty staging file. `ReplaceFile` gives the file it moves into place the access list
///    of the file it replaces, so the staging file now carries the target's, and the target is untouched.
/// 3. Open the result for writing.
///
/// Every failure removes the names made here (never the target) and refuses the save before any byte has been
/// written. The one exception is a volume that cannot make hard links *and* keeps no access lists: there is nothing
/// to keep private, so the plain staging file is used.
#[cfg(windows)]
fn private_staging(plain: (PathBuf, File), target: &Path, existing: &Existing, steps: &Steps) -> Result<(PathBuf, File), PersistError> {
    use std::os::windows::fs::OpenOptionsExt;
    // Opens the reparse point itself, so a link swapped in for the name is not followed.
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    let (empty, handle) = plain;
    let tagged = |suffix: &str| {
        let mut name = empty.file_name().map(|n| n.to_os_string()).unwrap_or_default();
        name.push(suffix);
        empty.with_file_name(name)
    };
    let (link, backup) = (tagged(".acl"), tagged(".acl.bak"));
    let remove_all = |names: &[&Path]| names.iter().for_each(|n| drop(std::fs::remove_file(n)));
    // Keeps the kind of the underlying error (a read-only or locked target is still "permission denied").
    let refuse = |kind: io::ErrorKind, why: String| PersistError::at(Stage::Stage, io::Error::new(kind, why));

    if let Err(e) = (steps.link)(target, &link) {
        return match (steps.has_acls)(target) {
            // No access lists on this volume: the saved file has none either, so the staged bytes are as
            // protected as they can be.
            Ok(false) => Ok((empty, handle)),
            Ok(true) | Err(_) => {
                drop(handle);
                remove_all(&[&empty]);
                Err(refuse(
                    e.kind(),
                    format!("{}: a temporary copy can't be kept as private as this file on this volume ({e}); nothing was written", target.display()),
                ))
            }
        };
    }
    // `ReplaceFile` needs the empty file closed.
    drop(handle);
    let adopt = Replacement { staging: &empty, target: &link, existing, backup: &backup };
    if let Err(e) = (steps.adopt_acl)(&adopt) {
        remove_all(&[&empty, &link, &backup]);
        return Err(refuse(
            e.kind(),
            format!("{}: couldn't give the temporary copy the file's access list ({e}); nothing was written", target.display()),
        ));
    }
    // The old name of the link is now a second link to the target: drop it.
    remove_all(&[&backup]);
    let opened = std::fs::OpenOptions::new()
        .write(true)
        .share_mode(0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&link)
        .and_then(|f| if f.metadata()?.is_file() { Ok(f) } else { Err(io::Error::other("the temporary file is not a regular file")) });
    match opened {
        Ok(file) => Ok((link, file)),
        Err(e) => {
            remove_all(&[&link]);
            Err(refuse(e.kind(), format!("{}: couldn't open the temporary copy ({e}); nothing was written", target.display())))
        }
    }
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
        /// not. (The staging file now carries the target's list from the start, so even the rename would keep it;
        /// the staging tests below are what show a plain staging file would not.)
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
            let _ = std::fs::remove_dir_all(&dir);
        }

        /// The one other file in the folder besides `target`: the staging file, whatever it is called.
        fn staging_beside(dir: &Path, target: &Path) -> PathBuf {
            let others: Vec<PathBuf> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p != target).collect();
            assert_eq!(others.len(), 1, "expected exactly the staging file beside the target: {others:?}");
            others.into_iter().next().unwrap()
        }

        /// R1 (Codex review): the staged bytes must be no more readable than the target at every point they exist. The
        /// folder here grants more (inherited entries) than the target's own list, so a staging file that merely
        /// inherited the folder's list would be readable by accounts the target refuses. The observer is the access
        /// list itself, read at each boundary: while the bytes are written, at the flush, with the handle closed just
        /// before the replacement, and at the failure (where a crash would leave the file behind).
        #[test]
        fn staged_bytes_are_never_more_readable_than_the_target_at_any_stage() {
            let dir = test_dir("private-staging");
            let target = dir.join("secret.pdf");
            std::fs::write(&target, b"OLD").unwrap();
            assert!(restrict(&target), "icacls must be available to run this test");
            let target_acl = acl(&target).unwrap();
            let folder_acl = acl(&dir).unwrap();
            assert!(folder_acl.contains("(I)") || !folder_acl.is_empty(), "the folder has an inherited list for the file to wrongly take");
            let seen = Cell::new(0u32);
            let check = |when: &str| {
                seen.set(seen.get() + 1);
                let staging = staging_beside(&dir, &target);
                assert_eq!(acl(&staging).unwrap(), target_acl, "staged bytes are exposed {when}: {staging:?}");
            };
            let steps = Steps {
                write: &|f, bytes| {
                    check("before the first byte is written");
                    f.write_all(bytes)?;
                    check("after the bytes were written");
                    Ok(())
                },
                flush: &|f| {
                    f.sync_all()?;
                    check("at the flush");
                    Ok(())
                },
                // The handle is closed here: this is where a crash would have left the file readable.
                replace: &|r| {
                    check("with the handle closed, before the replacement");
                    assert_eq!(r.target, target);
                    Err(io::Error::other("injected: the process dies here"))
                },
                ..Steps::real()
            };
            let e = save_with(&target, b"SYNTHETIC SECRET NEW BYTES", &steps).unwrap_err();
            assert_eq!(e.stage, Stage::Replace);
            assert_eq!(seen.get(), 4, "every boundary was observed");
            assert_eq!(std::fs::read(&target).unwrap(), b"OLD");
            assert_eq!(acl(&target).unwrap(), target_acl);
            assert_eq!(entries(&dir), ["secret.pdf"], "the failure leaves no staging, link or backup name");
            // And the whole save, with nothing injected, ends with the new bytes under the same list.
            save(&target, b"SYNTHETIC SECRET NEW BYTES").unwrap();
            assert_eq!(std::fs::read(&target).unwrap(), b"SYNTHETIC SECRET NEW BYTES");
            assert_eq!(acl(&target).unwrap(), target_acl);
            assert_eq!(entries(&dir), ["secret.pdf"]);
            let _ = std::fs::remove_dir_all(&dir);
        }

        /// The same property, shown to be real: a staging file made the old way (plain, inheriting the folder's list)
        /// does differ from the target's, so the test above cannot pass by accident.
        #[test]
        fn a_plainly_created_staging_file_would_have_been_readable_by_more() {
            let dir = test_dir("plain-staging");
            let target = dir.join("secret.pdf");
            std::fs::write(&target, b"OLD").unwrap();
            assert!(restrict(&target));
            let (plain, _handle) = create_staging(&dir, "secret.pdf", StagingName::SuffixThenTag, [1]).unwrap();
            assert_ne!(acl(&plain).unwrap(), acl(&target).unwrap(), "a plain staging file takes the folder's list, not the target's");
            drop(_handle);
            let _ = std::fs::remove_dir_all(&dir);
        }

        /// Where the guarantee cannot be set up the save is refused before a byte is written, and nothing is left.
        #[test]
        fn a_save_that_cannot_keep_its_staging_private_is_refused_before_writing() {
            let dir = test_dir("refuse-private");
            let target = dir.join("secret.pdf");
            std::fs::write(&target, b"OLD").unwrap();
            let written = Cell::new(false);
            let no_link = |_: &Path, _: &Path| Err(io::Error::other("no hard links here"));
            let no_write = |_: &mut File, _: &[u8]| {
                written.set(true);
                Ok(())
            };
            // No hard links on a volume that keeps access lists, and a volume that cannot even say: both refuse.
            for has_acls in [&(|_: &Path| Ok(true)) as &dyn Fn(&Path) -> io::Result<bool>, &|_: &Path| Err(io::Error::other("unknown volume"))] {
                let steps = Steps { link: &no_link, has_acls, write: &no_write, ..Steps::real() };
                let e = save_with(&target, b"SECRET", &steps).unwrap_err();
                assert_eq!(e.stage, Stage::Stage);
                assert!(e.to_string().contains("nothing was written"), "{e}");
                assert!(!written.get(), "no byte was written");
                assert_eq!(std::fs::read(&target).unwrap(), b"OLD");
                assert_eq!(entries(&dir), ["secret.pdf"]);
            }
            // Handing over the access list fails part way: refused the same way, and every name made is removed.
            let steps = Steps { adopt_acl: &|_| Err(io::Error::other("injected")), write: &no_write, ..Steps::real() };
            let e = save_with(&target, b"SECRET", &steps).unwrap_err();
            assert_eq!(e.stage, Stage::Stage);
            assert!(!written.get());
            assert_eq!(std::fs::read(&target).unwrap(), b"OLD");
            assert_eq!(entries(&dir), ["secret.pdf"]);
            let _ = std::fs::remove_dir_all(&dir);
        }

        /// FAT and exFAT keep no access lists and cannot make hard links: there is nothing to keep private, so a save
        /// there goes ahead (a refusal would stop saving to a USB stick).
        #[test]
        fn a_volume_without_access_lists_still_saves() {
            let dir = test_dir("no-acls");
            let target = dir.join("stick.pdf");
            std::fs::write(&target, b"OLD").unwrap();
            let steps = Steps { link: &|_, _| Err(io::Error::other("no hard links")), has_acls: &|_| Ok(false), ..Steps::real() };
            save_with(&target, b"NEW", &steps).unwrap();
            assert_eq!(std::fs::read(&target).unwrap(), b"NEW");
            assert_eq!(entries(&dir), ["stick.pdf"]);
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn the_real_volume_check_reports_access_lists_on_the_test_volume() {
            let dir = test_dir("volume");
            assert!(volume_has_acls(&dir).unwrap(), "the temp folder is on an NTFS or ReFS volume");
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
            // `ReplaceFile` refuses a read-only file at the first step that uses it, before any byte is written.
            assert_eq!(e.stage, Stage::Stage, "{e}");
            assert_eq!(std::fs::read(&target).unwrap(), b"ORIGINAL");
            assert_eq!(entries(&dir), ["ro.pdf"], "no staging, link or backup file is left");
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
