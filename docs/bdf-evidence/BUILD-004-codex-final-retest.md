# BUILD-004 — final correction retest

Status: **R1 and R2 closed; no remaining findings in this correction review.**

Reviewed 2026-10-19: [PR #5](https://github.com/btecme/bdf/pull/5), head
`26a0797adb803348acd375c296472bd063d42eeb`, base `main` at
`f54e3e539fb1da22c9a0cef21dac204f2dff3257`. The checkout was clean at the start.
This supersedes the open R2 disposition in the [previous retest](BUILD-004-codex-retest.md),
whose failing evidence is preserved. No implementation, tracked test, dependency or workflow
was changed by Codex. Review/progress records are the only tracked edits from this retest.

## R2 — closed: failed refresh preserves rendering and the next recovery copy

The correction saves the working and display byte buffers before temporarily installing the
candidate COS. On refresh failure, it restores both buffers with the working model and XFA
report, then rebuilds the renderer from the restored display. The committed destination is
still adopted and the document remains dirty, with a changed generation so recovery runs again.

Independent reproduction: rebuilt the current engine, selected its engine/render artifacts
from Cargo's JSON build output, and recompiled the original red probe against those artifacts.
The probe now also asserts the title after reopening the next snapshot in a fresh session.
Both clean-reopen and failed-refresh modes exit **0**:

```text
renderer before failed save refresh: 200x300, error=None
refresh result=Err(Reopen(...document has no pages...))
dirty=true; title=Some("KEEP WORKING EDIT"); pages=1
working_bytes_preserved=true; display_bytes_preserved=true
renderer after failed save refresh: 200x300, error=None
autosave after failed save refresh: has_working_edit=true; reopen=Ok(DocId(1))
```

The regression tests now verify byte-buffer equality, actual rendering and reopening of the
next snapshot. The UI tests write the next snapshot through the real `RecoveryStore`, read
it back and reopen it in a fresh session, checking that the deleted page stays deleted. Clean
documents are covered for unreadable bytes and a structurally readable but unshowable PDF.
These assertions address the missed recovery-overwrite case, rather than just counting files
or checking stale page metadata. The previous probe's exit 101 provides the independent red
baseline; production source was not mutated again for this retest.

## R1 and verification scope

R1 remains closed on the basis of the previous independent normal-writer/restricted-observer
experiment and platform tests. The persistence and staging source files are unchanged since
that retest; their SHA-256 values are checked against the previous source-identity record.
The new hosted Windows log also confirms **29 platform tests pass**. No fresh local ACL or
second-account experiment is claimed here.

- Independent local engine tests: **239 unit tests pass, 1 ignored**; 9 integration tests
  pass, with 2 private-font integration tests ignored.
- Independent local UI recovery suite: **14 pass**, including the strengthened committed-save
  and clean-document tests. The hosted Windows and Ubuntu runs pass the same 14 cases.
- Current-head [CI run 39242230979](https://github.com/btecme/bdf/actions/runs/39242230979):
  Windows, Ubuntu, wasm and dependency policy pass; fuzz is skipped by event. Windows and
  Ubuntu logs confirm the new engine regressions and UI cases ran on test merge `d522697d`
  of this exact PR head into the recorded base. Hosted format, clippy, layers and assets pass.
- [FreeBSD 39242231174](https://github.com/btecme/bdf/actions/runs/39242231174) passes.
  [ARM64 39242230910](https://github.com/btecme/bdf/actions/runs/39242230910) passes both
  cross-packaging and native install/run jobs.

The existing limits remain: no native macOS run, physical power-loss experiment or provisioned
second Windows account. They are not new findings or silently claimed coverage. The separate
BUILD-003 slice 4 / [PR #4](https://github.com/btecme/bdf/pull/4) remains open, draft and pending
its own review. This retest closes the BUILD-004 findings; it does not accept that dependency,
mark the broader milestone complete, or authorize a merge/release. Nothing was committed,
pushed or merged by Codex.

Raw evidence: ignored `target/bdf-evidence/BUILD-004-codex-final-retest/`, containing scoped
test logs, artifact selection, the rebuilt independent probe, source identities and hosted
job metadata/regression log excerpts. `manifest.sha256` records the text-evidence manifest
digest; executables are excluded. The 12-file manifest SHA-256 is
`abf163099622a5d7212fcd296d8d85836804b318b5dacaccc1378b1fb93b6afb`.

## Revision history

| Date | Change |
|---|---|
| 2026-10-19 | Second R2 correction passes the independent rendering/recovery counterexample; R1/R2 close. |
