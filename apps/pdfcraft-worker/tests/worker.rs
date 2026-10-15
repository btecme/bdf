//! Behaviour of the worker and supervisor that needs no fault injection.

mod common;

use std::time::Duration;

use common::*;
use pdfcraft_worker::protocol::{self, ErrorMsg, Hello, HelloOk, Kind, Limits, OpenBegin, hex};
use pdfcraft_worker::supervisor::{Session, Worker, WorkerError};
use sha2::{Digest, Sha256};

#[test]
fn a_page_rendered_by_the_worker_equals_the_same_page_rendered_in_process() {
    let doc = tiny_pdf("BDF worker equality");
    let mut s = Session::open(&cfg(), doc.clone(), None).expect("open through the worker");
    assert_eq!(s.pages(), 1);
    let r = s.render(1, 96.0).expect("render through the worker");
    let (w, h, bytes) = in_process(&doc, 0, 96.0);
    assert_eq!((r.width, r.height), (w, h));
    assert_eq!(r.rgba, bytes, "the pixels must survive the pipe unchanged");
    assert!(r.rgba.iter().any(|b| *b != 0), "the page is not blank");
}

#[test]
fn a_large_render_crosses_the_pipe_in_many_numbered_chunks() {
    // 300x300 pt at 600 dpi is 2500x2500 px, 25 MB: about a hundred chunks.
    let doc = tiny_pdf("big");
    let mut s = Session::open(&cfg(), doc.clone(), None).unwrap();
    let r = s.render(1, 600.0).unwrap();
    let (w, h, bytes) = in_process(&doc, 0, 600.0);
    assert_eq!((r.width, r.height, r.rgba.len()), (w, h, bytes.len()));
    assert!(r.rgba.len() > 20_000_000);
    assert_eq!(hex(&Sha256::digest(&r.rgba)), hex(&Sha256::digest(&bytes)));
}

#[test]
fn the_sessions_state_does_not_depend_on_the_worker() {
    let doc = tiny_pdf("state");
    let mut s = Session::open(&cfg(), doc.clone(), None).unwrap();
    let sha = s.document_sha256();
    let before = s.render(1, 72.0).unwrap().rgba;
    // Kill the worker behind the session's back: the next call must start another and give the same answer.
    s.worker_mut().expect("a live worker").kill();
    let after = s.render(1, 72.0).expect("the session recovers from a dead worker");
    assert_eq!(after.rgba, before);
    assert_eq!((s.document_sha256(), s.document_len(), s.pages()), (sha, doc.len(), 1));
    assert!(s.respawns >= 1 || s.retries >= 1);
}

#[test]
fn a_page_out_of_range_is_an_error_and_the_worker_carries_on() {
    let mut s = Session::open(&cfg(), tiny_pdf("range"), None).unwrap();
    for page in [0, 2, 99] {
        let err = s.render(page, 96.0).err().expect("must fail");
        assert!(matches!(&err, WorkerError::Remote { code, .. } if code == "invalid_argument"), "{err:?}");
    }
    assert!(s.render(1, 96.0).is_ok(), "the same worker still serves valid requests");
    assert_eq!(s.respawns, 0, "a refusal is not a failure of the worker");
}

#[test]
fn a_file_that_is_not_a_pdf_is_refused_with_a_message() {
    let err = Session::open(&cfg(), b"this is plain text, not a PDF".to_vec(), None).err().expect("must fail");
    assert!(matches!(&err, WorkerError::Remote { code, message } if code == "invalid_argument" && message.contains("PDF")), "{err:?}");
}

#[test]
fn a_document_that_does_not_match_its_declared_hash_is_never_opened() {
    let mut raw = Raw::start();
    raw.send(Kind::Hello, 1, &protocol::encode(&Hello { v: protocol::VERSION }));
    let hello = raw.recv().expect("hello reply");
    assert_eq!(hello.kind, Kind::HelloOk);
    let doc = tiny_pdf("hash");
    let begin = OpenBegin { total: doc.len() as u64, sha256: "0".repeat(64), password: None };
    raw.send(Kind::OpenBegin, 2, &protocol::encode(&begin));
    raw.send(Kind::Chunk, 2, &protocol::chunk_payload(0, &doc));
    raw.send(Kind::OpenEnd, 2, b"{}");
    let reply = raw.recv().expect("an error reply");
    assert_eq!(reply.kind, Kind::Error);
    assert!(protocol::decode::<ErrorMsg>(&reply.payload).unwrap().message.contains("SHA-256"));
    // And it is still alive to be asked something else.
    raw.send(Kind::Hello, 3, &protocol::encode(&Hello { v: protocol::VERSION }));
    assert_eq!(raw.recv().expect("still serving").kind, Kind::HelloOk);
}

