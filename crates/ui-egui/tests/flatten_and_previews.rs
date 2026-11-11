//! The one-click Flatten at the foot of Edit, comments that open readable, and the Organize
//! pages preview resolution.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_engine::Edit;
use pdfcraft_engine::commands;
use pdfcraft_ui_egui::canvas::GridQuality;
use pdfcraft_ui_egui::{PdfCraftApp, RightPanel};

const LONG: &str = "First reviewer: please check the totals in the second table against the invoice before this goes out, because the \
    rounding differs and the finance team will ask about it on Monday morning.";
const SHORT: &str = "Second reviewer: agreed.";

/// One page with two sticky notes (long and short comments).
fn with_comments() -> Vec<u8> {
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 400] >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /Annots [4 0 R 5 0 R] >>".to_string(),
        format!("<< /Type /Annot /Subtype /Text /Rect [20 300 40 320] /Contents ({LONG}) /T (Ann) /Name /Comment >>"),
        format!("<< /Type /Annot /Subtype /Text /Rect [60 300 80 320] /Contents ({SHORT}) /T (Bo) /Name /Comment >>"),
    ];
    pdf(&objs)
}

/// Three plain pages with no comments or fields.
fn plain(pages: usize) -> Vec<u8> {
    let kids: Vec<String> = (0..pages).map(|i| format!("{} 0 R", 3 + i)).collect();
    let mut objs = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!("<< /Type /Pages /Kids [{}] /Count {pages} /MediaBox [0 0 200 300] >>", kids.join(" ")),
    ];
    objs.extend((0..pages).map(|_| "<< /Type /Page /Parent 2 0 R >>".to_string()));
    pdf(&objs)
}

fn pdf(objs: &[String]) -> Vec<u8> {
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

fn counts(app: &PdfCraftApp) -> (usize, usize) {
    let doc = app.session.get(app.views[app.active.unwrap()].id).unwrap();
    (doc.info.annotations.len(), doc.form.len())
}

// ---------------------------------------------------------------------------------- Flatten

#[test]
fn flatten_is_the_last_item_of_the_edit_menu_and_does_everything() {
    let last = commands::menu("Edit").last().expect("the Edit menu has items");
    assert_eq!((last.id, last.label), ("doc.flatten", "Flatten"), "it is the button at the bottom of Edit");
    // One undo step, and it takes both kinds with it: the form's fields and the comment.
    let mut app = PdfCraftApp::new();
    app.set_option("language", "en").unwrap();
    app.open_bytes("form.pdf", None, include_bytes!("data/form.pdf").to_vec()).unwrap();
    let shape = pdfcraft_engine::Shape::Rectangle { rect: [10.0, 10.0, 50.0, 50.0] };
    app.apply_edit(Edit::AddAnnotation(pdfcraft_engine::NewAnnotation {
        page: 0,
        style: pdfcraft_engine::Style::default_for(&shape),
        shape,
        contents: String::new(),
        author: String::new(),
    }));
    let (comments, fields) = counts(&app);
    assert!(comments > 0 && fields > 0, "the fixture has {comments} comments and {fields} fields");
    assert!(app.execute("doc.flatten"));
    assert_eq!(counts(&app), (0, 0), "no comments and no fields are left");
    let id = app.views[0].id;
    assert_eq!(app.session.get(id).unwrap().can_undo(), Some("Flatten"));
    app.undo();
    assert_eq!(counts(&app), (comments, fields), "undo brings both back");
}

#[test]
fn flatten_is_disabled_with_a_reason_when_there_is_nothing_to_flatten() {
    let mut app = PdfCraftApp::new();
    app.set_option("language", "en").unwrap();
    let spec = commands::command("doc.flatten").unwrap();
    let enabled = |app: &PdfCraftApp| commands::is_enabled(spec, &app.session, app.views.get(app.active.unwrap_or(0)).map(|v| v.id));
    assert!(!enabled(&app), "no document");
    app.open_bytes("plain.pdf", None, plain(2)).unwrap();
    assert!(!enabled(&app), "a document without comments or fields");
    assert!(!app.execute("doc.flatten"));
    let id = app.views[0].id;
    assert!(!app.session.get(id).unwrap().dirty, "nothing changed");
    // With a comment it can run.
    app.open_bytes("notes.pdf", None, with_comments()).unwrap();
    assert!(enabled(&app));
    assert!(app.execute("doc.flatten"));
    assert_eq!(counts(&app), (0, 0));
}

#[test]
fn the_edit_menu_shows_flatten_last() {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("notes.pdf", None, with_comments()).unwrap();
        app
    });
    h.run_steps(4);
    h.get_by_label("Menu").click();
    h.run_steps(2);
    h.get_by_label("Edit ⏵").click();
    h.run_steps(4);
    let flatten = h.get_by_label_contains("Flatten");
    let preferences = h.get_by_label_contains("Preferences…");
    assert!(flatten.rect().top() > preferences.rect().top(), "Flatten sits below Preferences, at the bottom of Edit");
    flatten.click();
    h.run_steps(3);
    assert_eq!(counts(h.state()), (0, 0), "choosing it flattens the open document");
}

