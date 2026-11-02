//! Clients of the service (BUILD-008): what a delegated agent may and may not do, whatever the request says.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pdfcraft_app_service::*;

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

/// A new empty folder only the current user can use (see the crate's own test helper for why).
fn private_dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pdfcraft-agents-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("makes the folder");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).expect("restricts the folder");
    }
    #[cfg(windows)]
    {
        let me = pdfcraft_platform::identity::current_user().expect("a user");
        let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        let status = std::process::Command::new(root.join("System32").join("icacls.exe"))
            .arg(&dir)
            .args(["/inheritance:r", "/grant:r"])
            .arg(format!("*{me}:(OI)(CI)F"))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("runs icacls");
        assert!(status.success(), "icacls could not restrict the test folder");
    }
    dir
}

fn me() -> ActorContext {
    ActorContext::local_user().expect("the current user")
}

fn owner() -> OsPrincipal {
    OsPrincipal::current().expect("the current user")
}

fn human(id: u64) -> RequestContext {
    RequestContext::new(id, me())
}

/// A service that serves the person and the clients `grants` authorize.
fn service_with(grants: &GrantStore) -> Service {
    Service::new().with_policy(Policy::with_grants(grants.clone()))
}

fn grant(grants: &GrantStore, name: &str, capabilities: &[Capability], files: FileAccess) -> (ActorContext, String) {
    let request = GrantRequest {
        client: ClientId::parse(name).expect("a client name"),
        scopes: capabilities.iter().map(|c| Scope { capability: *c, documents: DocumentScope::All }).collect(),
        files,
        ttl_seconds: 3600,
    };
    let issued = grants.issue(request).expect("issues");
    let secret = issued.secret().to_owned();
    (grants.authenticate(name, &secret).expect("authenticates"), secret)
}

fn read_only(grants: &GrantStore, name: &str) -> ActorContext {
    grant(grants, name, &[Capability::DocumentRead], FileAccess::default()).0
}

fn run(service: &mut Service, context: &RequestContext, document: Option<DocumentKey>, operation: Operation) -> Result<OperationResult> {
    service.execute(context, OperationRequest::new(document, operation))
}

fn open_file(service: &mut Service, path: &Path) -> (DocumentKey, DocumentVersion) {
    let done = run(service, &human(1), None, Operation::Open { source: OpenSource::File(path.to_path_buf()), password: None }).expect("opens");
    match done.outcome {
        Outcome::Opened { document, .. } => (document, done.version.expect("version")),
        other => panic!("opened, got {other:?}"),
    }
}

fn rotate(revision: u64) -> Operation {
    Operation::Rotate { expected_revision: revision, pages: PageSelection::Numbers(vec![1]), degrees: 90 }
}

fn rotations(service: &Service, key: DocumentKey) -> Vec<u16> {
    service.document(&me(), key).expect("open").info().pages.iter().map(|p| p.rotation).collect()
}

fn refused(result: Result<OperationResult>) -> (ErrorCode, Option<&'static str>) {
    let error = result.expect_err("refused");
    (error.code, error.reason)
}

fn file_in(dir: &Path, name: &str, pages: usize) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, fixture(pages)).expect("writes the fixture");
    path
}