#[test]
fn a_declared_size_over_the_limit_is_refused_before_any_byte_is_sent() {
    let mut raw = Raw::start();
    let begin = OpenBegin { total: Limits::default().max_input + 1, sha256: "a".repeat(64), password: None };
    raw.send(Kind::OpenBegin, 1, &protocol::encode(&begin));
    let reply = raw.recv().expect("a refusal");
    assert_eq!(reply.kind, Kind::Error);
    assert_eq!(protocol::decode::<ErrorMsg>(&reply.payload).unwrap().code, "resource_limit");
}

#[test]
fn a_header_claiming_four_gigabytes_ends_the_worker_without_it_allocating() {
    let mut raw = Raw::start();
    let mut header = Vec::new();
    protocol::write_frame(&mut header, Kind::Chunk, 1, b"", &Limits::default()).unwrap();
    header[16..20].copy_from_slice(&(u32::MAX - 15).to_le_bytes());
    raw.send_bytes(&header);
    assert_eq!(raw.wait_exit(Duration::from_secs(5)), Some(2), "a protocol violation ends the worker with code 2");
    #[cfg(windows)]
    {
        // The process is gone, but its peak counters belong to the process object.
        use std::os::windows::io::AsRawHandle;
        let peak = win32job::utils::get_process_memory_info(raw.child.as_raw_handle() as isize).map(|m| m.peak_working_set_size).unwrap_or(0);
        assert!(peak < 100 * 1024 * 1024, "peak working set {peak} bytes: it must not have tried to hold 4 GiB");
    }
}

#[test]
fn text_on_stdin_instead_of_frames_ends_the_worker() {
    let mut raw = Raw::start();
    raw.send_bytes(b"GET / HTTP/1.1\r\nHost: example.invalid\r\n\r\n");
    assert_eq!(raw.wait_exit(Duration::from_secs(5)), Some(2));
}

#[test]
fn a_chunk_out_of_sequence_ends_the_worker() {
    let mut raw = Raw::start();
    let doc = tiny_pdf("seq");
    let begin = OpenBegin { total: doc.len() as u64, sha256: hex(&Sha256::digest(&doc)), password: None };
    raw.send(Kind::OpenBegin, 1, &protocol::encode(&begin));
    raw.send(Kind::Chunk, 1, &protocol::chunk_payload(5, &doc));
    assert_eq!(raw.wait_exit(Duration::from_secs(5)), Some(2));
}

#[test]
fn the_worker_exits_when_its_input_closes() {
    let mut raw = Raw::start();
    raw.send(Kind::Hello, 1, &protocol::encode(&Hello { v: protocol::VERSION }));
    let ok: HelloOk = protocol::decode(&raw.recv().unwrap().payload).unwrap();
    assert_eq!(ok.pid, raw.child.id());
    raw.stdin = None; // the parent "went away"
    assert_eq!(raw.wait_exit(Duration::from_secs(5)), Some(0));
}

#[test]
fn a_version_it_does_not_speak_is_refused_by_name() {
    let mut raw = Raw::start();
    raw.send(Kind::Hello, 1, &protocol::encode(&Hello { v: 99 }));
    let reply = raw.recv().expect("an error reply");
    assert_eq!(reply.kind, Kind::Error);
    assert!(protocol::decode::<ErrorMsg>(&reply.payload).unwrap().message.contains("99"));
    assert_eq!(raw.wait_exit(Duration::from_secs(5)), Some(2));
}

#[test]
fn shutdown_ends_the_process_cleanly() {
    let w = Worker::spawn(&cfg()).unwrap();
    let pid = w.pid();
    w.shutdown();
    assert!(wait_until(Duration::from_secs(3), || process_gone(pid)));
}

#[cfg(not(feature = "fault-injection"))]
#[test]
fn without_the_feature_a_debug_request_is_refused_not_obeyed() {
    let mut raw = Raw::start();
    raw.send(Kind::Debug, 1, &protocol::encode(&protocol::Debug::Crash));
    let reply = raw.recv().expect("a refusal, not a crash");
    assert_eq!(reply.kind, Kind::Error);
    assert_eq!(protocol::decode::<ErrorMsg>(&reply.payload).unwrap().code, "unsupported_feature");
    assert!(raw.child.try_wait().unwrap().is_none(), "the worker is still running");
}

/// A healthy worker ends when its parent does, because its input closes. (A *wedged* worker cannot
/// notice that; the job object is what ends it, tested in `faults.rs`.)
#[cfg(windows)]
#[test]
fn a_healthy_worker_ends_when_its_parent_is_killed() {
    let (mut parent, pid) = common::start_probe(false);
    assert!(!process_gone(pid), "the worker is running while its parent is");
    parent.kill().unwrap(); // TerminateProcess: no destructors, no chance to clean up
    parent.wait().unwrap();
    assert!(wait_until(Duration::from_secs(5), || process_gone(pid)), "the orphaned worker (pid {pid}) must end with its parent");
}

