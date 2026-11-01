//! The MCP server over the application service (BUILD-009): a client without a grant gets help and nothing else, and a
//! client with one gets exactly what the service's policy gives it, through the same revisions, confinement and refusals.

use std::path::{Path, PathBuf};

use pdfcraft_app_service::audit::AuditError;
use pdfcraft_app_service::{
    ActorContext, AuditEntry, AuditRecord, AuditSink, Capability, ClientId, DESCRIPTORS, DocumentScope, FileAccess, FileAudit, GrantRequest,
    GrantStore, OsPrincipal, Policy, Scope, Service,
};
use pdfcraft_automation::client_mcp::ClientMcpServer;
use serde_json::{Value, json};

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

/// A new empty folder only the current user can use.
fn private_dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pdfcraft-client-mcp-{}-{test}", std::process::id()));
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

fn owner() -> OsPrincipal {
    OsPrincipal::current().expect("the current user")
}

struct Fixture {
    dir: PathBuf,
    /// The folder the client may use.
    work: PathBuf,
    /// A folder it may not.
    elsewhere: PathBuf,
    grants: GrantStore,
}

impl Fixture {
    fn new(test: &str) -> Self {
        let dir = private_dir(test);
        let work = dir.join("work");
        let elsewhere = dir.join("elsewhere");
        std::fs::create_dir_all(&work).expect("work folder");
        std::fs::create_dir_all(&elsewhere).expect("other folder");
        let grants = GrantStore::open(&dir.join("grants.json"), owner()).expect("opens the grant store");
        Self { dir, work, elsewhere, grants }
    }

    fn issue(&self, name: &str, capabilities: &[Capability]) -> ActorContext {
        let files = FileAccess { read_under: vec![self.work.clone()], write_under: vec![self.work.clone()] };
        let request = GrantRequest {
            client: ClientId::parse(name).expect("a client name"),
            scopes: capabilities.iter().map(|c| Scope { capability: *c, documents: DocumentScope::All }).collect(),
            files,
            ttl_seconds: 3600,
        };
        let issued = self.grants.issue(request).expect("issues");
        self.grants.authenticate(name, issued.secret()).expect("authenticates")
    }

    fn service(&self) -> Service {
        Service::new().with_policy(Policy::with_grants(self.grants.clone()))
    }

    fn server(&self, actor: Option<ActorContext>) -> ClientMcpServer {
        ClientMcpServer::new(self.service(), actor)
    }

    fn file(&self, folder: &Path, name: &str, pages: usize) -> PathBuf {
        let path = folder.join(name);
        std::fs::write(&path, fixture(pages)).expect("writes the fixture");
        path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn rpc(server: &mut ClientMcpServer, id: u64, method: &str, params: Value) -> Value {
    server.handle(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })).expect("a reply")
}

/// Call a tool; returns the structured result and whether it was reported as an error.
fn call(server: &mut ClientMcpServer, tool: &str, args: Value) -> (Value, bool) {
    let reply = rpc(server, 1, "tools/call", json!({ "name": tool, "arguments": args }));
    let result = reply.get("result").unwrap_or_else(|| panic!("a result, got {reply}"));
    (result["structuredContent"].clone(), result["isError"].as_bool().expect("isError"))
}

fn ok(server: &mut ClientMcpServer, tool: &str, args: Value) -> Value {
    let (body, is_error) = call(server, tool, args);
    assert!(!is_error, "{tool} should succeed, got {body}");
    body
}

fn error_code(body: &Value) -> String {
    body["error"]["code"].as_str().unwrap_or_default().to_owned()
}

fn tool_names(server: &mut ClientMcpServer) -> Vec<String> {
    let reply = rpc(server, 1, "tools/list", Value::Null);
    reply["result"]["tools"].as_array().expect("tools").iter().map(|t| t["name"].as_str().expect("a name").to_owned()).collect()
}

