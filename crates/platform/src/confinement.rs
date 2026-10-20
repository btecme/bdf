//! Keeping a confined write inside its root while it happens (BUILD-006, finding F06).
//!
//! `--root` confines what the automation tools read and write to one folder. The tools resolve a path
//! (canonicalise it, check it is inside the root), and later open it. Between the two a different program with
//! write access to that tree can swap a folder for a link (a symbolic link, or on Windows a junction, which needs no
//! privilege) and the open lands outside the root. Canonicalising first does not prevent that: it narrows the window
//! and does not close it.
//!
//! What this module does, and what it does not:
//!
//! * **Windows: the destination folder is pinned.** The folder a write goes into is opened, and held open for the
//!   whole save, without sharing for delete. While that handle is open Windows refuses to rename or delete that
//!   folder or any folder above it, so none of the folders between the root and the destination can be replaced by a
//!   link. The folders the save has to create are created one at a time, each checked as it is made. This is what
//!   closes the swap for a folder below the root, on the platform BDF ships for.
//! * **Every platform: revalidation at the commit.** Immediately before the new bytes replace the target, the folder is
//!   canonicalised again and must still be the folder that was pinned, with no link between it and the root
//!   ([`Pinned::revalidate`]). On Unix, where an open folder cannot be pinned this way, this is all there is: a swap in
//!   the instant between that check and the rename is not seen. **On Unix `--root` is therefore not a boundary
//!   against a program that can modify the tree**; it is a guard against mistakes and against ordinary traversal.
//! * **Not covered:** the root itself and the folders above it (the root is chosen by the user and trusted); files
//!   (as opposed to folders) swapped for links, which the persistence service already replaces rather than follows; and
//!   reads other than document opens.
//!
//! Handle-relative I/O (`openat`-style, so no path is ever resolved twice) would close the Unix gap too. No safe
//! wrapper for it is in the approved dependency set; see `docs/bdf-evidence/BUILD-006.md`.

#[cfg(windows)]
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use crate::file_identity::{Authority, check};
use crate::persistence::{Committed, PersistError, Steps, write_replace_with};
use crate::staging::{StagingName, staging_suffixes};

/// A folder inside the root, verified link-free and (on Windows) held so that it and its ancestors cannot be replaced.
pub struct Pinned {
    dir: PathBuf,
    #[cfg(windows)]
    _handle: File,
}

impl Pinned {
    /// The folder.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Check, now, that the folder is still inside `root` with no link between: the commit-time guard.
    pub fn revalidate(&self, root: &Path) -> io::Result<()> {
        check_chain(root, &self.dir)?;
        let real = std::fs::canonicalize(&self.dir)?;
        if real != self.dir {
            return Err(refused(&self.dir, "the folder is no longer where it was checked"));
        }
        Ok(())
    }
}

fn refused(path: &Path, why: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, format!("{}: {why}; refused", path.display()))
}

