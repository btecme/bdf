# BDF product requirements — draft 0.1

Status: draft for owner review and implementation planning. Updated 2026-10-13.

Companion evidence: [foundation review](bdf-foundation-review.md). Implementation handoff: [build specification](bdf-build-spec.md), [work items](bdf-work-items.toml), [Claude Code kickoff](bdf-claude-kickoff.md). Requirements below are proposed unless explicitly identified as owner decisions. They are not claims about what the current build already delivers.

## 1. Product intent and confirmed direction

BDF is a dependable PDF workbench for ordinary professional work: open a file, understand it, mark it up, fill it out, rearrange pages and save a trustworthy result. It should feel familiar on the first use, stay responsive, and preserve the user's work when a document, device or operation fails.

Confirmed by the owner:

- **Windows 11 desktop is first.** Web is a later release and must not delay desktop v1.
- **Local-first.** Documents remain on the user's device by default. Shared cloud workspaces and accounts come later.
- **General office leads:** view, annotate, forms, organize, save. Serve construction, design, administration, executive, accounting, marketing and IT through that shared core.
- Agent access needs an API and MCP interface governed at the same authorization boundary as human actions.
- Maintenance/support integration should eventually become a reusable module for BDF and other products.
- Claude Code implements; Codex checks. Human ownership of product scope and releases remains explicit.

Success is reliable completion of these jobs, not a percentage of Acrobat's feature list. Retain familiar concepts and shortcuts; use original visual design and permitted assets.

## 2. Users and their first useful outcome

| Audience | Immediate job | Desktop v1 coverage | Later specialization |
|---|---|---|---|
| Administration | Complete a form, assemble attachments, correct page order | AcroForm filling, merge/split/reorder, saved output | Templates, OCR, batch filing |
| Executives | Read a packet, find a clause, comment, approve with visible mark | Fast reading/search/bookmarks, highlights/notes, simple signature appearance | Comparison, approval workflows, certificate signing |
| Accounting | Review invoices, annotate discrepancies, assemble records | Search when text exists, markups, page extraction/assembly | OCR, verified redaction, controlled data extraction |
| Marketing | Review collateral and return clear corrections | Accurate display, highlights, shapes, comments | Version comparison, compression, print-production checks |
| Design | Inspect drawings and annotate revisions | Zoom/pan, page labels, callouts/shapes, save | Color-managed proofing, overlays, measurements |
| Construction | Navigate a plan set and communicate changes | Large-page viewing, bookmarks/labels, markups, actual-size printing | Calibrated measurement, scale/units, revision overlays and takeoff |
| IT | Deploy, diagnose and automate document work safely | Signed desktop package, CLI/MCP core tools, local policy and diagnostics | Managed deployment policies, reusable support module and fleet reporting |

These are workflow hypotheses, not completed user research. Validate with 8–12 pilot users across at least four disciplines, including accounting/admin and users who routinely work with large plans.

## 3. Scope and feature policy

Desktop v1 targets Windows 11 x64 first. ARM64 is a follow-on unless it passes the same test matrix without delaying x64. Existing macOS/Linux code should remain buildable in CI, but those platforms are not BDF v1 acceptance targets. Preserve the headless engine and wasm-compatible boundaries so web work can resume later.

Inherited features outside v1 should be hidden from the default catalogue or clearly isolated behind an explicit experimental setting. Advanced destructive features stay unavailable through ordinary UI, CLI and MCP until qualified. Hiding a toolbar button alone is insufficient. The core should have a compact toolbar and contextual properties instead of a wall of advanced or unavailable tools.

