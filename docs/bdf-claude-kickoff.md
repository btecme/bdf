# Claude Code kickoff — BDF BUILD-001

Prepared 2026-10-13. This is a handoff prompt, not evidence that any implementation has run. Paste the prompt below into Claude Code in this repository. The first assignment is deliberately bounded to establishing a verified baseline.

## Prompt

We are building BDF, a local-first PDF utility forked from storytold/pdfcraft. Windows 11 x64 desktop is the priority; web follows. Our first product scope is view/search, annotate, fill existing AcroForms, organize pages, print, undo/redo and reliable saves/recovery. Claude Code implements and Codex independently reviews.

Read `AGENTS.md` and `CLAUDE.md`, then `docs/bdf-build-spec.md`, `docs/bdf-work-items.toml`, `docs/bdf-prd-draft.md` and `docs/bdf-foundation-review.md`. Use these committed BDF plans when referenced local `plan/` files are absent. Preserve repository safety, provenance, Rust-only, layering and opt-in automation rules.

Implement **BUILD-001 only**. Establish a reproducible Windows baseline and verify the pending branding cleanup before new feature development. Do not automatically continue through later work items, redesign the renderer/UI, merge upstream main, mass-rename crates, activate/register MCP, publish or merge a release.

1. Inspect HEAD, branch, remotes and worktree. There are prior uncommitted review/PRD/branding changes. Preserve them and identify exactly which changes you test. Use an appropriate task branch; do not reset or stash others' work or commit unrelated changes. If preparing a PR, follow the repository contributor-credit instructions.
2. Check Rust/Cargo, MSVC Build Tools, Windows SDK and graphics/runtime prerequisites. The earlier environment had no usable Rust/Cargo. Report missing prerequisites concretely; follow the machine's installation/approval policy if setup is needed. Use official publishers. Do not call unavailable checks passing. After a successful build, record/pin the exact compatible toolchain and align CI; root MSRV is an input, not proof that every dependency builds with it.
3. Build the existing desktop and CLI with `--locked`, two build jobs and a finite timeout. Run scoped fmt/clippy/tests, layers, the prescribed asset generator/check and parity. Get hosted workspace evidence where possible; do not run an uncontrolled full local workspace sweep. Test optional font input when relevant and preserve behavior without it.
4. Verify the prior removal of restricted ArtCraft visual marks and the marked screenshot. Keep upstream textual attribution, copyright and license notices. Run `cargo xtask assets --write` and `cargo xtask assets`; confirm they agree with the generated attribution already in the tree. Check the About dialog through the real UI with opt-in control. Do not restore restricted assets merely to satisfy old tests.
5. Run a synthetic headless open/create, render, rotate, undo/redo, save and reopen smoke using the current CLI schema. Inspect the produced `page_render` image. Launch the actual desktop with `--control` in a private location, exercise inspect/open/save behavior, and capture an unpublished UI image with system fonts disabled. Keep files/evidence under an ignored target directory and do not use proprietary documents or fonts. Stop and triage any crash/data loss with a small synthetic reproducer.
6. Produce `docs/bdf-evidence/BUILD-001.md` with baseline/reviewed snapshot, environment/toolchain, exact commands and exit codes, artifact paths/hashes, baseline failures, skipped/unavailable checks, and whether BUILD-002 is ready. Update work-item status honestly: source inspection or a subset of tests does not justify `verified`. Update the repository roadmap as required. Commit only verified green changes and follow the PR template if preparing a PR; do not merge or push main.

Your final report must distinguish existing functionality from changes you made, list actual verification and its limits, and identify the next bounded task. Codex will review the resulting diff and evidence. If an environment or baseline defect prevents completion, report the concrete blocker and preserved state without expanding the assignment into unrelated feature work.

## Starting commands

These refer to existing packages; supervise each with finite timeouts. They are a starting point, not a substitute for all BUILD-001 evidence.

```powershell
git status --short
git rev-parse HEAD
rustc --version
cargo --version
cargo build --locked -p pdfcraft -p pdfcraft-cli -j 2
cargo fmt --all -- --check
cargo clippy -p pdfcraft-ui-egui -p pdfcraft -p pdfcraft-cli -j 2 --all-targets -- -D warnings
cargo test -p pdfcraft-ui-egui -p pdfcraft -p pdfcraft-cli -j 2
cargo xtask layers
cargo xtask assets --write
cargo xtask assets
cargo xtask parity
```

Use `target/debug/pdfcraft-cli.exe tools` to inspect current schemas before constructing smoke scripts. Current `run --script` files are JSON arrays of `{ "tool": "...", "args": {...}, "out": "optional-output-path" }`; `doc_open` returns session-local integer IDs. Always set `--root` to the synthetic workspace for smoke runs. Do not assume future service/grant/revision contracts already exist.

Windows setup reference: [the official rustup Windows guide](https://rust-lang.github.io/rustup/installation/windows.html). The build spec describes later Windows persistence/ACL/worker requirements and their official platform references.

## Revision history

| Date | Revision | Change |
|---|---|---|
| 2026-10-13 | 0.1 | Initial bounded baseline-verification handoff to Claude Code. |
