//! The default-PDF-app shortcut and the Preferences gear (BUILD-018): the shortcut shows until Windows confirms BDF as the
//! default, never because Settings was merely opened, is read from a cache and refreshed on focus, and fits narrow windows
//! and scaled displays; the gear opens Preferences with or without a document.
//!
//! A fake host stands in for the registry, so nothing here reads or writes the real one.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use pdfcraft_platform::windows_integration::{INSTALLED_PROG_ID, PROG_ID, Status};
use pdfcraft_ui_egui::default_app::{Chosen, DefaultAppHost};
use pdfcraft_ui_egui::{Dialog, PdfCraftApp};

const SHORTCUT: &str = "Make BDF the default PDF app";

struct Fake {
    status: Mutex<Status>,
    reads: AtomicUsize,
    chosen: AtomicUsize,
    choice: Mutex<Result<Chosen, String>>,
}

impl Fake {
    fn new(status: Status) -> Arc<Fake> {
        Arc::new(Fake {
            status: Mutex::new(status),
            reads: AtomicUsize::new(0),
            chosen: AtomicUsize::new(0),
            choice: Mutex::new(Ok(Chosen::SettingsOpened)),
        })
    }
    fn set(&self, status: Status) {
        *self.status.lock().unwrap() = status;
    }
}

impl DefaultAppHost for Fake {
    fn status(&self) -> Status {
        self.reads.fetch_add(1, Ordering::Relaxed);
        self.status.lock().unwrap().clone()
    }
    fn choose(&self) -> Result<Chosen, String> {
        self.chosen.fetch_add(1, Ordering::Relaxed);
        self.choice.lock().unwrap().clone()
    }
    fn remove(&self) -> Result<(), String> {
        Ok(())
    }
}

/// Windows integration exists, nothing is registered.
fn unregistered() -> Status {
    Status { supported: true, ..Status::default() }
}

/// This copy is registered and chosen in Settings.
fn confirmed_default() -> Status {
    Status { supported: true, registered: true, this_copy: true, is_default: true, default_progid: Some(PROG_ID.into()), ..Status::default() }
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

fn harness(host: Option<Arc<Fake>>, size: egui::Vec2, scale: f32, with_document: bool) -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(size).with_pixels_per_point(scale).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.auto_update_check = false;
        app.default_app_host = host.clone().map(|h| h as Arc<dyn DefaultAppHost>);
        app
    });
    h.run_steps(6);
    if with_document {
        h.state_mut().open_bytes("doc.pdf", None, pdf()).unwrap();
        h.run_steps(6);
    }
    h
}

fn standard(host: &Arc<Fake>, with_document: bool) -> Harness<'static, PdfCraftApp> {
    harness(Some(host.clone()), egui::vec2(1400.0, 900.0), 1.0, with_document)
}

/// The window loses the focus (the person is in Windows Settings) and gets it back.
fn leave_and_return(h: &mut Harness<'static, PdfCraftApp>) {
    h.input_mut().focused = false;
    h.run_steps(3);
    h.input_mut().focused = true;
    h.run_steps(3);
}

fn shown(h: &Harness<'static, PdfCraftApp>) -> bool {
    h.query_by_label(SHORTCUT).is_some()
}

#[test]
fn the_shortcut_shows_until_windows_confirms_bdf_as_the_default_with_or_without_a_document() {
    let registered_not_chosen = Status { supported: true, registered: true, this_copy: true, ..Status::default() };
    let another_copy = Status { this_copy: false, is_default: true, default_progid: Some(PROG_ID.into()), ..registered_not_chosen.clone() };
    let another_app = Status { default_progid: Some("AcroExch.Document.DC".into()), ..registered_not_chosen.clone() };
    let installed_bdf = Status { supported: true, default_progid: Some(INSTALLED_PROG_ID.into()), ..Status::default() };
    for with_document in [false, true] {
        for (name, status, want) in [
            ("nothing registered", unregistered(), true),
            ("registered, not chosen (Settings alone is not the choice)", registered_not_chosen.clone(), true),
            ("another copy's registration is the default", another_copy.clone(), true),
            ("another application is the default", another_app.clone(), true),
            ("Windows reports this copy as the default", confirmed_default(), false),
            ("the installed BDF is the default", installed_bdf.clone(), false),
            ("integration unsupported here (not Windows)", Status::default(), false),
        ] {
            let h = standard(&Fake::new(status), with_document);
            assert_eq!(shown(&h), want, "{name}, document open: {with_document}");
        }
    }
    let h = harness(None, egui::vec2(1400.0, 900.0), 1.0, false);
    assert!(!shown(&h), "no host (the web build, tests): no shortcut");
}

