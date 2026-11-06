//! The journaled replacement of a managed copy: update, rollback and crash recovery.
//!
//! **Files are renamed, never overwritten.** For every path in the old and the new package list the old file
//! (if there is one) is renamed into `BDFUpdate/previous-new/`, then the new file is renamed in from
//! `BDFUpdate/stage/`. A rename is atomic per file, and both folders are on the volume the copy is on. The
//! launcher `bdf.exe` goes last, so a copy interrupted part-way still has the *old* launcher to start and
//! notice (`recover`).
//!
//! * **Nothing is touched until everything is ready.** The package is verified whole, staged and re-hashed, and
//!   every file the replacement will displace is checked to be free: if BDF, the command line or the worker is
//!   still running from this copy the update **waits, and gives up unchanged** after the time it was given. A
//!   process is never ended to make room.
//! * **The journal is written before the first rename** and carries both file lists, so recovery needs nothing
//!   from the copy itself (its `bdf-package.json` may be the very file in motion).
//! * **Interrupted means rolled back.** [`recover`] puts every displaced file back and removes every new one
//!   (moving it back to the stage, so no version is ever destroyed), then checks the old hashes. A run that
//!   finished replacing but not tidying is finished instead.
//! * **The replacement is verified** against the new list before it counts; a mismatch rolls back.
//! * **The previous version is kept** whole in `BDFPrevious/` ([`rollback`] swaps back to it).
//! * Only files named in a package list are ever moved. `BDFData`, and anything else the user keeps beside the
//!   program, is not in any list and is never opened.
//! * One updater at a time: a lock file held for the whole run (released by the system if the process dies).

use std::collections::HashSet;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::layout::{self, PREVIOUS_DIR, UPDATE_DIR};
use crate::manifest::{Assessment, SignedManifest};
use crate::package::{self, FileEntry, PACKAGE_FILE, PackageInfo, hash_file, join_rel};
use crate::{Result, Trust, UpdateError, Version, io_err, this_platform};

/// The launcher: replaced last, so an interrupted run still has an old one.
const LAUNCHER: &str = "bdf.exe";
const JOURNAL: &str = "journal.json";
const LOCK: &str = "lock";
const RESULT: &str = "last-result.json";

/// How an apply waits for the program to stop running.
#[derive(Clone, Debug)]
pub struct Options {
    /// How long to wait for files in use to be released before giving up (nothing is changed).
    pub wait: Duration,
    /// Set to stop waiting.
    pub cancel: Option<Arc<AtomicBool>>,
}

impl Default for Options {
    fn default() -> Self {
        Options { wait: Duration::from_secs(10 * 60), cancel: None }
    }
}

/// A finished replacement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Applied {
    pub from: Version,
    pub to: Version,
}

