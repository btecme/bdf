//! Bounded waits for the document-open worker.
//!
//! Since #1493 a desktop open (a path, a drop, a password retry, a recovery snapshot, a followed
//! link) is read and prepared on a worker thread; the UI polls it once per frame and commits the
//! result on the frame that sees it. A headless frame takes microseconds, so "run two or three
//! frames, then assert" races the worker and fails on a fast frame loop.
//!
//! A test waits here for the *specific* outcome it is about: the document that should have
//! opened, the password prompt that should have appeared, or the notice that refuses a file. Each
//! wait pumps frames, yields to the worker between them, stops at a monotonic deadline and fails
//! with the state it was looking at. A wait is a ceiling, not a delay: it returns on the frame the
//! outcome arrives. Do not wait for "any view": several tests already have a document open.
//!
//! Tests use `h.run_steps(n)` only for effects that are deterministic frame by frame (a click,
//! a pick that is already queued, an inline `open_bytes`); never for a worker's answer.

#![allow(dead_code)] // each test binary compiles this module and uses a subset

use std::time::{Duration, Instant};

use egui_kittest::Harness;
use pdfcraft_ui_egui::PdfCraftApp;

pub type Shell = Harness<'static, PdfCraftApp>;

/// The longest one wait may last. It only bounds a hang; a working open takes milliseconds.
pub const DEADLINE: Duration = Duration::from_secs(30);

/// How long to yield to the worker between frames when the outcome has not arrived.
const POLL: Duration = Duration::from_millis(2);

/// Pump frames until `done` is true for the app, or fail with `what` and a snapshot of the app.
pub fn wait_until(h: &mut Shell, what: &str, mut done: impl FnMut(&PdfCraftApp) -> bool) {
    pump(h, DEADLINE, what, &mut |h| done(h.state()));
}

/// [`wait_until`] with an explicit deadline (for testing the helper itself).
pub fn wait_until_for(h: &mut Shell, deadline: Duration, what: &str, mut done: impl FnMut(&PdfCraftApp) -> bool) {
    pump(h, deadline, what, &mut |h| done(h.state()));
}

/// Pump frames until `done` is true for the whole shell: for an outcome that is on screen
/// (a prompt drawn, a button present) rather than in the app's state.
pub fn wait_until_shown(h: &mut Shell, what: &str, mut done: impl FnMut(&Shell) -> bool) {
    pump(h, DEADLINE, what, &mut done);
}

fn pump(h: &mut Shell, deadline: Duration, what: &str, done: &mut dyn FnMut(&Shell) -> bool) {
    let start = Instant::now();
    let mut frames = 0u32;
    loop {
        h.step();
        frames = frames.saturating_add(1);
        if done(h) {
            return;
        }
        if start.elapsed() >= deadline {
            panic!(
                "timed out after {:.1} s and {frames} frames waiting for: {what}
{}",
                start.elapsed().as_secs_f32(),
                snapshot(h.state())
            );
        }
        std::thread::sleep(POLL);
    }
}

/// The tab showing the document called `name`, if one is open.
pub fn view_named(app: &PdfCraftApp, name: &str) -> Option<usize> {
    app.views.iter().position(|v| app.session.get(v.id).is_some_and(|d| d.name == name))
}

/// A password prompt is up for the document called `name`, with or without the wrong-password
/// error the prompt shows after a failed retry.
pub fn password_prompt_for(app: &PdfCraftApp, name: &str, with_error: bool) -> bool {
    app.password_prompt.as_ref().is_some_and(|p| p.name == name && p.error.is_some() == with_error)
}

/// A notice that starts with `prefix` is showing.
pub fn toast_starts_with(app: &PdfCraftApp, prefix: &str) -> bool {
    app.toast.as_ref().is_some_and(|(message, _)| message.starts_with(prefix))
}

/// A notice that contains `text` is showing.
pub fn toast_contains(app: &PdfCraftApp, text: &str) -> bool {
    app.toast.as_ref().is_some_and(|(message, _)| message.contains(text))
}

/// What a timed-out wait was looking at. Lists the state an open can end in: a tab, a prompt, a
/// dialog, or a notice.
pub fn snapshot(app: &PdfCraftApp) -> String {
    let tabs: Vec<String> = app
        .views
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let doc = app.session.get(v.id);
            format!(
                "#{i}{} {:?} path={:?} pages={}",
                if app.active == Some(i) { " (active)" } else { "" },
                doc.map(|d| d.name.as_str()),
                doc.and_then(|d| d.path.as_deref()),
                doc.map_or(0, |d| d.info.pages.len())
            )
        })
        .collect();
    format!(
        "tabs: [{}]\npassword prompt: {:?}\ndialog: {:?}\nnotice: {:?}\npending absolute link: {}\nstartup inbox: {} queued\nrecoverable: {}",
        tabs.join("; "),
        app.password_prompt.as_ref().map(|p| (p.name.as_str(), p.error.as_deref())),
        app.dialog,
        app.toast.as_ref().map(|(m, _)| m.as_str()),
        app.pending_remote_link.is_some(),
        app.startup_inbox.lock().map_or(0, |q| q.len()),
        app.recoverable.len()
    )
}
