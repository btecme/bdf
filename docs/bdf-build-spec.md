# BDF desktop build specification — 0.1

Status: ready to start baseline verification and bounded implementation. Prepared 2026-10-13. Architecture and budgets below are implementation directions; they do not certify the current build.

Inputs: [PRD](bdf-prd-draft.md), [foundation review](bdf-foundation-review.md), [work items](bdf-work-items.toml), [Claude kickoff](bdf-claude-kickoff.md). Read `AGENTS.md` and `CLAUDE.md` first; these documents do not waive their rules. Essential BDF plans live in committed `docs/` because this checkout has no local `plan/` directory.

## 1. Build objective

Deliver Windows 11 x64 desktop v1 with dependable viewing, embedded-text search/selection, annotations, existing AcroForm filling, page assembly, printing, undo/redo, saving and recovery. Documents remain local. Every released document capability has an engine API, CLI/MCP tool and end-to-end evidence. Human and agent actions cross the same application authorization boundary.

Web is a later milestone. Preserve engine wasm compatibility but do not build a new web shell, backend or cloud identity system now. ARM64 follows x64 qualification. Existing-text rewriting, OCR, redaction, optimization, measurements, Office conversion, digital signing, standards certification and AI features remain outside the release profile. Preserve their code; do not expose unqualified capabilities through alternate entry points.

Retain Rust, egui/eframe, COS and the current renderer abstraction. No wholesale rewrite, mass crate rename or new JavaScript/TypeScript product code. Keep internal `pdfcraft-*` package names during stabilization; BDF-facing branding, executable distribution names, settings and support routes are addressed deliberately in the packaging milestone.

## 2. The first build and first useful slice

The first task is **BUILD-001**, not a feature sprint. Establish a reproducible baseline on Windows; verify the existing uncommitted review/branding changes; capture failures and toolchain/platform evidence. Do not reset, stash or indiscriminately commit the existing worktree. Do not fetch/merge upstream main merely to get a build passing.

The first integrated slice after hardening is:

1. Open one synthetic PDF through the desktop and a detached headless session.
2. Render page 1 and inspect its output.
3. Rotate page 1, undo, redo, and inspect again.
4. Save a new copy, reopen it, verify page geometry and visible content.
5. Inject a failed save, a stale document revision, an externally replaced file and a revoked grant. Each refuses correctly and preserves working edits.
6. Repeat through direct service calls, ordinary CLI, full MCP, compact/meta MCP and UI control commands.

This slice establishes the shared service, persistence, authorization and evidence patterns before migrating the wider tool catalogue.

## 3. Implementation boundaries and dependency graph

| Location | Responsibility | Must not own |
|---|---|---|
| `crates/engine` (existing L6) | PDF sessions, immutable document queries, edits, undo, permission checks, serialization | OS identity, credential files, GUI, network listeners |
| `crates/platform` (existing L7) | Shared native file persistence, private ACLs, path/file identity, worker supervision primitives | PDF parsing, tool registry or product authorization policy |
| `crates/app-service` (new package `pdfcraft-app-service`, L7) | Public operations, capabilities, actors/grants, session/revision state, jobs, audit, orchestration | egui, MCP transport or CLI argument parsing |
| `crates/ui-egui` (existing L7) | UI state/input, accessible presentation, dialogs, service adapter | A separate authorization or save implementation |
| `crates/automation` (existing L7) | JSON schemas, argument adapters, MCP resources and dispatch into the service | Its own authoritative PDF session/persistence/policy |
| `apps/pdfcraft`, `apps/pdfcraft-cli` (existing L8) | Composition, startup, local authentication bootstrap and packaging | Divergent document semantics |
| `apps/pdfcraft-worker` (new L8, after feasibility gate) | Private bounded document-processing process | Public tool service, arbitrary filesystem access, external actions |
| `xtask` | Qualification fixtures, evidence validation and CI gates | Product runtime support agent or background updater |

Dependency direction: apps → UI/automation → app-service → engine and platform. Engine remains below platform and does not depend on it. Add `app-service` to `xtask/src/layers.rs` as L7 and only the explicit same-layer edges `ui-egui → app-service`, `automation → app-service`, `app-service → platform`. Preserve existing legitimate platform edges during migration, then remove dead ones. Do not relax general layering to permit cycles. Update workspace dependencies and crate READMEs with each boundary change.

