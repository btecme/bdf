//! The worker wire protocol: finite, versioned frames over the child's stdin and stdout.
//!
//! A frame is a fixed 20-byte header followed by a payload:
//!
//! ```text
//! magic "BDFW" | version u8 | kind u8 | reserved u16 (0) | request id u64 LE | payload length u32 LE
//! ```
//!
//! Every limit is checked on the header, **before** a single payload byte is allocated, so a
//! hostile or confused peer cannot make either side reserve memory by lying about a length. Control
//! messages are small JSON documents; document bytes and rendered pixels travel as raw chunks, each
//! numbered, with the total length and SHA-256 declared up front and checked at the end. stdout
//! carries frames and nothing else; diagnostics go to stderr, where the parent reads a bounded
//! amount.

use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};

pub const MAGIC: [u8; 4] = *b"BDFW";
pub const VERSION: u8 = 1;
pub const HEADER_LEN: usize = 20;

/// Size limits, agreed by both sides (spec section 7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Largest payload of a control frame (JSON).
    pub max_control: u32,
    /// Largest payload of a chunk frame, including its 4-byte sequence number.
    pub max_chunk: u32,
    /// Largest document the worker accepts (250 MiB).
    pub max_input: u64,
    /// Largest rendered pixel buffer the worker will send.
    pub max_result: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_control: 64 * 1024, max_chunk: 256 * 1024, max_input: 250 * 1024 * 1024, max_result: 256 * 1024 * 1024 }
    }
}

/// What a frame is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    Hello = 1,
    HelloOk = 2,
    OpenBegin = 3,
    Chunk = 4,
    OpenEnd = 5,
    Opened = 6,
    Render = 7,
    RenderBegin = 8,
    RenderEnd = 9,
    Cancel = 10,
    Canceled = 11,
    Error = 12,
    Shutdown = 13,
    /// Fault injection; only the `fault-injection` build of the worker acts on it.
    Debug = 14,
}

impl Kind {
    fn from_u8(b: u8) -> Option<Self> {
        Some(match b {
            1 => Self::Hello,
            2 => Self::HelloOk,
            3 => Self::OpenBegin,
            4 => Self::Chunk,
            5 => Self::OpenEnd,
            6 => Self::Opened,
            7 => Self::Render,
            8 => Self::RenderBegin,
            9 => Self::RenderEnd,
            10 => Self::Cancel,
            11 => Self::Canceled,
            12 => Self::Error,
            13 => Self::Shutdown,
            14 => Self::Debug,
            _ => return None,
        })
    }
}

/// One decoded frame.
#[derive(Debug, PartialEq, Eq)]
pub struct Frame {
    pub kind: Kind,
    pub id: u64,
    pub payload: Vec<u8>,
}

/// Why a frame (or the stream) was rejected. Each variant is something a peer did, not an I/O
/// accident, except [`ProtocolError::Io`].
#[derive(Debug, PartialEq, Eq)]
pub enum ProtocolError {
    BadMagic,
    BadVersion(u8),
    UnknownKind(u8),
    ReservedBits,
    TooLarge {
        kind: Kind,
        len: u32,
        max: u32,
    },
    /// The stream ended in the middle of a frame.
    Truncated,
    Io(String),
    Json(String),
    Sequence(String),
    Hash,
    Limit(String),
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadMagic => write!(f, "frame does not start with the protocol magic"),
            Self::BadVersion(v) => write!(f, "protocol version {v} is not {VERSION}"),
            Self::UnknownKind(k) => write!(f, "unknown frame kind {k}"),
            Self::ReservedBits => write!(f, "reserved header bits are not zero"),
            Self::TooLarge { kind, len, max } => write!(f, "{kind:?} frame declares {len} bytes; the limit is {max}"),
            Self::Truncated => write!(f, "the stream ended inside a frame"),
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::Json(e) => write!(f, "bad message body: {e}"),
            Self::Sequence(e) => write!(f, "out-of-sequence data: {e}"),
            Self::Hash => write!(f, "SHA-256 of the transferred bytes does not match the declared value"),
            Self::Limit(e) => write!(f, "limit exceeded: {e}"),
        }
    }
}

impl std::error::Error for ProtocolError {}

impl From<io::Error> for ProtocolError {
    fn from(e: io::Error) -> Self {
        Self::Io(e.to_string())
    }
}

fn payload_limit(kind: Kind, l: &Limits) -> u32 {
    if kind == Kind::Chunk { l.max_chunk } else { l.max_control }
}

