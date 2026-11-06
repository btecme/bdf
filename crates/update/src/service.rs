//! The one object the app, the command line and the agent tools share: status, check, prepare (download and
//! verify), apply (hand over to the helper) and rollback.
//!
//! Status and the cached release are read from disk and verified again, so they work offline and never touch
//! the network. Everything that does touch it (`check`, `prepare`) is a separate call, and everything that
//! changes the installed copy (`launch_apply`, `launch_rollback`) is separate again: the callers gate the
//! three kinds of call with three different authorisations.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::apply::{self, LastResult};
use crate::layout;
use crate::manifest::{Assessment, Available, SignedManifest};
#[cfg(feature = "net")]
use crate::now_unix;
use crate::package::{self, PACKAGE_FILE, PackageInfo};
use crate::state::State;
use crate::{Result, Trust, UpdateError, Version, this_platform};

/// The helper that applies an update: a separate small program, run from a copy so the one in the
/// folder is free to be replaced.
pub const HELPER_NAME: &str = if cfg!(windows) { "bdf-updater.exe" } else { "bdf-updater" };
/// Files the person's session may be asked to reopen after an update, at most.
pub const MAX_REOPEN: usize = 20;

/// The release on offer, as shown to the person.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Offer {
    pub version: String,
    pub notes: String,
    pub notes_url: Option<String>,
    pub published: String,
    /// Package size in bytes.
    pub size: u64,
}

impl Offer {
    fn from(a: &Available) -> Offer {
        Offer {
            version: a.version.to_string(),
            notes: a.notes.clone(),
            notes_url: a.notes_url.clone(),
            published: a.published.clone(),
            size: a.package.size,
        }
    }
}

/// Everything the interface, the CLI and the tools say about updates. Reading it never uses the network.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Status {
    pub current: String,
    pub channel: String,
    /// A development build, made with a test signing key (labelled in the interface).
    pub dev_build: bool,
    /// `configured` or `missing` (a build without a key refuses every update).
    pub trust: String,
    pub key_id: Option<String>,
    pub managed: bool,
    /// Why this copy cannot update itself, when it cannot.
    pub managed_note: Option<String>,
    pub app_dir: String,
    pub update_available: Option<Offer>,
    /// A verified package for the release on offer is already downloaded.
    pub ready_to_install: bool,
    /// Unix time of the last successful check.
    pub last_check: Option<u64>,
    /// Unix time the next automatic check is due.
    pub next_check: u64,
    pub last_error: Option<String>,
    /// A genuine release this check refused (older than the copy, or a replay), for diagnostics.
    pub rejected: Option<String>,
    pub previous_version: Option<String>,
    /// What the last update attempt reported (from the helper), if it has not been shown yet.
    pub last_result: Option<LastResult>,
    pub feed: Option<String>,
}

/// A downloaded, verified package ready to hand to the helper.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prepared {
    pub version: Version,
    pub zip: PathBuf,
    /// The signed release information (the envelope) the package was verified against.
    pub release: PathBuf,
}

pub struct Updater {
    pub trust: Trust,
    /// The folder `bdf.exe` runs from.
    pub app_dir: PathBuf,
    /// Where the check cache and backoff are kept.
    pub state_dir: PathBuf,
    pub current: Version,
    /// Where releases come from (overridable in tests).
    pub feed: Option<String>,
}

impl Updater {
    /// The updater for this build, for the copy in `app_dir`.
    pub fn new(app_dir: PathBuf, state_dir: PathBuf) -> Updater {
        let trust = Trust::build();
        #[cfg(feature = "net")]
        let feed = crate::feed::feed_url(&trust);
        #[cfg(not(feature = "net"))]
        let feed = None;
        Updater { trust, app_dir, state_dir, current: Version::current(), feed }
    }

    /// The updater for the running executable's folder, with the state kept where [`layout::state_dir`] says.
    pub fn for_current_exe() -> Result<Updater> {
        let exe = std::env::current_exe().map_err(|e| UpdateError::Io { what: "couldn't find the running program".into(), source: e })?;
        let app_dir = exe.parent().ok_or_else(|| UpdateError::Unmanaged("the program has no folder".into()))?.to_path_buf();
        let state_dir = layout::state_dir(&app_dir);
        Ok(Updater::new(app_dir, state_dir))
    }

    fn load(&self) -> State {
        State::load(&self.state_dir)
    }

    fn save(&self, state: &State) {
        if let Err(e) = state.save(&self.state_dir) {
            log::warn!("couldn't save the update state in {}: {e}", self.state_dir.display());
        }
    }

