//! `cargo xtask pins`: every action and reusable workflow that a workflow file uses must be pinned to a
//! full commit SHA (BUILD-003, finding F07).
//!
//! A branch or tag in a `uses:` line is a remote reference that can change without any change in this
//! repository, so what a release or a CI job runs would not be what was reviewed. A local path
//! (`./...`) is part of this repository and is allowed; a container image must be pinned by digest.
//!
//! The files are read with a YAML parser, so every spelling of the `uses` key (quoted, with spaces
//! before the colon, in a flow mapping, as a complex key, with the value on the next line or in a
//! block scalar) is found, and `uses:` text inside a `run` script or a description is not mistaken for
//! a reference. A value that is not a plain string (an alias, a mapping, a sequence), and a file that is
//! not valid YAML, are reported rather than skipped: what cannot be read cannot be shown to be pinned.
//!
//! The check reads only this repository's files. It cannot see inside a pinned reusable workflow, whose
//! own `uses:` lines are resolved by GitHub when the job runs: a pinned caller can still run floating
//! nested references. That residual is recorded in `docs/bdf-evidence/BUILD-003.md`, not hidden by
//! this check.

use std::path::Path;

use anyhow::{Context, Result, bail};
use yaml_rust2::parser::{Event, MarkedEventReceiver, Parser};
use yaml_rust2::scanner::Marker;

/// One `uses` reference found in a file, or a place where one could not be read.
#[derive(Debug, PartialEq, Eq)]
pub struct Use {
    pub file: String,
    /// 1-based line of the value (or of the problem).
    pub line: usize,
    pub target: String,
    /// Set when the file or the value could not be read as a reference; `target` is then empty.
    pub unreadable: Option<String>,
}

/// Where the reader is in the document: a mapping (and whether its next node is a key, and whether the
/// key just read was `uses`) or a sequence.
enum Frame {
    Map { expecting_key: bool, key_is_uses: bool },
    Seq,
}

struct Reader<'a> {
    file: &'a str,
    stack: Vec<Frame>,
    out: Vec<Use>,
}

impl Reader<'_> {
    /// A scalar, alias or the start of a collection, before it is pushed: it is a key or a value of
    /// the mapping that contains it.
    fn node(&mut self, event: &Event, mark: Marker) {
        let Some(Frame::Map { expecting_key, key_is_uses }) = self.stack.last_mut() else { return };
        if *expecting_key {
            *key_is_uses = matches!(event, Event::Scalar(text, ..) if text == "uses");
            *expecting_key = false;
            return;
        }
        *expecting_key = true;
        if !std::mem::take(key_is_uses) {
            return;
        }
        let line = mark.line();
        match event {
            Event::Scalar(text, ..) => self.out.push(Use { file: self.file.to_string(), line, target: text.trim().to_string(), unreadable: None }),
            Event::Alias(_) => self.unreadable(line, "`uses` is an alias; write the reference out so it can be checked"),
            _ => self.unreadable(line, "`uses` must be a plain string, not a mapping or a sequence"),
        }
    }

    fn unreadable(&mut self, line: usize, why: &str) {
        self.out.push(Use { file: self.file.to_string(), line, target: String::new(), unreadable: Some(why.to_string()) });
    }
}

impl MarkedEventReceiver for Reader<'_> {
    fn on_event(&mut self, ev: Event, mark: Marker) {
        match &ev {
            Event::Scalar(..) | Event::Alias(_) => self.node(&ev, mark),
            Event::MappingStart(..) => {
                self.node(&ev, mark);
                self.stack.push(Frame::Map { expecting_key: true, key_is_uses: false });
            }
            Event::SequenceStart(..) => {
                self.node(&ev, mark);
                self.stack.push(Frame::Seq);
            }
            Event::MappingEnd | Event::SequenceEnd => {
                self.stack.pop();
            }
            _ => {}
        }
    }
}

