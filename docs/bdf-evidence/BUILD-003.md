# BUILD-003 evidence: release infrastructure and the core capability inventory

Status: **in progress. Slices 1 and 2 (workflow pinning and owning the four reusable workflows) are written and checked locally; none of it has run on a hosted runner yet.** Work item BUILD-003 in [bdf-work-items.toml](../bdf-work-items.toml) stays `pending`. Branch `chore/build-003-release-pinning`, from the merged `main` (`f698f3a8`), pushed to the fork. Dated 2026-10-16. Local commands ran on Windows 11 x64 with Rust 1.99.0, `--offline --locked`, two jobs; raw logs are under the git-ignored `target/bdf-evidence/BUILD-003/`.

## 1. Slice 1: pin every workflow reference (deliverable 1, finding F07)

**What was floating.** Four workflows called the organization's reusable workflows at `@main` (`freebsd.yml`, `packaging-lint.yml`, `windows-arm64.yml`, `release.yml`). `ci.yml` used `EmbarkStudios/cargo-deny-action@v2` (a moving tag) and declared no token permissions.

| Reference | Before | After |
|---|---|---|
| `EmbarkStudios/cargo-deny-action` | `@v2` | `@3c6349835b2b7b196a839186cb8b78e02f7b5f25 # v2.1.1` (the commit behind tag `v2`; on the action's `main` line) |
| `dtolnay/rust-toolchain` in `ci.yml` (3 uses) | `@686976e1... # stable` | `@683b588c487a69d01f6c6ddf40f036ff682cc2c8 # v1` (tag `v1`, the action's default branch) |
| `ci.yml` token permissions | none declared | `contents: read` |

Why the `rust-toolchain` change: GitHub reports `686976e1` as diverged from the action's `stable` branch (2 commits ahead, 1 behind), so it could not be tied to a tag; every use passes an explicit `toolchain:`, so the tagged commit is equivalent and verifiable. The other pinned actions were checked against the upstream repositories' refs: `actions/checkout` `3d3c42e5` is tag `v7.0.1`; `actions/upload-artifact` `cf430e03` is `v7.0.2`; `actions/download-artifact` `9000827c` is `v8.0.2`; `Swatinem/rust-cache` `6323deb1` is `v2.9.2`; `vmactions/freebsd-vm` `c46abacb` is `v1.5.9`.

**A guard.** `cargo xtask pins` (also a test in the xtask suite and a step of `cargo xtask ci`) reads every `.yml` under `.github/workflows` and `.github/actions` and fails on any `uses:` that is not a full 40-hex commit SHA, a local `./` path, or a `docker://` image with a `@sha256:` digest. Red on the unmodified repository: exactly five unpinned references (`red-pins-tests.log`). A mutation that makes the check always pass fails two of its four tests (`red-pins-mutation-problem-always-none.log`). Green: `green-pins-*.log`.

## 2. Slice 2: the four reusable workflows are replaced by workflows in this repository

**The gap slice 1 could not close.** Pinning the caller to a commit of `storytold/craft-common-actions` fixes which reusable workflow runs, but not the `uses:` lines *inside* it, which GitHub resolves when the job runs. Reading the four workflows and their composite actions at the pinned commit (copies in `craft-common-actions-43f4e83c/`; read as text) found **17 nested references to `storytold/craft-common-actions/actions/*@main`** (`release.yml` 13, `windows-arm64.yml` 1, `freebsd.yml` 1, `packaging-lint.yml` 1, one inside the `install-linux-deps` action). The acceptance "release execution cannot change through an unreviewed remote branch ref" cannot be met by pinning callers. Separately, the release workflow was triggered by any push to a `release` branch with `contents: write` and was handed the upstream-style Apple, Windows and Azure signing secrets.

**Decision (owner, 2026-10-16): build what is needed here; no licence is being requested.** `craft-common-actions` is public but has no licence file, so its code is not copied. The replacements are written for BDF from this repository's own scripts, the inputs recorded in our retained FreeBSD job log, and the public documentation of the tools used. Disclosure: before the decision I had read the whole upstream `windows-arm64.yml` (to diagnose the ARM64 failure) and the `uses:` lists of the others; the new files were written from the requirements, not by editing those texts, and their structure is the obvious one for the job (package, then install on ARM64).

| Workflow | Now | Notes |
|---|---|---|
| `windows-arm64.yml` | package on `windows-latest` (Rust 1.99.0 with the ARM64 target installed directly, WiX, `package.ps1 -Arch arm64`, artifact), then install on `windows-11-arm` | The install step is `packaging/windows/install-test.ps1`: installs the MSI silently, checks the PE machine type of `pdfcraft.exe` and `pdfcraft-cli.exe`, the models folder, runs `pdfcraft-cli --version`, uninstalls and checks removal. The `Initialize-RustTarget` safety net from BUILD-001 stays in `package.ps1`. |
| `release.yml` | **Windows x64, unsigned, publishes nothing.** `workflow_dispatch`, and a dry run on pull requests that touch `packaging/windows/**`. `contents: read`, no secrets. | Builds the MSI and portable zip, runs the static MSI checks (inside `package.ps1`) and `install-test.ps1` for real, writes `SHA256SUMS.txt`, uploads an artifact named `...-UNSIGNED`. The `release`-branch push trigger, the signing secrets, the GitHub Release step and the macOS, Linux, Flatpak, FreeBSD and web jobs are gone; they return as reviewed work when a signing identity and a distribution endpoint exist. Package qualification stays pending, not faked. |
| `packaging-lint.yml` | actionlint 1.7.12 (release tarball, SHA-256 checked before unpacking), shellcheck, PowerShell parse, xmllint, MSI icon-id rule, desktop-file and AppStream validation, Flatpak manifests' agreement | These are the checks `docs/releasing.md` listed for the old job. I could not compare against its exact steps, so any check it ran that is not in this list is a gap. |
| `freebsd.yml` | FreeBSD 15.1 VM on `ubuntu-latest` through `vmactions/freebsd-vm` (SHA-pinned), the same package list and resources as the retained log, the toolchain from `rust-toolchain.toml`, craft-fonts at the CI-pinned commit and required, `cargo build --workspace --all-targets --locked` then `cargo test --workspace --locked` | Packaging (`packaging/freebsd/package.sh`) is not run, as before. |

Supporting scripts (new, in this repository): `packaging/windows/install-wix.ps1` installs the WiX tool only from the `wix.5.0.2.nupkg` whose SHA-512 matches the value recorded in the script (checked now by streaming the official package from nuget.org: 4,587,200 bytes, `z+fu1deT...qdOo2OA==`, equal to nuget.org's record; the package's owners are `wixtoolset`, `firegiant`, `robmen`, licence MS-RL), and `packaging/windows/install-test.ps1`. Both parse without errors; nothing was installed locally.

Local checks of this slice: `cargo xtask pins` reports 28 references, 0 unpinned; the whole packaging directory's PowerShell parses; `cargo fmt`, layers and assets pass. No local check can stand in for a hosted runner here.

## 3. Hosted validation: pending

None of the four replacements has run. Each can fail for reasons only a runner shows (a package name on FreeBSD, an actionlint finding in an existing workflow, a missing tool on the ARM64 image, craft-fonts paths on Windows). They are validated by the pull request for this branch, and a failure is fixed on the branch, not hidden. The `release.yml` dry run runs only when the pull request touches `packaging/windows/**`, which this branch does.

## 4. Open and not started

* Hosted results for the four workflows, and the first run of `cargo xtask pins` in CI (it is part of the xtask tests).
* Downloads inside our own workflows and scripts: the WiX package and the actionlint tarball are hash-checked; the `rustup` bootstrap script on FreeBSD (`https://sh.rustup.rs`) is fetched at run time and is not hash-pinned (it is a script that changes with rustup releases; pinning it needs a decision on how much to trust the first fetch). The OCR models fetched by `cargo xtask models` are verified by SHA-256 against `ATTRIBUTION.toml`.
* `cargo-deny-action` is a Docker action; its image is built from its own repository at the pinned commit, with the tool version it chooses. A digest or `cargo install --locked` of a pinned version would be tighter.
* Deliverables 2 to 5 (hosted parity and BDF qualification job contracts; the core capability inventory and desktop-core profile with unknown-defaults-to-denied; the BDF identity migration record; confirming network and MCP stay off by default) are not started. Required-job and repository-protection documentation must separate settings from observed run results; branch protection is a GitHub setting only the owner can change.
* The MSI still carries the upstream publisher name ("Learning Machines LLC") and the `ai.storyteller.pdfcraft` ids; that is the identity migration (deliverable 4), not done here.

## Revision history

| Date | Change |
|---|---|
| 2026-10-16 | Slice 1: references pinned, permissions minimized, `cargo xtask pins` added; nested floating references and the licence gap recorded. |
| 2026-10-16 | Slice 2: the four reusable workflows replaced by workflows in this repository after the owner's decision; unsigned Windows x64 package job; hash-checked WiX and actionlint; hosted validation pending. |
