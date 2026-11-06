# BUILD-018 evidence: portable deployment, optional Windows integration, signed user-initiated updates

Status: **implemented 2026-11-06 on `feat/build-018-portable-updates` (version 0.8.0); tested locally on Windows 11 x64; not
hosted-verified, not independently reviewed.** A signed release has not been published and no production signing key exists.
The flows that need a disposable Windows machine were **not run** (section 4). Design and operator instructions:
[bdf-portable-updates.md](../bdf-portable-updates.md).

Machine: Windows 11 Pro 22631, standard (non-elevated) token, Rust 1.99.0, two build jobs. Synthetic documents and packages only;
no sensitive PDF was used. Nothing outside a temp folder, the repository and a scratch registry area was changed; the real
`HKCU\Software\Classes\BDF.Portable.Document` and `Software\BDF\PortableRegistration` keys were checked absent before and after.

## 1. What was run

| Check | Result |
|---|---|
| `pdfcraft-update` unit and loopback-HTTP tests (`--features net`) | 65 passed |
| `bdf-updater` (helper) tests, real packages, test trust root | 7 passed |
| `pdfcraft-platform` `windows_integration` tests (real registry API, scratch key area) | 9 passed |
| `pdfcraft-automation` `software` + `client_mcp` tests (authorisation boundary, grant cannot reach the tools) | 6 + 17 passed |
| `pdfcraft-automation` lib (capability inventory covers the new tools) / `pdfcraft-engine` lib | 251 / 38 passed |
| `pdfcraft-cli` tests incl. `tests/software.rs` | passed |
| `pdfcraft-ui-egui` `tests/updates.rs` (fake host: button, link, flow, unsaved work, Later, auto check) and lib (i18n coverage etc.) | 9 passed / 375 passed |
| `xtask` release tooling round trip | 2 passed |
| `packaging/windows/test-deploy.ps1` (Action1 script vs synthetic packages and fake profiles) | 25 checks passed |
| `packaging/windows/test-update-e2e.ps1` (real `bdf-cli`/`bdf-updater` binaries, local feed): see section 2 | see below |
| `cargo xtask assets` | ok |
| fmt / clippy `-D warnings` / `layers` / `wasm` / `parity` | see section 5 |

Tests that were seen failing for the right reason before the code that fixes them: the idle-window defect and the inherited-pipe
defect (section 3) were found by the real-window and real-binary runs, not by unit tests; the interrupted-update test runs the
swap to every possible step and checks the old version comes back byte for byte.

## 2. User flows, with real binaries (`test-update-e2e.ps1`)

Three builds of the same source (0.8.0, 0.8.1, 0.8.2) were made with a **development** signing key
(`BDF_UPDATE_CHANNEL=dev`), packaged with `cargo xtask portable-package`, signed with `update-sign`, and served from a local
feed. The result lines are in [BUILD-018-e2e.txt](BUILD-018-e2e.txt). They cover: extract and run as the standard user; a managed
copy with a trust key and nothing on offer; check, download (verified), apply, 0.8.0 to 0.8.1 with `BDFData`, a recovery file and a
user file byte-identical, previous version kept; a second successive upgrade to 0.8.2; rollback; a tampered package discarded;
a forged signature and an altered signed payload rejected with nothing offered; an older release in the feed rejected; offline
(the earlier offer still shown, a manual check fails cleanly, a PDF still opens and renders); an update that **waits while a CLI
job is running**, does not end it, and applies when it finishes; a copy without its package list reporting that it cannot update
itself.

Real window (control channel, dev build 0.8.0 against the local 0.8.1 feed): the green **Update** button and the
**b-tec Built** mark appear at the top right on their own after the automatic check; Help ▸ Check for updates shows the dialog
with the version, the release notes, a "development build" label and **Download update / Later**.

## 3. Defects found by this testing and fixed

1. `bdf-cli update apply` started the updater with the caller's output pipe inherited, so a harness reading the CLI's output
   waited until BDF itself quit. The updater is now started with no inherited handles (`CreateProcess`, `bInheritHandles = FALSE`).
2. An idle window never ran the automatic check (nothing woke it). The window now schedules a repaint for the next check. The
   unit test covers the schedule only; the idle wake was confirmed in the real window.
