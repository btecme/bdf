//! The service through its public contract: typed requests in, results and errors out.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pdfcraft_app_service::*;
use pdfcraft_platform::file_identity::FileVersion;

/// A minimal `n`-page document; page `i` shows "Page i+1".
fn fixture(n: usize) -> Vec<u8> {
    let mut objs: Vec<Vec<u8>> = vec![b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()];
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 4 + 2 * i)).collect();
    objs.push(format!("<< /Type /Pages /Kids [{}] /Count {n} /MediaBox [0 0 200 300] >>", kids.join(" ")).into_bytes());
    objs.push(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec());
    for i in 0..n {
        objs.push(format!("<< /Type /Page /Parent 2 0 R /Contents {} 0 R /Resources << /Font << /F1 3 0 R >> >> >>", 5 + 2 * i).into_bytes());
        let body = format!("BT /F1 24 Tf 20 150 Td (Page {}) Tj ET", i + 1);
        objs.push(format!("<< /Length {} >>\nstream\n{body}\nendstream", body.len()).into_bytes());
    }
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

/// Two pages. Page 1 carries a web link and a link to page 2; the outline has two top-level bookmarks, one for each page.
fn fixture_with_links_and_bookmarks() -> Vec<u8> {
    let objs: Vec<&str> = vec![
        "<< /Type /Catalog /Pages 2 0 R /Outlines 8 0 R >>",
        "<< /Type /Pages /Kids [4 0 R 6 0 R] /Count 2 /MediaBox [0 0 200 300] >>",
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        "<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Resources << /Font << /F1 3 0 R >> >> /Annots [9 0 R 10 0 R] >>",
        "<< /Length 0 >>\nstream\n\nendstream",
        "<< /Type /Page /Parent 2 0 R /Contents 7 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
        "<< /Length 0 >>\nstream\n\nendstream",
        "<< /Type /Outlines /First 11 0 R /Last 12 0 R /Count 2 >>",
        "<< /Type /Annot /Subtype /Link /Rect [10 10 80 30] /A << /S /URI /URI (https://example.com/) >> >>",
        "<< /Type /Annot /Subtype /Link /Rect [10 40 80 60] /Dest [6 0 R /Fit] >>",
        "<< /Title (Intro) /Parent 8 0 R /Dest [4 0 R /Fit] /Next 12 0 R >>",
        "<< /Title (Second) /Parent 8 0 R /Dest [6 0 R /Fit] /Prev 11 0 R >>",
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

/// The person at the keyboard.
fn me() -> ActorContext {
    ActorContext::local_user().expect("the current user")
}

fn ctx() -> RequestContext {
    RequestContext::new(1, me())
}

fn run(service: &mut Service, document: Option<DocumentKey>, operation: Operation) -> Result<OperationResult> {
    service.execute(&ctx(), OperationRequest::new(document, operation))
}

fn open_bytes(service: &mut Service, pages: usize) -> (DocumentKey, DocumentVersion) {
    let source = OpenSource::Bytes { name: "fixture.pdf".into(), bytes: Arc::new(fixture(pages)) };
    let done = run(service, None, Operation::Open { source, password: None }).expect("opens");
    match done.outcome {
        Outcome::Opened { document, .. } => (document, done.version.expect("a version")),
        other => panic!("opened a document, got {other:?}"),
    }
}

fn open_file(service: &mut Service, path: &Path) -> (DocumentKey, DocumentVersion) {
    let done = run(service, None, Operation::Open { source: OpenSource::File(path.to_path_buf()), password: None }).expect("opens");
    match done.outcome {
        Outcome::Opened { document, .. } => (document, done.version.expect("a version")),
        other => panic!("opened a document, got {other:?}"),
    }
}

fn rotate(service: &mut Service, key: DocumentKey, revision: u64, page: i64) -> Result<OperationResult> {
    run(service, Some(key), Operation::Rotate { expected_revision: revision, pages: PageSelection::Numbers(vec![page]), degrees: 90 })
}

fn rotations(service: &Service, key: DocumentKey) -> Vec<u16> {
    service.document(&me(), key).expect("open").info().pages.iter().map(|p| p.rotation).collect()
}

fn save(service: &mut Service, key: DocumentKey, revision: u64, target: SaveTarget, overwrite: Overwrite) -> Result<OperationResult> {
    run(service, Some(key), Operation::Save { expected_revision: revision, target, overwrite, full: None })
}

fn saved_report(done: OperationResult) -> SaveReport {
    match done.outcome {
        Outcome::Saved(report) => report,
        other => panic!("saved, got {other:?}"),
    }
}

fn file_in(dir: &tempfile::TempDir, name: &str, pages: usize) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, fixture(pages)).expect("writes the fixture");
    path
}

// ---- descriptors ----

#[test]
fn every_operation_has_one_descriptor_and_the_descriptor_matches_the_typed_request() {
    for (index, id) in OperationId::ALL.into_iter().enumerate() {
        let descriptor = id.descriptor();
        assert_eq!(descriptor.id, id);
        assert_eq!(DESCRIPTORS[index].id, id, "the table is in declaration order");
        assert_eq!(OperationId::parse(descriptor.name), Some(id));
        assert_eq!(OperationId::ALL.iter().filter(|other| other.name() == descriptor.name).count(), 1, "names are unique");
    }
    let samples = [
        Operation::Open { source: OpenSource::Bytes { name: "a.pdf".into(), bytes: Arc::new(Vec::new()) }, password: None },
        Operation::Query,
        Operation::Rotate { expected_revision: 0, pages: PageSelection::All, degrees: 90 },
        Operation::Undo { expected_revision: 0 },
        Operation::Redo { expected_revision: 0 },
        Operation::History,
        Operation::Save { expected_revision: 0, target: SaveTarget::Tracked, overwrite: Overwrite::Tracked, full: None },
        Operation::Close { expected_revision: 0, discard_changes: false },
        Operation::List,
        Operation::Links,
        Operation::Bookmarks { offset: 0, limit: DEFAULT_BOOKMARK_PAGE },
    ];
    assert_eq!(samples.len(), OperationId::ALL.len());
    for op in samples {
        let descriptor = op.id().descriptor();
        assert_eq!(
            op.expected_revision().is_some(),
            descriptor.needs_revision,
            "{}: the typed request and the descriptor agree on the revision",
            descriptor.name
        );
        let changes = matches!(descriptor.access, Access::Mutate | Access::Persist);
        assert!(!changes || descriptor.needs_revision, "{}: anything that changes content or writes a file names its revision", descriptor.name);
    }
}

