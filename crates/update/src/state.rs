//! What the updater remembers between runs: when it last asked, how often that failed, the highest release
//! it ever verified, and the last verified release information (so the Update button can show offline).
//!
//! The file is the updater's own and is never trusted: it is read leniently (a damaged file is an empty
//! state, never an error), the cached release information is **verified again** against the build's trust
//! root every time it is used, and nothing in it can raise the privileges of anything.
//!
//! **Schedule.** After a success the next automatic check is 12 hours later. After a failure the wait doubles
//! from one hour (1, 2, 4, 8, 16) up to 24 hours, so an offline or blocked machine asks a handful of times a
//! day at most and never in a tight loop. A manual check ignores the schedule.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::manifest::SignedManifest;
use crate::{Trust, Version};

pub const STATE_FILE: &str = "update-state.json";
/// Seconds between automatic checks after a success.
pub const CHECK_INTERVAL: u64 = 12 * 3600;
const BACKOFF_BASE: u64 = 3600;
const BACKOFF_MAX: u64 = 24 * 3600;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub schema: u32,
    /// Unix time of the last attempt, successful or not.
    pub last_attempt: u64,
    /// Unix time of the last successful check.
    pub last_success: u64,
    /// Failed attempts in a row.
    pub failures: u32,
    /// The last failure, for the status line (cleared by a success).
    pub last_error: String,
    /// The highest release version ever verified: an older genuinely signed release is a replay.
    pub highest_seen: Option<String>,
    /// A version the person went back from; not offered again (a newer one is).
    pub skipped: Option<String>,
    /// The last verified release information (the envelope text), re-verified before use.
    pub release: Option<String>,
}

impl State {
    /// The state saved in `dir`, or an empty one (missing, unreadable or damaged).
    pub fn load(dir: &Path) -> State {
        std::fs::read(dir.join(STATE_FILE)).ok().and_then(|b| serde_json::from_slice::<State>(&b).ok()).unwrap_or_default()
    }

