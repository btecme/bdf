# BDF desktop-core capability inventory

Status: **inventory and profile specification; not enforced at run time yet.** Work item BUILD-003 (deliverable 3), 2026-10-16. The data lives in [`crates/automation/src/capability.rs`](../crates/automation/src/capability.rs), where tests keep it complete; this page is the readable form. Nothing in the product calls the profile today: hiding a button is still a UI matter, and every route in section 5 that is not `Core` is an open bypass until BUILD-005 to BUILD-008 close it.

## 1. What the profile is

BDF v1 is a Windows 11 desktop PDF workbench whose core is the PRD requirements CORE-01 to CORE-08 and CORE-10 ([PRD section 3](bdf-prd-draft.md)). The inherited application ships far more. The profile has four verdicts:

| Verdict | Meaning | Allowed? |
|---|---|---|
| **Core** | In the v1 core, subject to the parameter rules below | Yes |
| **Gated** | Reaches other capabilities (a dispatcher or a route); allowed only if the profile is applied to everything it runs | Only through the gate |
| **Deferred** | Not in v1; planned behind its own safety and fidelity qualification | No |
| **Excluded** | Not in v1 and not planned, or unsafe by default | No |

**Default deny.** A capability that is not listed is denied. For a core tool, a parameter that is not listed is denied, a refused parameter is denied, and an enumerated mode that is excluded or unknown is denied, where an omitted or null mode counts as its default. The tests fail whenever a tool, command, catalogue command, parameter or enumerated mode exists that the inventory does not classify, so a new capability cannot enter the core by being added. `check_tool_call(name, args)` and `check_command(id)` in the module give the decision.

**Limits.** The check inspects tool arguments, not the parameters of the *commands* that `command_run` and `command_batch` run (it only requires those commands to be core). Core commands must validate their own parameters until the command parameter modes are inventoried.

## 2. Counts

| Surface | Total | Core | Gated | Deferred | Excluded |
|---|---|---|---|---|---|
| Automation tools | 163 | 34 | 2 | 121 | 6 |
| Commands (registry and catalogue) | 219 | 54 | 0 | 152 | 13 |
| Routes (section 5) | 31 | 9 | 17 | 3 | 2 |

Of the commands, 27 are named only by the tool catalogue (planned or cloud items with no registry entry); all are deferred or excluded.

## 3. Core tools (by requirement)

| Requirement | Tools |
|---|---|
| CORE-01 | `bookmark_list`, `doc_close`, `doc_info`, `doc_inspect`, `doc_list`, `doc_open`, `link_list`, `page_render`, `render_preview` |
| CORE-02 | `text_extract`, `text_find` |
| CORE-03 | `comment_add`, `comment_delete`, `comment_edit`, `comment_list`, `comment_reply`, `comment_set_status` |
| CORE-04 | `form_fields`, `form_fill` |
| CORE-05 | `doc_combine`, `doc_split`, `page_delete`, `page_extract`, `page_insert_blank`, `page_insert_file`, `page_move`, `page_rotate` |
| CORE-06 | `doc_save` |
| CORE-07 | `doc_print`, `printer_options`, `printers` |
| CORE-08 | `edit_redo`, `edit_undo` |
| CORE-10 | `command_list` |
| CORE-10 (gated) | `command_batch`, `command_run` |

Requirements with no core tool or command: **CORE-09** (simple signature appearance, v1 "should", BUILD-016: `fill_sign_add`, `stamp_custom` stay deferred) and **CORE-11** (diagnostics: no tool or command exists yet; BUILD-014).

## 4. Parameter modes of core tools

Parameters a core tool refuses (any value but false or null), and enumerated modes split into core and excluded. Every other parameter of a core tool is allowed and listed in the module.