// ---- contract: schema, documents, revisions ----

#[test]
fn the_schema_version_is_negotiated_before_anything_else() {
    let mut service = Service::new();
    assert_eq!(service.schema(), SchemaRange { oldest: OLDEST_SCHEMA, newest: SCHEMA_VERSION });
    for version in [0, SCHEMA_VERSION + 1, u32::MAX] {
        let mut request = OperationRequest::new(None, Operation::Query);
        request.schema_version = version;
        let error = service.execute(&ctx(), request).expect_err("unsupported");
        assert_eq!(error.code, ErrorCode::UnsupportedSchema, "{version}");
        assert!(error.message.contains(&format!("{OLDEST_SCHEMA} to {SCHEMA_VERSION}")), "the answer carries the supported range: {}", error.message);
    }
    assert!(service.documents(&me()).is_empty(), "a refused request had no effect");
    let (key, _) = open_bytes(&mut service, 1);
    // The schema is checked even for a request that would otherwise fail on its document.
    let mut request = OperationRequest::new(Some(key), Operation::History);
    request.schema_version = 99;
    assert_eq!(service.execute(&ctx(), request).expect_err("unsupported").code, ErrorCode::UnsupportedSchema);
}

#[test]
fn a_document_is_named_by_session_and_number() {
    let mut a = Service::new();
    let mut b = Service::new();
    assert_ne!(a.session_id(), b.session_id());
    let (key_a, _) = open_bytes(&mut a, 1);
    let (key_b, _) = open_bytes(&mut b, 1);
    assert_eq!(key_a.document, key_b.document, "the same small number in both sessions");
    // Only the session makes it an identity: a key from the other session reaches nothing.
    assert_eq!(run(&mut a, Some(key_b), Operation::Query).expect_err("another session").code, ErrorCode::NotFound);
    assert_eq!(a.document(&me(), key_b).err().map(|e| e.code), Some(ErrorCode::NotFound));
    // A document operation without a document is a bad request, not a crash.
    assert_eq!(run(&mut a, None, Operation::Query).expect_err("no document").code, ErrorCode::InvalidArgument);
    let unknown = DocumentKey { session: a.session_id(), document: 9_999 };
    assert_eq!(run(&mut a, Some(unknown), Operation::Query).expect_err("unknown").code, ErrorCode::NotFound);
}

#[test]
fn query_numbers_pages_from_one() {
    let mut service = Service::new();
    let (key, _) = open_bytes(&mut service, 3);
    let done = run(&mut service, Some(key), Operation::Query).expect("queries");
    let Outcome::Info(info) = done.outcome else { panic!("info") };
    assert_eq!(info.pages.iter().map(|p| p.number).collect::<Vec<_>>(), [1, 2, 3]);
    assert!(info.editable && !info.signed && !info.has_file);
    assert_eq!((info.pages[0].width, info.pages[0].height), (200.0, 300.0));
}

#[test]
fn a_stale_mutation_changes_nothing() {
    let mut service = Service::new();
    let (key, opened) = open_bytes(&mut service, 2);
    let first = rotate(&mut service, key, opened.content_revision, 1).expect("rotates");
    let after = first.version.expect("version");
    assert!(after.content_revision > opened.content_revision);
    let rotations_before = rotations(&service, key);
    assert_eq!(rotations_before, [90, 0]);

    // Every kind of changing request, made against the revision before that edit, is refused whole.
    let stale = opened.content_revision;
    let attempts = [
        Operation::Rotate { expected_revision: stale, pages: PageSelection::All, degrees: 90 },
        Operation::Undo { expected_revision: stale },
        Operation::Redo { expected_revision: stale },
        Operation::Close { expected_revision: stale, discard_changes: true },
        Operation::Save { expected_revision: stale, target: SaveTarget::Path("never.pdf".into()), overwrite: Overwrite::Tracked, full: None },
    ];
    for attempt in attempts {
        let name = attempt.id().name();
        let error = run(&mut service, Some(key), attempt).expect_err(name);
        assert_eq!(error.code, ErrorCode::Conflict, "{name}");
        assert_eq!(error.conflict, Some(ConflictKind::StaleRevision { expected: stale, current: after.content_revision }), "{name}");
        assert!(error.committed.is_empty(), "{name}: nothing happened");
    }
    let view = service.document(&me(), key).expect("still open");
    assert_eq!(view.version(), after, "the version is untouched");
    assert_eq!(rotations(&service, key), rotations_before, "the pages are untouched");
    assert_eq!(view.next_undo().as_deref(), Some("Rotate page"), "so is the history");
    assert_eq!(view.next_redo(), None);
}

#[test]
fn a_request_made_against_the_current_revision_is_accepted_and_moves_it_forward() {
    let mut service = Service::new();
    let (key, v0) = open_bytes(&mut service, 2);
    let v1 = rotate(&mut service, key, v0.content_revision, 2).expect("edit").version.expect("version");
    let v2 = run(&mut service, Some(key), Operation::Undo { expected_revision: v1.content_revision }).expect("undo").version.expect("version");
    let v3 = run(&mut service, Some(key), Operation::Redo { expected_revision: v2.content_revision }).expect("redo").version.expect("version");
    assert!(v0.content_revision < v1.content_revision && v1.content_revision < v2.content_revision && v2.content_revision < v3.content_revision);
    assert_eq!(rotations(&service, key), [0, 90]);
    // The revision a client holds is invalidated by any change, including its own.
    assert_eq!(rotate(&mut service, key, v2.content_revision, 1).expect_err("stale").code, ErrorCode::Conflict);
}

#[test]
fn undo_back_to_the_opened_content_is_not_unsaved_and_a_save_does_not_move_the_revision() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = file_in(&dir, "doc.pdf", 2);
    let mut service = Service::new();
    let (key, v0) = open_file(&mut service, &path);
    assert!(!v0.dirty());
    let v1 = rotate(&mut service, key, v0.content_revision, 1).expect("edit").version.expect("version");
    assert!(v1.dirty());
    let v2 = run(&mut service, Some(key), Operation::Undo { expected_revision: v1.content_revision }).expect("undo").version.expect("version");
    assert!(!v2.dirty(), "undo back to the opened content leaves nothing unsaved, though the revision moved on");
    assert!(v2.content_revision > v1.content_revision);
    let v3 = run(&mut service, Some(key), Operation::Redo { expected_revision: v2.content_revision }).expect("redo").version.expect("version");
    assert!(v3.dirty());

    let done = save(&mut service, key, v3.content_revision, SaveTarget::Tracked, Overwrite::Tracked).expect("saves");
    let after = done.version.expect("version");
    assert_eq!(after.content_revision, v3.content_revision, "saving writes the same content");
    assert!(after.render_generation > v3.render_generation, "but the render generation moved");
    assert!(!after.dirty());
    // Undo past the saved content is unsaved; redo back to it is clean.
    let back = run(&mut service, Some(key), Operation::Undo { expected_revision: after.content_revision }).expect("undo").version.expect("version");
    assert!(back.dirty());
    let forth = run(&mut service, Some(key), Operation::Redo { expected_revision: back.content_revision }).expect("redo").version.expect("version");
    assert!(!forth.dirty());
}