#[test]
fn the_registry_is_read_on_events_not_on_every_frame() {
    let fake = Fake::new(unregistered());
    let mut h = standard(&fake, true);
    assert_eq!(fake.reads.load(Ordering::Relaxed), 1, "once, at start");
    h.run_steps(60);
    assert_eq!(fake.reads.load(Ordering::Relaxed), 1, "sixty more frames read nothing");
    leave_and_return(&mut h);
    assert_eq!(fake.reads.load(Ordering::Relaxed), 2, "regaining the focus reads it again, once");
    h.run_steps(30);
    assert_eq!(fake.reads.load(Ordering::Relaxed), 2);
}

#[test]
fn clicking_registers_and_opens_settings_but_the_shortcut_stays_until_windows_confirms() {
    for with_document in [false, true] {
        let fake = Fake::new(unregistered());
        let mut h = standard(&fake, with_document);
        h.get_by_label(SHORTCUT).click();
        h.run_steps(4);
        assert_eq!(fake.chosen.load(Ordering::Relaxed), 1, "the click ran the shared registration + Settings action once");
        h.get_by_label_contains("BDF is registered. Choose it in Windows");
        // Registered now, but opening Settings is not the choice.
        fake.set(Status { supported: true, registered: true, this_copy: true, ..Status::default() });
        h.run_steps(4);
        assert!(shown(&h), "registered, Settings open, nothing chosen: still on offer (document open: {with_document})");
        // The person cancels out of Settings: BDF regains the focus, Windows still says another app.
        leave_and_return(&mut h);
        assert!(shown(&h), "a cancelled choice keeps the shortcut (document open: {with_document})");
        // The person chooses BDF: the next time BDF has the focus, Windows says so.
        fake.set(confirmed_default());
        leave_and_return(&mut h);
        assert!(!shown(&h), "Windows confirmed BDF: the shortcut goes (document open: {with_document})");
    }
}

#[test]
fn a_failed_registration_says_why_and_keeps_the_shortcut() {
    let fake = Fake::new(unregistered());
    *fake.choice.lock().unwrap() = Err("Windows refused the change: access denied".into());
    let mut h = standard(&fake, false);
    h.get_by_label(SHORTCUT).click();
    h.run_steps(4);
    h.get_by_label_contains("Windows refused the change: access denied");
    leave_and_return(&mut h);
    assert!(shown(&h));
}

#[test]
fn when_settings_cannot_be_opened_it_says_so_and_keeps_the_shortcut() {
    let fake = Fake::new(unregistered());
    *fake.choice.lock().unwrap() = Ok(Chosen::SettingsFailed("no shell".into()));
    let mut h = standard(&fake, false);
    h.get_by_label(SHORTCUT).click();
    h.run_steps(4);
    h.get_by_label_contains("Windows Settings did not open: no shell");
    assert!(shown(&h));
}

#[test]
fn preferences_shares_the_action_and_follows_the_cached_status() {
    let fake = Fake::new(unregistered());
    let mut h = standard(&fake, false);
    h.state_mut().dialog = Some(Dialog::Preferences);
    h.run_steps(60);
    assert_eq!(fake.reads.load(Ordering::Relaxed), 1, "an open Preferences dialog does not read the registry every frame");
    // Far down the scrolling dialog: invoke the button as assistive technology would.
    h.get_by_label("Make BDF my default PDF app").click_accesskit();
    h.run_steps(4);
    assert_eq!(fake.chosen.load(Ordering::Relaxed), 1, "the same action as the shortcut");
    // The registry is the same one for both: the dialog and the shortcut agree once Windows confirms.
    fake.set(confirmed_default());
    leave_and_return(&mut h);
    assert!(!shown(&h));
    h.get_by_label_contains("is your default PDF app");
}