// ------------------------------------------------------------------------------- comments

fn open_with(app: &mut PdfCraftApp, name: &str, bytes: Vec<u8>) {
    app.open_bytes(name, None, bytes).unwrap();
}

#[test]
fn a_document_with_comments_opens_on_them_unless_comments_were_closed() {
    // Another side panel was closed earlier: that doesn't keep the comments away.
    let mut app = PdfCraftApp::new();
    app.choose_right_panel(Some(RightPanel::Bookmarks));
    app.choose_right_panel(None);
    assert!(app.right_panel_closed && !app.comments_panel_closed);
    open_with(&mut app, "notes.pdf", with_comments());
    assert_eq!(app.right, Some(RightPanel::Comments), "the comments are open to read");
    // A document with none doesn't open a panel the person closed.
    let mut quiet = PdfCraftApp::new();
    quiet.choose_right_panel(Some(RightPanel::Bookmarks));
    quiet.choose_right_panel(None);
    open_with(&mut quiet, "plain.pdf", plain(1));
    assert_eq!(quiet.right, None);
    // Closing Comments itself is the choice that keeps them closed.
    let mut closed = PdfCraftApp::new();
    closed.choose_right_panel(Some(RightPanel::Comments));
    closed.choose_right_panel(None);
    assert!(closed.comments_panel_closed);
    open_with(&mut closed, "notes.pdf", with_comments());
    assert_eq!(closed.right, None, "closing Comments is still respected");
}

#[test]
fn every_comment_is_readable_in_full_without_clicking_it() {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        open_with(&mut app, "notes.pdf", with_comments());
        app
    });
    h.run_steps(6);
    assert_eq!(h.state().right, Some(RightPanel::Comments));
    assert!(h.state().views[0].comments.selected.is_none(), "nothing is selected: these are the unselected cards");
    let long = h.get_by_label(LONG).rect();
    let short = h.get_by_label(SHORT).rect();
    assert!(long.height() > 2.5 * short.height(), "the long comment wraps over several lines ({} against {})", long.height(), short.height());
    assert!(long.height() > 40.0, "and is not cut to a single line");
}

// ------------------------------------------------------------------- preview resolution

#[test]
fn preview_resolution_has_three_levels_and_low_is_the_default() {
    assert_eq!(GridQuality::default(), GridQuality::Low);
    assert_eq!(PdfCraftApp::new().view_defaults.grid_quality, GridQuality::Low);
    let levels: Vec<_> = GridQuality::ALL.iter().map(|q| (q.name(), q.index(), q.multiplier())).collect();
    assert_eq!(levels, [("low", 0, 1.0), ("medium", 1, 2.0), ("high", 2, 3.0)]);
    for q in GridQuality::ALL {
        assert_eq!(GridQuality::from_index(q.index()), q);
        assert_eq!(GridQuality::from_name(&q.name().to_uppercase()), Some(q));
    }
    assert_eq!(GridQuality::from_index(99), GridQuality::High, "a position past the end is the highest");
    assert_eq!(GridQuality::from_name("ultra"), None);
}