#[test]
fn bad_arguments_are_refused_before_anything_changes() {
    let mut service = Service::new();
    let (key, v0) = open_bytes(&mut service, 2);
    let at = v0.content_revision;
    let cases: Vec<(&str, Operation, &str)> = vec![
        ("degrees", Operation::Rotate { expected_revision: at, pages: PageSelection::All, degrees: 45 }, "multiple of 90"),
        ("page 0", Operation::Rotate { expected_revision: at, pages: PageSelection::Numbers(vec![0]), degrees: 90 }, "start at 1 (got 0)"),
        ("negative", Operation::Rotate { expected_revision: at, pages: PageSelection::Numbers(vec![-3]), degrees: 90 }, "start at 1 (got -3)"),
        (
            "past the end",
            Operation::Rotate { expected_revision: at, pages: PageSelection::Numbers(vec![1, 3]), degrees: 90 },
            "page 3 is out of range: the document has 2 pages",
        ),
        ("huge", Operation::Rotate { expected_revision: at, pages: PageSelection::Numbers(vec![i64::MAX]), degrees: 90 }, "out of range"),
        ("empty", Operation::Rotate { expected_revision: at, pages: PageSelection::Numbers(Vec::new()), degrees: 90 }, "at least one page"),
    ];
    for (what, operation, expect) in cases {
        let error = run(&mut service, Some(key), operation).expect_err(what);
        assert_eq!(error.code, ErrorCode::InvalidArgument, "{what}");
        assert!(error.message.contains(expect), "{what}: {}", error.message);
    }
    assert_eq!(service.document(&me(), key).expect("open").version(), v0, "none of them changed anything, not even the render generation");
    assert_eq!(rotations(&service, key), [0, 0]);
    assert_eq!(
        run(&mut service, Some(key), Operation::Undo { expected_revision: at }).expect_err("nothing to undo").code,
        ErrorCode::InvalidArgument
    );
}

#[test]
fn history_reports_what_undo_and_redo_would_do() {
    let mut service = Service::new();
    let (key, v0) = open_bytes(&mut service, 1);
    let Outcome::History(h) = run(&mut service, Some(key), Operation::History).expect("history").outcome else { panic!("history") };
    assert_eq!((h.next_undo, h.next_redo, h.unsaved), (None, None, false));
    let v1 = rotate(&mut service, key, v0.content_revision, 1).expect("edit").version.expect("version");
    let Outcome::History(h) = run(&mut service, Some(key), Operation::History).expect("history").outcome else { panic!("history") };
    assert_eq!((h.next_undo.as_deref(), h.next_redo, h.unsaved), (Some("Rotate page"), None, true));
    run(&mut service, Some(key), Operation::Undo { expected_revision: v1.content_revision }).expect("undo");
    let Outcome::History(h) = run(&mut service, Some(key), Operation::History).expect("history").outcome else { panic!("history") };
    assert_eq!((h.next_undo, h.next_redo.as_deref(), h.unsaved), (None, Some("Rotate page"), false));
}

// ---- close ----

#[test]
fn close_refuses_to_drop_changes_nobody_chose_to_drop() {
    let mut service = Service::new();
    let (key, v0) = open_bytes(&mut service, 1);
    let v1 = rotate(&mut service, key, v0.content_revision, 1).expect("edit").version.expect("version");
    let error =
        run(&mut service, Some(key), Operation::Close { expected_revision: v1.content_revision, discard_changes: false }).expect_err("unsaved");
    assert_eq!((error.code, error.conflict), (ErrorCode::Conflict, Some(ConflictKind::UnsavedChanges)));
    assert_eq!(service.documents(&me()), [key], "still open");
    let done = run(&mut service, Some(key), Operation::Close { expected_revision: v1.content_revision, discard_changes: true }).expect("closes");
    assert_eq!((done.outcome, done.effects, done.version), (Outcome::Closed, vec![Effect::DocumentClosed], None));
    assert!(service.documents(&me()).is_empty());
    assert_eq!(run(&mut service, Some(key), Operation::Query).expect_err("closed").code, ErrorCode::NotFound);
}

// ---- saving ----

#[test]
fn save_writes_the_file_and_a_new_session_reads_the_edit_back() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = file_in(&dir, "doc.pdf", 3);
    let original = std::fs::read(&path).expect("reads");
    let mut service = Service::new();
    let (key, v0) = open_file(&mut service, &path);
    let v1 = rotate(&mut service, key, v0.content_revision, 2).expect("edit").version.expect("version");
    let report = saved_report(save(&mut service, key, v1.content_revision, SaveTarget::Tracked, Overwrite::Tracked).expect("saves"));
    assert_eq!(report.file, "doc.pdf");
    assert!(report.incremental && report.document_open && !report.edited_since);
    let on_disk = std::fs::read(&path).expect("reads");
    assert!(on_disk.starts_with(&original), "an in-place save appends");
    assert_eq!(report.fingerprint, FileVersion::of_bytes(&on_disk, None).fingerprint());

    let mut fresh = Service::new();
    let (reopened, _) = open_file(&mut fresh, &path);
    assert_eq!(rotations(&fresh, reopened), [0, 90, 0], "independently reopened");
}