| Requirement | Priority | User-visible behavior | Acceptance evidence |
|---|---|---|---|
| CORE-01 Open and view | v1 must | Open through Explorer, drag/drop, file picker and recent files; password prompt; tabs; page number/label; bookmarks/thumbnails; fit page/width; continuous and single-page view; zoom/pan | Same representative corpus opens through UI and headless API; large/invalid/locked files give actionable errors; no silent blank-page success |
| CORE-02 Find and select | v1 must | Search embedded text, next/previous result, text selection/copy, visible no-text state for scans | Matches and geometry remain correct under page rotation/zoom; Unicode selection/copy fixtures; partial extraction failures are disclosed |
| CORE-03 Annotate | v1 must | Highlight, underline, strikeout, note, text box, rectangle, arrow and freehand; edit/delete; author and comment list | Save/reopen and independent-reader round trips; undo/redo; no accidental switch from selection to drawing |
| CORE-04 Fill forms | v1 must | Existing AcroForm text, checkbox, radio and choice fields; tab order; multiline; save | Field values and appearances agree after reopen in an independent reader; unsupported XFA/script requirements are identified, not silently ignored |
| CORE-05 Organize | v1 must | Rotate, reorder, delete, insert blank pages, merge PDFs, split and extract selected ranges | Preview exact affected pages; one operation/undo step; preserve or explicitly report affected bookmarks, links, forms and labels; signed-file effects explained |
| CORE-06 Save safely | v1 must | Save, Save As, conflict detection, unsaved indicator, recovery and close confirmation | Same durable save contract for UI/API/MCP; fault-injected failures retain original file and dirty work; no success before commit |
| CORE-07 Print | v1 must | Choose printer, page range, copies, actual size or fit, orientation and preview | Physical Windows printer plus PDF-print destination; dimensions verified with a synthetic ruler; large sheets not silently scaled; cancellation/error surfaced |
| CORE-08 Undo and redo | v1 must | Predictable labels; shortcuts; edits grouped into meaningful actions | All v1 mutations are undoable in session except clearly described external effects such as printing/export; bounded history |
| CORE-09 Simple signature appearance | v1 should | Place a typed or drawn signature/initials as a visible mark | Label clearly as an appearance, not a verified digital signature; reversible placement and saved output match; never imply cryptographic validation |
| CORE-10 Local agent core | v1 must | Headless equivalents for all released capabilities, scoped to granted documents/actions | Same behavior and permission decisions across UI/API/CLI/MCP, with saved/reopened and rendered evidence |
| CORE-11 Diagnostics | v1 must | Version, health, bounded local logs and a previewable export of diagnostics | No document content, passwords, tokens, private keys or full paths in default diagnostics; user explicitly approves sharing |

For an encrypted PDF, opening with an authorized password and honoring its permissions are part of core reading/editing. Adding encryption, complex certificate workflows and signature validation claims are separate qualified features. Existing signed documents must be detected; preservation and the effect of edits must be explicit.

**Deferred from v1:** arbitrary existing-text/image rewriting, Office import/export, OCR, permanent redaction/sanitization, aggressive optimization, calibrated measurements/takeoff, comparison/overlays, PDF/A/X/UA certification claims, advanced XFA, certificate-based signing, cloud collaboration, AI chat/summarization and autonomous document repair. Prioritize OCR, redaction and measurement after pilot evidence; each needs its own safety/fidelity gate. Never substitute an opaque rectangle for redaction.

## 4. Interaction and accessibility requirements

The document is the primary workspace. Proposed layout: document tabs across the top; compact Open/Save/Print and common tools; optional left navigation for pages/bookmarks; contextual annotation/form properties and comment list on the right. Tool labels and keyboard hints must be understandable without knowing PDF terminology. Use progressive disclosure for advanced options.

Familiarity means Ctrl+O/S/Shift+S/P/F/Z/Y, standard selection, Escape to cancel, predictable context menus, a visible active tool, and conventional page-range entry. Verify shortcuts against text-field behavior. Do not copy another product's icons, graphics or branding.

Critical interactions:

1. **Review:** open a packet, find a phrase, highlight it, add a note, save and reopen. The user can tell where the file was saved.
2. **Form:** tab through fields, change choices, save and reopen. Scripts disabled or unsupported are explained before the user relies on a calculated value.
3. **Assembly:** select pages, preview deletion/reorder or merge, apply, undo, then save a new copy. Page labels and physical indexes are distinct.
4. **Failure:** fail a save or kill a worker; show the failed operation and recovery path while keeping the original and user's edits available.
5. **Agent task:** grant access to a file/folder and allowed operations, inspect proposed changes where needed, execute, see who acted and revoke access.

