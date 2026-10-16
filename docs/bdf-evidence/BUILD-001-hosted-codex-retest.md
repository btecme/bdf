# BUILD-001 â€” Codex retest of the hosted CI corrections

Status: **all three CI corrections pass independent review and retesting; no blocking finding in this scope.**

Reviewed 2026-10-16 on `chore/build-002-fidelity-resources`, clean starting revision
`de8d6fcc4d98e3c26fbf8d6d7ad784d8801f47a4`, matching [draft PR #1](https://github.com/btecme/bdf/pull/1).
The code corrections are `e0b7738a`, `a1ab8c01` and `a3335589`. The commits after `a3335589`
change documentation only; source comparison confirms that the reviewed code equals the code
whose earlier hosted passes CC recorded. This review uses the final head's own hosted results.

## Findings closed

| Original failure | Independent verification | Result |
|---|---|---|
| Ubuntu Clippy rejects `drop(Option<()>)` | Cleanup is conditional on Windows; real Windows job closure still precedes output-thread joins. Current Ubuntu Clippy passes. Both Windows process-tree/pipe-cleanup regressions pass in the fresh 95-test xtask suite. | Closed |
| FreeBSD worker tests demand Windows-only containment | Only test helpers opt into uncontained execution on non-Windows. Production `Config::new` remains fail-closed. Current Linux and FreeBSD logs each show 27 worker integration tests passing, including default-startup refusal/cleanup and explicit-opt-in tests, plus nine protocol unit tests. | Closed |
| ARM64 target belongs to the wrong Rust compiler | The script queries/adds the target from the repository root, using rustup's selected compiler. Current hosted logs show Rust 1.99.0, installation of its ARM64 target, successful package creation, and successful native install/check/uninstall. Independent local script probes also pass. | Closed |

The worker's production code change is documentation only. The default is not relaxed, and the
memory-cap fault test is now correctly limited to Windows, where its OS counters exist. The test
process-existence check no longer assumes `/proc` exists on FreeBSD. No blanket test exclusion,
warning suppression, compiler-pin removal or disabled workflow was introduced.

## Fresh local checks

Windows x64, Rust/Cargo 1.99.0, locked/offline dependencies, two build jobs and finite outer timeouts.
Worker configurations ran sequentially through Cargo with normal Windows process visibility;
the tests' `tasklist` checks cannot run reliably under the restricted sandbox account.

| Check | Independent result |
|---|---|
| `cargo test --locked --offline -p pdfcraft-worker -j 2` | **36 pass:** 9 unit + 27 integration |
| Same with `--features fault-injection` | **51 pass:** 9 unit + 16 faults + 26 other integration |
| `cargo test --locked --offline -p xtask -j 2` | **95 pass**, including both new process-cleanup regressions |
| Scoped worker/xtask clippy, all targets, `-D warnings` | Pass |
| Worker fault-injection clippy, all targets, `-D warnings` | Pass |
| `cargo fmt --all -- --check` | Pass |
| Layers, assets and parity through locked/offline Cargo | Pass |
| Rust 1.95.0 checks, all targets, worker + xtask; worker again with fault injection | Pass; compile checks, not MSRV test execution |

The independent PowerShell probe parses `package.ps1` and loads only `Invoke-Native` and
`Initialize-RustTarget`; it never executes the packaging script's installer/signing/staging code.
With a stand-in rustup it verifies: one target addition when missing, no addition when installed,
propagation of target-install failure, selection from the repository root, and restoration of the
caller's directory even on failure. A real-rustup check verifies the already-installed host target
path. No ARM64 target or new toolchain was installed on the local machine.

## Current-head hosted results

GitHub's run and job APIs confirm **all four configured workflows pass on `de8d6fcc`**. The failed
jobs from the first run are not used as the current result. Linux, FreeBSD and both ARM64 job logs
were fetched and inspected, not inferred solely from workflow badges.

| Workflow / job | Result | Current-head evidence |
|---|---|---|
| Main CI / Windows x64 | Pass | [116947204287](https://github.com/btecme/bdf/actions/runs/38893135754/job/116947204287) |
| Main CI / Ubuntu | Pass: format, Clippy, workspace tests with craft-fonts, layers, assets, CLI without MCP | [116947204267](https://github.com/btecme/bdf/actions/runs/38893135754/job/116947204267) |
| Main CI / wasm | Pass | [116947203950](https://github.com/btecme/bdf/actions/runs/38893135754/job/116947203950) |
| Main CI / dependency policy | Pass | [116947204210](https://github.com/btecme/bdf/actions/runs/38893135754/job/116947204210) |
| Main CI / fuzz | Skipped by its existing PR-event condition | [116947205410](https://github.com/btecme/bdf/actions/runs/38893135754/job/116947205410) |
| FreeBSD | Pass; worker suite 27/27, production refusal included | [116947205291](https://github.com/btecme/bdf/actions/runs/38893136165/job/116947205291) |
| ARM64 package | Pass; target added to active Rust 1.99.0, ARM64 package produced | [116947205643](https://github.com/btecme/bdf/actions/runs/38893136176/job/116947205643) |
| ARM64 native install/run | Pass | [116951576905](https://github.com/btecme/bdf/actions/runs/38893136176/job/116951576905) |
| Packaging lint | Pass | [38893136113](https://github.com/btecme/bdf/actions/runs/38893136113) |

The native ARM64 job installs the MSI, checks both executables' ARM64 PE type, runs the CLI's
`--version` and uninstalls successfully. It does not establish full native ARM64 Rust-test,
document-workflow or GUI qualification. The hosted non-Windows jobs run default features;
fault-injection tests have still not been executed on Linux or FreeBSD. Windows remains the only
supported production platform for the worker's containment/memory-monitoring path. The fuzz skip
is not a fuzz pass. Per-OS conditional skips in otherwise passing jobs are unchanged.

## Evidence, documentation correction and scope

Local evidence is private under `target/bdf-evidence/BUILD-001-hosted-codex-retest/`: exact Cargo
argument arrays, exits/durations, logs, the independent packaging-script probe, decoded hosted logs,
GitHub metadata and artifact/source identities. The files are pinned by `MANIFEST.sha256`.
That manifest covers **32 files** and hashes to
`463082c63606590fd060cd588b95182049563d67b278684b38775c7429b05d10`.
It includes a hash inventory of the 22 supplied CC correction-evidence files; no separate builder
manifest digest was supplied for comparison.

CC's simulation log `green-fix2-simulated-no-containment.log` says **24 passed / 3 failed**, while the
written account said two expected Windows-only failures. The third is the cross-platform assertion
that a Windows test helper should not opt out of containment; the simulation intentionally made it
opt out. This is corrected in the hosted evidence record. The actual, unmodified Windows/Linux/
FreeBSD tests pass; simulation is not substituted for native platform evidence.

This review closes the three failures in [the bounded CI follow-up](../bdf-hosted-ci-followup.md).
It does not re-review every earlier BUILD-001 UI change, ratify BUILD-002 performance compliance,
finish BUILD-003's workflow ownership/pinning, or qualify production release behavior. The current
configured hosted baseline CI is green; the work-item's overall verification status remains separate.
At completion of this retest, no product/workflow code had changed and no hosted rerun, commit,
push, merge, release or registration had been performed; the PR was still a draft. The owner
subsequently authorized pushing this review record and merging PR #1 after final-head CI passes.
This authorization does not change the qualification limits recorded above.

## Merge preparation: timestamp-dependent batch tests

The owner authorized pushing the independent review record and merging PR #1 after green CI.
The documentation commit `6a273136` triggered a fresh run. Windows x64, wasm, dependency policy,
packaging lint and ARM64 packaging/native installation passed, but [Ubuntu job 116972788807](https://github.com/btecme/bdf/actions/runs/38900956477/job/116972788807)
failed two existing `pdfcraft-automation --test batch` assertions. This supersedes any implication
that later heads automatically inherit the green result on `de8d6fcc`.

Both assertions called `pdf()` again to generate expected bytes after processing. Decoding the
logged object streams shows the original fixture has `/ModDate (D:20261016123730Z)` and the
new expectation has `/ModDate (D:20261016123731Z)`. The generated timestamp alone explains the
byte differences. The tests must compare outputs with the exact input bytes they supplied.

The correction retains one `original` fixture in each failing test and compares both source and
numbered copies against it. The concurrent-writer test still checks distinct output names, exact
bytes and absence of staging files. The related optimized-size test also uses its actual source
fixture's length. No product code, timestamp behavior, workflow or test exclusion changes.

A temporary two-second delay before the assertions reproduces both old failures locally:
**6 passed / 2 failed**. With the correction, the same delayed probe passes **8/8** on Windows.
The delays are then removed and the final batch suite passes **8/8** without them. The first
local attempt reached its 240-second compilation timeout before tests; the cached retry completed
under a 600-second limit with two build jobs. This timeout is not reported as a test result.

The final scoped automation Clippy check (all targets, `-D warnings`), format, layers, assets
and wasm checks all pass. Local commands, exits and logs are under
`target/bdf-evidence/BUILD-001-merge/`, pinned by its 20-file `MANIFEST.sha256`
(SHA-256 `aff3ab0fa60de946abdaf2442679d242ffcce81914e6dbbbe8215b8e958f178b`). The earlier 32-file retest manifest remains unchanged. Final-head hosted CI is required before merge; the
PR records its live result without requiring another documentation-only commit to rerun CI.

## Revision history

| Date | Change |
|---|---|
| 2026-10-16 | Independent source review, local retests and current-head hosted log verification close all three CI failures. |
| 2026-10-16 | Merge preparation exposed timestamp-dependent batch assertions; retained original fixtures and verified the delayed before/after probe. |
