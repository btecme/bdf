//! Shared helpers for the worker tests.
#![allow(dead_code)]

use std::io::BufReader;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pdfcraft_render::{PageRenderer, RenderConfig, RenderRequest, RequestKind};
use pdfcraft_worker::protocol::{Frame, Kind, Limits, read_frame, write_frame};
use pdfcraft_worker::supervisor::Config;

pub const WORKER: &str = env!("CARGO_BIN_EXE_pdfcraft-worker");

pub fn cfg() -> Config {
    Config::new(WORKER)
}

/// A valid one-page PDF with a line of text, built by hand so the tests need no fixture files.
pub fn tiny_pdf(text: &str) -> Vec<u8> {
    let content = format!("BT /F1 24 Tf 72 200 Td ({text}) Tj ET");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for off in offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    out
}

/// What the renderer produces for a page when called directly in this process.
pub fn in_process(doc: &[u8], page: usize, dpi: f32) -> (u32, u32, Vec<u8>) {
    let mut r = PageRenderer::new(Arc::new(doc.to_vec()), RenderConfig { reject_oversize: true, ..RenderConfig::default() });
    let p = r.render(RenderRequest { page, kind: RequestKind::Pixels, tile: None, scale: dpi / 72.0, tag: 0, preview: false });
    assert!(p.error.is_none(), "{:?}", p.error);
    let bytes: &[u8] = &p.rgba;
    (p.width, p.height, bytes.to_vec())
}

/// A worker spoken to directly, without the supervisor, so a test can send what the supervisor never would.
pub struct Raw {
    pub child: Child,
    pub stdin: Option<ChildStdin>,
    pub stdout: BufReader<ChildStdout>,
    pub limits: Limits,
}

impl Raw {
    pub fn start() -> Self {
        let mut child = Command::new(WORKER).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().expect("spawn the worker");
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().expect("stdout"));
        Self { child, stdin, stdout, limits: Limits::default() }
    }

    pub fn send(&mut self, kind: Kind, id: u64, payload: &[u8]) {
        let l = self.limits;
        write_frame(self.stdin.as_mut().expect("stdin open"), kind, id, payload, &l).expect("write a frame");
    }

    pub fn send_bytes(&mut self, bytes: &[u8]) {
        use std::io::Write;
        let w = self.stdin.as_mut().expect("stdin open");
        w.write_all(bytes).expect("write");
        w.flush().expect("flush");
    }

    pub fn recv(&mut self) -> Option<Frame> {
        read_frame(&mut self.stdout, &self.limits).ok().flatten()
    }

    /// Wait for the process to exit; `None` if it is still running after `limit`.
    pub fn wait_exit(&mut self, limit: Duration) -> Option<i32> {
        let start = Instant::now();
        while start.elapsed() < limit {
            if let Ok(Some(s)) = self.child.try_wait() {
                return s.code();
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        None
    }
}

impl Drop for Raw {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// True when no process with this id is running (Windows: `tasklist`).
#[cfg(windows)]
pub fn process_gone(pid: u32) -> bool {
    let out = Command::new("tasklist").args(["/FI", &format!("PID eq {pid}"), "/NH"]).output().expect("tasklist");
    !String::from_utf8_lossy(&out.stdout).contains(&pid.to_string())
}

#[cfg(not(windows))]
pub fn process_gone(pid: u32) -> bool {
    !std::path::Path::new(&format!("/proc/{pid}")).exists()
}

pub fn wait_until(limit: Duration, mut f: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < limit {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    f()
}

/// Start `examples/parent_probe` (a parent that starts a worker, optionally wedges it, prints the
/// worker's pid and sleeps). Returns the probe and the worker's pid.
#[cfg(windows)]
pub fn start_probe(hang: bool) -> (Child, u32) {
    use std::io::BufRead;
    let exe = std::env::current_exe().unwrap();
    let probe = exe.parent().and_then(|p| p.parent()).unwrap().join("examples").join("parent_probe.exe");
    assert!(probe.is_file(), "build the example first: {}", probe.display());
    let mut cmd = Command::new(&probe);
    cmd.arg(WORKER);
    if hang {
        cmd.arg("hang");
    }
    let mut parent = cmd.stdout(Stdio::piped()).spawn().expect("start the probe");
    let mut line = String::new();
    BufReader::new(parent.stdout.take().unwrap()).read_line(&mut line).unwrap();
    let pid: u32 = line.trim().strip_prefix("PID ").expect("a PID line").parse().unwrap();
    (parent, pid)
}
