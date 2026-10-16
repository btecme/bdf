# BUILD-003 evidence: release infrastructure and the core capability inventory

Status: **in progress; slice 1 (workflow pinning) done locally, not yet run on a hosted runner.** Work item BUILD-003 in [bdf-work-items.toml](../bdf-work-items.toml) stays `pending`. Branch `chore/build-003-release-pinning`, from the merged `main` (`f698f3a8`). Dated 2026-10-16. Local commands ran on Windows 11 x64 with Rust 1.99.0, `--offline --locked`, two jobs; raw logs are under the git-ignored `target/bdf-evidence/BUILD-003/`. Nothing here is pushed or merged.

## 1. Slice 1: pin every workflow reference (deliverable 1, finding F07)

**What was floating.** Four workflows called the organization's reusable workflows at `@main` ("intentionally tracked ... so improvements propagate"): `freebsd.yml`, `packaging-lint.yml`, `windows-arm64.yml` and `release.yml`. `ci.yml` used `EmbarkStudios/cargo-deny-action@v2` (a moving tag) and had no `permissions` block. A push to the upstream organization's `main` could therefore change what this fork's CI, and its release job, run.

**What changed.**

| Reference | Before | After |
|---|---|---|
| `storytold/craft-common-actions` reusable workflows (4) | `@main` | `@43f4e83c68764f9d880f8a6c10328bc550465f2b`, which is the tip of that repository's `main` on 2026-10-16 and the commit Codex recorded the first hosted run of PR #1 resolving |
| `EmbarkStudios/cargo-deny-action` | `@v2` | `@3c6349835b2b7b196a839186cb8b78e02f7b5f25 # v2.1.1` (the commit behind tag `v2`; on the action's `main` line) |
| `dtolnay/rust-toolchain` in `ci.yml` (3 uses) | `@686976e1... # stable` | `@683b588c487a69d01f6c6ddf40f036ff682cc2c8 # v1`, the commit the reusable workflows already use, tag `v1` on the action's default branch |
| `ci.yml` token permissions | none declared (repository default) | `contents: read` for the whole workflow |

Why the `rust-toolchain` change: GitHub reports `686976e1` as diverged from the action's `stable` branch (2 commits ahead, 1 behind), so it is not part of the branch's current history and could not be tied to a tag. Every use passes an explicit `toolchain:` input, so the commit's only difference (its default toolchain) did not matter; the tagged commit is verifiable. The other pinned actions were checked against the upstream repositories' refs: `actions/checkout` `3d3c42e5` is tag `v7.0.1`; `actions/upload-artifact` `cf430e03` is `v7.0.2`; `actions/download-artifact` `9000827c` is `v8.0.2`; `Swatinem/rust-cache` `6323deb1` is `v2.9.2`; `vmactions/freebsd-vm` `c46abacb` is `v1.5.9`.

**A guard, so it stays pinned.** `cargo xtask pins` (and a test in the xtask suite, so it runs wherever tests run) reads every `.yml` under `.github/workflows` and `.github/actions` and fails on any `uses:` that is not a full 40-hex commit SHA, a local `./` path, or a `docker://` image with a `@sha256:` digest. Red before the change: the test reported exactly the five unpinned references (`red-pins-tests.log`). Green after: 17 references checked, 0 unpinned; xtask clippy `-D warnings` and 99 tests pass (`green-pins-*.log`). A mutation that makes the check always pass fails two of the four tests (`red-pins-mutation-problem-always-none.log`). The scan is also in `cargo xtask ci`.

## 2. What this does not achieve: the acceptance "release execution cannot change through an unreviewed remote branch ref" is NOT yet met

Pinning the caller fixes which commit of the *reusable workflow* runs. It does not reach the `uses:` lines **inside** it, which GitHub resolves when the job runs. Reading the four workflows and their composite actions at `43f4e83c` (copies in `craft-common-actions-43f4e83c/`; read as text, nothing executed) finds **17 nested references to `storytold/craft-common-actions/actions/*@main`**: in `release.yml` (`resolve-version`, `checkout-craft-fonts` x5, `install-wix`, `install-linux-deps`, `install-trunk`, `install-riscv64-cross`, `install-nfpm`, `draft-release`), `windows-arm64.yml` (`install-wix`), `freebsd.yml` (`checkout-craft-fonts`), `packaging-lint.yml` (`packaging-lint`) and one inside the `install-linux-deps` action (`install-nfpm`). Third-party actions inside are SHA-pinned, but those composite actions can still change underneath a pinned caller. The release workflow also downloads `nfpm`, `trunk` and `actionlint` release binaries by version from GitHub.

`release.yml` is the sensitive one: it runs on a push to the `release` branch or a manual dispatch with `contents: write`, and the caller passes it the upstream-style signing secrets (Apple certificate and notarization, Azure signing, Windows certificate). No such secrets are known to be configured in this fork, and BUILD-003 requires removing reliance on the upstream signing identity; until it is done, **do not configure signing secrets in this repository**, and nothing should be pushed to a `release` branch.

**Why this was not "fixed" by copying.** The cleanest way to close the gap is to own the workflows (local copies with nested references rewritten to local paths). `storytold/craft-common-actions` is public but has **no licence file** (GitHub reports none), so copying its code into this repository is not allowed by default. The honest options are: get a licence or written permission from the organization (then vendor and review), or write the small workflows we need here from scratch (`freebsd`, `packaging-lint` and `windows-arm64` are 1 to 7 KB; the 43 KB release workflow must be replaced anyway because it embeds the upstream signing and publishing flow). Which one is a decision for the owner; neither was started.

## 3. Hosted validation

Not yet run. The workflow edits only change which commit and permissions are used, but they can only be proven on GitHub: the four reusable calls must still resolve at the pinned SHA, `cargo-deny-action` at the new SHA must still run, and `ci.yml` must still pass with `contents: read`. This needs the branch pushed and a pull request, which has not been authorized for this branch.

## 4. Not started

Deliverables 2 to 5 (hosted parity and BDF qualification job contracts; the core capability inventory and desktop-core profile with unknown-defaults-to-denied; the BDF identity migration record; confirming network and MCP stay off by default) and the acceptance items for them. Required-job and repository-protection documentation must distinguish settings from observed run results; branch protection is a GitHub setting only the owner can change.

## Revision history

| Date | Change |
|---|---|
| 2026-10-16 | Slice 1: references pinned and permissions minimized in the caller workflows; `cargo xtask pins` added; nested floating references and the licence gap recorded as open. |