#[test]
fn preview_resolution_is_a_setting_that_survives_a_restart() {
    let mut app = PdfCraftApp::new();
    app.set_option("organize-quality", "high").unwrap();
    assert_eq!(app.view_defaults.grid_quality, GridQuality::High);
    assert!(app.set_option("organize-quality", "sharp").is_err(), "an unknown level is refused");
    assert_eq!(app.view_defaults.grid_quality, GridQuality::High, "and changes nothing");
    let mut again = PdfCraftApp::new();
    again.restore(&app.persist());
    assert_eq!(again.view_defaults.grid_quality, GridQuality::High);
    // A damaged value leaves the default.
    let mut damaged = PdfCraftApp::new();
    damaged.restore(r#"{"organize_quality":"???"}"#);
    assert_eq!(damaged.view_defaults.grid_quality, GridQuality::Low);
}

fn organize(pages: usize) -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("doc.pdf", None, plain(pages)).unwrap();
        app.set_option("organize", "on").unwrap();
        app
    });
    h.run_steps(4);
    h
}

#[test]
fn the_organize_toolbar_has_a_three_position_resolution_slider() {
    let mut h = organize(6);
    h.get_by_role_and_label(Role::Slider, "Preview resolution: Low");
    assert_eq!(h.state().view_defaults.grid_quality, GridQuality::Low, "the previews start at Low");
    // Keys move it one position at a time, as they do any slider.
    let press = |h: &mut Harness<'static, PdfCraftApp>, level: &str, key: egui::Key| {
        h.get_by_role_and_label(Role::Slider, &format!("Preview resolution: {level}")).focus();
        h.run_steps(1);
        h.key_press(key);
        h.run_steps(3);
    };
    press(&mut h, "Low", egui::Key::ArrowRight);
    h.get_by_role_and_label(Role::Slider, "Preview resolution: Medium");
    assert_eq!(h.state().view_defaults.grid_quality, GridQuality::Medium, "the choice is kept for the next document");
    press(&mut h, "Medium", egui::Key::ArrowRight);
    h.get_by_role_and_label(Role::Slider, "Preview resolution: High");
    press(&mut h, "High", egui::Key::ArrowRight);
    h.get_by_role_and_label(Role::Slider, "Preview resolution: High");
    assert_eq!(h.state().view_defaults.grid_quality, GridQuality::High, "there is no fourth position");
    for level in ["High", "Medium"] {
        press(&mut h, level, egui::Key::ArrowLeft);
    }
    h.get_by_role_and_label(Role::Slider, "Preview resolution: Low");
    assert_eq!(h.state().view_defaults.grid_quality, GridQuality::Low);
}

#[test]
fn a_higher_resolution_asks_for_larger_page_images_and_low_asks_for_none_extra() {
    let mut h = organize(6);
    // Settle: the thumbnails are rendered and nothing larger is wanted at Low.
    let settle = |h: &mut Harness<'static, PdfCraftApp>| {
        for _ in 0..100 {
            h.run_steps(2);
            if !h.state().render_pending() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    };
    settle(&mut h);
    let widest = |h: &Harness<'static, PdfCraftApp>| (0..6).filter_map(|p| h.state().views[0].grid_page_pixels(p)).max().unwrap_or(0);
    let low = widest(&h);
    h.state_mut().set_option("organize-quality", "high").unwrap();
    settle(&mut h);
    let high = widest(&h);
    assert!(high > 0, "at High the pages in view get a render of their own");
    assert!(high > low, "larger than at Low ({high} against {low})");
    h.state_mut().set_option("organize-quality", "low").unwrap();
    settle(&mut h);
    assert_eq!(widest(&h), low, "back at Low the extra renders are released");
}

// ------------------------------------------------------- what Flatten must never lose (PR 18 review)

/// A one-page form with a single field the way another program leaves it: filled in, with
/// `/NeedAppearances` and no stored appearance.
fn form_without_appearance(field: &str) -> Vec<u8> {
    pdf(&[
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [4 0 R] /NeedAppearances true /DA (/Helv 12 Tf 0 g) /DR << /Font << /Helv 5 0 R >> >> >> >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 300] >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /Annots [4 0 R] >>".to_string(),
        format!("<< /Type /Annot /Subtype /Widget {field} /Rect [20 250 180 270] /P 3 0 R >>"),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_string(),
    ])
}

