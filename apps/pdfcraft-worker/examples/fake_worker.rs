//! A scripted misbehaving worker, for tests of the supervisor.
//!
//! It speaks enough of the protocol to complete the handshake and accept a document, then does what
//! its first argument says instead of rendering. The real worker cannot be made to send these
//! replies, and the supervisor must survive them all:
//!
//! * `overflow`: answers a render with dimensions whose pixel count overflows `u64`, zero bytes of
//!   pixels and the SHA-256 of nothing;
//! * `stall-after-hello`: completes the handshake and then never reads another byte, so the
//!   parent's upload blocks on a full pipe;
//! * `silent-render`: accepts the document and then never answers a render;
//! * `begin-then-stall`: starts a valid render reply and then stops sending;
//! * `exit-after-hello`: completes the handshake and exits.

use std::io::{BufReader, Write};
use std::time::Duration;

use pdfcraft_worker::protocol::{self, Frame, HelloOk, Kind, Limits, Opened, Render, RenderBegin, hex, read_frame, write_frame};
use sha2::{Digest, Sha256};

fn send(out: &mut impl Write, kind: Kind, id: u64, payload: &[u8]) {
    let _ = write_frame(out, kind, id, payload, &Limits::default());
}

fn stall() -> ! {
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let mut input = BufReader::new(std::io::stdin());
    let mut out = std::io::stdout().lock();
    let limits = Limits::default();
    while let Ok(Some(Frame { kind, id, payload })) = read_frame(&mut input, &limits) {
        match kind {
            Kind::Hello => {
                send(&mut out, Kind::HelloOk, id, &protocol::encode(&HelloOk { v: protocol::VERSION, pid: std::process::id() }));
                match mode.as_str() {
                    "stall-after-hello" => stall(),
                    "exit-after-hello" => std::process::exit(0),
                    _ => {}
                }
            }
            Kind::OpenEnd => send(&mut out, Kind::Opened, id, &protocol::encode(&Opened { pages: 1 })),
            Kind::Render => {
                let _: Result<Render, _> = protocol::decode(&payload);
                match mode.as_str() {
                    "overflow" => {
                        let begin =
                            RenderBegin { width: 2_147_483_648, height: 2_147_483_648, total: 0, sha256: hex(&Sha256::digest(b"")), millis: 0 };
                        send(&mut out, Kind::RenderBegin, id, &protocol::encode(&begin));
                        send(&mut out, Kind::RenderEnd, id, b"{}");
                    }
                    "begin-then-stall" => {
                        let begin = RenderBegin { width: 100, height: 100, total: 40_000, sha256: "0".repeat(64), millis: 0 };
                        send(&mut out, Kind::RenderBegin, id, &protocol::encode(&begin));
                        stall();
                    }
                    "silent-render" => stall(),
                    _ => {}
                }
            }
            Kind::Shutdown => return,
            _ => {}
        }
    }
}
