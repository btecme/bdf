# BUILD-001 evidence: fork baseline on Windows 11 x64

Status: **evidence recorded, baseline not green, pending Codex review.** Run on 2026-10-13 (UTC-5). This is the report for work item BUILD-001 in [bdf-work-items.toml](../bdf-work-items.toml); the work item is `pending_review`, not `verified`. Raw logs, renders and screenshots are unpublished and live under the git-ignored `target/bdf-evidence/BUILD-001/` on the test machine. Paths below are relative to it. Hashes are SHA-256.

## 1. Verdict

| Question | Answer |
|---|---|
| Does the baseline build on Windows 11 x64? | **Yes.** `pdfcraft.exe` and `pdfcraft-cli.exe` build with `--locked -j 2`, with no compiler warnings, on the baseline plus the pending owner changes. |
| fmt, clippy `-D warnings`, layers, assets, parity | **All pass** for the scope listed in section 4. |
| Do the scoped tests pass? | **No.** `pdfcraft-ui-egui` has 13 tests that failed in all four runs and 3 more that failed intermittently (16 distinct). 14 of them failed at the pristine baseline commit with none of the pending changes. 1,195 tests passed. The cause is tests that wait too few frames for the asynchronous open worker. Nothing was weakened or deleted. See section 7. |
| Headless smoke (create/open, render, rotate, undo/redo, save, reopen) | **Pass**, including pixel checks and a rerun with the pinned toolchain. |
| Real desktop with the opt-in control channel | **Pass**: inspect, About dialog, open, rotate, undo, redo, save, screenshots, clean quit. |
| Branding cleanup | **Verified** by build, tests, `cargo xtask assets`, inspection of the live UI and review of the other screenshots. Residual upstream references are listed in section 6. |
| Hosted workspace results | **Unavailable for the fork** (it has no workflow runs). Upstream's run on the baseline commit **failed** `test (windows-latest)`, `test (ubuntu-latest)` and `freebsd`. See section 8. |
| Ready for BUILD-002? | **Conditionally.** See section 11. |

## 2. Baseline and tested state

- Repository `btecme/bdf` (`origin`); upstream is `storytold/pdfcraft`. Baseline commit `7437bde378f43856eada40af25b983812e615e8a` (`v0.7.1-65-g7437bde3`), on `main`. No `git fetch`, merge, push or publication happened (the only network reads were the toolchain download and read-only calls to the public GitHub API).
- The worktree was not clean. At the start 23 paths differed from HEAD: 9 modified files (4 in `crates/ui-egui`, plus README, ROADMAP, `ATTRIBUTION.md`, `ATTRIBUTION.toml` and `docs/roadmap.md`), 9 deleted files (8 ArtCraft logo/mark files and `docs/images/pdfcraft-tools.png`) and 5 untracked `docs/bdf-*` files. Nothing was reset or stashed. The pre-edit state is recorded as a diff (`7437bde3/00-worktree-tracked.diff`, SHA-256 `00986634a3c6977dcb6ea3c2205ce1261eb6b673e0474a460781ec0edee9b072`), per-file hashes (`7437bde3/00-modified-file-hashes.txt`) and a Git tree id computed with a throwaway index: `8d5ee638ecc16f13b8a410f25d70bb2b985a9778`.
- **Tested tree** `fa724bcbdddf10ce17c58e7cee78c878f24de981` = that snapshot plus one README fix made during this task (the removed screenshot left a dangling table caption cell). Every check in sections 4 to 6 except the pin checks ran on this tree.
- Task branch `chore/build-001-baseline`, created from `7437bde3`:
  - `83973357` commits the tested tree as it was (`git rev-parse 83973357^{tree}` = `fa724bcb…`).
  - `c0b95b00` adds the toolchain pin (section 3), `ci.yml` alignment and a development note. Revert it alone to go back to `stable`.
  - The commit that adds this file records the evidence, work-item status and roadmap lines.