Suggested service modules: `operations`, `capabilities`, `actors`, `grants`, `sessions`, `revisions`, `jobs`, `audit`, `errors`, `limits`. Start them inside one crate; defer a standalone support framework. Suggested platform modules: `persistence`, `private_files`, `file_identity`, `confinement`, `workers`; retain existing staging helpers.

The automation schemas remain adapters. A service operation descriptor owns stable operation ID, required capabilities, resource access, mutation/side-effect class and release status. Both tool definitions and UI command availability derive from this descriptor. The service must never depend back on `automation::ToolDef`.

UI migration is incremental: introduce a read-only service document facade, move one mutation/save path at a time, then make the underlying mutable session private. Existing direct-engine tests remain useful internal tests; no production public adapter may bypass the service. Catalogue visibility and policy refusal must ship together.

## 4. Public operation and state contracts

Use typed internal IDs and versioned JSON adapters. Reuse the existing integer `DocId` externally within a session; never treat it as globally unique. The resource key is `(session_id, document_id)`. Authentication creates `ActorContext`; a caller naming an actor in JSON does not authenticate it.

| Contract | Required fields / invariant |
|---|---|
| `ActorContext` | Principal kind human/agent, authenticated local OS principal, client ID, optional delegating human and grant ID; supplied by trusted bootstrap |
| `RequestContext` | Unique request ID, session ID, actor context, cancellation/deadline; no secrets in logs |
| `DocumentVersion` | Monotonic content revision, engine-generation mapping, saved snapshot identity, optional storage fingerprint |
| `FileFingerprint` | Stable native file identity when available, byte length and SHA-256 of content; metadata timestamps are a hint, never sufficient alone |
| `OperationRequest` | Schema version, operation ID, typed arguments and expected content revision for mutations |
| `OperationResult` | Outcome, document version, warnings/repairs, committed effects, job/result references when applicable |
| `OperationError` | Stable code, useful message, retryable flag and bounded details; avoid leaking inaccessible paths |

Allocate revisions with checked arithmetic. Undo and redo that change content each advance the public revision; save rebasing alone need not change content revision. Engine render-generation changes are not automatically public content changes. Derive dirty state from working versus saved snapshot identity, not from monotonic revision equality; undo back to a saved snapshot must be recognized correctly.

Errors: `invalid_argument`, `not_authorized`, `not_found`, `unsupported_feature`, `password_required`, `wrong_password`, `conflict`, `resource_limit`, `canceled`, `io_failure`, `worker_failure`, `internal_failure`. Distinguish a failed operation before commit from a committed file whose subsequent UI refresh failed. Never encourage retry of an ambiguous print/save as though nothing happened.

Serialize document mutations within a session. Use optimistic revision checks for attached agents. A detached process additionally checks storage versions before replacing disk files. No long-held global UI mutex during I/O or rendering. Results arriving for a closed document or older revision are discarded safely.

Keep current tool names where compatible. For the new BDF API contract, document mutating requests require `expected_revision`; do not silently use the latest revision on behalf of an agent. Introduce schema-version negotiation and explicit migration errors for older clients. Preserve current 1-based external pages and top-left displayed-page coordinates; validate then convert exactly once.

`ToolDef.read_only` and `destructive` are descriptive hints, not permissions. Compact `tool_call`, `command_run`, batches, previews and resource reads must traverse the same service checks as direct calls. Batch responses disclose transactional versus best-effort behavior and exactly which steps committed.

## 5. Local identity, grants and capability policy

The interactive human is the local OS user. CLI direct commands run as that local user and authorize only explicitly requested actions. MCP clients are distinct agents delegated by that user. Establish the bootstrap identity from the launched process/owner-protected credentials; do not trust command-line actor-name claims. Same-user malicious software and administrator compromise are outside the guarantee of local capability policy; Windows ACLs must protect against other ordinary accounts.