/// Every folder from `root` (exclusive) down to `dir` that exists must be a real folder: not a link of any kind.
fn check_chain(root: &Path, dir: &Path) -> io::Result<()> {
    let relative = dir.strip_prefix(root).map_err(|_| refused(dir, "outside the allowed folder"))?;
    let mut at = root.to_path_buf();
    for part in relative.components() {
        at.push(part);
        match std::fs::symlink_metadata(&at) {
            Ok(m) if m.file_type().is_symlink() => return Err(refused(&at, "a link inside the allowed folder")),
            Ok(m) if !m.is_dir() => return Err(refused(&at, "not a folder")),
            Ok(_) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Make sure `dir` (below `root`, both canonical) exists, creating the missing folders one at a time with each checked
/// as it is made, then pin it. Fails if a folder on the way is a link, or if `dir` is not inside `root`.
pub fn pin_creating(root: &Path, dir: &Path) -> io::Result<Pinned> {
    pin_inside(root, dir, true)
}

/// [`pin_creating`] for a read: the folder must already exist, and nothing is created.
pub fn pin_existing(root: &Path, dir: &Path) -> io::Result<Pinned> {
    pin_inside(root, dir, false)
}

fn pin_inside(root: &Path, dir: &Path, create: bool) -> io::Result<Pinned> {
    let relative = dir.strip_prefix(root).map_err(|_| refused(dir, "outside the allowed folder"))?;
    let mut at = root.to_path_buf();
    for part in relative.components() {
        at.push(part);
        match std::fs::symlink_metadata(&at) {
            Ok(m) if m.file_type().is_symlink() => return Err(refused(&at, "a link inside the allowed folder")),
            Ok(m) if m.is_dir() => {}
            Ok(_) => return Err(refused(&at, "not a folder")),
            Err(e) if e.kind() == io::ErrorKind::NotFound && !create => return Err(e),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                // Created here, by this process, one level at a time: it cannot be a link that was there before. A
                // different program that races the creation is caught by the checks below.
                match std::fs::create_dir(&at) {
                    Ok(()) => {}
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(e),
                }
                match std::fs::symlink_metadata(&at) {
                    Ok(m) if m.is_dir() && !m.file_type().is_symlink() => {}
                    _ => return Err(refused(&at, "a link inside the allowed folder")),
                }
            }
            Err(e) => return Err(e),
        }
    }
    let pinned = pin(dir)?;
    // With the folder pinned nothing above it can change any more, so this second look is the one that counts.
    pinned.revalidate(root)?;
    Ok(pinned)
}

#[cfg(windows)]
fn pin(dir: &Path) -> io::Result<Pinned> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 1;
    const FILE_SHARE_WRITE: u32 = 2;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    // Shared for reading and writing but not for delete: a folder open this way cannot be renamed or removed, and nor
    // can any folder above it.
    let handle =
        std::fs::OpenOptions::new().read(true).share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE).custom_flags(FILE_FLAG_BACKUP_SEMANTICS).open(dir)?;
    Ok(Pinned { dir: dir.to_path_buf(), _handle: handle })
}

#[cfg(not(windows))]
fn pin(dir: &Path) -> io::Result<Pinned> {
    Ok(Pinned { dir: dir.to_path_buf() })
}

/// Save `bytes` over `path` (inside `root`) with the folder pinned and revalidated at the commit, and, if given,
/// `authority` checked there too. `path`'s folder is created if it does not exist.
pub fn write_confined(root: &Path, path: &Path, bytes: &[u8], form: StagingName, authority: Option<Authority>) -> Result<Committed, PersistError> {
    write_confined_with(root, path, bytes, form, authority, staging_suffixes(), &Steps::real())
}

