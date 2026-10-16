//! A worker that hangs, crashes, grows without bound or lies must not take the parent's state with it.
//! Needs the test-only `fault-injection` feature:
//! `cargo test -p pdfcraft-worker --features fault-injection`.
#![cfg(feature = "fault-injection")]

mod common;

use std::time::{Duration, Instant};

use common::*;
use pdfcraft_worker::protocol::Debug;
use pdfcraft_worker::supervisor::{Config, Session, Worker, WorkerError};

fn quick() -> Config {
    Config { call_deadline: Duration::from_millis(800), ..cfg() }
}

#[test]
fn a_crash_is_reported_as_one_and_a_read_only_render_is_retried_on_a_fresh_worker() {
    let doc = tiny_pdf("crash once");
    let mut s = Session::open(&cfg(), doc.clone(), None).unwrap();
    let (sha, len, pages) = (s.document_sha256(), s.document_len(), s.pages());
    s.worker_mut().unwrap().debug(&Debug::CrashOnce).unwrap();
    let r = s.render(1, 96.0).expect("the retry on a new worker succeeds");
    assert_eq!(r.rgba, in_process(&doc, 0, 96.0).2);
    assert_eq!((s.retries, s.respawns), (1, 1));
    assert_eq!((s.document_sha256(), s.document_len(), s.pages()), (sha, len, pages), "the accepted state is untouched");
}

#[test]
fn a_crash_without_a_retry_names_the_exit_code() {
    let mut w = Worker::spawn(&cfg()).unwrap();
    w.open(&tiny_pdf("crash"), None).unwrap();
    w.debug(&Debug::Crash).unwrap();
    let err = w.render(1, 96.0).err().expect("must fail");
    assert!(matches!(err, WorkerError::Crashed(Some(code)) if code != 0), "{err:?}");
}

#[test]
fn a_hung_worker_is_terminated_at_the_deadline_and_never_outlives_it() {
    let mut w = Worker::spawn(&quick()).unwrap();
    w.open(&tiny_pdf("hang"), None).unwrap();
    let pid = w.pid();
    w.debug(&Debug::Hang).unwrap();
    let start = Instant::now();
    let err = w.render(1, 96.0).err().expect("must fail");
    let took = start.elapsed();
    assert_eq!(err, WorkerError::Timeout);
    assert!(took >= Duration::from_millis(700) && took < Duration::from_millis(2500), "took {took:?} for an 800 ms deadline");
    assert!(wait_until(Duration::from_secs(3), || process_gone(pid)), "the wedged worker must be gone");
}

#[test]
fn a_session_survives_a_hang_and_serves_the_next_request() {
    let doc = tiny_pdf("after hang");
    let mut s = Session::open(&quick(), doc.clone(), None).unwrap();
    s.worker_mut().unwrap().debug(&Debug::Hang).unwrap();
    // The first attempt hangs and is retried once on a fresh worker; the retry succeeds.
    let r = s.render(1, 96.0).expect("retried after the hang");
    assert_eq!(r.rgba, in_process(&doc, 0, 96.0).2);
    assert_eq!(s.retries, 1);
}

#[test]
fn cancel_answers_at_once_and_the_worker_is_gone_within_the_grace_period() {
    let mut w = Worker::spawn(&Config { cancel_grace: Duration::from_millis(300), ..cfg() }).unwrap();
    w.open(&tiny_pdf("cancel"), None).unwrap();
    let pid = w.pid();
    w.debug(&Debug::Hang).unwrap();
    let handle = w.cancel_handle();
    let t = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        let at = Instant::now();
        handle.cancel();
        at
    });
    let err = w.render(1, 96.0).err().expect("must fail");
    let returned = Instant::now();
    let cancelled_at = t.join().unwrap();
    assert_eq!(err, WorkerError::Canceled);
    let latency = returned.duration_since(cancelled_at);
    eprintln!("cancel to return: {latency:?}");
    assert!(latency < Duration::from_millis(1000), "cancel took {latency:?}; the budget is 2 s");
    assert!(wait_until(Duration::from_secs(2), || process_gone(pid)));
}