    /// Save to `dir` (created if needed), atomically.
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let mut state = self.clone();
        state.schema = 1;
        let bytes = serde_json::to_vec_pretty(&state).map_err(std::io::Error::other)?;
        crate::apply::write_atomic(&dir.join(STATE_FILE), &bytes)
    }

    /// Seconds to wait after `failures` failures in a row.
    pub fn backoff(failures: u32) -> u64 {
        if failures == 0 {
            return 0;
        }
        BACKOFF_BASE.saturating_mul(1u64 << (failures - 1).min(10)).min(BACKOFF_MAX)
    }

    /// When the next automatic check is due (Unix time).
    pub fn next_check(&self) -> u64 {
        if self.failures > 0 {
            self.last_attempt.saturating_add(Self::backoff(self.failures))
        } else {
            self.last_success.saturating_add(CHECK_INTERVAL)
        }
    }

    /// Whether an automatic check should run now. A clock that moved backwards (the last attempt is in the
    /// future by more than a day) counts as due, so a wrong clock cannot switch checking off for good.
    pub fn is_due(&self, now: u64) -> bool {
        if self.last_attempt > now.saturating_add(24 * 3600) {
            return true;
        }
        now >= self.next_check()
    }

    pub fn record_success(&mut self, now: u64, release: &SignedManifest) {
        self.last_attempt = now;
        self.last_success = now;
        self.failures = 0;
        self.last_error.clear();
        if self.highest_seen.as_deref().and_then(Version::parse).is_none_or(|h| release.version > h) {
            self.highest_seen = Some(release.version.to_string());
        }
        self.release = String::from_utf8(release.envelope()).ok();
    }

    /// A check that reached the server but found nothing to offer (older or same release): the schedule moves on.
    pub fn record_checked(&mut self, now: u64) {
        self.last_attempt = now;
        self.last_success = now;
        self.failures = 0;
        self.last_error.clear();
    }

    pub fn record_failure(&mut self, now: u64, error: &str) {
        self.last_attempt = now;
        self.failures = self.failures.saturating_add(1);
        self.last_error = error.chars().filter(|c| !c.is_control()).take(300).collect();
    }

    /// The highest verified release version so far.
    pub fn floor(&self) -> Option<Version> {
        self.highest_seen.as_deref().and_then(Version::parse)
    }

    pub fn skipped_version(&self) -> Option<Version> {
        self.skipped.as_deref().and_then(Version::parse)
    }

    /// The cached release information, verified again now. A cache that no longer verifies (another key, a
    /// damaged file, a hand edit) is simply not there.
    pub fn cached_release(&self, trust: &Trust) -> Option<SignedManifest> {
        SignedManifest::from_envelope(self.release.as_deref()?.as_bytes(), trust).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Channel;
    use crate::manifest::testing::{SHA, manifest_json, signed};
    use crate::trust::testkeys;

    #[test]
    fn the_backoff_doubles_to_a_day_and_never_overflows() {
        let steps: Vec<u64> = (0..8).map(State::backoff).collect();
        assert_eq!(steps, [0, 3600, 7200, 14_400, 28_800, 57_600, 86_400, 86_400]);
        assert_eq!(State::backoff(u32::MAX), 86_400);
    }

    #[test]
    fn checks_are_spaced_after_success_and_backed_off_after_failure() {
        let mut s = State::default();
        assert!(s.is_due(1_000_000), "never checked: due");
        let m = signed(1, &manifest_json("0.8.1", "stable", SHA, 10), Channel::Stable).unwrap();
        s.record_success(1_000_000, &m);
        assert!(!s.is_due(1_000_000 + CHECK_INTERVAL - 1));
        assert!(s.is_due(1_000_000 + CHECK_INTERVAL));
        s.record_failure(2_000_000, "offline");
        assert_eq!(s.failures, 1);
        assert!(!s.is_due(2_000_000 + 3599));
        assert!(s.is_due(2_000_000 + 3600));
        s.record_failure(2_010_000, "offline");
        assert!(!s.is_due(2_010_000 + 7199));
        assert!(s.is_due(2_010_000 + 7200));
        s.record_checked(3_000_000);
        assert_eq!((s.failures, s.last_error.as_str()), (0, ""));
    }

    #[test]
    fn a_clock_set_back_does_not_switch_checking_off() {
        let s = State { last_attempt: 9_999_999_999, last_success: 9_999_999_999, ..State::default() };
        assert!(s.is_due(1_000_000));
    }

    #[test]
    fn the_floor_only_rises_and_the_cached_release_is_verified_again() {
        let trust = testkeys::trust(1, Channel::Stable);
        let mut s = State::default();
        let m2 = signed(1, &manifest_json("0.8.2", "stable", SHA, 10), Channel::Stable).unwrap();
        let m1 = signed(1, &manifest_json("0.8.1", "stable", SHA, 10), Channel::Stable).unwrap();
        s.record_success(10, &m2);
        s.record_success(20, &m1);
        assert_eq!(s.floor(), Some(Version(0, 8, 2)), "a replay of an older release never lowers the floor");
        assert_eq!(s.cached_release(&trust).unwrap().version, Version(0, 8, 1));
        // The same cache under another build's key is nothing.
        assert!(s.cached_release(&testkeys::trust(2, Channel::Stable)).is_none());
        assert!(s.cached_release(&Trust::none(Channel::Stable)).is_none());
        // A hand-edited cache is nothing.
        s.release = s.release.map(|r| r.replace("signature\": \"", "signature\": \"00"));
        assert!(s.cached_release(&trust).is_none());
    }

    #[test]
    fn a_damaged_state_file_is_an_empty_state() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(STATE_FILE), b"{ not json").unwrap();
        assert_eq!(State::load(dir.path()), State::default());
        let mut s = State::default();
        s.record_failure(5, "x\u{0}y\nz");
        s.save(dir.path()).unwrap();
        let back = State::load(dir.path());
        assert_eq!(back.failures, 1);
        assert_eq!(back.last_error, "xyz");
        // Unknown and missing fields are tolerated.
        std::fs::write(dir.path().join(STATE_FILE), br#"{"failures": 3, "future": true}"#).unwrap();
        assert_eq!(State::load(dir.path()).failures, 3);
    }
}
