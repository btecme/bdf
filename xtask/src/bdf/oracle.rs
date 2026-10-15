//! `cargo xtask bdf-oracle`: open every corpus case in the engine and in an independent reader
//! (MuPDF, as a separate process) and compare both with what the generator knows is in the file.
//!
//! Three kinds of evidence per case:
//!
//! * **semantic**: page count, page sizes, text tokens, form fields, outline, attachments, page
//!   labels and annotation counts, each checked against the generator's expectation (the author
//!   knows what it wrote, so this oracle shares nothing with either reader);
//! * **visual**: the engine's render of selected pages against MuPDF's at the same DPI, judged by
//!   the tolerance class fixed in `model.rs` before any comparison existed;
//! * **behaviour**: how each reader ended (opened, refused with a message, crashed, hung) and how
//!   long it took.
//!
//! The harness records facts and a coarse verdict; triage of every divergence is done by reading
//! the images and the matrix, not by this code.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::imgcmp;
use super::proc;
use crate::gates::{root, target_dir};

#[derive(Deserialize)]
struct InManifest {
    case: Vec<InCase>,
}

#[derive(Deserialize)]
struct InCase {
    id: String,
    family: String,
    class: String,
    outcome: String,
    tolerance: String,
    expect: InExpect,
}

#[derive(Deserialize, Default)]
struct InExpect {
    pages: Option<usize>,
    #[serde(default)]
    text: Vec<(usize, String)>,
    #[serde(default)]
    sizes: Vec<[f64; 2]>,
    annotations: Option<usize>,
    links: Option<usize>,
    #[serde(default)]
    fields: Vec<String>,
    #[serde(default)]
    outlines: Vec<String>,
    #[serde(default)]
    attachments: Vec<String>,
    #[serde(default)]
    page_labels: Vec<String>,
    password: Option<String>,
}

#[derive(Serialize, Default, Clone)]
struct Render {
    page: usize,
    ok: bool,
    error: Option<String>,
    width: u32,
    height: u32,
    ms: u64,
}

/// What one reader did with the file.
#[derive(Serialize, Default)]
struct Side {
    opened: bool,
    refusal: Option<String>,
    crashed_or_hung: bool,
    exit: Option<i32>,
    open_ms: u64,
    pages: Option<usize>,
    sizes: Vec<[f64; 2]>,
    labels: Vec<String>,
    fields: Vec<String>,
    outlines: Vec<String>,
    attachments: Vec<String>,
    annotations: Option<usize>,
    links: Option<usize>,
    text: BTreeMap<usize, String>,
    renders: Vec<Render>,
    warnings: usize,
}

#[derive(Serialize)]
struct Check {
    name: String,
    status: &'static str,
    detail: String,
}

#[derive(Serialize)]
struct Visual {
    page: usize,
    ours: (usize, usize),
    theirs: (usize, usize),
    size_match: bool,
    diff_frac: f64,
    mae: f64,
    ink_ratio: Option<f64>,
    max_block: f64,
    passes: bool,
}

#[derive(Serialize)]
struct CaseResult {
    id: String,
    family: String,
    class: String,
    expected_outcome: String,
    tolerance: String,
    engine: Side,
    oracle: Side,
    checks: Vec<Check>,
    visual: Vec<Visual>,
    verdict: String,
}

struct Tools {
    cli: PathBuf,
    mutool: PathBuf,
    corpus: PathBuf,
    dpi: u32,
    timeout: Duration,
}

const CAP: usize = 8 * 1024 * 1024;

/// Every JSON value in a stream of pretty-printed values.
fn json_stream(bytes: &[u8]) -> Vec<Value> {
    serde_json::Deserializer::from_slice(bytes).into_iter::<Value>().map_while(Result::ok).collect()
}

fn first_line(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).lines().find(|l| !l.trim().is_empty()).unwrap_or("").chars().take(160).collect()
}

/// An engine run: a script of steps over one file, in the corpus directory as `--root`.
fn engine_run(t: &Tools, steps: &Value) -> Result<proc::Output> {
    let script = t.corpus.join("_script.json");
    std::fs::write(&script, serde_json::to_vec(steps)?)?;
    let mut c = Command::new(&t.cli);
    c.arg("run").arg("--script").arg(&script).arg("--root").arg(&t.corpus);
    proc::run(&mut c, t.timeout, CAP)
}