#[test]
fn a_save_refuses_a_file_that_changed_and_both_versions_survive() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = file_in(&dir, "doc.pdf", 2);
    let mut service = Service::new();
    let (key, v0) = open_file(&mut service, &path);
    let v1 = rotate(&mut service, key, v0.content_revision, 1).expect("edit").version.expect("version");

    // Another program replaces the file after it was opened.
    let theirs = fixture(5);
    std::fs::write(&path, &theirs).expect("overwrites");
    let error = save(&mut service, key, v1.content_revision, SaveTarget::Tracked, Overwrite::Tracked).expect_err("conflict");
    assert_eq!(error.code, ErrorCode::Conflict);
    let Some(ConflictKind::FileChanged { disk }) = error.conflict.clone() else { panic!("file changed, got {:?}", error.conflict) };
    assert_eq!(disk, FileVersion::of_bytes(&theirs, None).fingerprint());
    assert!(!error.message.contains(dir.path().to_str().expect("utf-8")), "the message names the file, not the folders: {}", error.message);
    assert_eq!(std::fs::read(&path).expect("reads"), theirs, "their version is untouched");
    let view = service.document(&me(), key).expect("open");
    assert_eq!(view.version(), v1, "ours is untouched too");
    assert!(view.version().dirty());

    // Inspected and confirmed by fingerprint: now it may be replaced. A wrong or malformed one may not.
    let wrong = FileVersion::of_bytes(b"not the file", None).fingerprint();
    assert_eq!(
        save(&mut service, key, v1.content_revision, SaveTarget::Tracked, Overwrite::Version(wrong)).expect_err("wrong").code,
        ErrorCode::Conflict
    );
    assert_eq!(
        save(&mut service, key, v1.content_revision, SaveTarget::Tracked, Overwrite::Version("zz".into())).expect_err("malformed").code,
        ErrorCode::InvalidArgument
    );
    assert_eq!(std::fs::read(&path).expect("reads"), theirs);
    saved_report(save(&mut service, key, v1.content_revision, SaveTarget::Tracked, Overwrite::Version(disk)).expect("confirmed"));
    assert_ne!(std::fs::read(&path).expect("reads"), theirs);
}

#[test]
fn a_deleted_file_is_not_silently_recreated_and_a_new_path_is_not_overwritten() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = file_in(&dir, "doc.pdf", 1);
    let mut service = Service::new();
    let (key, v0) = open_file(&mut service, &path);
    let v1 = rotate(&mut service, key, v0.content_revision, 1).expect("edit").version.expect("version");

    std::fs::remove_file(&path).expect("deletes");
    let error = save(&mut service, key, v1.content_revision, SaveTarget::Tracked, Overwrite::Tracked).expect_err("gone");
    assert_eq!(error.conflict, Some(ConflictKind::FileGone));
    assert!(!path.exists());
    saved_report(save(&mut service, key, v1.content_revision, SaveTarget::Tracked, Overwrite::Absent).expect("recreated on request"));
    assert!(path.exists());

    // Another name: created, never replaced.
    let other = file_in(&dir, "other.pdf", 1);
    let before = std::fs::read(&other).expect("reads");
    let error = save(&mut service, key, v1.content_revision, SaveTarget::Path(other.clone()), Overwrite::Tracked).expect_err("exists");
    assert!(matches!(error.conflict, Some(ConflictKind::FileExists { .. })), "{:?}", error.conflict);
    assert_eq!(std::fs::read(&other).expect("reads"), before);
    let fresh = dir.path().join("fresh.pdf");
    let report = saved_report(save(&mut service, key, v1.content_revision, SaveTarget::Path(fresh.clone()), Overwrite::Tracked).expect("created"));
    assert!(!report.incremental, "a new file is a full rewrite");
    assert!(fresh.exists());
}

#[test]
fn a_document_that_was_never_saved_needs_a_path() {
    let mut service = Service::new();
    let (key, v0) = open_bytes(&mut service, 1);
    let error = save(&mut service, key, v0.content_revision, SaveTarget::Tracked, Overwrite::Tracked).expect_err("no path");
    assert_eq!(error.code, ErrorCode::InvalidArgument);
    assert!(error.message.contains("never been saved"));
}

#[test]
fn a_late_save_result_does_not_overwrite_a_newer_document() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = file_in(&dir, "doc.pdf", 2);
    let mut service = Service::new();
    let (key, v0) = open_file(&mut service, &path);
    let v1 = rotate(&mut service, key, v0.content_revision, 1).expect("edit").version.expect("version");

    // A save is planned and written elsewhere (a worker); meanwhile the user edits again.
    let plan = service.prepare_save(&ctx(), key, v1.content_revision, SaveTarget::Tracked, Overwrite::Tracked, None).expect("plans");
    assert_eq!((plan.revision(), plan.document()), (v1.content_revision, key));
    let written = plan.write().expect("writes");
    assert!(service.is_current(key, v1.content_revision));
    let v2 = rotate(&mut service, key, v1.content_revision, 2).expect("a newer edit").version.expect("version");
    assert!(!service.is_current(key, v1.content_revision), "a worker can tell that the document moved on");

    let report = saved_report(service.finish_save(&written).expect("finishes"));
    assert!(report.edited_since && report.document_open);
    let view = service.document(&me(), key).expect("open");
    assert_eq!(view.version().content_revision, v2.content_revision, "the newer edit is still the content");
    assert!(view.version().dirty(), "and it is still unsaved");
    assert_eq!(rotations(&service, key), [90, 90]);
    // The file holds the first edit only, and the session knows that is the version it wrote.
    let mut fresh = Service::new();
    let (reopened, _) = open_file(&mut fresh, &path);
    assert_eq!(rotations(&fresh, reopened), [90, 0]);
    saved_report(
        save(&mut service, key, v2.content_revision, SaveTarget::Tracked, Overwrite::Tracked).expect("the next save may replace what was written"),
    );
}

#[test]
fn splitting_a_save_does_not_skip_the_revision_check() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = file_in(&dir, "doc.pdf", 1);
    let mut service = Service::new();
    let (key, v0) = open_file(&mut service, &path);
    let v1 = rotate(&mut service, key, v0.content_revision, 1).expect("edit").version.expect("version");
    let before = std::fs::read(&path).expect("reads");
    let error = service.prepare_save(&ctx(), key, v0.content_revision, SaveTarget::Tracked, Overwrite::Tracked, None).expect_err("stale");
    assert_eq!(error.conflict, Some(ConflictKind::StaleRevision { expected: v0.content_revision, current: v1.content_revision }));
    assert_eq!(std::fs::read(&path).expect("reads"), before);
    let plan = service.prepare_save(&ctx(), key, v1.content_revision, SaveTarget::Tracked, Overwrite::Tracked, None).expect("plans");
    let shown = format!("{plan:?}");
    assert!(shown.contains("doc.pdf") && !shown.contains("%PDF"), "a debug print names the file and never carries the document: {shown}");
}