#[test]
fn without_a_grant_the_server_lists_only_help_and_serves_nothing() {
    let f = Fixture::new("no-grant");
    let path = f.file(&f.work, "a.pdf", 2);
    let mut server = f.server(None);
    assert!(!server.is_authorized());
    assert_eq!(tool_names(&mut server), ["bdf_help"]);

    let init = rpc(&mut server, 1, "initialize", json!({ "protocolVersion": "2025-06-18" }));
    assert_eq!(init["result"]["serverInfo"]["name"], "bdf");
    assert!(init["result"]["instructions"].as_str().expect("instructions").contains("without a valid grant"));

    let help = ok(&mut server, "bdf_help", json!({}));
    assert_eq!(help["authorized"], false);
    assert_eq!(help["operations"].as_array().expect("operations").len(), DESCRIPTORS.len());

    // Every document tool is refused, whether or not the file exists, and nothing about the file is said.
    for (tool, args) in [
        ("doc_open", json!({ "path": path.to_string_lossy() })),
        ("doc_open", json!({ "path": f.elsewhere.join("missing.pdf").to_string_lossy() })),
        ("doc_info", json!({ "doc": 1 })),
        ("doc_close", json!({ "doc": 1, "revision": 0 })),
    ] {
        let (body, is_error) = call(&mut server, tool, args);
        assert!(is_error);
        assert_eq!(error_code(&body), "not_authorized", "{tool}");
    }
    // The tools that were never offered are not tools.
    let reply = rpc(&mut server, 2, "tools/call", json!({ "name": "text_extract", "arguments": {} }));
    assert!(reply.get("error").is_some(), "{reply}");
}

#[test]
fn a_read_only_grant_opens_inside_its_folder_and_is_refused_every_change() {
    let f = Fixture::new("read-only");
    let path = f.file(&f.work, "a.pdf", 3);
    let mut server = f.server(Some(f.issue("reader", &[Capability::DocumentRead])));
    assert_eq!(tool_names(&mut server).len(), 9, "help and the eight operations");

    let opened = ok(&mut server, "doc_open", json!({ "path": path.to_string_lossy() }));
    assert_eq!(opened["pages"].as_array().expect("pages").len(), 3);
    let (doc, revision) = (opened["doc"].as_u64().expect("doc"), opened["revision"].as_u64().expect("revision"));

    let info = ok(&mut server, "doc_info", json!({ "doc": doc }));
    assert_eq!(info["name"], "a.pdf");
    assert_eq!(ok(&mut server, "edit_history", json!({ "doc": doc }))["unsaved"], false);

    for (tool, args) in [
        ("page_rotate", json!({ "doc": doc, "revision": revision, "pages": [1], "degrees": 90 })),
        ("edit_undo", json!({ "doc": doc, "revision": revision })),
        ("doc_save", json!({ "doc": doc, "revision": revision })),
        ("doc_close", json!({ "doc": doc, "revision": revision })),
    ] {
        let (body, is_error) = call(&mut server, tool, args);
        assert!(is_error, "{tool} should be refused");
        assert_eq!(error_code(&body), "not_authorized", "{tool}: {body}");
        assert!(body["error"]["reason"].is_string(), "the refusal says why: {body}");
    }
    // Nothing changed: the file is as it was and the document is where it was.
    assert_eq!(std::fs::read(&path).expect("reads"), fixture(3));
    assert_eq!(ok(&mut server, "doc_info", json!({ "doc": doc }))["revision"], revision);
}

