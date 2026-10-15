//! Run a child process under a hard timeout, capturing bounded output, and optionally measure its
//! memory and confine it to a few CPUs.
//!
//! Both pipes are drained on their own threads, so a child that writes a lot to one of them can
//! never block on a full pipe while we wait for it to exit.
//!
//! Memory comes from the operating system's own counters for the process (peak working set and
//! peak commit), read through `win32job`, a maintained safe wrapper: the workspace forbids
//! `unsafe`, so xtask never calls the Win32 API itself. Off Windows nothing is measured.

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::Serialize;

/// How a child ended.
pub struct Output {
    /// `None` when it was killed for running past the timeout (or ended without a code).
    pub code: Option<i32>,
    pub timed_out: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub ms: u64,
}

/// What the operating system recorded about a process's memory, in bytes.
#[derive(Clone, Copy, Default, Debug, Serialize, PartialEq, Eq)]
pub struct Mem {
    /// Highest working set (physical memory in use) the process ever had.
    pub peak_working_set: u64,
    /// Highest commit charge (memory it asked the system to back) the process ever had.
    pub peak_commit: u64,
    /// Working set at the last sample.
    pub working_set: u64,
}

/// Read a live child's counters. `None` when the platform or the call does not provide them.
#[cfg(windows)]
pub fn sample(child: &Child) -> Option<Mem> {
    use std::os::windows::io::AsRawHandle;
    let c = win32job::utils::get_process_memory_info(child.as_raw_handle() as isize).ok()?;
    Some(Mem { peak_working_set: c.peak_working_set_size as u64, peak_commit: c.peak_pagefile_usage as u64, working_set: c.working_set_size as u64 })
}

#[cfg(not(windows))]
pub fn sample(_child: &Child) -> Option<Mem> {
    None
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

/// Put `child` in a job that limits it to the first `cores` logical processors and ends it if the
/// job is closed. The job must stay alive as long as the child runs.
#[cfg(windows)]
fn confine(child: &Child, cores: u32) -> Option<win32job::Job> {
    use std::os::windows::io::AsRawHandle;
    let mask = if cores >= usize::BITS { usize::MAX } else { (1usize << cores) - 1 };
    let mut info = win32job::ExtendedLimitInfo::new();
    info.limit_affinity(mask).limit_kill_on_job_close();
    let job = win32job::Job::create_with_limit_info(&info).ok()?;
    job.assign_process(child.as_raw_handle() as isize).ok()?;
    Some(job)
}

#[cfg(not(windows))]
fn confine(_child: &Child, _cores: u32) -> Option<()> {
    None
}

/// Spawn `cmd`, wait up to `timeout`, kill it if it is still running, and return what it printed,
/// how much memory it used and whether confinement to `cores` CPUs took effect.
pub fn run_measured(cmd: &mut Command, timeout: Duration, cap: usize, cores: Option<u32>) -> Result<(Output, Mem, bool)> {
    let start = Instant::now();
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("starting {:?}", cmd.get_program()))?;
    let job = cores.and_then(|c| confine(&child, c));
    let confined = cores.is_none() || job.is_some();
    let out = drain(child.stdout.take().context("stdout pipe")?, cap);
    let err = drain(child.stderr.take().context("stderr pipe")?, cap);
    let mut mem = Mem::default();
    let mut timed_out = false;
    let code = loop {
        if let Some(m) = sample(&child) {
            mem = Mem {
                peak_working_set: mem.peak_working_set.max(m.peak_working_set),
                peak_commit: mem.peak_commit.max(m.peak_commit),
                // A sample taken while the process is being torn down reads 0; keep the last real one.
                working_set: if m.working_set > 0 { m.working_set } else { mem.working_set },
            };
        }
        if let Some(status) = child.try_wait()? {
            // The peak counters belong to the process object, so one more read still sees them.
            if let Some(m) = sample(&child) {
                mem.peak_working_set = mem.peak_working_set.max(m.peak_working_set);
                mem.peak_commit = mem.peak_commit.max(m.peak_commit);
            }
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
    drop(job);
    let stdout = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();
    Ok((Output { code, timed_out, stdout, stderr, ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX) }, mem, confined))
}

/// [`run_measured`] without the measurement.
pub fn run(cmd: &mut Command, timeout: Duration, cap: usize) -> Result<Output> {
    Ok(run_measured(cmd, timeout, cap, None)?.0)
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

    #[cfg(windows)]
    #[test]
    fn a_process_that_allocates_shows_a_larger_peak_than_one_that_does_not() {
        // `powershell` allocates and touches 200 MB; `cmd /C ver` does not.
        let mut big = Command::new("powershell");
        big.args(["-NoProfile", "-Command", "$a = New-Object byte[] 200000000; for ($i=0; $i -lt $a.Length; $i+=4096) { $a[$i]=1 }; 'done'"]);
        let (o1, m1, _) = run_measured(&mut big, Duration::from_secs(60), 1 << 16, None).unwrap();
        let mut small = Command::new("cmd");
        small.args(["/C", "ver"]);
        let (_, m2, _) = run_measured(&mut small, Duration::from_secs(60), 1 << 16, None).unwrap();
        assert_eq!(o1.code, Some(0));
        assert!(m1.peak_working_set > 200_000_000, "{m1:?}");
        assert!(m2.peak_working_set < 100_000_000, "{m2:?}");
        assert!(m1.peak_working_set > m2.peak_working_set + 150_000_000);
    }

    #[cfg(windows)]
    #[test]
    fn a_cpu_limit_is_applied_to_the_child() {
        let mut c = Command::new("cmd");
        c.args(["/C", "ver"]);
        let (o, _, confined) = run_measured(&mut c, Duration::from_secs(30), 1 << 16, Some(2)).unwrap();
        assert!(confined, "the job object could not be assigned");
        assert_eq!(o.code, Some(0));
    }
}