    /// The release on offer from what is cached (verified again now), unless it was skipped.
    fn offer(&self, state: &State) -> Option<(SignedManifest, Available)> {
        let release = state.cached_release(&self.trust)?;
        let (os, arch) = this_platform();
        match release.assess(self.current, state.floor(), os, arch, self.trust.channel()) {
            Ok(Assessment::Available(a)) if state.skipped_version() != Some(a.version) => Some((release, a)),
            _ => None,
        }
    }

    /// Status, from disk only.
    pub fn status(&self) -> Status {
        let state = self.load();
        let (managed, managed_note) = match layout::detect(&self.app_dir) {
            Ok(_) => (true, None),
            Err(e) => (false, Some(e.to_string())),
        };
        let offer = if self.trust.is_configured() { self.offer(&state) } else { None };
        let previous_version =
            std::fs::read(layout::previous_dir(&self.app_dir).join(PACKAGE_FILE)).ok().and_then(|b| PackageInfo::parse(&b).ok()).map(|i| i.version);
        Status {
            current: self.current.to_string(),
            channel: self.trust.channel().name().into(),
            dev_build: self.trust.channel() == crate::Channel::Dev,
            trust: if self.trust.is_configured() { "configured" } else { "missing" }.into(),
            key_id: self.trust.key_id().map(str::to_string),
            managed,
            managed_note,
            app_dir: self.app_dir.display().to_string(),
            ready_to_install: offer.as_ref().is_some_and(|(_, a)| self.prepared().is_some_and(|p| p.version == a.version)),
            update_available: offer.as_ref().map(|(_, a)| Offer::from(a)),
            last_check: (state.last_success > 0).then_some(state.last_success),
            next_check: state.next_check(),
            last_error: (!state.last_error.is_empty()).then(|| state.last_error.clone()),
            rejected: None,
            previous_version,
            last_result: apply::peek_result(&self.app_dir),
            feed: self.feed.clone(),
        }
    }

    /// Ask the feed. An automatic check (`manual == false`) does nothing when one is not yet due; a manual
    /// check always asks. Failures are remembered for the backoff and returned.
    #[cfg(feature = "net")]
    pub fn check(&self, manual: bool) -> Result<Status> {
        if !self.trust.is_configured() {
            return Err(UpdateError::NoTrustRoot);
        }
        let mut state = self.load();
        let now = now_unix();
        if !manual && !state.is_due(now) {
            return Ok(self.status());
        }
        let Some(feed) = self.feed.clone() else {
            return Err(UpdateError::Network("this development build has no update feed (set BDF_UPDATE_FEED)".into()));
        };
        let release = match crate::feed::fetch_release(&self.trust, &feed) {
            Ok(r) => r,
            Err(e) => {
                state.record_failure(now, &e.to_string());
                self.save(&state);
                return Err(e);
            }
        };
        let (os, arch) = this_platform();
        let mut rejected = None;
        match release.assess(self.current, state.floor(), os, arch, self.trust.channel()) {
            Ok(_) => state.record_success(now, &release),
            Err(UpdateError::NotApplicable(why)) => {
                state.record_checked(now);
                rejected = Some(why);
            }
            Err(e) => {
                state.record_failure(now, &e.to_string());
                self.save(&state);
                return Err(e);
            }
        }
        self.save(&state);
        let mut status = self.status();
        status.rejected = rejected;
        Ok(status)
    }

