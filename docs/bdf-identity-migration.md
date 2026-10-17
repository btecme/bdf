# BDF identity migration record

Status: **record and plan, 2026-10-17. Nothing is renamed by this page.** BUILD-003 deliverable 4: record the BDF display, support, update, settings and portable identity migration, without mass-renaming Rust packages.

The product today carries the upstream identity (PdfCraft, ArtCraft, `storytold`, "Learning Machines LLC"). A BDF build needs its own, but several choices are only the owner's to make, and some surfaces can break users' data or installs if changed carelessly. This page lists each surface as it is now, the rule for migrating it, and what is waiting on a decision. Where the owner has not decided, the page does not invent a value.

## 1. Rules

1. **Rust package and crate names stay** (`pdfcraft`, `pdfcraft-engine`, ...). They are internal, renaming them touches every file and every contributor's branch, and nothing a user sees depends on them. The `[[bin]]` names `pdfcraft` and `pdfcraft-cli` are user-visible commands and are decided in section 2.
2. **User-visible text comes from one place each**, so changing it is a small diff: the display name and links from `crates/engine/src/links.rs` and the window and About strings in `apps/pdfcraft` and `crates/ui-egui`; the installer's strings from the packaging files.
3. **Existing user data is moved, never abandoned and never silently imported from elsewhere.** The app already did this once, from PrintCraft to PdfCraft (`migrate_legacy_folders` in `apps/pdfcraft/src/main.rs`): settings and crash-recovery folders move once, only if the new folder does not exist, best effort, and a portable copy leaves per-user folders alone. A BDF migration follows the same shape: move or read the old PdfCraft folder once, with the user told in the log, and never copy credentials or tokens from other software (there are none to copy; section 3 of [network and activation](bdf-network-and-activation.md)).
4. **No support or update address is invented.** Until the owner supplies one, the product either keeps the upstream address or shows none; it does not show a made-up one.
5. **Do not change what an installer uses to recognise itself** (UpgradeCode, product and file-association ids) without deciding what happens to machines that already have the other product installed (section 2).

## 2. Surfaces, as they are now

