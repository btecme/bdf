//! `bdf-updater`: applies a verified update to the portable folder after BDF has closed. It is started by
//! BDF (from a copy of itself, so the one in the folder can be replaced) and runs without a window; what it
//! did is written to `BDFUpdate/last-result.json` for BDF to show on its next start.
//!
//! ```text
//! bdf-updater apply    --app-dir DIR --release FILE --zip FILE [--wait-secs N] [--restart EXE [FILE...]]
//! bdf-updater rollback --app-dir DIR [--wait-secs N] [--restart EXE [FILE...]]
//! bdf-updater recover  --app-dir DIR [--restart EXE [FILE...]]
//! ```
//!
//! It makes no network connection and has no network code.

#![cfg_attr(windows, windows_subsystem = "windows")]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::ExitCode::from(pdfcraft_updater::run(&args))
}