fn open_step(file: &str, password: Option<&str>) -> Value {
    match password {
        Some(p) => json!({"tool": "doc_open", "args": {"path": file, "password": p}}),
        None => json!({"tool": "doc_open", "args": {"path": file}}),
    }
}

fn flatten_outline(v: &Value, out: &mut Vec<String>) {
    for n in v.as_array().into_iter().flatten() {
        if let Some(t) = n.get("title").and_then(Value::as_str) {
            out.push(t.to_string());
        }
        if let Some(k) = n.get("children") {
            flatten_outline(k, out);
        }
    }
}

fn strings(v: Option<&Value>, key: &str) -> Vec<String> {
    v.and_then(Value::as_array).into_iter().flatten().filter_map(|x| x.get(key).and_then(Value::as_str)).map(str::to_string).collect()
}

/// Which pages to render and extract for a case.
fn pick_pages(pages: usize, e: &InExpect) -> (Vec<usize>, Vec<usize>) {
    let mut render: BTreeSet<usize> = BTreeSet::new();
    let mut text: BTreeSet<usize> = BTreeSet::new();
    render.insert(1);
    text.insert(1);
    if pages <= 4 {
        render.extend(1..=pages);
        text.extend(1..=pages);
    } else {
        render.insert(pages);
    }
    for (p, _) in &e.text {
        if *p > 0 && *p <= pages {
            render.insert(*p);
            text.insert(*p);
        }
    }
    if e.text.iter().any(|(p, _)| *p == 0) {
        text.extend(1..=pages.min(60));
    }
    // Keep the work bounded: at most six renders.
    let render: Vec<usize> = render.into_iter().take(6).collect();
    (render, text.into_iter().collect())
}

fn engine_side(t: &Tools, case: &InCase, file: &str) -> Result<Side> {
    let mut s = Side::default();
    let pw = case.expect.password.as_deref();
    // Open and describe.
    let o = engine_run(t, &json!([open_step(file, pw), {"tool": "doc_info", "args": {"doc": 1}}]))?;
    s.open_ms = o.ms;
    s.exit = o.code;
    if o.timed_out || !matches!(o.code, Some(0 | 1)) {
        s.crashed_or_hung = true;
        s.refusal = Some(if o.timed_out { "timed out".to_string() } else { format!("exit {:?}: {}", o.code, first_line(&o.stderr)) });
        return Ok(s);
    }
    let vals = json_stream(&o.stdout);
    let Some(info) = vals.get(1) else {
        s.refusal = Some(first_line(&o.stderr));
        return Ok(s);
    };
    s.opened = true;
    s.pages = info.get("pages").and_then(Value::as_array).map(Vec::len);
    for p in info.get("pages").and_then(Value::as_array).into_iter().flatten() {
        s.sizes.push([p.get("width").and_then(Value::as_f64).unwrap_or(0.0), p.get("height").and_then(Value::as_f64).unwrap_or(0.0)]);
        s.labels.push(p.get("label").and_then(Value::as_str).unwrap_or("").to_string());
    }
    s.fields = strings(info.get("fields"), "name");
    s.attachments = strings(info.get("attachments"), "name");
    s.annotations = info.get("annotations").and_then(Value::as_array).map(Vec::len);
    s.links = info.get("links").and_then(Value::as_array).map(Vec::len);
    if let Some(o) = info.get("outline") {
        flatten_outline(o, &mut s.outlines);
    }
    let pages = s.pages.unwrap_or(0);
    let (render_pages, text_pages) = pick_pages(pages, &case.expect);
    // Text, all wanted pages in one run.
    let o = engine_run(t, &json!([open_step(file, pw), {"tool": "text_extract", "args": {"doc": 1, "pages": text_pages}}]))?;
    if o.timed_out || !matches!(o.code, Some(0 | 1)) {
        s.crashed_or_hung = true;
    }
    for p in json_stream(&o.stdout).get(1).and_then(|v| v.get("pages")).and_then(Value::as_array).into_iter().flatten() {
        if let (Some(n), Some(txt)) = (p.get("page").and_then(Value::as_u64), p.get("text").and_then(Value::as_str)) {
            s.text.insert(n as usize, txt.to_string());
        }
    }
    // One render per run, so a failure on one page does not hide the others.
    let dir = t.corpus.join("_render");
    std::fs::create_dir_all(&dir)?;
    for p in render_pages {
        let rel = format!("_render/engine-{}-{p}.png", case.id);
        let _ = std::fs::remove_file(t.corpus.join(&rel));
        let o = engine_run(t, &json!([open_step(file, pw), {"tool": "page_render", "args": {"doc": 1, "page": p, "dpi": t.dpi}, "out": rel}]))?;
        let mut r = Render { page: p, ms: o.ms, ..Render::default() };
        if o.timed_out || !matches!(o.code, Some(0 | 1)) {
            s.crashed_or_hung = true;
            r.error = Some(if o.timed_out { "timed out".to_string() } else { format!("exit {:?}", o.code) });
        } else if o.code == Some(1) {
            r.error = Some(first_line(&o.stderr));
        } else if let Some(v) = json_stream(&o.stdout).get(1) {
            r.ok = t.corpus.join(&rel).is_file();
            r.width = v.get("width").and_then(Value::as_u64).unwrap_or(0) as u32;
            r.height = v.get("height").and_then(Value::as_u64).unwrap_or(0) as u32;
        }
        s.renders.push(r);
    }
    Ok(s)
}