3. Script and test defects: PowerShell's `cli` alias shadowed a helper; ECDSA key construction needed a whole `ECPoint`;
   a test variable shadowed a parameter; the restarted BDF consumes the one-time result note by design (tests wait on the
   installed version instead).

## 4. Not verified: needs a disposable Windows machine (or the owner)

* **Register, then choose BDF in Default apps, then double-click a PDF opens BDF.** The registry writer is verified against a
  scratch key area and against the layout the MSI already uses; Windows' own handling of those keys, the Settings page and the
  double-click were not exercised, because that would change this machine's PDF associations. `bdf-cli windows open-settings`
  opens the real Settings page; it was not exercised either.
* **Deployment as Local System** (Action1) and **a second, standard user's profile**: the script was tested as the current user
  against fake profile folders (`-UsersRoot`): signature/package verification, per-user install, reruns, running-copy skip,
  default profile, junction refusal, `-WhatIf`. The Local System token, `icacls` ownership changes on real profiles and the
  profile-list discovery were not run (this shell is not elevated).
* Updating a real **GUI** copy through the Update button (the helper, the close flow and the restart are tested; the button path
  is tested against a fake host and the CLI path against the real helper).
* A power-cut style interruption of a real update (the swap is interrupted at every step in tests with the real filesystem, and
  recovery runs at the next start, but no real process was killed mid-swap).
* Disk-full and read-only folders: classified and unit-tested (a file standing in for the update folder; error mapping); no real
  full volume.
* Translations: 20 strings in 21 catalogues were written by the implementer and are **unreviewed**.
* Hosted CI, SmartScreen/Authenticode behaviour, MSI interplay, other operating systems.

## 5. Gates

Run locally on the final source (each gate is the repository's own command):

* `cargo fmt --check`: clean.
* `cargo clippy -D warnings --all-targets` on `pdfcraft-update`, `pdfcraft-updater`, `pdfcraft-platform`, `pdfcraft-automation`,
  `pdfcraft-engine`, `pdfcraft-ui-egui`, `pdfcraft`, `pdfcraft-cli`, `xtask`: clean.
* `cargo xtask layers`: 39 crates, no violations (`update` is L7 with sideways edges only from `ui-egui` and `automation`;
  `updater` is an exempt app). `cargo xtask assets`: ok (the 21 catalogue hashes updated). `cargo xtask parity`: ok after two
  evidence repairs: `view.check-for-updates` now cites the new tests, and `view.theme-system` cited a test renamed in PR #15
  (so that entry was already stale on `main`). `cargo xtask wasm`: ok (`pdfcraft-update` needed the same
  `getrandom/wasm_js` setting as `pdfcraft-sign`).
* Scoped test sweeps with `--no-fail-fast`: update, updater, platform, automation, engine, cli, xtask: no failure;
  `pdfcraft-ui-egui` and `pdfcraft` (every target): no failure after two repairs described below.
* Not run locally: `cargo xtask deny` and the full workspace sweep (CI's job).

Existing tests that had to change, and why:

* `automation/tests/automation.rs`: the default MCP server listed every tool; it now lists every tool except the ones that need
  the software authorisation (`listed_count()`).
* `ui-egui/tests/ui_group/links.rs`: the title bar now carries a "b-tec Built" mark (2 matches with the About note open, 1 on
  the home screen), and `BTEC_LINKS` has a third link (the BDF page).
* `ui-egui/tests/ui_group/updates.rs`: ported from the old GitHub-API `UpdateSource` to the new host; the "no check at start"
  test became "with automatic checks off nothing is asked until the user checks" (automatic checks are now on by default).
* `ui-egui/tests/ui.rs` (tab strip widths): passes unchanged once the strip reserves exactly the width of the mark and the
  Update button instead of a fixed margin.

## 6. Decisions made (routine, reversible)

Version 0.8.0; work item BUILD-018; per-user app location `%LOCALAPPDATA%\Programs\BDF` (Windows' per-user convention, and it
keeps the app away from the `%LOCALAPPDATA%\BDF` data folders); automatic checks default **on** (off switch in Preferences);
registration names `BDF.Portable.Document` / `BDF Portable` so it coexists with the MSI's `BDF.Document` / `BDF`; ECDSA P-256 so
the PowerShell deployment script can verify without running package code; one-file signed envelope; feed on GitHub Releases of
`btecme/bdf`; the updater waits up to 10 minutes for running programs and then gives up unchanged.