#[test]
fn a_save_that_finishes_after_its_document_closed_leaves_the_file_and_nothing_else() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = file_in(&dir, "doc.pdf", 1);
    let mut service = Service::new();
    let (key, v0) = open_file(&mut service, &path);
    let v1 = rotate(&mut service, key, v0.content_revision, 1).expect("edit").version.expect("version");
    let plan = service.prepare_save(&ctx(), key, v1.content_revision, SaveTarget::Tracked, Overwrite::Tracked, None).expect("plans");
    let written = plan.write().expect("writes");
    run(&mut service, Some(key), Operation::Close { expected_revision: v1.content_revision, discard_changes: true }).expect("closes");

    let done = service.finish_save(&written).expect("the file stands");
    let Outcome::Saved(report) = &done.outcome else { panic!("saved") };
    assert!(!report.document_open);
    assert_eq!((done.version, done.effects.len()), (None, 1));
    assert!(service.documents(&me()).is_empty(), "the closed document did not come back");
}

// ---- confinement ----

#[test]
fn a_confined_service_touches_only_files_below_its_root() {
    let base = tempfile::tempdir().expect("temp dir");
    let root = base.path().join("root");
    std::fs::create_dir_all(&root).expect("root");
    std::fs::write(root.join("in.pdf"), fixture(1)).expect("fixture");
    let outside = base.path().join("outside.pdf");
    std::fs::write(&outside, fixture(1)).expect("fixture");

    let mut service = Service::confined_to(&root).expect("a folder");
    for escape in [outside.clone(), PathBuf::from("../outside.pdf"), PathBuf::from("sub/../../outside.pdf"), root.join("..").join("outside.pdf")] {
        let error = run(&mut service, None, Operation::Open { source: OpenSource::File(escape.clone()), password: None }).expect_err("outside");
        assert_eq!(error.code, ErrorCode::InvalidArgument, "{}", escape.display());
        assert!(error.message.contains("outside the allowed directory"), "{}", error.message);
    }
    let (key, v0) = open_file(&mut service, Path::new("in.pdf"));
    let v1 = rotate(&mut service, key, v0.content_revision, 1).expect("edit").version.expect("version");
    for escape in [outside.clone(), PathBuf::from("../escaped.pdf")] {
        let error = save(&mut service, key, v1.content_revision, SaveTarget::Path(escape), Overwrite::Absent).expect_err("outside");
        assert_eq!(error.code, ErrorCode::InvalidArgument);
    }
    assert!(!base.path().join("escaped.pdf").exists());
    // Inside, new subfolders are made and the save is an ordinary one.
    saved_report(
        save(&mut service, key, v1.content_revision, SaveTarget::Path("a/b/out.pdf".into()), Overwrite::Absent).expect("saves below the root"),
    );
    assert!(root.join("a").join("b").join("out.pdf").exists());
    saved_report(save(&mut service, key, v1.content_revision, SaveTarget::Tracked, Overwrite::Tracked).expect("saves in place"));
}

#[test]
fn a_folder_replaced_by_a_link_out_of_the_root_is_not_written_through() {
    let base = tempfile::tempdir().expect("temp dir");
    let root = base.path().join("root");
    let outside = base.path().join("outside");
    std::fs::create_dir_all(root.join("work")).expect("root");
    std::fs::create_dir_all(&outside).expect("outside");
    std::fs::write(root.join("work").join("in.pdf"), fixture(1)).expect("fixture");
    // A byte-identical decoy outside, so that the version check alone could not tell the files apart.
    std::fs::write(outside.join("in.pdf"), fixture(1)).expect("decoy");
    let decoy = std::fs::read(outside.join("in.pdf")).expect("reads");

    let mut service = Service::confined_to(&root).expect("a folder");
    let (key, v0) = open_file(&mut service, Path::new("work/in.pdf"));
    let v1 = rotate(&mut service, key, v0.content_revision, 1).expect("edit").version.expect("version");

    // The program that can write the tree swaps the folder for a link out. On Windows the open document's pinned
    // folder cannot be renamed while it is held, but between operations nothing holds it.
    std::fs::rename(root.join("work"), root.join("work-moved")).expect("moves the folder");
    link_dir(&outside, &root.join("work"));

    let error = save(&mut service, key, v1.content_revision, SaveTarget::Tracked, Overwrite::Tracked).expect_err("refused");
    assert!(matches!(error.code, ErrorCode::InvalidArgument | ErrorCode::IoFailure | ErrorCode::Conflict), "{error}");
    assert_eq!(std::fs::read(outside.join("in.pdf")).expect("reads"), decoy, "nothing was written through the link");
    assert!(service.document(&me(), key).expect("open").version().dirty(), "and the edit is still held");
}

/// A directory link `at` -> `to`: a symbolic link on Unix, a junction on Windows (which needs no privilege).
fn link_dir(to: &Path, at: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(to, at).expect("symlink");
    #[cfg(windows)]
    {
        let made = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(at).arg(to).output().expect("runs cmd").status.success();
        assert!(made, "mklink /J must be available to run this test");
    }
}

// ---- saves that land late, out of order, or with the wrong receipt (BUILD-007 review, P1) ----

#[test]
fn undoing_during_a_save_leaves_the_document_unsaved_because_the_file_holds_the_edit() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = file_in(&dir, "doc.pdf", 2);
    let mut service = Service::new();
    let (key, v0) = open_file(&mut service, &path);
    let v1 = rotate(&mut service, key, v0.content_revision, 1).expect("edit").version.expect("version");

    let plan = service.prepare_save(&ctx(), key, v1.content_revision, SaveTarget::Tracked, Overwrite::Tracked, None).expect("plans");
    let written = plan.write().expect("writes");
    // While the write is landing the user undoes the edit: back at the opened content, which is what the file held.
    let v2 = run(&mut service, Some(key), Operation::Undo { expected_revision: v1.content_revision }).expect("undo").version.expect("version");
    assert!(!v2.dirty());

    let report = saved_report(service.finish_save(&written).expect("finishes"));
    assert!(report.edited_since);
    let view = service.document(&me(), key).expect("open");
    assert!(view.version().dirty(), "the file now holds the edit and the document shows the opened content: not clean");
    let refused =
        run(&mut service, Some(key), Operation::Close { expected_revision: v2.content_revision, discard_changes: false }).expect_err("unsaved");
    assert_eq!(refused.conflict, Some(ConflictKind::UnsavedChanges), "closing without discarding asks first");
    // The file really does hold the edit.
    let mut fresh = Service::new();
    let (reopened, _) = open_file(&mut fresh, &path);
    assert_eq!(rotations(&fresh, reopened), [90, 0]);
    // Redo is back at what the file holds: clean.
    let v3 = run(&mut service, Some(key), Operation::Redo { expected_revision: v2.content_revision }).expect("redo").version.expect("version");
    assert!(!v3.dirty());
}