/// What [`recover`] found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Recovery {
    /// No interrupted update.
    Nothing,
    /// An updater is running right now; leave it alone.
    Busy,
    /// An interrupted update was undone: the copy is back at `from`.
    RolledBack { from: Version, to: Version },
    /// An update that had finished replacing was tidied up: the copy is at `to`.
    Finalized { to: Version },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Kind {
    Update,
    Rollback,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum JState {
    Swapping,
    Verified,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Journal {
    schema: u32,
    state: JState,
    kind: Kind,
    from: String,
    to: String,
    old: Vec<FileEntry>,
    new: Vec<FileEntry>,
}

/// What the helper tells the next BDF launch about the last update attempt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastResult {
    pub when: u64,
    /// `updated`, `rolled-back`, `recovered`, `postponed` or `failed`.
    pub outcome: String,
    pub from: String,
    pub to: String,
    pub message: String,
}

/// Remember `result` for the next launch (best effort: a result that cannot be written is only a lost notice).
pub fn write_result(app_dir: &Path, outcome: &str, from: &str, to: &str, message: &str) {
    let result = LastResult {
        when: crate::now_unix(),
        outcome: outcome.into(),
        from: from.into(),
        to: to.into(),
        message: message.chars().take(600).collect(),
    };
    let dir = layout::update_dir(app_dir);
    if std::fs::create_dir_all(&dir).is_ok()
        && let Ok(bytes) = serde_json::to_vec_pretty(&result)
    {
        let _ = write_atomic(&dir.join(RESULT), &bytes);
    }
}

/// The result of the last update attempt, read once: reading removes it.
pub fn take_result(app_dir: &Path) -> Option<LastResult> {
    let path = layout::update_dir(app_dir).join(RESULT);
    let bytes = std::fs::read(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    serde_json::from_slice(&bytes).ok()
}

/// The result of the last attempt without removing it.
pub fn peek_result(app_dir: &Path) -> Option<LastResult> {
    serde_json::from_slice(&std::fs::read(layout::update_dir(app_dir).join(RESULT)).ok()?).ok()
}

// ---------------------------------------------------------------------------------------------------------------------
// Public operations

/// Install the verified-before-use package at `zip` over the managed copy in `app_dir`.
///
/// `manifest` is the verified release information, `trust` the build's trust root: the package is checked
/// against the signed size and hash, and its contents against its own list, before anything is written.
pub fn apply(app_dir: &Path, zip: &Path, manifest: &SignedManifest, trust: &Trust, opts: &Options) -> Result<Applied> {
    apply_inner(app_dir, zip, manifest, trust, opts, None)
}

/// Swap the copy for the version kept in `BDFPrevious`, keeping the one it replaces as the new `BDFPrevious`.
pub fn rollback(app_dir: &Path, opts: &Options) -> Result<Applied> {
    rollback_inner(app_dir, opts, None)
}

/// Undo or finish an update that was interrupted. Cheap when there is nothing to do: it reads no file when the
/// journal is absent, and creates nothing.
pub fn recover(app_dir: &Path) -> Result<Recovery> {
    let update = layout::update_dir(app_dir);
    if !update.join(JOURNAL).is_file() {
        return Ok(Recovery::Nothing);
    }
    let Some(_lock) = try_lock(&update)? else { return Ok(Recovery::Busy) };
    recover_locked(app_dir)
}

/// The managed files among `paths` that some process has open in a way that stops them being replaced
/// (a running program, a library in use). Always empty off Windows, where files can be replaced while open.
pub fn files_in_use(app_dir: &Path, paths: &[String]) -> Vec<String> {
    paths.iter().filter(|p| in_use(&join_rel(app_dir, p))).cloned().collect()
}

// ---------------------------------------------------------------------------------------------------------------------
// The plan

struct Plan {
    app: PathBuf,
    kind: Kind,
    from: Version,
    to: Version,
    old: Vec<FileEntry>,
    new: Vec<FileEntry>,
}

impl Plan {
    fn update(&self) -> PathBuf {
        layout::update_dir(&self.app)
    }
    fn stage(&self) -> PathBuf {
        self.update().join("stage")
    }
    fn prev_new(&self) -> PathBuf {
        self.update().join("previous-new")
    }
    fn previous(&self) -> PathBuf {
        layout::previous_dir(&self.app)
    }

    fn from_journal(app: &Path, j: &Journal) -> Result<Plan> {
        let v = |s: &str| Version::parse(s).ok_or_else(|| UpdateError::Malformed("the update journal has a bad version".into()));
        for f in j.old.iter().chain(&j.new) {
            if !package::safe_rel_path(&f.path) && !f.path.eq_ignore_ascii_case(PACKAGE_FILE) {
                return Err(UpdateError::Malformed("the update journal names an unsafe path".into()));
            }
        }
        Ok(Plan { app: app.to_path_buf(), kind: j.kind, from: v(&j.from)?, to: v(&j.to)?, old: j.old.clone(), new: j.new.clone() })
    }

    fn journal(&self, state: JState) -> Journal {
        Journal {
            schema: 1,
            state,
            kind: self.kind,
            from: self.from.to_string(),
            to: self.to.to_string(),
            old: self.old.clone(),
            new: self.new.clone(),
        }
    }

    fn write_journal(&self, state: JState) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(&self.journal(state))
            .map_err(|e| UpdateError::Malformed(format!("couldn't write the update journal ({e})")))?;
        write_atomic(&self.update().join(JOURNAL), &bytes).map_err(io_err("couldn't write the update journal"))
    }

    fn remove_journal(&self) {
        let _ = std::fs::remove_file(self.update().join(JOURNAL));
    }

    /// Every path in either list once (case-insensitively), the package list then the launcher last.
    fn order(&self) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut paths: Vec<String> =
            self.new.iter().chain(&self.old).filter(|f| seen.insert(f.path.to_ascii_lowercase())).map(|f| f.path.clone()).collect();
        let rank = |p: &str| match p.to_ascii_lowercase().as_str() {
            LAUNCHER => 2,
            x if x == PACKAGE_FILE => 1,
            _ => 0,
        };
        paths.sort_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| a.to_ascii_lowercase().cmp(&b.to_ascii_lowercase())));
        paths
    }

    fn old_has(&self, path: &str) -> bool {
        self.old.iter().any(|f| f.path.eq_ignore_ascii_case(path))
    }
    fn new_entry(&self, path: &str) -> Option<&FileEntry> {
        self.new.iter().find(|f| f.path.eq_ignore_ascii_case(path))
    }

    /// Refuse before anything moves: a folder or a stranger's file where a package file goes, or a staged file
    /// that is no longer what it was staged as.
    fn preflight(&self) -> Result<()> {
        for f in &self.new {
            let live = join_rel(&self.app, &f.path);
            if live.is_dir() {
                return Err(UpdateError::Io {
                    what: format!("a folder named {} is where BDF needs a file", f.path),
                    source: std::io::Error::other("folder in the way"),
                });
            }
            if live.exists() && !self.old_has(&f.path) {
                return Err(UpdateError::Io {
                    what: format!("a file named {} that is not part of BDF is where the update needs to put one; move it and try again", f.path),
                    source: std::io::Error::other("file in the way"),
                });
            }
            let staged = join_rel(&self.stage(), &f.path);
            let (len, hash) = hash_file(&staged).map_err(io_err(format!("couldn't read the staged {}", f.path)))?;
            if len != f.size || hash != f.sha256 {
                return Err(UpdateError::Tampered(format!("the staged {} changed after it was verified", f.path)));
            }
        }
        Ok(())
    }

    /// Move the old files out and the new files in. `stop_after` ends the run after that many renames without
    /// any clean-up, standing in for a process that died there (tests only).
    fn swap(&self, mut stop_after: Option<usize>) -> Result<()> {
        let (stage, prev) = (self.stage(), self.prev_new());
        std::fs::create_dir_all(&prev).map_err(io_err("couldn't create the folder for the previous version"))?;
        let mut step = |what: &str| -> Result<()> {
            if let Some(left) = stop_after.as_mut() {
                if *left == 0 {
                    return Err(UpdateError::Cancelled);
                }
                *left -= 1;
            }
            let _ = what;
            Ok(())
        };
        for path in self.order() {
            let live = join_rel(&self.app, &path);
            if live.is_file() {
                step("displace")?;
                let to = join_rel(&prev, &path);
                make_parent(&to)?;
                rename_retry(&live, &to).map_err(io_err(format!("couldn't move the old {path} aside")))?;
            }
            if self.new_entry(&path).is_some() {
                step("install")?;
                make_parent(&live)?;
                rename_retry(&join_rel(&stage, &path), &live).map_err(io_err(format!("couldn't put the new {path} in place")))?;
            }
        }
        Ok(())
    }

    /// Check the installed files against the new list.
    fn verify_new(&self) -> Result<()> {
        for f in &self.new {
            let live = join_rel(&self.app, &f.path);
            let (len, hash) = hash_file(&live).map_err(|e| UpdateError::Tampered(format!("{} is missing after the replacement ({e})", f.path)))?;
            if len != f.size || hash != f.sha256 {
                return Err(UpdateError::Tampered(format!("{} does not match the release after the replacement", f.path)));
            }
        }
        Ok(())
    }

    /// Put every displaced file back and move every installed one back to the stage; then check the old
    /// hashes. Safe to run again after an interruption of itself.
    fn roll_back(&self) -> Result<()> {
        let (stage, prev) = (self.stage(), self.prev_new());
        let mut order = self.order();
        order.reverse();
        for path in &order {
            let live = join_rel(&self.app, path);
            let displaced = join_rel(&prev, path);
            if displaced.is_file() {
                if live.is_file() {
                    let to = join_rel(&stage, path);
                    make_parent(&to)?;
                    // A copy of this file may already be in the stage (a rollback of a rollback); the live one wins.
                    let _ = std::fs::remove_file(&to);
                    rename_retry(&live, &to).map_err(io_err(format!("couldn't move the new {path} aside")))?;
                }
                make_parent(&live)?;
                rename_retry(&displaced, &live).map_err(io_err(format!("couldn't restore the old {path}")))?;
            } else if !self.old_has(path)
                && live.is_file()
                && let Some(new) = self.new_entry(path)
                && hash_file(&live).is_ok_and(|(len, hash)| len == new.size && hash == new.sha256)
            {
                // Installed by this update and nothing was displaced for it: it did not exist before.
                let to = join_rel(&stage, path);
                make_parent(&to)?;
                let _ = std::fs::remove_file(&to);
                rename_retry(&live, &to).map_err(io_err(format!("couldn't remove the new {path}")))?;
            }
        }
        // Folders the new version created and the old one never had.
        for f in self.new.iter().filter(|f| !self.old_has(&f.path)) {
            remove_empty_parents(&self.app, &join_rel(&self.app, &f.path));
        }
        for f in &self.old {
            let live = join_rel(&self.app, &f.path);
            let (len, hash) = hash_file(&live).map_err(|e| UpdateError::Tampered(format!("{} could not be restored ({e})", f.path)))?;
            if len != f.size || hash != f.sha256 {
                return Err(UpdateError::Tampered(format!("{} does not match the previous version after restoring it", f.path)));
            }
        }
        // A rolled-back rollback gives the kept version its name back; a rolled-back update has nothing to keep.
        if self.kind == Kind::Rollback && self.stage().is_dir() && !self.previous().exists() {
            std::fs::rename(self.stage(), self.previous()).map_err(io_err("couldn't put the previous version back"))?;
        }
        let _ = std::fs::remove_dir_all(self.stage());
        let _ = std::fs::remove_dir_all(self.prev_new());
        Ok(())
    }

    /// Keep the displaced version as `BDFPrevious` and tidy up. Safe to run again.
    fn finalize(&self) -> Result<()> {
        if self.prev_new().is_dir() {
            let previous = self.previous();
            if previous.exists() {
                std::fs::remove_dir_all(&previous).map_err(io_err("couldn't remove the older previous version"))?;
            }
            std::fs::rename(self.prev_new(), &previous).map_err(io_err("couldn't keep the previous version"))?;
        }
        let _ = std::fs::remove_dir_all(self.stage());
        Ok(())
    }
}