| Tool | Parameter | Default when omitted or null | Core | Excluded |
|---|---|---|---|---|
| `doc_save` | `flatten_fill_sign` | (none) | other values | **refused** |
| `doc_split` | `bookmarks` | (none) | other values | **refused** |
| `doc_split` | `max_mb` | (none) | other values | **refused** |
| `page_extract` | `delete` | (none) | other values | **refused** |
| `page_render` | `hide_text_block` | (none) | other values | **refused** |
| `comment_add` | `endings` | (none: required, or only read by an excluded mode) | `None`, `OpenArrow`, `ClosedArrow` | `Square`, `Circle`, `Diamond`, `Butt`, `ROpenArrow`, `RClosedArrow`, `Slash` |
| `comment_add` | `type` | (none: required, or only read by an excluded mode) | `note`, `highlight`, `underline`, `strikeout`, `rectangle`, `arrow`, `ink`, `textbox` | `squiggly`, `replace`, `oval`, `line`, `stamp`, `polygon`, `cloud`, `polyline`, `callout`, `caret`, `attachment` |
| `comment_set_status` | `status` | (none: required, or only read by an excluded mode) | `none`, `accepted`, `rejected`, `cancelled`, `completed` | (none) |
| `doc_combine` | `field_conflicts` | `rename` | `rename` | `merge` |
| `doc_print` | `binding` | (none: required, or only read by an excluded mode) | (none) | `left`, `right` |
| `doc_print` | `booklet_subset` | (none: required, or only read by an excluded mode) | (none) | `both`, `front`, `back` |
| `doc_print` | `comments_forms` | `document-and-markups` | `document`, `document-and-markups` | `document-and-stamps`, `form-fields-only` |
| `doc_print` | `duplex` | `off` | `off` | `long-edge`, `short-edge` |
| `doc_print` | `layout` | `fit` | `fit`, `actual` | `shrink`, `custom`, `multiple`, `booklet`, `poster` |
| `doc_print` | `order` | (none: required, or only read by an excluded mode) | (none) | `horizontal`, `horizontal-reversed`, `vertical`, `vertical-reversed`, `cut-stack` |
| `doc_print` | `orientation` | `auto` | `auto`, `portrait`, `landscape` | (none) |
| `doc_print` | `subset` | `all` | `all`, `odd`, `even` | (none) |
| `page_rotate` | `orientation` | `all` | `all` | `landscape`, `portrait` |
| `page_rotate` | `subset` | `all` | `all` | `even`, `odd` |

A mode that is omitted or null runs as its default, so the profile judges the default exactly like an explicit value; every default is a core mode, and the tools read the same constants (`crates/automation/src/defaults.rs`) that the inventory names, so the two cannot drift. `doc_print.comments_forms` defaults to `document-and-markups` (print what is on screen, comments included). That default is **core by decision** (Codex review R3): refusing it would deny every ordinary print. `document-and-stamps` and `form-fields-only` stay excluded.

Why these: annotation kinds follow CORE-03 (note, highlight, underline, strikeout, rectangle, arrow, freehand, text box); printing follows CORE-07 (actual size or fit; poster, booklet, shrink and multiple-up layouts could scale large sheets silently); `flatten_fill_sign`, extract-and-`delete` and split by bookmarks or size are irreversible or outside the CORE-05 list; `page_render.hide_text_block` belongs to text editing.

## 5. Routes: the other ways in (inherited bypasses)

Calling a tool by name is one route. These are the others. A route is **Core** when it only reads or manages the window; **Gated** when it can reach tools or commands without the profile.