    /// Download the release on offer and verify it completely. A package already downloaded and still valid
    /// is reused. `progress(received, total)` returns `false` to cancel.
    #[cfg(feature = "net")]
    pub fn prepare(&self, progress: &mut dyn FnMut(u64, u64) -> bool) -> Result<Prepared> {
        if !self.trust.is_configured() {
            return Err(UpdateError::NoTrustRoot);
        }
        layout::detect(&self.app_dir)?;
        let state = self.load();
        let (release, available) =
            self.offer(&state).ok_or_else(|| UpdateError::NotApplicable("no update is available; check for updates first".into()))?;
        layout::ensure_writable(&self.app_dir)?;
        let (os, arch) = this_platform();
        let dir = layout::downloads_dir(&self.app_dir);
        std::fs::create_dir_all(&dir).map_err(crate::io_err("couldn't create the download folder"))?;
        let zip = dir.join(format!("bdf-{}-{os}-{arch}-portable.zip", available.version));
        let release_path = dir.join("release.json");
        // Other versions' leftovers are ours to remove.
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                if e.path() != zip && e.path() != release_path {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
        let reusable = zip.is_file() && package::verify_zip(&zip, &available.package, available.version, os, arch).is_ok();
        if !reusable {
            let _ = std::fs::remove_file(&zip);
            crate::feed::download(&available.package, self.trust.channel(), &zip, progress)?;
            if let Err(e) = package::verify_zip(&zip, &available.package, available.version, os, arch) {
                let _ = std::fs::remove_file(&zip);
                return Err(e);
            }
        }
        apply::write_atomic(&release_path, &release.envelope()).map_err(crate::io_err("couldn't save the release information"))?;
        Ok(Prepared { version: available.version, zip, release: release_path })
    }

    /// The package that is downloaded and verified for the release on offer, if there is one. Verifies it
    /// again (hash and contents), so a file changed since the download is not offered.
    pub fn prepared(&self) -> Option<Prepared> {
        let state = self.load();
        let (_, available) = self.offer(&state)?;
        let (os, arch) = this_platform();
        let dir = layout::downloads_dir(&self.app_dir);
        let zip = dir.join(format!("bdf-{}-{os}-{arch}-portable.zip", available.version));
        let release = dir.join("release.json");
        let on_disk = SignedManifest::from_envelope(&std::fs::read(&release).ok()?, &self.trust).ok()?;
        (on_disk.version == available.version && package::verify_zip(&zip, &available.package, available.version, os, arch).is_ok())
            .then_some(Prepared { version: available.version, zip, release })
    }

    /// Hand the prepared update to a copy of the helper and return: the caller then closes BDF, and the helper
    /// applies the update once nothing runs from the folder, then starts BDF again with `reopen` (existing
    /// files only). Nothing is changed by this call.
    pub fn launch_apply(&self, reopen: &[PathBuf]) -> Result<()> {
        let prepared = self.prepared().ok_or_else(|| UpdateError::NotApplicable("no verified update is ready; download it first".into()))?;
        layout::detect(&self.app_dir)?;
        layout::ensure_writable(&self.app_dir)?;
        let mut args = vec![
            "apply".to_string(),
            "--app-dir".into(),
            self.app_dir.display().to_string(),
            "--release".into(),
            prepared.release.display().to_string(),
            "--zip".into(),
            prepared.zip.display().to_string(),
        ];
        self.push_restart(&mut args, reopen);
        self.spawn_helper(&args)
    }

    /// Hand a rollback to the helper (see [`Self::launch_apply`]).
    pub fn launch_rollback(&self, reopen: &[PathBuf]) -> Result<()> {
        layout::detect(&self.app_dir)?;
        if !layout::previous_dir(&self.app_dir).join(PACKAGE_FILE).is_file() {
            return Err(UpdateError::NotApplicable("there is no previous version to go back to".into()));
        }
        layout::ensure_writable(&self.app_dir)?;
        // The version being left is not offered again until a newer one appears.
        let mut state = self.load();
        state.skipped = Some(self.current.to_string());
        self.save(&state);
        let mut args = vec!["rollback".to_string(), "--app-dir".into(), self.app_dir.display().to_string()];
        self.push_restart(&mut args, reopen);
        self.spawn_helper(&args)
    }

    fn push_restart(&self, args: &mut Vec<String>, reopen: &[PathBuf]) {
        args.push("--restart".into());
        args.push(self.app_dir.join(if cfg!(windows) { "bdf.exe" } else { "bdf" }).display().to_string());
        for p in reopen.iter().filter(|p| p.is_absolute() && p.is_file()).take(MAX_REOPEN) {
            args.push(p.display().to_string());
        }
    }

    fn spawn_helper(&self, args: &[String]) -> Result<()> {
        let source = self.app_dir.join(HELPER_NAME);
        if !source.is_file() {
            return Err(UpdateError::Unmanaged(format!(
                "{HELPER_NAME} is missing from this folder, so BDF cannot apply updates; extract the portable package again"
            )));
        }
        let helper_dir = layout::update_dir(&self.app_dir).join("helper");
        std::fs::create_dir_all(&helper_dir).map_err(crate::io_err("couldn't create the helper folder"))?;
        // Old copies (finished runs) are removed; one still running is locked and stays.
        if let Ok(entries) = std::fs::read_dir(&helper_dir) {
            for e in entries.flatten() {
                let _ = std::fs::remove_file(e.path());
            }
        }
        let copy = helper_dir.join(format!("bdf-updater-{}{}", std::process::id(), if cfg!(windows) { ".exe" } else { "" }));
        std::fs::copy(&source, &copy).map_err(crate::io_err("couldn't copy the updater"))?;
        let mut command = std::process::Command::new(&copy);
        command
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .current_dir(&helper_dir);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW: it must outlive the program that started it.
            command.creation_flags(0x0000_0008 | 0x0000_0200 | 0x0800_0000);
        }
        command.spawn().map(|_| ()).map_err(crate::io_err("couldn't start the updater"))
    }

