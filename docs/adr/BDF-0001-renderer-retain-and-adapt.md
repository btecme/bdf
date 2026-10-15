# BDF-0001: Retain the inherited renderer and adapt it where measured

> **Status:** Proposed for owner approval · **Date:** 2026-10-15 · **Work item:** BUILD-002 · **Evidence:** [BUILD-002](../bdf-evidence/BUILD-002.md) sections 4 and 5

## Context

The PRD (BDF-01) requires a documented retain, adapt or replace decision for the renderer, resting on
measurement and not on feature counts. The renderer is `pdfcraft-render`, a thin layer over the
vendored, patched `hayro` rasterizer. It is single-threaded and CPU-only, and it is the only
renderer the desktop app, the CLI and the automation tools use.

## Evidence

Measured against MuPDF 1.28.5 (a separate process) and against what the corpus generator knows is in
each file, on a 100-case synthetic corpus, with tolerances fixed before the first comparison.

- **Semantic fidelity is complete on the corpus.** 306 of 306 checks pass: page counts, page sizes
  after rotation, crop box and user unit, text found on the right page (including Greek, Cyrillic,
  combining marks and a composite font), form fields, bookmarks, attachments, page labels, annotation
  and link counts. The engine reads encrypted files with all four encryption schemes and a Unicode
  password.
- **Visual fidelity is close where it counts.** 91 of 103 page comparisons are within tolerance. The
  passing ones differ from MuPDF by at most 0.08% of pixels and a mean error of 2.3 grey levels.
- **The 12 comparisons outside tolerance have four causes**, triaged by looking at both images:
  1. *Image downscaling uses point sampling*, not area averaging, so a 300 dpi scan shown at 96 dpi
     keeps speckle and a 600 dpi black-and-white scan shows jagged, aliased strokes (5 comparisons).
     This is a real defect for scans at reduced zoom.
  2. *Form fields without an appearance stream are not drawn* by the headless render, where MuPDF
     draws the box and value (1 comparison). A real defect: the field values are known to the
     engine and the form tools report them.
  3. *Thin strokes are drawn as solid one-pixel lines* where MuPDF draws them faint (2 plan-sheet
     comparisons). A policy difference, not an error; it makes dense drawings look heavier.
  4. *Borderline stroke placement* (3 comparisons) and a tiling-pattern phase that is regular in
     the engine and irregular in MuPDF (1): not material.
- **Robustness is good.** 0 crashes or hangs in 100 cases, including 11 damaged files and 13 hostile
  structures and decode bombs. The engine recovers 2 damaged files MuPDF refuses; it refuses 1 that
  MuPDF recovers (an invalid page reference at the head of a page list), a recovery gap.
- **Speed and memory are far inside the proposed budgets** (BDF-0003): 48 ms to the first page of a
  100-page office document, 2.35 s for the heaviest synthetic plan sheet.

## Decision

**Retain `pdfcraft-render`/`hayro` for the desktop v1 core, and adapt it in three bounded ways.** The
evidence shows no systematic fidelity failure in the core classes (text, vector, forms,
annotations, structure), so a replacement would be unjustified cost and risk.

Adaptations, each to be done as its own work item with the matching corpus case as the regression:

1. Downscale images by area averaging (or mip-mapping) when the destination is smaller than the source.
2. Draw a fallback appearance for widgets that have no appearance stream in the page render, as the
   interactive view already must.
3. Skip an invalid page reference during page-tree repair instead of giving up on the document.

Revisit the decision if the real-world pilot corpus (BDF-05, at least 500 cases) shows failures the
synthetic one could not: DCT, JPEG 2000 and JBIG2 images, CJK text, and complex Office exports are
not in the synthetic corpus.

## Consequences

- The oracle harness (`cargo xtask bdf-oracle`) and the corpus become the regression gate for any
  change to rendering.
- The two visual limits that sit near the noise floor (see BUILD-002 section 4.4) must be reviewed
  when budgets are ratified.
- Nothing here changes the headless engine's role as the single rendering path; the worker process
  (BDF-0002) calls the same `PageRenderer`.
