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
        check_state: "up_to_date".into(),
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
    harness_with(host, false)
}

fn harness_with(host: Option<Arc<Fake>>, auto: bool) -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.auto_update_check = auto;
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

/// The dialog promises "reopen your documents". A document whose unsaved-work question was answered (Save, or Don't save) is
/// closed by that answer before the updater starts, so the list to reopen has to be taken when the install begins. Found with
/// the real window (qualification, 2026-11-07): after Don't save the restarted BDF opened nothing.
#[test]
fn documents_whose_unsaved_work_question_was_answered_are_still_reopened_after_the_update() {
    for answer in ["Don't save", "Save"] {
        let dir = std::env::temp_dir().join(format!("bdf-reopen-{}-{}", std::process::id(), answer.len()));
        std::fs::create_dir_all(&dir).unwrap();
        let clean = dir.join("clean.pdf");
        let dirty = dir.join("dirty.pdf");
        std::fs::write(&clean, pdf()).unwrap();
        std::fs::write(&dirty, pdf()).unwrap();
        let fake = Fake::new(status(true, true, true, true));
        let mut h = harness(Some(fake.clone()));
        h.state_mut().open_bytes("clean.pdf", Some(clean.to_string_lossy().into_owned()), pdf()).unwrap();
        h.state_mut().open_bytes("dirty.pdf", Some(dirty.to_string_lossy().into_owned()), pdf()).unwrap();
        h.run_steps(4);
        let id = h.state().views[1].id;
        h.state_mut().session.apply(id, Edit::SetInfo { key: "Title".into(), value: "changed".into() }).unwrap();
        h.run_steps(2);
        h.get_by_label("Update to BDF 9.9.9").click();
        h.run_steps(3);
        h.get_by_label("Close BDF and update").click();
        h.run_steps(4);
        h.get_by_label_contains("Save changes to");
        h.get_by_label(answer).click();
        h.run_steps(8);
        let installs = fake.installs.lock().unwrap().clone();
        assert_eq!(installs.len(), 1, "{answer}: the updater starts once the question is answered");
        let mut names: Vec<String> = installs[0].iter().filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).collect();
        names.sort();
        assert_eq!(names, ["clean.pdf", "dirty.pdf"], "{answer}: both documents are handed to the updater to reopen");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// A document with no file of its own, edited, and sitting at the "Save changes to ...?" question of an update that has
/// been asked to close BDF.
fn unnamed_documents_at_the_save_question(fake: &Arc<Fake>, count: usize) -> Harness<'static, PdfCraftApp> {
    let mut h = harness(Some(fake.clone()));
    for n in 0..count {
        h.state_mut().open_bytes(&format!("untitled-{n}.pdf"), None, pdf()).unwrap();
    }
    h.run_steps(4);
    let ids: Vec<_> = h.state().views.iter().map(|v| v.id).collect();
    for id in ids {
        h.state_mut().session.apply(id, Edit::SetInfo { key: "Title".into(), value: "synthetic change".into() }).unwrap();
    }
    h.run_steps(2);
    h.get_by_label("Update to BDF 9.9.9").click();
    h.run_steps(3);
    h.get_by_label("Close BDF and update").click();
    h.run_steps(4);
    h.get_by_label_contains("Save changes to");
    h
}