    /// What the last update attempt reported, once: reading removes it.
    pub fn take_last_result(&self) -> Option<LastResult> {
        apply::take_result(&self.app_dir)
    }

    /// Undo an interrupted update, if one was interrupted (cheap when none was).
    pub fn recover(&self) -> Result<apply::Recovery> {
        apply::recover(&self.app_dir)
    }
}

/// The folder an app that was started from `exe` runs from.
pub fn app_dir_of(exe: &Path) -> Option<PathBuf> {
    exe.parent().map(Path::to_path_buf)
}

#[cfg(all(test, feature = "net"))]
mod tests {
    use super::*;
    use crate::Channel;
    use crate::feed::tests::{TestServer, serve_with};
    use crate::manifest::make_envelope;
    use crate::manifest::testing::manifest_json;
    use crate::testing::{build, installed_copy, sign};
    use crate::trust::testkeys;

    const V0: &[(&str, &[u8])] = &[("bdf.exe", b"exe-0"), ("bdf-updater.exe", b"helper-0"), ("portable.txt", b"p")];
    const V1: &[(&str, &[u8])] = &[("bdf.exe", b"exe-1"), ("bdf-updater.exe", b"helper-1"), ("portable.txt", b"p")];

    /// A dev-channel release of `files` served over loopback at `/release.json`; the package is served
    /// corrupted when asked.
    fn release_server(root: &Path, version: &str, files: &[(&str, &[u8])], corrupt: bool) -> TestServer {
        let pkg = build(root, &format!("srv-{version}.zip"), version, files);
        let mut zip = std::fs::read(&pkg.zip).unwrap();
        if corrupt {
            let mid = zip.len() / 2;
            zip[mid] ^= 0x42;
        }
        let path = format!("/v{version}/bdf-{version}-windows-x64-portable.zip");
        serve_with(|base| {
            let json = manifest_json(version, "dev", &pkg.entry.sha256, pkg.entry.size)
                .replace("https://github.com/btecme/bdf/releases/download/v", &format!("{base}/v"));
            vec![("/release.json".into(), 200, make_envelope(json.as_bytes(), &sign(1, json.as_bytes()))), (path, 200, zip)]
        })
    }

    fn updater(root: &Path, app: &Path, current: Version, feed: Option<String>) -> Updater {
        Updater { trust: testkeys::trust(1, Channel::Dev), app_dir: app.to_path_buf(), state_dir: root.join("state"), current, feed }
    }

    #[test]
    fn a_build_without_a_trust_root_offers_nothing_and_refuses_to_check_or_download() {
        let root = tempfile::tempdir().unwrap();
        let app = installed_copy(root.path(), "0.8.0", V0);
        let u = Updater {
            trust: Trust::none(Channel::Stable),
            app_dir: app,
            state_dir: root.path().join("state"),
            current: Version(0, 8, 0),
            feed: Some("https://example.test/x".into()),
        };
        let s = u.status();
        assert_eq!((s.trust.as_str(), s.update_available.is_none(), s.managed), ("missing", true, true));
        assert!(matches!(u.check(true), Err(UpdateError::NoTrustRoot)));
        assert!(matches!(u.prepare(&mut |_, _| true), Err(UpdateError::NoTrustRoot)));
        assert!(matches!(u.launch_apply(&[]), Err(UpdateError::NotApplicable(_))));
    }

    #[test]
    fn check_download_and_prepare_work_and_the_offer_survives_going_offline() {
        let root = tempfile::tempdir().unwrap();
        let app = installed_copy(root.path(), "0.8.0", V0);
        let srv_root = tempfile::tempdir().unwrap();
        let server = release_server(srv_root.path(), "0.8.1", V1, false);
        let u = updater(root.path(), &app, Version(0, 8, 0), Some(format!("{}/release.json", server.base)));
        let before = u.status();
        assert!(before.update_available.is_none() && before.last_check.is_none() && before.managed && before.dev_build);

        let after = u.check(true).unwrap();
        let offer = after.update_available.clone().expect("a newer release is on offer");
        assert_eq!((offer.version.as_str(), offer.notes.as_str()), ("0.8.1", "Fixes."));
        assert!(!after.ready_to_install && after.last_check.is_some());

        let mut last = (0, 0);
        let prepared = u
            .prepare(&mut |got, total| {
                last = (got, total);
                true
            })
            .unwrap();
        assert_eq!(prepared.version, Version(0, 8, 1));
        assert_eq!(last.0, last.1, "progress ran to the end");
        assert!(u.status().ready_to_install && u.prepared().is_some());

        // Offline: the server is gone, yet the offer, its readiness and the downloaded package are still there.
        drop(server);
        let again = updater(root.path(), &app, Version(0, 8, 0), Some("http://127.0.0.1:9/release.json".into()));
        let offline = again.status();
        assert_eq!(offline.update_available.as_ref().map(|o| o.version.as_str()), Some("0.8.1"));
        assert!(offline.ready_to_install);
        // Preparing again reuses the verified package without the network.
        assert_eq!(again.prepare(&mut |_, _| true).unwrap().version, Version(0, 8, 1));
    }