Local state lives under a BDF-specific per-user application directory. Private secrets, grants, recovery and audit files have verified restrictive ACLs before sensitive content is written. Portable mode requires a verified private state directory; fail safely on a filesystem that cannot provide the required protections. Do not import upstream credentials or signing IDs silently.

Grants contain client ID, delegating OS principal, resource scope, operations, expiry and revocation generation. Start read-only. A document grant allows access to its in-memory content but does not automatically authorize writing any filesystem path. A selected output root is a separate resource permission.

Capabilities initially: `document.read`, `document.edit.annotations`, `document.edit.forms`, `document.edit.pages`, `document.save`, `document.export`, `document.print`, `diagnostics.read`, `diagnostics.export`. `document.save` authorizes replacing a tracked target subject to its expected storage version; an unrelated existing destination requires explicit overwrite authority bound to that file version. New agent grants receive only `document.read`. Print and external file writes are distinct from in-memory editing.

Effective access is the intersection of grant, delegating human's rights, document permission bits and BDF release policy. Check on entry, before returning protected data, and immediately before each external side effect. Revocation cancels queued work and prevents uncommitted effects. Undo/redo may traverse only changes permitted for the calling actor; they are not a back door to broader mutation capabilities.

MCP stays stdio and explicitly launched; no registration or background server by default. A root is confinement, not an authorization grant. For the future BDF MCP invocation, an absent valid grant yields only public version/capability help—no filesystem discovery or PDF content. Do not grandfather unrestricted MCP filesystem access as a compatibility workaround.

UI control stays explicitly enabled, loopback-only with a fresh token. Harden owner-only endpoint creation first. Control credentials represent a delegated session actor; `ui.command`, `ui.open` and settings that alter document trust obey service policy. Pointer automation is a test/control aid, not the primary document API.

Session attachment is explicit and owner-authenticated. Prefer a private local IPC bridge; no new network exposure. Attach to an existing service session and preserve revision/undo history. Detached sessions use independent IDs and cannot silently replace a file opened by another session. Prove attached and detached conflict behavior before enabling attachment in release builds.

Audit record: sequence, time, actor/client/grant IDs, operation/request IDs, non-sensitive resource ID, before/after revision, outcome, warnings and committed effects. Bounded JSONL storage with rotation and accidental-corruption detection; no PDF text, full paths, passwords or tokens by default. Export requires a separate action and a user-visible preview.

## 6. Persistence and recovery protocol

Replace duplicated desktop/headless save writers with a shared platform implementation orchestrated by app-service. Persistence operates on bytes and filesystem capabilities; it does not know PDF internals. Do not move OS operations into the engine.

Required save sequence:

1. Authorize actor, target and expected document version; snapshot immutable working state and its revision.
2. Serialize using the engine's appropriate incremental/full mode; keep the working document untouched if serialization fails. Refuse unsupported signature/security combinations explicitly.
3. Validate generated bytes on a temporary engine state before replacing a target. Preserve repairs/warnings; perform expensive validation in supervised work, not the UI frame.
4. Resolve/authorize the complete target lifecycle. Obtain the appropriate file/directory identity and verify expected storage fingerprint. Refuse changed, missing or newly created targets unless the requested mode explicitly permits them.
5. Create an unpredictable, exclusive staging file in the target filesystem with private permissions. Write all bytes, flush and handle every error. Never temporarily expose sensitive bytes via a permissive staging file.
6. Recheck grant/revocation and target version at the commit boundary; serialize BDF writers to that target. Use an evaluated Windows replacement primitive with required ACL/attribute behavior. Test its failure outcomes; do not claim path revalidation alone prevents adversarial junction races.
7. Publish the new saved snapshot/version only after successful filesystem commit. If newer edits were made while saving revision N, mark N saved and retain newer working edits/dirty state; never rebase N over N+1.
8. Update recovery and view state. If commit succeeded but reopening/refresh fails, report that distinct state, retain recoverable working bytes and the commit record, and do not delete recovery prematurely.

Provide explicit `create_new`, `replace_tracked`, and `replace_confirmed` modes. `replace_confirmed` carries the inspected target fingerprint; an unqualified boolean `overwrite=true` is insufficient. Do not quietly downgrade a flush failure, copy over an original when atomic replacement fails, or delete originals before a rename.