| Route | Kind | Verdict | What it reaches, and what it needs | Owner |
|---|---|---|---|---|
| `cli:info` | CLI subcommand | Core | Reads a document summary. Opens the file named on the command line, outside any grant. | BUILD-006 |
| `cli:render` | CLI subcommand | Core | Renders one page to an image file. Same file-access note as `info`. | BUILD-006 |
| `cli:text` | CLI subcommand | Core | Extracts text. Same file-access note as `info`. | BUILD-006 |
| `cli:edit` | CLI subcommand | Gated | Rotate, delete, move, insert blank, title, author, full rewrite in one call, writing the output file. Runs engine edits directly, not through tool or command checks. | BUILD-007 |
| `cli:combine` | CLI subcommand | Gated | Merges PDFs into an output file without the tool layer. | BUILD-007 |
| `cli:extract` | CLI subcommand | Gated | Extracts pages into an output file without the tool layer. | BUILD-007 |
| `cli:split` | CLI subcommand | Gated | Splits into files in a directory without the tool layer. | BUILD-007 |
| `cli:batch` | CLI subcommand | Deferred | Runs convert, OCR and optimize over files and folders; all three are deferred from v1. | BUILD-007 |
| `cli:check` | CLI subcommand | Excluded | Robustness sweep over a corpus for development; not a user capability. | BUILD-007 |
| `cli:audit` | CLI subcommand | Deferred | Finds faux redactions; redaction is deferred. | BUILD-007 |
| `cli:tools` | CLI subcommand | Core | Lists every tool and schema, including deferred ones, so the catalogue is visible to agents. | BUILD-008 |
| `cli:run` | CLI subcommand | Gated | Runs any tool by name, and any script of tools, with no profile check: the main inherited bypass for tools and for `command_run`. | BUILD-007 |
| `cli:mcp` | CLI subcommand | Gated | Starts the MCP server on stdin/stdout (opt-in). Its routes are listed separately. | BUILD-008 |
| `cli:ui` | CLI subcommand | Gated | Drives a running app through the control channel. Its methods are listed separately. | BUILD-005 |
| `mcp:tools/list` | MCP method | Core | Lists every tool (all 163) in full mode; the compact list is a convenience, not a restriction. | BUILD-008 |
| `mcp:tools/call` | MCP method | Gated | Calls any tool by name with no profile check. | BUILD-008 |
| `mcp:tool_search` | MCP meta tool (compact mode) | Core | Searches all tools, including deferred ones. | BUILD-008 |
| `mcp:tool_call` | MCP meta tool (compact mode) | Gated | Calls any tool by name even though compact `tools/list` shows only a core set: the compact list does not restrict what can be called. | BUILD-008 |
| `mcp:resources/read` | MCP method | Gated | Reads document information, text, page text and page images by URI, equal to doc_info, text_extract and page_render, outside any per-tool grant. | BUILD-008 |
| `mcp:resources/list` | MCP method | Gated | Lists open documents and the command catalogue (`pdfcraft://document`, `pdfcraft://commands`); reveals document names. | BUILD-008 |
| `control:ui.command` | UI control method | Gated | Runs any registry command in the running app, including deferred ones. | BUILD-005 |
| `control:ui.open` | UI control method | Gated | Opens any file path in the running app. | BUILD-005 |
| `control:ui.key/type/click/drag/move/focus/set` | UI control methods | Gated | Injects input events and sets values; reaches whatever the visible UI reaches, including commands the profile denies. | BUILD-005 |
| `control:ui.screenshot/inspect/state/commands` | UI control methods | Gated | Reads what is on screen, including document content and file names. | BUILD-005 |
| `control:ui.dock/dock.reset/resize/quit` | UI control methods | Core | Window layout and lifecycle only. | BUILD-005 |
| `app:file-association-open` | OS entry point | Core | Explorer Open with, double click, drag and drop, recent files and the single-instance hand-off open files as tabs. | BUILD-006 |
| `app:shell-verb-edit` | OS entry point | Core | "Edit with PdfCraft" on PDF files opens each selected PDF. | BUILD-006 |
| `app:shell-verb-create-pdf` | OS entry point | Deferred | The "Create PDF" verb on image files (png, jpg, tif, gif, bmp, jp2 ...) runs `--create-images`, a creation flow outside the v1 list. | BUILD-011 |
| `app:cli-option-flags` | OS entry point | Gated | Any `--name value` on the app command line is applied as a setting override. | BUILD-008 |
| `app:update-check` | OS entry point | Excluded | Help > Check for updates (`help.check_updates`) asks api.github.com for the latest release of the upstream project (`storytold/pdfcraft`). It is started by the user, but `command_run` and `ui.command` reach it too; BDF has no update endpoint, and the network stays off by default (identity migration, BUILD-003 deliverables 4 and 5). | BUILD-003 |
| `app:ui-menus-toolbars-palette` | UI | Gated | Menus, the toolbar, the command palette and the tool panel build from the registry and catalogue and show deferred items; hiding a button does not deny the command (`ui.command`, `command_run`, shortcuts reach it). | BUILD-011 |