#[test]
fn a_save_that_was_overtaken_does_not_restore_its_older_destination() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = file_in(&dir, "doc.pdf", 2);
    let mut service = Service::new();
    let (key, v0) = open_file(&mut service, &path);
    let v1 = rotate(&mut service, key, v0.content_revision, 1).expect("edit").version.expect("version");
    let (a, b) = (dir.path().join("a.pdf"), dir.path().join("b.pdf"));

    // Two Save As, planned in this order; the second finishes first.
    let plan_a = service.prepare_save(&ctx(), key, v1.content_revision, SaveTarget::Path(a.clone()), Overwrite::Tracked, None).expect("plans a");
    let plan_b = service.prepare_save(&ctx(), key, v1.content_revision, SaveTarget::Path(b.clone()), Overwrite::Tracked, None).expect("plans b");
    let written_b = plan_b.write().expect("writes b");
    let done_b = service.finish_save(&written_b).expect("finishes b");
    let after_b = done_b.version.expect("version");
    assert!(!saved_report(done_b.clone()).superseded);
    assert_eq!(service.document(&me(), key).expect("open").info().name, "b.pdf");

    let written_a = plan_a.write().expect("writes a");
    let done_a = service.finish_save(&written_a).expect("finishes a");
    let report_a = saved_report(done_a.clone());
    assert!(report_a.superseded && !report_a.edited_since, "the older save only wrote its file");
    assert_eq!(done_a.effects.len(), 1, "and says so");
    let view = service.document(&me(), key).expect("open");
    assert_eq!(view.info().name, "b.pdf", "the document did not go back to the older destination");
    assert_eq!(view.version(), after_b, "nor did its saved state change");
    assert!(a.exists(), "the older file was written all the same");
    assert!(!view.version().dirty(), "the document is clean against b, which is the newer save");

    // The tracked version is b's: the next save replaces it without a conflict, and leaves a alone.
    let a_bytes = std::fs::read(&a).expect("reads");
    let v2 = rotate(&mut service, key, after_b.content_revision, 2).expect("edit").version.expect("version");
    saved_report(save(&mut service, key, v2.content_revision, SaveTarget::Tracked, Overwrite::Tracked).expect("saves to b"));
    assert_eq!(std::fs::read(&a).expect("reads"), a_bytes);
    let mut fresh = Service::new();
    let (reopened, _) = open_file(&mut fresh, &b);
    assert_eq!(rotations(&fresh, reopened), [90, 90]);
}

#[test]
fn a_write_completion_finishes_its_own_save_and_no_other() {
    let dir = tempfile::tempdir().expect("temp dir");
    // Two documents with byte-identical files and the same edit: the bytes cannot tell the saves apart.
    let (path_a, path_b) = (file_in(&dir, "a.pdf", 1), file_in(&dir, "b.pdf", 1));
    let mut service = Service::new().with_clock(|| 1_700_000_000);
    let (key_a, a0) = open_file(&mut service, &path_a);
    let (key_b, b0) = open_file(&mut service, &path_b);
    let a1 = rotate(&mut service, key_a, a0.content_revision, 1).expect("edit").version.expect("version");
    let b1 = rotate(&mut service, key_b, b0.content_revision, 1).expect("edit").version.expect("version");
    let on_disk_a = std::fs::read(&path_a).expect("reads");

    let plan_a = service.prepare_save(&ctx(), key_a, a1.content_revision, SaveTarget::Tracked, Overwrite::Tracked, None).expect("plans a");
    let plan_b = service.prepare_save(&ctx(), key_b, b1.content_revision, SaveTarget::Tracked, Overwrite::Tracked, None).expect("plans b");
    // Only B is written. Its completion is B's: it names B's document, and finishing it leaves A alone, unsaved and
    // unwritten (the completion has no way to be paired with A's plan, which is the point).
    let written_b = plan_b.write().expect("writes b");
    assert_eq!(written_b.document(), key_b);
    let done_b = service.finish_save(&written_b).expect("b finishes");
    assert_eq!(done_b.effects.len(), 1);
    assert_eq!(service.document(&me(), key_a).expect("open").version(), a1, "A is exactly as it was: still unsaved");
    assert!(service.document(&me(), key_a).expect("open").version().dirty());
    assert_eq!(std::fs::read(&path_a).expect("reads"), on_disk_a, "A's file was never written");
    assert!(!service.document(&me(), key_b).expect("open").version().dirty());
    // A can still not be closed without discarding: it was never saved.
    let refused =
        run(&mut service, Some(key_a), Operation::Close { expected_revision: a1.content_revision, discard_changes: false }).expect_err("unsaved");
    assert_eq!(refused.conflict, Some(ConflictKind::UnsavedChanges));

    // A finishes with its own. A completion finishes once.
    let written_a = plan_a.write().expect("writes a");
    saved_report(service.finish_save(&written_a).expect("a finishes"));
    assert!(!service.document(&me(), key_a).expect("open").version().dirty());
    let again = service.finish_save(&written_a).expect_err("finished already");
    assert_eq!(again.code, ErrorCode::InvalidArgument);
    assert_eq!(again.committed.len(), 1, "the refusal still says what the write left on disk");
}

#[test]
fn a_completion_from_another_service_is_refused_and_says_what_it_wrote() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = file_in(&dir, "doc.pdf", 1);
    let mut first = Service::new();
    let mut second = Service::new();
    let (key, v0) = open_file(&mut first, &path);
    let other_path = file_in(&dir, "other.pdf", 1);
    let (other_key, w0) = open_file(&mut second, &other_path);
    assert_eq!(key.document, other_key.document, "both services number their first document alike, and plan numbers start alike too");
    let v1 = rotate(&mut first, key, v0.content_revision, 1).expect("edit").version.expect("version");
    let w1 = rotate(&mut second, other_key, w0.content_revision, 1).expect("edit").version.expect("version");
    // The second service has a save of its own in flight, with the same plan number as the first one's: a completion
    // from elsewhere must not be able to pass for it.
    let own_plan = second.prepare_save(&ctx(), other_key, w1.content_revision, SaveTarget::Tracked, Overwrite::Tracked, None).expect("plans");

    let plan = first.prepare_save(&ctx(), key, v1.content_revision, SaveTarget::Tracked, Overwrite::Tracked, None).expect("plans");
    let written = plan.write().expect("writes");
    let error = second.finish_save(&written).expect_err("another service's completion");
    assert_eq!(error.code, ErrorCode::InvalidArgument);
    assert!(matches!(error.committed.as_slice(), [Effect::FileWritten { file, .. }] if file == "doc.pdf"), "{:?}", error.committed);
    assert_eq!(second.document(&me(), other_key).expect("open").version(), w1, "the other service's document is untouched, still unsaved");
    // Its own service still finishes it, and the second service's own save is still waiting for its own write.
    saved_report(first.finish_save(&written).expect("finishes at home"));
    assert!(!first.document(&me(), key).expect("open").version().dirty());
    let own_written = own_plan.write().expect("writes");
    saved_report(second.finish_save(&own_written).expect("finishes its own"));
}

