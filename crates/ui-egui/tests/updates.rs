//! The Update button, the b-tec Built link and the install flow in the window (BUILD-018), over a fake update host:
//! the real network and updater are exercised by the update crate's tests and the packaged-build checks.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_engine::Edit;
use pdfcraft_ui_egui::PdfCraftApp;
use pdfcraft_ui_egui::updates::{LastResult, Status, UpdateHost};
use pdfcraft_update::service::Offer;

fn status(offer: bool, managed: bool, trusted: bool, ready: bool) -> Status {
    Status {
        current: "0.8.0".into(),
        channel: "stable".into(),
        dev_build: false,
        trust: if trusted { "configured" } else { "missing" }.into(),
        key_id: None,
        managed,
        managed_note: (!managed).then(|| "this copy was not installed from a portable package".to_string()),
        app_dir: "C:\\BDF".into(),
        update_available: offer.then(|| Offer {
            version: "9.9.9".into(),
            notes: "Fixes and a faster open.".into(),
            notes_url: None,
            published: String::new(),
            size: 1000,
        }),
        ready_to_install: ready,
        last_check: None,
        next_check: 0,
        last_error: None,
        rejected: None,
        previous_version: None,
        last_result: None,
        feed: None,
    }
}

struct Fake {
    status: Mutex<Status>,
    downloads: AtomicUsize,
    installs: Mutex<Vec<Vec<PathBuf>>>,
    checks: AtomicUsize,
    note: Mutex<Option<LastResult>>,
}

impl Fake {
    fn new(s: Status) -> Arc<Fake> {
        Arc::new(Fake {
            status: Mutex::new(s),
            downloads: AtomicUsize::new(0),
            installs: Mutex::new(Vec::new()),
            checks: AtomicUsize::new(0),
            note: Mutex::new(None),
        })
    }
}

impl UpdateHost for Fake {
    fn status(&self) -> Status {
        self.status.lock().unwrap().clone()
    }
    fn check(&self, _manual: bool) -> Result<Status, String> {
        self.checks.fetch_add(1, Ordering::Relaxed);
        Ok(self.status())
    }
    fn download(&self, progress: &mut dyn FnMut(u64, u64) -> bool) -> Result<(), String> {
        self.downloads.fetch_add(1, Ordering::Relaxed);
        progress(1000, 1000);
        self.status.lock().unwrap().ready_to_install = true;
        Ok(())
    }
    fn install(&self, reopen: &[PathBuf]) -> Result<(), String> {
        self.installs.lock().unwrap().push(reopen.to_vec());
        Ok(())
    }
    fn take_last_result(&self) -> Option<LastResult> {
        self.note.lock().unwrap().take()
    }
}

fn pdf() -> Vec<u8> {
    let objs: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Contents 4 0 R /Resources << >> >>".to_vec(),
        b"<< /Length 0 >>\nstream\n\nendstream".to_vec(),
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

fn harness(host: Option<Arc<Fake>>) -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.auto_update_check = false;
        app.update_host = host.clone().map(|h| h as Arc<dyn UpdateHost>);
        app
    });
    h.run_steps(6);
    h
}

#[test]
fn the_b_tec_built_mark_opens_the_bdf_page_and_no_button_shows_without_an_update() {
    let mut h = harness(Some(Fake::new(status(false, true, true, false))));
    assert!(h.query_by_label_contains("Update to BDF").is_none(), "no newer release: no Update button");
    h.get_by_label("b-tec Built").click();
    h.run_steps(3);
    assert_eq!(h.state().last_opened_url.as_deref(), Some("https://www.b-tec.org/bdf"));
}

#[test]
fn the_button_appears_only_for_an_applicable_trusted_update() {
    for (managed, trusted, shown) in [(true, true, true), (false, true, false), (true, false, false)] {
        let h = harness(Some(Fake::new(status(true, managed, trusted, false))));
        assert_eq!(h.query_by_label("Update to BDF 9.9.9").is_some(), shown, "managed {managed}, trusted {trusted}");
    }
    let h = harness(None);
    assert!(h.query_by_label_contains("Update to BDF").is_none(), "no host (the browser build): no button");
}

