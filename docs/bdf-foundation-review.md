# BDF foundation review

Status: initial source-level assessment, not release approval. Reviewed 2026-10-13.

Baseline: `7437bde378f43856eada40af25b983812e615e8a`, described locally as `v0.7.1-65-g7437bde3`. Origin is `btecme/bdf`; upstream is `storytold/pdfcraft`. The worktree was clean when review began. Findings refer to that baseline unless marked remediated.

## Recommendation

**Keep this as a candidate foundation, but do not treat it as a production-ready PDF engine or ship its full feature catalogue as BDF v1.** The existing layering, editing model, headless interface, asset inventory and tests are valuable. The right next investment is a Windows 11 stabilization milestone with independent document-fidelity evidence, consistent persistence, bounded execution and explicit local agent authority.

The owner's decisions are Windows 11 desktop first, local-first documents, and general office workflows first: view, annotate, fill forms, organize and save. Web follows and must not delay the desktop release. This is materially narrower than upstream's Acrobat Pro parity objective.

No evidence in this review justifies a wholesale rewrite. No evidence establishes that the current renderer will meet BDF's acceptance targets either. Measure it behind the existing abstraction before committing to maintaining or replacing it.

## Scope and limitations

This was a targeted review of architecture and high-risk boundaries: engine edits and permissions, desktop and automation saves, path confinement, UI control, MCP framing, web import/save, release/CI configuration, asset policy and upstream status. The tree contains 31 crates, 3 apps and approximately 225,400 lines across 422 first-party Rust files; those counts are not a claim that every line was reviewed. Vendored dependencies were considered through their patch inventory and integration boundaries, not exhaustively audited.

Rust/Cargo was not available on PATH or at the usual user Cargo location. Therefore compilation, clippy, Rust regression tests, fuzzing, cargo-deny, real UI operation and performance measurements were **not run**. Findings labelled source-confirmed establish a code path; they do not claim an exploit or data-loss experiment was performed. Severity describes release impact in the stated conditions, not a CVSS score.

The referenced `plan/` directory, including STATUS, execution plan, architecture and ADRs, is absent from this checkout. Public repository instructions and crate documentation were used instead. A maintainable fork needs its essential design decisions in version control.

## Latest upstream activity checked online

