# BUILD-007 — final independent correction retest

Status: **R1, R2 and R3 closed; no remaining findings in this correction review.**

Reviewed 2026-10-27: [PR #9](https://github.com/btecme/bdf/pull/9), head
`b03e2379eb437df3a743a0f273cce758716d7a8a`, base `main` at
`c85a1e55bfab2c563fbd8b0319102e46dfa0a92b`. The R3 code correction is
`9f7de34b`; the later commit records hosted results. The checkout was clean at the
start. This supersedes the open R3 disposition in the
[previous retest](BUILD-007-codex-retest.md); earlier failed evidence is preserved.

## R3 — closed: confined reads never reopen the filename

The platform reader now opens the final component once without following links,
checks that handle, and reads from it. Its verdict rejects symbolic links, every
Windows reparse point and non-regular files. The former ordinary `File::open(path)`
fallback is removed. The length and read limits still apply to the handle being read.
Both the service and automation confined readers use this primitive; the patch does
not change their callers or introduce a second path around it.

Independent validation has two parts:

- The six public-service probes were rebuilt against the Cargo-selected libraries
  for this head and pass: late undo, older completion, differing-byte receipt,
  identical-byte receipt, static outside leaf link and size boundary. The static-link
  probe actually creates a native Windows symlink; ordinary confined files still open,
  while unconfined reads retain their existing link behavior.
- A small harness extracts the current private reader functions verbatim into ignored
  test code, using its existing injectable open/reparse predicates. After the first
  open it renames the original file and installs a real symlink at the filename,
  pointing outside the selected fixture scope. The reparse verdict refuses the read
  after **one open**. With an ordinary-file verdict, the same replacement leaves the
  read returning **INSIDE** from the first handle, even though the filename now reads
  **OUTSIDE**. Both assertions pass.

An isolated mutant restores the former fallback in that extracted reader. With the
same fixture it opens the name **twice**, returns **OUTSIDE**, and fails the safety
assertion with exit **101**, as expected. Production source was never mutated. This
proves the regression detects the removed behavior rather than passing because a
fixture failed to set up or an earlier validation stopped the call.

**Scope of the fixture:** the reparse metadata predicate is simulated at the private
seam; the file handle and replacement symlink are native Windows objects. No native
cloud placeholder/provider was provisioned or tested. The source has no second open,
and the verdict fails closed for all reparse metadata. This is sufficient to close
the reported unchecked-reopen defect without claiming cloud-storage qualification.

**Product limitation:** confined reads reject Windows cloud/deduplication reparse
files as well as links. Ordinary unconfined desktop opens retain their existing
reader. The fail-closed choice is within the requested BUILD-007 correction scope;
adding safe placeholder support is not a prerequisite for this slice.

## Validation and disposition

Local tests: platform **59 pass**, service **26 pass**, automation **163 pass**,
service parity **3 pass**: **251 passed, 0 failed, 0 ignored**. The six public-service
probes and two strict-reader cases exit 0; the isolated mutant exits 101 as expected.

Tests ran under the normal Windows user token with bounded subprocesses and
`--locked --offline -j 2`. Platform/service tests used `--nocapture`, so their link
skip notices would be visible; none were printed. The independent probes additionally
require successful symlink setup and do not silently skip it.

Current-head hosted evidence is independently confirmed:

- [CI 40241318673](https://github.com/btecme/bdf/actions/runs/40241318673): Windows,
  Ubuntu, wasm and dependency policy pass; fuzz is skipped by event.
- [FreeBSD 40241318765](https://github.com/btecme/bdf/actions/runs/40241318765): build
  and tests pass.
- [ARM64 40241318668](https://github.com/btecme/bdf/actions/runs/40241318668): both
  cross-packaging and native install/run jobs pass.

Windows, Ubuntu and FreeBSD job logs identify GitHub merge `5ad37cd` of this exact
head into `c85a1e55` and show the new placeholder/single-open/verdict regressions
passing. Captured hosted tests do not independently prove optional link subcases ran;
the explicit local probes provide that evidence for Windows.

R1/R2 implementation is unchanged from the previous passing retest, and their public
counterexamples were rerun successfully here. Engine/UI suites were not rebuilt
locally for this platform-only correction; their previous results remain historical,
and the current-head hosted workspace sweep passes. No native macOS, real cloud-file,
power-loss or new acceptance of BUILD-006 is claimed.

The first service slice is clear of the three review findings. BUILD-007 returns to
`pending_review` with `review_status = "corrections_pass"`, following the work-item
status vocabulary; overall owner verification and integration are separate from this
correction review. BUILD-008 authorization and BUILD-009 adapter migration remain
their own work. No production fix, commit, push or merge was made by Codex.

Raw evidence is under ignored `target/bdf-evidence/BUILD-007-codex-r3-retest/`.
The 30-file text manifest SHA-256 is
`731c2d305bccc2a326f78678b400752b9866bd0011ea288e705643f8b117cdbb` (`manifest.json` / `manifest.sha256`).
It includes commands, logs, Cargo artifact identities, exact extracted-function hashes,
hosted job metadata/excerpts and probe results; generated fixtures and binaries are
excluded. Both earlier raw-evidence manifests and all their 44 file hashes were
verified unchanged.

## Revision history

| Date | Change |
|---|---|
| 2026-10-27 | Independent R3 retest passes; all three BUILD-007 findings are closed for this service slice. |
