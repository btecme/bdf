//! `pdfcraft-cli mcp` over its real stdin and stdout: message lines are bounded before they are buffered or parsed
//! (BUILD-005, finding F05). The unit and library tests cover every message kind; this one proves the bound is
//! reached through the actual process, in plain and compact mode, with the 16 MiB default.
#![cfg(feature = "mcp")]

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

/// Largest line the server reads (`pdfcraft_automation::mcp::DEFAULT_MAX_MESSAGE`).
const LIMIT: usize = pdfcraft_automation::mcp::DEFAULT_MAX_MESSAGE;

/// A valid request exactly `len` bytes long, padded by an extra string member of its params.
fn request_of(len: usize, id: u64, method: &str, mut params: Value) -> String {
    params["padding"] = json!("");
    let base = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }).to_string();
    let line = base.replacen("\"padding\":\"\"", &format!("\"padding\":\"{}\"", "x".repeat(len - base.len())), 1);
    assert_eq!(line.len(), len);
    line
}

fn session(flags: &[&str], lines: Vec<String>) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_pdfcraft-cli"))
        .arg("mcp")
        .args(flags)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    // Writing a multi-megabyte line blocks until the server reads it, so write from a thread and read the replies here.
    let writer = std::thread::spawn(move || {
        for line in lines {
            if stdin.write_all(line.as_bytes()).and_then(|()| stdin.write_all(b"\n")).is_err() {
                break;
            }
        }
        // Dropping stdin ends the session.
    });
    let mut replies = Vec::new();
    let mut line = String::new();
    while stdout.read_line(&mut line).unwrap() > 0 {
        replies.push(serde_json::from_str(&line).unwrap());
        line.clear();
    }
    writer.join().unwrap();
    let mut rest = String::new();
    let _ = stdout.read_to_string(&mut rest);
    assert!(child.wait().unwrap().success());
    replies
}

/// Exactly at the limit is served, over it is refused with a clear error without stopping the server, and the next
/// request is answered, whichever mode the server runs in.
#[test]
fn the_real_server_bounds_a_message_line_and_keeps_serving() {
    for flags in [&[][..], &["--compact"][..]] {
        let replies = session(
            flags,
            vec![
                request_of(LIMIT, 1, "tools/list", json!({})),
                request_of(LIMIT + 1, 2, "tools/list", json!({})),
                request_of(LIMIT * 2, 3, "tools/call", json!({ "name": "tool_search", "arguments": { "query": "rotate" } })),
                request_of(200, 4, "ping", json!({})),
            ],
        );
        assert_eq!(replies.len(), 4, "{flags:?}: one reply per line");
        assert!(replies[0]["result"]["tools"].is_array(), "{flags:?}: a request exactly at the limit is served");
        for over in &replies[1..3] {
            assert_eq!(over["error"]["code"], -32600, "{flags:?}: {over}");
            assert!(over["error"]["message"].as_str().unwrap().contains(&format!("longer than {LIMIT} bytes")), "{flags:?}");
        }
        assert_eq!(replies[3]["id"], 4, "{flags:?}: the request after the long lines is served");
        assert_eq!(replies[3]["result"], json!({}));
    }
}
