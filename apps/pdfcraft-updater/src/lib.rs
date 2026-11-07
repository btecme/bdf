//! The updater's work, as a library so it can be tested without starting a process.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use pdfcraft_update::apply::{self, Options, Recovery};
use pdfcraft_update::{SignedManifest, Trust, UpdateError, layout};

/// Exit codes.
pub const OK: u8 = 0;
pub const FAILED: u8 = 1;
pub const USAGE: u8 = 2;
/// BDF was still running when the wait ran out; nothing was changed.
pub const POSTPONED: u8 = 3;

const DEFAULT_WAIT_SECS: u64 = 600;
/// A restart never reopens more than this many files.
const MAX_REOPEN: usize = 20;

struct Args {
    command: String,
    app_dir: PathBuf,
    release: Option<PathBuf>,
    zip: Option<PathBuf>,
    wait: Duration,
    restart: Option<PathBuf>,
    reopen: Vec<PathBuf>,
}

fn parse(args: &[String]) -> Result<Args, String> {
    let mut it = args.iter();
    let command = it.next().ok_or("no command (apply, rollback or recover)")?.clone();
    if !matches!(command.as_str(), "apply" | "rollback" | "recover") {
        return Err(format!("unknown command {command}"));
    }
    let (mut app_dir, mut release, mut zip, mut restart) = (None, None, None, None);
    let mut wait = Duration::from_secs(DEFAULT_WAIT_SECS);
    let mut reopen = Vec::new();
    while let Some(a) = it.next() {
        let mut value = |name: &str| it.next().cloned().ok_or(format!("{name} needs a value"));
        match a.as_str() {
            "--app-dir" => app_dir = Some(PathBuf::from(value("--app-dir")?)),
            "--release" => release = Some(PathBuf::from(value("--release")?)),
            "--zip" => zip = Some(PathBuf::from(value("--zip")?)),
            "--wait-secs" => wait = Duration::from_secs(value("--wait-secs")?.parse::<u64>().map_err(|_| "--wait-secs needs a number")?.min(3600)),
            "--restart" => {
                restart = Some(PathBuf::from(value("--restart")?));
                // Everything after the program is a file to reopen.
                reopen = it.by_ref().take(MAX_REOPEN).map(PathBuf::from).collect();
            }
            other => return Err(format!("unknown option {other}")),
        }
    }
    let app_dir = app_dir.filter(|d| d.is_absolute() && d.is_dir()).ok_or("--app-dir must be an existing absolute folder")?;
    if command == "apply" && (release.is_none() || zip.is_none()) {
        return Err("apply needs --release and --zip".into());
    }
    Ok(Args { command, app_dir, release, zip, wait, restart, reopen })
}

/// Run one command line (without the program name) and return the exit code.
pub fn run(args: &[String]) -> u8 {
    run_with(args, &Trust::build())
}

/// [`run`] with an explicit trust root (what the tests use; the program always uses the one it was built with).
pub fn run_with(args: &[String], trust: &Trust) -> u8 {
    let args = match parse(args) {
        Ok(a) => a,
        Err(e) => {
            // No window and often no console: the only place left is the log beside the app, if the folder is known.
            log_to_fallback(&format!("usage error: {e}"));
            return USAGE;
        }
    };
    let log = |line: &str| log_line(&args.app_dir, line);
    log(&format!("{} started", args.command));
    let opts = Options { wait: args.wait, cancel: None };
    let installed = layout::detect(&args.app_dir).map(|i| i.version.to_string()).unwrap_or_default();
    let (code, restart_wanted) = match args.command.as_str() {
        "apply" => apply_command(&args, trust, &opts, &installed),
        "rollback" => match apply::rollback(&args.app_dir, &opts) {
            Ok(a) => {
                apply::write_result(&args.app_dir, "updated", &a.from.to_string(), &a.to.to_string(), &format!("BDF went back to version {}.", a.to));
                (OK, true)
            }
            Err(e) => failure(&args.app_dir, &installed, "", &e),
        },
        _ => match apply::recover(&args.app_dir) {
            Ok(Recovery::Busy) => (POSTPONED, false),
            Ok(_) => (OK, true),
            Err(e) => failure(&args.app_dir, &installed, "", &e),
        },
    };
    log(&format!("{} finished with code {code}", args.command));
    if restart_wanted {
        restart(&args);
    }
    code
}

fn apply_command(args: &Args, trust: &Trust, opts: &Options, installed: &str) -> (u8, bool) {
    let (Some(release), Some(zip)) = (&args.release, &args.zip) else { return (USAGE, false) };
    let manifest = match std::fs::read(release)
        .map_err(|e| UpdateError::Io { what: "couldn't read the release information".into(), source: e })
        .and_then(|b| SignedManifest::from_envelope(&b, trust))
    {
        Ok(m) => m,
        Err(e) => return failure(&args.app_dir, installed, "", &e),
    };
    let to = manifest.version.to_string();
    match apply::apply(&args.app_dir, zip, &manifest, trust, opts) {
        Ok(a) => {
            apply::write_result(&args.app_dir, "updated", &a.from.to_string(), &a.to.to_string(), &format!("BDF was updated to version {}.", a.to));
            let _ = std::fs::remove_dir_all(layout::downloads_dir(&args.app_dir));
            (OK, true)
        }
        Err(e) => failure(&args.app_dir, installed, &to, &e),
    }
}

/// Record a failed attempt. BDF is started again after any failure that left it free, so the person
/// is never left without their program; when something still runs from the folder it is simply left running.
fn failure(app_dir: &Path, from: &str, to: &str, e: &UpdateError) -> (u8, bool) {
    match e {
        UpdateError::InUse(m) => {
            apply::write_result(app_dir, "postponed", from, to, m);
            (POSTPONED, false)
        }
        UpdateError::RolledBack(_) => {
            apply::write_result(app_dir, "rolled-back", from, to, &e.to_string());
            (FAILED, true)
        }
        _ => {
            apply::write_result(app_dir, "failed", from, to, &e.to_string());
            (FAILED, true)
        }
    }
}

/// Start BDF again, but only the program in the folder it was updated in, with the files to reopen.
fn restart(args: &Args) {
    let Some(exe) = &args.restart else { return };
    let expected = args.app_dir.join(if cfg!(windows) { "bdf.exe" } else { "bdf" });
    if !same_file(exe, &expected) || !expected.is_file() {
        log_line(&args.app_dir, "not restarting: the program to start is not this folder's launcher");
        return;
    }
    let mut command = std::process::Command::new(&expected);
    command
        .args(args.reopen.iter().filter(|p| p.is_absolute() && p.is_file()))
        .current_dir(&args.app_dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0000_0008 | 0x0000_0200);
    }
    match command.spawn() {
        Ok(_) => log_line(&args.app_dir, "BDF started again"),
        Err(e) => log_line(&args.app_dir, &format!("couldn't start BDF again: {e}")),
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn log_line(app_dir: &Path, line: &str) {
    let dir = layout::update_dir(app_dir);
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join("updater.log");
    // Keep the log small: a run's few lines, replaced when it grows past 64 KiB.
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 64 * 1024) {
        let _ = std::fs::remove_file(&path);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{} {line}", pdfcraft_update::now_unix());
    }
}

fn log_to_fallback(line: &str) {
    if let Some(dir) = std::env::current_dir()
        .ok()
        .filter(|d| d.join("..").join(pdfcraft_update::package::PACKAGE_FILE).is_file())
        .and_then(|d| d.parent().map(Path::to_path_buf))
    {
        log_line(&dir, line);
    }
}
