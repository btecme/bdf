//! `pdfcraft-worker`: the private document-processing process. Speaks the frame protocol on
//! stdin/stdout; see `protocol.rs`. Not meant to be run by hand.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::io::BufReader;
use std::process::ExitCode;

fn main() -> ExitCode {
    let input = BufReader::with_capacity(64 * 1024, std::io::stdin());
    let output = std::io::BufWriter::with_capacity(64 * 1024, std::io::stdout().lock());
    let code = pdfcraft_worker::worker::run(input, output, &pdfcraft_worker::worker::WorkerConfig::default());
    ExitCode::from(u8::try_from(code).unwrap_or(3))
}
