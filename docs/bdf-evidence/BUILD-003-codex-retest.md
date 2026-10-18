# BUILD-003 — Codex retest of R1–R3

Status: **R1, R2 and R3 closed for slices 1–3; PR #2 merged, PR #3 merge preparation in progress.**

Reviewed 2026-10-16: PR #2 at `7d5087492cfdeeb9207bacdeb0b9dca371ebc77b`, and stacked
PR #3 at `8609fff22b94dbb572143e9e0bac3fb3cc149a0e`. The checkout was clean and matched
PR #3. This supplements [the initial review](BUILD-003-codex-review.md).

## R2 — original alias-key finding (closed by the final retest below)

Location: `xtask/src/pins.rs:54–57`.

The new parser closes the original quoted-key, whitespace, flow-mapping and shell-text cases.
However, `Reader::node` only recognizes `Event::Scalar` when the node is a mapping key.
An alias key is treated as a different key and the subsequent floating reference is ignored.
This complete workflow is a counterexample:

```yaml
name: &stepkey uses
on: workflow_dispatch
jobs:
  probe:
    runs-on: ubuntu-latest
    steps:
      - *stepkey : actions/checkout@main
```

The alias resolves to the string `uses`. Independently compiled probes against the current
module report **0 scanned / 0 violations**. Actionlint **1.7.12**, the version used by this
repository, accepts the entire workflow with exit **0**. This is different from an alias in
the *value* position (`uses: *ref`), which the new scanner correctly rejects.

A separate regression assertion requiring one violation fails on the reviewed head:
**0 passed / 1 failed**, exit **101**, with actual violations `[]`. The normal nine pinning
tests still pass, demonstrating the missing case rather than an existing-suite failure.

Required correction: resolve alias keys before interpreting the key, or explicitly refuse
unresolved/unsupported alias keys with a diagnostic. Do not silently label an alias key as
not-`uses`. Add this full workflow as a failing-before-fix regression and keep the existing
syntax, malformed-input, alias-value and shell-block coverage. This remains a correction to
R2, not a request for broader product work.

## R1 — closed: verified package installation

Source review confirms a dedicated NuGet configuration with only the verified feed, isolated
package/HTTP-cache paths, a separate tool path, and checks of every installed `tools/` file
against the verified archive before invoking the tool by its full path. The change no longer
relies on NuGet's SDK-dependent `.nupkg.sha512` record.

The real `packaging/windows/test-install-wix.ps1` passes independently on Windows x64:

- The normal install uses the recorded package and runs the installed tool.
- Tampered bytes and a wrong expected hash are refused without installation.
- A control really installs a poisoned package from an ambient source; the corrected helper,
  called from the same directory, installs the verified package and none of the poison.

The test used workspace-contained temporary/cache/tool paths, disabled CLI telemetry and used
a wrapper that validates resolved cleanup paths before recursive removal. No global tool or
MSI was installed. The wrapper does not alter the installer or its test assertions.