#[test]
fn the_rest_of_a_refused_transfer_is_ignored_not_treated_as_a_violation() {
    // A parent that has already started sending when the refusal arrives keeps sending chunks; the
    // worker must discard them and stay available, and must have said why exactly once.
    let mut raw = Raw::start();
    let doc = tiny_pdf("refused");
    let begin = OpenBegin { total: Limits::default().max_input + 1, sha256: "a".repeat(64), password: None };
    raw.send(Kind::OpenBegin, 1, &protocol::encode(&begin));
    for seq in 0..3 {
        raw.send(Kind::Chunk, 1, &protocol::chunk_payload(seq, &doc));
    }
    raw.send(Kind::OpenEnd, 1, b"{}");
    let reply = raw.recv().expect("one refusal");
    assert_eq!(reply.kind, Kind::Error);
    assert_eq!(protocol::decode::<ErrorMsg>(&reply.payload).unwrap().code, "resource_limit");
    raw.send(Kind::Hello, 2, &protocol::encode(&Hello { v: protocol::VERSION }));
    let next = raw.recv().expect("still serving");
    assert_eq!(next.kind, Kind::HelloOk, "the next frame is the answer to the next request, not a second error");
    assert!(raw.child.try_wait().unwrap().is_none());
}

#[test]
fn the_parent_refuses_an_oversize_document_without_sending_it() {
    let limits = Limits { max_input: 1024, ..Limits::default() };
    let err = Session::open(&pdfcraft_worker::supervisor::Config { limits, ..cfg() }, vec![b' '; 4096], None).err().expect("must fail");
    assert!(matches!(&err, WorkerError::Remote { code, .. } if code == "resource_limit"), "{err:?}");
}

/// R1: a worker that sends dimensions whose pixel count overflows must be refused as a protocol
/// fault and ended. It must never panic the parent (overflow checks on) nor be believed (off).
#[test]
fn a_reply_whose_dimensions_overflow_is_a_protocol_fault_and_never_a_render() {
    let mut w = Worker::spawn(&fake("overflow")).expect("the fake completes the handshake");
    let pid = w.pid();
    w.open(&tiny_pdf("overflow"), None).expect("the fake accepts the document");
    let err = w.render(1, 96.0).err().expect("an overflowing render must not be accepted");
    assert!(matches!(&err, WorkerError::Protocol(m) if m.contains("dimensions")), "{err:?}");
    assert!(wait_until(Duration::from_secs(3), || process_gone(pid)), "the peer that sent it must be ended");
}

// ---- R2: cancellation must reach every blocking step, not only the wait for a reply ----

/// Cancel `handle` after `after`; returns when it was requested.
fn cancel_later(handle: pdfcraft_worker::supervisor::CancelHandle, after: Duration) -> std::thread::JoinHandle<std::time::Instant> {
    std::thread::spawn(move || {
        std::thread::sleep(after);
        let at = std::time::Instant::now();
        handle.cancel();
        at
    })
}

fn quick_cancel(mode: &str) -> pdfcraft_worker::supervisor::Config {
    pdfcraft_worker::supervisor::Config { cancel_grace: Duration::from_millis(100), call_deadline: Duration::from_secs(4), ..fake(mode) }
}

/// The reviewed counterexample: a worker that completes the handshake and then stops reading. A 16 MiB
/// upload blocks on the full pipe; the caller cannot reach any cancellation logic of its own.
#[test]
fn cancel_ends_an_upload_that_is_blocked_on_a_full_pipe() {
    let mut w = Worker::spawn(&quick_cancel("stall-after-hello")).unwrap();
    let pid = w.pid();
    let later = cancel_later(w.cancel_handle(), Duration::from_millis(100));
    let err = w.open(&vec![0u8; 16 << 20], None).expect_err("must fail");
    let returned = std::time::Instant::now();
    let latency = returned.duration_since(later.join().unwrap());
    assert_eq!(err, WorkerError::Canceled, "{err:?} after {latency:?}");
    assert!(latency < Duration::from_millis(1000), "cancel took {latency:?} to take effect; the budget is 2 s and the grace was 100 ms");
    assert!(wait_until(Duration::from_secs(3), || process_gone(pid)));
}

