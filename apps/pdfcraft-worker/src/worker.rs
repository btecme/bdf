//! The worker process body: receive a document in bounded, hashed chunks, open it, render pages on
//! request and send the pixels back the same way.
//!
//! The worker trusts nothing it is sent. Declared sizes are checked before memory is reserved, the
//! document hash is checked before it is parsed, requested pages and DPI are range-checked, and a
//! panic in the renderer becomes an error message rather than a dead process. What it cannot do
//! from inside a single-threaded render call is stop one halfway: a render that must be cut short
//! is ended by the parent terminating the process, which is exactly what the supervisor is for.
//!
//! stdout carries frames and nothing else (the crate denies `print!`); diagnostics go to stderr.

use std::io::{Read, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;

use pdfcraft_render::{PageRenderer, RenderConfig, RenderRequest, RequestKind};
use sha2::{Digest, Sha256};

use crate::protocol::{
    self, Cancel, ErrorMsg, Frame, Hello, HelloOk, Kind, Limits, OpenBegin, Opened, ProtocolError, Render, RenderBegin, chunk_payload, hex,
    read_frame, split_chunk, write_frame,
};

/// What the worker will accept and produce.
#[derive(Clone, Copy, Debug)]
pub struct WorkerConfig {
    pub limits: Limits,
    /// Refuse to render a page whose pixel count would exceed this.
    pub max_pixels: u64,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self { limits: Limits::default(), max_pixels: 64_000_000 }
    }
}

enum Event {
    Frame(Frame),
    Fault(ProtocolError),
    Eof,
}

/// A document being received.
struct Incoming {
    id: u64,
    total: u64,
    expect_sha: String,
    hasher: Sha256,
    buf: Vec<u8>,
    next_seq: u32,
    password: Option<String>,
    /// The transfer was refused (and the refusal sent); its remaining chunks are discarded.
    refused: bool,
}

impl Incoming {
    fn refused(id: u64) -> Self {
        Self { id, total: 0, expect_sha: String::new(), hasher: Sha256::new(), buf: Vec::new(), next_seq: 0, password: None, refused: true }
    }
}