#[test]
fn an_editing_grant_rotates_undoes_saves_and_closes_against_the_revision_it_was_given() {
    let f = Fixture::new("editing");
    let path = f.file(&f.work, "a.pdf", 2);
    let caps = [Capability::DocumentRead, Capability::DocumentEditPages, Capability::DocumentSave, Capability::DocumentClose];
    let mut server = f.server(Some(f.issue("editor", &caps)));

    let opened = ok(&mut server, "doc_open", json!({ "path": path.to_string_lossy() }));
    let doc = opened["doc"].as_u64().expect("doc");
    let r0 = opened["revision"].as_u64().expect("revision");

    let rotated = ok(&mut server, "page_rotate", json!({ "doc": doc, "revision": r0, "pages": [1], "degrees": 90 }));
    assert_eq!(rotated["pages_affected"], 1);
    let r1 = rotated["revision"].as_u64().expect("revision");
    assert_ne!(r1, r0);

    // A request made against the old revision is a conflict that names both, and changes nothing.
    let (stale, is_error) = call(&mut server, "page_rotate", json!({ "doc": doc, "revision": r0, "pages": [2], "degrees": 90 }));
    assert!(is_error);
    assert_eq!(error_code(&stale), "conflict");
    assert_eq!(stale["error"]["conflict"]["kind"], "stale_revision");
    assert_eq!(stale["error"]["conflict"]["current"], r1);
    assert_eq!(ok(&mut server, "doc_info", json!({ "doc": doc }))["pages"][1]["rotation"], 0);

    // Undo and redo go through the same check, and both are possible with this grant.
    let undone = ok(&mut server, "edit_undo", json!({ "doc": doc, "revision": r1 }));
    let r2 = undone["revision"].as_u64().expect("revision");
    let redone = ok(&mut server, "edit_redo", json!({ "doc": doc, "revision": r2 }));
    let r3 = redone["revision"].as_u64().expect("revision");
    assert_eq!(ok(&mut server, "doc_info", json!({ "doc": doc }))["pages"][0]["rotation"], 90);
    assert_eq!(ok(&mut server, "edit_history", json!({ "doc": doc }))["unsaved"], true);

    // Closing with unsaved changes is refused unless asked for.
    let (refused, is_error) = call(&mut server, "doc_close", json!({ "doc": doc, "revision": r3 }));
    assert!(is_error);
    assert_eq!(refused["error"]["conflict"]["kind"], "unsaved_changes");

    let saved = ok(&mut server, "doc_save", json!({ "doc": doc, "revision": r3 }));
    assert_eq!(saved["saved"], "a.pdf");
    assert_ne!(std::fs::read(&path).expect("reads"), fixture(2), "the file on disk changed");
    assert_eq!(ok(&mut server, "edit_history", json!({ "doc": doc }))["unsaved"], false);
    assert_eq!(ok(&mut server, "doc_close", json!({ "doc": doc, "revision": r3 }))["closed"], true);

    // What was saved is what was edited: reopen it.
    let again = ok(&mut server, "doc_open", json!({ "path": path.to_string_lossy() }));
    assert_eq!(again["pages"][0]["rotation"], 90);
    assert_eq!(again["pages"][1]["rotation"], 0);
}

#[test]
fn a_file_outside_the_granted_folder_is_refused_for_reading_and_for_saving() {
    let f = Fixture::new("confined");
    let outside = f.file(&f.elsewhere, "secret.pdf", 1);
    let inside = f.file(&f.work, "a.pdf", 1);
    let caps = [Capability::DocumentRead, Capability::DocumentEditPages, Capability::DocumentSave];
    let mut server = f.server(Some(f.issue("editor", &caps)));

    let (body, is_error) = call(&mut server, "doc_open", json!({ "path": outside.to_string_lossy() }));
    assert!(is_error);
    assert_eq!(error_code(&body), "not_authorized");

    let opened = ok(&mut server, "doc_open", json!({ "path": inside.to_string_lossy() }));
    let (doc, revision) = (opened["doc"].as_u64().expect("doc"), opened["revision"].as_u64().expect("revision"));
    let target = f.elsewhere.join("copy.pdf");
    let (body, is_error) =
        call(&mut server, "doc_save", json!({ "doc": doc, "revision": revision, "path": target.to_string_lossy(), "create_only": true }));
    assert!(is_error);
    assert_eq!(error_code(&body), "not_authorized");
    assert!(!target.exists(), "nothing was written outside the granted folder");
}

#[test]
fn a_grant_revoked_while_the_server_runs_stops_the_next_call() {
    let f = Fixture::new("revoked");
    let path = f.file(&f.work, "a.pdf", 1);
    let actor = f.issue("short-lived", &[Capability::DocumentRead]);
    let mut server = f.server(Some(actor.clone()));
    let opened = ok(&mut server, "doc_open", json!({ "path": path.to_string_lossy() }));
    let doc = opened["doc"].as_u64().expect("doc");

    let id = f.grants.list().into_iter().find(|g| g.0.client.as_str() == "short-lived").expect("the grant").0.id;
    f.grants.revoke(&id).expect("revokes");

    let (body, is_error) = call(&mut server, "doc_info", json!({ "doc": doc }));
    assert!(is_error);
    assert_eq!(error_code(&body), "not_authorized");
    assert_eq!(body["error"]["reason"], "grant_revoked");
}