#[test]
fn the_gear_opens_preferences_with_or_without_a_document() {
    for with_document in [false, true] {
        let mut h = harness(None, egui::vec2(1400.0, 900.0), 1.0, with_document);
        assert!(h.state().dialog.is_none());
        let gear = h.get_by_label_contains("Preferences (");
        assert_eq!(gear.accesskit_node().role(), egui::accesskit::Role::Button, "a real button, reachable by keyboard and screen reader");
        gear.click();
        h.run_steps(4);
        assert_eq!(h.state().dialog, Some(Dialog::Preferences), "document open: {with_document}");
    }
}

#[test]
fn the_gear_sits_between_document_properties_and_save_and_its_tooltip_names_the_shortcut() {
    let h = harness(None, egui::vec2(1400.0, 900.0), 1.0, true);
    let x = |label: &str| h.get_by_label_contains(label).rect().center().x;
    let (info, gear, save) = (x("Document properties"), x("Preferences ("), x("Save ("));
    assert!(info < gear && gear < save, "order left to right: properties {info}, preferences {gear}, save {save}");
    // The label carries the key the shortcut really uses on this system (Ctrl+, on Windows and Linux).
    let label = h.get_by_label_contains("Preferences (").accesskit_node().label().unwrap_or_default();
    assert!(label.starts_with("Preferences (") && label.ends_with(",)"), "{label}");
}

#[test]
fn the_shortcut_fits_narrow_windows_and_scaled_displays_and_never_covers_the_toolbar() {
    for (width, height, scale) in [(1400.0, 900.0, 1.0), (1000.0, 700.0, 1.25), (640.0, 600.0, 1.5), (480.0, 600.0, 1.75), (360.0, 640.0, 2.0)] {
        for with_document in [false, true] {
            let fake = Fake::new(unregistered());
            let mut h = harness(Some(fake.clone()), egui::vec2(width, height), scale, with_document);
            let window = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, height));
            let rect = h.get_by_label(SHORTCUT).rect();
            let what = format!("{width}x{height} at {scale}x, document open: {with_document}");
            assert!(window.contains_rect(rect), "{what}: the shortcut {rect:?} is inside the window");
            assert!(rect.height() >= 20.0 && rect.width() >= 40.0, "{what}: a usable target, {rect:?}");
            assert!(rect.right() > window.right() - 40.0, "{what}: at the right edge, {rect:?}");
            assert!(rect.bottom() > window.bottom() - 40.0, "{what}: at the bottom, {rect:?}");
            // The gear is in the toolbar above, and the shortcut does not overlap it or the strip it sits in.
            let gear = h.get_by_label_contains("Preferences (").rect();
            assert!(!gear.intersects(rect), "{what}: the shortcut {rect:?} covers the gear {gear:?}");
            h.get_by_label(SHORTCUT).click();
            h.run_steps(3);
            assert_eq!(fake.chosen.load(Ordering::Relaxed), 1, "{what}: clickable");
        }
    }
}

#[test]
fn the_shortcut_overlaps_no_other_control_and_leaves_nothing_covered() {
    use egui::accesskit::Role;
    for with_document in [false, true] {
        let h = standard(&Fake::new(unregistered()), with_document);
        let strip = h.get_by_label(SHORTCUT).rect();
        assert!(strip.height() <= 26.0, "a compact strip: {strip:?}");
        for role in [Role::Button, Role::Tab, Role::Link, Role::TextInput, Role::CheckBox] {
            for other in h.query_all_by_role(role) {
                let r = other.rect();
                if r == strip || r.width() <= 0.0 || r.height() <= 0.0 {
                    continue;
                }
                assert!(
                    !r.intersects(strip),
                    "document open {with_document}: {role:?} {:?} at {r:?} is covered by the shortcut at {strip:?}",
                    other.accesskit_node().label()
                );
            }
        }
    }
}
