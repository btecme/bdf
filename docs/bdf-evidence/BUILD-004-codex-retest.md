# BUILD-004 — independent Codex retest

Status: **changes requested: R1 closes; R2 remains open with one P1 recovery defect.**

Reviewed 2026-10-18: [PR #5](https://github.com/btecme/bdf/pull/5), head
`c1d7b67e9f4c6db7923d7536c298ca57f52a80d2`, base `main` at
`f54e3e539fb1da22c9a0cef21dac204f2dff3257`. The checkout was clean at the start.
This supplements the [initial review](BUILD-004-codex-review.md); its original red evidence
is preserved. No production code, tracked test, dependency or workflow was changed by Codex.

## R1 — closed: private staging survives the replacement boundary

The empty staging file now adopts the target's DACL through `ReplaceFile` on a temporary
hard link before any document bytes are written. Failure to establish that protection on an
ACL-bearing or unknown volume refuses the save before writing. The no-ACL volume branch
does not drop an existing ACL; it is not evidence of a physical FAT/exFAT test.

Independent actual-source probe: a normal Windows process creates a synthetic target with
inheritance removed and the current user granted access, inside a folder granting ordinary
Users inherited read access. At the closed-handle, pre-replacement seam, a separate restricted
Codex process gets **permission denied for both the original and the staged document**.
The creator can read the staged synthetic bytes, observes the target DACL on the stage, injects
a replacement failure, then verifies the original bytes and removal of staging/link/backup names.
Both final probe processes exit 0. This is a restricted-token experiment, not a provisioned
second Windows account. The first observer assertion incorrectly required Python `winerror=5`;
Python reported `errno=13` with no `winerror`. The assertion was corrected to EACCES and the
experiment rerun. That harness correction is not a product defect.

The normal-token Windows platform suite independently passes **29 tests**, including the
before-write, after-write, flush, closed-handle, refusal and final-target ACL checks.

## R2 — P1 — restore the working bytes before rebuilding view and recovery

Location: `crates/engine/src/lib.rs:3802–3810`, particularly the call to `show_released`
after restoring only `editor.cos` and XFA. Related consumers: `release_working` at 3672,
`show_released` at 3682 and `autosave_snapshots` at 4324.

The correction preserves the edit model, dirty flag, destination and undo history. The clean
Save As reopen-failure probe now passes. However, `refresh` first calls `release_working`,
which sets `doc.bytes` and `doc.display` to the **candidate's** original bytes. When inspection
rejects that candidate, the error branch restores the old COS but leaves those two byte buffers
pointing at the rejected file. `show_released` builds its renderer from that invalid display.

The independent probe links the freshly rebuilt engine, uses the same missing-page-tree-child
counterexample as the first review, and adds actual rendering and recovery round trips:

```text
renderer before failed save refresh: 200x300, error=None
refresh result=Err(Reopen(...document has no pages...))
dirty=true; title=Some("KEEP WORKING EDIT"); pages=1
working_bytes_preserved=false; display_bytes_preserved=false
renderer after failed save refresh: 0x0, error=Some("page 1 does not exist")
autosave after failed save refresh: has_working_edit=false
reopen=Err(Invalid("the document has no pages"))
```

The safety assertion fails with exit **101**. A pre-failure snapshot reopens successfully.
The next post-failure snapshot is unreadable and lacks the retained edit. The UI's
`autosave_now` writes that snapshot under the existing recovery key, so the next autosave
can overwrite a good recovery copy with the rejected file. Merely counting recovery files or
checking the old `info.pages` does not catch this. A crash after that write can lose the only
recoverable working edit. This keeps R2 at P1; it is not solely a visual issue.

Required correction: restore the working/display byte state along with the model before
rebuilding the renderer, or perform the candidate refresh without publishing any working
state until success. Preserve the committed destination and dirty/generation behavior already
fixed. Extend regression coverage to render the page after rejection and to autosave **again**,
then reopen the stored recovery bytes and verify the edit. Keep this a bounded BUILD-004
correction; no new product decision, platform expansion or persistence framework is needed.

## Verification and limits

- Independent Windows engine suite: **237 unit tests pass, 1 ignored**; 9 integration tests
  pass and 2 private-font integration tests are ignored. The clean-reopen probe passes; the
  strengthened refresh/recovery probe fails as above.
- Independent Windows platform suite: **29 pass**, run with the normal user token because
  the restrictive Codex token is the observer, not the intended save credential.
- Independent Windows UI recovery suite: **14 pass**; hosted Windows and Ubuntu runs also
  pass those 14 tests. They do not render or
  reopen the next stored recovery copy after a failed candidate refresh.
- Current-head CI [39138223967](https://github.com/btecme/bdf/actions/runs/39138223967):
  Windows, Ubuntu, wasm and dependency policy pass; fuzz is skipped by event. Windows and
  Ubuntu job logs confirm the regression tests ran, at GitHub's test merge `97ec8774` of this
  exact PR head into `f54e3e53`.
- [FreeBSD](https://github.com/btecme/bdf/actions/runs/39138223971) passes.
  [ARM64](https://github.com/btecme/bdf/actions/runs/39138223948) cross-package and native
  install/run jobs pass. Native macOS, physical power interruption and a provisioned second
  Windows account were not tested; none is silently counted as covered.
- The three advisory-ID-specific `deny.toml` entries concern informational unmaintained
  crates, with an explicit migration/revisit condition. This is a bounded dependency-policy
  disposition, not a fix for a known exploit or a blanket vulnerability exception. See the
  [RustSec advisory](https://rustsec.org/advisories/RUSTSEC-2026-0370.html) and companion
  [0371](https://rustsec.org/advisories/RUSTSEC-2026-0371.html) and
  [0372](https://rustsec.org/advisories/RUSTSEC-2026-0372.html) records.

The PR body still describes the superseded staging limitation; update it with the final
implementation and review result when submitting the correction. BUILD-003 slice 4 / PR #4
is a separate dependency and is not accepted by this retest. No merge is recommended yet.

Raw evidence: ignored `target/bdf-evidence/BUILD-004-codex-retest/`, including the source
probes, fresh engine artifact identity, compile/test logs, privacy observer/creator results and
current-head hosted metadata/log excerpts. The manifest records text evidence, excluding
executables and protected synthetic fixture files. The 19-file manifest SHA-256 is
`42ddc7810a50bc36d1f830d5b48a20d0bd1f0c14961aa7dc21d49e598f536186` (also in `manifest.sha256`).

## Revision history

| Date | Change |
|---|---|
| 2026-10-18 | Independent correction retest: R1 closes; R2 still contaminates display and the next recovery snapshot. |