#[test]
fn arguments_are_read_strictly() {
    let f = Fixture::new("strict");
    let path = f.file(&f.work, "a.pdf", 1);
    let mut server = f.server(Some(f.issue("editor", &[Capability::DocumentRead, Capability::DocumentEditPages])));
    let opened = ok(&mut server, "doc_open", json!({ "path": path.to_string_lossy() }));
    let (doc, revision) = (opened["doc"].as_u64().expect("doc"), opened["revision"].as_u64().expect("revision"));

    // An argument the tool does not have is a protocol error, as in the other server.
    let reply = rpc(&mut server, 9, "tools/call", json!({ "name": "doc_info", "arguments": { "doc": doc, "path": "x" } }));
    assert_eq!(reply["error"]["code"], -32602, "{reply}");

    // A value of the wrong type is reported, not coerced.
    for args in [
        json!({ "doc": doc, "revision": revision, "degrees": "90" }),
        json!({ "doc": doc, "revision": revision, "degrees": 90.5 }),
        json!({ "doc": doc, "revision": -1, "degrees": 90 }),
        json!({ "doc": "1", "revision": revision, "degrees": 90 }),
        json!({ "doc": doc, "revision": revision, "degrees": 90, "pages": "1" }),
        json!({ "doc": doc, "revision": revision }),
    ] {
        let (body, is_error) = call(&mut server, "page_rotate", args.clone());
        assert!(is_error, "{args}");
        assert_eq!(error_code(&body), "invalid_argument", "{args}: {body}");
    }
    // A request the service itself finds wrong is its refusal, with its wording.
    let (body, is_error) = call(&mut server, "page_rotate", json!({ "doc": doc, "revision": revision, "pages": [0], "degrees": 90 }));
    assert!(is_error);
    assert_eq!(error_code(&body), "invalid_argument");
    // A document number from nowhere is not found, not a crash.
    let (body, _) = call(&mut server, "doc_info", json!({ "doc": 4_000_000_000_u64 }));
    assert_eq!(error_code(&body), "not_found");
    // Nothing above changed the document.
    assert_eq!(ok(&mut server, "doc_info", json!({ "doc": doc }))["revision"], revision);
}

#[test]
fn a_message_over_the_bound_is_refused_and_the_next_one_is_served() {
    let f = Fixture::new("framing");
    let server = f.server(None);
    let mut server = server.with_max_message(64);
    let long = format!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\",\"params\":{{\"pad\":\"{}\"}}}}\n", "x".repeat(200));
    let input = format!("{long}{{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}}\n");
    let mut out = Vec::new();
    server.serve(std::io::Cursor::new(input), &mut out).expect("serves");
    let lines: Vec<Value> = String::from_utf8(out).expect("utf-8").lines().map(|l| serde_json::from_str(l).expect("json")).collect();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert_eq!(lines[0]["error"]["code"], -32600);
    assert_eq!(lines[1]["id"], 2);
    assert!(lines[1].get("result").is_some());
}

#[test]
fn every_service_operation_has_exactly_one_tool() {
    let f = Fixture::new("coverage");
    let mut server = f.server(Some(f.issue("reader", &[Capability::DocumentRead])));
    // help plus one tool per descriptor: a new operation without a tool (or the reverse) fails here.
    assert_eq!(tool_names(&mut server).len(), DESCRIPTORS.len() + 1);
}

// ---- the record of a call the adapter refuses -----------------------------------------------------------------------

/// A server whose service records to the file log in `log`.
fn audited(f: &Fixture, actor: ActorContext, log: &Path) -> ClientMcpServer {
    let audit = FileAudit::open(log).expect("opens the log");
    ClientMcpServer::new(Service::new().with_policy(Policy::with_grants(f.grants.clone())).with_audit(Box::new(audit)), Some(actor))
}

/// The records in `log`, oldest first.
fn recorded(log: &Path) -> Vec<Value> {
    pdfcraft_app_service::audit::recent_lines(log, 1000)
        .expect("reads the log")
        .iter()
        .map(|line| serde_json::from_str(line).expect("a JSON record"))
        .collect()
}

