//! Help ▸ Check for updates (issue #28, reworked in BUILD-018), with a stand-in update host (no network).
//! The button, the install flow and the automatic check are in `tests/updates.rs`.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::PdfCraftApp;
use pdfcraft_ui_egui::updates::{LastResult, Status, UpdateHost, is_newer};
use pdfcraft_update::service::Offer;

fn status(offer: Option<&str>) -> Status {
    Status {
        current: env!("CARGO_PKG_VERSION").into(),
        channel: "stable".into(),
        dev_build: false,
        trust: "configured".into(),
        key_id: None,
        managed: true,
        managed_note: None,
        app_dir: "C:\\BDF".into(),
        update_available: offer.map(|v| Offer { version: v.into(), notes: String::new(), notes_url: None, published: String::new(), size: 1 }),
        ready_to_install: false,
        last_check: None,
        next_check: 0,
        last_error: None,
        rejected: None,
        previous_version: None,
        last_result: None,
        feed: None,
        check_state: "up_to_date".into(),
    }
}

struct Host {
    answer: Result<Status, String>,
    calls: Arc<AtomicUsize>,
    /// What `status()` says the last check found (`check_state`).
    state: &'static str,
}

impl UpdateHost for Host {
    fn status(&self) -> Status {
        Status { check_state: self.state.into(), ..status(None) }
    }
    fn check(&self, _manual: bool) -> Result<Status, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.answer.clone()
    }
    fn download(&self, _progress: &mut dyn FnMut(u64, u64) -> bool) -> Result<(), String> {
        Ok(())
    }
    fn install(&self, _reopen: &[PathBuf]) -> Result<(), String> {
        Ok(())
    }
    fn take_last_result(&self) -> Option<LastResult> {
        None
    }
}

fn harness(answer: Result<Status, String>, calls: Arc<AtomicUsize>) -> Harness<'static, PdfCraftApp> {
    harness_in_state(answer, calls, "up_to_date")
}

fn harness_in_state(answer: Result<Status, String>, calls: Arc<AtomicUsize>, state: &'static str) -> Harness<'static, PdfCraftApp> {
    Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        // Background checks are covered in tests/updates.rs; here only the person's own check runs.
        app.auto_update_check = false;
        app.update_host = Some(Arc::new(Host { answer: answer.clone(), calls: calls.clone(), state }));
        app
    })
}

/// Run frames until the background check has reported (or give up).
fn settle(h: &mut Harness<'static, PdfCraftApp>) {
    for _ in 0..200 {
        h.run_steps(2);
        if h.query_by_label_contains("Checking for a newer version").is_none() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    h.run_steps(2);
}

#[test]
fn versions_compare_by_number() {
    assert!(is_newer("v0.2.0", "0.1.1"));
    assert!(is_newer("v0.1.10", "0.1.9"));
    assert!(is_newer("1", "0.9.9"));
    assert!(!is_newer("v0.1.1", "0.1.1"));
    assert!(!is_newer("v0.1.0", "0.1.1"));
    assert!(!is_newer("v0.1.1-beta.2", "0.1.1"), "suffixes are ignored");
    assert!(!is_newer("nightly", "0.1.1"), "a tag that isn't a version is never newer");
    assert!(!is_newer("v1.2.3.4", "0.1.1"));
    assert!(!is_newer("v99999999999999999999.0.0", "0.1.1"), "out of range");
}

#[test]
fn a_newer_release_is_offered_for_download() {
    let mut h = harness(Ok(status(Some("99.0.0"))), Arc::default());
    h.state_mut().execute("help.check_updates");
    settle(&mut h);
    h.get_by_label_contains("BDF 99.0.0 is available");
    h.get_by_label("Download update");
    h.get_by_label("Later").click();
    h.run_steps(3);
    assert!(h.query_by_label_contains("is available").is_none(), "Later closes the dialog");
}

#[test]
fn an_up_to_date_or_failed_check_says_so() {
    let mut h = harness(Ok(status(None)), Arc::default());
    h.state_mut().execute("help.check_updates");
    settle(&mut h);
    h.get_by_label_contains("is up to date");
    assert!(h.query_by_label("Download update").is_none());

    let mut h = harness(Err("couldn't reach the update server".into()), Arc::default());
    h.state_mut().execute("help.check_updates");
    settle(&mut h);
    h.get_by_label_contains("Couldn't update: couldn't reach the update server");
}

/// A check that cannot even start (a development build with no feed) is a state, not a failure: the dialog says the
/// state once, without the error that the attempt returned.
#[test]
fn a_check_that_could_not_start_shows_its_state_and_not_its_error() {
    let mut h = harness_in_state(Err("couldn't reach the update server".into()), Arc::default(), "no_feed");
    h.state_mut().execute("help.check_updates");
    settle(&mut h);
    h.get_by_label_contains("no update feed configured");
    assert!(h.query_by_label_contains("Couldn't update").is_none(), "the error is not repeated");
}

#[test]
fn with_automatic_checks_off_nothing_is_asked_until_the_user_checks() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut h = harness(Ok(status(Some("99.0.0"))), calls.clone());
    settle(&mut h);
    h.run_steps(20);
    assert_eq!(calls.load(Ordering::SeqCst), 0, "no check without the person asking, when automatic checks are off");
    assert!(h.query_by_label_contains("is available").is_none());
    h.state_mut().execute("help.check_updates");
    settle(&mut h);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    h.get_by_label_contains("BDF 99.0.0 is available");
}