struct Open {
    renderer: PageRenderer,
    pages: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Flow {
    Continue,
    Exit(i32),
}

fn send(out: &mut impl Write, kind: Kind, id: u64, payload: &[u8], limits: &Limits) -> Result<(), ProtocolError> {
    write_frame(out, kind, id, payload, limits)
}

fn send_error(out: &mut impl Write, id: u64, code: &str, message: &str, limits: &Limits) -> Result<(), ProtocolError> {
    let msg = ErrorMsg { code: code.to_string(), message: message.chars().take(500).collect() };
    send(out, Kind::Error, id, &protocol::encode(&msg), limits)
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Serve requests from `input` until it ends or a protocol violation, writing replies to `output`.
/// Returns the process exit code: 0 for a clean end, 2 for a protocol violation.
pub fn run(input: impl Read + Send + 'static, mut output: impl Write, cfg: &WorkerConfig) -> i32 {
    let cancel = Arc::new(AtomicU64::new(0));
    let (tx, rx) = mpsc::sync_channel::<Event>(8);
    let reader_cancel = cancel.clone();
    let limits = cfg.limits;
    // Frames are read on their own thread so a Cancel is seen even while a render is running.
    thread::spawn(move || {
        let mut input = input;
        loop {
            let event = match read_frame(&mut input, &limits) {
                Ok(Some(f)) => {
                    if f.kind == Kind::Cancel
                        && let Ok(c) = protocol::decode::<Cancel>(&f.payload)
                    {
                        reader_cancel.store(c.target, Ordering::SeqCst);
                    }
                    Event::Frame(f)
                }
                Ok(None) => Event::Eof,
                Err(e) => Event::Fault(e),
            };
            let last = !matches!(event, Event::Frame(_));
            if tx.send(event).is_err() || last {
                break;
            }
        }
    });
    let mut incoming: Option<Incoming> = None;
    let mut open: Option<Open> = None;
    let mut fault: Option<protocol::Debug> = None;
    loop {
        match rx.recv() {
            Ok(Event::Frame(frame)) => {
                let flow = catch_unwind(AssertUnwindSafe(|| handle(frame, &mut output, cfg, &cancel, &mut incoming, &mut open, &mut fault)));
                match flow {
                    Ok(Ok(Flow::Continue)) => {}
                    Ok(Ok(Flow::Exit(code))) => return code,
                    // The parent is gone or sent nonsense we cannot answer.
                    Ok(Err(_)) => return 2,
                    Err(_) => {
                        eprintln!("pdfcraft-worker: panic while handling a request");
                        let _ = send_error(&mut output, 0, "internal_failure", "the worker hit an internal error", &cfg.limits);
                        return 3;
                    }
                }
            }
            Ok(Event::Fault(e)) => {
                eprintln!("pdfcraft-worker: protocol violation: {e}");
                let _ = send_error(&mut output, 0, "invalid_argument", &e.to_string(), &cfg.limits);
                return 2;
            }
            Ok(Event::Eof) | Err(_) => return 0,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn handle(
    frame: Frame,
    out: &mut impl Write,
    cfg: &WorkerConfig,
    cancel: &AtomicU64,
    incoming: &mut Option<Incoming>,
    open: &mut Option<Open>,
    fault: &mut Option<protocol::Debug>,
) -> Result<Flow, ProtocolError> {
    let l = &cfg.limits;
    let id = frame.id;
    match frame.kind {
        Kind::Hello => {
            let h: Hello = protocol::decode(&frame.payload)?;
            if h.v != protocol::VERSION {
                send_error(out, id, "invalid_argument", &format!("protocol version {} is not supported", h.v), l)?;
                return Ok(Flow::Exit(2));
            }
            send(out, Kind::HelloOk, id, &protocol::encode(&HelloOk { v: protocol::VERSION, pid: std::process::id() }), l)?;
        }
        Kind::OpenBegin => {
            *open = None;
            *incoming = None;
            let b: OpenBegin = protocol::decode(&frame.payload)?;
            if b.total > l.max_input {
                send_error(out, id, "resource_limit", &format!("{} bytes exceeds the {} byte input limit", b.total, l.max_input), l)?;
                *incoming = Some(Incoming::refused(id));
                return Ok(Flow::Continue);
            }
            if !is_sha256_hex(&b.sha256) {
                send_error(out, id, "invalid_argument", "sha256 must be 64 hexadecimal digits", l)?;
                *incoming = Some(Incoming::refused(id));
                return Ok(Flow::Continue);
            }
            let mut buf = Vec::new();
            // Reserve only after the declared size passed the limit, and without aborting on failure.
            if usize::try_from(b.total).ok().and_then(|n| buf.try_reserve_exact(n).ok()).is_none() {
                send_error(out, id, "resource_limit", "cannot reserve memory for the document", l)?;
                *incoming = Some(Incoming::refused(id));
                return Ok(Flow::Continue);
            }
            *incoming = Some(Incoming {
                id,
                total: b.total,
                expect_sha: b.sha256.to_ascii_lowercase(),
                hasher: Sha256::new(),
                buf,
                next_seq: 0,
                password: b.password,
                refused: false,
            });
        }
        Kind::Chunk => {
            let Some(inc) = incoming.as_mut().filter(|i| i.id == id) else {
                return Err(ProtocolError::Sequence("chunk without an open transfer".into()));
            };
            if inc.refused {
                return Ok(Flow::Continue);
            }
            let (seq, data) = split_chunk(&frame.payload)?;
            if seq != inc.next_seq {
                return Err(ProtocolError::Sequence(format!("chunk {seq} where {} was expected", inc.next_seq)));
            }
            if (inc.buf.len() as u64).saturating_add(data.len() as u64) > inc.total {
                *inc = Incoming::refused(id);
                send_error(out, id, "invalid_argument", "more bytes than the declared total", l)?;
                return Ok(Flow::Continue);
            }
            inc.hasher.update(data);
            inc.buf.extend_from_slice(data);
            inc.next_seq += 1;
        }
        Kind::OpenEnd => {
            let Some(inc) = incoming.take().filter(|i| i.id == id) else {
                return Err(ProtocolError::Sequence("OpenEnd without an open transfer".into()));
            };
            if inc.refused {
                return Ok(Flow::Continue);
            }
            if inc.buf.len() as u64 != inc.total {
                send_error(out, id, "invalid_argument", "fewer bytes than the declared total", l)?;
                return Ok(Flow::Continue);
            }
            if hex(&inc.hasher.finalize()) != inc.expect_sha {
                send_error(out, id, "invalid_argument", "the document does not match its declared SHA-256", l)?;
                return Ok(Flow::Continue);
            }
            let config = RenderConfig { password: inc.password.map(|p| Arc::from(p.as_str())), reject_oversize: true, ..RenderConfig::default() };
            let renderer = PageRenderer::new(Arc::new(inc.buf), config);
            let pages = renderer.page_count();
            if pages == 0 {
                send_error(out, id, "invalid_argument", "the file is not a readable PDF, or it needs a password", l)?;
                return Ok(Flow::Continue);
            }
            let pages = u32::try_from(pages).unwrap_or(u32::MAX);
            *open = Some(Open { renderer, pages });
            send(out, Kind::Opened, id, &protocol::encode(&Opened { pages }), l)?;
        }
        Kind::Render => {
            let r: Render = protocol::decode(&frame.payload)?;
            let Some(doc) = open.as_mut() else {
                send_error(out, id, "invalid_argument", "no document is open", l)?;
                return Ok(Flow::Continue);
            };
            if r.page == 0 || r.page > doc.pages || !(1_000..=600_000).contains(&r.dpi_milli) {
                send_error(out, id, "invalid_argument", "page or DPI out of range", l)?;
                return Ok(Flow::Continue);
            }
            if let Some(protocol::Debug::CrashOnce) = fault {
                std::process::abort();
            }
            let scale = r.dpi_milli as f32 / 1000.0 / 72.0;
            let page = doc.renderer.render(RenderRequest {
                page: (r.page - 1) as usize,
                kind: RequestKind::Pixels,
                tile: None,
                scale,
                tag: 0,
                preview: false,
            });
            if cancel.load(Ordering::SeqCst) == id {
                send(out, Kind::Canceled, id, b"{}", l)?;
                return Ok(Flow::Continue);
            }
            if let Some(e) = page.error {
                send_error(out, id, "internal_failure", &e, l)?;
                return Ok(Flow::Continue);
            }
            let bytes: &[u8] = &page.rgba;
            if u64::from(page.width) * u64::from(page.height) > cfg.max_pixels || bytes.len() as u64 > l.max_result {
                send_error(out, id, "resource_limit", "the rendered page is larger than the result limit", l)?;
                return Ok(Flow::Continue);
            }
            send_render(out, id, page.width, page.height, page.millis, bytes, l, fault.take())?;
        }
        Kind::Cancel => {
            // The reader thread has recorded the target; a render in progress sees it on completion.
        }
        Kind::Shutdown => return Ok(Flow::Exit(0)),
        Kind::Debug => return debug(&frame, out, fault, l),
        Kind::HelloOk | Kind::Opened | Kind::RenderBegin | Kind::RenderEnd | Kind::Canceled | Kind::Error => {
            return Err(ProtocolError::Sequence(format!("{:?} is a reply, not a request", frame.kind)));
        }
    }
    Ok(Flow::Continue)
}

/// Send pixels as `RenderBegin`, numbered chunks and `RenderEnd`. A pending fault may corrupt the
/// answer on purpose (fault-injection builds only).
#[allow(clippy::too_many_arguments)]
fn send_render(
    out: &mut impl Write,
    id: u64,
    width: u32,
    height: u32,
    millis: u32,
    bytes: &[u8],
    l: &Limits,
    fault: Option<protocol::Debug>,
) -> Result<(), ProtocolError> {
    let mut sha = hex(&Sha256::digest(bytes));
    #[allow(unused_mut)]
    let mut reply_id = id;
    #[cfg(feature = "fault-injection")]
    match fault {
        Some(protocol::Debug::WrongId) => reply_id = id.wrapping_add(1),
        Some(protocol::Debug::WrongHash) => sha = "0".repeat(64),
        Some(protocol::Debug::HugeChunk) => {
            send(out, Kind::RenderBegin, id, &protocol::encode(&RenderBegin { width, height, total: bytes.len() as u64, sha256: sha, millis }), l)?;
            // A header that claims a 4 GiB chunk, written by hand because the writer refuses to.
            let mut header = [0u8; protocol::HEADER_LEN];
            header[..4].copy_from_slice(&protocol::MAGIC);
            header[4] = protocol::VERSION;
            header[5] = Kind::Chunk as u8;
            header[8..16].copy_from_slice(&id.to_le_bytes());
            header[16..20].copy_from_slice(&(u32::MAX - 15).to_le_bytes());
            out.write_all(&header)?;
            out.flush()?;
            return Ok(());
        }
        _ => {}
    }
    #[cfg(not(feature = "fault-injection"))]
    let _ = fault;
    send(
        out,
        Kind::RenderBegin,
        reply_id,
        &protocol::encode(&RenderBegin { width, height, total: bytes.len() as u64, sha256: std::mem::take(&mut sha), millis }),
        l,
    )?;
    let max_data = l.max_chunk as usize - 4;
    for (seq, piece) in bytes.chunks(max_data).enumerate() {
        send(out, Kind::Chunk, reply_id, &chunk_payload(u32::try_from(seq).unwrap_or(u32::MAX), piece), l)?;
    }
    send(out, Kind::RenderEnd, reply_id, b"{}", l)
}

#[cfg(feature = "fault-injection")]
fn debug(frame: &Frame, out: &mut impl Write, fault: &mut Option<protocol::Debug>, l: &Limits) -> Result<Flow, ProtocolError> {
    let d: protocol::Debug = protocol::decode(&frame.payload)?;
    match d {
        protocol::Debug::Hang => loop {
            thread::sleep(std::time::Duration::from_secs(1));
        },
        protocol::Debug::Crash => std::process::abort(),
        protocol::Debug::Alloc { mib } => {
            // Touch every page so the memory is really resident, then hold it forever.
            let held = vec![1u8; usize::try_from(mib.saturating_mul(1024 * 1024)).unwrap_or(usize::MAX)];
            let mut sum = 0u64;
            for i in (0..held.len()).step_by(4096) {
                sum = sum.wrapping_add(u64::from(held[i]));
            }
            eprintln!("pdfcraft-worker: holding {} MiB ({sum})", held.len() >> 20);
            loop {
                thread::sleep(std::time::Duration::from_secs(1));
            }
        }
        protocol::Debug::Spam => {
            out.write_all(b"this is not a frame, it is text on stdout\n")?;
            out.flush()?;
        }
        other => *fault = Some(other),
    }
    let _ = l;
    Ok(Flow::Continue)
}

#[cfg(not(feature = "fault-injection"))]
fn debug(frame: &Frame, out: &mut impl Write, _fault: &mut Option<protocol::Debug>, l: &Limits) -> Result<Flow, ProtocolError> {
    send_error(out, frame.id, "unsupported_feature", "this worker was built without fault injection", l)?;
    Ok(Flow::Continue)
}