/// Hashing a large document happens before anything is sent; cancelling then must not wait for it.
#[test]
fn cancel_during_the_hashing_of_a_large_document_is_prompt() {
    let mut w = Worker::spawn(&quick_cancel("silent-render")).unwrap();
    let pid = w.pid();
    // Build the document first: the cancel must land while the call is hashing it, not before it.
    let doc = vec![7u8; 249 << 20]; // the largest document the worker accepts is 250 MiB
    let later = cancel_later(w.cancel_handle(), Duration::from_millis(15));
    let err = w.open(&doc, None).expect_err("must fail: it was cancelled");
    let latency = std::time::Instant::now().duration_since(later.join().unwrap());
    assert_eq!(err, WorkerError::Canceled, "{err:?}");
    // Tight on purpose: the watcher alone would end the worker after the 100 ms grace, but the call
    // would still be hashing. Only a cancel check inside the hashing loop returns this quickly.
    assert!(latency < Duration::from_millis(60), "took {latency:?}: the hashing loop must notice a cancel");
    assert!(wait_until(Duration::from_secs(3), || process_gone(pid)));
}

#[test]
fn cancel_of_a_silent_render_is_classified_canceled_and_the_session_state_is_unchanged() {
    let doc = tiny_pdf("silent");
    let mut s = Session::open(&quick_cancel("silent-render"), doc.clone(), None).unwrap();
    let (sha, len, pages) = (s.document_sha256(), s.document_len(), s.pages());
    let later = cancel_later(s.cancel_handle().expect("a live worker"), Duration::from_millis(100));
    let err = s.render(1, 96.0).err().expect("must fail");
    let latency = std::time::Instant::now().duration_since(later.join().unwrap());
    assert_eq!(err, WorkerError::Canceled, "{err:?}");
    assert!(latency < Duration::from_millis(1000), "took {latency:?}");
    assert_eq!(s.retries, 0, "a cancel is not retried");
    assert_eq!((s.document_sha256(), s.document_len(), s.pages()), (sha, len, pages), "the accepted state is untouched");
}

#[test]
fn cancel_of_a_render_that_started_and_then_stopped_is_prompt() {
    let mut w = Worker::spawn(&quick_cancel("begin-then-stall")).unwrap();
    w.open(&tiny_pdf("stalled"), None).unwrap();
    let pid = w.pid();
    let later = cancel_later(w.cancel_handle(), Duration::from_millis(100));
    let err = w.render(1, 96.0).err().expect("must fail");
    let latency = std::time::Instant::now().duration_since(later.join().unwrap());
    assert_eq!(err, WorkerError::Canceled, "{err:?}");
    assert!(latency < Duration::from_millis(1000), "took {latency:?}");
    assert!(wait_until(Duration::from_secs(3), || process_gone(pid)));
}

// ---- R3: validate the configuration, and fail closed when containment cannot be established ----

#[test]
fn a_cpu_limit_of_zero_is_an_invalid_configuration_not_a_silently_uncontained_worker() {
    let cfg = pdfcraft_worker::supervisor::Config { cores: Some(0), ..cfg() };
    let err = Worker::spawn(&cfg).err().expect("a zero-CPU limit cannot be satisfied and must not be ignored");
    assert!(matches!(err, WorkerError::InvalidConfig(_)), "{err:?}");
}

#[test]
fn a_chunk_limit_too_small_to_carry_data_is_rejected_before_anything_is_sent() {
    // max_chunk - 4 (room for the sequence number) is zero or underflows: `chunks(0)` panics the parent.
    for max_chunk in [0u32, 2, 4] {
        let limits = Limits { max_chunk, ..Limits::default() };
        let err = Session::open(&pdfcraft_worker::supervisor::Config { limits, ..cfg() }, tiny_pdf("chunk"), None).err().expect("must fail");
        assert!(matches!(err, WorkerError::InvalidConfig(_)), "max_chunk {max_chunk}: {err:?}");
    }
}

#[test]
fn nonsense_limits_and_deadlines_are_rejected() {
    use pdfcraft_worker::supervisor::Config;
    for bad in [
        Config { limits: Limits { max_control: 0, ..Limits::default() }, ..cfg() },
        Config { limits: Limits { max_input: 0, ..Limits::default() }, ..cfg() },
        Config { limits: Limits { max_result: 0, ..Limits::default() }, ..cfg() },
        Config { call_deadline: Duration::ZERO, ..cfg() },
        Config { memory_cap: Some(0), ..cfg() },
    ] {
        assert!(matches!(Worker::spawn(&bad).err(), Some(WorkerError::InvalidConfig(_))), "{bad:?}");
    }
}

/// The healthy path of the containment the failure tests above depend on: the job really is applied.
#[cfg(windows)]
#[test]
fn a_contained_worker_is_in_its_job_with_the_cpu_limit_applied() {
    let w = Worker::spawn(&pdfcraft_worker::supervisor::Config { cores: Some(2), ..cfg() }).expect("a worker with a valid CPU limit starts");
    assert!(w.contained());
    assert_eq!(w.cpu_affinity(), Some(0b11), "the job's affinity limit must be in force on the worker process");
}
