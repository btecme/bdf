//! `grant` and `activity` through the real binary (BUILD-008): a client is read-only until it is told otherwise, its secret
//! is shown once, and revoking from the command reaches a service that is already running in another process.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use std::sync::{Mutex, mpsc};
use std::time::Duration;

use pdfcraft_app_service::{
    ActorContext, AuditEffect, AuditEntry, AuditOutcome, AuditSink, ClientId, FileAudit, GrantRequest, GrantStore, OsPrincipal, PrincipalKind,
};

const BIN: &str = env!("CARGO_BIN_EXE_pdfcraft-cli");

/// A new empty folder only the current user can use (a system temporary folder is not that on every machine).
fn private_dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pdfcraft-cli-access-{}-{test}", std::process::id()));
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

fn run(state: &Path, args: &[&str]) -> Output {
    Command::new(BIN).args(args).arg("--state-dir").arg(state).output().expect("runs the binary")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn json(output: &Output) -> serde_json::Value {
    assert!(output.status.success(), "exited {:?}: {}", output.status, text(&output.stderr));
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| panic!("not JSON ({e}): {}", text(&output.stdout)))
}

fn owner() -> OsPrincipal {
    OsPrincipal::current().expect("the current user")
}

#[test]
fn a_new_grant_is_read_only_and_its_secret_is_shown_once_and_never_stored() {
    let state = private_dir("issue");
    let issued = run(&state, &["grant", "issue", "--client", "claude-desktop"]);
    let v = json(&issued);
    assert_eq!(v["client"], "claude-desktop");
    assert_eq!(v["allows"], serde_json::json!(["document.read"]), "read-only unless told otherwise");
    assert_eq!(v["read_under"], serde_json::json!([]));
    assert_eq!(v["write_under"], serde_json::json!([]));
    let secret = v["secret"].as_str().expect("a secret").to_owned();
    assert!(secret.starts_with("bdfg_"));
    assert!(text(&issued.stderr).contains("shown once"), "the person is told: {}", text(&issued.stderr));

    // It is on disk only as a hash, and a later listing does not show it.
    let file = std::fs::read_to_string(state.join("grants.json")).expect("the grant file");
    assert!(!file.contains(&secret) && !file.contains(&secret[5..]));
    let listed = run(&state, &["grant", "list"]);
    assert!(!text(&listed.stdout).contains(&secret[5..]), "the secret is not listed");
    let l = json(&listed);
    assert_eq!(l["grants"].as_array().map(Vec::len), Some(1));
    assert_eq!((l["grants"][0]["revoked"].clone(), l["grants"][0]["expired"].clone()), (false.into(), false.into()));
    assert_eq!(l["grants"][0]["grant"], v["grant"]);

    // The secret does what it says: it makes an agent for this account and no other client's name works with it.
    let store = GrantStore::open(&state.join("grants.json"), owner()).expect("opens");
    assert!(store.authenticate("claude-desktop", &secret).is_ok());
    assert!(store.authenticate("someone-else", &secret).is_err());
    let _ = std::fs::remove_dir_all(&state);
}

#[test]
fn grants_are_listed_revoked_and_pruned() {
    let state = private_dir("lifecycle");
    let keep = json(&run(&state, &["grant", "issue", "--client", "keeper"]));
    let drop = json(&run(&state, &["grant", "issue", "--client", "dropper", "--ttl", "2h"]));
    let id = drop["grant"].as_str().expect("an id");
    assert_eq!(json(&run(&state, &["grant", "revoke", id]))["revoked"], id);
    let l = json(&run(&state, &["grant", "list"]));
    let by_client = |name: &str| l["grants"].as_array().expect("a list").iter().find(|g| g["client"] == name).cloned().expect(name);
    assert_eq!(by_client("dropper")["revoked"], true);
    assert_eq!(by_client("keeper")["revoked"], false);
    assert_eq!(json(&run(&state, &["grant", "revoke", id]))["revoked"], id, "revoking twice is fine");
    assert_eq!(json(&run(&state, &["grant", "prune"]))["removed"], 1);
    let after = json(&run(&state, &["grant", "list"]));
    assert_eq!(after["grants"].as_array().map(Vec::len), Some(1));
    assert_eq!(after["grants"][0]["grant"], keep["grant"]);
    // An unknown grant is an error, not a silent success.
    let missing = run(&state, &["grant", "revoke", "g-0000000000000000"]);
    assert!(!missing.status.success() && text(&missing.stderr).contains("no such grant"), "{}", text(&missing.stderr));
    let _ = std::fs::remove_dir_all(&state);
}

