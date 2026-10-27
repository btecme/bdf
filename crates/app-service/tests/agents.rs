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
        scopes: vec![Scope { capability: Capability::DocumentRead, documents: DocumentScope::Only(vec![first.document]) }],
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
