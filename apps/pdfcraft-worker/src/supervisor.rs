//! The parent's side: start a worker, talk to it, and contain whatever goes wrong with it.
//!
//! What the supervisor guarantees (each is a test in `tests/worker.rs`):
//!
//! * a call never outlives its deadline, even when the worker is wedged and the parent is blocked
//!   writing to a pipe nobody reads: a watcher thread ends the process at the deadline;
//! * a worker that grows past the memory cap is terminated (a job object's working-set limit is
//!   soft, so the cap is enforced by polling the OS counters);
//! * every response is validated (request id, sizes, order, SHA-256) and a malformed one ends the
//!   worker instead of being believed;
//! * cancellation reaches every blocking step (hashing, a write to a pipe the worker has stopped
//!   reading, the wait for a reply, receiving pixels): the caller gets `Canceled` and the worker is gone
//!   within the grace period, and the watcher does not depend on writing to the pipe to do it;
//! * the worker dies with its parent: it lives in a job object that kills it when the job closes;
//! * none of this touches the parent's own state: [`Session`] keeps the accepted document, and a
//!   dead worker is replaced and, for a read-only render, retried once.
//!
//! What it does **not** do: process separation is not a sandbox. The worker keeps the user's
//! ambient file and network access; see the architecture decision record.

use std::fmt;
use std::io::{BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, sync_channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use crate::protocol::{
    self, ErrorMsg, Frame, Kind, Limits, OpenBegin, Opened, ProtocolError, Render, RenderBegin, chunk_payload, hex, read_frame, write_frame,
};

/// How to start and bound a worker.
#[derive(Clone, Debug)]
pub struct Config {
    pub exe: PathBuf,
    /// Arguments passed to the worker (none for the real worker; tests select a scripted fake).
    pub args: Vec<String>,
    pub limits: Limits,
    /// Longest any single call may take.
    pub call_deadline: Duration,
    /// Terminate the worker when its working set exceeds this many bytes (needs OS counters).
    pub memory_cap: Option<u64>,
    /// After a cancel, how long the worker gets to stop by itself before it is terminated.
    pub cancel_grace: Duration,
    /// Restrict the worker to this many logical CPUs.
    pub cores: Option<u32>,
}

impl Config {
    pub fn new(exe: impl Into<PathBuf>) -> Self {
        Self {
            exe: exe.into(),
            args: Vec::new(),
            limits: Limits::default(),
            call_deadline: Duration::from_secs(30),
            memory_cap: Some(1536 * 1024 * 1024),
            cancel_grace: Duration::from_millis(300),
            cores: None,
        }
    }
}

/// Why a call failed.
#[derive(Debug, PartialEq)]
pub enum WorkerError {
    Spawn(String),
    /// The call ran past its deadline and the worker was terminated.
    Timeout,
    /// The worker process ended without being asked to.
    Crashed(Option<i32>),
    /// The worker sent something the protocol does not allow; it was terminated.
    Protocol(String),
    /// The worker exceeded the memory cap and was terminated.
    ResourceLimit(u64),
    /// The worker reported an error of its own.
    Remote {
        code: String,
        message: String,
    },
    /// The caller cancelled the request.
    Canceled,
    Io(String),
}

impl fmt::Display for WorkerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(e) => write!(f, "could not start the worker: {e}"),
            Self::Timeout => write!(f, "the worker did not answer in time and was terminated"),
            Self::Crashed(c) => write!(f, "the worker ended unexpectedly (exit code {c:?})"),
            Self::Protocol(e) => write!(f, "the worker broke the protocol: {e}"),
            Self::ResourceLimit(b) => write!(f, "the worker exceeded its memory limit of {} MiB and was terminated", b >> 20),
            Self::Remote { code, message } => write!(f, "{code}: {message}"),
            Self::Canceled => write!(f, "the request was cancelled"),
            Self::Io(e) => write!(f, "I/O error talking to the worker: {e}"),
        }
    }
}

