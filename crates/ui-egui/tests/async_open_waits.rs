//! The bounded wait (`tests/support`) that tests of the document-open worker rely on. It must
//! wait for the outcome a test names, fail at its monotonic deadline with the state it saw, and
//! leave opening itself asynchronous: these tests drive the real worker, not an inline open.

use std::time::{Duration, Instant};

use egui_kittest::Harness;
use pdfcraft_ui_egui::PdfCraftApp;

mod support;
use support::{DEADLINE, Shell, view_named, wait_until, wait_until_for};

/// A one-page document with a proper xref table.
fn fixture() -> Vec<u8> {
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 300] >>".to_string(),
        "<< /Type /Page /Parent 2 0 R >>".to_string(),
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

/// A shell past its first frame, so a path opens through the worker, with `open` already open.
fn shell(open: &[&str]) -> Shell {
    let open: Vec<String> = open.iter().map(|name| name.to_string()).collect();
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        for name in &open {
            app.open_bytes(name, None, fixture()).unwrap();
        }
        app
    });
    h.run_steps(3);
    h
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("pdfcraft-async-open-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn opening_a_path_returns_at_once_and_the_wait_ends_when_the_document_arrives() {
    let dir = scratch("path");
    let file = dir.join("late.pdf");
    std::fs::write(&file, fixture()).unwrap();
    let mut h = shell(&[]);
    h.state_mut().open_path(&file.to_string_lossy());
    // The call only schedules the open. Nothing is committed until a frame sees the worker's answer.
    assert!(h.state().views.is_empty(), "the open does not block the caller");
    let started = Instant::now();
    wait_until(&mut h, "late.pdf opens as the first tab", |app| view_named(app, "late.pdf") == Some(0));
    assert!(started.elapsed() < DEADLINE, "a working open ends the wait long before its ceiling");
    assert_eq!(h.state().views.len(), 1);
    assert_eq!(h.state().active, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_wait_names_the_intended_document_when_another_is_already_open() {
    let dir = scratch("second");
    let file = dir.join("second.pdf");
    std::fs::write(&file, fixture()).unwrap();
    let mut h = shell(&["first.pdf"]);
    h.state_mut().open_path(&file.to_string_lossy());
    // "Any view" would already be satisfied by first.pdf; the wait must be for the new document.
    assert_eq!(h.state().views.len(), 1);
    assert_eq!(view_named(h.state(), "second.pdf"), None);
    wait_until(&mut h, "second.pdf opens as the second tab", |app| view_named(app, "second.pdf") == Some(1));
    assert_eq!(h.state().views.len(), 2);
    assert_eq!(view_named(h.state(), "first.pdf"), Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_wait_for_an_outcome_that_never_comes_fails_at_its_deadline_and_shows_the_state() {
    let mut h = shell(&["only.pdf"]);
    let deadline = Duration::from_millis(300);
    let started = Instant::now();
    let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        wait_until_for(&mut h, deadline, "a tab named never.pdf", |app| view_named(app, "never.pdf").is_some());
    }))
    .expect_err("an outcome that never arrives must fail the wait, not pass or hang");
    let elapsed = started.elapsed();
    assert!(elapsed >= deadline, "the wait ran to its deadline: {elapsed:?}");
    assert!(elapsed < Duration::from_secs(10), "and stopped soon after it: {elapsed:?}");
    let message = failure.downcast_ref::<String>().cloned().unwrap_or_default();
    for expected in ["timed out", "a tab named never.pdf", "tabs: [#0 (active) Some(\"only.pdf\")", "password prompt: None", "notice:"] {
        assert!(message.contains(expected), "the failure shows `{expected}`:\n{message}");
    }
}
