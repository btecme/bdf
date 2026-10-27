//! `grant` and `activity`: the person's own commands for the clients they delegate to (BUILD-008).
//!
//! These are the person acting for themselves, not an operation on a document, so they do not go through the service's
//! policy: they read and write the per-user grant file and read the activity log, both private to the user. Nothing here
//! takes a name or an identity from a request; the account is the one the command runs as.
//!
//! * `grant issue` makes a grant. **A new grant is read-only**; anything more is named with `--allow`, files are separate
//!   (`--read DIR`, `--write DIR`), and it expires (`--ttl`, eight hours by default). The client's secret is printed once and
//!   is not stored anywhere: only its hash is.
//! * `grant revoke` withdraws one. A running service notices the next time anything asks, because the grant file is checked
//!   for changes (`pdfcraft_app_service::GrantStore`), so no restart is needed.
//! * `activity` verifies and shows the record of what clients did. It holds no document content, secret or full path.

use std::path::{Path, PathBuf};

use pdfcraft_app_service::{Capability, ClientId, DocumentScope, FileAccess, GrantId, GrantRequest, GrantStore, OsPrincipal, Scope};

use super::{CliError, OptionSpec, flag, stdout_line, validate_options};

const STATE: OptionSpec = OptionSpec { name: "--state-dir", takes_value: true };
const ISSUE_OPTIONS: &[OptionSpec] = &[
    OptionSpec { name: "--client", takes_value: true },
    OptionSpec { name: "--allow", takes_value: true },
    OptionSpec { name: "--read", takes_value: true },
    OptionSpec { name: "--write", takes_value: true },
    OptionSpec { name: "--ttl", takes_value: true },
    STATE,
];
const STATE_ONLY: &[OptionSpec] = &[STATE];
const SHOW_OPTIONS: &[OptionSpec] = &[OptionSpec { name: "--last", takes_value: true }, STATE];

/// How many records `activity show` prints at most.
const MAX_SHOWN: usize = 1000;
const DEFAULT_SHOWN: usize = 20;

pub fn grant(args: &[String]) -> Result<(), CliError> {
    let rest = args.get(1..).unwrap_or(&[]);
    match args.first().map(String::as_str) {
        Some("issue") => issue(rest),
        Some("list") => list(rest),
        Some("revoke") => revoke(rest),
        Some("prune") => prune(rest),
        _ => Err("grant: use `issue`, `list`, `revoke` or `prune`; see pdfcraft-cli --help".into()),
    }
}

pub fn activity(args: &[String]) -> Result<(), CliError> {
    let rest = args.get(1..).unwrap_or(&[]);
    match args.first().map(String::as_str) {
        Some("verify") => verify(rest),
        Some("show") => show(rest),
        _ => Err("activity: use `verify` or `show`; see pdfcraft-cli --help".into()),
    }
}

fn issue(args: &[String]) -> Result<(), CliError> {
    validate_options("grant issue", args, ISSUE_OPTIONS)?;
    let name = flag(args, "--client").ok_or("grant issue: --client NAME is required")?;
    let client = ClientId::parse(name).map_err(|e| format!("grant issue: {e}"))?;
    let mut request = GrantRequest::read_only(client);
    // Read is always there; anything else is asked for by name.
    for list in values(args, "--allow") {
        for text in list.split(',').map(str::trim).filter(|t| !t.is_empty()) {
            let capability = Capability::parse(text).ok_or_else(|| {
                let known: Vec<&str> = Capability::GRANTABLE.iter().map(|c| c.name()).collect();
                format!("grant issue: `{text}` is not a capability a grant can hold (known: {})", known.join(", "))
            })?;
            if !request.scopes.iter().any(|s| s.capability == capability) {
                request.scopes.push(Scope { capability, documents: DocumentScope::All });
            }
        }
    }
    request.files = FileAccess {
        read_under: values(args, "--read").into_iter().map(PathBuf::from).collect(),
        write_under: values(args, "--write").into_iter().map(PathBuf::from).collect(),
    };
    if let Some(ttl) = flag(args, "--ttl") {
        request.ttl_seconds = parse_ttl(ttl)?;
    }
    let store = open_store(args)?;
    let issued = store.issue(request).map_err(|e| format!("grant issue: {e}"))?;
    let t = &issued.terms;
    eprintln!("pdfcraft-cli: the secret below is shown once and is not stored; give it to the client, and revoke the grant if it leaks");
    stdout_line(format_args!(
        "{}",
        serde_json::json!({
            "grant": t.id.as_str(),
            "client": t.client.as_str(),
            "allows": t.scopes.iter().map(|s| s.capability.name()).collect::<Vec<_>>(),
            "read_under": t.files.read_under,
            "write_under": t.files.write_under,
            "expires": t.expires_at,
            "secret": issued.secret(),
        })
    ))
}

fn list(args: &[String]) -> Result<(), CliError> {
    validate_options("grant list", args, STATE_ONLY)?;
    let store = open_store(args)?;
    let now = now();
    let grants: Vec<serde_json::Value> = store
        .list()
        .into_iter()
        .map(|(t, revoked)| {
            serde_json::json!({
                "grant": t.id.as_str(),
                "client": t.client.as_str(),
                "allows": t.scopes.iter().map(|s| s.capability.name()).collect::<Vec<_>>(),
                "read_under": t.files.read_under,
                "write_under": t.files.write_under,
                "issued": t.issued_at,
                "expires": t.expires_at,
                "revoked": revoked,
                "expired": t.expires_at <= now,
            })
        })
        .collect();
    stdout_line(format_args!("{}", serde_json::json!({ "grants": grants })))
}