impl std::error::Error for WorkerError {}

impl WorkerError {
    /// Whether repeating a read-only request on a fresh worker could help. A timeout, a crash and a
    /// protocol fault might not recur; a memory-limit hit, a refusal or a cancel will.
    pub fn worth_one_retry(&self) -> bool {
        matches!(self, Self::Timeout | Self::Crashed(_) | Self::Protocol(_) | Self::Io(_))
    }
}

/// A rendered page.
pub struct Rendered {
    pub width: u32,
    pub height: u32,
    /// Premultiplied RGBA8.
    pub rgba: Vec<u8>,
    pub millis: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reason {
    Deadline,
    Memory(u64),
    /// The caller cancelled and the grace period ran out.
    Canceled,
}

/// State shared with the watcher and the caller's cancel handle.
struct Shared {
    child: Mutex<Child>,
    /// The worker's stdin. Writers hold the lock for the whole of a (possibly blocking) write, so
    /// anything that must not wait behind one (the watcher) never takes it directly.
    stdin: Mutex<Option<ChildStdin>>,
    limits: Limits,
    cancel_grace: Duration,
    /// Id of the request in progress, the target of a cooperative Cancel.
    call_id: AtomicU64,
    cancel_at: Mutex<Option<Instant>>,
    cancel_sent: AtomicBool,
    cancel: AtomicBool,
    stop: AtomicBool,
    reason: Mutex<Option<Reason>>,
    deadline: Mutex<Option<Instant>>,
    stderr_tail: Mutex<Vec<u8>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Lets another thread cancel the call a [`Worker`] is making.
#[derive(Clone)]
pub struct CancelHandle(Arc<Shared>);

impl CancelHandle {
    /// Ask for the current call to stop. Returns at once. The watcher thread, which never waits on
    /// the call, asks the worker to stop and ends it once the grace period has passed, so the call
    /// returns [`WorkerError::Canceled`] within about that long even when it is blocked writing to a
    /// pipe the worker no longer reads. Ignored when no call is in progress.
    pub fn cancel(&self) {
        if lock(&self.0.deadline).is_some() {
            self.0.cancel.store(true, Ordering::SeqCst);
            lock(&self.0.cancel_at).get_or_insert_with(Instant::now);
        }
    }
}

enum Ev {
    Frame(Frame),
    Eof,
    Bad(ProtocolError),
}

/// One running worker process.
pub struct Worker {
    shared: Arc<Shared>,
    rx: Receiver<Ev>,
    cfg: Config,
    next_id: u64,
    pid: u32,
    #[cfg(windows)]
    _job: Option<win32job::Job>,
}

impl Worker {
    /// Start the worker and complete the version handshake.
    pub fn spawn(cfg: &Config) -> Result<Self, WorkerError> {
        let mut cmd = Command::new(&cfg.exe);
        cmd.args(&cfg.args);
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| WorkerError::Spawn(format!("{}: {e}", cfg.exe.display())))?;
        let pid = child.id();
        #[cfg(windows)]
        let job = contain(&child, cfg.cores);
        let stdin = child.stdin.take().ok_or_else(|| WorkerError::Spawn("no stdin pipe".into()))?;
        let stdout = child.stdout.take().ok_or_else(|| WorkerError::Spawn("no stdout pipe".into()))?;
        let stderr = child.stderr.take().ok_or_else(|| WorkerError::Spawn("no stderr pipe".into()))?;
        let shared = Arc::new(Shared {
            child: Mutex::new(child),
            stdin: Mutex::new(Some(stdin)),
            limits: cfg.limits,
            cancel_grace: cfg.cancel_grace,
            call_id: AtomicU64::new(0),
            cancel_at: Mutex::new(None),
            cancel_sent: AtomicBool::new(false),
            cancel: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            reason: Mutex::new(None),
            deadline: Mutex::new(None),
            stderr_tail: Mutex::new(Vec::new()),
        });
        // Frames are parsed on their own thread; the channel is bounded so a flood backs up into the
        // pipe instead of into our memory.
        let (tx, rx) = sync_channel::<Ev>(16);
        let limits = cfg.limits;
        thread::spawn(move || {
            let mut r = BufReader::with_capacity(64 * 1024, stdout);
            loop {
                let ev = match read_frame(&mut r, &limits) {
                    Ok(Some(f)) => Ev::Frame(f),
                    Ok(None) => Ev::Eof,
                    Err(e) => Ev::Bad(e),
                };
                let last = !matches!(ev, Ev::Frame(_));
                if tx.send(ev).is_err() || last {
                    break;
                }
            }
        });
        let tail = shared.clone();
        thread::spawn(move || {
            let mut r = stderr;
            let mut buf = [0u8; 4096];
            while let Ok(n) = r.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let mut t = lock(&tail.stderr_tail);
                let room = 64 * 1024 - t.len().min(64 * 1024);
                t.extend_from_slice(buf.get(..n.min(room)).unwrap_or_default());
            }
        });
        let watch = shared.clone();
        let cap = cfg.memory_cap;
        thread::spawn(move || watcher(&watch, cap));
        let mut w = Self {
            shared,
            rx,
            cfg: cfg.clone(),
            next_id: 1,
            pid,
            #[cfg(windows)]
            _job: job,
        };
        let id = w.take_id();
        w.begin_call(id);
        let hello = protocol::encode(&protocol::Hello { v: protocol::VERSION });
        w.send(Kind::Hello, id, &hello)?;
        let f = w.await_frame(id)?;
        w.end_call();
        match f.kind {
            Kind::HelloOk => {
                let ok: protocol::HelloOk = protocol::decode(&f.payload).map_err(|e| w.fail_protocol(e.to_string()))?;
                if ok.pid != pid {
                    return Err(w.fail_protocol(format!("handshake came from process {} but the child is {pid}", ok.pid)));
                }
                Ok(w)
            }
            _ => Err(w.remote_or_protocol(f)),
        }
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub fn cancel_handle(&self) -> CancelHandle {
        CancelHandle(self.shared.clone())
    }

