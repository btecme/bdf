//! Regressions for three behaviours the asynchronous open (#1493) changed (BUILD-001 follow-up,
//! "D1" to "D3"). A native open is read and prepared on a worker and committed on a later frame,
//! so anything that used to run right after an open, or while a prompt was up, must now account
//! for an open that is still in flight. The worker is held at its prepare barrier
//! (`open_worker::prepare_gate`) so "in flight" is a state to assert, not a race.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;

use crate::{Dialog, PdfCraftApp, open_worker};

const HELD: Duration = Duration::from_secs(5);

fn fixture(pages: usize) -> Vec<u8> {
    let mut objects = vec!["<< /Type /Catalog /Pages 2 0 R >>".to_string()];
    let kids = (0..pages).map(|page| format!("{} 0 R", 4 + 2 * page)).collect::<Vec<_>>().join(" ");
    objects.push(format!("<< /Type /Pages /Kids [{kids}] /Count {pages} /MediaBox [0 0 200 300] >>"));
    objects.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into());
    for page in 0..pages {
        objects.push(format!("<< /Type /Page /Parent 2 0 R /Contents {} 0 R /Resources << /Font << /F1 3 0 R >> >> >>", 5 + 2 * page));
        let body = format!("BT /F1 24 Tf 20 150 Td (Page {}) Tj ET", page + 1);
        objects.push(format!("<< /Length {} >>\nstream\n{body}\nendstream", body.len()));
    }
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
    }
    let xref = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    bytes
}

fn encrypted(password: &str) -> Vec<u8> {
    use pdfcraft_engine::{Edit, Protection, Session};
    let mut session = Session::new();
    let id = session.open("e.pdf", None, Arc::new(fixture(1)), None).unwrap();
    session.apply(id, Edit::Protect(Protection { open_password: Some(password.into()), ..Default::default() })).unwrap();
    session.save_bytes(id).unwrap().as_ref().clone()
}

/// A shell past its first frame, so opening a path or an unlock goes through the worker.
fn shell() -> Harness<'static, PdfCraftApp> {
    let mut harness = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_eframe(|_creation| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").expect("language preference");
        app
    });
    harness.run_steps(3);
    harness
}

fn tab_names(app: &PdfCraftApp) -> Vec<String> {
    app.views.iter().filter_map(|view| app.session.get(view.id)).map(|doc| doc.name.clone()).collect()
}

fn asking(app: &PdfCraftApp) -> Option<(String, bool)> {
    app.password_prompt.as_ref().map(|prompt| (prompt.name.clone(), prompt.error.is_some()))
}