#[test]
fn update_downloads_then_closing_installs_and_hands_over_the_open_documents() {
    let fake = Fake::new(status(true, true, true, false));
    let mut h = harness(Some(fake.clone()));
    h.get_by_label("Update to BDF 9.9.9").click();
    h.run_steps(4);
    h.get_by_label_contains("BDF 9.9.9 is available.");
    h.get_by_label_contains("Fixes and a faster open.");
    h.get_by_label("Download update").click();
    for _ in 0..40 {
        h.run_steps(2);
        if h.query_by_label("Close BDF and update").is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(fake.downloads.load(Ordering::Relaxed), 1);
    assert!(fake.installs.lock().unwrap().is_empty(), "downloading installs nothing");
    h.get_by_label("Close BDF and update").click();
    h.run_steps(4);
    assert_eq!(fake.installs.lock().unwrap().len(), 1, "with nothing unsaved, BDF closes and starts the updater");
}

#[test]
fn unsaved_work_is_asked_about_first_and_cancelling_withdraws_the_update() {
    let fake = Fake::new(status(true, true, true, true));
    let mut h = harness(Some(fake.clone()));
    h.state_mut().open_bytes("doc.pdf", None, pdf()).unwrap();
    h.run_steps(4);
    let id = h.state().views[0].id;
    h.state_mut().session.apply(id, Edit::SetInfo { key: "Title".into(), value: "changed".into() }).unwrap();
    h.run_steps(2);
    h.get_by_label("Update to BDF 9.9.9").click();
    h.run_steps(3);
    h.get_by_label("Close BDF and update").click();
    h.run_steps(4);
    h.get_by_label_contains("Save changes to");
    assert!(fake.installs.lock().unwrap().is_empty(), "the updater does not start while the question is open");
    h.get_by_label("Cancel").click();
    h.run_steps(4);
    assert!(fake.installs.lock().unwrap().is_empty(), "cancelling means no update");
    assert_eq!(h.state().views.len(), 1, "the document is still open and still unsaved");
    assert!(h.state().session.get(id).is_some_and(|d| d.dirty));
    // The update can be started again later.
    h.get_by_label("Update to BDF 9.9.9").click();
    h.run_steps(3);
    h.get_by_label("Close BDF and update");
}

#[test]
fn later_leaves_everything_as_it_was() {
    let fake = Fake::new(status(true, true, true, true));
    let mut h = harness(Some(fake.clone()));
    h.get_by_label("Update to BDF 9.9.9").click();
    h.run_steps(3);
    h.get_by_label("Later").click();
    h.run_steps(3);
    assert!(h.query_by_label("Close BDF and update").is_none(), "the dialog closed");
    assert!(fake.installs.lock().unwrap().is_empty() && fake.downloads.load(Ordering::Relaxed) == 0);
    assert!(h.query_by_label("Update to BDF 9.9.9").is_some(), "the button stays for later");
}

#[test]
fn automatic_checks_are_off_when_turned_off_and_a_manual_check_always_asks() {
    let fake = Fake::new(status(false, true, true, false));
    let mut h = harness(Some(fake.clone()));
    h.run_steps(10);
    assert_eq!(fake.checks.load(Ordering::Relaxed), 0, "automatic checks are off in this harness");
    h.state_mut().check_for_updates();
    for _ in 0..40 {
        h.run_steps(2);
        if fake.checks.load(Ordering::Relaxed) > 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(fake.checks.load(Ordering::Relaxed), 1);
}

#[test]
fn the_auto_check_preference_is_saved_and_restored() {
    let mut app = PdfCraftApp::new();
    assert!(app.auto_update_check, "on by default");
    app.auto_update_check = false;
    let saved = app.persist();
    let mut fresh = PdfCraftApp::new();
    fresh.restore(&saved);
    assert!(!fresh.auto_update_check);
}

#[test]
fn the_portable_markers_match_the_update_crates() {
    assert_eq!(pdfcraft_ui_egui::portable::MARKERS, pdfcraft_update::layout::PORTABLE_MARKERS);
}