fn apply_inner(app_dir: &Path, zip: &Path, manifest: &SignedManifest, trust: &Trust, opts: &Options, crash_after: Option<usize>) -> Result<Applied> {
    let install = layout::detect(app_dir)?;
    let (os, arch) = this_platform();
    let available = match manifest.assess(install.version, None, os, arch, trust.channel())? {
        Assessment::Available(a) => a,
        Assessment::UpToDate => return Err(UpdateError::NotApplicable(format!("version {} is already installed", install.version))),
    };
    // The whole package is checked (hash, signature-covered size, contents) before anything is written.
    let verified = package::verify_zip(zip, &available.package, available.version, os, arch)?;
    layout::ensure_writable(app_dir)?;
    let update = layout::update_dir(app_dir);
    let _lock = try_lock(&update)?.ok_or_else(|| UpdateError::InUse("another BDF update is already running".into()))?;
    recover_locked(app_dir)?;

    let installed_list = package_file_entry(&app_dir.join(PACKAGE_FILE)).map_err(io_err(format!("couldn't read {PACKAGE_FILE}")))?;
    let mut old = install.info.files.clone();
    old.push(installed_list);
    let mut new = verified.info.files.clone();
    new.push(FileEntry { path: PACKAGE_FILE.into(), size: verified.info_bytes.len() as u64, sha256: package::sha256_hex(&verified.info_bytes) });
    let plan = Plan { app: app_dir.to_path_buf(), kind: Kind::Update, from: install.version, to: available.version, old, new };

    let _ = std::fs::remove_dir_all(plan.stage());
    let _ = std::fs::remove_dir_all(plan.prev_new());
    let cleanup = || {
        let _ = std::fs::remove_dir_all(plan.stage());
        let _ = std::fs::remove_dir_all(plan.prev_new());
    };
    if let Err(e) = package::stage(zip, &verified, &plan.stage()) {
        cleanup();
        return Err(e);
    }
    let ready = plan.preflight().and_then(|()| wait_until_free(app_dir, &plan.order(), opts));
    if let Err(e) = ready {
        cleanup();
        return Err(e);
    }
    run_swap(&plan, crash_after)
}