fn page_text(app: &PdfCraftApp) -> String {
    let doc = app.session.get(app.views[0].id).unwrap();
    let mut renderer = pdfcraft_render::PageRenderer::new(doc.bytes.clone(), Default::default());
    let request = pdfcraft_render::RenderRequest { page: 0, kind: pdfcraft_render::RequestKind::Text, scale: 1.0, ..Default::default() };
    renderer.render(request).text.map(|t| t.plain_text()).unwrap_or_default()
}

#[test]
fn flatten_draws_a_filled_field_that_has_no_stored_appearance() {
    let mut app = PdfCraftApp::new();
    app.open_bytes("form.pdf", None, form_without_appearance("/FT /Tx /T (name) /V (Ada Example)")).unwrap();
    assert_eq!(app.session.get(app.views[0].id).unwrap().form[0].value, ["Ada Example"]);
    assert!(app.execute("doc.flatten"), "the value can be drawn, so it flattens");
    assert_eq!(counts(&app), (0, 0), "the field is gone");
    assert!(page_text(&app).contains("Ada Example"), "and its value is on the page: {:?}", page_text(&app));
}

#[test]
fn flatten_stops_instead_of_deleting_a_value_it_cannot_draw() {
    let mut app = PdfCraftApp::new();
    // A ticked check box with no appearance: nothing to bake in.
    app.open_bytes("ticked.pdf", None, form_without_appearance("/FT /Btn /T (agree) /V /Yes /AS /Yes")).unwrap();
    let before = counts(&app);
    app.execute("doc.flatten");
    assert_eq!(counts(&app), before, "the field is still there");
    assert!(!app.session.get(app.views[0].id).unwrap().dirty, "and nothing changed");
    let said = app.toast.clone().map(|t| t.0).unwrap_or_default();
    assert!(said.contains("agree") && said.contains("nothing was flattened"), "the person is told why: {said:?}");
}

#[test]
fn flatten_still_removes_an_empty_field_and_an_unticked_box() {
    for field in ["/FT /Tx /T (name)", "/FT /Btn /T (agree) /V /Off /AS /Off"] {
        let mut app = PdfCraftApp::new();
        app.open_bytes("empty.pdf", None, form_without_appearance(field)).unwrap();
        assert!(app.execute("doc.flatten"), "{field}: nothing to lose");
        assert_eq!(counts(&app), (0, 0), "{field}");
    }
}

#[test]
fn flatten_keeps_the_stored_appearance_of_a_field_whose_other_widget_has_none() {
    // One field, value 42, shown twice: the first widget stores "USD 42.00", the second stores nothing.
    let ap = "<< /Type /XObject /Subtype /Form /BBox [0 0 160 20] /Resources << /Font << /Helv 7 0 R >> >> /Length 39 >>\nstream\nBT /Helv 12 Tf 2 5 Td (USD 42.00) Tj ET\nendstream";
    let bytes = pdf(&[
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [4 0 R] /NeedAppearances true /DA (/Helv 12 Tf 0 g) /DR << /Font << /Helv 7 0 R >> >> >> >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 300] >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /Annots [5 0 R 6 0 R] >>".to_string(),
        "<< /FT /Tx /T (amount) /V (42) /Kids [5 0 R 6 0 R] >>".to_string(),
        "<< /Type /Annot /Subtype /Widget /Parent 4 0 R /Rect [20 250 180 270] /P 3 0 R /AP << /N 8 0 R >> >>".to_string(),
        "<< /Type /Annot /Subtype /Widget /Parent 4 0 R /Rect [20 200 180 220] /P 3 0 R >>".to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_string(),
        ap.to_string(),
    ]);
    let mut app = PdfCraftApp::new();
    app.open_bytes("mixed.pdf", None, bytes).unwrap();
    assert!(app.execute("doc.flatten"), "both copies can be shown");
    assert_eq!(counts(&app), (0, 0), "the field is gone");
    let text = page_text(&app);
    assert!(text.contains("USD 42.00"), "the stored appearance is kept as it was: {text:?}");
    assert!(text.matches("42").count() >= 2, "and the widget with none is drawn from the value: {text:?}");
}