- Changes outside the repository: rustup 1.29.1 and the `stable` toolchain were installed per user under `%USERPROFILE%\.cargo` and `.rustup` after approval (`rustup-init.exe` SHA-256 `6f4bef66…db7e` matched the value published by static.rust-lang.org; the user PATH was **not** modified). Creating `rust-toolchain.toml` made rustup auto-install a second copy of the same release as `1.99.0`; that second download was not separately approved. One 0-byte file, `%LOCALAPPDATA%\PdfCraft\portable-instance.sock`, was left behind by the desktop run and could not be deleted (section 9). The control directory ACL was restricted on a directory this run created.

## 3. Environment and toolchain

| Item | Value |
|---|---|
| OS | Windows 11 Pro 10.0.22631 (23H2), x64 |
| Hardware | AMD Ryzen 5 5600X (6 cores, 12 threads), 15.9 GB RAM, NVIDIA GeForce RTX 3060 (driver 32.0.16.1088, 2026-07-21) |
| MSVC | VS Build Tools 2022 17.14.36717.8 (toolset 14.44.35207) and VS Enterprise 2022 17.4.33213.308 (14.34.31933), both with VC tools; which one the linker picked was not recorded |
| Windows SDK | 10.0.26100.0 and 10.0.22000.0 |
| Rust | rustc 1.99.0 (b940084d7 2026-09-28), cargo 1.99.0, host `x86_64-pc-windows-msvc`, components clippy and rustfmt, rustup 1.29.1 |
| Pin | `rust-toolchain.toml` channel `1.99.0` (same compiler commit as `stable` at install time); `ci.yml` `toolchain:` set to 1.99.0 in its three places |
| Others | git 2.56.0.windows.1, Python 3.14.0 (stdlib only, run with `-I`) |
| Not set | `CRAFT_FONTS_DIR` (optional font input absent); `PDFCRAFT_SYSTEM_FONTS=0` for the desktop run |

Files: `7437bde3/01-environment.txt`. The `rust-version = "1.95"` in `Cargo.toml` was **not** tested: no toolchain older than 1.99.0 was installed.

## 4. Commands, exit codes and results

All commands ran from the repository root with `--locked` where Cargo accepts it, `-j 2` for builds, and a finite timeout enforced by a wrapper that kills the process tree. Full logs and the wall-clock table are in `7437bde3/` (`commands.tsv`). Toolchain for rows 05 to 81: `stable` (rustc 1.99.0); rows 90 to 97: the pinned `1.99.0` toolchain (same compiler commit).

