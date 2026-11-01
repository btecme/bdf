# BUILD-009: evidence record (inventory and design, nothing migrated yet)

Status: **started 2026-10-31; first slice implemented and locally tested (section 5), not hosted, not reviewed.** One adapter, the grant-gated MCP server, runs on the service; the desktop, the CLI subcommands and the older MCP server do not. The work item stays `pending`. Branch `feat/build-009-adapter-migration`, from `main` at `c1c38b07` (BUILD-008's two slices are merged there, not yet independently accepted by Codex). This file is where the inventory, the decisions and the per-surface results go.

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

## 4. Recommendation (adopted: option A, see section 5)

**A, in this order:** (1) the read-only operations the other tools need first (`doc_info`, `doc_list`, `doc_inspect`, `page_render`, `render_preview`, `text_extract`, `text_find`, `link_list`, `bookmark_list`), because they carry no write risk and unblock the viewing slice; (2) the page edits (`page_delete/move/extract/insert_*`, `doc_combine/split`); (3) comments and forms; (4) printing; each group with its own capability and the per-surface matrix. Adapters move onto the service in the same step as the group they use, never ahead of it. Interim state, stated plainly: until a group lands, its tools stay on the old path for the person only and are **not available to any grant-authenticated client**; the grant policy is already not applied to the opt-in CLI/MCP (BUILD-008 section 6), and this plan does not widen that.

## 5. Decision, and the first slice

**Decision (owner, 2026-10-31): option A**, migrate every Core tool in groups. One refinement follows from the specification (section 5: an MCP invocation without a valid grant "yields only public version/capability help ... do not grandfather unrestricted MCP filesystem access"): the **MCP server is an agent surface, so it moves to the service as a grant-only server**, while the desktop and the CLI subcommands (the person's own surfaces) stay on the old path until their groups land. The older `pdfcraft-cli mcp` (no `--client`) therefore remains, unchanged except for a stderr notice that it is not limited by a grant, **only until the groups cover its tools; removing it is part of this work item, not optional** (stop condition).

**Slice 1: `pdfcraft-cli mcp --client NAME` over the service.**

* `crates/automation/src/client_mcp.rs`: `ClientMcpServer` holds a `Service` and no engine session. Eight tools, one per service operation (`doc_open`, `doc_info`, `page_rotate`, `edit_undo`, `edit_redo`, `edit_history`, `doc_save`, `doc_close`), plus `bdf_help` for a caller without a grant. Arguments are read strictly (no coercion, unknown names refused), and a changing tool must name its `revision`. The line reader and its message bound are shared with the older server (`mcp::serve_lines`), not copied.
* `apps/pdfcraft-cli/src/access.rs` (`mcp_client`): the secret comes from `BDF_CLIENT_SECRET`; a grant that is not accepted gives help only, and every way of not being accepted gives the same answer; the activity log (`<state>/activity`) is attached, so every call is recorded.
* `pdfcraft-automation` now depends on `pdfcraft-app-service` (it was a dev-dependency); the edge was already allowed by `cargo xtask layers`.

**Tests (local, Windows 11 x64, Rust 1.99.0, two jobs).** `crates/automation/tests/client_mcp.rs` (8): help only without a grant; a read-only grant opens inside its folder and is refused every change with a reason, leaving the file and the document as they were; an editing grant rotates, undoes, redoes, saves and closes against the revisions it was given, a stale revision is a conflict that changes nothing, closing with unsaved changes is refused, and what was saved reopens as edited; a file outside the granted folder is refused for reading and for saving and nothing is written there; a grant revoked while the server runs stops the next call; arguments are read strictly; an over-long message is refused and the next one served; one tool per service operation. `apps/pdfcraft-cli/tests/access.rs` (4 more, through the real binary): the full flow with the activity record shown and verified and the secret absent from it; a wrong, a missing and an unknown-client secret get the same help-only answer; a revocation from another process stops a running client; the two servers' options are not mixed. The whole automation and CLI suites pass (automation 40 + 163 + 8 + 8 + 4 + 2 + 3, CLI all), as do `cargo fmt --check`, clippy with `-D warnings` for the three crates, `cargo xtask layers` and `cargo xtask assets`.

**Tests seen to fail with the thing they test broken:** listing the document tools without a grant fails the no-grant test; skipping the argument-name check fails the strictness test; letting a negative revision wrap to a number fails the strictness test; leaving the audit log off fails the activity test.

## 6. Not done, and not claimed

* **Only the eight operations exist for a client.** The other 28 Core tools (section 2) have no service operation yet, so a grant-authenticated client cannot extract text, render pages, edit comments or forms, combine, split, delete or move pages, or print. This is the interim cost of option A for the MCP surface, and it is the product behaviour until the groups land: nothing is hidden by accident, the tool list is the truth.
* **Not migrated:** the desktop (about 440 engine references across 40 files), the plain CLI subcommands, CLI `run`/`batch`, the UI control channel, and `pdfcraft-cli mcp` without `--client`. Each still bypasses the service and its policy. Until they move, the grant policy does not protect them and agent access through the whole product is **not released**.
* **MCP resources and compact/meta tools** are not on the client server (no resources are listed; there is no `tool_search`/`tool_call`). The work item asks for them to go through the service: not done.
* The client server was not exercised by a real agent client, only by the tests above. Hosted CI has not run on this branch. Codex has not reviewed it. The 10-second grant-lock timeout path and a lock file made by another account remain untested (BUILD-008 section 7).
* `BDF_CLIENT_SECRET` is visible to the account's own processes that can read the environment of this one; it is not on the command line. Whether that is enough for the release is a question for the release profile, not settled here.
* BUILD-008's fixes are merged but not yet retested by Codex, and this work builds on them.

## Revision history

| Date | Change |
|---|---|
| 2026-10-31 | First version: inventory of the entry points that bypass the service and the migration options. |
| 2026-10-31 | Owner chose option A; first slice: the grant-gated MCP server over the service, with tests and mutation proofs. |
