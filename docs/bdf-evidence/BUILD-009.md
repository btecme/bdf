# BUILD-009: evidence record (inventory and design, nothing migrated yet)

Status: **started 2026-10-31, inventory only.** No adapter has been moved onto the service; the work item stays `pending`. Branch `feat/build-009-adapter-migration`, from `main` at `c1c38b07` (BUILD-008's two slices are merged there, not yet independently accepted by Codex). This file is where the inventory, the decisions and the per-surface results go.

## 1. What the work item asks

Open, view, rotate, undo, redo, save and close must dispatch through the application service from the desktop, the CLI and the MCP server (including its compact/meta tools, command dispatch, batches, resources and the UI control channel), with no direct-session or policy bypass; a per-surface allowed/denied/conflict/fault matrix; and the superseded duplicate orchestration deleted. Stop condition: any production alternate entry point that bypasses service policy means finishing the migration before adding features.

## 2. Inventory of what bypasses the service today

Read from the tree at `c1c38b07`; counts are of source lines that name the thing, not of call paths.

| Surface | How it reaches a document today | Slice operations it performs itself |
|---|---|---|
| Desktop (`crates/ui-egui`) | The app owns an engine `Session` (`lib.rs:871`) and `apply_edit`, `undo`, `redo`, `save_bytes` and `close` are called on it from `editing.rs`, `commands.rs`, `canvas.rs`, `files.rs`, `sign_ui.rs` and others (about 440 mentions of the engine across 40-odd files). | all six |
| Plain CLI subcommands (`apps/pdfcraft-cli/src/main.rs`) | Each subcommand builds its own engine `Session` (`:299`, `:357`, `:453`, `:511`, `:527`, and `Session::new().combine` at `:502`). | open, rotate, save |
| CLI `run` / `batch` / MCP (`crates/automation`) | `Automation` owns a `Session` and 163 tools (`lib.rs` 2,900 lines, `tools.rs` 1,300, `mcp.rs` 580); `doc_open`, `page_rotate`, `edit_undo`, `edit_redo`, `doc_save`, `doc_close` run on it directly. | all six |
| Grant commands (`apps/pdfcraft-cli/src/access.rs`) | Issue grants and show activity; they do not open documents. | none |
| The service (`crates/app-service`) | Owns a private `Session`; eight operations (`document.open/query/save/close`, `page.rotate`, `edit.undo/redo/history`); used by tests only. | all six, with policy |

The release profile (`crates/automation/src/capability.rs`) lists **163 tools: 34 Core, 121 Deferred, 6 Excluded, 2 Gated.** The service has eight operations. So 28 of the 34 Core tools have no service operation yet: `doc_info`, `doc_list`, `doc_inspect`, `doc_combine`, `doc_split`, `doc_print`, `page_delete`, `page_move`, `page_extract`, `page_insert_blank`, `page_insert_file`, `page_render`, `render_preview`, `text_extract`, `text_find`, `link_list`, `bookmark_list`, `comment_*` (six), `form_fields`, `form_fill`, `command_list`, `printers`, `printer_options`.

## 3. The decision this forces

Moving only the six slice operations is not enough on its own, because documents live in exactly one session: a document opened through the service cannot be seen by the other 28 Core tools, which run on the automation session. Two sources of truth for open documents is what the work item forbids. There are three ways out:

* **A. Service is the only owner; migrate every Core tool as an operation, in groups.** The adapters keep working throughout (a tool not yet migrated is refused with "not available in this build" for clients, and keeps running only for the person on the old path **until its group lands**, which is the interim bypass this item exists to remove). Largest change; ends with one source of truth and no regression for the person.
* **B. Shrink the shipped surface to what the service has, until the rest is migrated.** The v1 release profile for the CLI, MCP and UI would be the eight operations; the other 28 Core tools are hidden and refused everywhere (the profile and the server agree). No bypass at any point; **a visible regression** in the shipped product (page delete/move/extract, text extraction, rendering, comments, forms, combine/split, print) until each group is migrated.
* **C. Give the service a general "run this engine edit" operation** that wraps the existing tool implementations behind the same policy, capability and audit checks. Fastest, but it moves the policy surface from a closed list of typed operations to a wide pass-through, which weakens exactly what BUILD-007 and BUILD-008 built (typed requests, per-operation capabilities, the journal that ties undo to the capability of the change reverted).

## 4. Recommendation

**A, in this order:** (1) the read-only operations the other tools need first (`doc_info`, `doc_list`, `doc_inspect`, `page_render`, `render_preview`, `text_extract`, `text_find`, `link_list`, `bookmark_list`), because they carry no write risk and unblock the viewing slice; (2) the page edits (`page_delete/move/extract/insert_*`, `doc_combine/split`); (3) comments and forms; (4) printing; each group with its own capability and the per-surface matrix. Adapters move onto the service in the same step as the group they use, never ahead of it. Interim state, stated plainly: until a group lands, its tools stay on the old path for the person only and are **not available to any grant-authenticated client**; the grant policy is already not applied to the opt-in CLI/MCP (BUILD-008 section 6), and this plan does not widen that.

## 5. Not started, not claimed

Nothing here is migrated, tested or reviewed. No surface routes through the service. BUILD-008's fixes are merged but not yet retested by Codex, and this work builds on them.

## Revision history

| Date | Change |
|---|---|
| 2026-10-31 | First version: inventory of the entry points that bypass the service and the migration options. |