Target: at least 90% unassisted completion of the first three workflows among pilot participants, with no destructive misunderstanding. Record task time and observed errors; compare against their current workflow without claiming a competitor performance advantage before measuring.

Keyboard-only operation, visible focus, high contrast, accessible names and screen-reader operation must cover all v1 tasks. Test Windows Narrator and NVDA, 100/150/200% display scaling, mixed-DPI monitors, maximized/restored windows and common integrated GPUs. Distinguish application accessibility from tagging the PDF itself. English UI is the initial acceptance language; preserve international document text, disclose missing glyph support and test representative Latin, CJK and RTL fixtures without promising full editing support.

Long work must have progress and cancellation. Pointer/keyboard interaction should remain responsive. Error dialogs must name the failed operation and a useful next action, never claim completion after a partial failure.

## 5. Proposed architecture

Reuse the Rust engine, COS model, renderer abstraction and tool registry. Retain the existing Rust-only product/build rule. Do not introduce a new web framework, cloud backend or renderer rewrite as an incidental part of v1. If measured UI accessibility or rendering targets cannot be met, record an explicit architecture decision with a bounded prototype and comparative evidence before changing direction.

```text
Desktop UI          CLI / opt-in MCP          Future web adapter
       \                  |                         /
        BDF application service: actor + policy + revisions + jobs
                   |                       |
          PDF engine operations       Local document store
                   |                 durable saves + recovery
          supervised document workers
                   |
          renderer / parser / format libraries

Reusable support adapter -> same application service and scoped diagnostics
```

The application service owns public operation contracts. It validates caller authority, document revision and capability status before invoking internal engine work. UI visibility is not the authorization mechanism. Keep PDF-format permission checks in the engine as an additional restriction. A document cannot grant itself access through its scripts, text or metadata.

Desktop document processing should run in supervised workers with finite resources. Cancellation or a worker crash must not destroy the application/session's recoverable state. Start by measuring clone/copy costs and isolating open/render; define a safe transactional boundary for edits rather than moving shared mutable state across processes ad hoc. Workers must have only necessary file/network access. Windows process isolation and resource limits need maintained safe wrappers; do not violate the no-unsafe rule to implement them.

Document storage is a shared service, not duplicated glue in UI and automation. It handles staging, flush/replacement, target version checks, permissions, save validation and recovery lifecycle. Keep incremental saves when appropriate, use full rewrites when required, and disclose repairs or unsupported content. Save As must not silently overwrite an unrelated file.

## 6. Identity and authority for humans and agents

**Equal policy enforcement does not mean shared credentials or an agent impersonating a human.** Both enter the same application-service boundary. The agent has its own identity and a revocable grant from the human; it can do no more than the intersection of the human's rights, its grant, current document permissions and release capability policy.

Desktop v1 needs no cloud login. Use the local OS user as the human principal and a local grant store protected by the OS. Proposed grant fields: grant ID, agent/client ID, issuing user, allowed operations, resource IDs or confined roots, session scope, expiry, revocation version and confirmation rules. Client-supplied strings alone do not authenticate a principal. PDF author metadata is only attribution.

Required behavior:

- No default MCP registration, listener, remote support connection or telemetry. Stdio MCP remains explicitly launched; UI control remains explicitly enabled and owner-only.
- New agent grants default to named documents or a selected root and read-only access. Editing/export/overwrite are separately granted capabilities; the UI clearly explains them.
- Human and agent requests use the same capability decisions. Meta tools, batch operations, document resources and previews cannot bypass policy.
- A grant is checked at request entry and again before external side effects. Revocation blocks queued/uncommitted work and future reads; data already delivered cannot be recalled.
- Require concrete previews/confirmation for operations outside standing grants or irreversible effects. Avoid repeated prompts for ordinary edits already covered by the grant.
- Each mutation has a document ID, expected revision and operation ID. Stale revisions return `conflict`; callers cannot silently overwrite newer user work. Export/print retry uses idempotency where feasible and identifies ambiguous completion.
- Audit actor, delegating user, operation, resource identifier, before/after revision, time and result. Do not log raw PDF content or credentials. Local logs are integrity-protected against accidental modification; do not claim resistance to a fully privileged local attacker.
- UI session attachment is explicit. Detached CLI sessions must not silently modify an open UI session or its file. An attached agent sees the same current document revision and undo history, with its changes identified to the user.