#[test]
fn a_read_only_agent_reads_and_nothing_else() {
    let dir = private_dir("read-only");
    let path = file_in(&dir, "doc.pdf", 2);
    let grants = GrantStore::in_memory(owner());
    let reader = read_only(&grants, "reader");
    let mut service = service_with(&grants);
    let (key, v0) = open_file(&mut service, &path);
    let on_disk = std::fs::read(&path).expect("reads");
    let ctx = RequestContext::new(10, reader.clone());

    // What it can do: look at the open document, and read what undo would do.
    let info = run(&mut service, &ctx, Some(key), Operation::Query).expect("queries");
    assert!(matches!(info.outcome, Outcome::Info(ref i) if i.pages.len() == 2));
    assert!(run(&mut service, &ctx, Some(key), Operation::History).is_ok());
    assert_eq!(service.documents(&reader), [key]);
    assert_eq!(service.document(&reader, key).expect("reads").page_count(), 2);

    // What it cannot: change, undo, redo, write, close, or read a file nobody gave it.
    let attempts: Vec<(&str, Operation)> = vec![
        ("rotate", rotate(v0.content_revision)),
        ("undo", Operation::Undo { expected_revision: v0.content_revision }),
        ("redo", Operation::Redo { expected_revision: v0.content_revision }),
        ("save", Operation::Save { expected_revision: v0.content_revision, target: SaveTarget::Tracked, overwrite: Overwrite::Tracked, full: None }),
        (
            "save elsewhere",
            Operation::Save {
                expected_revision: v0.content_revision,
                target: SaveTarget::Path(dir.join("copy.pdf")),
                overwrite: Overwrite::Absent,
                full: None,
            },
        ),
        ("close", Operation::Close { expected_revision: v0.content_revision, discard_changes: true }),
    ];
    for (what, operation) in attempts {
        let (code, reason) = refused(run(&mut service, &ctx, Some(key), operation));
        assert_eq!(code, ErrorCode::NotAuthorized, "{what}");
        assert!(matches!(reason, Some("capability_not_granted" | "history_not_permitted")), "{what}: {reason:?}");
    }
    let (code, reason) = refused(run(&mut service, &ctx, None, Operation::Open { source: OpenSource::File(path.clone()), password: None }));
    assert_eq!((code, reason), (ErrorCode::NotAuthorized, Some("folder_not_granted")));
    // And the person's document is untouched: same version, same pages, same file, no copy written.
    assert_eq!(service.document(&me(), key).expect("open").version(), v0);
    assert_eq!(rotations(&service, key), [0, 0]);
    assert_eq!(std::fs::read(&path).expect("reads"), on_disk);
    assert!(!dir.join("copy.pdf").exists());
    assert_eq!(service.documents(&me()), [key]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn undo_and_redo_need_the_capability_of_the_change_they_revert() {
    let dir = private_dir("undo");
    let path = file_in(&dir, "doc.pdf", 2);
    let grants = GrantStore::in_memory(owner());
    let reader = read_only(&grants, "reader");
    let (editor, _) = grant(&grants, "editor", &[Capability::DocumentRead, Capability::DocumentEditPages], FileAccess::default());
    let mut service = service_with(&grants);
    let (key, v0) = open_file(&mut service, &path);

    // The person rotates a page. A client that can only read cannot undo it, and so cannot make it disappear.
    let v1 = run(&mut service, &human(2), Some(key), rotate(v0.content_revision)).expect("a human edit").version.expect("version");
    let reader_ctx = RequestContext::new(3, reader);
    let (code, reason) = refused(run(&mut service, &reader_ctx, Some(key), Operation::Undo { expected_revision: v1.content_revision }));
    assert_eq!((code, reason), (ErrorCode::NotAuthorized, Some("history_not_permitted")));
    assert_eq!(rotations(&service, key), [90, 0], "still rotated");

    // A client that may edit pages may undo and redo a page edit, and only that capability makes it possible.
    let editor_ctx = RequestContext::new(4, editor.clone());
    let v2 = run(&mut service, &editor_ctx, Some(key), Operation::Undo { expected_revision: v1.content_revision })
        .expect("undo")
        .version
        .expect("version");
    assert_eq!(rotations(&service, key), [0, 0]);
    let (code, _) = refused(run(&mut service, &reader_ctx, Some(key), Operation::Redo { expected_revision: v2.content_revision }));
    assert_eq!(code, ErrorCode::NotAuthorized, "a reader cannot redo the change the editor undid");
    run(&mut service, &editor_ctx, Some(key), Operation::Redo { expected_revision: v2.content_revision }).expect("redo");
    assert_eq!(rotations(&service, key), [90, 0]);

    // An edit the client made is one only a client that could make it can undo.
    let v3 = service.document(&me(), key).expect("open").version();
    let v4 = run(&mut service, &editor_ctx, Some(key), rotate(v3.content_revision)).expect("an agent edit").version.expect("version");
    assert_eq!(rotations(&service, key), [180, 0]);
    let (code, _) = refused(run(&mut service, &reader_ctx, Some(key), Operation::Undo { expected_revision: v4.content_revision }));
    assert_eq!(code, ErrorCode::NotAuthorized);
    // The person can undo anything.
    run(&mut service, &human(5), Some(key), Operation::Undo { expected_revision: v4.content_revision }).expect("the person undoes it");
    assert_eq!(rotations(&service, key), [90, 0]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_grant_is_limited_to_its_documents_and_the_refusal_does_not_say_whether_one_exists() {
    let dir = private_dir("scope");
    let grants = GrantStore::in_memory(owner());
    let mut service = service_with(&grants);
    let (first, _) = open_file(&mut service, &file_in(&dir, "a.pdf", 1));
    let (second, _) = open_file(&mut service, &file_in(&dir, "b.pdf", 3));
    let request = GrantRequest {
        scopes: vec![Scope { capability: Capability::DocumentRead, documents: DocumentScope::Only(vec![first]) }],
        ..GrantRequest::read_only(ClientId::parse("narrow").expect("a name"))
    };
    let issued = grants.issue(request).expect("issues");
    let narrow = grants.authenticate("narrow", issued.secret()).expect("authenticates");
    let ctx = RequestContext::new(2, narrow.clone());

    assert!(run(&mut service, &ctx, Some(first), Operation::Query).is_ok());
    assert_eq!(service.documents(&narrow), [first]);
    let elsewhere = refused(run(&mut service, &ctx, Some(second), Operation::Query));
    assert_eq!(elsewhere, (ErrorCode::NotAuthorized, Some("document_out_of_scope")));
    // A document that does not exist, and one of another session, get the very same answer as one that exists.
    let missing = DocumentKey { session: service.session_id(), document: 9_999 };
    let foreign = DocumentKey { session: Service::new().session_id(), document: first.document };
    assert_eq!(refused(run(&mut service, &ctx, Some(missing), Operation::Query)), elsewhere);
    assert_eq!(refused(run(&mut service, &ctx, Some(foreign), Operation::Query)), elsewhere);
    assert_eq!(service.document(&narrow, second).err().map(|e| (e.code, e.reason)), Some((ErrorCode::NotAuthorized, Some("document_out_of_scope"))));
    // The person is told the truth.
    assert_eq!(refused(run(&mut service, &human(3), Some(missing), Operation::Query)).0, ErrorCode::NotFound);
    let _ = std::fs::remove_dir_all(&dir);
}

fn open_links(service: &mut Service) -> DocumentKey {
    let source = OpenSource::Bytes { name: "links.pdf".into(), bytes: Arc::new(fixture_with_links_and_bookmarks()) };
    match run(service, &human(1), None, Operation::Open { source, password: None }).expect("opens").outcome {
        Outcome::Opened { document, .. } => document,
        other => panic!("opened, got {other:?}"),
    }
}

#[test]
fn a_client_lists_reads_links_and_bookmarks_only_of_the_documents_it_was_granted() {
    let grants = GrantStore::in_memory(owner());
    let mut service = service_with(&grants);
    let first = open_links(&mut service);
    let second = open_links(&mut service);
    let request = GrantRequest {
        scopes: vec![Scope { capability: Capability::DocumentRead, documents: DocumentScope::Only(vec![first]) }],
        ..GrantRequest::read_only(ClientId::parse("narrow").expect("a name"))
    };
    let issued = grants.issue(request).expect("issues");
    let narrow = grants.authenticate("narrow", issued.secret()).expect("authenticates");
    let ctx = RequestContext::new(2, narrow);

    // The list names the granted document and does not mention the other.
    match run(&mut service, &ctx, None, Operation::List).expect("lists").outcome {
        Outcome::Documents(documents) => assert_eq!(documents.iter().map(|d| d.document).collect::<Vec<_>>(), [first]),
        other => panic!("documents, got {other:?}"),
    }
    assert!(matches!(run(&mut service, &ctx, Some(first), Operation::Links).expect("links").outcome, Outcome::Links(ref l) if l.total == 2));
    assert!(matches!(
        run(&mut service, &ctx, Some(first), Operation::Bookmarks { offset: 0, limit: 10 }).expect("bookmarks").outcome,
        Outcome::Bookmarks(ref b) if b.rows.len() == 2
    ));
    // The other document, one that does not exist and one of another session all get the one answer.
    let missing = DocumentKey { session: service.session_id(), document: 9_999 };
    let foreign = DocumentKey { session: Service::new().session_id(), document: first.document };
    for operation in [Operation::Links, Operation::Bookmarks { offset: 0, limit: 10 }] {
        for key in [second, missing, foreign] {
            assert_eq!(refused(run(&mut service, &ctx, Some(key), operation.clone())), (ErrorCode::NotAuthorized, Some("document_out_of_scope")));
        }
    }
    // The person still sees both.
    match run(&mut service, &human(3), None, Operation::List).expect("lists").outcome {
        Outcome::Documents(documents) => assert_eq!(documents.len(), 2),
        other => panic!("documents, got {other:?}"),
    }
}

#[test]
fn a_client_that_was_not_granted_reading_gets_no_list_links_or_bookmarks() {
    let grants = GrantStore::in_memory(owner());
    let mut service = service_with(&grants);
    let key = open_links(&mut service);
    let (editor, _) = grant(&grants, "editor", &[Capability::DocumentEditPages], FileAccess::default());
    let ctx = RequestContext::new(2, editor);
    for operation in [Operation::List, Operation::Links, Operation::Bookmarks { offset: 0, limit: 10 }] {
        let document = if matches!(operation, Operation::List) { None } else { Some(key) };
        assert_eq!(refused(run(&mut service, &ctx, document, operation)), (ErrorCode::NotAuthorized, Some("capability_not_granted")));
    }
}

#[test]
fn reading_links_and_bookmarks_is_audited_without_what_was_read() {
    let grants = GrantStore::in_memory(owner());
    let mut service = service_with(&grants);
    let key = open_links(&mut service);
    let reader = read_only(&grants, "reader");
    let ctx = RequestContext::new(2, reader);
    run(&mut service, &ctx, None, Operation::List).expect("lists");
    run(&mut service, &ctx, Some(key), Operation::Links).expect("links");
    run(&mut service, &ctx, Some(key), Operation::Bookmarks { offset: 0, limit: 10 }).expect("bookmarks");
    run(&mut service, &ctx, Some(key), Operation::Bookmarks { offset: 0, limit: 0 }).expect_err("a bad bound");
    let records = service.recent_audit();
    let by_agent: Vec<(&str, &AuditOutcome)> =
        records.iter().filter(|r| r.entry.actor.as_str() == "agent").map(|r| (r.entry.operation.as_str(), &r.entry.outcome)).collect();
    assert_eq!(
        by_agent,
        [
            ("document.list", &AuditOutcome::Ok),
            ("document.links", &AuditOutcome::Ok),
            ("document.bookmarks", &AuditOutcome::Ok),
            ("document.bookmarks", &AuditOutcome::Failed("invalid_argument".into())),
        ]
    );
    let everything = format!("{records:?}");
    for read in ["Intro", "Second", "example.com", "links.pdf"] {
        assert!(!everything.contains(read), "the record does not hold {read:?}");
    }
}

#[test]
fn a_grant_for_one_documents_does_not_reach_the_same_number_in_another_session() {
    let dir = private_dir("session-scope");
    let grants = GrantStore::in_memory(owner());
    let (mut mine, mut theirs) = (service_with(&grants), service_with(&grants));
    let (mine_doc, mine_version) = open_file(&mut mine, &file_in(&dir, "mine.pdf", 2));
    let (theirs_doc, theirs_version) = open_file(&mut theirs, &file_in(&dir, "theirs.pdf", 3));
    assert_eq!(mine_doc.document, theirs_doc.document, "both sessions number their first document the same way");
    assert_ne!(mine_doc.session, theirs_doc.session);

    let request = GrantRequest {
        scopes: [Capability::DocumentRead, Capability::DocumentEditPages]
            .into_iter()
            .map(|capability| Scope { capability, documents: DocumentScope::Only(vec![mine_doc]) })
            .collect(),
        ..GrantRequest::read_only(ClientId::parse("one-document").expect("a name"))
    };
    let issued = grants.issue(request).expect("issues");
    let agent = grants.authenticate("one-document", issued.secret()).expect("authenticates");
    let ctx = RequestContext::new(2, agent.clone());

    // Its own document, yes.
    assert!(run(&mut mine, &ctx, Some(mine_doc), Operation::Query).is_ok());
    assert!(run(&mut mine, &ctx, Some(mine_doc), rotate(mine_version.content_revision)).is_ok());
    // The other session's document with the same number: neither read nor edited, and nothing about it is revealed.
    let denied = (ErrorCode::NotAuthorized, Some("document_out_of_scope"));
    assert_eq!(refused(run(&mut theirs, &ctx, Some(theirs_doc), Operation::Query)), denied);
    assert_eq!(refused(run(&mut theirs, &ctx, Some(theirs_doc), rotate(theirs_version.content_revision))), denied);
    assert_eq!(theirs.documents(&agent), []);
    assert_eq!(theirs.document(&agent, theirs_doc).err().map(|e| (e.code, e.reason)), Some(denied));
    assert_eq!(rotations(&theirs, theirs_doc), [0, 0, 0], "the other session's document is untouched");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_agent_reads_and_writes_files_only_in_its_folders() {
    let base = private_dir("folders");
    let (inside, outside) = (base.join("inside"), base.join("outside"));
    std::fs::create_dir_all(&inside).expect("folder");
    std::fs::create_dir_all(&outside).expect("folder");
    let allowed = file_in(&inside, "in.pdf", 2);
    let forbidden = file_in(&outside, "out.pdf", 2);
    let grants = GrantStore::in_memory(owner());
    let files = FileAccess { read_under: vec![inside.clone()], write_under: vec![inside.clone()] };
    let (worker, _) = grant(&grants, "worker", &[Capability::DocumentRead, Capability::DocumentEditPages, Capability::DocumentSave], files);
    let ctx = RequestContext::new(2, worker);
    let mut service = service_with(&grants);

    let opened = run(&mut service, &ctx, None, Operation::Open { source: OpenSource::File(allowed.clone()), password: None }).expect("opens inside");
    let (key, v0) = match opened.outcome {
        Outcome::Opened { document, .. } => (document, opened.version.expect("version")),
        other => panic!("opened, got {other:?}"),
    };
    assert_eq!(
        refused(run(&mut service, &ctx, None, Operation::Open { source: OpenSource::File(forbidden.clone()), password: None })),
        (ErrorCode::NotAuthorized, Some("folder_not_granted"))
    );
    let climbing = inside.join("..").join("outside").join("out.pdf");
    assert_eq!(
        refused(run(&mut service, &ctx, None, Operation::Open { source: OpenSource::File(climbing), password: None })),
        (ErrorCode::NotAuthorized, Some("folder_not_granted")),
        "a path that climbs out is not inside"
    );
    let v1 = run(&mut service, &ctx, Some(key), rotate(v0.content_revision)).expect("edits").version.expect("version");
    // Saving into the folder works; saving anywhere else is refused and writes nothing.
    let save = |target: SaveTarget, overwrite: Overwrite| Operation::Save { expected_revision: v1.content_revision, target, overwrite, full: None };
    let stray = outside.join("copy.pdf");
    assert_eq!(
        refused(run(&mut service, &ctx, Some(key), save(SaveTarget::Path(stray.clone()), Overwrite::Absent))),
        (ErrorCode::NotAuthorized, Some("folder_not_granted"))
    );
    assert!(!stray.exists());
    run(&mut service, &ctx, Some(key), save(SaveTarget::Path(inside.join("copy.pdf")), Overwrite::Absent)).expect("saves inside");
    assert!(inside.join("copy.pdf").exists());
    // A document the person opened from outside the folder cannot be saved in place by the client: its file is not the client's to write.
    let (theirs, t0) = open_file(&mut service, &forbidden);
    let before = std::fs::read(&forbidden).expect("reads");
    let t1 = run(&mut service, &human(9), Some(theirs), rotate(t0.content_revision)).expect("a human edit").version.expect("version");
    let tracked = Operation::Save { expected_revision: t1.content_revision, target: SaveTarget::Tracked, overwrite: Overwrite::Tracked, full: None };
    assert_eq!(refused(run(&mut service, &ctx, Some(theirs), tracked)), (ErrorCode::NotAuthorized, Some("folder_not_granted")));
    assert_eq!(std::fs::read(&forbidden).expect("reads"), before);
    let _ = std::fs::remove_dir_all(&base);
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

/// A file link `at` -> `to`; `false` if this machine will not make one (Windows without the privilege).
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

/// Two folders, `inside` (granted) and `outside` (not), a decoy in `outside`, and a client that may read and save in `inside`.
fn granted_and_not(test: &str) -> (PathBuf, PathBuf, PathBuf, GrantStore, ActorContext) {
    let base = private_dir(test);
    let (inside, outside) = (base.join("inside"), base.join("outside"));
    std::fs::create_dir_all(&inside).expect("folder");
    std::fs::create_dir_all(&outside).expect("folder");
    file_in(&outside, "out.pdf", 4);
    let grants = GrantStore::in_memory(owner());
    let files = FileAccess { read_under: vec![inside.clone()], write_under: vec![inside.clone()] };
    let (worker, _) = grant(&grants, "worker", &[Capability::DocumentRead, Capability::DocumentEditPages, Capability::DocumentSave], files);
    (base, inside, outside, grants, worker)
}

#[test]
fn a_link_inside_a_granted_folder_does_not_carry_a_client_out_of_it_when_reading() {
    let (base, inside, outside, grants, worker) = granted_and_not("link-read");
    let mut service = service_with(&grants);
    let ctx = RequestContext::new(2, worker);
    link_dir(&outside, &inside.join("alias"));
    let open = |service: &mut Service, ctx: &RequestContext, path: PathBuf| {
        run(service, ctx, None, Operation::Open { source: OpenSource::File(path), password: None })
    };

    // Named inside the granted folder, but the folder on the way is a link to one that is not granted.
    let error = open(&mut service, &ctx, inside.join("alias").join("out.pdf")).expect_err("refused");
    assert_eq!(error.code, ErrorCode::IoFailure, "{error}");
    assert!(service.documents(&me()).is_empty(), "nothing was opened");
    // And so is a link to a single file.
    if link_file(&outside.join("out.pdf"), &inside.join("linked.pdf")) {
        assert!(open(&mut service, &ctx, inside.join("linked.pdf")).is_err());
        assert!(service.documents(&me()).is_empty());
    }
    // The refusal says nothing about what the link points at.
    let text = format!("{error:?}");
    assert!(!text.contains("outside") && !text.contains("pdfcraft-agents"), "{text}");
    // A real file in the granted folder still opens for the client...
    file_in(&inside, "in.pdf", 2);
    assert!(open(&mut service, &ctx, inside.join("in.pdf")).is_ok());
    // ...and the person is not held to a grant: links work for them as they always did.
    assert!(open(&mut service, &human(3), inside.join("alias").join("out.pdf")).is_ok());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_link_inside_a_granted_folder_does_not_carry_a_clients_save_out_of_it() {
    let (base, inside, outside, grants, worker) = granted_and_not("link-write");
    let mut service = service_with(&grants);
    let ctx = RequestContext::new(2, worker);
    let (key, version) = open_file(&mut service, &file_in(&inside, "in.pdf", 2));
    let decoy = std::fs::read(outside.join("out.pdf")).expect("reads");
    link_dir(&outside, &inside.join("alias"));
    let save = |service: &mut Service, target: PathBuf, overwrite: Overwrite| {
        run(
            service,
            &ctx,
            Some(key),
            Operation::Save { expected_revision: version.content_revision, target: SaveTarget::Path(target), overwrite, full: None },
        )
    };

    // A new file through the link, and the replacement of the file the link leads to (confirmed by its real fingerprint,
    // so that only the link stands in the way).
    let error = save(&mut service, inside.join("alias").join("new.pdf"), Overwrite::Absent).expect_err("refused");
    assert_eq!(error.code, ErrorCode::IoFailure, "{error}");
    assert!(!error.message.contains("inside") && !error.message.contains("outside"), "the message leaks the folder: {}", error.message);
    let fingerprint = pdfcraft_platform::file_identity::FileVersion::of_bytes(&decoy, None).fingerprint();
    assert!(save(&mut service, inside.join("alias").join("out.pdf"), Overwrite::Version(fingerprint)).is_err());
    assert!(!outside.join("new.pdf").exists(), "nothing was created outside the granted folder");
    assert_eq!(std::fs::read(outside.join("out.pdf")).expect("reads"), decoy, "and nothing there was replaced");
    // Saving where the grant says still works, new subfolders and all.
    assert!(save(&mut service, inside.join("sub").join("ok.pdf"), Overwrite::Absent).is_ok());
    assert!(inside.join("sub").join("ok.pdf").exists());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_granted_folder_swapped_for_a_link_between_open_and_save_is_not_written_through() {
    let (base, inside, outside, grants, worker) = granted_and_not("link-swap");
    let mut service = service_with(&grants);
    let ctx = RequestContext::new(2, worker);
    std::fs::create_dir_all(inside.join("work")).expect("folder");
    let (key, v0) = open_file(&mut service, &file_in(&inside.join("work"), "in.pdf", 2));
    let v1 = run(&mut service, &ctx, Some(key), rotate(v0.content_revision)).expect("an agent edit").version.expect("version");
    let decoy = fixture(2);
    std::fs::write(outside.join("in.pdf"), &decoy).expect("a byte-identical decoy, so that a version check alone could not tell");

    std::fs::rename(inside.join("work"), inside.join("work-moved")).expect("moves the folder");
    link_dir(&outside, &inside.join("work"));
    let saved = run(
        &mut service,
        &ctx,
        Some(key),
        Operation::Save { expected_revision: v1.content_revision, target: SaveTarget::Tracked, overwrite: Overwrite::Tracked, full: None },
    );
    assert!(saved.is_err());
    assert_eq!(std::fs::read(outside.join("in.pdf")).expect("reads"), decoy, "nothing was written through the link");
    assert!(service.document(&me(), key).expect("open").version().dirty(), "and the edit is still held");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_service_without_the_grants_refuses_every_client() {
    let theirs = GrantStore::in_memory(owner());
    let stranger = read_only(&theirs, "stranger");
    let mut alone = Service::new();
    let source = OpenSource::Bytes { name: "a.pdf".into(), bytes: Arc::new(fixture(1)) };
    let key = match run(&mut alone, &human(1), None, Operation::Open { source, password: None }).expect("opens").outcome {
        Outcome::Opened { document, .. } => document,
        other => panic!("opened, got {other:?}"),
    };
    let ctx = RequestContext::new(2, stranger.clone());
    assert_eq!(refused(run(&mut alone, &ctx, Some(key), Operation::Query)), (ErrorCode::NotAuthorized, Some("grant_unknown")));
    assert!(alone.documents(&stranger).is_empty());
    // A service that holds a different store does not know the grant either.
    let mut other = service_with(&GrantStore::in_memory(owner()));
    let key = match run(
        &mut other,
        &human(1),
        None,
        Operation::Open { source: OpenSource::Bytes { name: "a.pdf".into(), bytes: Arc::new(fixture(1)) }, password: None },
    )
    .expect("opens")
    .outcome
    {
        Outcome::Opened { document, .. } => document,
        other => panic!("opened, got {other:?}"),
    };
    assert_eq!(refused(run(&mut other, &ctx, Some(key), Operation::Query)).0, ErrorCode::NotAuthorized);
}

#[test]
fn every_outcome_is_audited_without_content_paths_or_secrets() {
    let dir = private_dir("audit");
    let path = file_in(&dir, "doc.pdf", 2);
    let grants = GrantStore::in_memory(owner());
    let (reader, secret) = grant(&grants, "reader", &[Capability::DocumentRead], FileAccess::default());
    let mut service = service_with(&grants).with_clock(|| 1_700_000_000);
    let (key, v0) = open_file(&mut service, &path);
    let v1 = run(&mut service, &human(2), Some(key), rotate(v0.content_revision)).expect("edit").version.expect("version");
    run(&mut service, &RequestContext::new(3, reader.clone()), Some(key), Operation::Query).expect("queries");
    let denied = run(&mut service, &RequestContext::new(4, reader.clone()), Some(key), rotate(v1.content_revision));
    assert!(denied.is_err());
    let save = Operation::Save { expected_revision: v1.content_revision, target: SaveTarget::Tracked, overwrite: Overwrite::Tracked, full: None };
    run(&mut service, &human(5), Some(key), save).expect("saves");
    run(&mut service, &human(6), Some(key), rotate(0)).expect_err("a stale request");

    let records = service.recent_audit();
    let summary: Vec<(u64, &str, &str, &AuditOutcome)> =
        records.iter().map(|r| (r.entry.request_id, r.entry.actor.as_str(), r.entry.operation.as_str(), &r.entry.outcome)).collect();
    assert_eq!(
        summary,
        [
            (1, "human", "document.open", &AuditOutcome::Ok),
            (2, "human", "page.rotate", &AuditOutcome::Ok),
            (3, "agent", "document.query", &AuditOutcome::Ok),
            (4, "agent", "page.rotate", &AuditOutcome::Denied("capability_not_granted".into())),
            (5, "human", "document.save", &AuditOutcome::Ok),
            (6, "human", "page.rotate", &AuditOutcome::Failed("conflict".into())),
        ]
    );
    assert_eq!(records.iter().map(|r| r.seq).collect::<Vec<_>>(), [1, 2, 3, 4, 5, 6]);
    // Who: a client's name and grant, never anything the request said.
    assert_eq!(records[2].entry.client.as_deref(), Some("reader"));
    assert_eq!(records[2].entry.grant.as_deref(), reader.grant().map(GrantId::as_str));
    assert_eq!((records[0].entry.client.clone(), records[0].entry.grant.clone()), (None, None));
    // Which document and in what state: revisions before and after, the same for a refusal.
    assert_eq!((records[1].entry.revision_before, records[1].entry.revision_after), (Some(v0.content_revision), Some(v1.content_revision)));
    assert_eq!((records[3].entry.revision_before, records[3].entry.revision_after), (Some(v1.content_revision), Some(v1.content_revision)));
    assert_eq!(records[1].entry.document, Some(key.document));
    // What it did outside memory: the file's name and size, nothing else.
    assert!(
        matches!(records[4].entry.effects.as_slice(), [AuditEffect { kind, file: Some(file), bytes: Some(_) }] if kind == "file_written" && file == "doc.pdf")
    );
    assert!(records.iter().all(|r| r.entry.at == 1_700_000_000));
    // And nothing that must not be there.
    let text = format!("{records:?}");
    for forbidden in [dir.to_string_lossy().as_ref(), secret.as_str(), "bdfg_", "Page 1", "%PDF"] {
        assert!(!text.contains(forbidden), "the audit log holds {forbidden:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A sink that stops working after `allowed` records.
struct Failing {
    allowed: usize,
    seen: usize,
}

impl AuditSink for Failing {
    fn record(&mut self, entry: AuditEntry) -> std::result::Result<AuditRecord, pdfcraft_app_service::audit::AuditError> {
        self.seen += 1;
        if self.seen > self.allowed {
            return Err(pdfcraft_app_service::audit::AuditError("the disk is full".into()));
        }
        Ok(AuditRecord { seq: self.seen as u64, entry })
    }
}

#[test]
fn a_log_that_cannot_be_written_stops_serving_clients_but_not_the_person() {
    let grants = GrantStore::in_memory(owner());
    let reader = read_only(&grants, "reader");
    let mut service = service_with(&grants).with_audit(Box::new(Failing { allowed: 1, seen: 0 }));
    let source = OpenSource::Bytes { name: "a.pdf".into(), bytes: Arc::new(fixture(1)) };
    let key = match run(&mut service, &human(1), None, Operation::Open { source, password: None }).expect("opens").outcome {
        Outcome::Opened { document, .. } => document,
        other => panic!("opened, got {other:?}"),
    };
    assert!(!service.audit_failed());
    // The next record fails. The person's request still succeeds (their documents are theirs), and from then on a client is refused.
    assert!(run(&mut service, &human(2), Some(key), Operation::Query).is_ok());
    assert!(service.audit_failed());
    let ctx = RequestContext::new(3, reader);
    assert_eq!(refused(run(&mut service, &ctx, Some(key), Operation::Query)), (ErrorCode::NotAuthorized, Some("audit_unavailable")));
    assert!(run(&mut service, &human(4), Some(key), Operation::Query).is_ok(), "the person is not locked out by a full disk");
    // A working log serves clients again.
    let mut service = service.with_audit(Box::new(MemoryAudit::new(8)));
    assert!(!service.audit_failed());
    assert!(run(&mut service, &ctx, Some(key), Operation::Query).is_ok());
}

/// A service whose log fails on its second record, with one document open (record one) and an agent that may save into `dir`.
fn failing_log_with_saver(dir: &Path) -> (Service, DocumentKey, DocumentVersion, ActorContext) {
    let grants = GrantStore::in_memory(owner());
    let files = FileAccess { read_under: vec![dir.to_path_buf()], write_under: vec![dir.to_path_buf()] };
    let (saver, _) = grant(&grants, "saver", &[Capability::DocumentRead, Capability::DocumentSave], files);
    let mut service = service_with(&grants).with_audit(Box::new(Failing { allowed: 1, seen: 0 }));
    let (key, version) = open_file(&mut service, &file_in(dir, "doc.pdf", 1));
    (service, key, version, saver)
}

fn break_the_log(service: &mut Service, key: DocumentKey) {
    assert!(run(service, &human(2), Some(key), Operation::Query).is_ok(), "the person still succeeds");
    assert!(service.audit_failed());
}

#[test]
fn once_the_log_has_failed_a_client_cannot_read_a_document_by_the_direct_calls_either() {
    let dir = private_dir("audit-direct-read");
    let (mut service, key, _, saver) = failing_log_with_saver(&dir);
    assert_eq!(service.documents(&saver), [key]);
    assert!(service.document(&saver, key).is_ok());
    break_the_log(&mut service, key);

    assert!(service.documents(&saver).is_empty());
    assert_eq!(service.document(&saver, key).err().map(|e| (e.code, e.reason)), Some((ErrorCode::NotAuthorized, Some("audit_unavailable"))));
    assert_eq!(service.documents(&me()), [key], "the person is not locked out");
    assert!(service.document(&me(), key).is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn once_the_log_has_failed_a_client_cannot_plan_or_write_a_save_and_the_person_can() {
    let dir = private_dir("audit-direct-save");
    let (mut service, key, version, saver) = failing_log_with_saver(&dir);
    let out = dir.join("out.pdf");
    let ctx = RequestContext::new(5, saver);
    // A plan made while the log works...
    let queued = service
        .prepare_save(&ctx, key, version.content_revision, SaveTarget::Path(out.clone()), Overwrite::Absent, None)
        .expect("the client may plan a save while the log works");
    break_the_log(&mut service, key);

    // ...is not written once it does not: nothing is staged and nothing appears on disk.
    let error = queued.write().expect_err("refused");
    assert_eq!((error.code, error.reason), (ErrorCode::NotAuthorized, Some("audit_unavailable")));
    assert!(!out.exists(), "the refused write left nothing behind");
    service.abandon_save(&queued);
    // A new plan is refused outright, and the refusal does not look at the document.
    let again = service.prepare_save(&ctx, key, version.content_revision, SaveTarget::Path(out.clone()), Overwrite::Absent, None);
    assert_eq!(again.err().map(|e| (e.code, e.reason)), Some((ErrorCode::NotAuthorized, Some("audit_unavailable"))));
    assert!(!out.exists());
    // The person saves whatever the state of the log.
    let mine = service
        .prepare_save(&human(6), key, version.content_revision, SaveTarget::Path(out.clone()), Overwrite::Absent, None)
        .expect("the person may plan");
    assert!(mine.write().is_ok());
    assert!(out.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_log_on_disk_records_what_was_done_and_verifies() {
    let dir = private_dir("disk-audit");
    let log = dir.join("audit");
    let path = file_in(&dir, "doc.pdf", 1);
    let grants = GrantStore::in_memory(owner());
    let reader = read_only(&grants, "reader");
    let mut service = service_with(&grants).with_audit(Box::new(FileAudit::open(&log).expect("opens the log")));
    let (key, v0) = open_file(&mut service, &path);
    run(&mut service, &RequestContext::new(2, reader), Some(key), Operation::Query).expect("queries");
    run(&mut service, &human(3), Some(key), rotate(v0.content_revision)).expect("edits");
    drop(service);
    let report = pdfcraft_app_service::audit::verify(&log).expect("verifies");
    assert_eq!((report.records, report.first_bad), (3, None));
    let text = std::fs::read_to_string(log.join("audit-000001.jsonl")).expect("reads");
    assert!(text.contains("\"client\":\"reader\"") && text.contains("\"op\":\"page.rotate\""), "{text}");
    assert!(!text.contains(&*dir.to_string_lossy()), "no full path reaches the log");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn document_scripts_are_off_unless_someone_has_decided_to_trust_them() {
    assert!(!Service::new().scripts_enabled(), "off by default");
    assert!(!Service::default().scripts_enabled());
    assert!(!Service::new().with_script_trust(ScriptTrust::Disabled).scripts_enabled());
    assert!(Service::new().with_script_trust(ScriptTrust::Enabled).scripts_enabled(), "trust is explicit");
    assert!(!Service::new().with_script_trust(ScriptTrust::Enabled).with_script_trust(ScriptTrust::Disabled).scripts_enabled());
    let grants = GrantStore::in_memory(owner());
    assert!(!service_with(&grants).scripts_enabled(), "granting a client access does not turn scripts on");
    assert_eq!(ScriptTrust::default(), ScriptTrust::Disabled);
}

fn open_notes(service: &mut Service) -> DocumentKey {
    let source = OpenSource::Bytes { name: "notes.pdf".into(), bytes: Arc::new(fixture(2)) };
    match run(service, &human(1), None, Operation::Open { source, password: None }).expect("opens").outcome {
        Outcome::Opened { document, .. } => document,
        other => panic!("opened, got {other:?}"),
    }
}

fn text_ops() -> [Operation; 2] {
    [
        Operation::TextExtract { pages: PageSelection::All, rect: None },
        Operation::TextFind { query: "Page 2".into(), limit: 10, start_page: 1, skip: 0 },
    ]
}

#[test]
fn a_client_reads_and_searches_text_only_in_the_documents_it_was_granted() {
    let grants = GrantStore::in_memory(owner());
    let mut service = service_with(&grants);
    let first = open_notes(&mut service);
    let second = open_notes(&mut service);
    let request = GrantRequest {
        scopes: vec![Scope { capability: Capability::DocumentRead, documents: DocumentScope::Only(vec![first]) }],
        ..GrantRequest::read_only(ClientId::parse("narrow").expect("a name"))
    };
    let issued = grants.issue(request).expect("issues");
    let narrow = grants.authenticate("narrow", issued.secret()).expect("authenticates");
    let ctx = RequestContext::new(2, narrow);

    match run(&mut service, &ctx, Some(first), text_ops()[0].clone()).expect("reads").outcome {
        Outcome::Text(text) => assert_eq!(text.pages.iter().map(|p| p.text.as_str()).collect::<Vec<_>>(), ["Page 1", "Page 2"]),
        other => panic!("text, got {other:?}"),
    }
    match run(&mut service, &ctx, Some(first), text_ops()[1].clone()).expect("searches").outcome {
        Outcome::Found(report) => assert_eq!(report.matches.len(), 1),
        other => panic!("found, got {other:?}"),
    }
    // The other document, one that does not exist and one of another session all get the one answer, and what the
    // person (or a wider grant) has already read of the other document is not served from the cache to this client.
    run(&mut service, &human(3), Some(second), text_ops()[0].clone()).expect("the person reads it");
    let missing = DocumentKey { session: service.session_id(), document: 9_999 };
    let foreign = DocumentKey { session: Service::new().session_id(), document: first.document };
    for operation in text_ops() {
        for key in [second, missing, foreign] {
            assert_eq!(refused(run(&mut service, &ctx, Some(key), operation.clone())), (ErrorCode::NotAuthorized, Some("document_out_of_scope")));
        }
    }
}

#[test]
fn a_client_that_was_not_granted_reading_gets_no_text() {
    let grants = GrantStore::in_memory(owner());
    let mut service = service_with(&grants);
    let key = open_notes(&mut service);
    let (editor, _) = grant(&grants, "editor", &[Capability::DocumentEditPages], FileAccess::default());
    let ctx = RequestContext::new(2, editor);
    for operation in text_ops() {
        assert_eq!(refused(run(&mut service, &ctx, Some(key), operation)), (ErrorCode::NotAuthorized, Some("capability_not_granted")));
    }
}

#[test]
fn reading_and_searching_text_is_audited_without_the_text_or_the_phrase() {
    let grants = GrantStore::in_memory(owner());
    let mut service = service_with(&grants);
    let key = open_notes(&mut service);
    let ctx = RequestContext::new(2, read_only(&grants, "reader"));
    for operation in text_ops() {
        run(&mut service, &ctx, Some(key), operation).expect("runs");
    }
    run(&mut service, &ctx, Some(key), Operation::TextFind { query: "Page 2".into(), limit: 0, start_page: 1, skip: 0 }).expect_err("a bad bound");
    let records = service.recent_audit();
    let by_agent: Vec<(&str, &AuditOutcome)> =
        records.iter().filter(|r| r.entry.actor.as_str() == "agent").map(|r| (r.entry.operation.as_str(), &r.entry.outcome)).collect();
    assert_eq!(
        by_agent,
        [
            ("document.text", &AuditOutcome::Ok),
            ("document.find", &AuditOutcome::Ok),
            ("document.find", &AuditOutcome::Failed("invalid_argument".into())),
        ]
    );
    let everything = format!("{records:?}");
    for read in ["Page 1", "Page 2", "notes.pdf"] {
        assert!(!everything.contains(read), "the record does not hold {read:?}");
    }
}