The successful [hosted x64 package job](https://github.com/btecme/bdf/actions/runs/38954319523/job/117153228978)
was also fetched and inspected: all self-test checks pass, the real package install verifies
**39 tool files**, and MSI install/run/uninstall succeeds on the hosted SDK. The relevant
implementation/workflow files are unchanged between that correction head `f8bbf3c0` and
current PR #2 head `7d508749`.

## R3 — closed: technical correction passes and owner approves core annotation printing

The profile now evaluates omitted/null values through declared defaults, and the execution
paths read the same constants. The original print counterexample returns the same result in
all three forms. Tests include an excluded synthetic default and actual print/rotation behavior,
so simply allowing every default would not satisfy the tests.

Printing with `document-and-markups` is now classified as core; `document-and-stamps` and
`form-fields-only` remain excluded. This is the one product classification change. On
2026-10-16, the owner explicitly approved including annotations in core printing. With that
decision and the passing technical correction, R3 is closed.

Runtime enforcement, dispatch parameter checks and route-completeness automation remain
explicitly deferred. No claim of live policy parity is added by this retest.

## Independent checks and hosted state

Rust 1.99.0, locked/offline Cargo, two build jobs, finite timeouts:

| Check | Result |
|---|---|
| Capability tests | **14 pass**; includes behavioral default checks |
| Pinning tests | **9 pass** |
| Scoped automation/xtask Clippy, all targets, `-D warnings` | Pass |
| Format and repository pins gate | Pass |
| Original YAML counterexamples / whole-repository fixture | Now detected: **13 scanned / 1 violation** |
| New aliased-key workflow | Scanner reports 0 violations; actionlint accepts; independent regression fails as expected |
| Real WiX isolation self-test | Pass |
| New parser dependencies | All three cached `.crate` hashes match Cargo.lock; manifest licenses are MIT/Apache-2.0 variants |

At the original inspection, documentation-head CI was still running: PR #2 runs
[CI](https://github.com/btecme/bdf/actions/runs/38959311536),
[FreeBSD](https://github.com/btecme/bdf/actions/runs/38959311515),
[x64 package](https://github.com/btecme/bdf/actions/runs/38959311715),
[ARM64](https://github.com/btecme/bdf/actions/runs/38959311475), and passing
[packaging lint](https://github.com/btecme/bdf/actions/runs/38959311620);
PR #3 runs [CI](https://github.com/btecme/bdf/actions/runs/38959334655).
Wasm and dependency policy had passed on both heads; Windows/Ubuntu tests and remaining
platform jobs were pending. Fuzz is skipped by its existing event condition. Earlier green
correction runs are not relabeled as current-head passes. The additional R2 counterexample
blocks clearing PR #2 regardless of how those runs finish.

Private evidence: `target/bdf-evidence/BUILD-003-codex-retest/`. Its manifest covers **32 selected
evidence files** (excluding incidental .NET caches) and has SHA-256
`408c6d9ad2eea33e951fbad3ecf4ddb745426bba52f2c52c3e0e00edbe6605e0`.
It includes the actual-module probes, intentionally failing regression, real WiX test commands
and log, hosted metadata/log, dependency/source identities, and the validator archive and
official checksum. The validator archive matched SHA-256
`6e7241b51e6817ea6a047693d8e6fed13b31819c9a0dd6c5a726e1592d22f6e9` before execution.
An initial checksum-read command failed before verification/execution and was corrected to
read the downloaded checksum file explicitly; it was not counted as a validation result.

## PR order and remaining scope

PR #2 remains open and unmerged; PR #3 correctly still targets its branch. The user authorized
retargeting #3 to `main` **after** #2 merges, so the base is not changed early. No merge was
requested or performed in this re-review. An attempted hourly conditional follow-up was
rejected by automatic approval review because it would create a persistent scheduler;
explicit approval was requested. On 2026-10-16 the owner approved the temporary hourly
follow-up, and automation `retarget-bdf-pr-3-after-pr-2-merges` was created with status
`ACTIVE`. It waits quietly for #2 to merge, retargets only #3's base to `main`, verifies the
result, and disables itself after completion or a terminal need for user action. It does not
authorize merging either PR.

BUILD-003 deliverables 2/4/5 and runtime enforcement remain open. This review changes only
documentation; no implementation/workflow change, commit, push, merge or retarget was made.

## Re-review — 2026-10-17

GitHub still reports PR #2 open/unmerged at `7d508749` and PR #3 open at `8609fff2`,
targeting #2's branch. These are the same heads reviewed above. The notification about
`e79ad0d6` concerns an older head and does not close the remaining finding.

Current-head hosted results are now complete: all five workflows linked above pass for #2;
the CI workflow linked above passes for #3. Windows and Ubuntu tests, dependency policy and
wasm jobs pass on both heads; fuzz remains skipped by its event condition. Packaging/platform
workflows do not run for #3's path-filtered changes. These results replace the pending snapshot
above, without turning the independent R2 counterexample into a pass.

All ten implementation/workflow/lockfile hashes in the original source-identity record still
match. A fresh compilation of the actual scanner with the aliased-key regression again fails:
**0 passed / 1 failed**, exit **101**, actual references `[]`. Actionlint 1.7.12 again accepts
the full workflow, exit **0**. Fresh logs and hosted metadata are in
`target/bdf-evidence/BUILD-003-codex-rereview-2026-10-17/`; the original evidence manifest is
unchanged. No repeat of the unchanged passing suites was needed.

**Disposition:** PR #2 still needs the bounded R2 alias-key correction described above. No
remaining finding applies to PR #3's own capability changes after R3's correction and owner
approval, but it inherits #2 and stays stacked until #2 merges. Neither PR was merged, and
#3 was not retargeted early. The authorized hourly follow-up remains active.

## Final correction retest and merge preparation — 2026-10-17

PR #2 head `2730f7a4bacb80585091853f5eb9e2e70f852db4` resolves anchored scalar keys
before checking whether the key is `uses`. The independent regression that failed before the
fix now passes. A fresh Rust test binary built from that exact commit's scanner passes all
**10 pinning tests plus the independent alias regression**. The unchanged PR #3 capability
implementation passes all **14 capability tests** again.

All five workflows pass on #2's exact head: [CI](https://github.com/btecme/bdf/actions/runs/39018433587),
[FreeBSD](https://github.com/btecme/bdf/actions/runs/39018433650),
[ARM64](https://github.com/btecme/bdf/actions/runs/39018433553),
[x64 package](https://github.com/btecme/bdf/actions/runs/39018433582), and
[packaging lint](https://github.com/btecme/bdf/actions/runs/39018433549). Fuzz remains skipped
by its event condition. R2 is closed; there are no remaining findings against slices 1–3.
This does not claim runtime capability enforcement or acceptance of PR #4's separate slice.

On the owner's explicit merge/push instruction, #2 was merged through the signed-in GitHub UI
as `daa880b3db12dee20cca606674207cafee136a64`, after the connector returned HTTP 403.
#3 was then retargeted to `main` and verified through GitHub metadata. The hourly retarget
watcher is paused. A dedicated checkout under the ignored `target/` directory preserves the
owner's active PR #4 checkout. Merging `main` into #3 conflicts only in this evidence document;
both R2 and R3 records and both revision-history rows are retained. Production code is merged
without manual edits. The merge-preparation head must pass CI before #3 is merged.

Merge-preparation checks all pass locally with two build jobs and finite timeouts: format,
scoped xtask/automation Clippy (`-D warnings`), **105 xtask tests**, **37 automation unit tests
and 167 integration tests** (153 + 8 + 4 + 2), layers, assets, pins (**28 references / 0
violations**), parity, and wasm. Each gate's command output and timing is recorded in
`prep-*.log` and `prep-results.json`. The full automation run took about 454 seconds, including
its isolated-checkout rebuild; that is verification duration, not a product performance result.

Fresh evidence is under `target/bdf-evidence/BUILD-003-codex-final-retest/` in the primary
checkout, with preparation gate logs recorded there separately. No output from private
observations or third-party documents is included.

## Revision history

| Date | Change |
|---|---|
| 2026-10-16 | Independently retested R1–R3; closed R1, validated R3's technical correction, and reproduced the remaining R2 alias-key gap with an actionlint-valid workflow. |
| 2026-10-16 | Owner approved annotation-inclusive core printing, closing R3, and explicitly authorized the temporary hourly conditional retarget follow-up; automation created active. R2 remains open. |
| 2026-10-17 | Rechecked unchanged PR heads; current-head hosted CI is green, but freshly compiled aliased-key regression still fails. R2 remains open; #3 awaits #2's merge before retargeting. |
| 2026-10-17 | Final R2 correction independently passes; R1–R3 closed; #2 merged and #3 retargeted, evidence-only conflict resolved for merge preparation. |