    /// The worker's peak working set and peak commit so far, in bytes (Windows).
    pub fn memory(&self) -> Option<(u64, u64)> {
        memory_of(&lock(&self.shared.child))
    }

    /// The last bytes the worker wrote to stderr.
    pub fn stderr_tail(&self) -> String {
        String::from_utf8_lossy(&lock(&self.shared.stderr_tail)).into_owned()
    }

    fn take_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn begin_call(&self, id: u64) {
        self.shared.call_id.store(id, Ordering::SeqCst);
        self.shared.cancel.store(false, Ordering::SeqCst);
        self.shared.cancel_sent.store(false, Ordering::SeqCst);
        *lock(&self.shared.cancel_at) = None;
        *lock(&self.shared.deadline) = Some(Instant::now() + self.cfg.call_deadline);
    }

    fn cancel_requested(&self) -> bool {
        self.shared.cancel.load(Ordering::SeqCst)
    }

    /// A call that was cancelled returns `Canceled` even if its answer happened to arrive, and the
    /// worker, which may still be mid-reply, is ended so the next call starts clean.
    fn finish<T>(&mut self, result: Result<T, WorkerError>) -> Result<T, WorkerError> {
        self.end_call();
        if self.cancel_requested() {
            self.kill();
            return Err(WorkerError::Canceled);
        }
        result
    }

    fn end_call(&self) {
        *lock(&self.shared.deadline) = None;
    }

    /// End the worker now.
    pub fn kill(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        let mut c = lock(&self.shared.child);
        let _ = c.kill();
        let _ = c.wait();
    }