fn mutool(t: &Tools, args: &[String]) -> Result<proc::Output> {
    let mut c = Command::new(&t.mutool);
    c.args(args);
    proc::run(&mut c, t.timeout, CAP)
}

fn oracle_side(t: &Tools, case: &InCase, file: &Path, engine_pages: Option<usize>) -> Result<Side> {
    let mut s = Side::default();
    let f = file.to_string_lossy().to_string();
    let mut base: Vec<String> = Vec::new();
    if let Some(p) = &case.expect.password {
        base.extend(["-p".to_string(), p.clone()]);
    }
    let mut info = vec!["info".to_string()];
    info.extend(base.clone());
    info.push(f.clone());
    let o = mutool(t, &info)?;
    s.open_ms = o.ms;
    s.exit = o.code;
    let err = String::from_utf8_lossy(&o.stderr).to_string();
    s.warnings = err.lines().filter(|l| l.contains("warning") || l.contains("error")).count();
    if o.timed_out || !matches!(o.code, Some(0 | 1)) {
        s.crashed_or_hung = true;
        s.refusal = Some(if o.timed_out { "timed out".to_string() } else { format!("exit {:?}", o.code) });
        return Ok(s);
    }
    let out = String::from_utf8_lossy(&o.stdout).to_string();
    s.pages = out.lines().find_map(|l| l.strip_prefix("Pages: ")).and_then(|n| n.trim().parse().ok());
    if o.code != Some(0) || s.pages.unwrap_or(0) == 0 {
        s.refusal = Some(first_line(&o.stderr));
        return Ok(s);
    }
    s.opened = true;
    let pages = s.pages.or(engine_pages).unwrap_or(1);
    let (render_pages, text_pages) = pick_pages(pages, &case.expect);
    for p in text_pages {
        let mut a = vec!["draw".to_string(), "-F".into(), "txt".into(), "-o".into(), "-".into()];
        a.extend(base.clone());
        a.extend([f.clone(), p.to_string()]);
        let o = mutool(t, &a)?;
        if o.timed_out {
            s.crashed_or_hung = true;
        }
        s.text.insert(p, String::from_utf8_lossy(&o.stdout).to_string());
    }
    let dir = t.corpus.join("_render");
    for p in render_pages {
        let png = dir.join(format!("oracle-{}-{p}.png", case.id));
        let _ = std::fs::remove_file(&png);
        let mut a = vec!["draw".to_string(), "-r".into(), t.dpi.to_string(), "-o".into(), png.to_string_lossy().to_string()];
        a.extend(base.clone());
        a.extend([f.clone(), p.to_string()]);
        let o = mutool(t, &a)?;
        let mut r = Render { page: p, ms: o.ms, ..Render::default() };
        if o.timed_out {
            s.crashed_or_hung = true;
            r.error = Some("timed out".to_string());
        } else if o.code != Some(0) || !png.is_file() {
            r.error = Some(first_line(&o.stderr));
        } else {
            r.ok = true;
        }
        s.renders.push(r);
    }
    Ok(s)
}

fn check(out: &mut Vec<Check>, name: impl Into<String>, status: &'static str, detail: impl Into<String>) {
    out.push(Check { name: name.into(), status, detail: detail.into() });
}

