# BUILD-007 — independent Codex correction retest

Status: **changes requested: R1 and R2 closed; R3 partially corrected, one P1 remains.**

Reviewed 2026-10-25: [PR #9](https://github.com/btecme/bdf/pull/9), head
`87683c7eb6aa81a3a80ff72e4c98e9e996cee5da`, base `main` at
`c85a1e55bfab2c563fbd8b0319102e46dfa0a92b`. The correction commit is `fbe46487`;
the later head records hosted evidence. The checkout was clean at the start.
This retest follows the [original three findings](BUILD-007-codex-review.md).
It retains the first service slice's scope: authorization and adapter migration belong
to BUILD-008 and BUILD-009, respectively.

## R3 — P1 — the Windows reparse fallback still reopens an unchecked filename

Location: `crates/platform/src/file_identity.rs:143–149`; callers include
`crates/app-service/src/service.rs:321` and `crates/automation/src/lib.rs:909`.

The original static leaf-link case is fixed. `read_regular_with_version` opens with
`FILE_FLAG_OPEN_REPARSE_POINT`, rejects a symbolic-link handle and checks the size on
the handle it reads. Ordinary confined files and unconfined link opens still work.

However, when the first handle reports a non-symlink reparse point, such as a cloud
placeholder, the function uses ordinary `File::open(path)` to obtain a second handle.
This resolves the filename again and follows a newly substituted symlink. Checking
`meta.is_file()` on that second handle accepts an ordinary PDF outside the root.
There is no comparison with the first file identity and no check of the final handle's
location against the root. Pinning the parent directory does not pin its child filename.
The first file open also retains Rust's default sharing flags, which permit replacement
while that handle is open. See the official
[Rust sharing documentation](https://doc.rust-lang.org/std/os/windows/fs/trait.OpenOptionsExt.html#tymethod.share_mode)
and Microsoft's description of
[reparse-point opens](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew).

This is the remaining part of the original R3 request to reject a leaf swapped during
open. Section 7 of the author evidence and the new function's own comment explicitly
acknowledge the exception; documenting it does not satisfy the confinement contract.
It matters when another process may replace entries in the permitted folder, which
is the scenario the existing Windows directory pin addresses.

**Evidence boundary:** this remaining branch is confirmed by source inspection and
documented Windows/Rust handle semantics. The ordinary static-symlink counterexample
was rerun against the actual service library and now passes. No native cloud-provider
placeholder race was reproduced, and that passing static test does not cover this
fallback. No cloud provider, outside workspace directory or account configuration was
changed to manufacture a fixture.

**Focused correction:** remove the unchecked reopen from the confined reader. Failing
closed on unsupported reparse files with an actionable error is sufficient for this
slice; ordinary unconfined desktop reads can continue using their existing reader.
Alternatively, retain placeholder support only if the same validated file remains
bound to the read throughout. Add a deterministic regression covering the fallback
and a replacement attempt; it must fail with the current unchecked reopen. Keep the
same-handle byte limit, ordinary-file behavior and directory pin. No owner decision
about platform APIs or a new cloud-storage feature is needed.

## R1 — closed — late saves and older completions

The engine's `SaveToken` carries the captured content identity as well as the cache
generation. A late save now records the content actually written and recomputes dirty
state. The service separately orders save publication per document, so an older
completion cannot replace a newer destination or tracked baseline.

Independent public-API retests against the freshly built library pass:

- Open at 0°, rotate 90°, prepare save, undo to 0°, then write/finish 90°:
  memory stays 0°, disk is 90°, `dirty=true`, and close without discard returns
  `Conflict(UnsavedChanges)`.
- Prepare Save As A at 90°, then B at 180°; write both and finish B before A:
  A reports `superseded=true`, the document retains B's name and clean state, and
  the next tracked write updates B while leaving A unchanged. A fresh read of B is 180°.

The new engine reload/in-flight-save regressions and service regressions also pass.
No separate UI ordering acceptance is implied before adapter migration; the existing
desktop save path now carries the same engine token.

## R2 — closed — a completion carries its own plan

`WrittenSave` has private plan/version fields, is returned by `PreparedSave::write`,
and is the sole argument to `finish_save`. A caller can no longer pair an unrelated
plan with a receipt. Service identity and pending-plan checks reject foreign, repeated
or abandoned completions; rejection still reports the actual file write.

Independent cases with different serialized documents and byte-identical serialized
documents pass. Writing/finishing B leaves unwritten A dirty and refuses A's close;
only finishing A's own write makes A clean. A foreign service with its own pending
plan rejects B's completion; B can still finish at home, and replay is rejected with
the committed effect retained. The registered-plan bound and abandon regressions pass.

## Validation and evidence

The six independent probes use synthetic fixtures under ignored
`target/bdf-evidence/BUILD-007-codex-retest/`. All exit **0**: late undo,
out-of-order completion, differing-byte receipts, identical-byte receipts, static leaf
link and open-size boundary. The link probe requires successful native symlink creation
and does not silently skip it. The size test refuses a file one byte above the configured
limit and accepts the exact limit. Original failed probes and their manifest remain
unchanged under `target/bdf-evidence/BUILD-007-codex-review/`.

The harness was rebuilt against the service/engine/platform libraries identified by
Cargo's JSON output for this checkout, not a library chosen by modification time.
Tests and probes ran under the normal Windows user token, with bounded subprocesses;
Cargo commands use `--locked --offline -j 2`.

Local suites: engine **248 unit tests pass / 1 ignored**, plus **9 integration tests
pass / 2 private-fixture tests ignored**; platform **56 pass**; service **26 pass**.
Automation integration **163 pass** and service parity **3 pass**. UI editing **122 pass / 4 ignored**; recovery **15 pass**. Across these scoped suites,
**642 tests pass / 7 ignored**, plus the six independent probes.

The initial UI attempt did not reach the tests: the linker reported `LNK1106` with
about 267 MB free on C:. Only generated incremental compiler caches created during
this retest were cleared (paths verified beneath the workspace); no source or existing
review evidence was removed. The same two UI suites then passed with one build job.
Both the failed build log and successful retry are retained.

Current-head hosted checks are independently confirmed:

- [CI 40200598206](https://github.com/btecme/bdf/actions/runs/40200598206): Windows,
  Ubuntu, wasm and dependency policy pass; fuzz is skipped by event. Windows/Ubuntu
  logs identify merge `988ff98` of this exact head into `c85a1e55` and show the new
  save/link regression functions passing. The Ubuntu strict-reader function passes.
- [FreeBSD 40200598219](https://github.com/btecme/bdf/actions/runs/40200598219): build
  and tests pass; logs identify the same merge and strict-reader/service-link functions.
- [ARM64 40200598210](https://github.com/btecme/bdf/actions/runs/40200598210): both
  cross-packaging and native install/run jobs pass.

A passed function in the default captured test log does not prove that every optional
link subcase ran. The independent local probe explicitly proves its link setup; the
hosted logs are not presented as a cloud-placeholder race test. No native macOS test,
power-loss test or new acceptance of BUILD-006 is claimed.

The 25-file text manifest SHA-256 is
`76867275c2d1a1a0ae035101b25392ea2b85f715da9b4833aebc3e728a65b5ee` (`manifest.json` / `manifest.sha256`).
It includes commands, logs, source identities, Cargo-selected probe artifact hash,
hosted job metadata/excerpts and the cache-cleanup record; binaries and generated
fixture files are excluded. All 19 files in the original red-evidence manifest were
verified unchanged.

BUILD-007 remains `changes_requested` for R3 only. Codex changed review/status documents
and ignored evidence, with no production fix, commit, push or merge.

## Revision history

| Date | Change |
|---|---|
| 2026-10-25 | Independent retest closes R1/R2 and the static R3 case; the Windows reparse fallback leaves R3 open. |