#[test]
fn planned_saves_are_bounded_and_can_be_given_up() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = file_in(&dir, "doc.pdf", 1);
    let mut service = Service::new();
    let (key, v0) = open_file(&mut service, &path);
    let v1 = rotate(&mut service, key, v0.content_revision, 1).expect("edit").version.expect("version");
    let mut plans = Vec::new();
    for _ in 0..MAX_SAVES_IN_FLIGHT {
        plans.push(service.prepare_save(&ctx(), key, v1.content_revision, SaveTarget::Tracked, Overwrite::Tracked, None).expect("plans"));
    }
    let error = service.prepare_save(&ctx(), key, v1.content_revision, SaveTarget::Tracked, Overwrite::Tracked, None).expect_err("too many");
    assert_eq!(error.code, ErrorCode::ResourceLimit);
    service.abandon_save(&plans[0]);
    service.prepare_save(&ctx(), key, v1.content_revision, SaveTarget::Tracked, Overwrite::Tracked, None).expect("room again");
    // A save that fails to write gives its place back by itself.
    std::fs::write(&path, fixture(4)).expect("another program changes the file");
    for p in &plans[1..] {
        service.abandon_save(p);
    }
    let failed = save(&mut service, key, v1.content_revision, SaveTarget::Tracked, Overwrite::Tracked).expect_err("conflict");
    assert_eq!(failed.code, ErrorCode::Conflict);
    for _ in 0..=MAX_SAVES_IN_FLIGHT {
        let _ = save(&mut service, key, v1.content_revision, SaveTarget::Tracked, Overwrite::Tracked);
    }
    let again = save(&mut service, key, v1.content_revision, SaveTarget::Tracked, Overwrite::Tracked).expect_err("still a conflict, not a limit");
    assert_eq!(again.code, ErrorCode::Conflict, "failed saves do not use up the places");
}

#[test]
fn a_confined_open_does_not_follow_a_file_link_out_of_the_root() {
    let base = tempfile::tempdir().expect("temp dir");
    let root = base.path().join("root");
    let outside = base.path().join("outside");
    std::fs::create_dir_all(&root).expect("root");
    std::fs::create_dir_all(&outside).expect("outside");
    std::fs::write(outside.join("secret.pdf"), fixture(3)).expect("a document outside");
    std::fs::write(root.join("plain.pdf"), fixture(1)).expect("a document inside");
    if !link_file(&outside.join("secret.pdf"), &root.join("in.pdf")) {
        eprintln!("SKIPPED: this machine cannot create a file symbolic link (Windows needs Developer Mode or an elevated user)");
        return;
    }

    let mut service = Service::confined_to(&root).expect("a folder");
    let error =
        run(&mut service, None, Operation::Open { source: OpenSource::File("in.pdf".into()), password: None }).expect_err("a link is not opened");
    assert!(matches!(error.code, ErrorCode::IoFailure | ErrorCode::InvalidArgument), "{error}");
    assert!(service.documents(&me()).is_empty(), "nothing from outside was opened");
    // An ordinary file in the root still opens, and an unconfined service still follows links as people expect.
    open_file(&mut service, Path::new("plain.pdf"));
    let mut free = Service::new();
    let (linked, _) = open_file(&mut free, &root.join("in.pdf"));
    assert_eq!(free.document(&me(), linked).expect("open").page_count(), 3);
}

/// A file link `at` -> `to`; `false` if this machine will not make one.
fn link_file(to: &Path, at: &Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(to, at).is_ok()
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_file(to, at).is_ok()
    }
}

#[test]
fn the_open_size_limit_refuses_a_large_file_as_a_resource_limit_and_opens_one_at_the_limit() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = file_in(&dir, "doc.pdf", 1);
    let size = std::fs::metadata(&path).expect("metadata").len();
    for (limit, opens) in [(size, true), (size - 1, false), (0, false)] {
        let mut service = Service::new().with_open_limit(limit);
        let result = run(&mut service, None, Operation::Open { source: OpenSource::File(path.clone()), password: None });
        match (opens, result) {
            (true, Ok(_)) => {}
            (false, Err(error)) => {
                assert_eq!(error.code, ErrorCode::ResourceLimit, "limit {limit}");
                assert!(service.documents(&me()).is_empty());
            }
            (expected, other) => panic!("limit {limit}: expected opens={expected}, got {other:?}"),
        }
    }
    // The same through a root, which reads on the validated handle.
    let mut confined = Service::confined_to(dir.path()).expect("a folder").with_open_limit(size - 1);
    let error = run(&mut confined, None, Operation::Open { source: OpenSource::File("doc.pdf".into()), password: None }).expect_err("over");
    assert_eq!(error.code, ErrorCode::ResourceLimit);
    // A limit above the maximum is the maximum.
    assert!(Service::new().with_open_limit(u64::MAX).documents(&me()).is_empty());
}

// ---- reading: list, links, bookmarks ----

fn open_links_fixture(service: &mut Service) -> (DocumentKey, DocumentVersion) {
    let source = OpenSource::Bytes { name: "links.pdf".into(), bytes: Arc::new(fixture_with_links_and_bookmarks()) };
    let done = run(service, None, Operation::Open { source, password: None }).expect("opens");
    match done.outcome {
        Outcome::Opened { document, .. } => (document, done.version.expect("a version")),
        other => panic!("opened a document, got {other:?}"),
    }
}

fn links_of(service: &mut Service, key: DocumentKey) -> LinkList {
    match run(service, Some(key), Operation::Links).expect("lists links").outcome {
        Outcome::Links(list) => list,
        other => panic!("links, got {other:?}"),
    }
}

fn bookmarks_of(service: &mut Service, key: DocumentKey, offset: usize, limit: usize) -> Result<BookmarkList> {
    run(service, Some(key), Operation::Bookmarks { offset, limit }).map(|done| match done.outcome {
        Outcome::Bookmarks(list) => list,
        other => panic!("bookmarks, got {other:?}"),
    })
}

