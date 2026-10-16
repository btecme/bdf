//! `cargo xtask pins`: every action and reusable workflow that a workflow file uses must be pinned to a
//! full commit SHA (BUILD-003, finding F07).
//!
//! A branch or tag in a `uses:` line is a remote reference that can change without any change in this
//! repository, so what a release or a CI job runs would not be what was reviewed. A local path
//! (`./...`) is part of this repository and is allowed; a container image must be pinned by digest.
//!
//! The check reads only this repository's files. It cannot see inside a pinned reusable workflow, whose
//! own `uses:` lines are resolved by GitHub when the job runs: a pinned caller can still run floating
//! nested references. That residual is recorded in `docs/bdf-evidence/BUILD-003.md`, not hidden by
//! this check.

use std::path::Path;

use anyhow::{Context, Result, bail};

/// One `uses:` reference found in a file.
#[derive(Debug, PartialEq, Eq)]
pub struct Use {
    pub file: String,
    pub line: usize,
    pub target: String,
}

/// The `uses:` references in a workflow or composite-action file. Comment lines are skipped, and a
/// trailing `# comment` and any quotes are removed from the value.
pub fn scan(file: &str, text: &str) -> Vec<Use> {
    let mut out = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim_start();
        let line = line.strip_prefix("- ").map_or(line, str::trim_start);
        let Some(rest) = line.strip_prefix("uses:") else { continue };
        let value = rest.split(" #").next().unwrap_or("").trim().trim_matches(|c| c == '"' || c == '\'');
        out.push(Use { file: file.to_string(), line: i + 1, target: value.to_string() });
    }
    out
}

fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Why a reference is not pinned, or `None` when it is.
fn problem(target: &str) -> Option<String> {
    if target.starts_with("./") {
        return None;
    }
    if let Some(image) = target.strip_prefix("docker://") {
        return match image.split_once("@sha256:") {
            Some((_, digest)) if is_hex(digest, 64) => None,
            _ => Some("a container image must be pinned by @sha256: digest".into()),
        };
    }
    match target.split_once('@') {
        None => Some("no @ref: it would run whatever the default branch is".into()),
        Some((_, r)) if is_hex(r, 40) => None,
        Some((_, r)) => Some(format!("`@{r}` is a branch, tag or abbreviated hash; pin a full 40-character commit SHA")),
    }
}

/// One message per unpinned reference.
pub fn violations(uses: &[Use]) -> Vec<String> {
    uses.iter().filter_map(|u| problem(&u.target).map(|why| format!("{}:{}: `{}`: {why}", u.file, u.line, u.target))).collect()
}

fn collect(dir: &Path, root: &Path, out: &mut Vec<Use>) -> Result<()> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Ok(()) };
    let mut paths: Vec<_> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        if p.is_dir() {
            collect(&p, root, out)?;
        } else if matches!(p.extension().and_then(|e| e.to_str()), Some("yml" | "yaml")) {
            let text = std::fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))?;
            let rel = p.strip_prefix(root).unwrap_or(&p).to_string_lossy().replace('\\', "/");
            out.extend(scan(&rel, &text));
        }
    }
    Ok(())
}

/// Every `uses:` in `.github/workflows` and `.github/actions` under `root`.
pub fn repository_uses(root: &Path) -> Result<Vec<Use>> {
    let mut out = Vec::new();
    collect(&root.join(".github/workflows"), root, &mut out)?;
    collect(&root.join(".github/actions"), root, &mut out)?;
    Ok(out)
}

pub fn run(_: &[String]) -> Result<()> {
    let root = crate::gates::root();
    let uses = repository_uses(&root)?;
    let bad = violations(&uses);
    println!("workflow pins: {} reference(s) checked, {} unpinned", uses.len(), bad.len());
    for b in &bad {
        println!("  {b}");
    }
    if !bad.is_empty() {
        bail!("{} workflow reference(s) are not pinned to a commit SHA", bad.len());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "3d3c42e5aac5ba805825da76410c181273ba90b1";

    fn bad(text: &str) -> Vec<String> {
        violations(&scan("w.yml", text))
    }

    #[test]
    fn floating_references_are_reported_with_their_line() {
        for (text, needle) in [
            ("jobs:\n  a:\n    uses: org/repo/.github/workflows/x.yml@main\n", "@main"),
            ("steps:\n  - uses: actions/checkout@v4\n", "@v4"),
            ("steps:\n  - uses: EmbarkStudios/cargo-deny-action@v2\n", "@v2"),
            ("steps:\n  - uses: actions/checkout@3d3c42e\n", "abbreviated"),
            ("steps:\n  - uses: actions/checkout\n", "no @ref"),
            ("steps:\n  - uses: docker://alpine:3.20\n", "digest"),
        ] {
            let found = bad(text);
            assert_eq!(found.len(), 1, "{text}: {found:?}");
            assert!(found[0].contains(needle) && found[0].starts_with("w.yml:"), "{found:?}");
        }
        assert!(bad("a:\n  b:\n    uses: org/r@main\n")[0].starts_with("w.yml:3:"));
    }

    #[test]
    fn pinned_local_and_commented_references_pass() {
        let ok = format!(
            "steps:\n  - uses: actions/checkout@{SHA} # v7.0.1\n  - uses: \"actions/checkout@{SHA}\"\n  - uses: ./.github/actions/x\n  - uses: docker://alpine@sha256:{}\n    # uses: org/r@main\n  - run: echo uses: org/r@main\n",
            "a".repeat(64)
        );
        assert!(bad(&ok).is_empty(), "{:?}", bad(&ok));
        assert_eq!(scan("w.yml", &ok).len(), 4, "only real uses: lines are read");
    }

    #[test]
    fn a_non_hex_forty_character_ref_is_not_a_sha() {
        let fake = "z".repeat(40);
        assert_eq!(bad(&format!("steps:\n  - uses: org/r@{fake}\n")).len(), 1);
    }

    /// The repository's own workflows: this is the gate. A new floating reference fails the xtask tests.
    #[test]
    fn this_repositorys_workflows_are_pinned() {
        let uses = repository_uses(&crate::gates::root()).unwrap();
        assert!(uses.len() > 10, "the scan found only {} references; is the path right?", uses.len());
        let v = violations(&uses);
        assert!(v.is_empty(), "unpinned workflow references:\n{}", v.join("\n"));
    }
}
