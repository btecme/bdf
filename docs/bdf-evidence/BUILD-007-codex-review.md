# BUILD-007 — independent Codex review

Status: **changes requested: three P1 findings. Do not merge yet.**

Reviewed 2026-10-24: [PR #9](https://github.com/btecme/bdf/pull/9), head
`295722f0f807f5d3fcc16ca0e4b4687f8f2e0a3b`, base `main` at
`c85a1e55bfab2c563fbd8b0319102e46dfa0a92b`. The checkout was clean at the start.
The code commit is `277a67cb`; the lock-only patch update is `b12a1ae4`.
This is a review of the engine revision changes and the first service slice, not an acceptance
of adapter migration or authorization work assigned to BUILD-009 and BUILD-008.

## R1 — P1 — reconcile late saves with the actual saved content and newer completions

Locations: `crates/engine/src/lib.rs:3851–3857` and
`crates/app-service/src/service.rs:206–208`.

`mark_saved_at` adopts the completion's path and then returns `EditedSince` on a generation
mismatch without changing the saved-content identity or dirty flag. Previously undo always
left dirty true; this PR makes undo back to the old saved state clean, so that assumption no
longer holds. `finish_save` also replaces the tracked file version unconditionally before
checking whether a newer save has already published.

Independent public-API reproductions:

1. Open the unrotated file; rotate 90°; prepare its save; undo back to the old saved state;
   write and finish the prepared 90° version. The service reports `edited_since=true`, but
   the working page is 0°, the disk page is 90°, and `dirty=false`. Closing with
   `discard_changes=false` succeeds. The unsaved working version can disappear without a prompt.
2. Prepare Save As A at 90°, edit to 180°, prepare Save As B; write both and finish B first,
   then finish A. The older completion changes the document name/destination back from
   `new-save.pdf` to `old-save.pdf` while leaving it clean. Its next tracked save is now aimed
   at A, although B was the newer completed save. Keeping the edit model alone is insufficient.

Both safety assertions fail with exit **101**. The current late-save test only adds a newer
edit that is already dirty and covers neither case.

Required correction: reconcile the saved identity with the content actually written, including
when an undo made the current state clean. If that identity cannot be retained, invalidate
the old saved marker conservatively and keep dirty/recovery/close behavior consistent. Also
prevent older save completions from replacing a newer published destination or tracked file
baseline. Report their real file effect without regressing the current document metadata.
Regressions must cover undo-before-completion and two saves finished out of order, including
the next tracked target and close without discard.

## R2 — P1 — bind each write receipt to its own prepared save

Locations: `crates/app-service/src/service.rs:190–208`, `410–412`, `426–447`.

`WrittenFile` contains only a `FileVersion`. `finish_save(prepared, written)` accepts any
receipt with any plan, uses the plan's document/target/bytes and the receipt's fingerprint,
and never verifies that those two objects describe the same write.

Independent reproduction: prepare saves for documents A and B, write B only, and supply B's
receipt to `finish_save` with A's unwritten plan. It returns success, records `FileWritten`
for A, marks A clean and lets A close without discard. A's advertised file does not exist.
This loses A's edit and makes the committed-effect record false. A second probe freezes the
clock and verifies byte-for-byte equality of serialized A and B; the wrong receipt still marks
unwritten A clean. Both probes fail with exit **101**. Checking a content hash alone cannot
bind a receipt to a destination/document.

Required correction: make a write completion inseparable from its originating plan, or bind
and validate the receipt's service/document/plan/target identity before touching document or
tracking state. Prefer an opaque completion returned by a consuming write and accepted by
finish, so adapters cannot accidentally pair unrelated worker results. Keep the already-written
file's actual effect truthful on any rejected completion. Cover differing and identical bytes,
distinct targets/documents, and foreign-service completions. This is part of the typed save
contract, not the future actor/grant policy.

## R3 — P1 — validate the final file component on confined open

Location: `crates/app-service/src/service.rs:233–248`.

The confined service joins a root-relative path and pins/checks its **parent directory**, but
`metadata` and `read_with_version` follow the final filename if it is a symbolic link. Unlike
the automation resolver, this path never checks the resolved file against the root. A safe
parent does not imply a safe leaf.

Independent Windows reproduction: create a synthetic three-page PDF outside the selected
root and a file symlink `root/alias.pdf` pointing to it. File-symlink creation succeeds with
the normal user token on this host. `Service::confined_to(root)` then successfully opens
`alias.pdf` and returns all three outside pages. The expected refusal assertion fails with
exit **101**. There is no race, external system, personal file or privileged account in this
probe. The current junction test covers a linked directory, not a linked filename.

Required correction: confine and validate the final opened file handle as well as its directory,
using a shared safe platform primitive that does not follow an unchecked leaf. Reject a static
outside leaf link and a leaf swapped during open; ordinary files below the root must continue
to work. Apply the size limit to the same validated handle/read. Preserve the existing Windows
directory pin and do not turn the documented Unix race limitation into a claim that static
traversal is allowed. This boundary must hold before adapter migration.

## Validation and evidence

The independent probes use only public service requests and generated synthetic fixtures under
ignored `target/bdf-evidence/BUILD-007-codex-review/`. No production source was changed to
trigger them. Their assertion failures are deliberate review evidence, not application panics.
The original fixture-generation helpers are reused from the repository's service tests.

Current-head hosted evidence is green:

- [CI 40091282303](https://github.com/btecme/bdf/actions/runs/40091282303): Windows, Ubuntu,
  wasm and dependency policy pass; fuzz is skipped by event. Windows/Ubuntu logs confirm
  GitHub's test merge `0d49cea` of this exact head into `c85a1e55`, engine **246 pass/1 ignored**,
  service **19 pass**, parity **3 pass** and UI recovery **15 pass**. These tests do not cover
  the failing cases above.
- [FreeBSD 40091282367](https://github.com/btecme/bdf/actions/runs/40091282367) passes.
  [ARM64 40091282349](https://github.com/btecme/bdf/actions/runs/40091282349) passes both
  cross-packaging and native install/run jobs.

Independent local tests also pass, with `--locked --offline -j 2` and bounded subprocesses:
engine **246 unit tests pass/1 ignored**, plus 9 integration tests passing and 2 private-font
integration tests ignored; service **19 pass**; automation service parity **3 pass**; UI recovery
**15 pass**. These ran as the normal Windows user, not the restricted observer token used in
earlier privacy experiments. The final probes were recompiled against the fresh service library
named in Cargo's build output, and **all five safety assertions fail with exit 101**: late undo,
out-of-order completion, wrong receipt, byte-identical wrong receipt and outside leaf link.

The scoped local suites and the final rebuilt probe runs are recorded in `tests.json`, the
individual logs and `probe-results.json`. Artifact selection is captured from Cargo's JSON
build output; source hashes identify the reviewed implementation. The manifest excludes
executables and generated fixture files. Its 19-file manifest SHA-256 is
`3654e6511499c0900c27fd9f7ffe16807d190a7b535c64dd1b65e11283739616` (also in `manifest.sha256`).

The new same-layer edges are specific, not a general relaxation; no new UI dependency below
L7 or service/automation production cycle was found. The lock changes add the service and
one dev-dependency edge and patch three registry dependencies; hosted dependency policy passes.
No adapter migration, authorization, worker deadlines or native macOS testing is claimed.

## Focused Claude handoff

Correct R1–R3 within BUILD-007 and add the missing public-contract regressions. Keep the
first-slice scope; do not start BUILD-008/009 or ask the owner to choose Rust APIs. Keep the
original failed probes as the red baseline, show the corrected cases green, run scoped engine,
service, parity and recovery tests, then update the PR/evidence against the actual final head.
No implementation fix, commit, push or merge was made by Codex.

## Revision history

| Date | Change |
|---|---|
| 2026-10-24 | Independent review: late-save state/publication, unbound write receipts and confined leaf-link reads require correction. |