/// [`write_confined`], trying the staging names `suffixes` give and performing the operations `steps` give (its
/// `guard` is replaced by this function's).
pub fn write_confined_with(
    root: &Path,
    path: &Path,
    bytes: &[u8],
    form: StagingName,
    authority: Option<Authority>,
    suffixes: impl IntoIterator<Item = u64>,
    steps: &Steps,
) -> Result<Committed, PersistError> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(root);
    let pinned = pin_creating(root, dir).map_err(|e| PersistError { stage: crate::persistence::Stage::Inspect, source: e, original_copy: None })?;
    let guard = |target: &Path| {
        pinned.revalidate(root)?;
        match authority {
            Some(a) => check(target, a),
            None => Ok(()),
        }
    };
    let steps = Steps { guard: &guard, ..*steps };
    let result = write_replace_with(path, bytes, form, suffixes, &steps);
    // The folder stays pinned until the save is over.
    drop(pinned);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tree(test: &str) -> (PathBuf, PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("pdfcraft-confine-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (root, outside) = (base.join("root"), base.join("outside"));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("sentinel.txt"), b"SENTINEL").unwrap();
        let root = root.canonicalize().unwrap();
        let outside = outside.canonicalize().unwrap();
        (base, root, outside)
    }

    /// A directory link `at` -> `to`: a symbolic link on Unix, a junction on Windows (no privilege needed).
    fn link_dir(to: &Path, at: &Path) {
        #[cfg(unix)]
        std::os::unix::fs::symlink(to, at).unwrap();
        #[cfg(windows)]
        {
            let made = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(at).arg(to).output().unwrap().status.success();
            assert!(made, "mklink /J must be available to run this test");
        }
    }

    fn unlink_dir(at: &Path) {
        #[cfg(unix)]
        let _ = std::fs::remove_file(at);
        #[cfg(windows)]
        let _ = std::fs::remove_dir(at);
    }

    fn sentinel_intact(outside: &Path) {
        assert_eq!(std::fs::read(outside.join("sentinel.txt")).unwrap(), b"SENTINEL");
        let names: Vec<_> = std::fs::read_dir(outside).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["sentinel.txt"], "nothing was written outside the root: {names:?}");
    }

    #[test]
    fn a_folder_that_is_a_link_is_refused_and_nothing_is_created_through_it() {
        let (base, root, outside) = tree("link");
        link_dir(&outside, &root.join("inside"));
        for dir in [root.join("inside"), root.join("inside").join("deeper")] {
            let e = pin_creating(&root, &dir).err().expect("a link in the chain is refused");
            assert!(e.to_string().contains("link"), "{e}");
        }
        sentinel_intact(&outside);
        // Writing through it is refused the same way, and leaves nothing behind.
        let e = write_confined(&root, &root.join("inside").join("doc.pdf"), b"X", StagingName::SuffixThenTag, None).unwrap_err();
        assert!(e.to_string().contains("link"), "{e}");
        sentinel_intact(&outside);
        unlink_dir(&root.join("inside"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn missing_folders_are_created_one_at_a_time_inside_the_root() {
        let (base, root, outside) = tree("create");
        let path = root.join("a").join("b").join("doc.pdf");
        write_confined(&root, &path, b"CONTENT", StagingName::SuffixThenTag, None).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"CONTENT");
        assert!(pin_creating(&root, &outside).is_err(), "a folder outside the root is refused");
        sentinel_intact(&outside);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Windows: while the destination is pinned, neither it nor the folders above it can be renamed away, so none can
    /// be swapped for a junction; afterwards they can.
    #[cfg(windows)]
    #[test]
    fn a_pinned_folder_and_its_ancestors_cannot_be_renamed_while_held() {
        let (base, root, outside) = tree("pinned");
        let dir = root.join("a").join("b");
        let pinned = pin_creating(&root, &dir).unwrap();
        for victim in [root.join("a").join("b"), root.join("a")] {
            let moved = victim.with_file_name(format!("{}-moved", victim.file_name().unwrap().to_string_lossy()));
            assert!(std::fs::rename(&victim, &moved).is_err(), "{} could be renamed while pinned", victim.display());
        }
        drop(pinned);
        assert!(std::fs::rename(root.join("a"), root.join("a-moved")).is_ok(), "released after the save");
        sentinel_intact(&outside);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// F06: the folder is swapped for a link while the save is under way. On Windows the swap itself fails (the folder
    /// is pinned) and the save lands inside the root. Elsewhere the swap succeeds and the check at the commit refuses
    /// the save. Either way nothing is written outside the root.
    #[test]
    fn a_folder_swapped_for_a_link_during_the_save_never_receives_the_write() {
        let (base, root, outside) = tree("race");
        let dir = root.join("work");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("doc.pdf");
        std::fs::write(&path, b"OLD").unwrap();
        let swapped = std::cell::Cell::new(false);
        let swap_attempt = std::cell::RefCell::new(None);
        let steps = Steps {
            write: &|file, bytes| {
                // The other program acts while the new bytes are being written.
                let aside = root.join("work-aside");
                let renamed = std::fs::rename(&dir, &aside);
                if renamed.is_ok() {
                    link_dir(&outside, &dir);
                    swapped.set(true);
                }
                *swap_attempt.borrow_mut() = Some(renamed);
                file.write_all(bytes)
            },
            ..Steps::real()
        };
        let result = write_confined_with(&root, &path, b"NEW", StagingName::SuffixThenTag, None, staging_suffixes(), &steps);
        let attempt = swap_attempt.borrow_mut().take().expect("the hook ran");
        if cfg!(windows) {
            assert!(attempt.is_err(), "the pinned folder was renamed away");
            result.expect("the save lands inside the root");
            assert_eq!(std::fs::read(&path).unwrap(), b"NEW");
        } else {
            assert!(swapped.get(), "on this platform the swap is possible");
            let e = result.expect_err("the commit-time check refuses the save");
            assert!(e.to_string().contains("no longer where it was checked") || e.to_string().contains("link"), "{e}");
        }
        sentinel_intact(&outside);
        let _ = std::fs::remove_dir_all(&base);
    }
}