/// The `uses` references in a workflow or composite-action file (every document in it), and the places
/// that could not be read.
pub fn scan(file: &str, text: &str) -> Vec<Use> {
    let mut reader = Reader { file, stack: Vec::new(), out: Vec::new() };
    if let Err(e) = Parser::new_from_str(text).load(&mut reader, true) {
        let line = e.marker().line();
        reader.unreadable(line, &format!("not valid YAML ({}); a file that cannot be read cannot be shown to be pinned", e.info()));
    }
    reader.out
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

/// One message per unpinned or unreadable reference.
pub fn violations(uses: &[Use]) -> Vec<String> {
    uses.iter()
        .filter_map(|u| match &u.unreadable {
            Some(why) => Some(format!("{}:{}: {why}", u.file, u.line)),
            None => problem(&u.target).map(|why| format!("{}:{}: `{}`: {why}", u.file, u.line, u.target)),
        })
        .collect()
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

/// Every `uses` in `.github/workflows` and `.github/actions` under `root`.
pub fn repository_uses(root: &Path) -> Result<Vec<Use>> {
    let mut out = Vec::new();
    collect(&root.join(".github/workflows"), root, &mut out)?;
    collect(&root.join(".github/actions"), root, &mut out)?;
    Ok(out)
}

/// A scan that found nothing proves nothing: a moved `.github` directory or a failing walk would otherwise
/// read as "0 unpinned" and pass.
fn require_references(uses: &[Use]) -> Result<()> {
    if uses.is_empty() {
        bail!("no `uses` reference was found under .github/, so nothing was checked; the repository's workflows are expected to use actions");
    }
    Ok(())
}

pub fn run(_: &[String]) -> Result<()> {
    let root = crate::gates::root();
    let uses = repository_uses(&root)?;
    require_references(&uses)?;
    let bad = violations(&uses);
    println!("workflow pins: {} reference(s) checked, {} unpinned or unreadable", uses.iter().filter(|u| u.unreadable.is_none()).count(), bad.len());
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
            "steps:\n  - uses: actions/checkout@{SHA} # v7.0.1\n  - uses: \"actions/checkout@{SHA}\"\n  - uses: ./.github/actions/x\n  - uses: docker://alpine@sha256:{}\n    # uses: org/r@main\n  - run: 'echo uses: org/r@main'\n",
            "a".repeat(64)
        );
        assert!(bad(&ok).is_empty(), "{:?}", bad(&ok));
        assert_eq!(scan("w.yml", &ok).len(), 4, "only real uses: lines are read");
    }

    #[test]
    fn a_scan_that_found_no_reference_is_not_a_pass() {
        assert!(require_references(&[]).is_err());
        assert!(
            require_references(&scan(
                "w.yml",
                "steps:
  - uses: ./local
"
            ))
            .is_ok()
        );
    }

    #[test]
    fn a_non_hex_forty_character_ref_is_not_a_sha() {
        let fake = "z".repeat(40);
        assert_eq!(bad(&format!("steps:\n  - uses: org/r@{fake}\n")).len(), 1);
    }

    // ---- R2 (Codex review): the scanner reads YAML mappings, not one spelling of the key ----

    /// Every equivalent way to write a floating `uses` must be reported once, whatever the spelling.
    #[test]
    fn equivalent_yaml_spellings_of_a_floating_use_are_all_reported() {
        for (what, text) in [
            ("quoted key", "steps:\n  - \"uses\": actions/checkout@main\n"),
            ("single-quoted key", "steps:\n  - 'uses': actions/checkout@main\n"),
            ("space before the colon", "steps:\n  - uses : actions/checkout@main\n"),
            ("flow mapping", "steps:\n  - {uses: actions/checkout@main}\n"),
            ("flow mapping with other keys", "steps:\n  - { name: x, \"uses\": actions/checkout@main, with: {a: b} }\n"),
            ("value on the next line", "steps:\n  - uses:\n      actions/checkout@main\n"),
            ("folded scalar", "steps:\n  - uses: >-\n      actions/checkout@main\n"),
            ("literal scalar", "steps:\n  - uses: |\n      actions/checkout@main\n"),
            ("multi-line plain scalar", "steps:\n  - uses: actions/\n      checkout@main\n"),
            ("quoted value", "steps:\n  - uses: 'actions/checkout@main'\n"),
            ("complex key", "steps:\n  - ? uses\n    : actions/checkout@main\n"),
            ("escaped key", "steps:\n  - \"u\\x73es\": actions/checkout@main\n"),
            ("tagged value", "steps:\n  - uses: !!str actions/checkout@main\n"),
            ("after an apostrophe in a plain scalar", "steps:\n  - {name: it's, uses: actions/checkout@main}\n"),
            ("document marker and comment", "---\n# first\nsteps:\n  - uses: actions/checkout@main # floating\n"),
        ] {
            let found = bad(text);
            assert_eq!(found.len(), 1, "{what}: expected one violation in {text:?}, got {found:?}");
            assert!(found[0].starts_with("w.yml:") && found[0].contains("@main"), "{what}: {found:?}");
        }
    }

    #[test]
    fn a_value_the_scanner_cannot_read_literally_is_refused_not_skipped() {
        for (what, text) in [
            ("alias", "x: &a actions/checkout@main\nsteps:\n  - uses: *a\n"),
            ("a mapping as the value", "steps:\n  - uses: {name: actions/checkout@main}\n"),
            ("a sequence as the value", "steps:\n  - uses: [actions/checkout@main]\n"),
            ("a number", "steps:\n  - uses: 3\n"),
            ("an empty value", "steps:\n  - uses:\n  - name: x\n"),
        ] {
            let found = bad(text);
            assert_eq!(found.len(), 1, "{what}: {found:?}");
        }
        // Text that is not YAML cannot be trusted to contain no floating reference.
        let broken = bad("steps:\n  - uses: actions/checkout@main\n  - [unclosed\n");
        assert!(broken.iter().any(|m| m.starts_with("w.yml:") && m.contains("not valid YAML")), "{broken:?}");
    }

    #[test]
    fn pinned_references_pass_in_every_spelling_and_shell_text_is_not_a_reference() {
        let ok = format!(
            concat!(
                "steps:\n",
                "  - \"uses\": actions/checkout@{sha} # quoted key\n",
                "  - uses : actions/checkout@{sha}\n",
                "  - {{uses: actions/checkout@{sha}}}\n",
                "  - uses:\n      actions/checkout@{sha}\n",
                "  - uses: >-\n      actions/checkout@{sha}\n",
                "  - name: shell\n",
                "    run: |\n",
                "      echo \"uses: actions/checkout@main\"\n",
                "      cat <<EOF\n",
                "      - uses: org/r@main\n",
                "      EOF\n",
                "  - name: a description\n",
                "    with:\n",
                "      note: 'uses: actions/checkout@main is only text here'\n",
                "  - uses: ./local/action\n",
            ),
            sha = SHA
        );
        let found = scan("w.yml", &ok);
        assert!(violations(&found).is_empty(), "{:?}", violations(&found));
        assert_eq!(found.len(), 6, "the five pinned spellings and the local action are the only references: {found:?}");
    }

    #[test]
    fn a_diagnostic_names_the_line_of_the_value() {
        let found = bad("steps:\n  - name: x\n    uses:\n      actions/checkout@main\n");
        assert!(found[0].starts_with("w.yml:4:"), "{found:?}");
        let flow = bad("a: 1\nb: {uses: org/r@main}\n");
        assert!(flow[0].starts_with("w.yml:2:"), "{flow:?}");
    }

    /// Codex's fixture: twelve pinned references plus one floating quoted-key reference must not read
    /// as 12 references and 0 violations.
    #[test]
    fn a_floating_quoted_key_in_a_whole_repository_fixture_is_found() {
        let dir = std::env::temp_dir().join(format!("bdf-pins-{}-{}", std::process::id(), line!()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".github/workflows")).unwrap();
        let mut pinned = String::from("jobs:\n  a:\n    steps:\n");
        for i in 0..12 {
            pinned.push_str(&format!("      - uses: org/r{i}@{SHA}\n"));
        }
        std::fs::write(dir.join(".github/workflows/pinned.yml"), pinned).unwrap();
        std::fs::write(dir.join(".github/workflows/floating.yml"), "jobs:\n  a:\n    steps:\n      - \"uses\": org/r@main\n").unwrap();
        let uses = repository_uses(&dir).unwrap();
        let v = violations(&uses);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(uses.len(), 13, "{uses:?}");
        assert_eq!(v.len(), 1, "{v:?}");
        assert!(v[0].contains("floating.yml:4:"), "{v:?}");
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