/// The journaled part shared by update and rollback: swap, verify, finalize; roll back on any failure.
fn run_swap(plan: &Plan, crash_after: Option<usize>) -> Result<Applied> {
    plan.write_journal(JState::Swapping)?;
    if plan.kind == Kind::Rollback {
        std::fs::rename(plan.previous(), plan.stage()).map_err(|e| {
            plan.remove_journal();
            io_err("couldn't prepare the previous version")(e)
        })?;
    }
    let outcome = plan.swap(crash_after).and_then(|()| {
        #[cfg(test)]
        tests::corrupt_hook(plan);
        plan.verify_new()
    });
    if let Err(e) = outcome {
        if matches!(e, UpdateError::Cancelled) && crash_after.is_some() {
            // Test only: the process "died" here; recovery happens on the next start.
            return Err(e);
        }
        return match plan.roll_back() {
            Ok(()) => {
                plan.remove_journal();
                Err(UpdateError::RolledBack(e.to_string()))
            }
            Err(second) => Err(UpdateError::Io {
                what: format!("the update failed ({e}) and the previous version could not be restored ({second}); run bdf-updater recover"),
                source: std::io::Error::other("rollback failed"),
            }),
        };
    }
    plan.write_journal(JState::Verified)?;
    if crash_after == Some(usize::MAX) {
        return Err(UpdateError::Cancelled);
    }
    plan.finalize()?;
    plan.remove_journal();
    Ok(Applied { from: plan.from, to: plan.to })
}

fn rollback_inner(app_dir: &Path, opts: &Options, crash_after: Option<usize>) -> Result<Applied> {
    let install = layout::detect(app_dir)?;
    let previous = layout::previous_dir(app_dir);
    let none = || UpdateError::NotApplicable("there is no previous version to go back to".into());
    let bytes = std::fs::read(previous.join(PACKAGE_FILE)).map_err(|_| none())?;
    let prev_info = PackageInfo::parse(&bytes).map_err(|_| none())?;
    let prev_version = prev_info.parsed_version()?;
    // The kept version must still be whole before the working one is moved for it.
    let mut new = prev_info.files.clone();
    for f in &new {
        let (len, hash) = hash_file(&join_rel(&previous, &f.path))
            .map_err(|_| UpdateError::Tampered(format!("the previous version is incomplete ({} is missing)", f.path)))?;
        if len != f.size || hash != f.sha256 {
            return Err(UpdateError::Tampered(format!("the previous version is damaged ({} differs)", f.path)));
        }
    }
    new.push(package_file_entry(&previous.join(PACKAGE_FILE)).map_err(io_err("couldn't read the previous package list"))?);
    layout::ensure_writable(app_dir)?;
    let update = layout::update_dir(app_dir);
    let _lock = try_lock(&update)?.ok_or_else(|| UpdateError::InUse("another BDF update is already running".into()))?;
    recover_locked(app_dir)?;
    let mut old = install.info.files.clone();
    old.push(package_file_entry(&app_dir.join(PACKAGE_FILE)).map_err(io_err(format!("couldn't read {PACKAGE_FILE}")))?);
    let plan = Plan { app: app_dir.to_path_buf(), kind: Kind::Rollback, from: install.version, to: prev_version, old, new };
    let _ = std::fs::remove_dir_all(plan.stage());
    let _ = std::fs::remove_dir_all(plan.prev_new());
    // The kept files are the stage: hand them over by name, then check the list against them.
    wait_until_free(app_dir, &plan.order(), opts)?;
    run_swap_rollback_preflight(&plan)?;
    run_swap(&plan, crash_after)
}

/// A rollback's preflight runs against `BDFPrevious` (it becomes the stage inside [`run_swap`]).
fn run_swap_rollback_preflight(plan: &Plan) -> Result<()> {
    for f in &plan.new {
        let live = join_rel(&plan.app, &f.path);
        if live.is_dir() {
            return Err(UpdateError::Io {
                what: format!("a folder named {} is where BDF needs a file", f.path),
                source: std::io::Error::other("folder in the way"),
            });
        }
        if live.exists() && !plan.old_has(&f.path) {
            return Err(UpdateError::Io {
                what: format!("a file named {} that is not part of BDF is in the way; move it and try again", f.path),
                source: std::io::Error::other("file in the way"),
            });
        }
    }
    Ok(())
}

fn recover_locked(app_dir: &Path) -> Result<Recovery> {
    let update = layout::update_dir(app_dir);
    let journal_path = update.join(JOURNAL);
    let bytes = match std::fs::read(&journal_path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Recovery::Nothing),
        Err(e) => return Err(io_err("couldn't read the update journal")(e)),
    };
    let journal: Journal = serde_json::from_slice(&bytes).map_err(|e| UpdateError::Malformed(format!("the update journal is unreadable ({e})")))?;
    let plan = Plan::from_journal(app_dir, &journal)?;
    match journal.state {
        JState::Swapping => {
            // A rollback in progress may have moved BDFPrevious to the stage already.
            plan.roll_back()?;
            plan.remove_journal();
            write_result(
                app_dir,
                "recovered",
                &plan.from.to_string(),
                &plan.to.to_string(),
                &format!("An update to version {} was interrupted; BDF was restored to version {}.", plan.to, plan.from),
            );
            Ok(Recovery::RolledBack { from: plan.from, to: plan.to })
        }
        JState::Verified => {
            plan.finalize()?;
            plan.remove_journal();
            Ok(Recovery::Finalized { to: plan.to })
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Pieces

fn package_file_entry(path: &Path) -> std::io::Result<FileEntry> {
    let (size, sha256) = hash_file(path)?;
    Ok(FileEntry { path: PACKAGE_FILE.into(), size, sha256 })
}

/// Take the updater lock, or `None` when another updater holds it.
fn try_lock(update_dir: &Path) -> Result<Option<File>> {
    std::fs::create_dir_all(update_dir).map_err(io_err("couldn't create the update folder"))?;
    let file =
        File::options().create(true).truncate(false).write(true).open(update_dir.join(LOCK)).map_err(io_err("couldn't open the update lock"))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(e)) => Err(io_err("couldn't take the update lock")(e)),
    }
}

