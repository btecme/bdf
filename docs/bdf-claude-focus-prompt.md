# Claude Code — BDF Windows preview delivery

Status: proposed owner handoff, updated 2026-11-04. This focuses the next delivery;
it does not certify a build, waive repository rules, authorize a release, or send
instructions to another chat automatically.

## Prompt

Our immediate objective is a Windows 11 x64 desktop preview the owner can use this
week. Proposed checkpoint: Friday, 2026-11-06. Deliver a concrete app, a launch path
and a short test checklist. The later v1 release still needs its agreed gates.

Read AGENTS.md and CLAUDE.md. Retain the agreed Rust/egui architecture, local-first
behavior, core office scope and option A for service migration. Keep API/MCP parity
and the common authorization boundary as release requirements. Web/cloud and the
reusable maintenance/support module remain later work.

Starting point: commit `aee35194`. Codex has built a local portable preview and
checked native open/render, form entry, rotate/undo/redo, save and reopen, plus a
headless office workflow. Read `docs/bdf-evidence/2026-11-04-progress-check.md` for
actual results and limits. Use the provided candidate and evidence; do not repeat
unchanged checks or restart the foundation. The missing desktop workflow checks
and any defects they reveal are the next bounded work.

Immediate preview blocker: read `docs/bdf-evidence/windows-open-picker-issue.md`.
Selecting online-only PDFs in the OneDrive shell view disables native Open, while
opening the local fixture through the native picker now passes. Correct and qualify
that user-visible cloud-selection case before the branding pass. Do not substitute
path/control-channel tests for the actual native dialog.

Next implementation priority: complete `docs/bdf-branding-build-spec.md`. The owner
confirmed on 2026-11-04 that this product is BDF throughout its visible identity,
executables, storage and packaging. This identity pass precedes another backend
catalogue group. Preserve required legal credits and historical/dependency accuracy.

Take ownership of ordinary engineering choices, branch housekeeping, CI repairs,
error handling and compliant dependencies. Ask the owner only when the answer
changes product behavior, scope, cost, external commitments or a safety guarantee.
Explain the user impact and recommend one choice; do not ask them to select APIs,
Rust crates or test commands.

For this checkpoint:

1. Freeze a Windows preview candidate at a named commit. Include the latest reviewed
   save fixes. Finish only changes needed to make the candidate work reliably; avoid
   starting another catalogue-wide migration or framework expansion first.
2. Build and stage a portable preview in its own folder, with isolated settings and
   recovery data. Provide the executable/ZIP path, commit, checksum and launch steps.
   A public installer, signing, installation/default-handler registration and broader
   platform qualification are separate from this private preview.
3. Exercise the actual desktop: open/navigate/zoom/search; add a highlight and note;
   fill an existing AcroForm; reorder/rotate pages; undo/redo; save a copy; close and
   reopen it. Check content, field values, annotation placement and page order.
   Check print preview; do not claim a physical printer test without doing one.
4. Exercise failed saves and external file conflicts without losing the working edit
   or original file. Keep recovery usable. A false save success, lost work, corrupt
   output or reproducible crash blocks the preview. Record cosmetic/secondary gaps
   concisely rather than starting an unrelated redesign.
5. Use synthetic documents first and copies of representative office PDFs supplied
   for testing. Record which desktop actions were actually exercised; passing engine
   or MCP tests alone does not qualify the GUI.
6. Run scoped bounded tests and hosted CI against the exact candidate. Open/update
   the focused PR and keep its base/head accurate. Claude implements; Codex reviews.
   Do not merge, publish or install/register anything without separate authorization.

Do not represent the preview as an agent-ready release. The old desktop/CLI routes,
remaining Core service operations and job supervision are still migration work.
Keep MCP/control opt-in and out of the ordinary preview launch. Do not weaken
confinement, permissions, private staging, atomic writes, audit, revocation or other
agreed gates to reach a deadline. Do not enable an agent route that bypasses policy.
A person can evaluate the local desktop before the entire agent surface is released.

The next report should contain: the runnable artifact and exact commit; the desktop
checklist with pass/fail/not tested; any blocker stated in user terms; and the next
bounded action. Link technical logs instead of narrating implementation discoveries.
Keep mutation testing to the targeted regression proof the repository requires;
avoid blanket campaigns and repeated evidence-only cycles.

## Revision history

| Date | Change |
|---|---|
| 2026-11-04 | Replace stale BUILD-004 assignment with a bounded, testable Windows preview checkpoint; preserve v1 safety and service-migration requirements. |
| 2026-10-18 | Initial product-focused operating agreement and bounded BUILD-004 correction prompt. |