#[test]
fn capabilities_folders_and_times_are_asked_for_by_name_and_validated() {
    let state = private_dir("validation");
    let base = std::env::temp_dir();
    let read = base.to_string_lossy().into_owned();
    let write = base.join("out").to_string_lossy().into_owned();
    let granted = json(&run(
        &state,
        &[
            "grant",
            "issue",
            "--client",
            "editor",
            "--allow",
            "document.edit.pages,document.save",
            "--allow",
            "document.read",
            "--read",
            &read,
            "--write",
            &write,
            "--ttl",
            "90m",
        ],
    ));
    assert_eq!(granted["allows"], serde_json::json!(["document.read", "document.edit.pages", "document.save"]), "named, de-duplicated, in order");
    assert_eq!(granted["read_under"], serde_json::json!([read]));
    assert_eq!(granted["write_under"], serde_json::json!([write]));

    let refused = |args: &[&str], why: &str| {
        let out = run(&state, args);
        assert!(!out.status.success(), "{args:?} should be refused");
        assert!(out.stdout.is_empty(), "{args:?}: nothing on stdout, least of all a secret: {}", text(&out.stdout));
        assert!(text(&out.stderr).contains(why), "{args:?}: {}", text(&out.stderr));
    };
    refused(&["grant", "issue"], "--client NAME is required");
    refused(&["grant", "issue", "--client", "Bad Name"], "lowercase letters");
    refused(&["grant", "issue", "--client", "c", "--allow", "document.export"], "not a capability a grant can hold");
    refused(&["grant", "issue", "--client", "c", "--allow", "document.edit.reverted"], "not a capability a grant can hold");
    refused(&["grant", "issue", "--client", "c", "--read", "relative/folder"], "absolute");
    refused(&["grant", "issue", "--client", "c", "--write", &format!("{read}/../elsewhere")], "..");
    refused(&["grant", "issue", "--client", "c", "--ttl", "forever"], "not a time");
    refused(&["grant", "issue", "--client", "c", "--ttl", "10s"], "between a minute and thirty days");
    refused(&["grant", "issue", "--client", "c", "--ttl", "99d"], "between a minute and thirty days");
    refused(&["grant", "issue", "--client", "c", "--bogus", "x"], "unknown option --bogus");
    refused(&["grant", "revoke", "not-an-id"], "not a grant id");
    refused(&["grant", "frobnicate"], "use `issue`");
    refused(&["activity", "frobnicate"], "use `verify` or `show`");
    refused(&["activity", "show", "--last", "0"], "from 1 to 1000");
    refused(&["activity", "show", "--last", "5000"], "from 1 to 1000");
    // Nothing the refused commands did reached the store.
    let l = json(&run(&state, &["grant", "list"]));
    assert_eq!(l["grants"].as_array().map(Vec::len), Some(1), "only the good grant exists");
    let _ = std::fs::remove_dir_all(&state);
}