/// The cap is enforced by polling the OS memory counters, which the supervisor reads on Windows only;
/// without them an uncontained worker is not limited, so the guarantee is not asserted elsewhere.
#[cfg(windows)]
#[test]
fn a_worker_over_the_memory_cap_is_terminated() {
    let cap = 200 * 1024 * 1024;
    let mut s = Session::open(&Config { memory_cap: Some(cap), call_deadline: Duration::from_secs(20), ..cfg() }, tiny_pdf("memory"), None).unwrap();
    let pid = s.worker_mut().unwrap().pid();
    s.worker_mut().unwrap().debug(&Debug::Alloc { mib: 900 }).unwrap();
    let start = Instant::now();
    let err = s.render(1, 96.0).err().expect("must fail");
    // Not retried: it would only do the same again.
    assert_eq!(err, WorkerError::ResourceLimit(cap), "{err:?}");
    assert_eq!(s.retries, 0);
    assert!(start.elapsed() < Duration::from_secs(10), "took {:?}", start.elapsed());
    assert!(wait_until(Duration::from_secs(3), || process_gone(pid)));
    // The session is unharmed and starts a new worker for the next request.
    assert!(s.render(1, 96.0).is_ok());
}

#[test]
fn text_where_a_frame_should_be_is_a_protocol_fault_that_ends_the_worker() {
    let mut w = Worker::spawn(&cfg()).unwrap();
    w.open(&tiny_pdf("spam"), None).unwrap();
    let pid = w.pid();
    w.debug(&Debug::Spam).unwrap();
    let err = w.render(1, 96.0).err().expect("must fail");
    assert!(matches!(err, WorkerError::Protocol(_)), "{err:?}");
    assert!(wait_until(Duration::from_secs(3), || process_gone(pid)));
}

#[test]
fn a_reply_for_the_wrong_request_is_rejected() {
    let mut w = Worker::spawn(&cfg()).unwrap();
    w.open(&tiny_pdf("id"), None).unwrap();
    w.debug(&Debug::WrongId).unwrap();
    let err = w.render(1, 96.0).err().expect("must fail");
    assert!(matches!(&err, WorkerError::Protocol(m) if m.contains("request")), "{err:?}");
}

#[test]
fn pixels_that_do_not_match_their_hash_are_rejected() {
    let mut w = Worker::spawn(&cfg()).unwrap();
    w.open(&tiny_pdf("hash"), None).unwrap();
    w.debug(&Debug::WrongHash).unwrap();
    let err = w.render(1, 96.0).err().expect("must fail");
    assert!(matches!(&err, WorkerError::Protocol(m) if m.contains("SHA-256")), "{err:?}");
}

#[test]
fn a_chunk_header_claiming_four_gigabytes_is_rejected_without_the_parent_allocating() {
    let mut w = Worker::spawn(&cfg()).unwrap();
    w.open(&tiny_pdf("huge"), None).unwrap();
    w.debug(&Debug::HugeChunk).unwrap();
    let err = w.render(1, 96.0).err().expect("must fail");
    assert!(matches!(&err, WorkerError::Protocol(m) if m.contains("limit")), "{err:?}");
}

#[test]
fn a_session_recovers_from_every_kind_of_worker_failure_with_its_state_intact() {
    let doc = tiny_pdf("all faults");
    let expected = in_process(&doc, 0, 96.0).2;
    let mut s = Session::open(&Config { call_deadline: Duration::from_secs(2), ..cfg() }, doc, None).unwrap();
    let sha = s.document_sha256();
    for fault in [Debug::WrongId, Debug::WrongHash, Debug::HugeChunk, Debug::Spam, Debug::Crash, Debug::Hang] {
        s.worker_mut().expect("a worker").debug(&fault).unwrap();
        // The first attempt fails; the retry on a fresh worker succeeds.
        let r = s.render(1, 96.0).unwrap_or_else(|e| panic!("{fault:?}: {e}"));
        assert_eq!(r.rgba, expected, "{fault:?}");
        assert_eq!(s.document_sha256(), sha);
    }
    assert!(s.retries >= 6, "{} retries", s.retries);
}