/// Write one frame. The payload must already respect the limits; a violation is a bug in the
/// caller and is reported rather than sent.
pub fn write_frame(w: &mut impl Write, kind: Kind, id: u64, payload: &[u8], limits: &Limits) -> Result<(), ProtocolError> {
    let len = u32::try_from(payload.len()).map_err(|_| ProtocolError::Limit("payload longer than 4 GiB".into()))?;
    let max = payload_limit(kind, limits);
    if len > max {
        return Err(ProtocolError::TooLarge { kind, len, max });
    }
    let mut header = [0u8; HEADER_LEN];
    header[..4].copy_from_slice(&MAGIC);
    header[4] = VERSION;
    header[5] = kind as u8;
    header[8..16].copy_from_slice(&id.to_le_bytes());
    header[16..20].copy_from_slice(&len.to_le_bytes());
    w.write_all(&header)?;
    w.write_all(payload)?;
    w.flush()?;
    Ok(())
}

/// Read one frame. `Ok(None)` is a clean end of stream between frames.
pub fn read_frame(r: &mut impl Read, limits: &Limits) -> Result<Option<Frame>, ProtocolError> {
    let mut header = [0u8; HEADER_LEN];
    let mut got = 0;
    while got < HEADER_LEN {
        match r.read(&mut header[got..]) {
            Ok(0) if got == 0 => return Ok(None),
            Ok(0) => return Err(ProtocolError::Truncated),
            Ok(n) => got += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.into()),
        }
    }
    if header[..4] != MAGIC {
        return Err(ProtocolError::BadMagic);
    }
    if header[4] != VERSION {
        return Err(ProtocolError::BadVersion(header[4]));
    }
    let kind = Kind::from_u8(header[5]).ok_or(ProtocolError::UnknownKind(header[5]))?;
    if header[6] != 0 || header[7] != 0 {
        return Err(ProtocolError::ReservedBits);
    }
    let id = u64::from_le_bytes(header[8..16].try_into().map_err(|_| ProtocolError::Truncated)?);
    let len = u32::from_le_bytes(header[16..20].try_into().map_err(|_| ProtocolError::Truncated)?);
    let max = payload_limit(kind, limits);
    if len > max {
        // Rejected on the header alone: nothing has been allocated for the payload.
        return Err(ProtocolError::TooLarge { kind, len, max });
    }
    let mut payload = vec![0u8; len as usize];
    r.read_exact(&mut payload).map_err(|e| if e.kind() == io::ErrorKind::UnexpectedEof { ProtocolError::Truncated } else { e.into() })?;
    Ok(Some(Frame { kind, id, payload }))
}

/// Encode a control message as a JSON payload.
pub fn encode<T: Serialize>(msg: &T) -> Vec<u8> {
    serde_json::to_vec(msg).unwrap_or_default()
}