Future cloud identity should map into these contracts rather than replacing engine APIs. A future remote service requires its own threat model and policy approval; it must not widen the current loopback-only MCP/control rules as a shortcut.

## 7. API, MCP and job contracts

Version the public schema independently of the app marketing version. Keep current tool names where practical and provide adapters for compatible changes. Preserve documented 1-based external page numbering and displayed-page coordinates; convert exactly once at the engine boundary.

Proposed request envelope:

```json
{
  "schema_version": "1",
  "request_id": "unique-operation-id",
  "session_id": "authorized-session",
  "document_id": "stable-session-document-id",
  "expected_revision": 12,
  "operation": "page_rotate",
  "arguments": {"pages": [1, 2], "degrees": 90}
}
```

Authenticated actor/grant context is bound by the adapter, not accepted merely because a request body names it. The envelope is a proposed service contract, not the current MCP schema.

Responses distinguish success, refusal, partial result, canceled work and failure. Standard error categories: invalid argument, not authorized, not found, unsupported feature, password required, conflict, resource limit, canceled, I/O failure and internal failure. Include actionable detail and retryability without leaking protected paths/content.

Long operations return a job ID with queued/running/succeeded/failed/canceled states, bounded progress, a cancel operation and a result reference. Every batch declares whether it is transactional; partial results identify exactly what committed. Do not call a batch atomic when earlier edits remain after failure.

Transport caps, output caps, timeouts and per-session queue limits apply before expensive allocation. Cancellation acknowledgments distinguish a requested cancel from a completed cancel. Test exact-limit, over-limit, malformed, stale and repeated requests through actual adapters, including compact MCP dispatch.

## 8. Reliability, privacy and performance acceptance

The following numbers are **initial proposed budgets**, to be baselined and ratified during BDF-01, not measurements of current code. Test release builds on a recorded Windows 11 x64 machine with at least a 4-core CPU, 16 GB RAM, SSD, integrated graphics and a 1920×1080 display. Record hardware, OS/driver/build versions, document hashes and cold/warm conditions. Measure at least 30 runs for p95 latency; report regressions per document class rather than averaging away outliers.

| Dimension | Proposed desktop target | Method |
|---|---|---|
| Launch | p95 usable empty window within 2 seconds | Cold launch on reference machine, no hidden preload |
| Ordinary opening | p95 first useful page within 2 seconds for a 100-page, ≤10 MiB office PDF | Embedded fonts, images, links and bookmarks; full completion separately reported |
| Scan/plan opening | p95 first useful page within 5 seconds for a ≤100 MiB fixture | Scans and vector-heavy plans tested separately |
| Interaction | p95 input feedback <100 ms; cached scrolling near 60 fps, p95 frame ≤33 ms | 60-second trace at normal zoom; never wait for all pages before interaction |
| Cancellation | UI acknowledges within 100 ms; worker quiesces or is safely terminated within 2 seconds | Render/search and large-file workloads |
| Memory | ≤500 MiB process-tree peak for ordinary fixture; ≤1.5 GiB for qualified large fixture | Include workers/caches/undo; ten open-edit-close cycles settle within 10% of baseline |
| Recovery | Committed edits checkpointed within 5 seconds while idle; failures visible | Kill application after checkpoint and reopen; original unchanged |
| Stability | Zero crashes, hangs, silent corruption or lost accepted edits in the qualification suite | Invalid inputs may be rejected clearly; no guarantee for every possible PDF |
| Regression budget | No unexplained >10% regression in a ratified metric | Compare identical machine/build configuration and corpus |

Proposed supported envelope for initial qualification: up to 250 MiB per document and 2,000 pages, with complexity/decoded-data limits that may reject a smaller hostile file. These are admission limits, not a promise that every file under them is renderable. Establish a finite total-session budget and preflight before loading. Gracefully refuse larger/unmanageable files rather than risking the OS; expand after evidence from plan-set users.