fn wait_until_free(app_dir: &Path, paths: &[String], opts: &Options) -> Result<()> {
    let start = Instant::now();
    loop {
        let busy = files_in_use(app_dir, paths);
        if busy.is_empty() {
            return Ok(());
        }
        if opts.cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed)) {
            return Err(UpdateError::Cancelled);
        }
        if start.elapsed() >= opts.wait {
            let names: Vec<&str> = busy.iter().take(3).map(String::as_str).collect();
            return Err(UpdateError::InUse(format!(
                "BDF is still running from this folder ({}); close it, or let its work finish, and update again. Nothing was changed",
                names.join(", ")
            )));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

#[cfg(windows)]
fn in_use(path: &Path) -> bool {
    use std::os::windows::fs::OpenOptionsExt;
    if !path.is_file() {
        return false;
    }
    // Share mode 0: fails with a sharing violation while any process has the file open or mapped as a program.
    match File::options().read(true).write(true).share_mode(0).open(path) {
        Ok(_) => false,
        Err(e) => matches!(e.raw_os_error(), Some(32 | 33)),
    }
}

#[cfg(not(windows))]
fn in_use(_path: &Path) -> bool {
    false
}

fn make_parent(path: &Path) -> Result<()> {
    match path.parent() {
        Some(parent) => std::fs::create_dir_all(parent).map_err(io_err("couldn't create a folder")),
        None => Ok(()),
    }
}

/// Remove now-empty folders from `file`'s parent up to (not including) `root`.
fn remove_empty_parents(root: &Path, file: &Path) {
    let mut dir = file.parent();
    while let Some(d) = dir {
        if d == root || !d.starts_with(root) || std::fs::remove_dir(d).is_err() {
            break;
        }
        dir = d.parent();
    }
}

/// A rename that tries again briefly: a virus scanner or an indexer can hold a freshly written file for a moment.
fn rename_retry(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut attempt = 0;
    loop {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) if attempt < 8 && (e.kind() == std::io::ErrorKind::PermissionDenied || matches!(e.raw_os_error(), Some(5 | 32 | 33))) => {
                attempt += 1;
                std::thread::sleep(Duration::from_millis(150));
            }
            Err(e) => return Err(e),
        }
    }
}