fn revoke(args: &[String]) -> Result<(), CliError> {
    validate_options("grant revoke", args, STATE_ONLY)?;
    let id = super::positional(args).first().copied().ok_or("grant revoke: missing grant id")?;
    let id = GrantId::parse(id).ok_or("grant revoke: that is not a grant id (they look like g-0123456789abcdef)")?;
    let store = open_store(args)?;
    store.revoke(&id).map_err(|e| format!("grant revoke: {e}"))?;
    stdout_line(format_args!("{}", serde_json::json!({ "revoked": id.as_str() })))
}

fn prune(args: &[String]) -> Result<(), CliError> {
    validate_options("grant prune", args, STATE_ONLY)?;
    let removed = open_store(args)?.prune().map_err(|e| format!("grant prune: {e}"))?;
    stdout_line(format_args!("{}", serde_json::json!({ "removed": removed })))
}

fn verify(args: &[String]) -> Result<(), CliError> {
    validate_options("activity verify", args, STATE_ONLY)?;
    let dir = state_dir(args)?.join("activity");
    let report = pdfcraft_app_service::audit::verify(&dir).or_else(|e| {
        // No log yet is a log of nothing.
        if e.kind() == std::io::ErrorKind::NotFound { Ok(Default::default()) } else { Err(e) }
    });
    let report = report.map_err(|e| format!("activity verify: {e}"))?;
    stdout_line(format_args!(
        "{}",
        serde_json::json!({
            "records": report.records,
            "last_seq": report.last_seq,
            "segments": report.segments.len(),
            "intact": report.first_bad.is_none(),
            "first_bad": report.first_bad.map(|(segment, line)| serde_json::json!({ "segment": segment, "line": line })),
        })
    ))?;
    match report.first_bad {
        None => Ok(()),
        Some((segment, line)) => Err(format!("activity verify: the log is damaged at segment {segment}, line {line}").into()),
    }
}

fn show(args: &[String]) -> Result<(), CliError> {
    validate_options("activity show", args, SHOW_OPTIONS)?;
    let count = match flag(args, "--last") {
        None => DEFAULT_SHOWN,
        Some(text) => match text.parse::<usize>() {
            Ok(n) if (1..=MAX_SHOWN).contains(&n) => n,
            _ => return Err(format!("activity show: --last is a number from 1 to {MAX_SHOWN}").into()),
        },
    };
    let dir = state_dir(args)?.join("activity");
    for line in pdfcraft_app_service::audit::recent_lines(&dir, count).map_err(|e| format!("activity show: {e}"))? {
        stdout_line(format_args!("{line}"))?;
    }
    Ok(())
}

/// Every value given to a repeatable option.
fn values<'a>(args: &'a [String], name: &str) -> Vec<&'a str> {
    args.iter().enumerate().filter(|(_, a)| a.as_str() == name).filter_map(|(i, _)| args.get(i + 1)).map(String::as_str).collect()
}

fn state_dir(args: &[String]) -> Result<PathBuf, CliError> {
    match flag(args, "--state-dir") {
        Some(dir) if !dir.is_empty() => Ok(PathBuf::from(dir)),
        Some(_) => Err("--state-dir needs a folder".into()),
        None => pdfcraft_platform::state::access_dir()
            .ok_or_else(|| "this account has no per-user data folder in the environment; pass --state-dir FOLDER".into()),
    }
}

fn open_store(args: &[String]) -> Result<GrantStore, CliError> {
    let dir = state_dir(args)?;
    let owner = OsPrincipal::current().map_err(|e| format!("can't tell which account this is: {e}"))?;
    GrantStore::open(&dir.join("grants.json"), owner).map_err(|e| CliError::from(format!("{} ({})", e, display_name(&dir))))
}

/// A folder for an error message: its last component, not the whole path.
fn display_name(dir: &Path) -> String {
    dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok().and_then(|d| i64::try_from(d.as_secs()).ok()).unwrap_or(0)
}

/// `90s`, `45m`, `8h`, `2d`, or plain seconds; the bounds are the store's.
fn parse_ttl(text: &str) -> Result<i64, CliError> {
    let bad = || CliError::from(format!("grant issue: --ttl `{text}` is not a time (for example 90m, 8h, 2d)"));
    let (digits, unit) = match text.char_indices().last() {
        Some((i, c)) if c.is_ascii_alphabetic() => (text.get(..i).ok_or_else(bad)?, c),
        Some(_) => (text, 's'),
        None => return Err(bad()),
    };
    // Digits only: a sign is not a time, whatever the bounds downstream would say about it.
    let n: i64 = if digits.chars().all(|c| c.is_ascii_digit()) { digits.parse().map_err(|_| bad())? } else { return Err(bad()) };
    let factor = match unit {
        's' => 1,
        'm' => 60,
        'h' => 3600,
        'd' => 86_400,
        _ => return Err(bad()),
    };
    n.checked_mul(factor).ok_or_else(bad)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_are_read_in_seconds_minutes_hours_and_days_and_nothing_else() {
        let ok = |t: &str| parse_ttl(t).expect(t);
        assert_eq!((ok("90"), ok("90s"), ok("45m"), ok("8h"), ok("2d")), (90, 90, 2700, 28_800, 172_800));
        for bad in ["", "h", "8x", "-1h", "1.5h", "8 h", "ten", "99999999999999999999d", &format!("{}d", i64::MAX)] {
            assert!(parse_ttl(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn repeated_options_give_every_value() {
        let args: Vec<String> = ["--read", "a", "--client", "c", "--read", "b"].iter().map(|s| (*s).to_owned()).collect();
        assert_eq!(values(&args, "--read"), ["a", "b"]);
        assert!(values(&args, "--write").is_empty());
    }
}