| Log | Command | Exit | Result |
|---|---|---|---|
| 05 | `cargo fmt --all -- --check` | 0 | clean (8 s) |
| 10 | `cargo build --locked -p pdfcraft -p pdfcraft-cli -j 2` | 0 | finished in 20 m 27 s from a cold `target/`; 0 warnings |
| 20 | `cargo xtask layers` | 0 | layering table printed, no violation |
| 30 | `cargo xtask assets` | 0 | `261 repository assets, 6 bundled by dependencies, 22 fetched at build time` |
| 31 | `cargo xtask assets --write` | 0 | regenerated `ATTRIBUTION.md`; SHA-256 before and after `97288a11407d03d72f31f79729585fcff9ae8f9af5e37e3b37c6b31f9bca3cef`, identical |
| 32 | `cargo xtask assets` | 0 | same result as 30 |
| 40 | `cargo xtask parity` | 0 | 836 features (813 counted): 429 shipped, 89 partial; the tool also notes 96 shipped features without an automation tool (upstream state) |
| 50 | `cargo clippy --locked -p pdfcraft-ui-egui -p pdfcraft -p pdfcraft-cli -j 2 --all-targets -- -D warnings` | 0 | 0 warnings, 0 errors (2 m 20 s) |
| 60 | `pdfcraft-cli tools` | 0 | 163 tools; schemas used to write the smoke scripts |
| 70 | `cargo test --locked --no-fail-fast -p pdfcraft-ui-egui -p pdfcraft -p pdfcraft-cli -j 2` | **101** | 1,195 passed, **16 failed**, 6 ignored (4 need private PDFs, 1 a theme probe, 1 network) |
| 71 | the six failing test binaries, rerun | **101** | 14 failed |
| 72 | the same binaries with `--test-threads=1` | **101** | 13 failed |
| 80 | the same binaries on the **pristine baseline** commit (detached worktree, shared target dir) | **101** | 14 failed, all also failing on the pending tree |
| 81 | experiment on that pristine worktree: bounded wait for the open worker in two tests | 0 | both pass (section 7); worktree deleted, diff kept in `81-experiment-baseline-bounded-wait.diff` |
| 90 | `cargo fmt --all -- --check` under the pin | 0 | clean |
| 91 | `cargo build --locked -p pdfcraft -p pdfcraft-cli -j 2` under the pin | 0 | finished in 3 m 18 s |
| 92 | headless smoke A and B rerun with the pinned binary | 0 | PNG hashes identical to the first run |
| 93 to 97 | `cargo fmt --check`, `cargo xtask layers`, `assets`, `parity` rerun on the final tree under the pin | 0 each | same results as rows 05, 20, 30 and 40 (`ATTRIBUTION.md` unchanged, `git diff --check` clean) |