fn compare_lists(out: &mut Vec<Check>, name: &str, expected: &[String], got: &[String], ordered: bool) {
    if expected.is_empty() {
        return;
    }
    let same = if ordered { expected == got } else { expected.iter().collect::<BTreeSet<_>>() == got.iter().collect::<BTreeSet<_>>() };
    check(out, name, if same { "pass" } else { "fail" }, format!("expected {expected:?}, engine reported {got:?}"));
}

fn semantic_checks(case: &InCase, e: &Side, o: &Side) -> Vec<Check> {
    let ex = &case.expect;
    let mut c = Vec::new();
    let wants_open = matches!(case.outcome.as_str(), "opens" | "password");
    if wants_open {
        check(&mut c, "engine opens", if e.opened { "pass" } else { "fail" }, e.refusal.clone().unwrap_or_default());
    }
    if let Some(n) = ex.pages
        && e.opened
    {
        check(&mut c, "pages", if e.pages == Some(n) { "pass" } else { "fail" }, format!("expected {n}, engine {:?}, oracle {:?}", e.pages, o.pages));
    }
    if e.opened && !ex.sizes.is_empty() {
        let bad: Vec<String> = ex
            .sizes
            .iter()
            .enumerate()
            .filter(|(i, want)| e.sizes.get(*i).is_none_or(|got| (got[0] - want[0]).abs() > 1.0 || (got[1] - want[1]).abs() > 1.0))
            .map(|(i, want)| format!("page {} expected {want:?} got {:?}", i + 1, e.sizes.get(i)))
            .collect();
        check(&mut c, "page sizes", if bad.is_empty() { "pass" } else { "fail" }, bad.join("; "));
    }
    for (page, token) in &ex.text {
        let found_in =
            |s: &Side| if *page == 0 { s.text.values().any(|t| t.contains(token)) } else { s.text.get(page).is_some_and(|t| t.contains(token)) };
        if e.opened {
            check(
                &mut c,
                format!("text {token} (page {page})"),
                if found_in(e) { "pass" } else { "fail" },
                format!("oracle found it: {}", found_in(o)),
            );
        }
    }
    if e.opened {
        compare_lists(&mut c, "form fields", &ex.fields, &e.fields, false);
        compare_lists(&mut c, "outline", &ex.outlines, &e.outlines, true);
        compare_lists(&mut c, "attachments", &ex.attachments, &e.attachments, false);
        compare_lists(&mut c, "page labels", &ex.page_labels, &e.labels, true);
        if let Some(n) = ex.annotations {
            check(&mut c, "annotations", if e.annotations == Some(n) { "pass" } else { "fail" }, format!("expected {n}, engine {:?}", e.annotations));
        }
        if let Some(n) = ex.links {
            check(&mut c, "links", if e.links == Some(n) { "pass" } else { "fail" }, format!("expected {n}, engine {:?}", e.links));
        }
    }
    c
}

fn visual_checks(t: &Tools, case: &InCase, e: &Side, o: &Side, tol: super::model::Tolerance) -> Vec<Visual> {
    let Some(limits) = tol.limits() else { return Vec::new() };
    let mut out = Vec::new();
    for er in e.renders.iter().filter(|r| r.ok) {
        let Some(or) = o.renders.iter().find(|r| r.ok && r.page == er.page) else { continue };
        let ours = imgcmp::load(&t.corpus.join(format!("_render/engine-{}-{}.png", case.id, er.page)));
        let theirs = imgcmp::load(&t.corpus.join(format!("_render/oracle-{}-{}.png", case.id, or.page)));
        let (Ok(a), Ok(b)) = (ours, theirs) else { continue };
        let m = imgcmp::compare(&a, &b);
        let passes = m.size_match
            && m.diff_frac <= limits.diff_frac
            && m.mae <= limits.mae
            && m.ink_ratio >= limits.ink.0
            && m.ink_ratio <= limits.ink.1
            && m.max_block <= limits.max_block;
        out.push(Visual {
            page: er.page,
            ours: m.ours,
            theirs: m.theirs,
            size_match: m.size_match,
            diff_frac: m.diff_frac,
            mae: m.mae,
            ink_ratio: m.ink_ratio.is_finite().then_some(m.ink_ratio),
            max_block: m.max_block,
            passes,
        });
    }
    out
}

