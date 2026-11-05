# BUILD-015 — Codex review, 2026-11-05

Status: R15-1 and R15-2 closed in PR #15; the panel-side regression R15-3 is
corrected in this follow-up. Local correction checks pass. Merge requires green
checks on the final head and the matching package UI check. Final hosted and
integration status: [PR #15](https://github.com/btecme/bdf/pull/15).
Initial reviewed branch: `feat/build-015-bdf-branding`, head
`0bf65dfc14305c57d23a162044269847a1e3d78d`, against `3744d28c`.
The initial staged executable source was `63596d5c`; the subsequent commit changed
two documentation files only. The initial pass made no implementation changes.
The PR #15 retest below covers `2af4a8df` and the Codex layout correction added
on the owner's instruction to review, commit, push and merge. No local installation
or default-handler registration is authorized or performed.

## Initial branding findings — closed in PR #15

These findings describe the initial `0bf65dfc` review and `63596d5c` preview.
CC corrected both before the `2af4a8df` PR #15 head; their original descriptions
are retained for the review trail.

### R15-1 — P2: CLI help still sends people to the upstream Discord

`apps/pdfcraft-cli/src/main.rs:82` retains
`help and feedback: https://discord.gg/artcraft` in `USAGE`. This is visible in the
actual staged `bdf-cli.exe --help`, despite the updated version output and UI links.
It contradicts the branding spec and the residuals document's claim that support
links belong to BDF. Replace the line with the BDF issue destination, cover the
complete help output with the existing help test, and rebuild the preview.

### R15-2 — P2: macOS package still ships the old visible identity

`packaging/macos/package.sh:138` copies `dmg/background.tiff` into the installer.
Visual inspection of that committed raster confirms the old lion and the title
`PdfCraft`. Updating `background.svg` alone does not update the packaged raster.
Regenerate the raster from the BDF source and refresh its attribution hash, or
explicitly exclude that stale artwork from packaging. This does not block private
Windows preview testing; it does prevent accepting the assignment as branding
completed everywhere. The existing generator says it runs on any OS, so a macOS
machine is not inherently required to regenerate this asset.

## Independent results

- All three staged executable SHA-256 values match `BUILD.json`. Each Windows
  resource reports ProductName `BDF` and its correct `bdf*` OriginalFilename.
- The staged CLI version output identifies BDF's repository and issue tracker.
- An isolated, grantless MCP initialization reports name `bdf`, title `BDF`, and
  serves help only. No grant was issued and no server was registered.
- The existing 16-step synthetic office CLI workflow passes: search, rotate,
  undo/redo, note, form fill, page reorder, save, close/reopen and render. Readback
  confirms the saved field value, annotation and page rotations. The rendered
  page was visually inspected. This checks the CLI/engine, not the native picker.
- A separate new text PDF reports Producer `BDF`. An additional synthetic
  open/edit/save/reopen check preserves an existing `PdfCraft synthetic provenance`
  Producer and the word `PdfCraft` in the document's text; branding does not
  rewrite that user-document content.
- Independent hashing of all 260 local manifest assets finds no hash mismatch or
  missing listed license file. This is not a substitute for every `xtask` gate.
- Scoped desktop/platform/worker tests pass: 136 passed, one ignored (desktop
  65/1, platform 62/0, worker 9/0). The offline two-job run completed in 596.7
  seconds, including compilation. Formatting passes. Both targeted UI tests
  of the cloud-provider message also pass (255.3 seconds including compilation):
  total 138 passing tests, one ignored network test.
- After the owner opened the app, actual Home, Help and About inspection confirms
  BDF titles and links, with the plain-text upstream credit preserved. The native
  Windows picker opens and renders the local synthetic office PDF; the document
  window is titled `office.pdf — BDF`. The picker uses the correct PDF filter.
- The owner selected and approved a OneDrive PDF for testing. The native picker
  reported `Available when online` and its Open button was enabled. Codex
  confirmed Open, observed the dialog close, and verified that the BDF document
  title matched the approved selection, printing became available, and no read
  error or loading state remained. No document screenshot was taken. Reopening
  and cancelling the picker left that document active. Its filename and contents
  are excluded from the review and raw result summary.

## Open-dialog disposition and remaining checks

The code change adds actionable text for Windows error 362, a stopped cloud sync
provider. It does not change `rfd` or the native picker's options. CC records a
useful comparison matrix, including other native dialogs and successful cloud
opening after the provider connects. That supports CC's diagnosis for the tested
provider state; this review has not independently reproduced that matrix.

The owner's earlier report that the same files opened in another reader remains
relevant. The earlier Codex inference that the failure must be BDF-specific was
too strong. Do not promise that the new error text itself enables a disabled
native Open button or resolves every cloud-selection failure.

Desktop-control launch approval initially timed out. The owner then opened BDF
and confirmed the desktop was unlocked; the GUI checks above followed through
the computer-use skill. The owner subsequently approved the cloud sample and the
single-file native opening/cancel check above passed. This confirms the current
working-provider case; deliberately stopping the provider and multiple-cloud-file
opening were not independently retested. Private document appearance/fidelity was
not evaluated. No private filename or document content is included in this
evidence. MSI build/install and non-Windows packaging remain unverified. No PR
for this head or PR-triggered hosted workflow run was found at review time.

## Initial next-delivery request (superseded by the retest below)

Correct R15-1 and the packaged artwork, rebuild the BDF preview, and open the
focused PR so hosted CI can run. Preserve the passing native local and single
cloud-file opening/cancel checks; multiple-cloud-file opening remains an
independent verification gap.
Preserve local opening and the existing data/privacy boundaries. No new cloud
framework or backend migration is needed for this correction.

Raw local results: `target/bdf-evidence/2026-11-05-review/` (ignored; synthetic
content and a filename-free native-check summary only). Preview:
`C:\Users\btec\Documents\GitHub\bdf\target\bdf-preview-63596d5c\bdf.exe`.

## Distribution follow-up, 2026-11-05

Direct PE import-table inspection of the three staged executables confirms x64
architecture (`0x8664`) and an external `VCRUNTIME140.dll` import in each. The
preview folder contains no copy of that runtime. Do not describe this exact
preview as guaranteed to run on a clean Windows 11 PC without prerequisites.
The Windows package script already requests `+crt-static`; use that path for the
next testing ZIP, inspect the resulting imports, and qualify it on a clean PC/VM.

Distribute a clean folder containing the three matching executables, portable
marker, instructions/build identity and applicable license/notice files. The
currently staged preview lacks the adjacent license/notice files. Exclude the
owner's `BDFData`; each tester should get fresh settings/recovery data. Portable
mode still uses small single-instance socket/lock files under `%LOCALAPPDATA%\BDF`.
No source checkout, Rust toolchain, Git or coding-agent installation is required
for ordinary GUI use. OCR models are absent and OCR is outside this core preview.

## PR #15 retest and layout correction, 2026-11-05

Reviewed CC's `2af4a8df` against the prior `0bf65dfc` review. All five hosted
workflows passed on `2af4a8df`: CI, FreeBSD, Windows ARM64, Windows x64 unsigned
packaging, and packaging lint. The x64 package job includes real install/run/
uninstall checks; its package remains unsigned. The code correction below needs
fresh final-head checks before merge.

- **R15-1 closed:** the actual `2af4a8df` CLI help names only the BDF repository;
  no upstream Discord remains. The CLI regression test covers that output.
- **R15-2 closed:** visual inspection of the regenerated committed DMG raster
  confirms BDF artwork and title; the Finder layout was regenerated too. All
  260 local asset hashes and listed license-file paths independently check out.
- Native inspection of an isolated `2af4a8df` preview confirms dark first launch,
  Inspector/icon rail left, Tools right, and the b-tec Built About note. Default
  settings save `theme_choice: null`; explicitly selecting Light survives a full
  exit/restart. The synthetic document survives rotate/undo/redo/save/reopen.
- **R15-3 found and corrected here:** switching Tools left then right produced
  `Inspector, Tools, Canvas`, while reporting Tools as right. The nearest split
  neighbor is not necessarily the document. Inspector rail side had the same
  ambiguity. Queries now compare with the Canvas in a bounded layout probe,
  including saved hidden panels, without changing the live layout or visibility.
- Both new regression tests failed against the original implementation (wrong
  order and rail side). With the correction, 375 UI unit tests pass, one is
  ignored; the actual `ui_group` theme tests (7), link tests (10), and scroll-
  preservation test (1) pass. Total: **393 passed, one ignored**. Scoped UI
  Clippy with all targets and `-D warnings`, and workspace formatting pass.
  Layers (37 crates), assets, and the scoped UI WASM compilation check also pass.
  The WASM build emits platform-specific warnings outside the correction.
- The earlier local CLI test compilation was stopped as redundant; the staged
  CLI output was checked directly and `2af4a8df` has hosted CLI coverage. It is
  not represented as a local test failure or as a completed local CLI suite.

No new dependencies, assets, network startup, MCP/control defaults or document
permissions were introduced by the correction. The links open only on explicit
activation. Native testing used an isolated synthetic-document/settings folder;
the owner's existing BDF session and private PDF were left alone.

Remaining broad release gates (including signing, agent-service migration and
pilot qualification) stay open. Merge acceptance is for this identity/UI slice.
Raw retest results: `target/bdf-evidence/2026-11-05-pr15/`. That is also the evidence location for the matching final-head package probe and
merge result; the PR checks are authoritative for final hosted status.

## Revision history

| Date | Change |
|---|---|
| 2026-11-05 | Independent source/artifact review, two branding corrections, bounded smoke checks and explicit native/hosted verification limits. |
| 2026-11-05 | Owner-approved online-only OneDrive PDF opens through the native picker; Cancel preserves the active document. |
| 2026-11-05 | Inspect actual preview runtime imports and document the clean testing-ZIP requirements. |