fn wait_for_install(h: &mut Harness<'static, PdfCraftApp>, fake: &Fake) {
    for _ in 0..120 {
        h.run_steps(2);
        if !fake.installs.lock().unwrap().is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

struct Remove(PathBuf);
impl Drop for Remove {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn scratch(tag: &str) -> (PathBuf, Remove) {
    let out = std::env::temp_dir().join(format!("bdf-update-save-{tag}-{}.pdf", std::process::id()));
    let _ = std::fs::remove_file(&out);
    (out.clone(), Remove(out))
}

/// R18-5 (independent review, 2026-11-08). Save for a document that has no file asks where, and the answer arrives on a later
/// frame; that gap was read as the person cancelling, so the update was dropped although the file was written.
#[test]
fn saving_an_unnamed_document_through_the_picker_carries_on_with_the_update_and_reopens_it() {
    let (out, _guard) = scratch("picker");
    let fake = Fake::new(status(true, true, true, true));
    let mut h = unnamed_documents_at_the_save_question(&fake, 1);
    h.state_mut().pick_override = Some(vec![out.to_string_lossy().into_owned()]);
    h.get_by_label("Save").click();
    wait_for_install(&mut h, &fake);
    assert!(out.is_file(), "the newly named document was saved");
    let installs = fake.installs.lock().unwrap().clone();
    assert_eq!(installs.len(), 1, "the update proceeds after saving");
    assert_eq!(installs[0], vec![out.clone()], "the file that was just saved is the one handed to the updater to reopen");
    assert!(h.query_by_label_contains("Update postponed").is_none());
}

/// The same, where the destination is known at once: the update ran, but the reopen list had been taken before the document
/// had a path.
#[test]
fn saving_an_unnamed_document_directly_hands_the_new_file_to_the_updater() {
    let (out, _guard) = scratch("direct");
    let fake = Fake::new(status(true, true, true, true));
    let mut h = unnamed_documents_at_the_save_question(&fake, 1);
    h.state_mut().save_override = Some(out.to_string_lossy().into_owned());
    h.get_by_label("Save").click();
    wait_for_install(&mut h, &fake);
    assert!(out.is_file());
    let installs = fake.installs.lock().unwrap().clone();
    assert_eq!(installs.len(), 1);
    assert_eq!(installs[0], vec![out.clone()]);
}

/// Several unnamed documents: each is asked about in turn, each one's own destination is remembered, and the updater starts
/// once, after the last.
#[test]
fn several_unnamed_documents_are_each_saved_and_all_reopened() {
    let (first, _g1) = scratch("multi-a");
    let (second, _g2) = scratch("multi-b");
    let fake = Fake::new(status(true, true, true, true));
    let mut h = unnamed_documents_at_the_save_question(&fake, 2);
    h.state_mut().pick_override = Some(vec![first.to_string_lossy().into_owned()]);
    h.get_by_label("Save").click();
    for _ in 0..120 {
        h.run_steps(2);
        if first.is_file() && h.query_by_label_contains("Save changes to").is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(first.is_file(), "the first document was saved");
    h.get_by_label_contains("Save changes to");
    assert!(fake.installs.lock().unwrap().is_empty(), "the updater waits for the second question");
    h.state_mut().pick_override = Some(vec![second.to_string_lossy().into_owned()]);
    h.get_by_label("Save").click();
    wait_for_install(&mut h, &fake);
    assert!(second.is_file());
    let installs = fake.installs.lock().unwrap().clone();
    assert_eq!(installs.len(), 1, "the updater starts once");
    let mut got = installs[0].clone();
    got.sort();
    let mut want = vec![first.clone(), second.clone()];
    want.sort();
    assert_eq!(got, want);
}

/// Cancelling the Save As dialog is a real cancellation: nothing is written, nothing closes, the update is withdrawn.
#[test]
fn cancelling_the_save_as_dialog_withdraws_the_update_and_keeps_the_document() {
    let fake = Fake::new(status(true, true, true, true));
    let mut h = unnamed_documents_at_the_save_question(&fake, 1);
    h.state_mut().pick_override = Some(Vec::new());
    h.get_by_label("Save").click();
    h.run_steps(12);
    assert!(fake.installs.lock().unwrap().is_empty(), "no update after a cancelled Save As");
    assert_eq!(h.state().views.len(), 1, "the document is still open");
    assert!(h.state().session.get(h.state().views[0].id).is_some_and(|d| d.dirty), "and still unsaved");
    h.get_by_label_contains("Update postponed");
}

/// A Save As that fails (the folder is not there) is a cancellation too, and never counts as saved.
#[test]
fn a_failed_save_as_withdraws_the_update_and_keeps_the_document() {
    let missing = std::env::temp_dir().join(format!("bdf-no-such-folder-{}", std::process::id())).join("x.pdf");
    let fake = Fake::new(status(true, true, true, true));
    let mut h = unnamed_documents_at_the_save_question(&fake, 1);
    h.state_mut().pick_override = Some(vec![missing.to_string_lossy().into_owned()]);
    h.get_by_label("Save").click();
    h.run_steps(12);
    assert!(fake.installs.lock().unwrap().is_empty());
    assert_eq!(h.state().views.len(), 1);
    assert!(!missing.exists());
    h.get_by_label_contains("Update postponed");
}

/// A document that already has a file keeps it in the list when it is saved over: Save never goes to a new destination by
/// itself, and the file reopened is the original.
#[test]
fn saving_a_named_document_still_reopens_its_own_file() {
    let dir = std::env::temp_dir().join(format!("bdf-update-named-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("named.pdf");
    std::fs::write(&file, pdf()).unwrap();
    let fake = Fake::new(status(true, true, true, true));
    let mut h = harness(Some(fake.clone()));
    h.state_mut().open_bytes("named.pdf", Some(file.to_string_lossy().into_owned()), pdf()).unwrap();
    h.run_steps(4);
    let id = h.state().views[0].id;
    h.state_mut().session.apply(id, Edit::SetInfo { key: "Title".into(), value: "changed".into() }).unwrap();
    h.run_steps(2);
    h.get_by_label("Update to BDF 9.9.9").click();
    h.run_steps(3);
    h.get_by_label("Close BDF and update").click();
    h.run_steps(4);
    h.get_by_label_contains("Save changes to");
    h.get_by_label("Save").click();
    wait_for_install(&mut h, &fake);
    assert_eq!(fake.installs.lock().unwrap().clone(), vec![vec![file.clone()]]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// An existing document whose Save has to ask for a new destination (an email attachment's temporary copy): the new file is
/// reopened, not the temporary path the install began with.
#[test]
fn a_mail_attachments_save_to_a_new_place_replaces_its_temporary_path() {
    let root = std::env::temp_dir().join(format!("bdf-update-mail-{}", std::process::id()));
    let mail_dir = root.join("INetCache").join("Content.Outlook").join("ABC123");
    std::fs::create_dir_all(&mail_dir).unwrap();
    let temp_copy = mail_dir.join("attachment.pdf");
    std::fs::write(&temp_copy, pdf()).unwrap();
    let (out, _guard) = scratch("mail");
    let fake = Fake::new(status(true, true, true, true));
    let mut h = harness(Some(fake.clone()));
    h.state_mut().open_bytes("attachment.pdf", Some(temp_copy.to_string_lossy().into_owned()), pdf()).unwrap();
    h.run_steps(4);
    let id = h.state().views[0].id;
    h.state_mut().session.apply(id, Edit::SetInfo { key: "Title".into(), value: "changed".into() }).unwrap();
    h.run_steps(2);
    h.get_by_label("Update to BDF 9.9.9").click();
    h.run_steps(3);
    h.get_by_label("Close BDF and update").click();
    h.run_steps(4);
    h.get_by_label_contains("Save changes to");
    h.state_mut().pick_override = Some(vec![out.to_string_lossy().into_owned()]);
    h.get_by_label("Save").click();
    wait_for_install(&mut h, &fake);
    assert!(out.is_file(), "saved to the new place");
    assert_eq!(fake.installs.lock().unwrap().clone(), vec![vec![out.clone()]], "the new file, not the temporary copy");
    let _ = std::fs::remove_dir_all(&root);
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

#[test]
fn an_automatic_check_runs_on_its_own_a_few_seconds_after_start() {
    let fake = Fake::new(status(true, true, true, false));
    let mut h = harness_with(Some(fake.clone()), true);
    assert_eq!(fake.checks.load(Ordering::Relaxed), 0, "not at the very start: opening documents comes first");
    for _ in 0..600 {
        h.run_steps(2);
        if fake.checks.load(Ordering::Relaxed) > 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(fake.checks.load(Ordering::Relaxed), 1, "the first automatic check ran by itself");
}

/// Open the update dialog the way Help > Check for updates does and let its check finish.
fn open_dialog(h: &mut Harness<'static, PdfCraftApp>, fake: &Fake) {
    let before = fake.checks.load(Ordering::Relaxed);
    h.state_mut().check_for_updates();
    for _ in 0..60 {
        h.run_steps(2);
        if fake.checks.load(Ordering::Relaxed) > before {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    h.run_steps(4);
}

/// R18-1: a development build's offer is a test offer, and says so in the button, its tooltip and the dialog.
#[test]
fn an_offer_in_a_development_build_is_labelled_as_a_test_everywhere() {
    let mut s = status(true, true, true, false);
    s.dev_build = true;
    s.channel = "dev".into();
    s.check_state = "available".into();
    let fake = Fake::new(s);
    let mut h = harness(Some(fake.clone()));
    assert!(h.query_by_label("Update").is_none(), "the plain Update label is for real releases only");
    // The button's accessible name and tooltip are the test label (its visible text is "Test update").
    assert!(h.query_by_label("Update to BDF 9.9.9").is_none(), "no ordinary update wording for a test offer");
    h.get_by_label_contains("Test offer from a development feed").click();
    h.run_steps(4);
    h.get_by_label_contains("BDF 9.9.9 is available.");
    // The dialog repeats it above the offer, next to the existing development-build line.
    assert!(h.query_all_by_label_contains("Test offer from a development feed").count() >= 1);
    h.get_by_label_contains("Development build: updates come from a test feed.");
}

/// R18-1: each empty state says what is true. A dialog never says "up to date" about a check that did not happen.
#[test]
fn the_dialog_says_what_actually_happened_not_up_to_date_for_everything() {
    let say = |state: &str, error: Option<&str>, trusted: bool, dev: bool| -> Vec<String> {
        let mut s = status(false, true, trusted, false);
        s.dev_build = dev;
        s.check_state = state.into();
        s.last_error = error.map(str::to_string);
        let fake = Fake::new(s);
        let mut h = harness(Some(fake.clone()));
        open_dialog(&mut h, &fake);
        ["is up to date", "has not checked for updates yet", "no update feed configured", "cannot verify updates", "Couldn't update"]
            .iter()
            .filter(|needle| h.query_by_label_contains(needle).is_some())
            .map(|n| n.to_string())
            .collect()
    };
    assert_eq!(say("up_to_date", None, true, false), ["is up to date"]);
    assert_eq!(say("never_checked", None, true, false), ["has not checked for updates yet"]);
    assert_eq!(say("no_feed", None, true, true), ["no update feed configured"]);
    assert_eq!(say("not_configured", None, false, false), ["cannot verify updates"]);
    assert_eq!(say("failed", Some("couldn't reach the update server"), true, false), ["Couldn't update"]);
}