| Surface | Today | Where | Migration rule and decision needed |
|---|---|---|---|
| Display name | "PdfCraft" (window title, About, tab titles, MCP `serverInfo`, PDF `Producer`, signature app name, comment author default) | `apps/pdfcraft/src/main.rs` (`with_title`, `run_native`), `crates/ui-egui/src/lib.rs` (`— PdfCraft` suffix), `dialogs.rs`, `crates/automation/src/mcp.rs`, `comments.rs`, `crates/create/src/lib.rs`, `crates/sign/src/pdf.rs` | **Owner:** the BDF product name. Then one pass over these strings. The PDF `Producer` and signature application name are written into users' files, so they change only with the display name, not separately. |
| Executables | `pdfcraft.exe`, `pdfcraft-cli.exe`, `pdfcraft-worker` | `apps/*/Cargo.toml`, `packaging/windows/pdfcraft.wxs`, `build.rs` (`ProductName`) | **Owner:** whether the command names change. If they do, keep the old name as an alias for one release so scripts and `App Paths` entries keep working. |
| Settings folder | `eframe::storage_dir("PdfCraft")` (the platform's per-user app-data location): `app.ron` and `logs`; legacy key `printcraft` | `apps/pdfcraft/src/main.rs` (`settings_dir`, `LEGACY_STORAGE_KEY`) | Move once to the BDF folder name when the display name is decided, using `migrate_legacy_folders`' pattern. Do not delete the old folder; leave it for rollback. |
| Recovery folder | `%LOCALAPPDATA%\PdfCraft\Recovery` (or the settings folder off Windows); legacy `PrintCraft` moved already | `crates/ui-egui/src/recovery.rs`, `apps/pdfcraft/src/main.rs` | Same move. Unsaved work in the old folder must be offered for recovery before it is moved or ignored. |
| Other local data | clipboard images `%LOCALAPPDATA%\PdfCraft\Clipboard`; single-instance socket `%LOCALAPPDATA%\PdfCraft\instance-*.sock`; OCR models `PdfCraft/models` (or `PDFCRAFT_MODELS`) | `crates/ui-egui/src/image_pick.rs`, `apps/pdfcraft/src/single_instance.rs`, `crates/ocr/src/lib.rs` | The socket is recreated on each run and needs no migration, but a BDF and an upstream PdfCraft that share its name would hand files to each other: the name must differ once the product name does. Models are re-fetchable; move or leave. Clipboard images are temporary. |
| Environment variables | `PDFCRAFT_RENDERER`, `PDFCRAFT_SYSTEM_FONTS`, `PDFCRAFT_MODELS`, and test and debug variables, all prefixed `PDFCRAFT_` | `apps`, `crates` | Keep the `PDFCRAFT_` names working. Add BDF-prefixed names only if the owner wants them, with the old ones as fallbacks. None carries a credential. |
| Portable mode | Marker file `portable.txt` or `PdfCraft.portable` beside the executable; data in `PdfCraftData`; the zip ships `portable.txt` | `crates/ui-egui/src/portable.rs`, `packaging/windows/portable.txt` | Keep accepting `portable.txt`. Accept a BDF-named marker alongside the old one; do not drop `PdfCraft.portable`. A portable copy never touches `%APPDATA%` or `%LOCALAPPDATA%`; the migration skips it (as `migrate_legacy_folders` does). |
| Windows installer identity | Product "PdfCraft", manufacturer "Learning Machines LLC", **UpgradeCode `EA00C367-7270-4C50-9D8B-A7A5C88AD27F`**, install folder `PdfCraft`, `HKLM\Software\Classes\PdfCraft.Document`, `Software\PdfCraft\Capabilities`, `App Paths\pdfcraft.exe` | `packaging/windows/pdfcraft.wxs` | **Owner, and the hardest one.** A BDF MSI that keeps this UpgradeCode upgrades or replaces an upstream PdfCraft install; one with a new UpgradeCode installs beside it and both claim `.pdf` Open With and the same `pdfcraft.exe` App Path. Decide: coexist (new UpgradeCode, new ids, a different exe name) or replace (same UpgradeCode, with a downgrade policy). Manufacturer is the owner's legal publisher name; it is not guessed here. |
| Linux, macOS, FreeBSD, Flatpak ids | `ai.storyteller.pdfcraft` (desktop file, AppStream id, MIME, StartupWMClass), macOS bundle from `Info.plist.in`, `nfpm.yaml` | `packaging/linux`, `packaging/macos`, `packaging/freebsd` | **Owner:** the reverse-DNS id BDF will use. The id is also the Flatpak and icon file name; `packaging-lint` checks they agree, so a rename that misses one fails CI. BDF's first target is Windows 11 (build spec), so these wait. |
| Icons and assets | The app icon is under `assets/app-icon` and must satisfy `AGENTS.md` section 1 (openly licensed or original) | `ATTRIBUTION.toml`, `cargo xtask assets` | **Owner:** a BDF icon. Do not reuse upstream branding that the asset policy excludes. `cargo xtask assets` enforces the entry. |
| Support and help links | Discord (ArtCraft), `getartcraft.com/apps/pdfcraft`, `github.com/storytold/pdfcraft`, `getartcraft.com` | `crates/engine/src/links.rs`, also used by the About dialog, home screen, CLI and README | **Owner:** BDF's support address, or none. Until then: no invented address. If none is given, remove the Help links rather than leaving the upstream community as BDF's support channel. |
| Update source | `https://api.github.com/repos/storytold/pdfcraft/releases/latest`, release page `https://github.com/storytold/pdfcraft/releases`; user-initiated only | `apps/pdfcraft/src/updates.rs`, `crates/ui-egui/src/updates.rs` | **Owner:** the BDF releases location (for example `btecme/bdf`), once there is a release to find. Until a BDF release exists, an update check can only report "none" or point at someone else's builds; leaving the command out of the BDF menu is safer than a check that offers upstream's installer. The parse and the host-allowlist logic (`only a page under RELEASES_PAGE is offered`) need no change beyond the constants. |
| Licence and credits | Upstream copyright and licence kept; credits generated by `crates/ui-egui/build.rs` | `LICENSE*`, `ATTRIBUTION.md` | Keep upstream credit as plain text. Add the BDF owner's line when the owner states the form of it. |
| Signing identity | None for BDF. The upstream pipeline's signing and notarisation identity is not used | `docs/releasing.md`, `release.yml` (unsigned, no secrets) | **Owner:** BDF's certificate, later. Do not configure secrets or push a `release` branch until then. |

## 3. Order of work (when the owner decides)

1. Product name, command names, publisher, support address (or none), update location. These are inputs, not code.
2. One change of the user-visible strings and links (display name, About, window title, `links.rs`, `updates.rs` constants), with the tests that pin them updated in the same commit.
3. The one-time folder move for settings and recovery, with a test that an existing PdfCraft folder is read or moved once, that a portable copy is untouched, and that nothing outside those folders is read.
4. The installer: ids, UpgradeCode decision and its upgrade test (`install-test.ps1` already installs, runs and uninstalls on a hosted runner).
5. The Linux and macOS ids and packaging, when those targets are in scope.

## 4. What this record does not do

It does not rename anything, choose a product name, create a GUID, or add a link. It does not claim the migration is safe: the folder move has a precedent in the code but has not been exercised against a BDF name, and the installer decision has no test yet.

## Revision history

| Date | Change |
|---|---|
| 2026-10-17 | First version (BUILD-003 deliverable 4). |
