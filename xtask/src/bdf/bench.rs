//! `cargo xtask bdf-bench`: latency and memory of the headless engine and the real desktop app
//! against the PRD's budgets (section 8), on the BUILD-002 corpus.
//!
//! What it measures, and how:
//!
//! * **Headless first page** (`pdfcraft-cli run`: open, render page 1 at 96 dpi): wall time from
//!   spawn to exit, and the process's peak working set and peak commit from the operating
//!   system's counters, `runs` times per fixture. Reported as min, median, p95 and max (nearest
//!   rank), never an average.
//! * **Headless open/edit/close cycles**: one process doing 1 or 10 cycles of open, rotate,
//!   render, close; memory at the end of 10 cycles against 1 shows whether it settles.
//! * **Desktop launch**: spawn the real app with the opt-in control channel until it answers
//!   `ui.state` (a frame has been drawn and the app handles requests).
//! * **Desktop open**: `ui.open` until the first page raster exists, then until every queued render
//!   has finished, then close, repeated in one app instance, with the app's memory after each
//!   close.
//! * **Desktop cycles**: open, rotate, save, close, ten times; memory after each close.
//!
//! Every number belongs to the recorded machine, build and corpus, and to warm caches: a cold
//! start (after a reboot) is not measured. The PRD's reference machine is a 4-core CPU with
//! integrated graphics on a 1080p display; `--cores N` restricts the CPU count to approximate the
//! core count only, not the clock speed or the graphics.

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write as _};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::{Value, json};

use super::proc::{self, Mem};
use crate::gates::{root, target_dir};

const MIB: f64 = 1_048_576.0;

#[derive(Serialize, Clone)]
pub struct Stats {
    n: usize,
    min: f64,
    median: f64,
    p95: f64,
    max: f64,
}

/// Order statistics by nearest rank: the p-th percentile of n samples is the ceil(p*n)-th smallest.
pub fn stats(samples: &[f64]) -> Option<Stats> {
    if samples.is_empty() {
        return None;
    }
    let mut v = samples.to_vec();
    v.sort_by(f64::total_cmp);
    let rank = |p: f64| v.get(((p * v.len() as f64).ceil() as usize).saturating_sub(1).min(v.len() - 1)).copied().unwrap_or(0.0);
    Some(Stats { n: v.len(), min: v[0], median: rank(0.5), p95: rank(0.95), max: v[v.len() - 1] })
}

#[derive(Serialize)]
struct Row {
    scenario: String,
    class: String,
    case: String,
    runs: usize,
    failures: Vec<String>,
    wall_ms: Option<Stats>,
    /// A second timing, where a scenario has two (desktop: first page, then fully rendered).
    wall2_ms: Option<Stats>,
    peak_working_set_mib: Option<Stats>,
    peak_commit_mib: Option<Stats>,
    /// Working set after each run, in order (desktop settle checks).
    working_set_series_mib: Vec<f64>,
    note: String,
}

impl Row {
    fn new(scenario: &str, class: &str, case: &str, note: &str) -> Self {
        Self {
            scenario: scenario.into(),
            class: class.into(),
            case: case.into(),
            runs: 0,
            failures: Vec::new(),
            wall_ms: None,
            wall2_ms: None,
            peak_working_set_mib: None,
            peak_commit_mib: None,
            working_set_series_mib: Vec::new(),
            note: note.into(),
        }
    }
}

struct Cfg {
    cli: PathBuf,
    app: PathBuf,
    corpus: PathBuf,
    out: PathBuf,
    runs: usize,
    cores: Option<u32>,
    timeout: Duration,
    ui: bool,
    cli_scenarios: bool,
    only: Option<BTreeSet<String>>,
}

fn script_run(c: &Cfg, steps: &Value) -> Result<(proc::Output, Mem)> {
    let script = c.corpus.join("_script-bench.json");
    std::fs::write(&script, serde_json::to_vec(steps)?)?;
    let mut cmd = Command::new(&c.cli);
    cmd.arg("run").arg("--script").arg(&script).arg("--root").arg(&c.corpus);
    let (o, m, _) = proc::run_measured(&mut cmd, c.timeout, 4 << 20, c.cores)?;
    Ok((o, m))
}