fn tokens_found(case: &InCase, s: &Side) -> (usize, usize) {
    let total = case.expect.text.len();
    let hit = case
        .expect
        .text
        .iter()
        .filter(|(p, tok)| if *p == 0 { s.text.values().any(|t| t.contains(tok)) } else { s.text.get(p).is_some_and(|t| t.contains(tok)) })
        .count();
    (hit, total)
}

fn verdict(case: &InCase, e: &Side, o: &Side, checks: &[Check], visual: &[Visual]) -> String {
    if e.crashed_or_hung {
        return "engine crashed or hung".into();
    }
    let failed = checks.iter().any(|c| c.status == "fail");
    let visual_bad = visual.iter().any(|v| !v.passes);
    match case.outcome.as_str() {
        "opens" | "password" => {
            if failed {
                "semantic divergence".into()
            } else if visual_bad {
                "visual divergence".into()
            } else {
                "match".into()
            }
        }
        "refused" => {
            if e.opened {
                "limit not enforced".into()
            } else {
                "limit enforced".into()
            }
        }
        _ => {
            let (eh, total) = tokens_found(case, e);
            let (oh, _) = tokens_found(case, o);
            match (e.opened, o.opened) {
                (false, false) => "both refuse".into(),
                (false, true) if oh > 0 => "recovery gap (oracle recovers, engine refuses)".into(),
                (false, true) => "engine refuses, oracle opens without content".into(),
                (true, false) => "engine recovers, oracle refuses".into(),
                (true, true) if total == 0 => "both open".into(),
                (true, true) if eh > oh => "engine recovers more".into(),
                (true, true) if eh < oh => "recovery gap (oracle recovers more)".into(),
                (true, true) => "equal recovery".into(),
            }
        }
    }
}

fn run_case(t: &Tools, case: &InCase) -> Result<CaseResult> {
    let tol = match case.tolerance.as_str() {
        "text" => super::model::Tolerance::Text,
        "vector" => super::model::Tolerance::Vector,
        "image" => super::model::Tolerance::Image,
        _ => super::model::Tolerance::None,
    };
    let file = format!("{}.pdf", case.id);
    let path = t.corpus.join(&file);
    let mut engine = engine_side(t, case, &file)?;
    let oracle = oracle_side(t, case, &path, engine.pages)?;
    let mut checks = semantic_checks(case, &engine, &oracle);
    // A password-protected case must also be refused without the password.
    if case.outcome == "password" {
        let o = engine_run(t, &json!([open_step(&file, None)]))?;
        let refused = o.code == Some(1) && String::from_utf8_lossy(&o.stderr).to_lowercase().contains("password");
        check(&mut checks, "refused without the password", if refused { "pass" } else { "fail" }, first_line(&o.stderr));
    }
    let visual = visual_checks(t, case, &engine, &oracle, tol);
    let v = verdict(case, &engine, &oracle, &checks, &visual);
    engine.sizes.truncate(8);
    Ok(CaseResult {
        id: case.id.clone(),
        family: case.family.clone(),
        class: case.class.clone(),
        expected_outcome: case.outcome.clone(),
        tolerance: case.tolerance.clone(),
        engine,
        oracle,
        checks,
        visual,
        verdict: v,
    })
}

/// Negative control for the visual check: compare each engine render with the oracle render of a
/// *different* case of the same size. A check that cannot fail would accept these.
fn negative_control(t: &Tools, manifest: &InManifest) -> (usize, Vec<String>) {
    let mut with_png = Vec::new();
    for c in &manifest.case {
        let e = t.corpus.join(format!("_render/engine-{}-1.png", c.id));
        let o = t.corpus.join(format!("_render/oracle-{}-1.png", c.id));
        if c.tolerance != "none" && e.is_file() && o.is_file() {
            with_png.push((c, e, o));
        }
    }
    let (mut tried, mut accepted) = (0, Vec::new());
    for (i, (a, ea, _)) in with_png.iter().enumerate() {
        let Ok(ours) = imgcmp::load(ea) else { continue };
        for (b, _, ob) in with_png.iter().skip(i + 1) {
            let Ok(theirs) = imgcmp::load(ob) else { continue };
            // Skip the same page under another name: encrypted copies of one base document, and
            // appearance-stream / no-appearance twins.
            let twin = |s: &str| s.replace("-no-ap", "").replace("-ap", "");
            if b.id == a.id
                || ours.w != theirs.w
                || ours.h != theirs.h
                || (a.family == "encrypted" && b.family == "encrypted")
                || twin(&a.id) == twin(&b.id)
            {
                continue;
            }
            let tol = match b.tolerance.as_str() {
                "vector" => super::model::Tolerance::Vector,
                "image" => super::model::Tolerance::Image,
                _ => super::model::Tolerance::Text,
            };
            let Some(l) = tol.limits() else { continue };
            let m = imgcmp::compare(&ours, &theirs);
            tried += 1;
            if m.size_match
                && m.diff_frac <= l.diff_frac
                && m.mae <= l.mae
                && m.ink_ratio >= l.ink.0
                && m.ink_ratio <= l.ink.1
                && m.max_block <= l.max_block
            {
                accepted.push(format!(
                    "{} vs {} (diff {:.4}, mae {:.2}, ink {:.2}, block {:.0})",
                    a.id, b.id, m.diff_frac, m.mae, m.ink_ratio, m.max_block
                ));
            }
            break;
        }
    }
    (tried, accepted)
}