#[test]
fn a_call_refused_for_its_arguments_is_recorded_with_who_asked_and_nothing_they_sent() {
    let f = Fixture::new("refused-recorded");
    let log = f.dir.join("log");
    let path = f.file(&f.work, "a.pdf", 2);
    let mut server = audited(&f, f.issue("agent", &[Capability::DocumentRead, Capability::DocumentEditPages]), &log);
    let opened = ok(&mut server, "doc_open", json!({ "path": path.to_string_lossy() }));
    let doc = opened["doc"].as_u64().expect("doc");
    assert_eq!(recorded(&log).len(), 1, "the open");

    let hidden = "hunter2-not-for-the-log";
    let hidden_path = f.elsewhere.join("never-for-the-log.pdf").to_string_lossy().into_owned();
    // Each is refused before it is an operation: a negative revision, a name the tool does not have, a value of the wrong
    // type, a missing argument, and arguments that are not an object.
    let refused: [(&str, Value, &str); 5] = [
        ("page_rotate", json!({ "doc": doc, "revision": -1, "degrees": 90 }), "page.rotate"),
        ("page_rotate", json!({ "doc": doc, "revision": 0, "degrees": 90, "bogus": hidden }), "page.rotate"),
        ("doc_open", json!({ "path": hidden_path, "password": 5 }), "document.open"),
        ("doc_save", json!({ "doc": doc, "path": hidden_path }), "document.save"),
        ("edit_undo", json!(hidden), "edit.undo"),
    ];
    for (n, (tool, args, operation)) in refused.iter().enumerate() {
        let reply = rpc(&mut server, 10, "tools/call", json!({ "name": tool, "arguments": args }));
        let shown = reply.to_string();
        assert!(reply.get("error").is_some() || reply["result"]["isError"] == true, "{tool} {args}: {shown}");
        let records = recorded(&log);
        assert_eq!(records.len(), 2 + n, "{tool} {args}: one record for the refusal");
        let last = records.last().expect("a record");
        assert_eq!(last["op"], *operation, "{tool}");
        assert_eq!(last["outcome"], "failed", "{tool}");
        assert_eq!(last["reason"], "invalid_argument", "{tool}");
        assert_eq!(last["client"], "agent");
        assert!(last["grant"].as_str().is_some_and(|g| g.starts_with("g-")), "{last}");
    }
    let everything = std::fs::read_to_string(log.join("audit-000001.jsonl")).expect("reads the log");
    for forbidden in [hidden, "never-for-the-log", "hunter2", "bogus"] {
        assert!(!everything.contains(forbidden), "{forbidden} reached the log: {everything}");
    }
    // The refusals do not stop the client, and the log is whole.
    assert_eq!(ok(&mut server, "doc_info", json!({ "doc": doc }))["name"], "a.pdf");
    let report = pdfcraft_app_service::audit::verify(&log).expect("verifies");
    assert_eq!((report.records, report.first_bad), (7, None));
}

#[test]
fn a_refusal_that_cannot_be_recorded_stops_the_next_call_like_any_other_failed_record() {
    struct Broken;
    impl AuditSink for Broken {
        fn record(&mut self, _: AuditEntry) -> Result<AuditRecord, AuditError> {
            Err(AuditError("the disk is full".into()))
        }
    }
    let f = Fixture::new("refused-unrecorded");
    let path = f.file(&f.work, "a.pdf", 1);
    let actor = f.issue("agent", &[Capability::DocumentRead]);
    let mut server =
        ClientMcpServer::new(Service::new().with_policy(Policy::with_grants(f.grants.clone())).with_audit(Box::new(Broken)), Some(actor));
    // The only thing this client has done is send a malformed request, and that could not be recorded.
    let (_, is_error) = call(&mut server, "doc_open", json!({ "path": 7 }));
    assert!(is_error);
    let (body, is_error) = call(&mut server, "doc_open", json!({ "path": path.to_string_lossy() }));
    assert!(is_error, "{body}");
    assert_eq!(body["error"]["reason"], "audit_unavailable", "{body}");
}