fn mib(b: u64) -> f64 {
    b as f64 / MIB
}

fn first_page_steps(case: &str) -> Value {
    json!([
        {"tool": "doc_open", "args": {"path": format!("{case}.pdf")}},
        {"tool": "page_render", "args": {"doc": 1, "page": 1, "dpi": 96}, "out": format!("_render/bench-{case}.png")}
    ])
}

/// `n` cycles of open, rotate, render, close in one process. Document ids are session-local and
/// count up from 1, so cycle `i` uses id `i + 1`; a wrong guess makes the script fail and the run
/// is reported as a failure.
fn cycle_steps(case: &str, n: usize) -> Value {
    let mut steps = Vec::new();
    for i in 0..n {
        let doc = i + 1;
        steps.push(json!({"tool": "doc_open", "args": {"path": format!("{case}.pdf")}}));
        steps.push(json!({"tool": "page_rotate", "args": {"doc": doc, "degrees": 90, "pages": [1]}}));
        steps.push(json!({"tool": "page_render", "args": {"doc": doc, "page": 1, "dpi": 96}, "out": format!("_render/bench-cycle-{case}.png")}));
        steps.push(json!({"tool": "doc_close", "args": {"doc": doc, "discard_changes": true}}));
    }
    Value::Array(steps)
}

fn cli_scenario(c: &Cfg, scenario: &str, class: &str, case: &str, steps: &Value, note: &str) -> Result<Row> {
    let mut row = Row::new(scenario, class, case, note);
    // One unrecorded run warms the file cache, so the runs below are all alike.
    let _ = script_run(c, steps)?;
    let (mut wall, mut ws, mut commit) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..c.runs {
        let (o, m) = script_run(c, steps)?;
        row.runs += 1;
        if o.timed_out || o.code != Some(0) {
            row.failures.push(if o.timed_out {
                "timed out".to_string()
            } else {
                format!("exit {:?}: {}", o.code, String::from_utf8_lossy(&o.stderr).lines().next().unwrap_or(""))
            });
            continue;
        }
        wall.push(o.ms as f64);
        ws.push(mib(m.peak_working_set));
        commit.push(mib(m.peak_commit));
        // Short processes are gone before a steady working set can be read; only peaks are reported.
    }
    row.wall_ms = stats(&wall);
    row.peak_working_set_mib = stats(&ws);
    row.peak_commit_mib = stats(&commit);
    Ok(row)
}

// ---- the desktop app, through its control channel ----

/// A JSON-RPC connection to the app's loopback control port.
struct Ctl {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    id: u64,
}

impl Ctl {
    fn connect(file: &Path) -> Result<Self> {
        let info: Value = serde_json::from_str(&std::fs::read_to_string(file)?)?;
        let port = u16::try_from(info["port"].as_u64().context("no port")?)?;
        let token = info["token"].as_str().context("no token")?.to_string();
        let stream = TcpStream::connect(("127.0.0.1", port))?;
        stream.set_read_timeout(Some(Duration::from_secs(30)))?;
        stream.set_nodelay(true)?;
        let mut c = Self { reader: BufReader::new(stream.try_clone()?), writer: stream, id: 0 };
        c.call("auth", json!({ "token": token }))?;
        Ok(c)
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        self.id += 1;
        writeln!(self.writer, "{}", json!({"jsonrpc": "2.0", "id": self.id, "method": method, "params": params}))?;
        let mut line = String::new();
        if self.reader.read_line(&mut line)? == 0 {
            bail!("the app closed the connection");
        }
        let reply: Value = serde_json::from_str(&line)?;
        match reply.get("error") {
            Some(e) => bail!("{}", e["message"].as_str().unwrap_or("error")),
            None => Ok(reply["result"].clone()),
        }
    }
}

struct App {
    child: Child,
    ctl: Ctl,
}