Windows ACL inheritance and replacement semantics require explicit evaluation rather than assuming POSIX behavior. Microsoft's [file security documentation](https://learn.microsoft.com/en-us/windows/win32/fileio/file-security-and-access-rights) describes inheritance, and [ReplaceFileW documentation](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-replacefilew) describes ACL/attribute preservation and failure states. A maintained safe Rust wrapper is required; this spec does not approve raw unsafe calls or promise that a particular dependency exposes all needed APIs.

Concurrent-modification guarantees must state their boundary: detect ordinary external replacements, lock/serialize BDF writers, and demonstrate race-resistant root confinement against the stated attacker model. A check-then-rename sequence cannot be advertised as an atomic compare-and-swap against every uncooperative external writer. BUILD-006 must record the supported solution and adversarial Windows evidence; do not close the finding with another canonicalization alone.

Recovery snapshots include a schema version, document/session identity, content revision, source fingerprint, snapshot hash and encryption indicator. Atomically checkpoint within five seconds of idle after an accepted edit; flush/ACL failures are visible. Do not store decrypted material from password-protected PDFs without an explicit policy preserving equivalent confidentiality. Validate hashes before recovery, tolerate corrupt sidecars, and keep a previous valid snapshot until its replacement commits. Recovery expiry applies only after saved/discarded resolution; do not expire unresolved edits silently.

Shared fault tests: allocation/serialization failure, short write, disk full, failed flush, denied/locked target, failed replacement, changed fingerprint, symlink/junction swap, revocation during save, edits during save, and successful commit followed by refresh failure. Verify original/snapshot bytes and dirty state, not merely error strings. Run these through UI and MCP as well as the platform/service tests.

## 7. Bounds, jobs and document workers

Initial implementation limits below are conservative starting values to record and ratify in BUILD-002. Existing stricter safety caps remain; changing any cap needs measured justification.

| Boundary | Initial limit / behavior |
|---|---|
| Input PDF | 250 MiB; 2,000 pages after bounded inspection; complexity/decode limits can reject smaller files |
| Retained input bytes | 500 MiB across open documents; total process-tree target ≤1.5 GiB, including workers/undo/caches |
| Undo | Existing 100-step ceiling plus an accounted byte budget; refuse or evict complete old snapshots, never a partial step |
| MCP/control JSON line | 1 MiB post-auth, 4 KiB control pre-auth; reject before allocation grows beyond the boundary |
| Scalar text argument | 256 KiB UTF-8 unless a tool has a smaller documented cap |
| Control connections | 8 total; 5-second auth deadline, 30-second idle read deadline; finite queued requests |
| Service work | 32 queued requests per session, 2 concurrent heavy jobs per app; document mutations remain serialized |
| Result message | 16 MiB including encoding; page images use bounded dimensions/dpi and a declared scoped result resource when larger |
| Open/render/search job | 30-second default deadline; explicit bounded extension for approved bulk jobs, never infinite |
| Cancellation | Feedback within 100 ms; cooperative stop or supervised termination within 2 seconds |

These are ceilings, not targets for routine allocations. Account allocations before copying, including base64/JSON expansion. Acquire file metadata and bounded reads before whole-file buffers. Input can change during reading; enforce the byte limit while reading too. A render result resource must not become an unscoped filesystem URL.

Jobs carry job/request/actor/session IDs, input revision, state, bounded progress, cancellation token, deadline and committed effects. States: queued → running → succeeded/failed/canceled, with cancel-requested distinguished from canceled. Mutation candidates commit only after authorization/revision recheck. Old results never overwrite current state. Print cancellation reports spool submission honestly; physical pages already printed cannot be undone or automatically retried.

Worker approach: BUILD-002 proves a private spawned-process prototype for open/render before integration. IPC uses finite frames and versioned messages; stdout is protocol only, bounded diagnostics go to stderr. Large document bytes use bounded chunks with a declared total/hash, not a giant JSON line. Parent owns session/recovery and validates responses. Kill/restart a worker safely on a hang; one bounded retry may be permitted for read-only rendering, never blindly repeat an external effect.