/// Decode a control message.
pub fn decode<'a, T: Deserialize<'a>>(payload: &'a [u8]) -> Result<T, ProtocolError> {
    serde_json::from_slice(payload).map_err(|e| ProtocolError::Json(e.to_string()))
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct Hello {
    pub v: u8,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct HelloOk {
    pub v: u8,
    pub pid: u32,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct OpenBegin {
    pub total: u64,
    pub sha256: String,
    pub password: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct Opened {
    pub pages: u32,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct Render {
    /// 1-based.
    pub page: u32,
    /// Dots per inch times 1000, so a fractional DPI needs no float on the wire.
    pub dpi_milli: u32,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct RenderBegin {
    pub width: u32,
    pub height: u32,
    pub total: u64,
    pub sha256: String,
    pub millis: u32,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct Cancel {
    pub target: u64,
}

/// Error codes, after the spec's list (section 4).
#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct ErrorMsg {
    pub code: String,
    pub message: String,
}

/// Fault-injection requests (honoured only by a worker built with the `fault-injection` feature).
#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(tag = "do", rename_all = "snake_case")]
pub enum Debug {
    /// Never answer.
    Hang,
    /// Abort the process.
    Crash,
    /// Allocate and touch this many MiB, then hang.
    Alloc { mib: u64 },
    /// Write junk to stdout instead of a frame.
    Spam,
    /// Answer the next render with a wrong request id.
    WrongId,
    /// Answer the next render with a wrong hash.
    WrongHash,
    /// Answer the next render with a header that declares a 4 GiB chunk.
    HugeChunk,
    /// Crash on the next render only (a one-shot fault, so a retry succeeds).
    CrashOnce,
}

/// Lower-case hex of a SHA-256 digest.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Encode a chunk payload: 4-byte sequence number, then the bytes.
pub fn chunk_payload(seq: u32, data: &[u8]) -> Vec<u8> {
    let mut p = Vec::with_capacity(4 + data.len());
    p.extend_from_slice(&seq.to_le_bytes());
    p.extend_from_slice(data);
    p
}

/// Split a chunk payload into its sequence number and bytes.
pub fn split_chunk(payload: &[u8]) -> Result<(u32, &[u8]), ProtocolError> {
    let (head, data) = payload.split_at_checked(4).ok_or_else(|| ProtocolError::Sequence("chunk shorter than its sequence number".into()))?;
    Ok((u32::from_le_bytes(head.try_into().map_err(|_| ProtocolError::Truncated)?), data))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(kind: Kind, id: u64, payload: &[u8]) -> Frame {
        let mut buf = Vec::new();
        write_frame(&mut buf, kind, id, payload, &Limits::default()).unwrap();
        read_frame(&mut buf.as_slice(), &Limits::default()).unwrap().unwrap()
    }

    #[test]
    fn a_frame_survives_a_round_trip() {
        let f = roundtrip(Kind::Render, 42, b"{\"page\":1}");
        assert_eq!((f.kind, f.id, f.payload.as_slice()), (Kind::Render, 42, b"{\"page\":1}".as_slice()));
        assert_eq!(roundtrip(Kind::Shutdown, 0, b"").payload.len(), 0);
    }

    #[test]
    fn a_clean_end_between_frames_is_none_but_a_cut_frame_is_an_error() {
        assert_eq!(read_frame(&mut [].as_slice(), &Limits::default()).unwrap(), None);
        let mut buf = Vec::new();
        write_frame(&mut buf, Kind::Hello, 1, b"{}", &Limits::default()).unwrap();
        for cut in 1..buf.len() {
            assert_eq!(read_frame(&mut &buf[..cut], &Limits::default()), Err(ProtocolError::Truncated), "cut at {cut}");
        }
    }

    #[test]
    fn bad_headers_are_rejected_before_any_payload_is_read() {
        let l = Limits::default();
        let mut good = Vec::new();
        write_frame(&mut good, Kind::Hello, 1, b"{}", &l).unwrap();
        let mut bad = good.clone();
        bad[0] = b'X';
        assert_eq!(read_frame(&mut bad.as_slice(), &l), Err(ProtocolError::BadMagic));
        let mut bad = good.clone();
        bad[4] = 9;
        assert_eq!(read_frame(&mut bad.as_slice(), &l), Err(ProtocolError::BadVersion(9)));
        let mut bad = good.clone();
        bad[5] = 200;
        assert_eq!(read_frame(&mut bad.as_slice(), &l), Err(ProtocolError::UnknownKind(200)));
        let mut bad = good.clone();
        bad[6] = 1;
        assert_eq!(read_frame(&mut bad.as_slice(), &l), Err(ProtocolError::ReservedBits));
    }

    #[test]
    fn a_length_over_the_limit_is_rejected_without_reading_or_allocating() {
        let l = Limits::default();
        let mut header = Vec::new();
        write_frame(&mut header, Kind::Chunk, 1, b"", &l).unwrap();
        header[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        // The stream holds only the header: a reader that tried to take 4 GiB would hit Truncated or
        // exhaust memory; the right answer is TooLarge from the header alone.
        assert!(matches!(read_frame(&mut header.as_slice(), &l), Err(ProtocolError::TooLarge { kind: Kind::Chunk, len: u32::MAX, .. })));
        let mut ctl = Vec::new();
        write_frame(&mut ctl, Kind::Hello, 1, b"", &l).unwrap();
        ctl[16..20].copy_from_slice(&(l.max_control + 1).to_le_bytes());
        assert!(matches!(read_frame(&mut ctl.as_slice(), &l), Err(ProtocolError::TooLarge { .. })));
    }

    #[test]
    fn the_writer_refuses_what_the_reader_would_refuse() {
        let l = Limits::default();
        let big = vec![0u8; l.max_chunk as usize + 1];
        assert!(matches!(write_frame(&mut Vec::new(), Kind::Chunk, 1, &big, &l), Err(ProtocolError::TooLarge { .. })));
        let ok = vec![0u8; l.max_chunk as usize];
        assert!(write_frame(&mut Vec::new(), Kind::Chunk, 1, &ok, &l).is_ok());
    }

    #[test]
    fn chunk_payloads_carry_their_sequence_number() {
        let p = chunk_payload(7, b"abc");
        assert_eq!(split_chunk(&p).unwrap(), (7, b"abc".as_slice()));
        assert!(split_chunk(&[1, 2]).is_err());
    }

    #[test]
    fn messages_round_trip_as_json() {
        let m = OpenBegin { total: 5, sha256: "ab".into(), password: Some("p".into()) };
        assert_eq!(decode::<OpenBegin>(&encode(&m)).unwrap(), m);
        assert!(decode::<OpenBegin>(b"{\"total\":\"x\"}").is_err());
        assert_eq!(decode::<Debug>(&encode(&Debug::Alloc { mib: 3 })).unwrap(), Debug::Alloc { mib: 3 });
    }
}