#[test]
fn revoking_from_the_command_reaches_a_service_already_running_in_another_process() {
    let state = private_dir("cross-process");
    let issued = json(&run(&state, &["grant", "issue", "--client", "running"]));
    let secret = issued["secret"].as_str().expect("a secret");
    // This process stands for the running service: it opened the grants before the command ran and holds on to them.
    let service = GrantStore::open(&state.join("grants.json"), owner()).expect("opens");
    let actor = service.authenticate("running", secret).expect("authenticates");
    assert!(service.check(&actor).is_ok());

    let id = issued["grant"].as_str().expect("an id");
    json(&run(&state, &["grant", "revoke", id]));
    assert!(service.check(&actor).is_err(), "the running service noticed the revocation without being restarted");
    assert!(service.authenticate("running", secret).is_err());
    let _ = std::fs::remove_dir_all(&state);
}

/// Two processes change the grants at the same moment. One issues a grant and is held, partway through its change, until
/// the test lets it go; the other, a real `pdfcraft-cli`, revokes a grant meanwhile. Without turns the first writes the
/// file from what it read before the revocation and the revoked grant is live again.
#[test]
fn a_revocation_is_not_lost_to_a_grant_issued_in_another_process_at_the_same_moment() {
    let state = private_dir("overlap");
    let victim = json(&run(&state, &["grant", "issue", "--client", "victim"]));
    let victim_id = victim["grant"].as_str().expect("an id").to_owned();
    let victim_secret = victim["secret"].as_str().expect("a secret").to_owned();
    let path = state.join("grants.json");

    let (inside_tx, inside_rx) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let (inside_tx, release_rx) = (Mutex::new(inside_tx), Mutex::new(release_rx));
    // The clock is read once while a grant is being issued, after the file has been read: that is where it is held.
    let issuing = GrantStore::open(&path, owner()).expect("opens").with_clock(move || {
        if let (Ok(inside), Ok(release)) = (inside_tx.lock(), release_rx.lock()) {
            let _ = inside.send(());
            let _ = release.recv_timeout(Duration::from_secs(60));
        }
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
    });
    let issuer =
        std::thread::spawn(move || issuing.issue(GrantRequest::read_only(ClientId::parse("later").expect("a name"))).map(|g| g.secret().to_owned()));
    inside_rx.recv_timeout(Duration::from_secs(30)).expect("the issuing process is partway through its change");

    let mut revoker = Command::new(BIN)
        .args(["grant", "revoke", &victim_id])
        .arg("--state-dir")
        .arg(&state)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("starts the revoking process");
    std::thread::sleep(Duration::from_millis(1500));
    assert!(revoker.try_wait().expect("polls").is_none(), "the second change waits for the first instead of racing it");

    release_tx.send(()).expect("lets the issuer go");
    let later_secret = issuer.join().expect("the issuing thread finished").expect("the grant was issued");
    assert!(revoker.wait().expect("waits").success(), "the revocation went through once its turn came");

    // Both changes are in the file: the revoked grant is revoked, and the new one exists.
    let listed = json(&run(&state, &["grant", "list"]));
    let by_client = |name: &str| listed["grants"].as_array().expect("a list").iter().find(|g| g["client"] == name).cloned();
    assert_eq!(by_client("victim").expect("still listed")["revoked"], true, "the revocation was not overwritten");
    assert_eq!(by_client("later").expect("the new grant is there")["revoked"], false);
    let fresh = GrantStore::open(&path, owner()).expect("opens");
    assert!(fresh.authenticate("victim", &victim_secret).is_err(), "a revoked secret does not work again");
    assert!(fresh.authenticate("later", &later_secret).is_ok());
    let _ = std::fs::remove_dir_all(&state);
}

