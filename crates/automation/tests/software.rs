//! The software tools (BUILD-018): update status, check, download, apply, rollback and Windows integration, and the
//! separate authorisation that gates them.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use pdfcraft_automation::mcp::McpServer;
use pdfcraft_automation::software::{NOT_AUTHORISED, SoftwareEnv};
use pdfcraft_automation::{Automation, Content};
use pdfcraft_update::manifest::make_envelope;
use pdfcraft_update::service::Updater;
use pdfcraft_update::testing::{build, installed_copy, manifest_json, sign};
use pdfcraft_update::{Channel, Version};
use serde_json::{Value, json};

const V0: &[(&str, &[u8])] = &[("bdf.exe", b"exe-0"), ("bdf-cli.exe", b"cli-0"), ("portable.txt", b"p")];
const V1: &[(&str, &[u8])] = &[("bdf.exe", b"exe-1"), ("bdf-cli.exe", b"cli-1"), ("portable.txt", b"p")];

const OPERATIONS: [&str; 7] = [
    "update_check",
    "update_download",
    "update_apply",
    "update_rollback",
    "windows_integration_register",
    "windows_integration_remove",
    "windows_open_default_apps",
];
const STATUS: [&str; 2] = ["update_status", "windows_integration_status"];

struct Server {
    base: String,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// A loopback server answering `routes(base)`.
fn serve(routes: impl FnOnce(&str) -> Vec<(String, Vec<u8>)>) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let routes = routes(&base);
    listener.set_nonblocking(true).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let handle = std::thread::spawn(move || {
        while !flag.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut s, _)) => {
                    s.set_nonblocking(false).unwrap();
                    let mut buf = [0u8; 4096];
                    let n = s.read(&mut buf).unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]).to_string();
                    let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
                    let (status, body) = routes.iter().find(|(p, _)| *p == path).map(|(_, b)| (200, b.clone())).unwrap_or((404, b"nope".to_vec()));
                    let _ = write!(s, "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                    let _ = s.write_all(&body);
                }
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(5)),
            }
        }
    });
    Server { base, stop, handle: Some(handle) }
}

fn release_server(root: &Path) -> Server {
    let pkg = build(root, "srv.zip", "0.8.1", V1);
    let zip = std::fs::read(&pkg.zip).unwrap();
    serve(|base| {
        let json = manifest_json("0.8.1", "dev", &pkg.entry.sha256, pkg.entry.size)
            .replace("https://github.com/btecme/bdf/releases/download/v", &format!("{base}/v"));
        vec![
            ("/release.json".into(), make_envelope(json.as_bytes(), &sign(1, json.as_bytes()))),
            ("/v0.8.1/bdf-0.8.1-windows-x64-portable.zip".into(), zip),
        ]
    })
}

fn env(root: &Path, feed: Option<String>) -> (SoftwareEnv, std::path::PathBuf) {
    let app = installed_copy(root, "0.8.0", V0);
    let trust = pdfcraft_update::testing::trust_for(Channel::Dev);
    let updater = Updater { trust, app_dir: app.clone(), state_dir: root.join("state"), current: Version(0, 8, 0), feed };
    (SoftwareEnv { updater, gui_exe: app.join("bdf.exe") }, app)
}

fn json_of(content: Vec<Content>) -> Value {
    match content.into_iter().next() {
        Some(Content::Json(v)) => v,
        other => panic!("expected JSON, got {other:?}"),
    }
}

#[test]
fn status_tools_work_without_any_authorisation_and_change_nothing() {
    let root = tempfile::tempdir().unwrap();
    let (software, app) = env(root.path(), None);
    let before = std::fs::read_dir(&app).unwrap().count();
    let mut a = Automation::new().with_software_env(software);
    assert!(!a.software_changes_allowed());
    let status = json_of(a.call("update_status", &json!({})).unwrap());
    assert_eq!(status["current"], "0.8.0");
    assert_eq!(status["managed"], true);
    assert_eq!(status["dev_build"], true);
    assert!(status["update_available"].is_null());
    let win = json_of(a.call("windows_integration_status", &json!({})).unwrap());
    assert_eq!(win["registered"].as_bool().is_some(), true);
    assert!(win["summary"].as_str().is_some_and(|s| !s.is_empty()));
    assert_eq!(std::fs::read_dir(&app).unwrap().count(), before, "status created nothing in the copy");
}