/// Write `bytes` to `path` through a temporary file in the same folder, flushed before the rename.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = path.with_extension("tmp");
    {
        let mut f = File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

/// The names reserved inside the app folder for the updater (never in a package).
pub fn reserved_names() -> [&'static str; 2] {
    [UPDATE_DIR, PREVIOUS_DIR]
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::collections::BTreeMap;

    use super::*;
    use crate::manifest::testing::manifest_json;
    use crate::trust::testkeys;

    thread_local! {
        static CORRUPT: Cell<Option<&'static str>> = const { Cell::new(None) };
    }

    /// Lets a test damage one installed file after the swap, before it is verified.
    pub(super) fn corrupt_hook(plan: &Plan) {
        if let Some(name) = CORRUPT.with(Cell::take) {
            let _ = std::fs::write(join_rel(&plan.app, name), b"damaged");
        }
    }

    const V0: &[(&str, &[u8])] = &[
        ("bdf.exe", b"exe-0"),
        ("bdf-cli.exe", b"cli-0"),
        ("old-only.dll", b"old-only"),
        ("portable.txt", b"p"),
        ("licenses/MIT.txt", b"license-0"),
    ];
    const V1: &[(&str, &[u8])] = &[
        ("bdf.exe", b"exe-1"),
        ("bdf-cli.exe", b"cli-1"),
        ("new-only.txt", b"new-only"),
        ("portable.txt", b"p"),
        ("licenses/MIT.txt", b"license-0"),
        ("docs/readme.txt", b"r1"),
    ];
    const V2: &[(&str, &[u8])] = &[
        ("bdf.exe", b"exe-2"),
        ("bdf-cli.exe", b"cli-2"),
        ("new-only.txt", b"new-only-2"),
        ("portable.txt", b"p"),
        ("licenses/MIT.txt", b"license-0"),
    ];

    use crate::testing::{installed_copy as installed, release, trust};

    fn quick() -> Options {
        Options { wait: Duration::from_millis(300), cancel: None }
    }

    /// Every file under `dir` with its hash, except the updater's own working folder.
    fn snapshot(dir: &Path) -> BTreeMap<String, String> {
        fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, String>) {
            for e in std::fs::read_dir(dir).unwrap() {
                let p = e.unwrap().path();
                let rel = p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
                if rel == UPDATE_DIR {
                    continue;
                }
                if p.is_dir() {
                    walk(root, &p, out);
                } else {
                    out.insert(rel, package::sha256_hex(&std::fs::read(&p).unwrap()));
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(dir, dir, &mut out);
        out
    }

    fn live_files(app: &Path) -> BTreeMap<String, Vec<u8>> {
        snapshot(app)
            .keys()
            .filter(|k| !k.starts_with("BDFData/") && !k.starts_with(PREVIOUS_DIR) && *k != "notes.txt")
            .map(|k| (k.clone(), std::fs::read(app.join(k)).unwrap()))
            .collect()
    }

    fn expect(files: &[(&str, &[u8])]) -> BTreeMap<String, Vec<u8>> {
        files.iter().map(|(p, c)| ((*p).to_string(), c.to_vec())).collect()
    }

    fn version_of(app: &Path) -> Version {
        layout::detect(app).unwrap().version
    }

    fn assert_files(app: &Path, files: &[(&str, &[u8])], version: Version) {
        let mut want = expect(files);
        let listing = std::fs::read(app.join(PACKAGE_FILE)).unwrap();
        want.insert(PACKAGE_FILE.into(), listing);
        assert_eq!(live_files(app), want);
        assert_eq!(version_of(app), version);
    }

    fn user_data(app: &Path) -> BTreeMap<String, String> {
        snapshot(app).into_iter().filter(|(k, _)| k.starts_with("BDFData/") || k == "notes.txt").collect()
    }

    #[test]
    fn an_update_replaces_the_program_keeps_the_data_and_keeps_the_previous_version() {
        let root = tempfile::tempdir().unwrap();
        let app = installed(root.path(), "0.8.0", V0);
        let data = user_data(&app);
        let (pkg, m) = release(root.path(), "0.8.1", V1);
        let applied = apply(&app, &pkg.zip, &m, &trust(), &quick()).unwrap();
        assert_eq!(applied, Applied { from: Version(0, 8, 0), to: Version(0, 8, 1) });
        assert_files(&app, V1, Version(0, 8, 1));
        assert_eq!(user_data(&app), data, "settings, recovery files and the user's own files are untouched");
        // The previous version is whole and recognisable.
        let prev = layout::previous_dir(&app);
        assert_eq!(std::fs::read(prev.join("bdf.exe")).unwrap(), b"exe-0");
        assert_eq!(std::fs::read(prev.join("old-only.dll")).unwrap(), b"old-only");
        assert_eq!(PackageInfo::parse(&std::fs::read(prev.join(PACKAGE_FILE)).unwrap()).unwrap().version, "0.8.0");
        // No working files left behind but the result.
        let update = layout::update_dir(&app);
        assert!(!update.join("stage").exists() && !update.join("previous-new").exists() && !update.join(JOURNAL).exists());
    }

    #[test]
    fn two_successive_upgrades_each_keep_the_data_and_the_one_before() {
        let root = tempfile::tempdir().unwrap();
        let app = installed(root.path(), "0.8.0", V0);
        let data = user_data(&app);
        let (p1, m1) = release(root.path(), "0.8.1", V1);
        apply(&app, &p1.zip, &m1, &trust(), &quick()).unwrap();
        let (p2, m2) = release(root.path(), "0.8.2", V2);
        apply(&app, &p2.zip, &m2, &trust(), &quick()).unwrap();
        assert_files(&app, V2, Version(0, 8, 2));
        assert_eq!(user_data(&app), data);
        let prev = PackageInfo::parse(&std::fs::read(layout::previous_dir(&app).join(PACKAGE_FILE)).unwrap()).unwrap();
        assert_eq!(prev.version, "0.8.1", "only the version just before is kept");
        assert!(
            !app.join("docs").exists() || app.join("docs").read_dir().unwrap().next().is_none(),
            "files the newer version dropped are gone from the live copy"
        );
    }

    #[test]
    fn rollback_returns_to_the_previous_version_and_back() {
        let root = tempfile::tempdir().unwrap();
        let app = installed(root.path(), "0.8.0", V0);
        let data = user_data(&app);
        let (pkg, m) = release(root.path(), "0.8.1", V1);
        apply(&app, &pkg.zip, &m, &trust(), &quick()).unwrap();
        let back = rollback(&app, &quick()).unwrap();
        assert_eq!(back, Applied { from: Version(0, 8, 1), to: Version(0, 8, 0) });
        assert_files(&app, V0, Version(0, 8, 0));
        assert_eq!(user_data(&app), data);
        // The version that was rolled back from is now the kept one, and can be returned to.
        rollback(&app, &quick()).unwrap();
        assert_files(&app, V1, Version(0, 8, 1));
        assert_eq!(user_data(&app), data);
    }

    #[test]
    fn rollback_without_a_previous_version_says_so() {
        let root = tempfile::tempdir().unwrap();
        let app = installed(root.path(), "0.8.0", V0);
        assert!(matches!(rollback(&app, &quick()), Err(UpdateError::NotApplicable(_))));
    }

    #[test]
    fn a_damaged_previous_version_is_not_swapped_in() {
        let root = tempfile::tempdir().unwrap();
        let app = installed(root.path(), "0.8.0", V0);
        let (pkg, m) = release(root.path(), "0.8.1", V1);
        apply(&app, &pkg.zip, &m, &trust(), &quick()).unwrap();
        std::fs::write(layout::previous_dir(&app).join("bdf.exe"), b"corrupt").unwrap();
        let before = snapshot(&app);
        assert!(matches!(rollback(&app, &quick()), Err(UpdateError::Tampered(_))));
        assert_eq!(snapshot(&app), before);
    }

    #[test]
    fn nothing_changes_when_the_release_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let app = installed(root.path(), "0.8.0", V0);
        let before = snapshot(&app);
        // The same version, an older one, a tampered package, a package for another version than signed.
        let (same, m_same) = release(root.path(), "0.8.0", V0);
        assert!(matches!(apply(&app, &same.zip, &m_same, &trust(), &quick()), Err(UpdateError::NotApplicable(_))));
        let (old, m_old) = release(root.path(), "0.7.9", V0);
        assert!(matches!(apply(&app, &old.zip, &m_old, &trust(), &quick()), Err(UpdateError::NotApplicable(_))));
        let (pkg, m) = release(root.path(), "0.8.1", V1);
        let mut bytes = std::fs::read(&pkg.zip).unwrap();
        let mid = bytes.len() / 2;
        bytes[mid] ^= 0x55;
        std::fs::write(&pkg.zip, &bytes).unwrap();
        assert!(matches!(apply(&app, &pkg.zip, &m, &trust(), &quick()), Err(UpdateError::Tampered(_))));
        // A manifest signed by someone else is not a manifest.
        let (pkg2, _) = release(root.path(), "0.8.1", V1);
        let forged = manifest_json("0.8.1", "stable", &pkg2.entry.sha256, pkg2.entry.size);
        assert!(crate::manifest::SignedManifest::verify(forged.as_bytes(), &testkeys::sign(9, forged.as_bytes()), &trust()).is_err());
        assert_eq!(snapshot(&app), before, "the copy is exactly as it was");
        assert!(!layout::update_dir(&app).join("stage").exists());
    }

    #[test]
    fn a_folder_that_is_not_a_managed_package_refuses_to_update() {
        let root = tempfile::tempdir().unwrap();
        let app = root.path().join("app");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(app.join("bdf.exe"), b"hand-built").unwrap();
        let (pkg, m) = release(root.path(), "0.8.1", V1);
        assert!(matches!(apply(&app, &pkg.zip, &m, &trust(), &quick()), Err(UpdateError::Unmanaged(_))));
        assert_eq!(std::fs::read(app.join("bdf.exe")).unwrap(), b"hand-built");
    }

    #[test]
    fn a_strangers_file_in_the_way_stops_the_update_before_anything_moves() {
        let root = tempfile::tempdir().unwrap();
        let app = installed(root.path(), "0.8.0", V0);
        std::fs::write(app.join("new-only.txt"), b"the user's own file").unwrap();
        let before = snapshot(&app);
        let (pkg, m) = release(root.path(), "0.8.1", V1);
        let err = apply(&app, &pkg.zip, &m, &trust(), &quick()).unwrap_err();
        assert!(err.to_string().contains("new-only.txt"), "{err}");
        assert_eq!(snapshot(&app), before);
        // A folder where a file goes.
        std::fs::remove_file(app.join("new-only.txt")).unwrap();
        std::fs::create_dir(app.join("new-only.txt")).unwrap();
        assert!(apply(&app, &pkg.zip, &m, &trust(), &quick()).is_err());
        assert!(app.join("bdf.exe").is_file() && version_of(&app) == Version(0, 8, 0));
    }

    #[cfg(windows)]
    #[test]
    fn a_running_program_makes_the_update_wait_and_give_up_unchanged_then_succeed_once_it_stops() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = tempfile::tempdir().unwrap();
        let app = installed(root.path(), "0.8.0", V0);
        let before = snapshot(&app);
        let (pkg, m) = release(root.path(), "0.8.1", V1);
        // "bdf-cli.exe is running": a handle that lets others read but not write or delete, as a running image does.
        let running = File::options().read(true).share_mode(1).open(app.join("bdf-cli.exe")).unwrap();
        let started = Instant::now();
        let err = apply(&app, &pkg.zip, &m, &trust(), &quick()).unwrap_err();
        assert!(matches!(err, UpdateError::InUse(_)), "{err}");
        assert!(err.to_string().contains("bdf-cli.exe") && err.to_string().contains("Nothing was changed"), "{err}");
        assert!(started.elapsed() >= Duration::from_millis(250), "it waited for the program to finish first");
        assert_eq!(snapshot(&app), before);
        assert!(!layout::update_dir(&app).join("stage").exists(), "the staged copy is removed");
        // Cancelling the wait is also a clean stop.
        let cancel = Arc::new(AtomicBool::new(true));
        let cancelled = apply(&app, &pkg.zip, &m, &trust(), &Options { wait: Duration::from_secs(30), cancel: Some(cancel) });
        assert!(matches!(cancelled, Err(UpdateError::Cancelled)));
        drop(running);
        apply(&app, &pkg.zip, &m, &trust(), &quick()).unwrap();
        assert_files(&app, V1, Version(0, 8, 1));
    }

    #[test]
    fn a_second_updater_does_not_run_alongside_the_first() {
        let root = tempfile::tempdir().unwrap();
        let app = installed(root.path(), "0.8.0", V0);
        let (pkg, m) = release(root.path(), "0.8.1", V1);
        let held = try_lock(&layout::update_dir(&app)).unwrap().unwrap();
        assert!(matches!(apply(&app, &pkg.zip, &m, &trust(), &quick()), Err(UpdateError::InUse(_))));
        assert_eq!(recover(&app).unwrap(), Recovery::Nothing);
        drop(held);
        apply(&app, &pkg.zip, &m, &trust(), &quick()).unwrap();
    }

    /// The process dies after any number of renames: the next start restores the old version exactly, keeps
    /// the user's data, and a fresh update then succeeds.
    #[test]
    fn an_update_interrupted_at_any_step_is_rolled_back_exactly() {
        let (pkg_root, _) = (tempfile::tempdir().unwrap(), ());
        let (pkg, m) = release(pkg_root.path(), "0.8.1", V1);
        // Count the renames of a full run, to cover every position.
        let steps = 2 * (V0.len().max(V1.len()) + 3);
        for stop in 0..steps {
            let root = tempfile::tempdir().unwrap();
            let app = installed(root.path(), "0.8.0", V0);
            let before = snapshot(&app);
            let data = user_data(&app);
            let result = apply_inner(&app, &pkg.zip, &m, &trust(), &quick(), Some(stop));
            if result.is_ok() {
                // The run was shorter than `stop` renames: it completed.
                assert_files(&app, V1, Version(0, 8, 1));
                continue;
            }
            assert!(layout::update_dir(&app).join(JOURNAL).is_file(), "stop {stop}: the interruption left its journal");
            let recovered = recover(&app).unwrap();
            assert!(matches!(recovered, Recovery::RolledBack { from: Version(0, 8, 0), to: Version(0, 8, 1) }), "stop {stop}: {recovered:?}");
            assert_eq!(snapshot(&app), before, "stop {stop}: the old version is back byte for byte");
            assert_eq!(user_data(&app), data);
            assert!(!layout::update_dir(&app).join(JOURNAL).exists());
            let note = peek_result(&app).unwrap();
            assert_eq!(note.outcome, "recovered");
            // And the update can simply be run again.
            apply(&app, &pkg.zip, &m, &trust(), &quick()).unwrap();
            assert_files(&app, V1, Version(0, 8, 1));
            assert_eq!(user_data(&app), data);
        }
    }

    #[test]
    fn a_replacement_that_finished_but_was_not_tidied_is_finished_on_recovery() {
        let root = tempfile::tempdir().unwrap();
        let app = installed(root.path(), "0.8.0", V0);
        let (pkg, m) = release(root.path(), "0.8.1", V1);
        let r = apply_inner(&app, &pkg.zip, &m, &trust(), &quick(), Some(usize::MAX));
        assert!(matches!(r, Err(UpdateError::Cancelled)));
        assert!(layout::update_dir(&app).join("previous-new").is_dir());
        assert_eq!(recover(&app).unwrap(), Recovery::Finalized { to: Version(0, 8, 1) });
        assert_files(&app, V1, Version(0, 8, 1));
        assert_eq!(std::fs::read(layout::previous_dir(&app).join("bdf.exe")).unwrap(), b"exe-0");
    }

    #[test]
    fn a_rollback_interrupted_at_any_step_is_undone_without_losing_a_version() {
        let (pkg_root, _) = (tempfile::tempdir().unwrap(), ());
        let (pkg, m) = release(pkg_root.path(), "0.8.1", V1);
        for stop in 0..(2 * (V0.len().max(V1.len()) + 3)) {
            let root = tempfile::tempdir().unwrap();
            let app = installed(root.path(), "0.8.0", V0);
            apply(&app, &pkg.zip, &m, &trust(), &quick()).unwrap();
            let before = snapshot(&app);
            let r = rollback_inner(&app, &quick(), Some(stop));
            if r.is_ok() {
                assert_files(&app, V0, Version(0, 8, 0));
                continue;
            }
            assert!(matches!(recover(&app).unwrap(), Recovery::RolledBack { .. }), "stop {stop}");
            assert_eq!(snapshot(&app), before, "stop {stop}: both the live and the kept version are as they were");
            assert!(layout::previous_dir(&app).join(PACKAGE_FILE).is_file(), "stop {stop}: the kept version still has its name");
        }
    }

    #[test]
    fn a_replacement_that_fails_verification_is_rolled_back() {
        let root = tempfile::tempdir().unwrap();
        let app = installed(root.path(), "0.8.0", V0);
        let before = snapshot(&app);
        let (pkg, m) = release(root.path(), "0.8.1", V1);
        CORRUPT.with(|c| c.set(Some("bdf-cli.exe")));
        let err = apply(&app, &pkg.zip, &m, &trust(), &quick()).unwrap_err();
        assert!(matches!(err, UpdateError::RolledBack(_)), "{err}");
        assert_eq!(snapshot(&app), before);
        assert!(!layout::update_dir(&app).join(JOURNAL).exists());
        // The folder is usable for the next attempt.
        apply(&app, &pkg.zip, &m, &trust(), &quick()).unwrap();
    }

    #[test]
    fn an_unwritable_update_folder_is_reported_and_changes_nothing() {
        let root = tempfile::tempdir().unwrap();
        let app = installed(root.path(), "0.8.0", V0);
        // A file where the updater's folder must go stands in for a folder it cannot create.
        std::fs::write(app.join(UPDATE_DIR), b"in the way").unwrap();
        let before = snapshot(&app);
        let (pkg, m) = release(root.path(), "0.8.1", V1);
        assert!(apply(&app, &pkg.zip, &m, &trust(), &quick()).is_err());
        assert_eq!(snapshot(&app), before);
    }

    #[test]
    fn recovery_reads_nothing_and_creates_nothing_when_no_update_was_interrupted() {
        let root = tempfile::tempdir().unwrap();
        let app = installed(root.path(), "0.8.0", V0);
        let before = std::fs::read_dir(&app).unwrap().count();
        assert_eq!(recover(&app).unwrap(), Recovery::Nothing);
        assert_eq!(std::fs::read_dir(&app).unwrap().count(), before);
        assert!(!layout::update_dir(&app).exists());
    }

    #[test]
    fn a_hostile_journal_cannot_move_files_outside_the_folder() {
        let root = tempfile::tempdir().unwrap();
        let app = installed(root.path(), "0.8.0", V0);
        let update = layout::update_dir(&app);
        std::fs::create_dir_all(&update).unwrap();
        let journal = serde_json::json!({
            "schema": 1, "state": "swapping", "kind": "update", "from": "0.8.0", "to": "0.8.1",
            "old": [{"path": "../outside.txt", "size": 1, "sha256": "a".repeat(64)}], "new": []
        });
        std::fs::write(update.join(JOURNAL), journal.to_string()).unwrap();
        std::fs::write(root.path().join("outside.txt"), b"x").unwrap();
        assert!(recover(&app).is_err());
        assert_eq!(std::fs::read(root.path().join("outside.txt")).unwrap(), b"x");
    }

    #[test]
    fn the_result_note_is_read_once() {
        let root = tempfile::tempdir().unwrap();
        write_result(root.path(), "updated", "0.8.0", "0.8.1", "ok");
        assert_eq!(take_result(root.path()).unwrap().to, "0.8.1");
        assert!(take_result(root.path()).is_none());
    }
}