As observed on October 13, 2026, GitHub identifies **v0.7.1, released October 12**, as the latest release. Its changes include panel docking, Windows printer enumeration/spooling, actual-size printing off the UI thread, rendering fallback fixes and numerous form/text corrections. These are useful improvements, not independent proof of stability. [Release notes](https://github.com/storytold/pdfcraft/releases/tag/v0.7.1), [release index](https://github.com/storytold/pdfcraft/releases).

The project's public README calls it **alpha** and acknowledges remaining hostile-file crashes/hangs and lack of side-by-side fidelity measurement. Its numerical readiness estimates are maintainer estimates, not BDF acceptance metrics. Local progress documents sometimes disagree with the public README—for example, web printing is recorded as implemented locally while the public summary still lists it among gaps. [Upstream repository](https://github.com/storytold/pdfcraft).

A recent text-drag data-loss report, issue #1368, was closed through #1381 on October 12. Local history contains its fix (`5bf6d175`); this review does **not** report it as an unfixed BDF defect. It is a useful required regression scenario. [Issue and resolution](https://github.com/storytold/pdfcraft/issues/1368).

The most useful current news found was first-party releases, issues and project documentation; no independent security certification was established. Upstream HEAD was not fetched or compared, so this is not a statement that the fork includes every latest upstream change.

## Foundations worth preserving

| Foundation | Evidence | Value and qualification |
|---|---|---|
| Shared headless engine | `crates/engine/README.md`, `crates/engine/src/lib.rs` | UI and automation can reuse document semantics; engine edits check PDF permissions. PDF permission bits are not user/agent authorization. |
| Transactional edit history | Engine `Session::apply`, clone-based edit model | Undo and commit-on-success reduce partial mutations. Resource exhaustion and escaped process failures still need isolation. |
| File format fidelity intent | COS object graph, incremental saves, explicit full rewrites | Better foundation than reconstructing entire PDFs from visible content. Unknown-object preservation must still be independently tested. |
| Shared CLI/MCP tool table | `crates/automation/src/tools.rs`, `lib.rs`, `mcp.rs` | Existing schemas and command mappings reduce agent integration work. Session, policy and transport hardening remain. |
| Opt-in automation | Stdio MCP; loopback UI control with fresh token | No default network MCP service. Retain this default. |
| Explicit hardening practices | Guards, synthetic regression tests, fuzz task, vendored fixes | Strong intent and useful infrastructure; catch-unwind does not recover an OOM abort, all stack overflows, or a hung worker. |
| Provenance and dependency controls | Attribution manifest, `deny.toml`, locked/pinned dependencies | A solid inventory to build on; local vendor patches and mutable release infrastructure need ongoing ownership. |

## Findings and required action

### F01 — High: Windows control token is not explicitly owner-only

**Source-confirmed.** `apps/pdfcraft/src/main.rs:459` creates a temporary endpoint file and applies mode `0600` only under `cfg(unix)` (around line 474). There is no Windows DACL setup or validation in this function. The chosen directory's inherited permissions determine who can read the port/token. `crates/ui-egui/src/control.rs:791` correctly binds loopback, but any local principal able to read that token can authenticate to the UI control channel.

Trigger: launch with `--control` in a directory readable by another local user. This is conditional exposure, not proof that a normal private profile directory is exposed, and the channel is off by default.

Required: create the secret file under a verified user-only ACL using a maintained safe wrapper; refuse unsafe destinations before publishing credentials. Test with two actual Windows accounts and inherited permissive ACLs, as well as existing-file, symlink/junction and failure cases. Never print the token. Release-blocking if UI control is enabled in BDF.

### F02 — High: automation saves have weaker durability than desktop saves

**Source-confirmed.** `crates/automation/src/lib.rs:2310` writes the staging file, closes it, then renames it. It does not call `sync_all`. Desktop `crates/ui-egui/src/editing.rs:925` explicitly calls `sync_all` before replacement. Shared staging-name generation does not make these two persistence contracts equivalent.

Impact: a headless save may report success before durable storage, even though the desktop path attempts to flush. Atomic replacement protects against partial application writes; it is not the same as power-loss durability. Neither code inspection nor this review proves durability on every filesystem.

Required: one shared persistence service with flush-error handling, appropriate replacement semantics and platform/filesystem tests. Cover disk full, access denied, locked target, failed flush/rename, ACL preservation and power-interruption experiments. Failed saves must retain dirty state and recovery data.

### F03 — High: headless save can overwrite an externally changed document

**Source-confirmed for automation.** `doc_open` reads a file into the session (`crates/automation/src/lib.rs:838`); `doc_save` writes the current session bytes over its target (`:860`) without comparing the current on-disk version against the opened/saved version. There is no expected file revision in this path.

Trigger: open through MCP/CLI, change that file in another application, then call `doc_save` with the existing path. The newer disk contents can be replaced. Desktop has a file watcher; this finding does not claim desktop lacks change notifications. A watcher also does not substitute for a commit-time conflict check.

Required: optimistic concurrency on all write surfaces. Return a structured conflict and retain both versions; require an explicit overwrite choice bound to the version inspected. Test two sessions and an external replacement between preparation and commit.

### F04 — Medium: local control transport allows unbounded connection and request resources

**Source-confirmed.** `crates/ui-egui/src/control.rs:796` starts a thread for every accepted connection. There is no authentication read deadline or connection limit in that loop. Pre-auth lines are correctly capped at 4 KiB, but idle connections can retain threads; authenticated lines use `usize::MAX` (`:808`). A response timeout is not a read timeout or cancellation of work already queued.

Impact: local denial of service while the opt-in control channel is active, including idle connections that never authenticate. This is not an Internet-exposed listener.

Required: finite connection count, auth/read deadlines, bounded request size and queue, rate limits and explicit overload errors. Stress-test with idle clients and oversized lines while another document remains usable.

### F05 — Medium: MCP framing and execution lack resource/cancellation boundaries

**Source-confirmed.** `crates/automation/src/mcp.rs:86` uses `input.lines()`, allocating an entire line without a size cap before JSON parsing. Calls execute synchronously; cancellation notifications are ignored (`:110` and automation README). A large line can exhaust memory before tool validation; a long-running operation blocks subsequent requests in that server.

The current transport is stdio launched by the user, so this is a local robustness boundary, not an unauthenticated remote service vulnerability.

Required: bounded framing, limits before allocation, operation/job deadlines, cancellation and bounded outputs. Keep ordinary calls synchronous if fast; expose durable job state for long work. Test through actual stdio, compact `tool_call`, batches and resource reads.

### F06 — High-priority investigation: root confinement is path-based and subject to filesystem changes

**Potential vulnerability; not reproduced.** `Automation::resolve` (`crates/automation/src/lib.rs:1879`) canonicalizes and checks ancestors, then later I/O opens a path. `doc_save` without an explicit path reuses the stored document path without resolving it again. A writable directory can change between authorization and use, including a parent replaced by a junction/symlink.

The existing checks and tests address ordinary traversal and static symlinks; they do not establish race-resistant confinement. An attack requires the ability to alter relevant filesystem entries. Do not describe `--root` as a strong sandbox against a malicious actor controlling that tree until proven.

Required: Windows junction/race reproducer, revalidation of stored paths, and handle/capability-based I/O where available through safe wrappers. Design against the complete read/write lifecycle, not only initial string validation. Revalidation alone narrows but does not eliminate a race.

### F07 — High release risk: mutable upstream workflow receives release privileges

**Source-confirmed.** `.github/workflows/release.yml:23` delegates to `storytold/craft-common-actions/.../release.yml@main` with `contents: write` and references to signing/cloud credentials. Changing that remote branch changes the release code without a BDF review. This is a supply-chain exposure, not evidence that upstream is malicious or that any credentials are populated in this fork.

Required: a BDF-owned, reviewed, immutable release workflow/dependency reference; least privilege; protected release environment; BDF signing identity; artifact provenance and SBOM; rollback exercise. CI also uses `cargo-deny-action@v2` rather than an immutable SHA. Keep dependency updates explicit and reviewed.

### F08 — Architectural gap: tool parity is not identity or authorization parity

**Source-confirmed in the reviewed entry points.** `Automation` owns a `Session` and optional root (`crates/automation/src/lib.rs:107`); `Session` owns document state (`crates/engine/src/lib.rs:2675`). Calls do not carry an authenticated actor, delegation grant, action/resource policy or audit context. PDF permission enforcement exists and must remain; comment authors and certificate trust stores serve different purposes.

This is reasonable for a single-user local utility but insufficient for the proposed maintenance/support module. A loopback token proves possession, not the human on whose behalf an agent acts. A plain HTTP wrapper over the current API would not solve this.

Required: a shared application-service boundary used by UI, CLI and MCP. Give agents separate, revocable identities acting on behalf of a human, with intersected scopes. Start with local OS identity and local grants; cloud accounts are unnecessary for desktop v1. Denial tests must cover nested dispatch and resource reads, not just tool listing.

### F09 — High for a future embedded web release: save success precedes persistence acknowledgment

**Source-confirmed; deferred from desktop critical path.** `apps/pdfcraft-web/src/main.rs:210` returns success when `postMessage` succeeds. `crates/ui-egui/src/editing.rs:623` then marks the document saved and can continue a close operation. No host persistence acknowledgment is required.

Trigger: a same-origin parent receives the save message but storage fails or its handler does nothing. A successful message transfer does not establish a saved file. Ordinary browser downloads also cannot prove the user retained the file; their status should describe export initiation honestly.

Required before embedded web release: save request IDs, document revisions, host success/failure acknowledgment, timeout, retry and dirty-state preservation until the correct revision is confirmed. Existing parent-origin/source validation is a positive control; it does not solve persistence.

### F10 — Medium: acquisition limits and aggregate memory budgets remain incomplete

**Source-confirmed entry points; total exhaustion not reproduced.** Automation `doc_open` uses `fs::read` before a file-size admission limit. Web `fetch_bytes` (`apps/pdfcraft-web/src/main.rs:110`) obtains the entire `array_buffer` and copies it; the parent bridge copies incoming buffers and queues them without a count/byte budget. These paths can allocate before parser or render limits help.

The code has useful stream, render and text-cache limits, but those are not a total session budget. Engine documentation and `docs/gaps.md` acknowledge memory duplication and missing enforced budgets. Catch-unwind is not protection from allocation aborts.

Required: finite input and aggregate budgets, bounded queues/undo/caches, admission control before copying, and separately supervised parser/render work for hostile inputs. Desktop measurements come first; web backpressure is a later gate.

### F11 — Medium: CI coverage does not fully match the stated gates

**Source-confirmed.** `.github/workflows/ci.yml` runs format, clippy, tests, layers, assets, wasm library checks and dependency checks, but does not invoke parity. `xtask::gates::wasm` filters to library/standalone layers (`xtask/src/gates.rs:74`); it does not build the `pdfcraft-web` app entry point. The full `xtask ci` includes parity, but the workflow does not invoke that aggregate command.

Required: explicitly required parity and BDF acceptance jobs; scoped local checks and full hosted Windows/Linux checks. Keep library wasm compatibility now; when web becomes a release target, add app build and browser runtime acceptance. Gate steps should still report independent failures when an earlier step fails. Existing workflow files are not evidence that the fork's branch protection or last run is green.

### F12 — Fork integration and license hygiene

**Restricted visual assets removed in this working tree under AGENTS.md §1.** Eight ArtCraft logo/mark files and the screenshot explicitly attributed as containing the mark were present. Their license excludes an independently distributed fork. Removed their manifest entries, README embeds and UI image references; preserved plain-text upstream credit, copyright notices and license text. Adjusted the existing About-dialog assertion.

`ATTRIBUTION.md` was regenerated programmatically, not hand-edited: because Cargo is unavailable, an equivalent renderer was first verified to reproduce the entire unchanged generated file exactly, then applied to the revised manifest. The prescribed Cargo generator/check still must be run. The remaining 261 repository asset entries were checked for file existence, SHA-256 and license-file existence: all passed this limited check. Dependency-bundled and downloaded material was not fetched/re-audited. No build/output directory was present in the checkout; past published artifacts and Git history were not changed.

BDF still needs a deliberate naming/configuration migration. `crates/ui-egui/src/updates.rs:15` points update checks at upstream releases; release app-name/title and multiple community links remain upstream-oriented. Do not ship a fork whose update/help routes imply upstream maintains it. Keep author credits and required notices while changing product/support identity.

## Other material engineering risks

- The renderer is a heavily patched dependency family, with additional vendor patches for lopdf, winit and egui-wgpu. Every patch needs an owner, upstream status, regression test and retirement condition. Workspace `unsafe_code = forbid` does not mean dependencies contain no unsafe code.
- Existing-text editing has difficult source-font, subset, layout and language cases. Keep it out of general-office v1. Adding text annotations is a different, safer workflow.
- PDF JavaScript is on by default at the session level. Its sandbox has useful loop/recursion/source limits and no direct file/network APIs; this review did not prove an escape. BDF should choose an explicit per-document script policy and clearly identify forms whose calculations are disabled.
- Redaction, optimization, digital signatures and standards claims have high correctness costs. Preserve implementation work, but do not expose these as verified BDF capabilities merely because a tool exists.
- Save/reopen in the same engine can repeat the same bug. Independent reader/render checks are necessary, especially for forms, annotations, encryption, rotation and page assembly.
- UI source/tests do not establish accessibility, discoverability or Windows usability. Actual keyboard, screen-reader, mixed-DPI, graphics-driver, file-association and printer testing is still needed.

## Decision and next gate

Proceed with BDF stabilization, not feature expansion. First restore a reproducible development environment and obtain a known-green baseline; then address F01–F08, F10–F12 as applicable to desktop. Keep F09 and web-specific parts of F10/F11 in the later web milestone. Record reproducible findings before fixing them, and do not label suspected races as closed on code inspection alone.

The companion [PRD](bdf-prd-draft.md) defines the narrower product and acceptance gates. Claude Code can implement individual work items; Codex reviews the resulting diffs and evidence against those gates. No commit, PR, publication, upstream synchronization or production security fix was performed during this review, apart from the mandated branding cleanup.

## Revision history

| Date | Revision | Change |
|---|---|---|
| 2026-10-13 | 0.1 | Initial targeted review; incorporated local-first, general-office and desktop-first decisions. |