#[test]
fn a_grant_file_other_accounts_can_read_is_refused_and_nothing_is_issued() {
    let state = private_dir("exposed");
    // A grant file that is not private (made so the way a folder shared with others would).
    #[cfg(windows)]
    {
        let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        let status = Command::new(root.join("System32").join("icacls.exe"))
            .arg(&state)
            .args(["/grant", "*S-1-1-0:(OI)(CI)R"])
            .output()
            .expect("runs icacls")
            .status;
        assert!(status.success());
    }
    let file = state.join("grants.json");
    std::fs::write(&file, r#"{"version":1,"grants":[]}"#).expect("writes");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).expect("opens the file");
    }
    let out = run(&state, &["grant", "issue", "--client", "c"]);
    assert!(!out.status.success() && out.stdout.is_empty());
    assert!(text(&out.stderr).contains("not private"), "{}", text(&out.stderr));
    assert_eq!(std::fs::read_to_string(&file).expect("reads"), r#"{"version":1,"grants":[]}"#, "the file was left alone");
    let _ = std::fs::remove_dir_all(&state);
}

fn entry(request: u64) -> AuditEntry {
    AuditEntry {
        at: 1_700_000_000,
        actor: PrincipalKind::Agent,
        client: Some("claude".into()),
        grant: Some("g-0123456789abcdef".into()),
        request_id: request,
        operation: "page.rotate".into(),
        document: Some(1),
        revision_before: Some(1),
        revision_after: Some(2),
        outcome: AuditOutcome::Ok,
        effects: vec![AuditEffect { kind: "file_written".into(), file: Some("out.pdf".into()), bytes: Some(10) }],
        warnings: 0,
    }
}

#[test]
fn the_activity_log_is_verified_shown_and_damage_is_reported() {
    let state = private_dir("activity");
    // No log yet is a log of nothing.
    let empty = json(&run(&state, &["activity", "verify"]));
    assert_eq!((empty["records"].clone(), empty["intact"].clone()), (0.into(), true.into()));
    assert!(text(&run(&state, &["activity", "show"]).stdout).is_empty());

    let log = state.join("activity");
    let mut audit = FileAudit::open(&log).expect("opens the log");
    for n in 1..=5 {
        audit.record(entry(n)).expect("records");
    }
    drop(audit);
    let v = json(&run(&state, &["activity", "verify"]));
    assert_eq!((v["records"].clone(), v["intact"].clone(), v["last_seq"].clone()), (5.into(), true.into(), 5.into()));
    let shown = run(&state, &["activity", "show", "--last", "2"]);
    let lines: Vec<String> = text(&shown.stdout).lines().map(str::to_owned).collect();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines.iter().all(|l| l.contains("\"op\":\"page.rotate\"") && l.contains("\"client\":\"claude\"")));
    assert!(lines[1].contains("\"seq\":5"), "newest last: {}", lines[1]);
    assert!(!text(&shown.stdout).contains(&state.to_string_lossy().into_owned()), "no full path in what is shown");

    // An edited record is found, and the command fails so a script can gate on it.
    let segment = log.join("audit-000001.jsonl");
    let original = std::fs::read_to_string(&segment).expect("reads");
    std::fs::write(&segment, original.replacen("\"rev_after\":2", "\"rev_after\":9", 1)).expect("damages");
    let broken = run(&state, &["activity", "verify"]);
    assert!(!broken.status.success());
    let b: serde_json::Value = serde_json::from_slice(&broken.stdout).expect("a report is printed even then");
    assert_eq!((b["intact"].clone(), b["first_bad"]["line"].clone()), (false.into(), 1.into()));
    assert!(text(&broken.stderr).contains("damaged at segment 1, line 1"));
    let _ = std::fs::remove_dir_all(&state);
}

#[test]
fn the_commands_are_listed_in_the_usage_and_the_person_is_not_an_actor_they_can_claim() {
    let help = Command::new(BIN).arg("--help").output().expect("runs");
    let usage = text(&help.stdout);
    for line in ["pdfcraft-cli grant issue --client NAME", "pdfcraft-cli grant revoke <GRANT-ID>", "pdfcraft-cli activity verify|show"] {
        assert!(usage.contains(line), "{line}: {usage}");
    }
    // The commands take no identity: there is no option that names who is asking.
    for option in ["--actor", "--user", "--as", "--principal"] {
        let out = Command::new(BIN).args(["grant", "list", option, "someone"]).output().expect("runs");
        assert!(!out.status.success() && text(&out.stderr).contains("unknown option"), "{option}");
    }
    // And the person's own context is still read from the system, not from anything given here.
    assert!(ActorContext::local_user().is_ok());
}

