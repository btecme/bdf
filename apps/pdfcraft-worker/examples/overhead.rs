//! What the process boundary costs: the same open and first-page render in this process and through
//! a spawned worker, repeated, with the worker's own memory. Prints one JSON object.
//!
//! `cargo run --release -p pdfcraft-worker --example overhead -- <worker.exe> <file.pdf> [runs] [dpi]`

use std::sync::Arc;
use std::time::Instant;

use pdfcraft_render::{PageRenderer, RenderConfig, RenderRequest, RequestKind};
use pdfcraft_worker::supervisor::{Config, Session};

fn percentile(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(f64::total_cmp);
    v.get(((p * v.len() as f64).ceil() as usize).saturating_sub(1).min(v.len().saturating_sub(1))).copied().unwrap_or(0.0)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (Some(worker), Some(file)) = (args.get(1), args.get(2)) else {
        eprintln!("usage: overhead <worker.exe> <file.pdf> [runs] [dpi]");
        std::process::exit(2);
    };
    let runs: usize = args.get(3).and_then(|r| r.parse().ok()).unwrap_or(30);
    let dpi: f32 = args.get(4).and_then(|r| r.parse().ok()).unwrap_or(96.0);
    let bytes = std::fs::read(file).expect("read the file");
    let (mut inproc, mut via, mut spawn_only) = (Vec::new(), Vec::new(), Vec::new());
    let mut worker_peak_mib = 0.0f64;
    for i in 0..=runs {
        let t = Instant::now();
        let mut r = PageRenderer::new(Arc::new(bytes.clone()), RenderConfig { reject_oversize: true, ..RenderConfig::default() });
        let p = r.render(RenderRequest { page: 0, kind: RequestKind::Pixels, tile: None, scale: dpi / 72.0, tag: 0, preview: false });
        assert!(p.error.is_none());
        let a = t.elapsed().as_secs_f64() * 1000.0;
        let t = Instant::now();
        let mut s = Session::open(&Config::new(worker), bytes.clone(), None).expect("open through the worker");
        let b0 = t.elapsed().as_secs_f64() * 1000.0;
        let r = s.render(1, dpi).expect("render through the worker");
        let b = t.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(r.width, p.width);
        if let Some((ws, _)) = s.worker_memory() {
            worker_peak_mib = worker_peak_mib.max(ws as f64 / 1_048_576.0);
        }
        if i > 0 {
            inproc.push(a);
            via.push(b);
            spawn_only.push(b0);
        }
    }
    println!(
        "{{\"file\":{file:?},\"runs\":{runs},\"dpi\":{dpi},\"in_process_ms\":{{\"median\":{:.1},\"p95\":{:.1}}},\"worker_ms\":{{\"median\":{:.1},\"p95\":{:.1}}},\"spawn_and_open_ms\":{{\"median\":{:.1},\"p95\":{:.1}}},\"worker_peak_working_set_mib\":{:.0}}}",
        percentile(&mut inproc.clone(), 0.5),
        percentile(&mut inproc, 0.95),
        percentile(&mut via.clone(), 0.5),
        percentile(&mut via, 0.95),
        percentile(&mut spawn_only.clone(), 0.5),
        percentile(&mut spawn_only, 0.95),
        worker_peak_mib
    );
}
