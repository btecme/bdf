# BDF hosted checks: what each job promises

Status: **contract and repository-protection specification, 2026-10-17.** BUILD-003 deliverable 2. Nothing in this page changes a repository setting, and nothing in it is an observed run result unless it says so and links the run.

This page separates three things that are easy to blur:

1. **The contract**: which job exists, the exact check name GitHub shows, what a pass proves and what it does not.
2. **The protection setup**: which checks the `main` branch should require. This is written down here; it is not configured. Changing repository settings is the owner's action.
3. **Observed results**: what a run actually did. Those live in the evidence files (for example [BUILD-003.md](bdf-evidence/BUILD-003.md)), with run links, and only for the commit they were observed on.

A job that cannot fail on the thing it names is not listed as a check. Where a qualification step does not exist yet, the table says **not implemented** and no placeholder job stands in for it: a green check that ran nothing would read as evidence that does not exist.

## 1. Jobs that exist

Check names are the names GitHub shows (`workflow job name`). The workflow files are the source; each file is pinned and covered by `cargo xtask pins`.

| Check name | Workflow | Runs on | What a pass proves | What it does not prove |
|---|---|---|---|---|
| `test (ubuntu-latest)` | `ci.yml` | every PR, push to `main`, nightly | rustfmt, clippy `-D warnings` over the workspace, the whole workspace test suite with the craft-fonts input, crate layering, asset policy, and that the CLI builds without the MCP feature | UI behaviour on a real display; Windows-only paths; the BDF qualification corpus |
| `test (windows-latest)` | `ci.yml` | every PR, push to `main`, nightly | clippy and the workspace tests on Windows with the executable resources required | installation, upgrade and uninstall (see the package workflows); real printers; mixed DPI |
| `parity` | `ci.yml` | every PR, push to `main`, nightly | `parity/acrobat-features.toml` is consistent with the command registry, the automation tools and the tests it cites | that a feature "shipped" there behaves like Acrobat. The checklist records existence and cited tests, not fidelity (see `parity/` notes) |
| `BDF gates (pins, network, core inventory)` | `ci.yml` | every PR, push to `main`, nightly | every workflow reference is pinned to a commit SHA and no `.github` file is unreadable (`cargo xtask pins`); no workspace crate other than the desktop app depends on an HTTP or WebSocket library (`cargo xtask network`); the core capability inventory classifies every tool, command, parameter and enumerated mode and matches the code (14 tests, with the count checked so a filter that matched nothing cannot pass) | that the inventory is **enforced**. Nothing calls the profile at run time yet (BUILD-005 to BUILD-008); see [the inventory](bdf-capability-inventory.md) |
| `wasm32 check` | `ci.yml` | every PR, push to `main`, nightly | the library crates below the application layer build for `wasm32-unknown-unknown` | that the web application builds or runs; it does not build `pdfcraft-web` |
| `dependency licences and advisories` | `ci.yml` | every PR, push to `main`, nightly | `cargo deny check` passes against the committed `deny.toml` and the advisory database as fetched by the action | advisories whose id is still `RUSTSEC-0000-0000`; cargo-deny ignores those |
| `fuzz (mutation, 15 min)` | `ci.yml` | nightly schedule and manual dispatch only | no new crash finding in a 15-minute seeded mutation run | anything on a pull request: it is skipped there by design, so it must never be a required check |
| `Windows x64 MSI and portable zip (unsigned)` | `release.yml` ("Package (Windows x64, unsigned)") | manual dispatch and PRs touching packaging paths | the WiX installer helper refuses a tampered or wrong-hash package and installs only the verified one (self-test), the MSI builds, installs, runs `pdfcraft-cli --version`, and uninstalls | signing (the job has no secrets and publishes nothing); that the artifact is a release |
| `package (cross-compiled on x64)` and `install and run on ARM64` | `windows-arm64.yml` | pushes and PRs touching its paths, weekly, manual | the ARM64 package builds and installs and runs natively on an ARM64 runner | the same limits as the x64 package |
| `FreeBSD 15 (build + test)` | `freebsd.yml` | pushes and PRs touching its paths, manual | the workspace builds and tests inside a FreeBSD 15 VM | the rustup bootstrap in the VM is fetched at run time (recorded in BUILD-003.md) |
| `lint` | `packaging-lint.yml` ("Packaging lint") | pushes and PRs touching workflow, packaging or icon paths | actionlint 1.7.12 (sha256-checked), shellcheck, PowerShell parse, XML, icon ids, desktop and AppStream files and the Flatpak manifest agree | that the scripts do what they say; the package workflows run them |