fn with_pending_redaction(app: &mut PdfCraftApp, bytes: Vec<u8>) {
    use pdfcraft_engine::{NewAnnotation, Shape, Style};
    app.open_bytes("marked.pdf", None, bytes).unwrap();
    let shape =
        Shape::Redact { quads: vec![pdfcraft_engine::rect_quad([20.0, 100.0, 100.0, 140.0])], overlay: String::new(), look: Default::default() };
    assert!(app.apply_edit(Edit::AddAnnotation(NewAnnotation {
        page: 0,
        style: Style::default_for(&shape),
        shape,
        contents: String::new(),
        author: String::new()
    })));
}

#[test]
fn flatten_leaves_a_pending_redaction_to_be_applied() {
    let mut app = PdfCraftApp::new();
    with_pending_redaction(&mut app, plain(1));
    let id = app.views[0].id;
    assert_eq!(app.session.get(id).unwrap().redaction_marks(), 1);
    let spec = commands::command("doc.flatten").unwrap();
    assert!(!commands::is_enabled(spec, &app.session, Some(id)), "a mark waiting to be applied is nothing to flatten");
    app.execute("doc.flatten");
    assert_eq!(app.session.get(id).unwrap().redaction_marks(), 1, "the mark is still there to apply");
}

#[test]
fn flatten_takes_the_comments_and_keeps_the_pending_redaction_beside_them() {
    let mut app = PdfCraftApp::new();
    with_pending_redaction(&mut app, with_comments());
    let id = app.views[0].id;
    assert!(app.execute("doc.flatten"));
    let doc = app.session.get(id).unwrap();
    assert_eq!(doc.redaction_marks(), 1);
    assert_eq!(doc.info.annotations.len(), 1, "only the mark is left of the annotations");
}

// ----------------------------------------------------------- the toolbar at small window sizes

/// Every control of the Organize toolbar that the tests can name, with where it is drawn.
fn toolbar_controls(h: &Harness<'static, PdfCraftApp>) -> Vec<(&'static str, egui::Rect)> {
    [
        "Rotate counterclockwise",
        "Rotate clockwise",
        "Delete pages",
        "Insert a blank page",
        "Insert pages from a file",
        "Replace pages",
        "Extract pages",
        "Split into files",
        "Select pages",
        "Move earlier",
        "Move later",
        "Save pages",
        "Larger pages",
        "Reset page size",
        "Smaller pages",
        "Preview resolution",
    ]
    .into_iter()
    .filter_map(|label| h.query_by_label_contains(label).map(|n| (label, n.rect())))
    .collect()
}

#[test]
fn the_organize_toolbar_never_overlaps_at_small_window_sizes() {
    let sizes = (16..=30).map(|k| (k as f32 * 50.0, 1.0)).chain([(900.0, 1.5), (820.0, 1.5), (820.0, 2.0)]);
    for (width, scale) in sizes {
        let mut h = Harness::builder().with_size(egui::vec2(width, 650.0)).with_pixels_per_point(scale).build_eframe(move |_cc| {
            let mut app = PdfCraftApp::new();
            app.set_option("language", "en").unwrap();
            app.open_bytes("doc.pdf", None, plain(6)).unwrap();
            app.set_option("organize", "on").unwrap();
            app.set_option("organize-quality", "high").unwrap();
            app
        });
        h.run_steps(6);
        let controls = toolbar_controls(&h);
        assert!(controls.len() >= 14, "{width}x650 at {scale}: found only {:?}", controls.iter().map(|c| c.0).collect::<Vec<_>>());
        for (i, (a, ra)) in controls.iter().enumerate() {
            assert!(ra.right() <= width + 0.5 && ra.left() >= -0.5, "{width}x650 at {scale}: {a} is drawn outside the window: {ra:?}");
            for (b, rb) in &controls[i + 1..] {
                assert!(!ra.shrink(1.0).intersects(rb.shrink(1.0)), "{width}x650 at {scale}: {a} {ra:?} overlaps {b} {rb:?}");
            }
        }
    }
}
