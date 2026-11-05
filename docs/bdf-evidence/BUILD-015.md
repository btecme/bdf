# BUILD-015 — BDF product identity: implementation and verification record

Current review: [Codex correction retest](BUILD-015-codex-review.md), [PR #15](https://github.com/btecme/bdf/pull/15). Both initial branding findings are closed. The panel-side follow-up has regression proof and passing local checks; final-head CI and package UI verification gate integration. The author record below describes its original verification state.

Status: implemented and locally verified on Windows 11 x64 (2026-11-04). Hosted CI has not been read and no
independent review has run. Branch `feat/build-015-bdf-branding`, stacked on `fix/open-cloud-provider-hint` (the
Open-dialog finding, [windows-open-picker-issue.md](windows-open-picker-issue.md)). Spec:
[../bdf-branding-build-spec.md](../bdf-branding-build-spec.md). What still carries an inherited name, and why:
[../bdf-branding-residuals.md](../bdf-branding-residuals.md).

Distinguish: implemented (all of it), locally tested (the rows below), hosted-green (not yet), independently accepted
(not yet).

## What was run, and what it showed

| Check | Result |
|---|---|
| Release build of `bdf.exe`, `bdf-cli.exe`, `bdf-worker.exe` from the committed tree, `--locked`, `PDFCRAFT_REQUIRE_WINRES=1` | built |
| Executable properties (`VersionInfo`) of all three | ProductName `BDF`; descriptions "BDF PDF workbench", "BDF command-line tool", "BDF document worker"; OriginalFilename and InternalName `bdf*`; version 0.7.1; copyright keeps the legal names ("ArtCraft Team and the PdfCraft contributors; BDF changes by the BDF contributors") |
| `bdf-cli --version` | `bdf-cli 0.7.1`, then `Source:` and `Issues:` at `btecme/bdf` (no Discord line) |
| Plain portable launch (`portable.txt`, no arguments) | window title `BDF`; `BDFData\logs\bdf.log` created, `app.ron` at exit; **no listening TCP socket and no UDP endpoint**; no `PdfCraftData`; only `%LOCALAPPDATA%\BDF\portable-instance.*` outside the folder |
| Launch with `--control FILE` (opt-in) | one loopback listener appears; the control channel drives the checks below |
| Installed-style launch (no marker) | `%APPDATA%\BDF\data\logs\bdf.log`; `%LOCALAPPDATA%\BDF\instance.*`; an existing `%LOCALAPPDATA%\PdfCraft` on this machine was not touched (last write unchanged) |
| About window (rendered by the app) | "BDF", "Version 0.7.1", the credit "Based on PdfCraft by the ArtCraft team.", pills "BDF on GitHub" and "Report an issue"; Home reads "Welcome to BDF"; no Discord control anywhere in the top bar or Home |
| Window titles | `BDF` at Home, `office.pdf — BDF` with a document open |
| Open, rotate, undo, redo, save, close, reopen (synthetic PDFs made by `bdf-cli`) | first page 612×792 before, 792×612 in the file the app saved (read by a separate `bdf-cli` process); reopened; the saved file's size grew by an incremental update |
| Native Open dialog (real `#32770` dialog of the new `bdf.exe`, driven through Windows UI Automation and window messages) | local single selection opens; two local files open together; Cancel leaves the app unchanged; online-only OneDrive PDF typed into the dialog's File name box opens and is downloaded (online-only count 6 → 5); two online-only PDFs together open and download (5 → 3); Cancel on the OneDrive location is clean |
| Scoped tests, `-j 2`, `--no-fail-fast` | `pdfcraft-engine` 250 + 9; `pdfcraft-ocr` 9; `pdfcraft-platform` 62; `pdfcraft-automation` 38 + 163 + 8 + 16 + the rest; `pdfcraft` (app) 65 + 10; `pdfcraft-cli` all targets; `pdfcraft-worker` all targets; `xtask` 105; `pdfcraft-ui-egui` 1112 passed after the two tests that named the removed Discord button and the old home links were rewritten (they failed first, as they should) |
| Gates (CI-exact, run locally) | `cargo fmt --check`; `cargo clippy --workspace --all-targets -- -D warnings` clean; `xtask layers` (37 crates, no violations); `xtask assets` (260 repository assets, hashes updated for the edited catalogues); `xtask wasm` (32 crates); `xtask parity --partial` reports |

New and changed tests that prove the behaviour: `ui_group/links.rs` (no Discord, the home card and About open only the
BDF repository and issues, no restricted marks), `ui.rs` tab-strip bounds (against the remaining right-hand buttons),
`links.rs` unit test (every link under `github.com/btecme/bdf`), `apps/pdfcraft/src/updates.rs` (a release page of
another project is never offered; the 404 text), `version_resource.rs` (ProductName BDF, the three `bdf*` names), the
MCP `serverInfo` name `bdf`, and the `open_worker` tests of the Open finding.

## Not verified here, and why

* **Hosted CI and the pull request.** This machine has no `gh` and the git credential must not be used to call the API
  (the harness refused it), so CI results were not read and no PR was opened. Push and compare URL:
  `https://github.com/btecme/bdf/compare/main...feat/build-015-bdf-branding`.
* **The MSI.** WiX is not installed here and installing a global tool was not authorised, so `bdf.wxs` and
  `test-msi.ps1` (which now asserts the new UpgradeCode, the BDF manufacturer and the worker) were edited but **not built
  or run**, and no installation, upgrade, uninstall or default-handler test happened. Static package checks would not
  count as an installation test either way.
* **Row selection of online-only items in the new executable's dialog.** The sessions' desktop was locked, so the
  list could not be clicked; the rows were exercised with the earlier build (same picker code, unchanged): Open enables
  for one and for three online-only rows and opens them (windows-open-picker-issue.md). The new build was driven by typing
  names, which uses the same native dialog and the same download.
* **Desktop screenshots of the real window.** Taken from the app's own renderer through the control channel (system-font
  fallback was on; the screenshots here carry only synthetic content). A desktop capture showed the lock screen.
* **Non-Windows packaging** (Linux, FreeBSD, macOS, Flatpak): only the binary name and visible names follow BDF; not exercised.

## Revision history

| Date | Change |
|---|---|
| 2026-11-04 | Record of the BUILD-015 implementation and its local verification. |
| 2026-11-05 | Review corrections (CLI help Discord line, Linux metadata, DMG artwork regenerated with resvg 0.49.0) and the owner-requested "b-tec Built" About note with its two links; scoped tests (engine, automation, ui-egui 1112 + the new About and Help link tests, cli help), clippy, fmt, layers, assets and wasm re-run green locally. Hosted CI not read. |