The main bypasses, in order of consequence: `pdfcraft-cli run` and `mcp:tools/call` call any tool with no check; compact-mode `tool_call` calls tools the compact list hides; `command_run`, `command_batch` and `ui.command` run any registry command; MCP resources read document content outside per-tool grants; the UI shows deferred tools and commands, so hiding a button does not deny them.

## 6. Deferred and excluded, by reason

### Tools

- **Deferred**: A simple signature appearance is CORE-09 (v1 should, BUILD-016), not yet qualified. `fill_sign_add`, `fill_sign_date_format`, `stamp_custom`
- **Deferred**: AI summarization is deferred from v1 (and needs a provider, so the network). `comments_summarize`
- **Deferred**: Accessibility authoring and review tools are outside the v1 list. `accessibility_autotag_apply`, `accessibility_autotag_preview`, `accessibility_check`, `accessibility_figures`, `accessibility_fix`, `accessibility_move_tag`, `accessibility_report`, `accessibility_review_html`, `accessibility_review_load`, `accessibility_review_record`, `accessibility_review_save`, `accessibility_set_alt`, `accessibility_set_tag_property`, `accessibility_tags`
- **Deferred**: Adding encryption or removing protection is a separate qualified feature. `doc_protect`, `doc_unprotect`
- **Deferred**: Aggressive optimization is deferred from v1. `doc_audit_space`, `doc_optimize`, `doc_reduce`
- **Deferred**: Batch processing and action wizards are deferred; they run deferred operations. `action_list`, `action_run`, `doc_batch`
- **Deferred**: Bookmark and link authoring is deferred; v1 only views them. `bookmark_add`, `bookmark_delete`, `bookmark_from_structure`, `bookmark_move`, `bookmark_rename`, `bookmark_set_page`, `link_add`, `link_delete`, `link_edit`, `links_from_urls`, `links_remove`
- **Deferred**: Certificate-based signing and signature validation claims are deferred from v1. `sign_document`, `sign_id_create`, `sign_keychain_ids`, `sign_list`, `sign_trust`, `sign_windows_ids`
- **Deferred**: Comment tools beyond add, edit, delete, reply and status are deferred. `comment_image_preview`, `comment_lock`, `comment_mark`, `comments_hide`
- **Deferred**: Comparison and overlays are deferred from v1. `doc_compare`, `doc_compare_mark`, `doc_compare_report`
- **Deferred**: Form authoring, data exchange and field reset are outside the v1 fill-only scope. `doc_export_data`, `doc_import_data`, `form_actions`, `form_add_field`, `form_delete_field`, `form_detect_fields`, `form_merge_data`, `form_reset`, `form_set_actions`, `form_set_image`, `form_set_props`, `form_tab_order`
- **Deferred**: Measurement and takeoff are deferred from v1. `measure_area`, `measure_distance`, `measure_export`, `measure_info`, `measure_list`, `measure_perimeter`, `measure_scale`, `measure_snap`
- **Deferred**: Metadata editing, document creation and view settings are outside the v1 list. `doc_create`, `doc_create_multiple`, `doc_initial_view`, `doc_set_info`, `view_reading_colors`
- **Deferred**: OCR is deferred from v1 (needs its own safety and fidelity gate). `ocr_recognize`, `ocr_recognize_files`, `ocr_status`, `page_enhance_camera`
- **Deferred**: Office and other export formats are deferred from v1. `doc_export_all_images`, `doc_export_images`, `doc_export_office`, `doc_export_text`
- **Deferred**: PDF/A, preflight and print readiness claims are deferred from v1. `pdfa_convert`, `pdfa_verify`, `print_readiness`
- **Deferred**: Page assembly beyond the v1 operations, and page marks, are deferred. `doc_background`, `doc_flatten`, `doc_header_footer`, `doc_remove_marks`, `doc_watermark`, `page_duplicate`, `page_number`, `page_replace`, `page_set_box`
- **Deferred**: Permanent redaction and sanitization are deferred from v1 (never an opaque rectangle). `doc_audit_redactions`, `doc_hidden_info`, `doc_remove_hidden`, `redact_apply`, `redact_clear`, `redact_mark`
- **Deferred**: Revision access arrives with the shared service and revision contracts (BUILD-007). `doc_open_revision`, `doc_revisions`
- **Deferred**: Rewriting existing text or images is deferred from v1. `content_delete`, `content_list`, `content_update`, `image_edit`, `image_hit`, `image_save`, `object_list`, `object_move`, `page_add_image`, `page_add_text`, `page_images`, `text_edit`, `text_edit_layout`, `text_font_files`, `text_font_import`, `text_fonts`, `text_lines`, `text_paragraphs`
- **Excluded**: Document scripts and the JavaScript console are not supported in v1; they must be identified, not run. `form_set_script`, `js_document_scripts`, `js_enabled`, `js_run`, `js_set_document_script`
- **Excluded**: Following a link can launch an external target (a program, a URL or another file). `link_open`