impl App {
    /// Start the app and wait until it answers `ui.state`; the time is the launch time.
    fn launch(c: &Cfg, dir: &Path) -> Result<(Self, f64)> {
        let ctl_file = dir.join("ctl.json");
        let _ = std::fs::remove_file(&ctl_file);
        let start = Instant::now();
        let mut cmd = Command::new(&c.app);
        cmd.current_dir(dir)
            .arg("--control")
            .arg(&ctl_file)
            .env("PDFCRAFT_SYSTEM_FONTS", "0")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(dir.join("launch.err"))?);
        let mut child = cmd.spawn().context("starting the app")?;
        loop {
            if start.elapsed() > Duration::from_secs(60) {
                let _ = child.kill();
                bail!("the app did not answer within 60 s");
            }
            if ctl_file.is_file()
                && let Ok(mut ctl) = Ctl::connect(&ctl_file)
                && ctl.call("ui.state", json!({})).is_ok()
            {
                return Ok((Self { child, ctl }, start.elapsed().as_secs_f64() * 1000.0));
            }
            if child.try_wait()?.is_some() {
                bail!("the app exited during launch");
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn mem(&self) -> Mem {
        proc::sample(&self.child).unwrap_or_default()
    }

    fn state(&mut self) -> Result<Value> {
        self.ctl.call("ui.state", json!({}))
    }

    fn quit(mut self) {
        let _ = self.ctl.call("ui.quit", json!({}));
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(15) {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    /// Open `path`; return `(ms to the first page raster, ms until every queued render finished)`.
    fn open(&mut self, path: &Path) -> Result<(f64, f64)> {
        let start = Instant::now();
        self.ctl.call("ui.open", json!({ "path": path.to_string_lossy() }))?;
        let mut first = None;
        loop {
            if start.elapsed() > Duration::from_secs(120) {
                bail!("no first page within 120 s");
            }
            let s = self.state()?;
            let docs = s["documents"].as_array().map_or(0, Vec::len);
            let pending = s["pending_opens"].as_array().map_or(0, Vec::len);
            let raster = &s["active"]["raster_memory"];
            let errors = s["active"]["page_errors"].as_array().map_or(0, Vec::len);
            if docs > 0 && pending == 0 {
                if errors > 0 {
                    bail!("page error: {}", s["active"]["page_errors"]);
                }
                if first.is_none() && raster["page_bytes"].as_u64().unwrap_or(0) > 0 {
                    first = Some(start.elapsed().as_secs_f64() * 1000.0);
                }
                if first.is_some() && raster["pending_requests"].as_u64().unwrap_or(1) == 0 {
                    return Ok((first.unwrap_or(0.0), start.elapsed().as_secs_f64() * 1000.0));
                }
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn command(&mut self, id: &str) -> Result<()> {
        self.ctl.call("ui.command", json!({ "id": id }))?;
        Ok(())
    }

    fn wait_closed(&mut self) -> Result<()> {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(30) {
            if self.state()?["documents"].as_array().is_none_or(Vec::is_empty) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        bail!("the document did not close")
    }

    /// Id of the first registry command whose id contains every word in `words`.
    fn find_command(&mut self, words: &[&str]) -> Result<Option<String>> {
        let list = self.ctl.call("ui.commands", json!({}))?;
        let items = list.get("commands").unwrap_or(&list).as_array().cloned().unwrap_or_default();
        Ok(items
            .iter()
            .filter_map(|c| c.as_str().map(str::to_string).or_else(|| c["id"].as_str().map(str::to_string)))
            .find(|id| words.iter().all(|w| id.contains(w))))
    }
}

fn app_dir(c: &Cfg) -> Result<PathBuf> {
    let dir = c.out.join("app");
    std::fs::create_dir_all(&dir)?;
    std::fs::copy(&c.app, dir.join("pdfcraft.exe")).context("copying the app")?;
    // A portable marker keeps settings inside this folder, out of the user's profile.
    std::fs::write(dir.join("portable.txt"), b"")?;
    Ok(dir)
}

fn ui_launch(c: &Cfg, dir: &Path) -> Result<Row> {
    let mut row = Row::new("ui-launch", "launch", "(empty window)", "spawn to the first answered ui.state; warm file cache");
    let mut copy = Cfg { ..clone_cfg(c) };
    copy.app = dir.join("pdfcraft.exe");
    let (mut wall, mut ws, mut commit) = (Vec::new(), Vec::new(), Vec::new());
    // One unrecorded launch warms caches and creates the settings folder.
    if let Ok((app, _)) = App::launch(&copy, dir) {
        app.quit();
    }
    for _ in 0..c.runs {
        row.runs += 1;
        match App::launch(&copy, dir) {
            Ok((app, ms)) => {
                let m = app.mem();
                wall.push(ms);
                ws.push(mib(m.peak_working_set));
                commit.push(mib(m.peak_commit));
                app.quit();
            }
            Err(e) => row.failures.push(e.to_string()),
        }
    }
    row.wall_ms = stats(&wall);
    row.peak_working_set_mib = stats(&ws);
    row.peak_commit_mib = stats(&commit);
    Ok(row)
}

fn clone_cfg(c: &Cfg) -> Cfg {
    Cfg {
        cli: c.cli.clone(),
        app: c.app.clone(),
        corpus: c.corpus.clone(),
        out: c.out.clone(),
        runs: c.runs,
        cores: c.cores,
        timeout: c.timeout,
        ui: c.ui,
        cli_scenarios: c.cli_scenarios,
        only: c.only.clone(),
    }
}

fn ui_open(c: &Cfg, dir: &Path, class: &str, case: &str) -> Result<Row> {
    let mut row =
        Row::new("ui-open", class, case, "ui.open to first page raster (wall_ms) and to all renders finished (wall2_ms); one warm app instance");
    let mut copy = clone_cfg(c);
    copy.app = dir.join("pdfcraft.exe");
    let (mut app, _) = App::launch(&copy, dir)?;
    let doc = dir.join(format!("{case}.pdf"));
    std::fs::copy(c.corpus.join(format!("{case}.pdf")), &doc)?;
    let (mut first, mut done) = (Vec::new(), Vec::new());
    // Open number 0 is an unrecorded warm-up; the next `runs` are measured.
    for i in 0..=c.runs {
        match app.open(&doc) {
            Ok((f, d)) => {
                if i > 0 {
                    first.push(f);
                    done.push(d);
                    row.runs += 1;
                }
                if app.command("file.close").is_err() || app.wait_closed().is_err() {
                    row.failures.push("close failed".into());
                    break;
                }
                if i > 0 {
                    row.working_set_series_mib.push(mib(app.mem().working_set));
                }
            }
            Err(e) => {
                row.failures.push(e.to_string());
                row.runs += 1;
                break;
            }
        }
    }
    let m = app.mem();
    row.wall_ms = stats(&first);
    row.wall2_ms = stats(&done);
    row.peak_working_set_mib = stats(&[mib(m.peak_working_set)]);
    row.peak_commit_mib = stats(&[mib(m.peak_commit)]);
    app.quit();
    Ok(row)
}

fn ui_cycles(c: &Cfg, dir: &Path, case: &str) -> Result<Row> {
    let mut row = Row::new("ui-cycles", "ordinary", case, "ten open, rotate, save, close cycles in one app; working set after each close");
    let mut copy = clone_cfg(c);
    copy.app = dir.join("pdfcraft.exe");
    let (mut app, _) = App::launch(&copy, dir)?;
    let Some(rotate) = app.find_command(&["rotate"])? else {
        row.failures.push("no rotate command in the registry".into());
        app.quit();
        return Ok(row);
    };
    row.note = format!("{} (rotate command: {rotate})", row.note);
    let doc = dir.join(format!("cycle-{case}.pdf"));
    for _ in 0..10 {
        std::fs::copy(c.corpus.join(format!("{case}.pdf")), &doc)?;
        row.runs += 1;
        let step = (|| -> Result<()> {
            app.open(&doc)?;
            app.command(&rotate)?;
            app.command("file.save")?;
            app.command("file.close")?;
            app.wait_closed()
        })();
        if let Err(e) = step {
            row.failures.push(e.to_string());
            break;
        }
        row.working_set_series_mib.push(mib(app.mem().working_set));
    }
    let m = app.mem();
    row.peak_working_set_mib = stats(&[mib(m.peak_working_set)]);
    row.peak_commit_mib = stats(&[mib(m.peak_commit)]);
    app.quit();
    Ok(row)
}

fn machine_description() -> Value {
    let wmic = |args: &[&str]| -> String {
        Command::new("powershell")
            .args(["-NoProfile", "-Command"])
            .arg(args.join(" "))
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    };
    json!({
        "os": wmic(&["(Get-CimInstance Win32_OperatingSystem).Caption + ' build ' + (Get-CimInstance Win32_OperatingSystem).BuildNumber"]),
        "cpu": wmic(&["(Get-CimInstance Win32_Processor | Select-Object -First 1).Name.Trim()"]),
        "logical_cpus": wmic(&["(Get-CimInstance Win32_ComputerSystem).NumberOfLogicalProcessors"]),
        "ram_gb": wmic(&["[math]::Round((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory/1GB,1)"]),
        "gpu": wmic(&["(Get-CimInstance Win32_VideoController | Select-Object -First 1).Name"]),
    })
}

/// `cargo xtask bdf-bench [--runs N] [--cores N] [--ui] [--no-cli] [--only a,b] [--cli P] [--app P] [--out DIR]`
pub fn run(args: &[String]) -> Result<()> {
    let r = root();
    let mut c = Cfg {
        cli: target_dir().join("bench/release/pdfcraft-cli.exe"),
        app: target_dir().join("bench/release/pdfcraft.exe"),
        corpus: target_dir().join("bdf-corpus"),
        out: r.join("target/bdf-evidence/BUILD-002/bench"),
        runs: 30,
        cores: None,
        timeout: Duration::from_secs(180),
        ui: false,
        cli_scenarios: true,
        only: None,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = || it.next().with_context(|| format!("{a} needs a value"));
        match a.as_str() {
            "--runs" => c.runs = val()?.parse()?,
            "--cores" => c.cores = Some(val()?.parse()?),
            "--ui" => c.ui = true,
            "--no-cli" => c.cli_scenarios = false,
            "--only" => c.only = Some(val()?.split(',').map(str::to_string).collect()),
            "--cli" => c.cli = PathBuf::from(val()?),
            "--app" => c.app = PathBuf::from(val()?),
            "--out" => c.out = PathBuf::from(val()?),
            "--timeout" => c.timeout = Duration::from_secs(val()?.parse()?),
            other => bail!("unknown argument `{other}`"),
        }
    }
    if c.runs < 2 {
        bail!("--runs must be at least 2");
    }
    // The app runs with its own folder as the working directory, so every path must be absolute.
    for p in [&mut c.cli, &mut c.app, &mut c.corpus, &mut c.out] {
        *p = std::path::absolute(&*p).with_context(|| format!("resolving {}", p.display()))?;
    }
    std::fs::create_dir_all(&c.out)?;
    std::fs::create_dir_all(c.corpus.join("_render"))?;
    let want = |name: &str| c.only.as_ref().is_none_or(|o| o.contains(name));
    let mut rows: Vec<Row> = Vec::new();
    if c.cli_scenarios {
        for (class, case) in [
            ("ordinary", "office-100p-embedded"),
            ("ordinary", "office-30p-std14"),
            ("scan", "scan-multipage-16"),
            ("scan", "scan-rgb-300"),
            ("plan", "plan-hatch-400k"),
            ("plan", "plan-labels-3k"),
            ("large", "hostile-size-251mib"),
            ("hostile", "hostile-huge-mediabox"),
        ] {
            if want(case) {
                eprintln!("cli first-page {case}");
                rows.push(cli_scenario(
                    &c,
                    "cli-first-page",
                    class,
                    case,
                    &first_page_steps(case),
                    "open and render page 1 at 96 dpi in one process",
                )?);
            }
        }
        for n in [1usize, 10] {
            let name = format!("cli-cycles-{n}");
            if want(&name) {
                eprintln!("cli {name}");
                rows.push(cli_scenario(
                    &c,
                    &name,
                    "ordinary",
                    "office-100p-embedded",
                    &cycle_steps("office-100p-embedded", n),
                    "open, rotate, render, close, repeated in one process; working_set is the last sample before exit",
                )?);
            }
        }
    }
    if c.ui {
        let dir = app_dir(&c)?;
        if want("ui-launch") {
            eprintln!("ui launch");
            rows.push(ui_launch(&c, &dir)?);
        }
        for (class, case) in [("ordinary", "office-100p-embedded"), ("scan", "scan-multipage-16"), ("plan", "plan-hatch-400k")] {
            if want(case) {
                eprintln!("ui open {case}");
                rows.push(ui_open(&c, &dir, class, case)?);
            }
        }
        if want("ui-cycles") {
            eprintln!("ui cycles");
            rows.push(ui_cycles(&c, &dir, "office-100p-embedded")?);
        }
    }
    let report = json!({
        "machine": machine_description(),
        "cores_limit": c.cores,
        "runs": c.runs,
        "cli": c.cli.to_string_lossy(),
        "app": c.app.to_string_lossy(),
        "rows": rows,
    });
    std::fs::write(c.out.join("bench.json"), serde_json::to_vec_pretty(&report)?)?;
    println!(
        "\n{:<18} {:<9} {:<24} {:>4} {:>9} {:>9} {:>9} {:>9} {:>10} {:>10}",
        "scenario", "class", "case", "n", "min ms", "med ms", "p95 ms", "max ms", "peakWS MiB", "commit MiB"
    );
    for r in &rows {
        let w = r.wall_ms.as_ref();
        let f = |s: Option<&Stats>, pick: fn(&Stats) -> f64| s.map_or("-".to_string(), |s| format!("{:.0}", pick(s)));
        println!(
            "{:<18} {:<9} {:<24} {:>4} {:>9} {:>9} {:>9} {:>9} {:>10} {:>10}{}",
            r.scenario,
            r.class,
            r.case,
            w.map_or(0, |s| s.n),
            f(w, |s| s.min),
            f(w, |s| s.median),
            f(w, |s| s.p95),
            f(w, |s| s.max),
            f(r.peak_working_set_mib.as_ref(), |s| s.max),
            f(r.peak_commit_mib.as_ref(), |s| s.max),
            if r.failures.is_empty() { String::new() } else { format!("  FAILURES: {}", r.failures.len()) }
        );
    }
    println!("\nbdf-bench: results in {}", c.out.join("bench.json").display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_use_nearest_rank() {
        let v: Vec<f64> = (1..=30).map(f64::from).collect();
        let s = stats(&v).unwrap();
        assert_eq!((s.n, s.min, s.max), (30, 1.0, 30.0));
        assert_eq!(s.median, 15.0);
        assert_eq!(s.p95, 29.0, "ceil(0.95 * 30) = 29th smallest");
        let one = stats(&[7.0]).unwrap();
        assert_eq!((one.min, one.median, one.p95, one.max), (7.0, 7.0, 7.0, 7.0));
        assert!(stats(&[]).is_none());
    }

    #[test]
    fn an_unsorted_input_gives_the_same_answer() {
        let a = stats(&[5.0, 1.0, 9.0, 3.0, 7.0]).unwrap();
        let b = stats(&[1.0, 3.0, 5.0, 7.0, 9.0]).unwrap();
        assert_eq!((a.min, a.median, a.p95, a.max), (b.min, b.median, b.p95, b.max));
        assert_eq!(a.median, 5.0);
        assert_eq!(a.p95, 9.0);
    }

    #[test]
    fn cycle_scripts_number_documents_from_one() {
        let s = cycle_steps("x", 3);
        let ids: Vec<u64> =
            s.as_array().unwrap().iter().filter(|st| st["tool"] == "doc_close").map(|st| st["args"]["doc"].as_u64().unwrap()).collect();
        assert_eq!(ids, vec![1, 2, 3]);
        assert_eq!(s.as_array().unwrap().len(), 12);
    }
}