/// A wedged worker cannot notice that its input closed, so it would outlive a parent that dies. The
/// job object is what ends it.
#[cfg(windows)]
#[test]
fn a_wedged_worker_dies_when_its_parent_is_killed() {
    let (mut parent, pid) = common::start_probe(true);
    assert!(!process_gone(pid), "the wedged worker is running while its parent is");
    parent.kill().unwrap(); // TerminateProcess: no destructors, no chance to clean up
    parent.wait().unwrap();
    let died = wait_until(Duration::from_secs(5), || process_gone(pid));
    if !died {
        // Do not leave the orphan behind (it would also keep cargo's output pipes open).
        let _ = std::process::Command::new("taskkill").args(["/PID", &pid.to_string(), "/F"]).output();
    }
    assert!(died, "the orphaned, wedged worker (pid {pid}) must be killed with its parent");
}

/// The memory cap is enforced by polling the OS counters (a job object's working-set limit is soft
/// and `win32job` does not expose a hard commit limit), so a worker can overshoot the cap by what it
/// allocates between two polls. This test records how far, so the design decision rests on a number.
#[cfg(windows)]
#[test]
fn how_far_a_worker_overshoots_the_memory_cap_is_bounded_and_recorded() {
    let cap: u64 = 200 * 1024 * 1024;
    let mut w = Worker::spawn(&Config { memory_cap: Some(cap), call_deadline: Duration::from_secs(20), ..cfg() }).unwrap();
    w.open(&tiny_pdf("overshoot"), None).unwrap();
    // A worker that allocates steadily (here 600 MiB, touched page by page in one burst).
    w.debug(&Debug::Alloc { mib: 600 }).unwrap();
    let err = w.render(1, 96.0).err().expect("must fail");
    assert_eq!(err, WorkerError::ResourceLimit(cap));
    let (peak_ws, peak_commit) = w.memory().expect("the OS counters");
    eprintln!("cap {} MiB; peak working set {} MiB; peak commit {} MiB", cap >> 20, peak_ws >> 20, peak_commit >> 20);
    assert!(peak_ws >= cap, "it was only stopped after crossing the cap");
    assert!(peak_ws < 700 * 1024 * 1024, "peak working set {} MiB: it must not have finished the whole allocation and kept going", peak_ws >> 20);
}

// ---- R3: injected failures of the containment setup must fail startup closed ----

/// The pid named in a containment error ("... (worker process N was ended)").
#[cfg(windows)]
fn pid_in(err: &WorkerError) -> u32 {
    let WorkerError::Containment(m) = err else { panic!("not a containment error: {err:?}") };
    let digits: String = m.split("worker process ").nth(1).expect("the pid is named").chars().take_while(char::is_ascii_digit).collect();
    digits.parse().expect("a pid")
}

#[cfg(windows)]
#[test]
fn a_job_that_cannot_be_created_fails_startup_closed_and_leaves_no_worker() {
    let cfg = Config { inject: Some(pdfcraft_worker::supervisor::Injection::JobCreate), ..cfg() };
    let err = Worker::spawn(&cfg).err().expect("an uncontained worker must not be handed out");
    assert!(matches!(err, WorkerError::Containment(_)), "{err:?}");
    let pid = pid_in(&err);
    assert!(wait_until(Duration::from_secs(3), || process_gone(pid)), "the child that was started must be ended");
}

#[cfg(windows)]
#[test]
fn a_process_that_cannot_be_assigned_to_its_job_fails_startup_closed_and_leaves_no_worker() {
    let cfg = Config { inject: Some(pdfcraft_worker::supervisor::Injection::JobAssign), ..cfg() };
    let err = Worker::spawn(&cfg).err().expect("an uncontained worker must not be handed out");
    assert!(matches!(err, WorkerError::Containment(_)), "{err:?}");
    let pid = pid_in(&err);
    assert!(wait_until(Duration::from_secs(3), || process_gone(pid)), "the child that was started must be ended");
}

/// Running uncontained is possible only when the caller says so, and the worker then reports it.
#[cfg(windows)]
#[test]
fn running_uncontained_is_an_explicit_opt_in_and_is_reported() {
    let cfg = Config { inject: Some(pdfcraft_worker::supervisor::Injection::JobCreate), allow_uncontained: true, ..cfg() };
    let w = Worker::spawn(&cfg).expect("allowed to run uncontained");
    assert!(!w.contained(), "an uncontained worker must say so");
}
