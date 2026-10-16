# BDF-0003: Proposed performance and memory budgets

> **Status:** Accepted by the owner (2026-10-15): the ceilings in the table are adopted as acceptance targets, not as measured results; the unmeasured rows stay unmeasured until qualified · **Date:** 2026-10-15 · **Work item:** BUILD-002 · **Evidence:** [BUILD-002](../bdf-evidence/BUILD-002.md) section 5

The PRD's budgets (section 8) were proposed before any measurement. BUILD-002 measured them on one
machine, with 30 timed runs per scenario, on a release build, on a synthetic corpus. This record
proposes which to keep, and what is still unmeasured. The measurements prove headroom on these
fixtures; they do not prove the budgets for real documents.

| PRD budget | Measured (this machine) | Proposal |
|---|---|---|
| Launch: p95 usable window ≤ 2 s | p95 381 ms, max 406 ms (warm) | Keep 2 s. Cold start unmeasured. |
| Ordinary open: p95 first page ≤ 2 s (100 pages, ≤ 10 MiB) | desktop p95 69 ms, headless p95 56 ms | Keep 2 s. |
| Scan or plan open: p95 first page ≤ 5 s (≤ 100 MiB) | scan (95 MiB) desktop p95 119 ms; plan sheet desktop p95 370 ms, headless p95 2.39 s | Keep 5 s. The heaviest synthetic plan uses 48% of it headless. |
| Interaction: input < 100 ms, frames ≤ 33 ms | not measured | Keep; needs frame instrumentation. |
| Cancellation: acknowledge < 100 ms, stopped ≤ 2 s | worker: acknowledged at once, gone in about 315 ms | Keep. |
| Memory: ≤ 500 MiB ordinary, ≤ 1.5 GiB large | desktop peak 222 MiB ordinary, 336 MiB for a 95 MiB scan | Keep both. |
| Ten open/edit/close cycles settle within 10% | +1.8% over 10 cycles, +1.9% over 30 opens | Keep 10%. |
| Recovery: checkpoint within 5 s | not measured (BUILD-004) | Keep. |
| Regression: no unexplained > 10% | needs a stored baseline | Store `bench.json` per release candidate. |

Observations that matter for the proposal:

- **A hostile page size costs about 500 MiB.** A 10,000,000-point page is clamped to an 8000x8000
  raster, which peaks at 498 MiB, as much as the whole budget for an ordinary document. Propose a
  lower pixel cap (for example 16 Mpx for a whole-page raster) with tiling beyond it. **Pixel
  admission must happen before the raster is allocated**, in the code that sizes the output: a check
  after allocation has already spent the memory it was meant to protect. The worker prototype checks
  the declared dimensions and length of a reply before it allocates, but the engine's own raster
  allocation is not yet gated, so the cap is not enforced end to end.
- **Admission limits are not a process-tree budget.** The per-operation limits (document size, page
  count, pixels per raster, chunk size) bound one request each. The memory budgets above are for the
  whole session, including the parent's copy and the worker's copy of the document and the raster. A
  hard memory limit and a sandbox for the worker are still open (BDF-0002), so neither the budgets
  nor the limits are guaranteed by the operating system today; they are checked by the code and
  polled.
- **A 251 MiB file is read whole** (260 MiB peak). The 250 MiB admission limit and the 2,000-page
  limit in the specification are not enforced by the engine today; BUILD-004 and BUILD-010 own that.
- **The 4-logical-CPU run matched the unrestricted run**, because rendering is single-threaded; it
  approximates the reference machine's core count only, not its clock speed or integrated graphics.

Not measured, and so not ratified: cold launch (needs a reboot), scrolling frame times, input
latency, recovery, a GPU-less machine, a 1920x1080 display, and any real-world document.
