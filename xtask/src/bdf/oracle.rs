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

/// One required comparison. Either it was made (`status` is `compared`, and `passes` says whether it
/// is within tolerance) or it could not be, and `status` says why: a comparison that could not be made
/// never passes, so a render refusal, a missing or undecodable image or a failed oracle cannot be
/// mistaken for agreement.
#[derive(Serialize)]
struct Visual {
    page: usize,
    status: String,
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
    /// Set when a check that the case requires was not made; the verdict is then `incomplete`.
    coverage_gap: Option<String>,
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

/// A required comparison that could not be made.
fn not_compared(page: usize, why: String) -> Visual {
    Visual {
        page,
        status: why,
        ours: (0, 0),
        theirs: (0, 0),
        size_match: false,
        diff_frac: 1.0,
        mae: 255.0,
        ink_ratio: None,
        max_block: 255.0,
        passes: false,
    }
}

/// Every page the engine was asked to render is a required comparison; the result has one entry for
/// each, in order, whether or not it could be made.
fn visual_checks(t: &Tools, case: &InCase, e: &Side, o: &Side, tol: super::model::Tolerance) -> Vec<Visual> {
    let Some(limits) = tol.limits() else { return Vec::new() };
    let mut out = Vec::new();
    for er in &e.renders {
        if !er.ok {
            out.push(not_compared(er.page, format!("engine render failed: {}", er.error.as_deref().unwrap_or("no output"))));
            continue;
        }
        let Some(or) = o.renders.iter().find(|r| r.page == er.page) else {
            out.push(not_compared(er.page, "oracle did not render this page".into()));
            continue;
        };
        if !or.ok {
            out.push(not_compared(er.page, format!("oracle render failed: {}", or.error.as_deref().unwrap_or("no output"))));
            continue;
        }
        let ours = imgcmp::load(&t.corpus.join(format!("_render/engine-{}-{}.png", case.id, er.page)));
        let theirs = imgcmp::load(&t.corpus.join(format!("_render/oracle-{}-{}.png", case.id, or.page)));
        let (a, b) = match (ours, theirs) {
            (Ok(a), Ok(b)) => (a, b),
            (Err(err), _) => {
                out.push(not_compared(er.page, format!("engine image unreadable: {err}")));
                continue;
            }
            (_, Err(err)) => {
                out.push(not_compared(er.page, format!("oracle image unreadable: {err}")));
                continue;
            }
        };
        let m = imgcmp::compare(&a, &b);
        let passes = m.size_match
            && m.diff_frac <= limits.diff_frac
            && m.mae <= limits.mae
            && m.ink_ratio >= limits.ink.0
            && m.ink_ratio <= limits.ink.1
            && m.max_block <= limits.max_block;
        out.push(Visual {
            page: er.page,
            status: "compared".into(),
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

fn tolerance_of(case: &InCase) -> super::model::Tolerance {
    match case.tolerance.as_str() {
        "text" => super::model::Tolerance::Text,
        "vector" => super::model::Tolerance::Vector,
        "image" => super::model::Tolerance::Image,
        _ => super::model::Tolerance::None,
    }
}

/// Why a case that should have been fully checked was not, or `None` when every required check was
/// made. Required: the oracle opens the file, and for a case with a visual class every page the engine
/// was asked to render has a comparison that was actually made.
fn coverage_gap(case: &InCase, e: &Side, o: &Side, visual: &[Visual]) -> Option<String> {
    if !o.opened {
        return Some(format!("oracle did not open the file ({})", o.refusal.as_deref().unwrap_or("no reason")));
    }
    if o.crashed_or_hung {
        return Some("oracle crashed or hung".into());
    }
    // A case with no visual class has no render to compare.
    tolerance_of(case).limits()?;
    if e.renders.is_empty() {
        return Some("no render was attempted although the case has a visual class".into());
    }
    let missing: Vec<String> = e
        .renders
        .iter()
        .filter(|r| !visual.iter().any(|v| v.page == r.page && v.status == "compared"))
        .map(|r| match visual.iter().find(|v| v.page == r.page) {
            Some(v) => format!("page {}: {}", r.page, v.status),
            None => format!("page {}: no comparison was made", r.page),
        })
        .collect();
    (!missing.is_empty()).then(|| missing.join("; "))
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
            } else if coverage_gap(case, e, o, visual).is_some() {
                "incomplete".into()
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

/// A divergence that is understood and accepted for now, recorded with the reason.
#[derive(Deserialize, Clone)]
struct Known {
    id: String,
    /// The exact verdict that is excused; a case whose verdict changes is no longer excused.
    verdict: String,
    reason: String,
}

#[derive(Deserialize)]
struct KnownFile {
    #[serde(default)]
    known: Vec<Known>,
}

/// Verdicts that need no excuse: the engine did what the case required.
fn accepted(verdict: &str) -> bool {
    matches!(
        verdict,
        "match" | "limit enforced" | "both refuse" | "both open" | "equal recovery" | "engine recovers more" | "engine recovers, oracle refuses"
    )
}

/// The strict gate: the reasons a run must fail. `rows` are (case id, verdict); `total` is how many
/// cases the corpus has. A run passes only if every case ran, every verdict is accepted or excused by a
/// known exception that names that exact verdict, and no exception is stale.
fn strict_failures(rows: &[(String, String)], total: usize, known: &[Known]) -> Vec<String> {
    let mut out = Vec::new();
    if rows.len() != total {
        out.push(format!("only {} of {total} cases ran; the strict gate needs all of them", rows.len()));
    }
    let mut seen = BTreeSet::new();
    for k in known {
        if k.reason.trim().is_empty() {
            out.push(format!("known exception for `{}` gives no reason", k.id));
        }
        if !seen.insert(k.id.as_str()) {
            out.push(format!("`{}` is excused more than once", k.id));
        }
        match rows.iter().find(|(id, _)| *id == k.id) {
            None if rows.len() == total => out.push(format!("known exception names `{}`, which is not a case", k.id)),
            None => {}
            Some((_, v)) if accepted(v) => out.push(format!("stale exception: `{}` is now `{v}`, which needs no excuse; remove it", k.id)),
            Some((_, v)) if *v != k.verdict => out.push(format!("stale exception: `{}` was excused as `{}` but is now `{v}`", k.id, k.verdict)),
            Some(_) => {}
        }
    }
    for (id, v) in rows {
        if !accepted(v) && !known.iter().any(|k| k.id == *id && k.verdict == *v) {
            out.push(format!("`{id}`: {v}"));
        }
    }
    out
}

fn run_case(t: &Tools, case: &InCase) -> Result<CaseResult> {
    let tol = tolerance_of(case);
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
    let gap = if matches!(case.outcome.as_str(), "opens" | "password") { coverage_gap(case, &engine, &oracle, &visual) } else { None };
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
        coverage_gap: gap,
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

/// `cargo xtask bdf-oracle [--strict [--known FILE]] [--cli P] [--mutool P] [--corpus DIR] [--out DIR] [--only ids] [--family F] [--dpi N] [--timeout S]`
///
/// Without `--strict` the run is exploratory: it records every divergence and exits 0. With it the run
/// is a regression gate: it fails unless all cases ran, every required comparison was made, and every
/// non-accepted verdict is excused by an entry in the known-exceptions file.
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
    let mut strict = false;
    let mut known_path = r.join("xtask/baselines/bdf-oracle-known.toml");
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
            "--strict" => strict = true,
            "--known" => known_path = PathBuf::from(val()?),
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
    let (mut required, mut made) = (0usize, 0usize);
    let mut gaps = Vec::new();
    for r in &results {
        if r.tolerance != "none" && matches!(r.expected_outcome.as_str(), "opens" | "password") {
            required += r.engine.renders.len();
            made += r.visual.iter().filter(|v| v.status == "compared").count();
        }
        if let Some(g) = &r.coverage_gap {
            gaps.push(format!("{}: {g}", r.id));
        }
    }
    println!("coverage: {made} of {required} required visual comparisons were made; {} case(s) with a coverage gap", gaps.len());
    for g in &gaps {
        println!("  gap: {g}");
    }
    let known: Vec<Known> = match std::fs::read_to_string(&known_path) {
        Ok(text) => toml::from_str::<KnownFile>(&text).with_context(|| format!("reading {}", known_path.display()))?.known,
        Err(_) if !strict => Vec::new(),
        Err(e) => bail!("strict mode needs the known exceptions at {}: {e}", known_path.display()),
    };
    let rows: Vec<(String, String)> = results.iter().map(|r| (r.id.clone(), r.verdict.clone())).collect();
    let failures = strict_failures(&rows, manifest.case.len(), &known);
    println!(
        "{}: {} reason(s) the strict gate would fail ({} known exception(s) read from {})",
        if strict { "strict gate" } else { "exploratory run (exit status does not depend on the gate)" },
        failures.len(),
        known.len(),
        known_path.display()
    );
    for f in &failures {
        println!("  - {f}");
    }
    if control {
        let (tried, accepted) = negative_control(&t, &manifest);
        println!("negative control: {} of {tried} comparisons between different cases were accepted (want ~0)", accepted.len());
        for a in &accepted {
            println!("  accepted: {a}");
        }
    }
    if strict && !failures.is_empty() {
        bail!("bdf-oracle --strict: {} failure(s)", failures.len());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn case(outcome: &str, tolerance: &str) -> InCase {
        InCase {
            id: "t-case".into(),
            family: "office".into(),
            class: "ordinary".into(),
            outcome: outcome.into(),
            tolerance: tolerance.into(),
            expect: InExpect::default(),
        }
    }

    fn render(page: usize, ok: bool) -> Render {
        Render { page, ok, error: (!ok).then(|| "refused".to_string()), width: 10, height: 10, ms: 1 }
    }

    fn side(opened: bool, renders: Vec<Render>) -> Side {
        Side { opened, renders, pages: Some(1), ..Side::default() }
    }

    fn good_visual(page: usize) -> Visual {
        Visual {
            page,
            status: "compared".into(),
            ours: (10, 10),
            theirs: (10, 10),
            size_match: true,
            diff_frac: 0.0,
            mae: 0.0,
            ink_ratio: Some(1.0),
            max_block: 0.0,
            passes: true,
        }
    }

    #[test]
    fn a_render_the_engine_refused_cannot_be_a_match() {
        // Codex's reproduction: open, metadata and text succeed, page_render exits 1; no visual entry exists.
        let c = case("opens", "text");
        let e = side(true, vec![render(1, false)]);
        let o = side(true, vec![render(1, true)]);
        assert_ne!(verdict(&c, &e, &o, &[], &[]), "match");
    }

    #[test]
    fn a_render_the_oracle_failed_to_make_cannot_be_a_match() {
        let c = case("opens", "vector");
        let e = side(true, vec![render(1, true)]);
        let o = side(true, vec![render(1, false)]);
        assert_ne!(verdict(&c, &e, &o, &[], &[]), "match");
    }

    #[test]
    fn an_oracle_that_did_not_open_the_file_cannot_make_it_a_match() {
        let c = case("opens", "text");
        let e = side(true, vec![render(1, true)]);
        let o = side(false, vec![]);
        assert_ne!(verdict(&c, &e, &o, &[], &[good_visual(1)]), "match");
    }

    #[test]
    fn a_required_render_that_was_never_attempted_cannot_be_a_match() {
        let c = case("opens", "text");
        assert_ne!(verdict(&c, &side(true, vec![]), &side(true, vec![]), &[], &[]), "match");
    }

    #[test]
    fn a_render_without_a_comparison_cannot_be_a_match() {
        // Both sides rendered, but nothing was compared (the comparison was skipped).
        let c = case("opens", "text");
        let e = side(true, vec![render(1, true)]);
        let o = side(true, vec![render(1, true)]);
        assert_ne!(verdict(&c, &e, &o, &[], &[]), "match");
    }

    fn row(id: &str, v: &str) -> (String, String) {
        (id.to_string(), v.to_string())
    }

    fn known(id: &str, v: &str, why: &str) -> Known {
        Known { id: id.into(), verdict: v.into(), reason: why.into() }
    }

    #[test]
    fn the_strict_gate_fails_on_an_unexcused_divergence_or_incomplete_case() {
        for bad in ["incomplete", "visual divergence", "semantic divergence", "engine crashed or hung", "limit not enforced"] {
            let f = strict_failures(&[row("a", "match"), row("b", bad)], 2, &[]);
            assert_eq!(f.len(), 1, "{bad}: {f:?}");
            assert!(f[0].contains('b') && f[0].contains(bad), "{f:?}");
        }
        assert!(strict_failures(&[row("a", "match"), row("b", "both open")], 2, &[]).is_empty());
    }

    #[test]
    fn a_known_exception_excuses_exactly_its_verdict() {
        let k = [known("b", "visual divergence", "aliasing, see 4.4")];
        assert!(strict_failures(&[row("b", "visual divergence")], 1, &k).is_empty());
        // The case got worse in a different way: no longer excused.
        assert_eq!(strict_failures(&[row("b", "incomplete")], 1, &k).len(), 2, "unexcused verdict and a stale exception");
    }

    #[test]
    fn exceptions_that_are_stale_unreasoned_or_unknown_fail_the_gate() {
        // The case now matches: the exception must be removed so the list stays true.
        assert_eq!(strict_failures(&[row("b", "match")], 1, &[known("b", "visual divergence", "x")]).len(), 1);
        // No reason given.
        assert!(!strict_failures(&[row("b", "visual divergence")], 1, &[known("b", "visual divergence", " ")]).is_empty());
        // Names a case that does not exist.
        assert!(!strict_failures(&[row("b", "match")], 1, &[known("zz", "visual divergence", "x")]).is_empty());
        // The same case excused twice.
        let twice = [known("b", "visual divergence", "x"), known("b", "visual divergence", "y")];
        assert!(!strict_failures(&[row("b", "visual divergence")], 1, &twice).is_empty());
    }

    #[test]
    fn the_strict_gate_fails_when_not_every_case_ran() {
        let f = strict_failures(&[row("a", "match")], 3, &[]);
        assert!(f.iter().any(|m| m.contains("1 of 3")), "{f:?}");
    }

    #[test]
    fn complete_passing_coverage_is_a_match_and_a_case_with_no_visual_class_needs_none() {
        let c = case("opens", "text");
        let e = side(true, vec![render(1, true)]);
        let o = side(true, vec![render(1, true)]);
        assert_eq!(verdict(&c, &e, &o, &[], &[good_visual(1)]), "match");
        let none = case("opens", "none");
        assert_eq!(verdict(&none, &side(true, vec![]), &side(true, vec![]), &[], &[]), "match");
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bdf-oracle-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("_render")).unwrap();
        d
    }

    fn tools(corpus: PathBuf) -> Tools {
        Tools { cli: PathBuf::new(), mutool: PathBuf::new(), corpus, dpi: 96, timeout: Duration::from_secs(1) }
    }

    fn write_png(path: &Path, rgb: [u8; 3]) {
        let mut buf = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut buf, 8, 8);
            enc.set_color(png::ColorType::Rgb);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().unwrap();
            w.write_image_data(&rgb.repeat(64)).unwrap();
        }
        std::fs::write(path, buf).unwrap();
    }

    #[test]
    fn comparisons_whose_images_are_missing_or_unreadable_are_reported_not_skipped() {
        let dir = scratch("images");
        let t = tools(dir.clone());
        let c = case("opens", "text");
        let e = side(true, vec![render(1, true), render(2, true), render(3, true)]);
        let o = side(true, vec![render(1, true), render(2, true), render(3, true)]);
        // Page 1: both images fine. Page 2: engine PNG missing. Page 3: oracle PNG is not an image.
        write_png(&dir.join("_render/engine-t-case-1.png"), [9, 9, 9]);
        write_png(&dir.join("_render/oracle-t-case-1.png"), [9, 9, 9]);
        write_png(&dir.join("_render/oracle-t-case-2.png"), [9, 9, 9]);
        write_png(&dir.join("_render/engine-t-case-3.png"), [9, 9, 9]);
        std::fs::write(dir.join("_render/oracle-t-case-3.png"), b"not a png").unwrap();
        let v = visual_checks(&t, &c, &e, &o, super::super::model::Tolerance::Text);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(v.len(), 3, "every required page has a result, compared or not");
        assert!(v[0].passes);
        assert!(!v[1].passes && !v[2].passes);
        assert_ne!(verdict(&c, &e, &o, &[], &v), "match");
    }
}