Not run, so **not passed**: `cargo xtask wasm` (the `wasm32-unknown-unknown` target is not installed); `cargo xtask deny` (`cargo-deny` is not installed); the full workspace test sweep (CI's job, per CLAUDE.md); tests of the other 28 crates; release-profile builds; the build with `CRAFT_FONTS_DIR` set (the font-gated tests run without the fonts and pass without exercising them, for example `arabic_ui_text_uses_craft_fonts`); `cargo xtask fuzz`, `demo-pdf` and `screenshots`; the `1.95` MSRV.

## 5. Headless smoke (existing CLI schema)

- Fixture `fixtures/make_fixture.py` (appendix) writes a 2-page synthetic PDF with no external assets: page 1 is US Letter with red, green, blue and yellow quadrants (top-left, top-right, bottom-left, bottom-right) and Helvetica text; page 2 is 420 x 300 with a grey box. `fixtures/smoke.pdf` = `3b5c990784070ef51948aa01f39360f6ca1fea8ebeb6823b6eaa6b85589b47ae`.
- Script A (`smoke/script-a.json`, `pdfcraft-cli run --script script-a.json --root .`, exit 0): `doc_open`, `doc_info`, `page_render` 72 dpi, `page_rotate` page 1 by 90, `page_render`, `edit_undo`, `page_render`, `edit_redo`, `page_render`, `doc_save` to a new path, `doc_list`, `doc_close`. Every step ran through the real CLI; stdout JSON is in `smoke/out/script-a.stdout.json`.
- Script B reopens the saved file (`doc_info`, both pages rendered, `text_extract`); script C creates a blank 2-page document (`doc_create`), rotates page 2 by 270, saves, closes, reopens and renders; script D opens the file the desktop saved (below). All exit 0. Two mistakes in my first B and C scripts (`page` instead of `pages` for `text_extract`; document ids are not reused after `doc_close`) exited 1 and were rerun after the fix; the first-run logs are kept in `smoke/out/run1-script-typos/`.
- `page_render` output inspected by eye and by a stdlib PNG pixel probe (`fixtures/png_probe.py`, 15% / 85% points of each quadrant):

| Image | Size | TL / TR / BL / BR | SHA-256 |
|---|---|---|---|
| a1 original, a3 after undo | 612 x 792 | red / green / blue / yellow | `bbbd1145…835f` (identical) |
| a2 rotated, a4 after redo, b1 reopened, d1 desktop-saved | 792 x 612 | blue / red / yellow / green (90 degrees clockwise) | `1f017589…292a` (all four identical) |

- After reopening, `doc_info` reports page 1 as 792 x 612 with rotation 90 and page 2 as 420 x 300, no repairs or warnings. Extracted text on page 1 is `BDF BUILD-001 smoke page 1` plus the second line. Script C reopened page 2 as 200 x 300, rotation 270.
- The save used a full rewrite (`incremental: false`, 1,160 bytes). `smoke/out/saved-rotated.pdf` is `50fc8012…ade8`; the pinned-binary run produced `b2883d14…124c`. The 205 differing bytes are inside a compressed stream (probably a timestamp; not decoded), while the renders are byte-identical.
- Oracle limit: the render and the reopen both use PdfCraft's own engine. This is **not** the independent-reader check the spec requires; that belongs to BUILD-002.

## 6. Branding cleanup and attribution

Checks:

- `docs/brand/` now holds only `LICENSE-brand.txt`. The eight logo and mark files and `docs/images/pdfcraft-tools.png` are deleted, along with 9 `ATTRIBUTION.toml` entries (repository assets 270 to 261). No source references `artcraft_mark` or `artcraft_logo` any more.
- `cargo xtask assets` passes, and `--write` reproduces the pending `ATTRIBUTION.md` **byte for byte** (rows 30 to 32), which is the agreement the work item asks for with the earlier hand-run renderer.
- Required text is kept: README trademark and credit paragraphs, `NOTICE`, `docs/brand/LICENSE-brand.txt`, the Adobe disclaimer, and the plain credit `Based on PdfCraft by the ArtCraft team.` in README and the About dialog.
- Tests: `links::about_dialog_shows_upstream_credit_and_links_without_restricted_marks` (rewritten in the pending change to assert zero `ArtCraft` image labels), `links::home_screen_links` and `the_about_dialog_shows_the_running_version` pass. I did not revert or restore any restricted asset to satisfy an old assertion.
- Real UI (`ui/05-shot-about.png`, `ui/06-inspect-about.json`): the About dialog shows text only; the accessibility tree has 3 `ArtCraft` hits (the credit twice and the button `ArtCraft website`) and no image nodes.
- I viewed the other 12 committed screenshots in `docs/images/`: none shows the mark.
- One defect in the pending change was fixed: README's table kept a caption cell for the deleted screenshot (commit `83973357`).

Residual items, not fixed here (BUILD-003 owns identity and links):

- The restricted files are still in Git history, and history was not rewritten (as instructed).
- The About dialog, Help and the home screen still link to upstream's Discord, ArtCraft website and PdfCraft pages, and the home card still reads "Join the ArtCraft community".
- `xtask/src/screenshots.rs:113` still describes the ArtCraft mark in the text it would write for the removed tools screenshot, and `xtask/src/assets.rs` still allows the trademark licence for `docs/brand/` logos. Neither runs unless someone regenerates screenshots or re-adds a logo.
- The remaining brand-licence text in README, NOTICE and AGENTS.md describes the removal rule and was left intact.

## 7. Failing tests: pre-existing, with a known mechanism

13 tests failed in all four runs (rows 70 to 72 and 80): `a_cancelled_recovery_password_does_not_attach_to_the_next_unlocked_document`, `a_local_gotor_link_opens_in_a_tab_and_applies_its_destination`, `a_remote_destination_survives_password_retries_and_cancel_opens_no_tab`, `a_startup_url_does_not_replace_another_documents_password_prompt`, `a_tool_chosen_with_no_document_asks_for_one_and_runs_on_it`, `an_absolute_gotor_waits_for_consent_and_keeps_its_destination_across_tabs`, `an_approved_absolute_gotor_preserves_password_retries_and_cancellation`, `an_encrypted_snapshot_without_metadata_still_requires_its_password`, `dropping_a_pdf_on_the_window_opens_it`, `encrypted_documents_are_autosaved_encrypted_and_recovered_with_the_password`, `file_open_returns_at_once_and_opens_the_pick_on_the_next_frame`, `password_prompt_opens_and_security_tab_reports_the_details`, `unlocking_a_late_startup_url_keeps_it_in_the_background`. Three more are intermittent: `files_dropped_outside_the_page_grid_still_open_as_documents` (failed in runs 70, 71 and at baseline), `the_open_recent_menu_lists_files_and_opens_one` and `consenting_to_absolute_gotor_still_validates_the_target` (each failed once, in run 70 only).

- **Not caused by the pending changes.** The pristine baseline commit fails 14 of the 16 (row 80): the 13 above and `files_dropped_outside_the_page_grid_still_open_as_documents`. The other two failed only in the first pending-tree run (row 70) and were not seen failing at baseline in the single run made there. They fail in the same file-opening flows and passed in the rerun and single-thread runs. One baseline run cannot separate flakiness from an effect of the changes, but the branding diff touches no file-opening code.
- **Mechanism, confirmed by experiment.** Since `55f76ab7` (#1493, "prepare large PDFs off the UI thread", in the baseline's history) a file open runs on a worker thread that the UI polls with `try_recv`. These tests assert that the document is open after a fixed 2 to 3 headless frames, which finish in microseconds. In row 81 I replaced only the wait in two tests with a bounded loop (at most 3 s, assertions untouched) on the disposable pristine worktree, and both passed in 0.04 s.
- **Not fixed here.** The failures are no crash or data loss, the work item says not to weaken tests, and a 14-test hardening change across six files is a separate task. The real desktop opened files normally in this run.
- Upstream's own CI points the same way: its run on this commit failed both OS test jobs (section 8). The log is not public, so I could not map its failures to these tests.

## 8. Hosted workspace results

- `btecme/bdf`: the GitHub API lists 0 workflow runs (2026-10-13). Nothing was pushed, so there is no hosted result for this branch.
- Upstream `storytold/pdfcraft`, commit `7437bde3` (public check-runs API): `test (windows-latest)` failure ("exit code 1"), `test (ubuntu-latest)` failure ("exit code 101"), `freebsd` failure, `wasm32 check` success, `dependency licences and advisories` success, `windows-arm64` package and install jobs success, fuzz skipped, and a Copilot reviewer job that failed to start. Runs for the preceding 13 commits were cancelled, so there is no earlier history to compare.
- `gh` is not installed on this machine and the job logs need authentication, so no log was read. These results describe upstream's commit, not the BDF tree, and are not evidence that a hosted BDF run is green.
- The `ci.yml` toolchain edit in `c0b95b00` has never run on a hosted runner.

## 9. Desktop run and other observations

Launch: a copy of the built `pdfcraft.exe` (`bin/`, `fb01a3dd…ed7460`) in `desktop/` with a `portable.txt` marker so settings stayed in `desktop/PdfCraftData`, `PDFCRAFT_SYSTEM_FONTS=0`, and `--control control-private/ctl.json`. No existing PdfCraft or PrintCraft data folder existed on the machine. The log reports wgpu on the NVIDIA GPU via DX12 and the control channel on `127.0.0.1:50234`; no token appears in the app log, stdout or stderr, and I never printed the control file.

Driven with `pdfcraft-cli ui --control …`: `state`, `commands` (193), `command help.about`, `screenshot`, `inspect`, `key Escape`, `open`, `command page.rotate`, `edit.undo`, `edit.redo`, `file.save`, `quit`. Results: the launch-time file rendered correctly; the About dialog is as in section 6; the opened copy showed `dirty` true after rotate, true after undo, true after redo and false after Save; the toast read "Saved desktop-edit.pdf"; the file grew from 1,267 to 1,709 bytes as an incremental update (`/Prev`), and CLI script D renders it byte-identically to the headless rotation. The app exited after `ui quit` and the process was gone. Screenshots `ui/02`, `05`, `11`, `13`, `16` (2160 x 1380) are unpublished; only text values are committed.

Observations to carry forward (none changed in this task):

1. **F01 supporting evidence, not a fix.** The app creates the control file with the directory's inherited ACL. Directories under this checkout grant Modify to `<computer>\CodexSandboxUsers` and two unresolved SIDs in addition to the owner, SYSTEM and Administrators (`7437bde3/95-acl-notes.txt`; an ordinary directory beside the control one still has that ACL), so a control file placed there would be readable by those principals. I made `control-private/` owner-only first; the resulting file inherited only the owner ACE. No second-account test was run.
2. The control file with its token **remains after the app quits**. I deleted it by hand. The token is per launch, so a stale one is useless, but the file is not cleaned up.
3. Portable mode still creates a single-instance lock and socket under `%LOCALAPPDATA%\PdfCraft`, outside the portable data folder. The 0-byte socket (a Windows AF_UNIX reparse point) could not be removed with PowerShell, `cmd`, `fsutil` or Python (error 1920). I did not investigate further; the file is harmless and remains on the test machine.
4. `dirty` stays true after undoing back to the original content, headless and in the UI. It is conservative, not data loss, and is the BUILD-007 item "derive dirty from saved snapshot identity".
5. Document ids are not reused after `doc_close` (the next open is id 2), which scripts must account for.

## 10. Evidence paths and hashes

`MANIFEST.sha256` lists 109 files (SHA-256 of the manifest: `4012349f83a9393ba45c4f0641d0d63a3e7b840773adc1bd313bddbc2352ef58`). Key items:

| Artifact | SHA-256 |
|---|---|
| `bin/pdfcraft.exe` (first build, `stable`) | `fb01a3ddfab6ee2d0aa1da4d87730c95f4ea96bcd7fa8d8ba8fddb1c9eed7460` |
| `bin/pdfcraft-cli.exe` (first build) | `9d52a2969e5352fba214bea2896e536ff4e47d5e84b466077e417b302fa839c4` |
| `bin-pinned/pdfcraft-cli.exe` (pinned rebuild; the pinned `pdfcraft.exe` is `af3a09da…ed1`, not copied) | `e5382d162185c96a8b590c92f7d8e18670326e06606076c923750cb3c861d660` |
| `fixtures/make_fixture.py` | `f82fff7432c3e00b9a804e248d9ca48a9cdd4f97e8b70b86c403577d7a29946a` |
| `fixtures/png_probe.py` | `3207c07051b5a7f1dcb8ccea26d7a4f5bec4afb874a65182de3f7f3df3e1f9e8` |
| `fixtures/smoke.pdf` | `3b5c990784070ef51948aa01f39360f6ca1fea8ebeb6823b6eaa6b85589b47ae` |
| `smoke/out/created.pdf` | `24653aa9668bb7e2d2b87fa346f45ee73730bce5af14d5a33bf5b05bebbccdae` |
| `ui/desktop-edit.pdf` (saved by the desktop) | `c6a830d65b5c5c51375eeafd5e4633698777ff379eaf0762cccf63faf3d3bcb9` |
| `ui/02-shot-open.png` | `1f652bdd0c795810558ec0f0ae82d08be85a80422dc49ca6f7214b537b158f85` |
| `ui/05-shot-about.png` | `c1220ef3c32bd18f70ee558ae65df3d36afc1550d9faf96df3803710250c78f0` |
| `ui/16-shot-saved.png` | `918ea6013416a7948e3689e1a470111fa6b3c598e142e874b137d1dfe88fc04b` |
| `7437bde3/70-test-ui-app-cli.log` | `2b1303093feb399ee53a71a565f18d2c303a1be6f15ecef8ad9c39d35cd4e70e` |
| `7437bde3/80-baseline-pristine-test-failing-bins.log` | `892298be34a10853306c77b55ebf9f0fe7121a459485dcc84a4f412934be7945` |

The binaries are not bit-for-bit reproducible across the two toolchain directory names (embedded paths), but their renders are.

## 11. Limitations and BUILD-002 readiness

Limitations: debug profile only; one machine and one GPU path (wgpu on DX12); no Narrator/NVDA, DPI scaling, mixed monitors, printing, file-association or installer checks; no independent PDF reader or raster oracle; no two-account ACL test; no clean-machine or fresh-clone build (the checkout carried the owner's changes and my commits); MSRV 1.95 and the optional font build untested; no hosted run of the fork. Upstream's `parity` numbers are upstream's claims, not BDF acceptance.

**BUILD-002 readiness: conditionally ready.**

- No missing prerequisite remains, and no crash or data-loss defect appeared, so neither stop condition of BUILD-001 applies. The toolchain, locked build, CLI and desktop control channel that BUILD-002 needs all work, and its scope (xtask, render, engine, corpus, worker prototype) does not depend on the failing UI tests.
- BDF-00's exit gate ("known-green baseline") is **not met**: the scoped UI test suite fails 13 to 16 tests on every run, on baseline and pending tree alike, and there is no green hosted run.
- Recommendation for the owner: either approve a small follow-up that makes those tests wait for the asynchronous open (assertions unchanged, rerun, then obtain a hosted Windows/Linux run), before or alongside BUILD-002, or record the 13 failures as an accepted exception. BUILD-001 stays `pending_review` until Codex has reviewed this report and that decision is made.

## Appendix: fixture generator

<details>
<summary><code>make_fixture.py</code> (run as <code>python -I make_fixture.py OUT.pdf</code>)</summary>

```python
import sys

def build(objs):
    out = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
    offs = []
    for i, body in enumerate(objs, start=1):
        offs.append(len(out))
        out += f"{i} 0 obj\n".encode() + body + b"\nendobj\n"
    xref = len(out)
    out += f"xref\n0 {len(objs)+1}\n".encode() + b"0000000000 65535 f \n"
    for o in offs:
        out += f"{o:010d} 00000 n \n".encode()
    out += f"trailer\n<< /Size {len(objs)+1} /Root 1 0 R /Info 7 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode()
    return bytes(out)

def stream(data: bytes) -> bytes:
    return b"<< /Length %d >>\nstream\n" % len(data) + data + b"\nendstream"

p1 = b"""
1 0 0 rg 0 396 306 396 re f
0 0.8 0 rg 306 396 306 396 re f
0 0 1 rg 0 0 306 396 re f
1 1 0 rg 306 0 306 396 re f
0 0 0 rg
BT /F1 28 Tf 120 560 Td (BDF BUILD-001 smoke page 1) Tj ET
BT /F1 14 Tf 120 520 Td (Synthetic fixture - quadrants R G B Y) Tj ET
"""
p2 = b"""
0.6 0.6 0.6 rg 40 40 340 220 re f
0 0 0 rg
BT /F1 20 Tf 60 150 Td (page 2 grey box) Tj ET
"""
objs = [
    b"<< /Type /Catalog /Pages 2 0 R >>",
    b"<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>",
    b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 6 0 R >> >> >>",
    stream(p1),
    b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 420 300] /Contents 8 0 R /Resources << /Font << /F1 6 0 R >> >> >>",
    b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
    b"<< /Title (BDF BUILD-001 synthetic fixture) /Producer (make_fixture.py) >>",
    stream(p2),
]
open(sys.argv[1], "wb").write(build(objs))
```

</details>

## Revision history

| Date | Revision | Change |
|---|---|---|
| 2026-10-13 | 0.1 | First evidence report for BUILD-001: baseline build, gates, smoke, desktop run, failing-test triage. |