Path-filtered workflows run only when their paths change, so GitHub reports no check at all on other pull requests. A required check that never reports blocks the pull request, so only checks from `ci.yml`, which has no path filter on pull requests, are suitable to require (section 3).

## 2. Qualification steps that do not exist yet

These are named by the PRD and the foundation review as release gates. None is a job today, and none is represented by a stand-in.

| Gate | Prerequisite that is missing |
|---|---|
| BDF qualification corpus (100 fixtures now, 500 for the pilot) with semantic and visual review | The corpus generator and strict oracle exist (`cargo xtask bdf-corpus`, `bdf-oracle`, BUILD-002) but need the MuPDF 1.28.5 oracle binary, which is a third-party download verified by hand and not fetched by any workflow. A hosted job needs a reviewed, hash-pinned way to obtain it. |
| Worker supervision, fault injection, memory ceilings | The supervised worker is integrated in BUILD-010. |
| Real-UI acceptance of each v1 workflow, with save and reopen and render checks | Needs the desktop-core profile enforced (BUILD-005 to BUILD-008) and a display-capable job. |
| Performance budgets (`cargo xtask bdf-bench`) | Budgets are acceptance targets, not measured results; runner hardware is not stable enough to gate on without a calibrated baseline. |
| Signed installer, checksums, SBOM, provenance | No signing identity exists for BDF. `release.yml` is unsigned by design and `docs/releasing.md` says not to configure signing secrets yet. |
| Web application build and browser runtime | Web is not a release target; the library `wasm32 check` stays. |

## 3. Repository protection to apply (not applied)

Recommended for `main`, to be set by the repository owner. The names are the exact check names in section 1.

* Pull requests required; direct pushes and force pushes disabled.
* Required status checks, branch up to date before merging: `test (ubuntu-latest)`, `test (windows-latest)`, `parity`, `BDF gates (pins, network, core inventory)`, `wasm32 check`, `dependency licences and advisories`.
* Not required: `fuzz` (does not run on pull requests) and the path-filtered package, ARM64, FreeBSD and lint checks (they do not report on every pull request). A change to packaging or workflows is checked by them when it touches their paths and is read in review.
* Actions permissions: read-only default token (the workflows already declare `permissions: contents: read`), and an allowlist of actions that matches the pinned set in `.github`.
* Review: the project reviews with Codex before merge; whether that is a required reviewer is the owner's choice.

**Observed 2026-10-17, through the unauthenticated GitHub API:** `GET /repos/btecme/bdf/branches/main` reports `"protected": false`. The branch-protection endpoint needs repository admin access, so classic protection rules and rulesets could not be read in more detail. Treat `main` as unprotected until the owner confirms a setting. Pull requests #1 to #3 were reviewed and merged by convention, not by enforcement.

## 4. Keeping this page true

* A new job in `ci.yml` is added to section 1 in the same change, with what it does not prove.
* A step that is skipped, conditional or `continue-on-error` is named as such here; none exists today.
* The `ci` aggregate in `xtask` (`cargo xtask ci`) runs the same gates locally as one command: fmt, clippy, tests, layers, pins, network, wasm, assets, deny and parity, stopping at the first failure. The hosted workflow reports each independently.
* A pass is evidence only for the commit it ran on.

## Revision history

| Date | Change |
|---|---|
| 2026-10-17 | First version (BUILD-003 deliverable 2): job contracts, missing qualification gates, protection to apply, observed `protected: false`. |