/// Pump frames until `done`, or fail after a monotonic deadline with what the app was showing.
fn wait(harness: &mut Harness<'static, PdfCraftApp>, what: &str, done: impl Fn(&PdfCraftApp) -> bool) {
    let start = Instant::now();
    loop {
        harness.step();
        let app = harness.state();
        if done(app) {
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "timed out waiting for: {what}\ntabs: {:?}\nprompt: {:?}\ndialog: {:?}\nnotice: {:?}",
            tab_names(app),
            asking(app),
            app.dialog,
            app.toast.as_ref().map(|(message, _)| message.as_str())
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("pdfcraft-open-regression-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn export_dialog(app: &PdfCraftApp) -> bool {
    matches!(app.dialog, Some(Dialog::Export(_)))
}

/// D1. A tool chosen with no document open asks for a file and runs on it once it is open.
/// `open_path` only schedules the open now, so the tool has to travel with the scheduled open.
#[test]
fn a_tool_chosen_with_no_document_runs_once_the_picked_file_has_opened() {
    let dir = scratch("d1");
    let path = dir.join("picked.pdf");
    std::fs::write(&path, fixture(2)).unwrap();
    let mut h = shell();
    let (gate, started, release, finished) = open_worker::prepare_gate();
    h.state_mut().open_prepare_gate = Some(gate);
    h.state_mut().pick_override = Some(vec![path.to_string_lossy().into_owned()]);
    h.state_mut().run_command("export.text");
    h.step(); // the pick is answered; the file is handed to the worker
    started.recv_timeout(HELD).expect("the worker reached its prepare barrier");
    assert!(h.state().views.is_empty(), "the open is in flight, not committed");
    assert!(h.state().dialog.is_none(), "the tool waits for the file");
    release.send(()).unwrap();
    finished.recv_timeout(HELD).expect("the worker finished preparing");
    h.step(); // the frame that commits the open
    assert_eq!(tab_names(h.state()), ["picked.pdf"]);
    assert_eq!(h.state().active, Some(0));
    assert!(export_dialog(h.state()), "the tool runs on the opened file: {:?}", h.state().dialog);
    let _ = std::fs::remove_dir_all(&dir);
}

/// D1, encrypted file: the tool waits for the password, then runs on the unlocked document.
#[test]
fn a_tool_chosen_with_no_document_runs_after_the_picked_file_is_unlocked() {
    let dir = scratch("d1-locked");
    let path = dir.join("locked.pdf");
    std::fs::write(&path, encrypted("pw")).unwrap();
    let mut h = shell();
    h.state_mut().pick_override = Some(vec![path.to_string_lossy().into_owned()]);
    h.state_mut().run_command("export.text");
    wait(&mut h, "locked.pdf asks for its password", |app| asking(app) == Some(("locked.pdf".into(), false)));
    assert!(h.state().dialog.is_none(), "the tool waits for the password");
    h.state_mut().submit_password(Some("pw".into()));
    wait(&mut h, "locked.pdf opens and the tool runs on it", |app| tab_names(app) == ["locked.pdf"] && export_dialog(app));
    assert_eq!(h.state().active, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

/// D1, refusal: the tool does not run on a file that fails to open.
#[test]
fn a_tool_chosen_with_no_document_does_not_run_when_the_picked_file_does_not_open() {
    let dir = scratch("d1-broken");
    let path = dir.join("broken.pdf");
    std::fs::write(&path, b"not a PDF").unwrap();
    let mut h = shell();
    h.state_mut().pick_override = Some(vec![path.to_string_lossy().into_owned()]);
    h.state_mut().run_command("export.text");
    wait(&mut h, "broken.pdf is refused", |app| app.toast.as_ref().is_some_and(|(message, _)| message.contains("broken.pdf")));
    h.run_steps(3);
    assert!(h.state().views.is_empty());
    assert!(h.state().dialog.is_none(), "no tool runs without a document: {:?}", h.state().dialog);
    let _ = std::fs::remove_dir_all(&dir);
}

/// D2. A late startup file waits for the interaction in progress. An unlock that has been
/// submitted but not finished is part of that interaction: the startup file must not open ahead
/// of the document being unlocked.
#[test]
fn a_startup_file_waits_behind_an_unlock_that_is_still_in_flight() {
    let mut h = shell();
    let (gate, started, release, finished) = open_worker::prepare_gate();
    h.state_mut().open_prepare_gate = Some(gate);
    h.state_mut().open_bytes("local-encrypted.pdf", None, encrypted("pw")).unwrap();
    h.state().startup_inbox.lock().unwrap().push(("late.pdf".into(), fixture(2)));
    h.run_steps(2);
    assert_eq!(asking(h.state()), Some(("local-encrypted.pdf".into(), false)));
    assert_eq!(h.state().startup_inbox.lock().unwrap().len(), 1, "the startup file waits for the prompt");
    h.state_mut().submit_password(Some("pw".into()));
    started.recv_timeout(HELD).expect("the worker reached its prepare barrier");
    h.run_steps(2);
    assert_eq!(h.state().startup_inbox.lock().unwrap().len(), 1, "it still waits while the unlock is in flight: {:?}", tab_names(h.state()));
    assert!(h.state().views.is_empty(), "the unlock is still in flight");
    release.send(()).unwrap();
    finished.recv_timeout(HELD).expect("the worker finished preparing");
    wait(&mut h, "the unlocked document, then the startup file", |app| tab_names(app) == ["local-encrypted.pdf", "late.pdf"]);
    assert_eq!(h.state().active, Some(0), "the document being unlocked stays in front");
    assert!(h.state().startup_inbox.lock().unwrap().is_empty());
}

/// D3. An unlock finishing must not clear a password prompt that belongs to another document:
/// the other file would neither open nor ask again.
#[test]
fn an_unlock_finishing_keeps_another_documents_password_prompt() {
    let mut h = shell();
    let (gate, started, release, finished) = open_worker::prepare_gate();
    h.state_mut().open_prepare_gate = Some(gate);
    h.state_mut().open_bytes("first.pdf", None, encrypted("pw-1")).unwrap();
    h.state_mut().submit_password(Some("pw-1".into()));
    started.recv_timeout(HELD).expect("the worker reached its prepare barrier");
    // While the first unlock is in flight, another encrypted file asks for its password.
    h.state_mut().open_bytes("second.pdf", None, encrypted("pw-2")).unwrap();
    assert_eq!(asking(h.state()), Some(("second.pdf".into(), false)));
    release.send(()).unwrap();
    finished.recv_timeout(HELD).expect("the worker finished preparing");
    wait(&mut h, "first.pdf opens", |app| tab_names(app) == ["first.pdf"]);
    assert_eq!(asking(h.state()), Some(("second.pdf".into(), false)), "second.pdf is still asking for its password");
    // And it can still be unlocked. The worker keeps its barrier for every job, so release the
    // second one in advance.
    release.send(()).unwrap();
    h.state_mut().submit_password(Some("pw-2".into()));
    wait(&mut h, "second.pdf opens", |app| tab_names(app) == ["first.pdf", "second.pdf"]);
}

/// D3, startup variant: a second encrypted startup file is neither lost nor left without a prompt.
#[test]
fn a_second_encrypted_startup_file_still_asks_after_the_first_unlock_finishes() {
    let mut h = shell();
    let (gate, started, release, finished) = open_worker::prepare_gate();
    h.state_mut().open_prepare_gate = Some(gate);
    h.state_mut().open_bytes("first.pdf", None, encrypted("pw-1")).unwrap();
    h.state().startup_inbox.lock().unwrap().push(("second.pdf".into(), encrypted("pw-2")));
    h.run_steps(2);
    h.state_mut().submit_password(Some("pw-1".into()));
    started.recv_timeout(HELD).expect("the worker reached its prepare barrier");
    h.run_steps(2);
    release.send(()).unwrap();
    finished.recv_timeout(HELD).expect("the worker finished preparing");
    wait(&mut h, "first.pdf opens and second.pdf asks for its password", |app| {
        tab_names(app) == ["first.pdf"] && asking(app) == Some(("second.pdf".into(), false))
    });
}
