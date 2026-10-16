# BUILD-001 — hosted CI observations

Status: **changes required; hosted baseline is not green**, observed 2026-10-16 on [draft PR #1](https://github.com/btecme/bdf/pull/1), head `9a14e6bfaeee56a0b072f432704896c5f15184fc`. All four workflows have finished: three failed, one passed.

## First ARM64 run

[Windows ARM64 run 38864435552](https://github.com/btecme/bdf/actions/runs/38864435552) failed in [package job 116853922573](https://github.com/btecme/bdf/actions/runs/38864435552/job/116853922573), at **Build and package (unsigned)**. The dependent **install and run on ARM64** job was skipped because the package job failed.

The log shows a toolchain/target mismatch:

1. The inherited reusable workflow installs `stable` with target `aarch64-pc-windows-msvc`. In this run stable resolved to **1.99.1**.
2. Rustup explicitly reports that the repository's `rust-toolchain.toml` selects **1.99.0** instead.
3. The packaging script invokes plain `cargo build --release --locked ... --target aarch64-pc-windows-msvc`, selecting the pinned toolchain.
4. Compilation fails in `cfg-if` with **E0463: can't find crate for `core`**, noting that the ARM64 target may not be installed. This is build setup evidence, not a demonstrated PDF-engine or Windows x64 application defect.

The correction should install the target for the compiler the build actually uses, and keep the packaging toolchain consistent with the repository pin. For example, the CI setup needs the equivalent of:

```text
rustup target add --toolchain 1.99.0 aarch64-pc-windows-msvc
```

The local ARM64 workflow calls a remote reusable workflow, so placing a step into the caller job is not directly possible. Check the reusable workflow's supported inputs or make the fork own the required setup. Do not remove the compiler pin, weaken checks or add targets globally to every developer's toolchain just to suppress this failure. Pinning the remote workflow to a reviewed immutable revision remains BUILD-003 work; this run resolved its `@main` to `43f4e83c68764f9d880f8a6c10328bc550465f2b`.

## Final results of the first hosted run

| Workflow / job | Result | Evidence |
|---|---|---|
| Main CI / Windows x64 | Pass | [Job 116853922129](https://github.com/btecme/bdf/actions/runs/38864435164/job/116853922129) |
| Main CI / wasm | Pass | [Job 116853922105](https://github.com/btecme/bdf/actions/runs/38864435164/job/116853922105) |
| Main CI / dependency policy | Pass | [Job 116853921885](https://github.com/btecme/bdf/actions/runs/38864435164/job/116853921885) |
| Main CI / Ubuntu | Fail at Clippy; tests not run | [Job 116853922137](https://github.com/btecme/bdf/actions/runs/38864435164/job/116853922137) |
| Packaging lint | Pass | [Run 38864435507](https://github.com/btecme/bdf/actions/runs/38864435507) |
| Windows ARM64 | Package fails; dependent native install/run skipped | [Run 38864435552](https://github.com/btecme/bdf/actions/runs/38864435552) |
| FreeBSD | Worker integration suite: 12 pass, 12 fail | [Job 116853923033](https://github.com/btecme/bdf/actions/runs/38864435579/job/116853923033) |

The main CI fuzz job was skipped by its event condition; it is configured for schedule/manual runs, not this PR event. It is not a passing fuzz result.

### Ubuntu: non-Windows lint error

The error is `calls to std::mem::drop with a value that implements Copy does nothing`, at `xtask/src/bdf/proc.rs:130`. On Windows, `job` owns a real job handle; on non-Windows it is `Option<()>`. `-D warnings` turns the latter's `dropping-copy-types` warning into a failure. This occurs before the test steps, so no Ubuntu test pass can be claimed.

Correct the platform-dependent cleanup while preserving the Windows job lifetime and its explicit closure before joining output-drain threads. Merely deleting the drop or globally suppressing warnings would not establish the intended behavior.

### FreeBSD: worker test configuration does not match the platform

The default worker configuration requires parent-death containment and sets `allow_uncontained = false`. Non-Windows containment deliberately returns an error. Both the ordinary and fake-peer test helpers construct that default configuration, so 12 integration tests fail at startup with `Containment("parent-death containment is implemented only on Windows ...")`, before exercising the behavior they assert. This is the non-Windows test issue flagged in the draft PR description; it was not a VM timeout.

Keep production startup fail-closed. The protocol/render/cancellation tests may explicitly opt into uncontained execution within non-Windows test helpers. Separately prove that an ordinary non-Windows production config refuses startup, and keep Windows job/parent-death guarantees tested on Windows. Audit fault-injection tests for OS-specific memory monitoring and CPU limits; an uncontained test must not claim those guarantees. Fixing Ubuntu Clippy alone is likely to expose the same worker-test failures there.

The exact two decoded logs are retained under `target/bdf-evidence/BUILD-002-pr-2026-10-16/linux-clippy-job.log` and `freebsd-test-job.log`. A bounded [CC follow-up](../bdf-hosted-ci-followup.md) covers these three corrections and their validation.

ARM64 follows x64 qualification in the build spec. Its failure remains visible and needs correction, while the required core baseline gates are assessed separately. No workflow or product code was changed, no rerun was requested and nothing was pushed or merged during this diagnosis. The decoded failed-job log is retained privately under `target/bdf-evidence/BUILD-002-pr-2026-10-16/windows-arm64-package-job.log`.

## Corrections and the second hosted run (head `a3335589`)

Three focused commits on top of `9a14e6bf`, pushed to the fork's branch without force (the draft PR stays draft; nothing merged). Local evidence is under the git-ignored `target/bdf-evidence/BUILD-002-hosted-ci-fix/`. Local checks ran on Windows 11 x64 with Rust 1.99.0, `--offline --locked`, two jobs; nothing local stands in for a Linux, FreeBSD or ARM64 result, which come only from the hosted jobs.

| # | Commit | Failure (first run) | Change | Evidence before the fix | Evidence after |
|---|---|---|---|---|---|
| 1 | `e0b7738a` | Ubuntu Clippy, [job 116853922137](https://github.com/btecme/bdf/actions/runs/38864435164/job/116853922137): `dropping-copy-types` at `xtask/src/bdf/proc.rs:130` | `drop(job)` is `#[cfg(windows)]`, still before the stdout/stderr drain threads are joined. Not moved after the joins, not removed, no lint suppressed. | **Local lint reproduction (not Linux):** a scratch crate compiling `proc.rs` with the Windows cfgs off fails with the hosted error (`red-fix1-nonwindows-view-clippy.log`). **Windows regressions:** two new tests (`closing_the_job_frees_a_pipe_held_by_a_process_the_child_left_behind`, `a_timeout_also_ends_the_children_of_the_child_when_confined`) fail with the closure moved after the joins, both after 59.4 s (`red-fix1-cleanup-after-join.log`). | The scratch crate is clean (`green-fix1-nonwindows-view-clippy.log`); xtask clippy `-D warnings` and 95 tests pass (`green-fix1-*.log`). |
| 2 | `a1ab8c01` | FreeBSD, [job 116853923033](https://github.com/btecme/bdf/actions/runs/38864435579/job/116853923033): 12 worker integration tests died at startup with `Containment("parent-death containment is implemented only on Windows ...")` | The test helpers (`common::portable`, used by `cfg()` and `fake()`) opt in to `allow_uncontained` on non-Windows only. `Config::new(..).allow_uncontained` stays `false` and production stays fail-closed. New tests: the default is fail-closed on every platform; on non-Windows the default refuses startup, ends the process it spawned, and a `Session` built on it holds no worker; an opted-in worker reports `contained() == false` and has no memory counters. The memory-cap fault test is Windows-only; `process_gone` no longer relies on `/proc`, which FreeBSD lacks by default (it would have reported every process gone and asserted nothing). Module and field docs say Windows is the only platform with containment. | **Simulation, not FreeBSD:** with `contain` forced to fail on Windows, the portable worker tests fail 12 of the same kind plus 2 Windows-only job tests (`red-fix2-simulated-no-containment.log`). | With the helper change the 12 pass under the same simulation; the 2 Windows-only ones fail there by design, and the simulation hook was removed (`green-fix2-simulated-no-containment.log`). Real Windows runs: default 9 + 27, fault-injection 9 + 16 + 26, clippy `-D warnings` in both configurations (`green-fix2-*.log`). |
| 3 | `a3335589` | ARM64 package, [job 116853922573](https://github.com/btecme/bdf/actions/runs/38864435552/job/116853922573): E0463 `core` missing; the reusable workflow installed `stable` (1.99.1) with the target but cargo ran the 1.99.0 pin | The reusable workflow ([`craft-common-actions` at `43f4e83c`](https://github.com/storytold/craft-common-actions/blob/43f4e83c68764f9d880f8a6c10328bc550465f2b/.github/workflows/windows-arm64.yml), copy in the evidence directory) has no toolchain input and a caller job cannot mix `uses` and `steps`. So `packaging/windows/package.ps1` now calls `Initialize-RustTarget`: from the repository root it lists the installed targets of the compiler rustup selects and runs `rustup target add` for the missing one, the equivalent of `--toolchain 1.99.0` without hard-coding the pin. It does nothing when the target is present, so ordinary x64 builds download nothing. The pin, the workflow and the packaging/install steps are unchanged; only a comment was added to `windows-arm64.yml`. | **Local reproduction:** the same E0463 with the pinned compiler and the ARM64 target absent (`red-fix3-arm64-target-missing.log`). A script test fails before the function exists (`red-fix3-target-script.log`). | The script test passes with a stand-in `rustup` (one `target add`, run from the repository root, none when present) and with the real `rustup` (no-op for the host target) (`green-fix3-target-script.log`). The ARM64 target was **not** installed locally, so no local ARM64 build was made. |

Gates on the final code (local, Windows): `cargo fmt --all -- --check`, `cargo xtask layers`, `cargo xtask assets`, `cargo deny --offline check`, `cargo +1.95.0 check --all-targets` of `xtask` and `pdfcraft-worker` (also with `fault-injection`) all pass (`gate-*.log`).

### Hosted results on head `a3335589`

Observed through GitHub's public API (run and job conclusions); the job logs need a signed-in session, so the per-test counts inside the Linux and FreeBSD logs were **not read here** and are for the reviewer.

| Workflow / job | Result | Link |
|---|---|---|
| Main CI / test (windows-latest) | Pass | [job 116922193058](https://github.com/btecme/bdf/actions/runs/38885519321/job/116922193058) |
| Main CI / test (ubuntu-latest) | **Pass** (Clippy, then tests with craft-fonts, layering, assets); was failing | [job 116922193415](https://github.com/btecme/bdf/actions/runs/38885519321/job/116922193415) |
| Main CI / dependency licences and advisories | Pass | [job 116922193435](https://github.com/btecme/bdf/actions/runs/38885519321/job/116922193435) |
| Main CI / wasm32 check | Pass | [job 116922193474](https://github.com/btecme/bdf/actions/runs/38885519321/job/116922193474) |
| Main CI / fuzz | **Skipped** by its event condition (schedule / manual only); not a fuzz pass | [job 116922194590](https://github.com/btecme/bdf/actions/runs/38885519321/job/116922194590) |
| FreeBSD 15 (build + test) | **Pass** in 33 m 25 s; was failing | [job 116922193697](https://github.com/btecme/bdf/actions/runs/38885519778/job/116922193697) |
| Windows ARM64 / package (cross-compiled on x64) | **Pass**; was failing | [job 116922193943](https://github.com/btecme/bdf/actions/runs/38885519647/job/116922193943) |
| Windows ARM64 / install and run on ARM64 | **Pass** on `windows-11-arm`; was skipped | [job 116931152559](https://github.com/btecme/bdf/actions/runs/38885519647/job/116931152559) |
| Packaging lint | Pass | [run 38885519858](https://github.com/btecme/bdf/actions/runs/38885519858) |

Steps skipped inside the passing jobs are the per-OS conditions in `ci.yml` (for example the craft-fonts and Format steps skip on Windows, the Windows-only resource and plain-test steps skip on Ubuntu); none was disabled by these commits.

### What this does and does not show

* All four workflows pass on this head, with the one skipped job named above. That removes the three diagnosed failures; it does not by itself make BUILD-001 verified, which stays `pending_review` for Codex's review and the owner.
* **The fault-injection worker suite runs on no non-Windows platform:** the hosted jobs run `cargo test` without the `fault-injection` feature. I audited it by source: the memory-cap test is now Windows-only, the parent-death, job-object and CPU-limit tests were already Windows-only, and the remaining tests (crash, hang, cancel, protocol faults, recovery) are portable but have not been executed on Linux or FreeBSD.
* An uncontained worker on a non-Windows platform has no parent-death guarantee and no enforced memory cap or CPU limit; the production default refuses to start one. Windows is the only supported production platform for the worker until a containment mechanism exists elsewhere.
* The ARM64 package job installed `stable` 1.99.1 and then built with the pinned 1.99.0; this is wasteful but consistent. The reusable workflows are still tracked at `@main` (BUILD-003 work), so a change upstream can alter these results.
* The documentation-only commit `2ef5c62d` re-ran every workflow on the code of `a3335589`, with the same outcome: CI ([run 38889481958](https://github.com/btecme/bdf/actions/runs/38889481958): Windows, Ubuntu, wasm and dependency policy pass, fuzz skipped by its event condition), [FreeBSD](https://github.com/btecme/bdf/actions/runs/38889482752), [Windows ARM64](https://github.com/btecme/bdf/actions/runs/38889482720) (package and native install/run) and [Packaging lint](https://github.com/btecme/bdf/actions/runs/38889482753) all succeeded. (FreeBSD took far longer than the 33 minutes of the first pass; an in-app browser tab showed it "In progress" after the API had it completed, so conclusions here come from the API.) Any later documentation commit re-runs the workflows again; those runs add no information about the code.

## Revision history

| Date | Change |
|---|---|
| 2026-10-16 | Diagnosed the first hosted ARM64 failure; other running jobs have no final conclusion yet. |
| 2026-10-16 | Recorded final results: Windows x64/wasm/dependencies/packaging pass; Ubuntu lint, FreeBSD worker test configuration and ARM64 setup fail. Added a bounded follow-up. |
| 2026-10-16 | Added the three corrections, their local evidence and the second hosted run: all four workflows pass on `a3335589` (fuzz skipped by event). BUILD-001 stays unverified pending review. |