// ---- `mcp --client` (BUILD-009) ---------------------------------------------------------------------------------

#[cfg(feature = "mcp")]
mod mcp_client {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Child, ChildStdin, ChildStdout, Stdio};

    use serde_json::{Value, json};

    /// A one-page PDF.
    fn pdf() -> Vec<u8> {
        let objs: [&[u8]; 4] = [
            b"<< /Type /Catalog /Pages 2 0 R >>",
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 300] >>",
            b"<< /Type /Page /Parent 2 0 R >>",
            b"<< /Producer (test) >>",
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

    struct Session {
        child: Child,
        stdin: Option<ChildStdin>,
        stdout: BufReader<ChildStdout>,
        next_id: u64,
    }

    impl Session {
        fn start(state: &Path, client: &str, secret: Option<&str>) -> Self {
            let mut command = Command::new(BIN);
            command.args(["mcp", "--client", client, "--state-dir"]).arg(state);
            command.env_remove("BDF_CLIENT_SECRET");
            if let Some(secret) = secret {
                command.env("BDF_CLIENT_SECRET", secret);
            }
            let mut child = command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().expect("starts the server");
            let stdin = child.stdin.take();
            let stdout = BufReader::new(child.stdout.take().expect("stdout"));
            Self { child, stdin, stdout, next_id: 0 }
        }

        fn rpc(&mut self, method: &str, params: Value) -> Value {
            self.next_id += 1;
            let line = json!({ "jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params });
            let stdin = self.stdin.as_mut().expect("stdin is open");
            writeln!(stdin, "{line}").expect("writes the request");
            stdin.flush().expect("flushes");
            let mut reply = String::new();
            self.stdout.read_line(&mut reply).expect("reads the reply");
            serde_json::from_str(&reply).unwrap_or_else(|e| panic!("not JSON ({e}): {reply:?}"))
        }

        /// A tool call's structured result and whether it was an error.
        fn tool(&mut self, name: &str, args: Value) -> (Value, bool) {
            let reply = self.rpc("tools/call", json!({ "name": name, "arguments": args }));
            let result = reply.get("result").unwrap_or_else(|| panic!("a result, got {reply}")).clone();
            (result["structuredContent"].clone(), result["isError"].as_bool().expect("isError"))
        }

        fn tool_names(&mut self) -> Vec<String> {
            let reply = self.rpc("tools/list", Value::Null);
            reply["result"]["tools"].as_array().expect("tools").iter().map(|t| t["name"].as_str().expect("name").to_owned()).collect()
        }

        fn finish(mut self) {
            drop(self.stdin.take());
            let _ = self.child.wait();
        }
    }

    fn grant_with_folder(state: &Path, folder: &Path, allow: &str) -> String {
        let folder = folder.to_string_lossy().into_owned();
        let issued = json(&run(state, &["grant", "issue", "--client", "agent", "--allow", allow, "--read", &folder, "--write", &folder]));
        issued["secret"].as_str().expect("a secret").to_owned()
    }

    fn work_folder(state: &Path) -> (PathBuf, PathBuf) {
        let work = state.join("work");
        std::fs::create_dir_all(&work).expect("work folder");
        let file = work.join("a.pdf");
        std::fs::write(&file, pdf()).expect("writes");
        (work, file)
    }

    #[test]
    fn a_client_with_a_grant_works_on_a_file_in_its_folder_and_the_activity_is_recorded() {
        let state = private_dir("mcp-client");
        let (work, file) = work_folder(&state);
        let secret = grant_with_folder(&state, &work, "document.read,document.edit.pages,document.save,document.close");

        let mut session = Session::start(&state, "agent", Some(&secret));
        assert_eq!(session.tool_names().len(), 9);
        let (opened, error) = session.tool("doc_open", json!({ "path": file.to_string_lossy() }));
        assert!(!error, "{opened}");
        let (doc, revision) = (opened["doc"].as_u64().expect("doc"), opened["revision"].as_u64().expect("revision"));
        let (rotated, error) = session.tool("page_rotate", json!({ "doc": doc, "revision": revision, "pages": [1], "degrees": 90 }));
        assert!(!error, "{rotated}");
        let revision = rotated["revision"].as_u64().expect("revision");
        let (saved, error) = session.tool("doc_save", json!({ "doc": doc, "revision": revision }));
        assert!(!error, "{saved}");
        session.finish();
        assert_ne!(std::fs::read(&file).expect("reads"), pdf(), "the file was saved with the rotation");

        // The person can see what the client did, and the record verifies.
        let shown = text(&run(&state, &["activity", "show"]).stdout);
        assert!(shown.contains("page.rotate") && shown.contains("document.save"), "{shown}");
        assert!(!shown.contains(&secret), "the secret is never recorded");
        assert!(run(&state, &["activity", "verify"]).status.success());
        let _ = std::fs::remove_dir_all(&state);
    }

    #[test]
    fn a_secret_that_is_wrong_or_missing_gets_help_only_and_the_same_answer() {
        let state = private_dir("mcp-no-grant");
        let (work, file) = work_folder(&state);
        let secret = grant_with_folder(&state, &work, "document.read");

        let mut replies = Vec::new();
        for (client, given) in [("agent", Some("not-the-secret")), ("agent", None), ("nobody", Some(secret.as_str()))] {
            let mut session = Session::start(&state, client, given);
            assert_eq!(session.tool_names(), ["bdf_help"], "{client} {given:?}");
            let (body, error) = session.tool("doc_open", json!({ "path": file.to_string_lossy() }));
            assert!(error);
            replies.push(body);
            session.finish();
        }
        assert!(replies.iter().all(|r| r == &replies[0]), "the three refusals are the same: {replies:?}");
        assert_eq!(replies[0]["error"]["code"], "not_authorized");
        let _ = std::fs::remove_dir_all(&state);
    }

    #[test]
    fn a_grant_revoked_from_another_process_stops_a_running_client() {
        let state = private_dir("mcp-revoke");
        let (work, file) = work_folder(&state);
        let secret = grant_with_folder(&state, &work, "document.read");
        let listed = json(&run(&state, &["grant", "list"]));
        let id = listed["grants"][0]["grant"].as_str().unwrap_or_else(|| panic!("a grant id in {listed}")).to_owned();

        let mut session = Session::start(&state, "agent", Some(&secret));
        let (opened, error) = session.tool("doc_open", json!({ "path": file.to_string_lossy() }));
        assert!(!error, "{opened}");
        let doc = opened["doc"].as_u64().expect("doc");
        assert!(run(&state, &["grant", "revoke", &id]).status.success());
        let (body, error) = session.tool("doc_info", json!({ "doc": doc }));
        assert!(error);
        assert_eq!(body["error"]["reason"], "grant_revoked");
        session.finish();
        let _ = std::fs::remove_dir_all(&state);
    }

    #[test]
    fn the_options_of_the_two_servers_are_not_mixed() {
        let state = private_dir("mcp-options");
        let dir = state.to_string_lossy().into_owned();
        let cases: [Vec<&str>; 3] = [
            vec!["mcp", "--client", "a", "--compact", "--state-dir", &dir],
            vec!["mcp", "--client", "a", "--root", &dir, "--state-dir", &dir],
            vec!["mcp", "--state-dir", &dir],
        ];
        for args in cases {
            let result = Command::new(BIN).args(&args).stdin(Stdio::null()).output().expect("runs");
            assert!(!result.status.success(), "{args:?}");
            let message = text(&result.stderr);
            assert!(message.contains("--client") || message.contains("--compact"), "{message}");
        }
        let _ = std::fs::remove_dir_all(&state);
    }
}