    fn send(&mut self, kind: Kind, id: u64, payload: &[u8]) -> Result<(), WorkerError> {
        let written = match lock(&self.shared.stdin).as_mut() {
            Some(w) => write_frame(w, kind, id, payload, &self.cfg.limits),
            None => Err(ProtocolError::Io("the worker's input is closed".into())),
        };
        written.map_err(|e| self.classify(e))
    }

    /// Map a write failure to what actually happened to the worker.
    fn classify(&mut self, e: ProtocolError) -> WorkerError {
        match self.ended() {
            Some(err) => err,
            None => WorkerError::Io(e.to_string()),
        }
    }

    /// If the worker is gone, why.
    fn ended(&mut self) -> Option<WorkerError> {
        let mut c = lock(&self.shared.child);
        let status = match c.try_wait() {
            Ok(Some(s)) => s,
            // Still running (or not inspectable): give a just-killed process a moment to be reaped.
            _ => {
                if lock(&self.shared.reason).is_some() {
                    let _ = c.wait();
                    return Some(self.reason_error());
                }
                return None;
            }
        };
        if lock(&self.shared.reason).is_some() {
            return Some(self.reason_error());
        }
        Some(WorkerError::Crashed(status.code()))
    }

    fn reason_error(&self) -> WorkerError {
        match *lock(&self.shared.reason) {
            Some(Reason::Memory(cap)) => WorkerError::ResourceLimit(cap),
            Some(Reason::Canceled) => WorkerError::Canceled,
            _ => WorkerError::Timeout,
        }
    }

    fn fail_protocol(&mut self, what: String) -> WorkerError {
        self.kill();
        WorkerError::Protocol(what)
    }

    fn remote_or_protocol(&mut self, f: Frame) -> WorkerError {
        if f.kind == Kind::Error
            && let Ok(e) = protocol::decode::<ErrorMsg>(&f.payload)
        {
            return WorkerError::Remote { code: e.code, message: e.message };
        }
        self.fail_protocol(format!("unexpected {:?} frame", f.kind))
    }