#[test]
fn links_are_listed_with_their_place_and_where_they_go() {
    let mut service = Service::new();
    let (key, _) = open_links_fixture(&mut service);
    let list = links_of(&mut service, key);
    assert_eq!(list.total, 2);
    assert_eq!(list.links.len(), 2);
    // The web link: page 1, the first annotation, in points from the top-left of the displayed page (the page is 300 high).
    let web = &list.links[0];
    assert_eq!((web.page, web.index), (1, 1));
    assert_eq!(web.rect, [10.0, 270.0, 80.0, 290.0]);
    assert_eq!(web.target, LinkTarget::Uri("https://example.com/".into()));
    // The internal link names page 2, 1-based.
    let internal = &list.links[1];
    assert_eq!((internal.page, internal.index), (1, 2));
    assert_eq!(internal.rect, [10.0, 240.0, 80.0, 260.0]);
    assert_eq!(internal.target, LinkTarget::Page(2));
}

#[test]
fn a_document_without_links_or_bookmarks_lists_none() {
    let mut service = Service::new();
    let (key, _) = open_bytes(&mut service, 2);
    let list = links_of(&mut service, key);
    assert_eq!((list.total, list.links.len()), (0, 0));
    let bookmarks = bookmarks_of(&mut service, key, 0, 10).expect("lists");
    assert_eq!((bookmarks.rows.len(), bookmarks.next), (0, None));
}

#[test]
fn bookmarks_come_in_outline_order_a_page_at_a_time() {
    let mut service = Service::new();
    let (key, _) = open_links_fixture(&mut service);
    let first = bookmarks_of(&mut service, key, 0, 1).expect("lists");
    assert_eq!(first.rows, [BookmarkEntry { path: vec![1], title: "Intro".into(), page: Some(1), open: false }]);
    assert_eq!(first.next, Some(1), "there is more");
    let rest = bookmarks_of(&mut service, key, first.next.expect("a next page"), 1).expect("lists");
    assert_eq!(rest.rows, [BookmarkEntry { path: vec![2], title: "Second".into(), page: Some(2), open: false }]);
    assert_eq!(rest.next, None, "that was the last");
    // Everything in one page is the same two rows, and an offset past the end is an empty page, not an error.
    assert_eq!(bookmarks_of(&mut service, key, 0, DEFAULT_BOOKMARK_PAGE).expect("lists").rows.len(), 2);
    assert!(bookmarks_of(&mut service, key, 50, 10).expect("lists").rows.is_empty());
}

#[test]
fn a_bookmark_page_is_bounded_and_a_bad_bound_changes_nothing() {
    let mut service = Service::new();
    let (key, before) = open_links_fixture(&mut service);
    for limit in [0, MAX_BOOKMARK_PAGE + 1, usize::MAX] {
        let error = bookmarks_of(&mut service, key, 0, limit).expect_err("refused");
        assert_eq!(error.code, ErrorCode::InvalidArgument, "limit {limit}");
    }
    assert!(bookmarks_of(&mut service, key, 0, MAX_BOOKMARK_PAGE).is_ok(), "the bound itself is allowed");
    assert!(bookmarks_of(&mut service, key, usize::MAX, 10).expect("an offset past the end").rows.is_empty());
    assert_eq!(service.document(&me(), key).expect("open").version(), before);
}

#[test]
fn listing_names_the_open_documents_and_changes_nothing() {
    let mut service = Service::new();
    let listed = |service: &mut Service| match run(service, None, Operation::List).expect("lists").outcome {
        Outcome::Documents(documents) => documents,
        other => panic!("documents, got {other:?}"),
    };
    assert!(listed(&mut service).is_empty());
    let (first, v1) = open_bytes(&mut service, 1);
    let (second, _) = open_links_fixture(&mut service);
    let documents = listed(&mut service);
    assert_eq!(documents.iter().map(|d| d.document).collect::<Vec<_>>(), [first, second]);
    assert_eq!(documents.iter().map(|d| d.pages.len()).collect::<Vec<_>>(), [1, 2]);
    assert_eq!(service.document(&me(), first).expect("open").version(), v1, "reading moved nothing");
    run(&mut service, Some(first), Operation::Close { expected_revision: v1.content_revision, discard_changes: false }).expect("closes");
    assert_eq!(listed(&mut service).iter().map(|d| d.document).collect::<Vec<_>>(), [second]);
}

#[test]
fn reading_links_and_bookmarks_of_a_closed_or_unknown_document_is_not_found() {
    let mut service = Service::new();
    let (key, v) = open_links_fixture(&mut service);
    let other_session = DocumentKey { session: Service::new().session_id(), document: key.document };
    let unknown = DocumentKey { session: service.session_id(), document: 9_999 };
    run(&mut service, Some(key), Operation::Close { expected_revision: v.content_revision, discard_changes: false }).expect("closes");
    for gone in [key, other_session, unknown] {
        assert_eq!(run(&mut service, Some(gone), Operation::Links).expect_err("refused").code, ErrorCode::NotFound);
        assert_eq!(run(&mut service, Some(gone), Operation::Bookmarks { offset: 0, limit: 10 }).expect_err("refused").code, ErrorCode::NotFound);
    }
}

#[test]
fn a_list_of_links_is_capped_and_says_how_many_there_are() {
    // One page with more links than a request lists.
    let count = MAX_LINKS + 5;
    let annots: Vec<String> = (0..count).map(|i| format!("{} 0 R", 6 + i)).collect();
    let mut objs: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [4 0 R] /Count 1 /MediaBox [0 0 200 300] >>".into(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        format!("<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Resources << /Font << /F1 3 0 R >> >> /Annots [{}] >>", annots.join(" ")),
        "<< /Length 0 >>\nstream\n\nendstream".into(),
    ];
    objs.extend((0..count).map(|_| "<< /Type /Annot /Subtype /Link /Rect [10 10 80 30] /A << /S /URI /URI (https://example.com/) >> >>".to_string()));
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offsets {
        bytes.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());

    let mut service = Service::new();
    let source = OpenSource::Bytes { name: "many.pdf".into(), bytes: Arc::new(bytes) };
    let key = match run(&mut service, None, Operation::Open { source, password: None }).expect("opens").outcome {
        Outcome::Opened { document, .. } => document,
        other => panic!("opened, got {other:?}"),
    };
    let list = links_of(&mut service, key);
    assert_eq!(list.total, count);
    assert_eq!(list.links.len(), MAX_LINKS);
    assert_eq!(list.links[MAX_LINKS - 1].index, MAX_LINKS, "the first ones, in order");
}
