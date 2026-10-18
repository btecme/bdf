# BUILD-004 evidence: one durable persistence service

Status: **in progress (2026-10-18). Implemented and tested locally on Windows 11 x64; Unix and macOS code is written but only a hosted run can exercise it; not yet reviewed by Codex.** Work item BUILD-004 in [bdf-work-items.toml](../bdf-work-items.toml) stays `pending`. Branch `feat/build-004-durable-persistence`, from `main` at `f54e3e53` (PRs #2 and #3 merged). BUILD-004 depends on BUILD-003, which is still `pending` because its slice 4 (draft PR #4) awaits Codex's review; this branch does not depend on PR #4's files. Local commands ran with Rust 1.99.0, `--offline --locked`, two jobs. Finding F02 of the [foundation review](../bdf-foundation-review.md).

## 1. The finding and what was found beyond it

F02: the headless save (`crates/automation/src/lib.rs`) wrote the staging file and renamed it without flushing, while the desktop save flushed. Reading both paths and the callers turned up more, all in the same area:

| # | What | Where (before) |
|---|---|---|
| 1 | Headless save never called `sync_all`; a failed flush could not fail the save. | `automation::write_atomic` |
| 2 | Two copies of the staging-write-rename routine (desktop, headless) with different guarantees. | `ui-egui::editing::write_atomically`, `automation::write_atomic` |
| 3 | **A rename replaces a restricted file's access list.** On Windows `std::fs::rename` (MoveFileEx) gives the new file the folder's inherited access list. Observed 2026-10-18: a file with only the current user in its ACL, replaced by rename, was left readable by the other accounts the folder grants (`CodexSandboxUsers`, `NT AUTHORITY\\SYSTEM`, `Administrators`, three unnamed SIDs). `ReplaceFile` kept the single-user list. | both |
| 4 | Plain rename also silently replaces a **read-only** file on Windows (`Move-Item -Force` did in the experiment); `ReplaceFile` refuses it. | both |
| 5 | The command-line tool (`pdfcraft-cli edit/combine/extract/split/render --out`, `check --json`, `ui screenshot --out`) wrote with `std::fs::write`, which truncates the target first: `edit in.pdf --out in.pdf` with a full disk destroyed the original. The headless `write_output` without a root did the same. | `apps/pdfcraft-cli/src/main.rs`, `automation::Automation::write_output` |
| 6 | After a save that reached the disk, `Session::mark_saved` reopened the saved bytes **before** adopting the new path. If the reopen failed, the file was on disk but the document still pointed at the old path (a later Save would write the old file) and the notice said only "Saved, but reopening failed". | `engine::Session::mark_saved` |
| 7 | `mark_saved` rebased on the saved bytes and cleared the dirty flag unconditionally. An edit made between taking the bytes and recording the save would have been discarded silently. Unreachable today because the desktop save is synchronous; it becomes reachable with the background jobs of BUILD-010. | same |

Experiment for 3 and 4 (PowerShell and `icacls`, scratch folder, nothing kept): target `target.pdf` with `icacls /inheritance:r /grant:r <me>:(F)`; replacement file created beside it. After `Move-Item -Force`: the broad inherited list. After `[IO.File]::Replace(new, target, backup)` (which calls `ReplaceFile`): `<me>:(F)` only. Read-only target: `Replace` fails with "Access to the path is denied"; the move succeeded.

## 2. The service

`crates/platform/src/persistence.rs` (new), documented at the top of the file:

* `write_replace(path, bytes, form)` / `write_replace_with(.., suffixes, &Steps)`: inspect the target (a folder or a device is refused), create a private staging file with an unpredictable name (`create_new`, as before; now `0600` on Unix and opened with no sharing on Windows), write, **flush** (`sync_all`; macOS falls back to `fsync` where the volume refuses a full flush, as the desktop already did), then replace. Every failure before the replacement removes the staging file and leaves the target untouched; the error says which step failed (`Stage::{Inspect, Stage, Write, Flush, Replace}`).
* **Replacement.** Unix: the existing file's mode is copied to the staging file (a new file gets the mode an ordinary new file gets in that folder, read from a probe file because the umask cannot be read without changing it), then rename, then the folder is flushed (`Committed::directory_flushed`). Windows: `ReplaceFile` through `winsafe` (already a workspace dependency; no new crate, no `unsafe`), with a backup name; if it fails part way and the target is gone, the original is renamed back, and if that fails too the error carries `original_copy`, the file that holds it. A new file or a symbolic link (the link itself is replaced, so a save cannot be redirected) is a plain rename.
* `write_output(path, bytes, form)` for command-line outputs: the same, but a symbolic link is followed (the file behind it is replaced and the link stays) and something that is neither file nor folder (`/dev/stdout`, a pipe) is written in place.
* `Steps` lets a test make the write, the flush or the replacement fail. `Steps::real()` is the only set the product uses.

**Callers.** Desktop `write_atomically` and headless `write_atomic` are now thin wrappers (same names, same signatures, same staging-name shapes, so the planted-name tests of both stay as they were); `Automation::write_output` and the seven `--out` writes of `pdfcraft-cli` use `write_output`. Dependency changes: `pdfcraft-platform` gains `winsafe` (Windows) and `rustix` (macOS, moved from `pdfcraft-ui-egui`, whose only use it was); `pdfcraft-cli` gains `pdfcraft-platform`; `Cargo.lock` changes by those edges only.

## 3. Save state

* `Session::mark_saved_at(id, bytes, path, generation)` and `Saved::{Rebased, EditedSince}` (new). The document always takes the new path once the file is written. If it was edited since the bytes were taken (`edit_generation` differs) nothing else is touched and `EditedSince` is returned; otherwise editing is rebased and the document is clean. If the saved bytes cannot be reopened it is `EditError::Reopen`, with the document unsaved and holding its own state. `mark_saved` keeps its signature and calls it with the current generation.
* Desktop `save_doc_to` takes the generation with the bytes. A failed write: "Couldn't save", the document unsaved, recovery data and original file untouched (it never reached `forget_recovery`). A committed save whose reopen failed: the existing, already translated notice "Saved, but reopening failed: {e}", now with the document pointing at the file that was written, recovery retained, no "after" action (a pending close does not proceed). `EditedSince`: a new notice, "Saved {name}, but changes made while saving are not saved yet", unsaved, recovery retained. The new notice has no translation in the 22 catalogues yet (untranslated labels use English).

## 4. Tests, and each one seen to fail

Platform (`persistence::tests`; the Unix module adds five, not run here): new-and-replace leaves nothing behind; short write; failed flush (also asserts the flush saw the bytes written and the target untouched); failed replacement; folder and planted names refused; restore only when the original is gone; flush helper; command-line output; on Windows, **restricted ACL survives** (with the old rename as the contrast: it must change the list, or the test fails loudly rather than being vacuous), read-only refused, target held open by another handle refused and then saved after release, staging unreadable by another handle while written. Headless (`automation`): failed flush, short write, failed replacement, restricted ACL. Engine: an edit during a save is kept and unsaved; a saved file that cannot be reopened moves the document but keeps it unsaved. UI: a save onto a folder leaves the edits, the path, the folder's contents and no temporary file.

Mutations (each reverted afterwards; the file was restored byte for byte):

| Mutation | Result |
|---|---|
| `ReplaceFile` replaced by a plain rename (the old behaviour) | `a_restricted_targets_access_list_survives_the_save` fails |
| Flush skipped | `a_failed_flush_keeps_the_original_and_fails_the_save` fails |
| Staging file not removed after a failed replacement | 3 platform tests fail |
| Original not restored | `restoring_puts_the_original_back_only_when_it_is_gone` fails |
| Stale snapshot not detected | `an_edit_made_while_a_save_was_in_flight...` fails |
| Path not adopted before the reopen | 3 engine tests fail |
| Headless save does not flush (the F02 baseline) | `a_failed_flush_fails_a_headless_save...` fails |

The headless flush baseline is shown by mutation, not by running the old code, because the old code has no seam to make a flush fail.

**Checks (Windows, final tree):** platform 24 tests (the new ones plus the existing staging and theme tests), engine 235 (1 ignored, as before), automation 40, the `pdfcraft-cli` suites, and the four UI save tests (`editing`: the new one, save writes an incremental update, closing a dirty tab asks and saves) all pass; rustfmt and `clippy -D warnings` over platform, engine, automation, CLI and the UI crate are clean; `cargo xtask layers`, `assets` and `pins` pass; the platform crate checks for `wasm32-unknown-unknown`. The full workspace test run, `cargo xtask wasm` and the MSRV check were not run locally (hosted CI runs the workspace).

**First hosted run (draft PR, head `0d1a64f`) found two defects of mine, both fixed.** (1) On Linux and FreeBSD `an_existing_files_mode_is_kept_and_a_new_file_gets_the_ordinary_one` failed (`0o600` against `0o644`): the umask probe used the staging helper, which is now private, so a new file came out `0600`. The probe now uses an ordinary open. (2) On the Windows runner `a_restricted_targets_access_list_survives_the_save` failed at its own contrast assertion: the runner's temp folder already grants the accounts my "restricted" list named, so the plain rename left the list unchanged and the test refused to pass vacuously. The test file now carries an explicit entry (read for the built-in Users group) that no folder's inherited list can contain; the headless ACL test got the same entry. A third failure, the `dependency licences and advisories` job, is not from this change: cargo-deny reports a new `unmaintained` advisory for `zbus_names 4.3.4`, which was already in the lock file.

## 5. Not done, and not claimed

* **Windows staging privacy is partial.** The staging file is exclusive while written, but between being closed and replacing the target it carries the folder's inherited access list; applying the target's list first needs an access-list API that `winsafe` does not offer safely, and the workspace forbids `unsafe`. The stop condition in the work item asks for a report instead of a workaround: this is it. A restricted document's bytes are therefore readable by whoever the folder grants, for the milliseconds between close and replace. The saved file's list is preserved.
* **Unix and macOS are not exercised here.** The mode copy, the umask probe, the folder flush, the `0600` staging file, the symbolic-link behaviour and the macOS `fsync` fallback are covered by tests that only a Linux or macOS run executes.
* **No power-interruption experiment.** `sync_all` and the folder flush are requested; whether a volume honours them was not tested.
* The owner is not preserved on Unix; `ReplaceFile` on Windows takes the Windows long-path and non-Unicode limits of `winsafe` (a name that is not valid Unicode is refused with a message rather than saved without its permissions).
* The `EditedSince` path and the reopen-failure path are tested in the engine; the UI's handling of them is not exercised by a UI test (the synchronous save cannot reach `EditedSince`, and no fixture makes the written bytes unreadable).
* Other writers were not changed: settings and recovery metadata already use `write_atomically`; the control-channel file, the log file and the PKCS#12 export (which has its own flush) were left alone.

## Revision history

| Date | Change |
|---|---|
| 2026-10-18 | First version: service, callers, save-state semantics, tests and mutations, limits. |