Windows Job Objects can govern process lifetime/resources; see [Microsoft's Job Objects documentation](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects). Evaluate safe wrappers and OS restrictions. Process separation alone is not a security sandbox; record whether the implementation actually removes ambient filesystem/network authority. Document scripts/actions cannot ask a worker to perform external effects. Every remaining in-process untrusted parser/edit operation needs explicit failure-containment evidence before release. If no compliant safe-wrapper/isolation design meets the gate, report the blocked architecture decision rather than introducing unsafe code.

## 8. Desktop core qualification and UI

Default workspace: document tabs, compact Open/Save/Print toolbar, visible active annotation tool, optional pages/bookmarks pane, optional comments/properties pane. Preserve a clean reading mode. Default minimum qualification size is 1280×720; all core controls remain reachable at 100/150/200% DPI and keyboard-only. Do not surface advanced unavailable tool groups in the normal catalogue.

| Core area | Existing adapters to inspect/reuse | Required behavior |
|---|---|---|
| View/search | `doc_open`, `doc_info`, `page_render`, `text_extract`, `text_find` | No silent partial extraction; readable errors; correct geometry at rotation/zoom; scans explain that embedded text is absent |
| Annotate | `comment_list`, `comment_add`, `comment_edit`, `comment_delete` | Standard highlight/underline/strikeout/note/textbox/rectangle/arrow/ink; one sensible undo step per action; independent-reader round trip |
| Forms | `form_fields`, `form_fill`, `form_reset` | Existing AcroForms; correct saved appearances and tab behavior; unsupported calculations/XFA visibly identified |
| Organize | `page_rotate`, `page_move`, `page_delete`, `page_insert_blank`, `page_insert_file`, `page_extract`, `doc_combine`, `doc_split` | Preview affected physical pages; preserve/disclose label/link/bookmark/form effects; deterministic duplicate-field naming |
| Persistence/history | `doc_save`, `doc_close`, `edit_undo`, `edit_redo` | Shared contracts above; unsaved close refused; no conflict bypass |
| Print | `printers`, `printer_options`, `doc_print` | Same authorized job through desktop/CLI/MCP; actual-size/fit and printed bounds verified |

Adapter names are from the current table, not a claim that they already meet the spec. Parameter-level allowlists matter: v1 combine accepts PDFs, not every inherited conversion source; page-render text-hiding previews are outside core; form scripting/authoring and permanent flattening are not released merely because the parent tool exists. Define allowed modes and shapes explicitly in the release profile and service validation.

Default PDF JavaScript off for untrusted documents; offer a per-document trust decision when calculations are needed, and do not claim form results were calculated if scripts are disabled. Initially refuse dynamic XFA in the v1 profile with an explanatory message rather than executing it invisibly. Basic signature appearance is optional after core; never use an appearance to assert a certificate is valid.

Keep Ctrl+O/S/Shift+S/P/F/Z/Y, Escape cancellation, conventional selection and context menus. Verify these against fields/text editors. Include edit-draft text in dirty/close/recovery handling. No accepted draft disappears when switching tools/tabs. Test Narrator and NVDA core tasks, focused-state names, mixed-DPI movement, GPU fallback, system file dialogs and a physical printer.

## 9. Tests, evidence and CI

Keep production changes small and regression tests synthetic. A defect test must be run against the baseline and fail before its fix; record both results. Do not replace independent document evidence with mocks of the same writer/parser.

Suggested evidence paths: `target/bdf-evidence/<work-item>/<commit>/` for local unpublished scripts, renders and logs; CI artifacts for reviewed evidence; committed `docs/bdf-evidence/` for concise manifests/summaries only. Never commit private documents, forbidden screenshots or new fonts. Any published asset must receive attribution and policy checks. Set `PDFCRAFT_SYSTEM_FONTS=0` for publishable UI captures.

Each work-item report records baseline and reviewed commit, requirement/finding IDs, OS/toolchain/dependency versions, commands and exit codes, test-oracle description, artifact paths/hashes, unavailable/skipped checks, limitations and remaining risks. A status changes to verified only when its cited evidence is present and rerun on the reviewed revision. Machine-readable task status is planning information until evidence validation exists.

Evidence suites:

- **Persistence:** all fault/concurrency cases in §6, plus ACL preservation and source-encryption privacy.
- **Policy:** same allowed/denied request across UI/service/CLI/full MCP/compact meta/batch/resources; denied operations leave document bytes, undo, outputs and audit consistent.
- **Hostile input:** malformed JSON, over-limit framing, negative/NaN/overflow values, cyclic/deep PDF graphs, decoded-data bombs, corrupt state, disconnected client and full disk.
- **Core round trips:** review packet, AcroForm and assembly workflows; retain untouched page objects and semantic content; inspect `page_render`; independent reader/raster oracle as a separate process.
- **Runtime:** worker crash/hang/cancel; ten open-edit-save-close cycles; Windows 100/150/200% DPI, restore/mixed monitors, keyboard/screen reader, printer spool errors.

Use the PRD's initial latency/memory/500-case release targets, measured on recorded hardware. Start with 100 fixtures and at least 30 timing runs per class. Set per-operation pixel/semantic tolerances before claiming a pass; triage every material visual difference. License and pin external oracle inputs/dependencies. Current shell and workers are measured together.

Required hosted gates: fmt, clippy, workspace tests Windows/Linux, layers, assets, dependency/advisory checks, upstream parity and BDF core acceptance. Library wasm checks stay; web app/browser acceptance is deferred. Do not infer green CI from a workflow file, and do not allow a failed/skipped required job to count as release approval. Pin reusable workflows/actions by reviewed immutable revisions and least privilege; release signing remains BDF-owned.

Typical local commands after toolchain setup, scoped to actual changes:

```powershell
cargo fmt --all -- --check
cargo clippy -p pdfcraft-platform -j 2 --all-targets -- -D warnings
cargo test -p pdfcraft-platform -j 2
cargo xtask layers
cargo xtask assets --write
cargo xtask assets
cargo xtask parity
```

Run per-crate commands under a process supervisor with a finite timeout; full workspace sweeps normally run in CI. New app-service checks start only after that crate exists. Do not hand-edit generated attribution or contributor statistics. Record both builds with/without optional craft-fonts when fonts affect evidence.

## 10. Starting sequence and delivery rules

The [work-item file](bdf-work-items.toml) is the executable planning index: dependencies, scope, findings, requirements, evidence and stop conditions. Initially all tasks are pending. Work-item IDs are commit/PR task identifiers, not dates or progress percentages.

Start BUILD-001 → BUILD-002 → BUILD-003. Then complete persistence/control/confinement corrections and the shared service before qualifying desktop core. Each task gets a branch from the verified baseline and a focused PR; do not mix asset cleanup, identity architecture and rendering changes into one patch. Existing unverified changes must be verified and selectively included, with owner changes preserved. Follow the repository contributor-credit check if preparing a PR.

No feature is complete without engine/API/tool parity and evidence. Every milestone ends with Codex reviewing the actual rebased diff and meaningful tests; the human owner decides release and architecture tradeoffs. Do not auto-merge, push main, publish, install production updates or register MCP from a successful local test.

The first final implementation report should answer: what baseline built, which checks passed/failed, what state was modified, where the evidence lives, and whether BUILD-002 can begin. Do not proceed through all work items from the kickoff prompt or label source-level review findings fixed without a reproducer.

## 11. Later support module and web boundary

Expose product-neutral capability discovery, scoped diagnostics, jobs and audit through the service interfaces. Keep PDF operation types in BDF. Extract these contracts only after a second product proves reuse; do not introduce an all-powerful support shell now. Support starts read-only; content sharing, repairs, uploads, updates and restarts are separately authorized actions with preview/rollback where applicable.

When web resumes: implement persistence acknowledgment bound to request/document revision, local recovery, bounded browser acquisition/queues, browser worker/job handling and actual app runtime CI. The existing parent-origin checks are retained. Desktop acceptance does not imply web acceptance.

## Revision history

| Date | Revision | Change |
|---|---|---|
| 2026-10-13 | 0.1 | Desktop implementation contracts, module boundaries, finite limits, test/evidence rules and initial build sequence. |