    #[test]
    fn a_tampered_package_is_discarded_and_never_becomes_ready() {
        let root = tempfile::tempdir().unwrap();
        let app = installed_copy(root.path(), "0.8.0", V0);
        let srv_root = tempfile::tempdir().unwrap();
        let server = release_server(srv_root.path(), "0.8.1", V1, true);
        let u = updater(root.path(), &app, Version(0, 8, 0), Some(format!("{}/release.json", server.base)));
        u.check(true).unwrap();
        assert!(matches!(u.prepare(&mut |_, _| true), Err(UpdateError::Tampered(_))));
        assert!(!u.status().ready_to_install && u.prepared().is_none());
        let leftovers: Vec<_> =
            std::fs::read_dir(layout::downloads_dir(&app)).map(|d| d.flatten().map(|e| e.file_name()).collect()).unwrap_or_default();
        assert!(leftovers.iter().all(|n| n != "bdf-0.8.1-windows-x64-portable.zip"), "{leftovers:?}");
    }

    #[test]
    fn a_failed_check_backs_off_and_an_automatic_check_waits_while_a_manual_one_does_not() {
        let root = tempfile::tempdir().unwrap();
        let app = installed_copy(root.path(), "0.8.0", V0);
        let u = updater(root.path(), &app, Version(0, 8, 0), Some("http://127.0.0.1:9/release.json".into()));
        assert!(matches!(u.check(false), Err(UpdateError::Network(_))), "never checked: an automatic check runs");
        let s = u.status();
        assert!(s.last_error.is_some() && s.next_check > crate::now_unix() + 3000, "backed off for about an hour");
        // The next automatic check does nothing and says nothing; a manual one tries (and fails) again.
        assert!(u.check(false).unwrap().update_available.is_none());
        assert!(matches!(u.check(true), Err(UpdateError::Network(_))));
        assert_eq!(State::load(&u.state_dir).failures, 2);
    }

    #[test]
    fn an_older_or_replayed_release_is_rejected_and_never_replaces_what_was_offered() {
        let root = tempfile::tempdir().unwrap();
        let app = installed_copy(root.path(), "0.8.0", V0);
        let srv_root = tempfile::tempdir().unwrap();
        // The feed is behind this copy: nothing is offered and the reason is reported.
        let old = release_server(srv_root.path(), "0.8.1", V1, false);
        let ahead = updater(root.path(), &app, Version(0, 9, 0), Some(format!("{}/release.json", old.base)));
        let s = ahead.check(true).unwrap();
        assert!(s.update_available.is_none() && s.rejected.as_deref().is_some_and(|r| r.contains("older")), "{s:?}");
        // A newer release is seen, then the feed serves an older, genuinely signed one: the newer offer stands.
        let newer = release_server(srv_root.path(), "0.8.2", V1, false);
        let u = updater(root.path(), &app, Version(0, 8, 0), Some(format!("{}/release.json", newer.base)));
        assert_eq!(u.check(true).unwrap().update_available.map(|o| o.version), Some("0.8.2".into()));
        let replay = updater(root.path(), &app, Version(0, 8, 0), Some(format!("{}/release.json", old.base)));
        let s = replay.check(true).unwrap();
        assert!(s.rejected.is_some(), "{s:?}");
        assert_eq!(s.update_available.map(|o| o.version), Some("0.8.2".into()), "the replay did not replace the cached release");
    }

    #[test]
    fn an_unmanaged_copy_says_so_and_applies_nothing() {
        let root = tempfile::tempdir().unwrap();
        let app = root.path().join("hand-built");
        std::fs::create_dir_all(&app).unwrap();
        let u = updater(root.path(), &app, Version(0, 8, 0), None);
        let s = u.status();
        assert!(!s.managed && s.managed_note.as_deref().is_some_and(|n| n.contains("portable package")), "{s:?}");
        assert!(u.launch_apply(&[]).is_err() && u.launch_rollback(&[]).is_err());
        assert!(u.prepare(&mut |_, _| true).is_err());
    }
}