Data safety requirements apply across all released surfaces:

- Save failure leaves original bytes intact and dirty work available. Flush/rename/permission failures cannot be converted into success. Test local NTFS, locked files and controlled network/synced-folder cases; document supported durability guarantees.
- Detect external changes at commit time. Keep both versions, with Save As/reload/explicit overwrite choices.
- Verify annotations, form values, page geometry/order and untouched content after save/reopen. Compare in an independent reader; internal round trips alone are insufficient.
- Preserve unknown structures where supported; report repairs/unsupported cases. Signed files have explicit edit/save consequences. Never label a digital signature valid from its visual appearance.
- Default to no document upload and no network access from embedded content. Opening an external URL/file requires a clear user/standing-policy decision. Local diagnostics and updates are separate from document processing.
- Recovery and temporary files follow the user's privacy/encryption expectations, have private ACLs and bounded retention. Passwords and signing material are never placed in diagnostic bundles.
- PDF JavaScript is proposed off for untrusted documents, with a per-document enable choice and clear indication when calculations/actions did not run. Supporting simple forms must not require globally trusting scripts.

## 9. Evidence and release gates

Create a licensed, provenance-tracked corpus: synthetic fixtures generated by project code, approved open documents, and private owner-authorized samples for local validation. Never commit proprietary customer files, forbidden fonts/artwork or Adobe observation outputs. Every published/committed generated asset must satisfy attribution policy. External copyleft readers may serve as separate oracle processes only.

BDF-01 establishes an initial 100-file qualification set; the desktop pilot expands to at least 500 cases with workflow coverage, not merely file count. Include scans, forms, annotations, rotated/cropped pages, subset fonts, Unicode, bookmarks, attachments, encryption, signed documents, damaged inputs, cycles, oversized dimensions and decompression bombs. Include recent upstream regression scenarios, including #1368, without assuming old reports remain unfixed.

Every v1 workflow needs engine, automation and real UI evidence plus save/reopen and rendering checks. Maintain a BDF-specific acceptance matrix alongside the upstream parity checklist. Upstream `shipped` means existence plus cited tests; it does not certify BDF usability or production fidelity.

Desktop release requires:

1. Required Windows/Linux build, fmt, clippy, tests, layering, assets, dependency/advisory and parity jobs pass. Scope local testing to affected crates; full sweeps belong in CI.
2. No open P0/P1 issues in the released scope; every security/data-loss finding has a reproducer or a documented, tested exclusion. A feature excluded from UI must also be excluded from ordinary agent access.
3. Core qualification corpus passes semantic and visual review, with no unexplained content loss or blank renders. Tolerances for antialiasing differences are recorded; every material difference is triaged.
4. Fault injection, malformed-input tests and bounded nightly fuzzing pass the agreed campaign; findings become small synthetic regressions. Worker failure cannot terminate the shell or erase accepted work.
5. Performance budgets and pilot usability targets pass; keyboard/screen-reader, mixed-DPI and two printer configurations are exercised on Windows.
6. BDF installer/portable artifacts have correct name, icons, support/update routes, signing, checksums, SBOM and provenance. Clean install, upgrade, rollback and uninstall preserve documents and handle settings/recovery explicitly.
7. No automatic agent/network activation. Windows credential ACL and revocation tests pass. Policy decisions match across UI, CLI, MCP and nested calls.

## 10. Implementation milestones and handoff

Milestones are ordered by evidence dependencies, not estimated agent hours. Do not inherit upstream's effort/readiness percentages as delivery commitments.