    /// Wait for the next frame for request `id`. The deadline and a cancellation are enforced by the
    /// watcher thread, which ends the worker; this loop then sees the worker's end and says why.
    fn await_frame(&mut self, id: u64) -> Result<Frame, WorkerError> {
        loop {
            match self.rx.recv_timeout(Duration::from_millis(5)) {
                Ok(Ev::Frame(f)) => {
                    if f.kind == Kind::Canceled && f.id == id {
                        return Err(WorkerError::Canceled);
                    }
                    if f.id != id {
                        return Err(self.fail_protocol(format!("{:?} frame for request {} while waiting for {id}", f.kind, f.id)));
                    }
                    return Ok(f);
                }
                Ok(Ev::Eof) => {
                    self.kill();
                    return Err(self.ended().unwrap_or(WorkerError::Crashed(None)));
                }
                Ok(Ev::Bad(e)) => {
                    // A worker that is already gone explains the bad stream better than the stream does.
                    if let Some(err) = self.ended() {
                        return Err(err);
                    }
                    return Err(self.fail_protocol(e.to_string()));
                }
                Err(RecvTimeoutError::Timeout) => {
                    if let Some(err) = self.ended() {
                        return Err(err);
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(self.ended().unwrap_or(WorkerError::Crashed(None)));
                }
            }
        }
    }

    /// Send a document and open it. Returns the page count.
    pub fn open(&mut self, doc: &[u8], password: Option<&str>) -> Result<u32, WorkerError> {
        let id = self.take_id();
        self.begin_call(id);
        let result = self.open_inner(id, doc, password);
        self.finish(result)
    }

    fn open_inner(&mut self, id: u64, doc: &[u8], password: Option<&str>) -> Result<u32, WorkerError> {
        // Refuse here what the worker would refuse anyway, without sending a byte of it.
        if doc.len() as u64 > self.cfg.limits.max_input {
            return Err(WorkerError::Remote {
                code: "resource_limit".into(),
                message: format!("{} bytes exceeds the {} byte input limit", doc.len(), self.cfg.limits.max_input),
            });
        }
        // Hash in pieces so a cancel is noticed while a large document is being hashed.
        let mut hasher = Sha256::new();
        for piece in doc.chunks(1 << 20) {
            if self.cancel_requested() {
                self.kill();
                return Err(WorkerError::Canceled);
            }
            hasher.update(piece);
        }
        let begin = OpenBegin { total: doc.len() as u64, sha256: hex(&hasher.finalize()), password: password.map(str::to_string) };
        self.send(Kind::OpenBegin, id, &protocol::encode(&begin))?;
        let max_data = self.cfg.limits.max_chunk as usize - 4;
        for (seq, piece) in doc.chunks(max_data).enumerate() {
            if self.cancel_requested() {
                self.kill();
                return Err(WorkerError::Canceled);
            }
            self.send(Kind::Chunk, id, &chunk_payload(u32::try_from(seq).unwrap_or(u32::MAX), piece))?;
        }
        self.send(Kind::OpenEnd, id, b"{}")?;
        let f = self.await_frame(id)?;
        match f.kind {
            Kind::Opened => {
                let o: Opened = protocol::decode(&f.payload).map_err(|e| self.fail_protocol(e.to_string()))?;
                Ok(o.pages)
            }
            _ => Err(self.remote_or_protocol(f)),
        }
    }

    /// Render a 1-based page at `dpi`.
    pub fn render(&mut self, page: u32, dpi: f32) -> Result<Rendered, WorkerError> {
        let id = self.take_id();
        self.begin_call(id);
        let result = self.render_inner(id, page, dpi);
        self.finish(result)
    }

    fn render_inner(&mut self, id: u64, page: u32, dpi: f32) -> Result<Rendered, WorkerError> {
        let req = Render { page, dpi_milli: (dpi * 1000.0).round().clamp(0.0, f32::from(u16::MAX) * 1000.0) as u32 };
        self.send(Kind::Render, id, &protocol::encode(&req))?;
        let f = self.await_frame(id)?;
        if f.kind != Kind::RenderBegin {
            return Err(self.remote_or_protocol(f));
        }
        let begin: RenderBegin = protocol::decode(&f.payload).map_err(|e| self.fail_protocol(e.to_string()))?;
        let expected = match protocol::rgba_len(begin.width, begin.height, self.cfg.limits.max_result) {
            Ok(n) => n,
            Err(why) => return Err(self.fail_protocol(format!("render dimensions {}x{}: {why}", begin.width, begin.height))),
        };
        if begin.total != expected as u64 {
            return Err(self.fail_protocol(format!(
                "render dimensions {}x{} need {expected} bytes but {} are declared",
                begin.width, begin.height, begin.total
            )));
        }
        let mut pixels = Vec::new();
        if pixels.try_reserve_exact(expected).is_err() {
            return Err(self.fail_protocol("cannot reserve memory for the render".into()));
        }
        let mut hasher = Sha256::new();
        let mut next_seq = 0u32;
        loop {
            let f = self.await_frame(id)?;
            match f.kind {
                Kind::Chunk => {
                    let (seq, data) = protocol::split_chunk(&f.payload).map_err(|e| self.fail_protocol(e.to_string()))?;
                    if seq != next_seq || (pixels.len() + data.len()) as u64 > begin.total {
                        return Err(self.fail_protocol(format!("chunk {seq} out of sequence or over the declared total")));
                    }
                    hasher.update(data);
                    pixels.extend_from_slice(data);
                    next_seq += 1;
                }
                Kind::RenderEnd => break,
                _ => return Err(self.remote_or_protocol(f)),
            }
        }
        if pixels.len() as u64 != begin.total || hex(&hasher.finalize()) != begin.sha256 {
            return Err(self.fail_protocol("the pixels do not match their declared length and SHA-256".into()));
        }
        Ok(Rendered { width: begin.width, height: begin.height, rgba: pixels, millis: begin.millis })
    }

    /// Send a fault-injection request (the worker must be a `fault-injection` build). Does not wait.
    pub fn debug(&mut self, d: &protocol::Debug) -> Result<(), WorkerError> {
        let id = self.take_id();
        self.send(Kind::Debug, id, &protocol::encode(d))
    }

    /// Send a fault-injection request and wait for the reply to it (used for requests that answer).
    pub fn debug_and_wait(&mut self, d: &protocol::Debug) -> Result<Frame, WorkerError> {
        let id = self.take_id();
        self.begin_call(id);
        let result = self.send(Kind::Debug, id, &protocol::encode(d)).and_then(|()| self.await_frame(id));
        self.finish(result)
    }

    /// Ask the worker to exit and wait for it.
    pub fn shutdown(mut self) {
        let id = self.take_id();
        let _ = self.send(Kind::Shutdown, id, b"{}");
        if let Some(w) = lock(&self.shared.stdin).as_mut() {
            let _ = w.flush();
        }
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(2) {
            if matches!(lock(&self.shared.child).try_wait(), Ok(Some(_))) {
                self.shared.stop.store(true, Ordering::SeqCst);
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
        self.kill();
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.kill();
    }
}

/// The watcher: ends the worker at its call deadline and when it grows past the memory cap.
fn watcher(shared: &Arc<Shared>, cap: Option<u64>) {
    loop {
        thread::sleep(Duration::from_millis(15));
        if shared.stop.load(Ordering::SeqCst) {
            return;
        }
        let mut child = lock(&shared.child);
        if !matches!(child.try_wait(), Ok(None)) {
            return;
        }
        let deadline = *lock(&shared.deadline);
        if deadline.is_some_and(|d| Instant::now() >= d) {
            *lock(&shared.reason) = Some(Reason::Deadline);
            let _ = child.kill();
            return;
        }
        let cancel_at = *lock(&shared.cancel_at);
        if let (Some(at), true) = (cancel_at, deadline.is_some()) {
            if !shared.cancel_sent.swap(true, Ordering::SeqCst) {
                // Ask the worker to stop by itself, from a helper thread: the write can block on a full
                // pipe, and the watcher must never be the thing that is blocked.
                let s = Arc::clone(shared);
                thread::spawn(move || {
                    let id = s.call_id.load(Ordering::SeqCst);
                    let body = protocol::encode(&protocol::Cancel { target: id });
                    if let Some(w) = lock(&s.stdin).as_mut() {
                        let _ = write_frame(w, Kind::Cancel, id, &body, &s.limits);
                    }
                });
            }
            if at.elapsed() >= shared.cancel_grace {
                *lock(&shared.reason) = Some(Reason::Canceled);
                let _ = child.kill();
                return;
            }
        }
        if let (Some(cap), Some(used)) = (cap, working_set(&child))
            && used > cap
        {
            *lock(&shared.reason) = Some(Reason::Memory(cap));
            let _ = child.kill();
            return;
        }
    }
}

#[cfg(windows)]
fn memory_of(child: &Child) -> Option<(u64, u64)> {
    use std::os::windows::io::AsRawHandle;
    win32job::utils::get_process_memory_info(child.as_raw_handle() as isize)
        .ok()
        .map(|c| (c.peak_working_set_size as u64, c.peak_pagefile_usage as u64))
}

#[cfg(not(windows))]
fn memory_of(_child: &Child) -> Option<(u64, u64)> {
    None
}

#[cfg(windows)]
fn working_set(child: &Child) -> Option<u64> {
    use std::os::windows::io::AsRawHandle;
    win32job::utils::get_process_memory_info(child.as_raw_handle() as isize).ok().map(|c| c.working_set_size as u64)
}

#[cfg(not(windows))]
fn working_set(_child: &Child) -> Option<u64> {
    None
}

/// Put the worker in a job object that kills it when the job is closed (that is, when this process
/// ends, however it ends), optionally limiting its CPUs.
#[cfg(windows)]
fn contain(child: &Child, cores: Option<u32>) -> Option<win32job::Job> {
    use std::os::windows::io::AsRawHandle;
    let mut info = win32job::ExtendedLimitInfo::new();
    info.limit_kill_on_job_close();
    if let Some(c) = cores {
        info.limit_affinity(if c >= usize::BITS { usize::MAX } else { (1usize << c) - 1 });
    }
    let job = win32job::Job::create_with_limit_info(&info).ok()?;
    job.assign_process(child.as_raw_handle() as isize).ok()?;
    Some(job)
}

/// The parent's accepted state for one document, and the workers that serve it.
///
/// The document bytes, their hash and the page count live here, in the parent. A worker is a
/// disposable view of them: when one dies the session's state is untouched and the next call starts
/// a new worker and re-sends the document.
pub struct Session {
    cfg: Config,
    doc: Arc<Vec<u8>>,
    password: Option<String>,
    pages: u32,
    worker: Option<Worker>,
    /// Workers started after the first (a measure of how often containment was needed).
    pub respawns: u32,
    /// Read-only calls repeated after a failure.
    pub retries: u32,
}

impl Session {
    pub fn open(cfg: &Config, doc: Vec<u8>, password: Option<String>) -> Result<Self, WorkerError> {
        let mut worker = Worker::spawn(cfg)?;
        let pages = worker.open(&doc, password.as_deref())?;
        Ok(Self { cfg: cfg.clone(), doc: Arc::new(doc), password, pages, worker: Some(worker), respawns: 0, retries: 0 })
    }

    pub fn pages(&self) -> u32 {
        self.pages
    }

    /// SHA-256 of the accepted document.
    pub fn document_sha256(&self) -> String {
        hex(&Sha256::digest(self.doc.as_slice()))
    }

    pub fn document_len(&self) -> usize {
        self.doc.len()
    }

    /// Peak memory of the live worker, if there is one.
    pub fn worker_memory(&self) -> Option<(u64, u64)> {
        self.worker.as_ref().and_then(Worker::memory)
    }

    pub fn cancel_handle(&self) -> Option<CancelHandle> {
        self.worker.as_ref().map(Worker::cancel_handle)
    }

    /// The live worker, for fault injection in tests.
    pub fn worker_mut(&mut self) -> Option<&mut Worker> {
        self.worker.as_mut()
    }

    fn ensure_worker(&mut self) -> Result<&mut Worker, WorkerError> {
        if self.worker.is_none() {
            let mut w = Worker::spawn(&self.cfg)?;
            let pages = w.open(&self.doc, self.password.as_deref())?;
            if pages != self.pages {
                return Err(WorkerError::Protocol(format!("a fresh worker reports {pages} pages; the session accepted {}", self.pages)));
            }
            self.worker = Some(w);
            self.respawns += 1;
        }
        self.worker.as_mut().ok_or_else(|| WorkerError::Spawn("no worker".into()))
    }

    /// Render a page. A read-only request, so a crash, hang or protocol fault is retried once on a
    /// fresh worker; a memory-limit hit, a refusal or a cancel is not.
    pub fn render(&mut self, page: u32, dpi: f32) -> Result<Rendered, WorkerError> {
        let first = self.ensure_worker().and_then(|w| w.render(page, dpi));
        match first {
            Err(e) if e.worth_one_retry() => {
                self.worker = None;
                self.retries += 1;
                let second = self.ensure_worker().and_then(|w| w.render(page, dpi));
                if second.is_err() {
                    self.worker = None;
                }
                second
            }
            Err(e) => {
                // The worker is dead or untrusted after any of these; the session is not.
                if !matches!(e, WorkerError::Remote { .. }) {
                    self.worker = None;
                }
                Err(e)
            }
            ok => ok,
        }
    }
}