#[test]
fn every_operation_is_refused_without_the_software_authorisation_and_nothing_changes() {
    let root = tempfile::tempdir().unwrap();
    let srv_root = tempfile::tempdir().unwrap();
    let server = release_server(srv_root.path());
    let (software, app) = env(root.path(), Some(format!("{}/release.json", server.base)));
    let tree = |p: &Path| {
        let mut v: Vec<_> = walk(p);
        v.sort();
        v
    };
    let before = tree(&app);
    // Even a --root confinement or any amount of document access does not authorise them.
    let mut a = Automation::new().with_root(root.path()).unwrap().with_software_env(software);
    for tool in OPERATIONS {
        let err = a.call(tool, &json!({})).unwrap_err().to_string();
        assert_eq!(err, NOT_AUTHORISED, "{tool}");
    }
    assert_eq!(tree(&app), before);
    assert!(!root.path().join("state").exists(), "no check ran, so no state was written");
}

fn walk(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(format!("{} {}", p.display(), std::fs::metadata(&p).unwrap().len()));
        }
    }
    out
}

#[test]
fn with_the_authorisation_check_and_download_run_and_status_follows() {
    let root = tempfile::tempdir().unwrap();
    let srv_root = tempfile::tempdir().unwrap();
    let server = release_server(srv_root.path());
    let (software, _app) = env(root.path(), Some(format!("{}/release.json", server.base)));
    let mut a = Automation::new().with_software_changes(true).with_software_env(software);
    let checked = json_of(a.call("update_check", &json!({})).unwrap());
    assert_eq!(checked["status"]["update_available"]["version"], "0.8.1");
    assert_eq!(checked["status"]["ready_to_install"], false);
    let downloaded = json_of(a.call("update_download", &json!({})).unwrap());
    assert_eq!(downloaded["ready"], true);
    let status = json_of(a.call("update_status", &json!({})).unwrap());
    assert_eq!(status["ready_to_install"], true);
    // Applying needs the updater program beside BDF; this fixture has none, and the failure is clean.
    let err = a.call("update_apply", &json!({})).unwrap_err().to_string();
    assert!(err.contains("bdf-updater"), "{err}");
}

#[test]
fn registering_a_program_that_does_not_exist_changes_no_registry_entry() {
    let root = tempfile::tempdir().unwrap();
    let (mut software, _app) = env(root.path(), None);
    software.gui_exe = root.path().join("no").join("such").join("bdf.exe");
    let mut a = Automation::new().with_software_changes(true).with_software_env(software);
    let err = a.call("windows_integration_register", &json!({})).unwrap_err().to_string();
    assert!(err.contains("cannot be registered") || err.contains("only available on Windows"), "{err}");
}

#[test]
fn the_mcp_server_lists_the_operations_only_when_it_was_started_with_the_authorisation() {
    let names = |auto: Automation| -> Vec<String> {
        let mut s = McpServer::new(auto);
        let reply: Value =
            serde_json::from_str(&s.handle_line(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }).to_string()).unwrap()).unwrap();
        reply["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect()
    };
    let plain = names(Automation::new());
    for tool in STATUS {
        assert!(plain.iter().any(|n| n == tool), "{tool} is always listed");
    }
    for tool in OPERATIONS {
        assert!(!plain.iter().any(|n| n == tool), "{tool} is not listed without the authorisation");
    }
    let allowed = names(Automation::new().with_software_changes(true));
    for tool in OPERATIONS.iter().chain(STATUS.iter()) {
        assert!(allowed.iter().any(|n| n == tool), "{tool} is listed with the authorisation");
    }
    // A call anyway (a client that guessed the name) is refused with the same message, as a tool error.
    let mut s = McpServer::new(Automation::new());
    let reply: Value = serde_json::from_str(
        &s.handle_line(
            &json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": "update_apply", "arguments": {} } }).to_string(),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(reply["result"]["isError"], true, "{reply}");
    assert!(reply.to_string().contains("not authorised"), "{reply}");
}

#[test]
fn bad_arguments_are_rejected_before_anything_runs() {
    let root = tempfile::tempdir().unwrap();
    let (software, _) = env(root.path(), None);
    let mut a = Automation::new().with_software_changes(true).with_software_env(software);
    assert!(a.call("update_check", &json!({ "manual": "yes" })).is_err());
    assert!(a.call("update_status", &json!({ "extra": 1 })).is_err());
    assert!(a.call("windows_integration_register", &json!({ "open_settings": 5 })).is_err());
}
