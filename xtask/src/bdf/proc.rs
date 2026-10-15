//! Run a child process under a hard timeout, capturing bounded output.
//!
//! Both pipes are drained on their own threads, so a child that writes a lot to one of them can
//! never block on a full pipe while we wait for it to exit.

use std::io::Read;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

/// How a child ended.
pub struct Output {
    /// `None` when it was killed for running past the timeout (or ended without a code).
    pub code: Option<i32>,
    pub timed_out: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub ms: u64,
}

/// Keep the first `cap` bytes of a stream and discard the rest while still draining it.
fn drain(mut r: impl Read + Send + 'static, cap: usize) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut kept = Vec::new();
        let mut buf = [0u8; 16 * 1024];
        loop {
            match r.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let room = cap.saturating_sub(kept.len());
                    kept.extend_from_slice(buf.get(..n.min(room)).unwrap_or_default());
                }
            }
        }
        kept
    })
}

/// Spawn `cmd`, wait up to `timeout`, kill it if it is still running, and return what it printed.
pub fn run(cmd: &mut Command, timeout: Duration, cap: usize) -> Result<Output> {
    let start = Instant::now();
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("starting {:?}", cmd.get_program()))?;
    let out = drain(child.stdout.take().context("stdout pipe")?, cap);
    let err = drain(child.stderr.take().context("stderr pipe")?, cap);
    let mut timed_out = false;
    let code = loop {
        if let Some(status) = child.try_wait()? {
            break status.code();
        }
        if start.elapsed() >= timeout {
            timed_out = true;
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        thread::sleep(Duration::from_millis(2));
    };
    let stdout = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();
    Ok(Output { code, timed_out, stdout, stderr, ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_output_and_exit_code() {
        let mut c = Command::new(std::env::current_exe().unwrap());
        c.arg("--help");
        let o = run(&mut c, Duration::from_secs(30), 1 << 20).unwrap();
        assert!(!o.timed_out);
        assert!(o.code.is_some());
    }

    #[test]
    fn kills_a_child_that_runs_past_the_timeout() {
        // A command that would run for a minute: ping on Windows, sleep elsewhere.
        let mut c = if cfg!(windows) {
            let mut c = Command::new("ping");
            c.args(["-n", "60", "127.0.0.1"]);
            c
        } else {
            let mut c = Command::new("sleep");
            c.arg("60");
            c
        };
        let o = run(&mut c, Duration::from_millis(400), 1 << 16).unwrap();
        assert!(o.timed_out);
        assert_eq!(o.code, None);
        assert!(o.ms < 10_000, "took {} ms", o.ms);
    }

    #[test]
    fn output_beyond_the_cap_is_dropped_but_drained() {
        let mut c = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.args(["/C", "dir", "/s", "C:\\Windows\\System32\\drivers"]);
            c
        } else {
            let mut c = Command::new("sh");
            c.args(["-c", "yes | head -c 5000000"]);
            c
        };
        let o = run(&mut c, Duration::from_secs(60), 1000).unwrap();
        assert!(!o.timed_out, "the child must finish: a full pipe would have blocked it");
        assert!(o.stdout.len() <= 1000);
    }
}