| ID | Outcome | Principal work | Exit gate |
|---|---|---|---|
| BDF-00 | Reproducible fork baseline | Install/verify approved Rust toolchain; run current tests; finish/verify brand cleanup; own release/update identity; commit essential architecture/setup docs; immutable CI dependencies | Known-green baseline, reproducible signed-package plan, recorded upstream revision and patch ownership |
| BDF-01 | Measured foundation | 100-file corpus, workflow matrix, profiler/fidelity harness, worker-boundary prototype, dependency/vendor review | Ratified budgets; documented retain/adapt/replace decision for renderer; no claim based only on feature counts |
| BDF-02 | Safe persistence and local control | Review F01–F07; shared save service; conflict detection; bounded transports; root-race investigation; recovery/privacy checks | Negative/fault tests prove original and dirty state survive; local security assumptions verified on Windows |
| BDF-03 | General office beta | CORE-01–08; compact UI; hide unqualified capabilities; Windows usability/accessibility/printing | Full review/form/assembly workflows pass UI, automation and independent-reader checks |
| BDF-04 | Authorized agent beta | Shared application service, local principals/grants, revisions, audit, session attach, jobs and CORE-10–11 | No policy bypass through resources/meta/batches; revocation/conflict/retry tests pass |
| BDF-05 | Desktop v1 | Expand corpus/pilot; installer/signing; support docs; performance/fuzz campaigns; upgrade/rollback | All desktop release gates above |
| BDF-06 | Later web release | Reuse engine/service contracts; web app CI; worker/memory limits; save acknowledgment and browser recovery; local-first privacy | Separate Edge/Chrome browser acceptance; no desktop delay |
| BDF-07 | Reusable support module | Extract proven capability/diagnostic adapters, shared policy contracts and support workflow | A second product integrates without importing PDF types or bypassing its own authorization |

BDF-02 and BDF-04 define the shared service together; do not harden a temporary bypass and then add identity after shipping agent access. BDF-03 UI refinement can proceed against the defined service contract. Web-specific review findings remain tracked even when deferred.

**Claude Code implementation contract:** take one bounded work item; identify relevant requirements/findings, produce a minimal patch, prove a defect regression fails on the baseline, run scoped checks, and include synthetic headless scripts plus `page_render` evidence. UI changes require the real app's opt-in control channel and visual inspection. Record test commands/results, unavailable checks, migration implications and changed public schemas. Never mark a feature shipped based solely on code being present.

**Codex review contract:** independently inspect the diff and surrounding code; verify input limits, persistence, undo, fidelity, authorization and adapter parity; rerun meaningful regression/acceptance checks; challenge test oracles that repeat implementation assumptions; distinguish verified from unverified findings. Review the rebased commit, not an earlier branch state. This is a work agreement, not an automation created by this document.

Human owner approves scope/architecture tradeoffs and releases. CI and review must pass before merging; no direct main pushes. Do not automatically merge upstream main: triage changes, import pinned changes through reviewed PRs and rerun the BDF corpus. Security fixes receive priority, but urgency does not waive release evidence.

## 11. Reusable maintenance/support module boundary

Build only the BDF adapter and stable contracts now. Later extraction should provide version/capability discovery, bounded health diagnostics, policy evaluation, audit events, job control and preview/approval of support actions. The PDF parser, renderer and PDF-specific operation logic remain in BDF.

A support agent gets a narrow diagnostic grant by default. File/content access, uploading a bundle, applying a repair, installing an update or restarting an app are distinct capabilities. Diagnostics are previewable and redacted before sharing. Production repair has a rollback plan and explicit authorization. The module must never gain an unrestricted shell or silently grant itself broader access.

Automatic source-code maintenance belongs in reviewed branches and CI. Production support must not self-modify installed executable code. Later cloud support is opt-in, has retention/access controls, and must work without turning on document uploads for everyone.

## 12. Open decisions that do not block the draft

Defaults proposed for implementation planning: Windows x64; English UI acceptance; existing Rust/egui shell; local-only telemetry/logs; manual update checks; no cloud login; simple signature appearance as a should-have; no advanced text editing in v1.

The owner should settle these at the relevant milestone: distribution/business model; final BDF name and signing identity; ARM64 timing; pilot organizations/documents; contractual large-file requirements; accessibility acceptance depth; first post-v1 vertical feature. Any change to the Rust-only, asset or network rules requires an explicit decision rather than silently changing implementation conventions.

## Revision history

| Date | Revision | Change |
|---|---|---|
| 2026-10-13 | 0.1 handoff | Added implementation specification, work-item index and bounded Claude kickoff links; product scope unchanged. |
| 2026-10-13 | 0.1 | Initial draft after source review; confirmed local-first, general-office core and Windows desktop priority, with web deferred. |