### Commands

- **Deferred**: Accessibility authoring is outside the v1 list. `a11y.alt_text`, `a11y.autotag`, `a11y.check`, `a11y.reading_options`, `a11y.reading_order`, `a11y.report`, `a11y.tags`
- **Deferred**: Action wizards run deferred operations. `actions.archive`, `actions.distribution`, `actions.make_accessible`, `actions.optimize_scans`, `actions.wizard`
- **Deferred**: Adding or removing protection is a separate qualified feature. `protect.certificate`, `protect.password`, `protect.properties`, `protect.remove`, `protect.remove_hidden`, `protect.unlock`
- **Deferred**: Annotation kinds and tools beyond the v1 set are deferred. `comment.attach`, `comment.callout`, `comment.caret`, `comment.circle`, `comment.cloud`, `comment.eraser`, `comment.export`, `comment.flatten`, `comment.hide_all`, `comment.import`, `comment.line`, `comment.polygon`, `comment.polyline`, `comment.replace`, `comment.squiggly`, `comment.stamp`, `comment.summarize`
- **Deferred**: Bookmark authoring is deferred; v1 views bookmarks. `bookmark.add`, `bookmark.from_structure`
- **Deferred**: Certificate signing, fill-and-sign and validation claims are deferred (a simple appearance is CORE-09, BUILD-016). `sign.certify`, `sign.certify_invisible`, `sign.digital`, `sign.fill.check`, `sign.fill.cross`, `sign.fill.date`, `sign.fill.dot`, `sign.fill.initials`, `sign.fill.initials.change`, `sign.fill.initials.remove`, `sign.fill.line`, `sign.fill.signature`, `sign.fill.signature.change`, `sign.fill.signature.remove`, `sign.fill.text`, `sign.panel`, `sign.timestamp`, `sign.validate`
- **Deferred**: Comparison is deferred from v1. `doc.compare`
- **Deferred**: Creating PDFs from other files is outside the v1 list. `create.blank`, `create.clipboard`, `create.file`, `create.images`, `create.multiple`, `create.multiple_separate`
- **Deferred**: Editing existing content and page marks is deferred from v1. `edit.advanced_search`, `edit.attach`, `edit.background`, `edit.background.remove`, `edit.background.update`, `edit.bates`, `edit.edit_text`, `edit.header_footer`, `edit.header_footer.remove`, `edit.header_footer.update`, `edit.image`, `edit.link`, `edit.links_from_urls`, `edit.remove_links`, `edit.snapshot`, `edit.text`, `edit.watermark`, `edit.watermark.remove`, `edit.watermark.update`
- **Deferred**: Export formats are deferred from v1. `export.all_images`, `export.docx`, `export.html`, `export.image`, `export.pptx`, `export.ps`, `export.rtf`, `export.text`, `export.xlsx`
- **Deferred**: Form authoring, data exchange and flattening are outside the v1 fill-only scope. `form.add.button`, `form.add.checkbox`, `form.add.combo`, `form.add.date`, `form.add.image`, `form.add.list`, `form.add.radio`, `form.add.signature`, `form.add.text`, `form.clear`, `form.detect`, `form.export_data`, `form.field.properties`, `form.flatten`, `form.import_data`, `form.merge_data`, `form.prepare`, `form.tab_order.column`, `form.tab_order.row`, `form.tab_order.structure`
- **Deferred**: Home-screen conveniences outside the v1 list. `file.pin_folder`
- **Deferred**: Measurement is deferred from v1. `measure.area`, `measure.distance`, `measure.export`, `measure.geo`, `measure.info`, `measure.perimeter`, `measure.scale`, `measure.snap`
- **Deferred**: OCR is deferred from v1. `ocr.correct`, `ocr.enhance`, `ocr.enhance_camera`, `ocr.recognize`, `ocr.recognize_batch`
- **Deferred**: Optimization is deferred from v1. `optimize.advanced`, `optimize.audit`, `optimize.reduce`
- **Deferred**: PDF/A, PDF/X and PDF/UA claims are deferred from v1. `standards.pdfa`, `standards.pdfua`, `standards.pdfx`
- **Deferred**: Page operations beyond the v1 set are deferred. `page.boxes`, `page.copy`, `page.crop`, `page.cut`, `page.duplicate`, `page.number`, `page.paste`, `page.replace`, `page.transitions`
- **Deferred**: Permanent redaction is deferred from v1. `redact.apply`, `redact.clear`, `redact.mark`, `redact.pages`, `redact.properties`, `redact.sanitize`, `redact.search`
- **Deferred**: Preflight is deferred from v1. `preflight.run`
- **Deferred**: Prepress is deferred from v1. `prepress.convert_colors`, `prepress.hairlines`, `prepress.marks`, `prepress.output_preview`
- **Deferred**: Search indexing is deferred from v1. `search.index`
- **Excluded**: Document scripts are not supported in v1. `js.actions`, `js.console`, `js.document`, `tools.document_js`, `tools.js_console`
- **Excluded**: Needs a cloud AI provider; the network stays off by default. `ai.ask`, `ai.summary`, `ai.translate`
- **Excluded**: Opens an external site or checks for updates; BDF has no support URL or update endpoint yet, and the network stays off by default. `help.app_page`, `help.check_updates`, `help.discord`, `help.github`, `help.website`

## 7. Keeping it true

Run `cargo test -p pdfcraft-automation capability`. A new tool, command, catalogue command, core-tool parameter or enumerated mode fails a test until it is classified; a removed one fails until its row is removed; a core tool whose command is not core (or the reverse) fails. Changing a verdict to `Core` is a product decision (it widens the v1 scope) and belongs in a reviewed change that also cites the requirement. The counts above are for 2026-10-16.

## Revision history

| Date | Change |
|---|---|
| 2026-10-16 | First inventory: 163 tools, 219 commands, 31 routes; profile API and completeness tests; not enforced at run time. |