/// `cargo xtask bdf-oracle [--cli P] [--mutool P] [--corpus DIR] [--out DIR] [--only ids] [--family F] [--dpi N] [--timeout S]`
pub fn run(args: &[String]) -> Result<()> {
    let r = root();
    let mut t = Tools {
        cli: target_dir().join("bench").join("release").join("pdfcraft-cli.exe"),
        mutool: r.join("target/bdf-evidence/BUILD-002/untrusted/mupdf-1.28.5/x/mupdf-1.28.5-windows/mutool.exe"),
        corpus: target_dir().join("bdf-corpus"),
        dpi: 96,
        timeout: Duration::from_secs(120),
    };
    let mut out = r.join("target/bdf-evidence/BUILD-002/oracle");
    let mut only: Option<BTreeSet<String>> = None;
    let mut family: Option<String> = None;
    let mut control = false;
    let mut control_only = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = || it.next().with_context(|| format!("{a} needs a value"));
        match a.as_str() {
            "--cli" => t.cli = PathBuf::from(val()?),
            "--mutool" => t.mutool = PathBuf::from(val()?),
            "--corpus" => t.corpus = PathBuf::from(val()?),
            "--out" => out = PathBuf::from(val()?),
            "--only" => only = Some(val()?.split(',').map(str::to_string).collect()),
            "--family" => family = Some(val()?.clone()),
            "--control" => control = true,
            "--control-only" => (control, control_only) = (true, true),
            "--dpi" => t.dpi = val()?.parse()?,
            "--timeout" => t.timeout = Duration::from_secs(val()?.parse()?),
            other => bail!("unknown argument `{other}`"),
        }
    }
    for (what, p) in [("engine CLI", &t.cli), ("mutool", &t.mutool)] {
        if !p.is_file() {
            bail!("{what} not found at {} (see --cli / --mutool)", p.display());
        }
    }
    let manifest: InManifest =
        toml::from_str(&std::fs::read_to_string(t.corpus.join("manifest.toml")).context("run `cargo xtask bdf-corpus` first")?)?;
    std::fs::create_dir_all(&out)?;
    let mut results = Vec::new();
    for case in manifest
        .case
        .iter()
        .filter(|c| !control_only && only.as_ref().is_none_or(|o| o.contains(&c.id)) && family.as_ref().is_none_or(|f| *f == c.family))
    {
        eprint!("{:<34} ", case.id);
        let r = run_case(&t, case)?;
        eprintln!("{}", r.verdict);
        results.push(r);
    }
    if !control_only {
        std::fs::write(out.join("results.json"), serde_json::to_vec_pretty(&results)?)?;
    }
    let mut tally: BTreeMap<&str, usize> = BTreeMap::new();
    for r in &results {
        *tally.entry(r.verdict.as_str()).or_default() += 1;
    }
    println!("\nbdf-oracle: {} cases, results in {}", results.len(), out.join("results.json").display());
    for (v, n) in tally {
        println!("  {n:>3}  {v}");
    }
    if control {
        let (tried, accepted) = negative_control(&t, &manifest);
        println!("negative control: {} of {tried} comparisons between different cases were accepted (want ~0)", accepted.len());
        for a in &accepted {
            println!("  accepted: {a}");
        }
    }
    Ok(())
}
