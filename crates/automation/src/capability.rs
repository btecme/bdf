//! The desktop-core capability inventory and profile (BUILD-003, findings F07 and F11).
//!
//! BDF v1 is a Windows desktop PDF workbench whose "core" is CORE-01 to CORE-08 and CORE-10 of the PRD.
//! The inherited application ships far more (about 160 automation tools and 220 commands). This module
//! is the reviewed list of what is in the core and why everything else is not:
//!
//! * [`TOOLS`]: every automation tool, its verdict, the requirement it serves, the parameters a core
//!   tool accepts, the parameters it refuses and the enumerated modes that are excluded;
//! * [`COMMANDS`]: every registry command and every command the tool catalogue names (including
//!   planned ones), with a verdict;
//! * [`ROUTES`]: the other ways to reach those capabilities (CLI subcommands, MCP methods and
//!   resources, the UI control channel, the operating-system entry points), each with the gate it needs.
//!
//! **Default deny.** Only [`Verdict::Core`] is allowed; [`Verdict::Gated`] is allowed only when the
//! profile is applied to what it runs ([`check_tool_call`] does that for `command_run` and
//! `command_batch`); a capability that is not listed is denied. The tests in this file fail when a
//! tool, command, catalogue command, parameter or enumerated mode exists that is not classified, so a
//! new capability cannot enter the core by being added.
//!
//! **Not enforced at run time yet.** Nothing calls [`check_tool_call`] from `Automation::call`, the CLI,
//! the MCP server or the UI today; hiding is still a UI matter and the inherited bypass routes in
//! [`ROUTES`] are open. Wiring the profile into the shared application service and the release policy
//! is BUILD-007 and BUILD-008. This module is the specification those items implement, kept honest by
//! its tests. The parameters of the *commands* a dispatcher runs are not inspected either.

use serde_json::Value;

/// What the desktop-core profile says about a capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// In the v1 core: allowed (subject to the parameter rules).
    Core,
    /// Reaches other capabilities: allowed only if the profile is applied to each of them.
    Gated,
    /// Not in v1, planned behind its own qualification: denied.
    Deferred,
    /// Not in v1 and not planned (or unsafe by default): denied.
    Excluded,
}

/// An enumerated parameter whose values are split into core and excluded modes.
#[derive(Clone, Copy, Debug)]
pub struct EnumRule {
    pub param: &'static str,
    /// The mode the tool uses when the parameter is omitted or null (a constant the tool itself reads,
    /// in `defaults.rs`), judged like an explicit value. `None` when the parameter is only read in a
    /// mode that is excluded anyway (such as `order` under the `multiple` layout), so an omitted value
    /// selects nothing.
    pub default: Option<&'static str>,
    pub core: &'static [&'static str],
    pub excluded: &'static [&'static str],
}

/// One automation tool.
#[derive(Clone, Copy, Debug)]
pub struct ToolCap {
    pub name: &'static str,
    pub verdict: Verdict,
    /// The PRD requirement a core or gated tool serves; empty for the rest.
    pub requirement: &'static str,
    pub why: &'static str,
    /// For a core tool: every parameter it may be called with.
    pub params: &'static [&'static str],
    /// Parameters a core tool refuses when set to anything but false or null.
    pub denied_params: &'static [&'static str],
    pub enums: &'static [EnumRule],
}

/// One command id (a registry command, or a command only the tool catalogue names).
#[derive(Clone, Copy, Debug)]
pub struct CommandCap {
    pub id: &'static str,
    pub verdict: Verdict,
    pub requirement: &'static str,
    pub why: &'static str,
}

/// A way to reach capabilities other than calling a tool by name.
#[derive(Clone, Copy, Debug)]
pub struct Route {
    pub id: &'static str,
    pub kind: &'static str,
    pub verdict: Verdict,
    /// What it can reach today, and what it needs before it can be in the core profile.
    pub note: &'static str,
    /// The work item that closes the gap.
    pub owner: &'static str,
}

/// The answer to "may this run in the desktop-core profile?".
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny(String),
}

impl Decision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Decision::Allow)
    }
}

/// Requirements with no core tool or command yet: recorded so their absence is deliberate.
pub const GAPS: &[(&str, &str)] = &[
    ("CORE-09", "Simple signature appearance is v1 'should' (BUILD-016); fill_sign_add and stamp_custom stay deferred until qualified."),
    ("CORE-11", "Diagnostics: no tool or command exists yet (version, health, bounded logs, previewable export); BUILD-014 adds it."),
];

#[rustfmt::skip]
pub static TOOLS: &[ToolCap] = &[
    ToolCap { name: "accessibility_autotag_apply", verdict: Verdict::Deferred, requirement: "", why: "Accessibility authoring and review tools are outside the v1 list.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "accessibility_autotag_preview", verdict: Verdict::Deferred, requirement: "", why: "Accessibility authoring and review tools are outside the v1 list.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "accessibility_check", verdict: Verdict::Deferred, requirement: "", why: "Accessibility authoring and review tools are outside the v1 list.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "accessibility_figures", verdict: Verdict::Deferred, requirement: "", why: "Accessibility authoring and review tools are outside the v1 list.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "accessibility_fix", verdict: Verdict::Deferred, requirement: "", why: "Accessibility authoring and review tools are outside the v1 list.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "accessibility_move_tag", verdict: Verdict::Deferred, requirement: "", why: "Accessibility authoring and review tools are outside the v1 list.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "accessibility_report", verdict: Verdict::Deferred, requirement: "", why: "Accessibility authoring and review tools are outside the v1 list.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "accessibility_review_html", verdict: Verdict::Deferred, requirement: "", why: "Accessibility authoring and review tools are outside the v1 list.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "accessibility_review_load", verdict: Verdict::Deferred, requirement: "", why: "Accessibility authoring and review tools are outside the v1 list.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "accessibility_review_record", verdict: Verdict::Deferred, requirement: "", why: "Accessibility authoring and review tools are outside the v1 list.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "accessibility_review_save", verdict: Verdict::Deferred, requirement: "", why: "Accessibility authoring and review tools are outside the v1 list.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "accessibility_set_alt", verdict: Verdict::Deferred, requirement: "", why: "Accessibility authoring and review tools are outside the v1 list.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "accessibility_set_tag_property", verdict: Verdict::Deferred, requirement: "", why: "Accessibility authoring and review tools are outside the v1 list.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "accessibility_tags", verdict: Verdict::Deferred, requirement: "", why: "Accessibility authoring and review tools are outside the v1 list.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "action_list", verdict: Verdict::Deferred, requirement: "", why: "Batch processing and action wizards are deferred; they run deferred operations.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "action_run", verdict: Verdict::Deferred, requirement: "", why: "Batch processing and action wizards are deferred; they run deferred operations.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "bookmark_add", verdict: Verdict::Deferred, requirement: "", why: "Bookmark and link authoring is deferred; v1 only views them.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "bookmark_delete", verdict: Verdict::Deferred, requirement: "", why: "Bookmark and link authoring is deferred; v1 only views them.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "bookmark_from_structure", verdict: Verdict::Deferred, requirement: "", why: "Bookmark and link authoring is deferred; v1 only views them.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "bookmark_list", verdict: Verdict::Core, requirement: "CORE-01", why: "Open and view.", params: &["doc", "limit", "offset"], denied_params: &[], enums: &[] },
    ToolCap { name: "bookmark_move", verdict: Verdict::Deferred, requirement: "", why: "Bookmark and link authoring is deferred; v1 only views them.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "bookmark_rename", verdict: Verdict::Deferred, requirement: "", why: "Bookmark and link authoring is deferred; v1 only views them.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "bookmark_set_page", verdict: Verdict::Deferred, requirement: "", why: "Bookmark and link authoring is deferred; v1 only views them.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "command_batch", verdict: Verdict::Gated, requirement: "CORE-10", why: "Runs several registry commands: allowed only if every step names a core command.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "command_list", verdict: Verdict::Core, requirement: "CORE-10", why: "Lists the commands and their mapped tools; read-only.", params: &["doc", "enabled_only", "filter"], denied_params: &[], enums: &[] },
    ToolCap { name: "command_run", verdict: Verdict::Gated, requirement: "CORE-10", why: "Runs any registry command by id: allowed only for a command that is itself core.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "comment_add", verdict: Verdict::Core, requirement: "CORE-03", why: "Annotate (the v1 kinds only; see the excluded modes).", params: &["all", "at", "author", "color", "contents", "doc", "dynamic", "endings", "fill", "find", "font_size", "from", "icon", "knee", "opacity", "page", "path", "points", "quads", "rect", "stamp", "strokes", "to", "type", "width"], denied_params: &[], enums: &[EnumRule { param: "endings", default: None, core: &["None", "OpenArrow", "ClosedArrow"], excluded: &["Square", "Circle", "Diamond", "Butt", "ROpenArrow", "RClosedArrow", "Slash"] }, EnumRule { param: "type", default: None, core: &["note", "highlight", "underline", "strikeout", "rectangle", "arrow", "ink", "textbox"], excluded: &["squiggly", "replace", "oval", "line", "stamp", "polygon", "cloud", "polyline", "callout", "caret", "attachment"] }] },
    ToolCap { name: "comment_delete", verdict: Verdict::Core, requirement: "CORE-03", why: "Annotate (the v1 kinds only; see the excluded modes).", params: &["doc", "id", "index", "page"], denied_params: &[], enums: &[] },
    ToolCap { name: "comment_edit", verdict: Verdict::Core, requirement: "CORE-03", why: "Annotate (the v1 kinds only; see the excluded modes).", params: &["color", "contents", "doc", "fill", "id", "index", "move", "opacity", "page", "rect", "width"], denied_params: &[], enums: &[] },
    ToolCap { name: "comment_image_preview", verdict: Verdict::Deferred, requirement: "", why: "Comment tools beyond add, edit, delete, reply and status are deferred.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "comment_list", verdict: Verdict::Core, requirement: "CORE-03", why: "Annotate (the v1 kinds only; see the excluded modes).", params: &["doc", "page"], denied_params: &[], enums: &[] },
    ToolCap { name: "comment_lock", verdict: Verdict::Deferred, requirement: "", why: "Comment tools beyond add, edit, delete, reply and status are deferred.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "comment_mark", verdict: Verdict::Deferred, requirement: "", why: "Comment tools beyond add, edit, delete, reply and status are deferred.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "comment_reply", verdict: Verdict::Core, requirement: "CORE-03", why: "Annotate (the v1 kinds only; see the excluded modes).", params: &["author", "doc", "id", "index", "page", "text"], denied_params: &[], enums: &[] },
    ToolCap { name: "comment_set_status", verdict: Verdict::Core, requirement: "CORE-03", why: "Annotate (the v1 kinds only; see the excluded modes).", params: &["author", "doc", "id", "index", "page", "status"], denied_params: &[], enums: &[EnumRule { param: "status", default: None, core: &["none", "accepted", "rejected", "cancelled", "completed"], excluded: &[] }] },
    ToolCap { name: "comments_hide", verdict: Verdict::Deferred, requirement: "", why: "Comment tools beyond add, edit, delete, reply and status are deferred.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "comments_summarize", verdict: Verdict::Deferred, requirement: "", why: "AI summarization is deferred from v1 (and needs a provider, so the network).", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "content_delete", verdict: Verdict::Deferred, requirement: "", why: "Rewriting existing text or images is deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "content_list", verdict: Verdict::Deferred, requirement: "", why: "Rewriting existing text or images is deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "content_update", verdict: Verdict::Deferred, requirement: "", why: "Rewriting existing text or images is deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "doc_audit_redactions", verdict: Verdict::Deferred, requirement: "", why: "Permanent redaction and sanitization are deferred from v1 (never an opaque rectangle).", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "doc_audit_space", verdict: Verdict::Deferred, requirement: "", why: "Aggressive optimization is deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "doc_background", verdict: Verdict::Deferred, requirement: "", why: "Page assembly beyond the v1 operations, and page marks, are deferred.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "doc_batch", verdict: Verdict::Deferred, requirement: "", why: "Batch processing and action wizards are deferred; they run deferred operations.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "doc_close", verdict: Verdict::Core, requirement: "CORE-01", why: "Open and view.", params: &["discard_changes", "doc"], denied_params: &[], enums: &[] },
    ToolCap { name: "doc_combine", verdict: Verdict::Core, requirement: "CORE-05", why: "Organize pages (the v1 operations only).", params: &["field_conflicts", "groups", "open", "out", "pages", "passwords", "paths"], denied_params: &[], enums: &[EnumRule { param: "field_conflicts", default: Some(crate::defaults::COMBINE_FIELD_CONFLICTS), core: &["rename"], excluded: &["merge"] }] },
    ToolCap { name: "doc_compare", verdict: Verdict::Deferred, requirement: "", why: "Comparison and overlays are deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "doc_compare_mark", verdict: Verdict::Deferred, requirement: "", why: "Comparison and overlays are deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "doc_compare_report", verdict: Verdict::Deferred, requirement: "", why: "Comparison and overlays are deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "doc_create", verdict: Verdict::Deferred, requirement: "", why: "Metadata editing, document creation and view settings are outside the v1 list.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "doc_create_multiple", verdict: Verdict::Deferred, requirement: "", why: "Metadata editing, document creation and view settings are outside the v1 list.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "doc_export_all_images", verdict: Verdict::Deferred, requirement: "", why: "Office and other export formats are deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "doc_export_data", verdict: Verdict::Deferred, requirement: "", why: "Form authoring, data exchange and field reset are outside the v1 fill-only scope.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "doc_export_images", verdict: Verdict::Deferred, requirement: "", why: "Office and other export formats are deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "doc_export_office", verdict: Verdict::Deferred, requirement: "", why: "Office and other export formats are deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "doc_export_text", verdict: Verdict::Deferred, requirement: "", why: "Office and other export formats are deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "doc_flatten", verdict: Verdict::Deferred, requirement: "", why: "Page assembly beyond the v1 operations, and page marks, are deferred.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "doc_header_footer", verdict: Verdict::Deferred, requirement: "", why: "Page assembly beyond the v1 operations, and page marks, are deferred.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "doc_hidden_info", verdict: Verdict::Deferred, requirement: "", why: "Permanent redaction and sanitization are deferred from v1 (never an opaque rectangle).", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "doc_import_data", verdict: Verdict::Deferred, requirement: "", why: "Form authoring, data exchange and field reset are outside the v1 fill-only scope.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "doc_info", verdict: Verdict::Core, requirement: "CORE-01", why: "Open and view.", params: &["doc"], denied_params: &[], enums: &[] },
    ToolCap { name: "doc_initial_view", verdict: Verdict::Deferred, requirement: "", why: "Metadata editing, document creation and view settings are outside the v1 list.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "doc_inspect", verdict: Verdict::Core, requirement: "CORE-01", why: "Open and view.", params: &["doc"], denied_params: &[], enums: &[] },
    ToolCap { name: "doc_list", verdict: Verdict::Core, requirement: "CORE-01", why: "Open and view.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "doc_open", verdict: Verdict::Core, requirement: "CORE-01", why: "Open and view.", params: &["password", "path"], denied_params: &[], enums: &[] },
    ToolCap { name: "doc_open_revision", verdict: Verdict::Deferred, requirement: "", why: "Revision access arrives with the shared service and revision contracts (BUILD-007).", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "doc_optimize", verdict: Verdict::Deferred, requirement: "", why: "Aggressive optimization is deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "doc_print", verdict: Verdict::Core, requirement: "CORE-07", why: "Print.", params: &["auto_rotate", "binding", "booklet_subset", "border", "collate", "comments_forms", "copies", "cut_marks", "doc", "duplex", "grayscale", "layout", "options", "order", "orientation", "overlap", "pages", "paper", "path", "per_sheet", "printer", "reverse", "scale", "subset"], denied_params: &[], enums: &[EnumRule { param: "binding", default: None, core: &[], excluded: &["left", "right"] }, EnumRule { param: "booklet_subset", default: None, core: &[], excluded: &["both", "front", "back"] }, EnumRule { param: "comments_forms", default: Some(crate::defaults::PRINT_COMMENTS_FORMS), core: &["document", "document-and-markups"], excluded: &["document-and-stamps", "form-fields-only"] }, EnumRule { param: "duplex", default: Some(crate::defaults::PRINT_DUPLEX), core: &["off"], excluded: &["long-edge", "short-edge"] }, EnumRule { param: "layout", default: Some(crate::defaults::PRINT_LAYOUT), core: &["fit", "actual"], excluded: &["shrink", "custom", "multiple", "booklet", "poster"] }, EnumRule { param: "order", default: None, core: &[], excluded: &["horizontal", "horizontal-reversed", "vertical", "vertical-reversed", "cut-stack"] }, EnumRule { param: "orientation", default: Some(crate::defaults::PRINT_ORIENTATION), core: &["auto", "portrait", "landscape"], excluded: &[] }, EnumRule { param: "subset", default: Some(crate::defaults::PRINT_SUBSET), core: &["all", "odd", "even"], excluded: &[] }] },
    ToolCap { name: "doc_protect", verdict: Verdict::Deferred, requirement: "", why: "Adding encryption or removing protection is a separate qualified feature.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "doc_reduce", verdict: Verdict::Deferred, requirement: "", why: "Aggressive optimization is deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "doc_remove_hidden", verdict: Verdict::Deferred, requirement: "", why: "Permanent redaction and sanitization are deferred from v1 (never an opaque rectangle).", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "doc_remove_marks", verdict: Verdict::Deferred, requirement: "", why: "Page assembly beyond the v1 operations, and page marks, are deferred.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "doc_revisions", verdict: Verdict::Deferred, requirement: "", why: "Revision access arrives with the shared service and revision contracts (BUILD-007).", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "doc_save", verdict: Verdict::Core, requirement: "CORE-06", why: "Save safely.", params: &["doc", "full", "overwrite_version", "path"], denied_params: &["flatten_fill_sign"], enums: &[] },
    ToolCap { name: "doc_set_info", verdict: Verdict::Deferred, requirement: "", why: "Metadata editing, document creation and view settings are outside the v1 list.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "doc_split", verdict: Verdict::Core, requirement: "CORE-05", why: "Organize pages (the v1 operations only).", params: &["before", "doc", "every", "out_dir"], denied_params: &["bookmarks", "max_mb"], enums: &[] },
    ToolCap { name: "doc_unprotect", verdict: Verdict::Deferred, requirement: "", why: "Adding encryption or removing protection is a separate qualified feature.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "doc_watermark", verdict: Verdict::Deferred, requirement: "", why: "Page assembly beyond the v1 operations, and page marks, are deferred.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "edit_redo", verdict: Verdict::Core, requirement: "CORE-08", why: "Undo and redo.", params: &["doc"], denied_params: &[], enums: &[] },
    ToolCap { name: "edit_undo", verdict: Verdict::Core, requirement: "CORE-08", why: "Undo and redo.", params: &["doc"], denied_params: &[], enums: &[] },
    ToolCap { name: "fill_sign_add", verdict: Verdict::Deferred, requirement: "", why: "A simple signature appearance is CORE-09 (v1 should, BUILD-016), not yet qualified.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "fill_sign_date_format", verdict: Verdict::Deferred, requirement: "", why: "A simple signature appearance is CORE-09 (v1 should, BUILD-016), not yet qualified.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "form_actions", verdict: Verdict::Deferred, requirement: "", why: "Form authoring, data exchange and field reset are outside the v1 fill-only scope.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "form_add_field", verdict: Verdict::Deferred, requirement: "", why: "Form authoring, data exchange and field reset are outside the v1 fill-only scope.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "form_delete_field", verdict: Verdict::Deferred, requirement: "", why: "Form authoring, data exchange and field reset are outside the v1 fill-only scope.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "form_detect_fields", verdict: Verdict::Deferred, requirement: "", why: "Form authoring, data exchange and field reset are outside the v1 fill-only scope.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "form_fields", verdict: Verdict::Core, requirement: "CORE-04", why: "Fill existing AcroForm fields.", params: &["doc"], denied_params: &[], enums: &[] },
    ToolCap { name: "form_fill", verdict: Verdict::Core, requirement: "CORE-04", why: "Fill existing AcroForm fields.", params: &["doc", "values"], denied_params: &[], enums: &[] },
    ToolCap { name: "form_merge_data", verdict: Verdict::Deferred, requirement: "", why: "Form authoring, data exchange and field reset are outside the v1 fill-only scope.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "form_reset", verdict: Verdict::Deferred, requirement: "", why: "Form authoring, data exchange and field reset are outside the v1 fill-only scope.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "form_set_actions", verdict: Verdict::Deferred, requirement: "", why: "Form authoring, data exchange and field reset are outside the v1 fill-only scope.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "form_set_image", verdict: Verdict::Deferred, requirement: "", why: "Form authoring, data exchange and field reset are outside the v1 fill-only scope.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "form_set_props", verdict: Verdict::Deferred, requirement: "", why: "Form authoring, data exchange and field reset are outside the v1 fill-only scope.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "form_set_script", verdict: Verdict::Excluded, requirement: "", why: "Document scripts and the JavaScript console are not supported in v1; they must be identified, not run.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "form_tab_order", verdict: Verdict::Deferred, requirement: "", why: "Form authoring, data exchange and field reset are outside the v1 fill-only scope.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "image_edit", verdict: Verdict::Deferred, requirement: "", why: "Rewriting existing text or images is deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "image_hit", verdict: Verdict::Deferred, requirement: "", why: "Rewriting existing text or images is deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "image_save", verdict: Verdict::Deferred, requirement: "", why: "Rewriting existing text or images is deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "js_document_scripts", verdict: Verdict::Excluded, requirement: "", why: "Document scripts and the JavaScript console are not supported in v1; they must be identified, not run.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "js_enabled", verdict: Verdict::Excluded, requirement: "", why: "Document scripts and the JavaScript console are not supported in v1; they must be identified, not run.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "js_run", verdict: Verdict::Excluded, requirement: "", why: "Document scripts and the JavaScript console are not supported in v1; they must be identified, not run.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "js_set_document_script", verdict: Verdict::Excluded, requirement: "", why: "Document scripts and the JavaScript console are not supported in v1; they must be identified, not run.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "link_add", verdict: Verdict::Deferred, requirement: "", why: "Bookmark and link authoring is deferred; v1 only views them.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "link_delete", verdict: Verdict::Deferred, requirement: "", why: "Bookmark and link authoring is deferred; v1 only views them.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "link_edit", verdict: Verdict::Deferred, requirement: "", why: "Bookmark and link authoring is deferred; v1 only views them.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "link_list", verdict: Verdict::Core, requirement: "CORE-01", why: "Open and view.", params: &["doc"], denied_params: &[], enums: &[] },
    ToolCap { name: "link_open", verdict: Verdict::Excluded, requirement: "", why: "Following a link can launch an external target (a program, a URL or another file).", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "links_from_urls", verdict: Verdict::Deferred, requirement: "", why: "Bookmark and link authoring is deferred; v1 only views them.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "links_remove", verdict: Verdict::Deferred, requirement: "", why: "Bookmark and link authoring is deferred; v1 only views them.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "measure_area", verdict: Verdict::Deferred, requirement: "", why: "Measurement and takeoff are deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "measure_distance", verdict: Verdict::Deferred, requirement: "", why: "Measurement and takeoff are deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "measure_export", verdict: Verdict::Deferred, requirement: "", why: "Measurement and takeoff are deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "measure_info", verdict: Verdict::Deferred, requirement: "", why: "Measurement and takeoff are deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "measure_list", verdict: Verdict::Deferred, requirement: "", why: "Measurement and takeoff are deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "measure_perimeter", verdict: Verdict::Deferred, requirement: "", why: "Measurement and takeoff are deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "measure_scale", verdict: Verdict::Deferred, requirement: "", why: "Measurement and takeoff are deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "measure_snap", verdict: Verdict::Deferred, requirement: "", why: "Measurement and takeoff are deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "object_list", verdict: Verdict::Deferred, requirement: "", why: "Rewriting existing text or images is deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "object_move", verdict: Verdict::Deferred, requirement: "", why: "Rewriting existing text or images is deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "ocr_recognize", verdict: Verdict::Deferred, requirement: "", why: "OCR is deferred from v1 (needs its own safety and fidelity gate).", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "ocr_recognize_files", verdict: Verdict::Deferred, requirement: "", why: "OCR is deferred from v1 (needs its own safety and fidelity gate).", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "ocr_status", verdict: Verdict::Deferred, requirement: "", why: "OCR is deferred from v1 (needs its own safety and fidelity gate).", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "page_add_image", verdict: Verdict::Deferred, requirement: "", why: "Rewriting existing text or images is deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "page_add_text", verdict: Verdict::Deferred, requirement: "", why: "Rewriting existing text or images is deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "page_delete", verdict: Verdict::Core, requirement: "CORE-05", why: "Organize pages (the v1 operations only).", params: &["doc", "pages"], denied_params: &[], enums: &[] },
    ToolCap { name: "page_duplicate", verdict: Verdict::Deferred, requirement: "", why: "Page assembly beyond the v1 operations, and page marks, are deferred.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "page_enhance_camera", verdict: Verdict::Deferred, requirement: "", why: "OCR is deferred from v1 (needs its own safety and fidelity gate).", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "page_extract", verdict: Verdict::Core, requirement: "CORE-05", why: "Organize pages (the v1 operations only).", params: &["doc", "open", "out", "out_dir", "pages", "separate"], denied_params: &["delete"], enums: &[] },
    ToolCap { name: "page_images", verdict: Verdict::Deferred, requirement: "", why: "Rewriting existing text or images is deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "page_insert_blank", verdict: Verdict::Core, requirement: "CORE-05", why: "Organize pages (the v1 operations only).", params: &["at", "doc", "height", "width"], denied_params: &[], enums: &[] },
    ToolCap { name: "page_insert_file", verdict: Verdict::Core, requirement: "CORE-05", why: "Organize pages (the v1 operations only).", params: &["at", "doc", "pages", "path"], denied_params: &[], enums: &[] },
    ToolCap { name: "page_move", verdict: Verdict::Core, requirement: "CORE-05", why: "Organize pages (the v1 operations only).", params: &["doc", "pages", "to"], denied_params: &[], enums: &[] },
    ToolCap { name: "page_number", verdict: Verdict::Deferred, requirement: "", why: "Page assembly beyond the v1 operations, and page marks, are deferred.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "page_render", verdict: Verdict::Core, requirement: "CORE-01", why: "Open and view.", params: &["doc", "dpi", "page"], denied_params: &["hide_text_block"], enums: &[] },
    ToolCap { name: "page_replace", verdict: Verdict::Deferred, requirement: "", why: "Page assembly beyond the v1 operations, and page marks, are deferred.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "page_rotate", verdict: Verdict::Core, requirement: "CORE-05", why: "Organize pages (the v1 operations only).", params: &["degrees", "doc", "orientation", "pages", "subset"], denied_params: &[], enums: &[EnumRule { param: "orientation", default: Some(crate::defaults::ROTATE_ORIENTATION), core: &["all"], excluded: &["landscape", "portrait"] }, EnumRule { param: "subset", default: Some(crate::defaults::ROTATE_SUBSET), core: &["all"], excluded: &["even", "odd"] }] },
    ToolCap { name: "page_set_box", verdict: Verdict::Deferred, requirement: "", why: "Page assembly beyond the v1 operations, and page marks, are deferred.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "pdfa_convert", verdict: Verdict::Deferred, requirement: "", why: "PDF/A, preflight and print readiness claims are deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "pdfa_verify", verdict: Verdict::Deferred, requirement: "", why: "PDF/A, preflight and print readiness claims are deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "print_readiness", verdict: Verdict::Deferred, requirement: "", why: "PDF/A, preflight and print readiness claims are deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "printer_options", verdict: Verdict::Core, requirement: "CORE-07", why: "Print.", params: &["printer"], denied_params: &[], enums: &[] },
    ToolCap { name: "printers", verdict: Verdict::Core, requirement: "CORE-07", why: "Print.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "redact_apply", verdict: Verdict::Deferred, requirement: "", why: "Permanent redaction and sanitization are deferred from v1 (never an opaque rectangle).", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "redact_clear", verdict: Verdict::Deferred, requirement: "", why: "Permanent redaction and sanitization are deferred from v1 (never an opaque rectangle).", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "redact_mark", verdict: Verdict::Deferred, requirement: "", why: "Permanent redaction and sanitization are deferred from v1 (never an opaque rectangle).", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "render_preview", verdict: Verdict::Core, requirement: "CORE-01", why: "Open and view.", params: &["doc", "max_side", "page"], denied_params: &[], enums: &[] },
    ToolCap { name: "sign_document", verdict: Verdict::Deferred, requirement: "", why: "Certificate-based signing and signature validation claims are deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "sign_id_create", verdict: Verdict::Deferred, requirement: "", why: "Certificate-based signing and signature validation claims are deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "sign_keychain_ids", verdict: Verdict::Deferred, requirement: "", why: "Certificate-based signing and signature validation claims are deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "sign_list", verdict: Verdict::Deferred, requirement: "", why: "Certificate-based signing and signature validation claims are deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "sign_trust", verdict: Verdict::Deferred, requirement: "", why: "Certificate-based signing and signature validation claims are deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "sign_windows_ids", verdict: Verdict::Deferred, requirement: "", why: "Certificate-based signing and signature validation claims are deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "stamp_custom", verdict: Verdict::Deferred, requirement: "", why: "A simple signature appearance is CORE-09 (v1 should, BUILD-016), not yet qualified.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "text_edit", verdict: Verdict::Deferred, requirement: "", why: "Rewriting existing text or images is deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "text_edit_layout", verdict: Verdict::Deferred, requirement: "", why: "Rewriting existing text or images is deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "text_extract", verdict: Verdict::Core, requirement: "CORE-02", why: "Find and select.", params: &["doc", "pages", "rect"], denied_params: &[], enums: &[] },
    ToolCap { name: "text_find", verdict: Verdict::Core, requirement: "CORE-02", why: "Find and select.", params: &["doc", "limit", "query", "skip", "start_page"], denied_params: &[], enums: &[] },
    ToolCap { name: "text_font_files", verdict: Verdict::Deferred, requirement: "", why: "Rewriting existing text or images is deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "text_font_import", verdict: Verdict::Deferred, requirement: "", why: "Rewriting existing text or images is deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "text_fonts", verdict: Verdict::Deferred, requirement: "", why: "Rewriting existing text or images is deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "text_lines", verdict: Verdict::Deferred, requirement: "", why: "Rewriting existing text or images is deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "text_paragraphs", verdict: Verdict::Deferred, requirement: "", why: "Rewriting existing text or images is deferred from v1.", params: &[], denied_params: &[], enums: &[] },
    ToolCap { name: "view_reading_colors", verdict: Verdict::Deferred, requirement: "", why: "Metadata editing, document creation and view settings are outside the v1 list.", params: &[], denied_params: &[], enums: &[] },
];

#[rustfmt::skip]
pub static COMMANDS: &[CommandCap] = &[
    CommandCap { id: "a11y.alt_text", verdict: Verdict::Deferred, requirement: "", why: "Accessibility authoring is outside the v1 list." },
    CommandCap { id: "a11y.autotag", verdict: Verdict::Deferred, requirement: "", why: "Accessibility authoring is outside the v1 list." },
    CommandCap { id: "a11y.check", verdict: Verdict::Deferred, requirement: "", why: "Accessibility authoring is outside the v1 list." },
    CommandCap { id: "a11y.reading_options", verdict: Verdict::Deferred, requirement: "", why: "Accessibility authoring is outside the v1 list." },
    CommandCap { id: "a11y.reading_order", verdict: Verdict::Deferred, requirement: "", why: "Accessibility authoring is outside the v1 list." },
    CommandCap { id: "a11y.report", verdict: Verdict::Deferred, requirement: "", why: "Accessibility authoring is outside the v1 list." },
    CommandCap { id: "a11y.tags", verdict: Verdict::Deferred, requirement: "", why: "Accessibility authoring is outside the v1 list." },
    CommandCap { id: "actions.archive", verdict: Verdict::Deferred, requirement: "", why: "Action wizards run deferred operations." },
    CommandCap { id: "actions.distribution", verdict: Verdict::Deferred, requirement: "", why: "Action wizards run deferred operations." },
    CommandCap { id: "actions.make_accessible", verdict: Verdict::Deferred, requirement: "", why: "Action wizards run deferred operations." },
    CommandCap { id: "actions.optimize_scans", verdict: Verdict::Deferred, requirement: "", why: "Action wizards run deferred operations." },
    CommandCap { id: "actions.wizard", verdict: Verdict::Deferred, requirement: "", why: "Action wizards run deferred operations." },
    CommandCap { id: "ai.ask", verdict: Verdict::Excluded, requirement: "", why: "Needs a cloud AI provider; the network stays off by default." },
    CommandCap { id: "ai.summary", verdict: Verdict::Excluded, requirement: "", why: "Needs a cloud AI provider; the network stays off by default." },
    CommandCap { id: "ai.translate", verdict: Verdict::Excluded, requirement: "", why: "Needs a cloud AI provider; the network stays off by default." },
    CommandCap { id: "app.preferences", verdict: Verdict::Core, requirement: "CORE-01", why: "Open and view." },
    CommandCap { id: "bookmark.add", verdict: Verdict::Deferred, requirement: "", why: "Bookmark authoring is deferred; v1 views bookmarks." },
    CommandCap { id: "bookmark.from_structure", verdict: Verdict::Deferred, requirement: "", why: "Bookmark authoring is deferred; v1 views bookmarks." },
    CommandCap { id: "comment.arrow", verdict: Verdict::Core, requirement: "CORE-03", why: "Annotate (the v1 kinds only; see the excluded modes)." },
    CommandCap { id: "comment.attach", verdict: Verdict::Deferred, requirement: "", why: "Annotation kinds and tools beyond the v1 set are deferred." },
    CommandCap { id: "comment.callout", verdict: Verdict::Deferred, requirement: "", why: "Annotation kinds and tools beyond the v1 set are deferred." },
    CommandCap { id: "comment.caret", verdict: Verdict::Deferred, requirement: "", why: "Annotation kinds and tools beyond the v1 set are deferred." },
    CommandCap { id: "comment.circle", verdict: Verdict::Deferred, requirement: "", why: "Annotation kinds and tools beyond the v1 set are deferred." },
    CommandCap { id: "comment.cloud", verdict: Verdict::Deferred, requirement: "", why: "Annotation kinds and tools beyond the v1 set are deferred." },
    CommandCap { id: "comment.eraser", verdict: Verdict::Deferred, requirement: "", why: "Annotation kinds and tools beyond the v1 set are deferred." },
    CommandCap { id: "comment.export", verdict: Verdict::Deferred, requirement: "", why: "Annotation kinds and tools beyond the v1 set are deferred." },
    CommandCap { id: "comment.flatten", verdict: Verdict::Deferred, requirement: "", why: "Annotation kinds and tools beyond the v1 set are deferred." },
    CommandCap { id: "comment.freetext", verdict: Verdict::Core, requirement: "CORE-03", why: "Annotate (the v1 kinds only; see the excluded modes)." },
    CommandCap { id: "comment.hide_all", verdict: Verdict::Deferred, requirement: "", why: "Annotation kinds and tools beyond the v1 set are deferred." },
    CommandCap { id: "comment.highlight", verdict: Verdict::Core, requirement: "CORE-03", why: "Annotate (the v1 kinds only; see the excluded modes)." },
    CommandCap { id: "comment.import", verdict: Verdict::Deferred, requirement: "", why: "Annotation kinds and tools beyond the v1 set are deferred." },
    CommandCap { id: "comment.ink", verdict: Verdict::Core, requirement: "CORE-03", why: "Annotate (the v1 kinds only; see the excluded modes)." },
    CommandCap { id: "comment.line", verdict: Verdict::Deferred, requirement: "", why: "Annotation kinds and tools beyond the v1 set are deferred." },
    CommandCap { id: "comment.list", verdict: Verdict::Core, requirement: "CORE-03", why: "Annotate (the v1 kinds only; see the excluded modes)." },
    CommandCap { id: "comment.note", verdict: Verdict::Core, requirement: "CORE-03", why: "Annotate (the v1 kinds only; see the excluded modes)." },
    CommandCap { id: "comment.polygon", verdict: Verdict::Deferred, requirement: "", why: "Annotation kinds and tools beyond the v1 set are deferred." },
    CommandCap { id: "comment.polyline", verdict: Verdict::Deferred, requirement: "", why: "Annotation kinds and tools beyond the v1 set are deferred." },
    CommandCap { id: "comment.replace", verdict: Verdict::Deferred, requirement: "", why: "Annotation kinds and tools beyond the v1 set are deferred." },
    CommandCap { id: "comment.square", verdict: Verdict::Core, requirement: "CORE-03", why: "Annotate (the v1 kinds only; see the excluded modes)." },
    CommandCap { id: "comment.squiggly", verdict: Verdict::Deferred, requirement: "", why: "Annotation kinds and tools beyond the v1 set are deferred." },
    CommandCap { id: "comment.stamp", verdict: Verdict::Deferred, requirement: "", why: "Annotation kinds and tools beyond the v1 set are deferred." },
    CommandCap { id: "comment.strikeout", verdict: Verdict::Core, requirement: "CORE-03", why: "Annotate (the v1 kinds only; see the excluded modes)." },
    CommandCap { id: "comment.summarize", verdict: Verdict::Deferred, requirement: "", why: "Annotation kinds and tools beyond the v1 set are deferred." },
    CommandCap { id: "comment.underline", verdict: Verdict::Core, requirement: "CORE-03", why: "Annotate (the v1 kinds only; see the excluded modes)." },
    CommandCap { id: "create.blank", verdict: Verdict::Deferred, requirement: "", why: "Creating PDFs from other files is outside the v1 list." },
    CommandCap { id: "create.clipboard", verdict: Verdict::Deferred, requirement: "", why: "Creating PDFs from other files is outside the v1 list." },
    CommandCap { id: "create.file", verdict: Verdict::Deferred, requirement: "", why: "Creating PDFs from other files is outside the v1 list." },
    CommandCap { id: "create.images", verdict: Verdict::Deferred, requirement: "", why: "Creating PDFs from other files is outside the v1 list." },
    CommandCap { id: "create.multiple", verdict: Verdict::Deferred, requirement: "", why: "Creating PDFs from other files is outside the v1 list." },
    CommandCap { id: "create.multiple_separate", verdict: Verdict::Deferred, requirement: "", why: "Creating PDFs from other files is outside the v1 list." },
    CommandCap { id: "doc.compare", verdict: Verdict::Deferred, requirement: "", why: "Comparison is deferred from v1." },
    CommandCap { id: "edit.advanced_search", verdict: Verdict::Deferred, requirement: "", why: "Editing existing content and page marks is deferred from v1." },
    CommandCap { id: "edit.attach", verdict: Verdict::Deferred, requirement: "", why: "Editing existing content and page marks is deferred from v1." },
    CommandCap { id: "edit.background", verdict: Verdict::Deferred, requirement: "", why: "Editing existing content and page marks is deferred from v1." },
    CommandCap { id: "edit.background.remove", verdict: Verdict::Deferred, requirement: "", why: "Editing existing content and page marks is deferred from v1." },
    CommandCap { id: "edit.background.update", verdict: Verdict::Deferred, requirement: "", why: "Editing existing content and page marks is deferred from v1." },
    CommandCap { id: "edit.bates", verdict: Verdict::Deferred, requirement: "", why: "Editing existing content and page marks is deferred from v1." },
    CommandCap { id: "edit.column_select", verdict: Verdict::Core, requirement: "CORE-02", why: "Find and select." },
    CommandCap { id: "edit.edit_text", verdict: Verdict::Deferred, requirement: "", why: "Editing existing content and page marks is deferred from v1." },
    CommandCap { id: "edit.find", verdict: Verdict::Core, requirement: "CORE-02", why: "Find and select." },
    CommandCap { id: "edit.header_footer", verdict: Verdict::Deferred, requirement: "", why: "Editing existing content and page marks is deferred from v1." },
    CommandCap { id: "edit.header_footer.remove", verdict: Verdict::Deferred, requirement: "", why: "Editing existing content and page marks is deferred from v1." },
    CommandCap { id: "edit.header_footer.update", verdict: Verdict::Deferred, requirement: "", why: "Editing existing content and page marks is deferred from v1." },
    CommandCap { id: "edit.image", verdict: Verdict::Deferred, requirement: "", why: "Editing existing content and page marks is deferred from v1." },
    CommandCap { id: "edit.link", verdict: Verdict::Deferred, requirement: "", why: "Editing existing content and page marks is deferred from v1." },
    CommandCap { id: "edit.links_from_urls", verdict: Verdict::Deferred, requirement: "", why: "Editing existing content and page marks is deferred from v1." },
    CommandCap { id: "edit.redo", verdict: Verdict::Core, requirement: "CORE-08", why: "Undo and redo." },
    CommandCap { id: "edit.remove_links", verdict: Verdict::Deferred, requirement: "", why: "Editing existing content and page marks is deferred from v1." },
    CommandCap { id: "edit.snapshot", verdict: Verdict::Deferred, requirement: "", why: "Editing existing content and page marks is deferred from v1." },
    CommandCap { id: "edit.text", verdict: Verdict::Deferred, requirement: "", why: "Editing existing content and page marks is deferred from v1." },
    CommandCap { id: "edit.undo", verdict: Verdict::Core, requirement: "CORE-08", why: "Undo and redo." },
    CommandCap { id: "edit.watermark", verdict: Verdict::Deferred, requirement: "", why: "Editing existing content and page marks is deferred from v1." },
    CommandCap { id: "edit.watermark.remove", verdict: Verdict::Deferred, requirement: "", why: "Editing existing content and page marks is deferred from v1." },
    CommandCap { id: "edit.watermark.update", verdict: Verdict::Deferred, requirement: "", why: "Editing existing content and page marks is deferred from v1." },
    CommandCap { id: "export.all_images", verdict: Verdict::Deferred, requirement: "", why: "Export formats are deferred from v1." },
    CommandCap { id: "export.docx", verdict: Verdict::Deferred, requirement: "", why: "Export formats are deferred from v1." },
    CommandCap { id: "export.html", verdict: Verdict::Deferred, requirement: "", why: "Export formats are deferred from v1." },
    CommandCap { id: "export.image", verdict: Verdict::Deferred, requirement: "", why: "Export formats are deferred from v1." },
    CommandCap { id: "export.pptx", verdict: Verdict::Deferred, requirement: "", why: "Export formats are deferred from v1." },
    CommandCap { id: "export.ps", verdict: Verdict::Deferred, requirement: "", why: "Export formats are deferred from v1." },
    CommandCap { id: "export.rtf", verdict: Verdict::Deferred, requirement: "", why: "Export formats are deferred from v1." },
    CommandCap { id: "export.text", verdict: Verdict::Deferred, requirement: "", why: "Export formats are deferred from v1." },
    CommandCap { id: "export.xlsx", verdict: Verdict::Deferred, requirement: "", why: "Export formats are deferred from v1." },
    CommandCap { id: "file.clear_recent", verdict: Verdict::Core, requirement: "CORE-01", why: "Open and view." },
    CommandCap { id: "file.close", verdict: Verdict::Core, requirement: "CORE-01", why: "Open and view." },
    CommandCap { id: "file.close_all", verdict: Verdict::Core, requirement: "CORE-01", why: "Open and view." },
    CommandCap { id: "file.open", verdict: Verdict::Core, requirement: "CORE-01", why: "Open and view." },
    CommandCap { id: "file.open_recent", verdict: Verdict::Core, requirement: "CORE-01", why: "Open and view." },
    CommandCap { id: "file.pin_folder", verdict: Verdict::Deferred, requirement: "", why: "Home-screen conveniences outside the v1 list." },
    CommandCap { id: "file.properties", verdict: Verdict::Core, requirement: "CORE-01", why: "Open and view." },
    CommandCap { id: "file.revert", verdict: Verdict::Core, requirement: "CORE-06", why: "Save safely." },
    CommandCap { id: "file.save", verdict: Verdict::Core, requirement: "CORE-06", why: "Save safely." },
    CommandCap { id: "file.save_as", verdict: Verdict::Core, requirement: "CORE-06", why: "Save safely." },
    CommandCap { id: "form.add.button", verdict: Verdict::Deferred, requirement: "", why: "Form authoring, data exchange and flattening are outside the v1 fill-only scope." },
    CommandCap { id: "form.add.checkbox", verdict: Verdict::Deferred, requirement: "", why: "Form authoring, data exchange and flattening are outside the v1 fill-only scope." },
    CommandCap { id: "form.add.combo", verdict: Verdict::Deferred, requirement: "", why: "Form authoring, data exchange and flattening are outside the v1 fill-only scope." },
    CommandCap { id: "form.add.date", verdict: Verdict::Deferred, requirement: "", why: "Form authoring, data exchange and flattening are outside the v1 fill-only scope." },
    CommandCap { id: "form.add.image", verdict: Verdict::Deferred, requirement: "", why: "Form authoring, data exchange and flattening are outside the v1 fill-only scope." },
    CommandCap { id: "form.add.list", verdict: Verdict::Deferred, requirement: "", why: "Form authoring, data exchange and flattening are outside the v1 fill-only scope." },
    CommandCap { id: "form.add.radio", verdict: Verdict::Deferred, requirement: "", why: "Form authoring, data exchange and flattening are outside the v1 fill-only scope." },
    CommandCap { id: "form.add.signature", verdict: Verdict::Deferred, requirement: "", why: "Form authoring, data exchange and flattening are outside the v1 fill-only scope." },
    CommandCap { id: "form.add.text", verdict: Verdict::Deferred, requirement: "", why: "Form authoring, data exchange and flattening are outside the v1 fill-only scope." },
    CommandCap { id: "form.clear", verdict: Verdict::Deferred, requirement: "", why: "Form authoring, data exchange and flattening are outside the v1 fill-only scope." },
    CommandCap { id: "form.detect", verdict: Verdict::Deferred, requirement: "", why: "Form authoring, data exchange and flattening are outside the v1 fill-only scope." },
    CommandCap { id: "form.export_data", verdict: Verdict::Deferred, requirement: "", why: "Form authoring, data exchange and flattening are outside the v1 fill-only scope." },
    CommandCap { id: "form.field.properties", verdict: Verdict::Deferred, requirement: "", why: "Form authoring, data exchange and flattening are outside the v1 fill-only scope." },
    CommandCap { id: "form.fields", verdict: Verdict::Core, requirement: "CORE-04", why: "Fill existing AcroForm fields." },
    CommandCap { id: "form.flatten", verdict: Verdict::Deferred, requirement: "", why: "Form authoring, data exchange and flattening are outside the v1 fill-only scope." },
    CommandCap { id: "form.import_data", verdict: Verdict::Deferred, requirement: "", why: "Form authoring, data exchange and flattening are outside the v1 fill-only scope." },
    CommandCap { id: "form.merge_data", verdict: Verdict::Deferred, requirement: "", why: "Form authoring, data exchange and flattening are outside the v1 fill-only scope." },
    CommandCap { id: "form.prepare", verdict: Verdict::Deferred, requirement: "", why: "Form authoring, data exchange and flattening are outside the v1 fill-only scope." },
    CommandCap { id: "form.tab_order.column", verdict: Verdict::Deferred, requirement: "", why: "Form authoring, data exchange and flattening are outside the v1 fill-only scope." },
    CommandCap { id: "form.tab_order.row", verdict: Verdict::Deferred, requirement: "", why: "Form authoring, data exchange and flattening are outside the v1 fill-only scope." },
    CommandCap { id: "form.tab_order.structure", verdict: Verdict::Deferred, requirement: "", why: "Form authoring, data exchange and flattening are outside the v1 fill-only scope." },
    CommandCap { id: "help.about", verdict: Verdict::Core, requirement: "CORE-01", why: "Open and view." },
    CommandCap { id: "help.app_page", verdict: Verdict::Excluded, requirement: "", why: "Opens an external site or checks for updates; BDF has no support URL or update endpoint yet, and the network stays off by default." },
    CommandCap { id: "help.check_updates", verdict: Verdict::Excluded, requirement: "", why: "Opens an external site or checks for updates; BDF has no support URL or update endpoint yet, and the network stays off by default." },
    CommandCap { id: "help.discord", verdict: Verdict::Excluded, requirement: "", why: "Opens an external site or checks for updates; BDF has no support URL or update endpoint yet, and the network stays off by default." },
    CommandCap { id: "help.github", verdict: Verdict::Excluded, requirement: "", why: "Opens an external site or checks for updates; BDF has no support URL or update endpoint yet, and the network stays off by default." },
    CommandCap { id: "help.shortcuts", verdict: Verdict::Core, requirement: "CORE-01", why: "Open and view." },
    CommandCap { id: "help.website", verdict: Verdict::Excluded, requirement: "", why: "Opens an external site or checks for updates; BDF has no support URL or update endpoint yet, and the network stays off by default." },
    CommandCap { id: "js.actions", verdict: Verdict::Excluded, requirement: "", why: "Document scripts are not supported in v1." },
    CommandCap { id: "js.console", verdict: Verdict::Excluded, requirement: "", why: "Document scripts are not supported in v1." },
    CommandCap { id: "js.document", verdict: Verdict::Excluded, requirement: "", why: "Document scripts are not supported in v1." },
    CommandCap { id: "measure.area", verdict: Verdict::Deferred, requirement: "", why: "Measurement is deferred from v1." },
    CommandCap { id: "measure.distance", verdict: Verdict::Deferred, requirement: "", why: "Measurement is deferred from v1." },
    CommandCap { id: "measure.export", verdict: Verdict::Deferred, requirement: "", why: "Measurement is deferred from v1." },
    CommandCap { id: "measure.geo", verdict: Verdict::Deferred, requirement: "", why: "Measurement is deferred from v1." },
    CommandCap { id: "measure.info", verdict: Verdict::Deferred, requirement: "", why: "Measurement is deferred from v1." },
    CommandCap { id: "measure.perimeter", verdict: Verdict::Deferred, requirement: "", why: "Measurement is deferred from v1." },
    CommandCap { id: "measure.scale", verdict: Verdict::Deferred, requirement: "", why: "Measurement is deferred from v1." },
    CommandCap { id: "measure.snap", verdict: Verdict::Deferred, requirement: "", why: "Measurement is deferred from v1." },
    CommandCap { id: "ocr.correct", verdict: Verdict::Deferred, requirement: "", why: "OCR is deferred from v1." },
    CommandCap { id: "ocr.enhance", verdict: Verdict::Deferred, requirement: "", why: "OCR is deferred from v1." },
    CommandCap { id: "ocr.enhance_camera", verdict: Verdict::Deferred, requirement: "", why: "OCR is deferred from v1." },
    CommandCap { id: "ocr.recognize", verdict: Verdict::Deferred, requirement: "", why: "OCR is deferred from v1." },
    CommandCap { id: "ocr.recognize_batch", verdict: Verdict::Deferred, requirement: "", why: "OCR is deferred from v1." },
    CommandCap { id: "optimize.advanced", verdict: Verdict::Deferred, requirement: "", why: "Optimization is deferred from v1." },
    CommandCap { id: "optimize.audit", verdict: Verdict::Deferred, requirement: "", why: "Optimization is deferred from v1." },
    CommandCap { id: "optimize.reduce", verdict: Verdict::Deferred, requirement: "", why: "Optimization is deferred from v1." },
    CommandCap { id: "page.boxes", verdict: Verdict::Deferred, requirement: "", why: "Page operations beyond the v1 set are deferred." },
    CommandCap { id: "page.combine", verdict: Verdict::Core, requirement: "CORE-05", why: "Organize pages (the v1 operations only)." },
    CommandCap { id: "page.copy", verdict: Verdict::Deferred, requirement: "", why: "Page operations beyond the v1 set are deferred." },
    CommandCap { id: "page.crop", verdict: Verdict::Deferred, requirement: "", why: "Page operations beyond the v1 set are deferred." },
    CommandCap { id: "page.cut", verdict: Verdict::Deferred, requirement: "", why: "Page operations beyond the v1 set are deferred." },
    CommandCap { id: "page.delete", verdict: Verdict::Core, requirement: "CORE-05", why: "Organize pages (the v1 operations only)." },
    CommandCap { id: "page.duplicate", verdict: Verdict::Deferred, requirement: "", why: "Page operations beyond the v1 set are deferred." },
    CommandCap { id: "page.extract", verdict: Verdict::Core, requirement: "CORE-05", why: "Organize pages (the v1 operations only)." },
    CommandCap { id: "page.insert", verdict: Verdict::Core, requirement: "CORE-05", why: "Organize pages (the v1 operations only)." },
    CommandCap { id: "page.insert_blank", verdict: Verdict::Core, requirement: "CORE-05", why: "Organize pages (the v1 operations only)." },
    CommandCap { id: "page.number", verdict: Verdict::Deferred, requirement: "", why: "Page operations beyond the v1 set are deferred." },
    CommandCap { id: "page.organize", verdict: Verdict::Core, requirement: "CORE-05", why: "Organize pages (the v1 operations only)." },
    CommandCap { id: "page.paste", verdict: Verdict::Deferred, requirement: "", why: "Page operations beyond the v1 set are deferred." },
    CommandCap { id: "page.replace", verdict: Verdict::Deferred, requirement: "", why: "Page operations beyond the v1 set are deferred." },
    CommandCap { id: "page.rotate", verdict: Verdict::Core, requirement: "CORE-05", why: "Organize pages (the v1 operations only)." },
    CommandCap { id: "page.rotate_ccw", verdict: Verdict::Core, requirement: "CORE-05", why: "Organize pages (the v1 operations only)." },
    CommandCap { id: "page.rotate_dialog", verdict: Verdict::Core, requirement: "CORE-05", why: "Organize pages (the v1 operations only)." },
    CommandCap { id: "page.split", verdict: Verdict::Core, requirement: "CORE-05", why: "Organize pages (the v1 operations only)." },
    CommandCap { id: "page.transitions", verdict: Verdict::Deferred, requirement: "", why: "Page operations beyond the v1 set are deferred." },
    CommandCap { id: "preflight.run", verdict: Verdict::Deferred, requirement: "", why: "Preflight is deferred from v1." },
    CommandCap { id: "prepress.convert_colors", verdict: Verdict::Deferred, requirement: "", why: "Prepress is deferred from v1." },
    CommandCap { id: "prepress.hairlines", verdict: Verdict::Deferred, requirement: "", why: "Prepress is deferred from v1." },
    CommandCap { id: "prepress.marks", verdict: Verdict::Deferred, requirement: "", why: "Prepress is deferred from v1." },
    CommandCap { id: "prepress.output_preview", verdict: Verdict::Deferred, requirement: "", why: "Prepress is deferred from v1." },
    CommandCap { id: "print.dialog", verdict: Verdict::Core, requirement: "CORE-07", why: "Print." },
    CommandCap { id: "protect.certificate", verdict: Verdict::Deferred, requirement: "", why: "Adding or removing protection is a separate qualified feature." },
    CommandCap { id: "protect.password", verdict: Verdict::Deferred, requirement: "", why: "Adding or removing protection is a separate qualified feature." },
    CommandCap { id: "protect.properties", verdict: Verdict::Deferred, requirement: "", why: "Adding or removing protection is a separate qualified feature." },
    CommandCap { id: "protect.remove", verdict: Verdict::Deferred, requirement: "", why: "Adding or removing protection is a separate qualified feature." },
    CommandCap { id: "protect.remove_hidden", verdict: Verdict::Deferred, requirement: "", why: "Adding or removing protection is a separate qualified feature." },
    CommandCap { id: "protect.unlock", verdict: Verdict::Deferred, requirement: "", why: "Adding or removing protection is a separate qualified feature." },
    CommandCap { id: "redact.apply", verdict: Verdict::Deferred, requirement: "", why: "Permanent redaction is deferred from v1." },
    CommandCap { id: "redact.clear", verdict: Verdict::Deferred, requirement: "", why: "Permanent redaction is deferred from v1." },
    CommandCap { id: "redact.mark", verdict: Verdict::Deferred, requirement: "", why: "Permanent redaction is deferred from v1." },
    CommandCap { id: "redact.pages", verdict: Verdict::Deferred, requirement: "", why: "Permanent redaction is deferred from v1." },
    CommandCap { id: "redact.properties", verdict: Verdict::Deferred, requirement: "", why: "Permanent redaction is deferred from v1." },
    CommandCap { id: "redact.sanitize", verdict: Verdict::Deferred, requirement: "", why: "Permanent redaction is deferred from v1." },
    CommandCap { id: "redact.search", verdict: Verdict::Deferred, requirement: "", why: "Permanent redaction is deferred from v1." },
    CommandCap { id: "search.index", verdict: Verdict::Deferred, requirement: "", why: "Search indexing is deferred from v1." },
    CommandCap { id: "sign.certify", verdict: Verdict::Deferred, requirement: "", why: "Certificate signing, fill-and-sign and validation claims are deferred (a simple appearance is CORE-09, BUILD-016)." },
    CommandCap { id: "sign.certify_invisible", verdict: Verdict::Deferred, requirement: "", why: "Certificate signing, fill-and-sign and validation claims are deferred (a simple appearance is CORE-09, BUILD-016)." },
    CommandCap { id: "sign.digital", verdict: Verdict::Deferred, requirement: "", why: "Certificate signing, fill-and-sign and validation claims are deferred (a simple appearance is CORE-09, BUILD-016)." },
    CommandCap { id: "sign.fill.check", verdict: Verdict::Deferred, requirement: "", why: "Certificate signing, fill-and-sign and validation claims are deferred (a simple appearance is CORE-09, BUILD-016)." },
    CommandCap { id: "sign.fill.cross", verdict: Verdict::Deferred, requirement: "", why: "Certificate signing, fill-and-sign and validation claims are deferred (a simple appearance is CORE-09, BUILD-016)." },
    CommandCap { id: "sign.fill.date", verdict: Verdict::Deferred, requirement: "", why: "Certificate signing, fill-and-sign and validation claims are deferred (a simple appearance is CORE-09, BUILD-016)." },
    CommandCap { id: "sign.fill.dot", verdict: Verdict::Deferred, requirement: "", why: "Certificate signing, fill-and-sign and validation claims are deferred (a simple appearance is CORE-09, BUILD-016)." },
    CommandCap { id: "sign.fill.initials", verdict: Verdict::Deferred, requirement: "", why: "Certificate signing, fill-and-sign and validation claims are deferred (a simple appearance is CORE-09, BUILD-016)." },
    CommandCap { id: "sign.fill.initials.change", verdict: Verdict::Deferred, requirement: "", why: "Certificate signing, fill-and-sign and validation claims are deferred (a simple appearance is CORE-09, BUILD-016)." },
    CommandCap { id: "sign.fill.initials.remove", verdict: Verdict::Deferred, requirement: "", why: "Certificate signing, fill-and-sign and validation claims are deferred (a simple appearance is CORE-09, BUILD-016)." },
    CommandCap { id: "sign.fill.line", verdict: Verdict::Deferred, requirement: "", why: "Certificate signing, fill-and-sign and validation claims are deferred (a simple appearance is CORE-09, BUILD-016)." },
    CommandCap { id: "sign.fill.signature", verdict: Verdict::Deferred, requirement: "", why: "Certificate signing, fill-and-sign and validation claims are deferred (a simple appearance is CORE-09, BUILD-016)." },
    CommandCap { id: "sign.fill.signature.change", verdict: Verdict::Deferred, requirement: "", why: "Certificate signing, fill-and-sign and validation claims are deferred (a simple appearance is CORE-09, BUILD-016)." },
    CommandCap { id: "sign.fill.signature.remove", verdict: Verdict::Deferred, requirement: "", why: "Certificate signing, fill-and-sign and validation claims are deferred (a simple appearance is CORE-09, BUILD-016)." },
    CommandCap { id: "sign.fill.text", verdict: Verdict::Deferred, requirement: "", why: "Certificate signing, fill-and-sign and validation claims are deferred (a simple appearance is CORE-09, BUILD-016)." },
    CommandCap { id: "sign.panel", verdict: Verdict::Deferred, requirement: "", why: "Certificate signing, fill-and-sign and validation claims are deferred (a simple appearance is CORE-09, BUILD-016)." },
    CommandCap { id: "sign.timestamp", verdict: Verdict::Deferred, requirement: "", why: "Certificate signing, fill-and-sign and validation claims are deferred (a simple appearance is CORE-09, BUILD-016)." },
    CommandCap { id: "sign.validate", verdict: Verdict::Deferred, requirement: "", why: "Certificate signing, fill-and-sign and validation claims are deferred (a simple appearance is CORE-09, BUILD-016)." },
    CommandCap { id: "standards.pdfa", verdict: Verdict::Deferred, requirement: "", why: "PDF/A, PDF/X and PDF/UA claims are deferred from v1." },
    CommandCap { id: "standards.pdfua", verdict: Verdict::Deferred, requirement: "", why: "PDF/A, PDF/X and PDF/UA claims are deferred from v1." },
    CommandCap { id: "standards.pdfx", verdict: Verdict::Deferred, requirement: "", why: "PDF/A, PDF/X and PDF/UA claims are deferred from v1." },
    CommandCap { id: "tools.document_js", verdict: Verdict::Excluded, requirement: "", why: "Document scripts are not supported in v1." },
    CommandCap { id: "tools.js_console", verdict: Verdict::Excluded, requirement: "", why: "Document scripts are not supported in v1." },
    CommandCap { id: "ui.dock.reset", verdict: Verdict::Core, requirement: "CORE-01", why: "Open and view." },
    CommandCap { id: "view.continuous", verdict: Verdict::Core, requirement: "CORE-01", why: "View-local: zoom, layout and theme act on view state, not on the document." },
    CommandCap { id: "view.fit_one_page", verdict: Verdict::Core, requirement: "CORE-01", why: "View-local: zoom, layout and theme act on view state, not on the document." },
    CommandCap { id: "view.fit_visible", verdict: Verdict::Core, requirement: "CORE-01", why: "View-local: zoom, layout and theme act on view state, not on the document." },
    CommandCap { id: "view.fit_width_scrolling", verdict: Verdict::Core, requirement: "CORE-01", why: "View-local: zoom, layout and theme act on view state, not on the document." },
    CommandCap { id: "view.focus_page_input", verdict: Verdict::Core, requirement: "CORE-01", why: "View-local: zoom, layout and theme act on view state, not on the document." },
    CommandCap { id: "view.full_screen", verdict: Verdict::Core, requirement: "CORE-01", why: "View-local: zoom, layout and theme act on view state, not on the document." },
    CommandCap { id: "view.layout.cover", verdict: Verdict::Core, requirement: "CORE-01", why: "View-local: zoom, layout and theme act on view state, not on the document." },
    CommandCap { id: "view.layout.single", verdict: Verdict::Core, requirement: "CORE-01", why: "View-local: zoom, layout and theme act on view state, not on the document." },
    CommandCap { id: "view.layout.two_up", verdict: Verdict::Core, requirement: "CORE-01", why: "View-local: zoom, layout and theme act on view state, not on the document." },
    CommandCap { id: "view.marquee_zoom", verdict: Verdict::Core, requirement: "CORE-01", why: "View-local: zoom, layout and theme act on view state, not on the document." },
    CommandCap { id: "view.palette", verdict: Verdict::Core, requirement: "CORE-01", why: "View-local: zoom, layout and theme act on view state, not on the document." },
    CommandCap { id: "view.read_mode", verdict: Verdict::Core, requirement: "CORE-01", why: "View-local: zoom, layout and theme act on view state, not on the document." },
    CommandCap { id: "view.theme", verdict: Verdict::Core, requirement: "CORE-01", why: "View-local: zoom, layout and theme act on view state, not on the document." },
    CommandCap { id: "view.theme.dark", verdict: Verdict::Core, requirement: "CORE-01", why: "View-local: zoom, layout and theme act on view state, not on the document." },
    CommandCap { id: "view.theme.light", verdict: Verdict::Core, requirement: "CORE-01", why: "View-local: zoom, layout and theme act on view state, not on the document." },
    CommandCap { id: "view.theme.system", verdict: Verdict::Core, requirement: "CORE-01", why: "View-local: zoom, layout and theme act on view state, not on the document." },
];

#[rustfmt::skip]
pub static ROUTES: &[Route] = &[
    Route { id: "cli:info", kind: "CLI subcommand", verdict: Verdict::Core, note: "Reads a document summary. Opens the file named on the command line, outside any grant.", owner: "BUILD-006" },
    Route { id: "cli:render", kind: "CLI subcommand", verdict: Verdict::Core, note: "Renders one page to an image file. Same file-access note as `info`.", owner: "BUILD-006" },
    Route { id: "cli:text", kind: "CLI subcommand", verdict: Verdict::Core, note: "Extracts text. Same file-access note as `info`.", owner: "BUILD-006" },
    Route { id: "cli:edit", kind: "CLI subcommand", verdict: Verdict::Gated, note: "Rotate, delete, move, insert blank, title, author, full rewrite in one call, writing the output file. Runs engine edits directly, not through tool or command checks.", owner: "BUILD-007" },
    Route { id: "cli:combine", kind: "CLI subcommand", verdict: Verdict::Gated, note: "Merges PDFs into an output file without the tool layer.", owner: "BUILD-007" },
    Route { id: "cli:extract", kind: "CLI subcommand", verdict: Verdict::Gated, note: "Extracts pages into an output file without the tool layer.", owner: "BUILD-007" },
    Route { id: "cli:split", kind: "CLI subcommand", verdict: Verdict::Gated, note: "Splits into files in a directory without the tool layer.", owner: "BUILD-007" },
    Route { id: "cli:batch", kind: "CLI subcommand", verdict: Verdict::Deferred, note: "Runs convert, OCR and optimize over files and folders; all three are deferred from v1.", owner: "BUILD-007" },
    Route { id: "cli:check", kind: "CLI subcommand", verdict: Verdict::Excluded, note: "Robustness sweep over a corpus for development; not a user capability.", owner: "BUILD-007" },
    Route { id: "cli:audit", kind: "CLI subcommand", verdict: Verdict::Deferred, note: "Finds faux redactions; redaction is deferred.", owner: "BUILD-007" },
    Route { id: "cli:tools", kind: "CLI subcommand", verdict: Verdict::Core, note: "Lists every tool and schema, including deferred ones, so the catalogue is visible to agents.", owner: "BUILD-008" },
    Route { id: "cli:run", kind: "CLI subcommand", verdict: Verdict::Gated, note: "Runs any tool by name, and any script of tools, with no profile check: the main inherited bypass for tools and for `command_run`.", owner: "BUILD-007" },
    Route { id: "cli:mcp", kind: "CLI subcommand", verdict: Verdict::Gated, note: "Starts the MCP server on stdin/stdout (opt-in). Its routes are listed separately.", owner: "BUILD-008" },
    Route { id: "cli:ui", kind: "CLI subcommand", verdict: Verdict::Gated, note: "Drives a running app through the control channel. Its methods are listed separately.", owner: "BUILD-005" },
    Route { id: "mcp:tools/list", kind: "MCP method", verdict: Verdict::Core, note: "Lists every tool (all 163) in full mode; the compact list is a convenience, not a restriction.", owner: "BUILD-008" },
    Route { id: "mcp:tools/call", kind: "MCP method", verdict: Verdict::Gated, note: "Calls any tool by name with no profile check.", owner: "BUILD-008" },
    Route { id: "mcp:tool_search", kind: "MCP meta tool (compact mode)", verdict: Verdict::Core, note: "Searches all tools, including deferred ones.", owner: "BUILD-008" },
    Route { id: "mcp:tool_call", kind: "MCP meta tool (compact mode)", verdict: Verdict::Gated, note: "Calls any tool by name even though compact `tools/list` shows only a core set: the compact list does not restrict what can be called.", owner: "BUILD-008" },
    Route { id: "mcp:resources/read", kind: "MCP method", verdict: Verdict::Gated, note: "Reads document information, text, page text and page images by URI, equal to doc_info, text_extract and page_render, outside any per-tool grant.", owner: "BUILD-008" },
    Route { id: "mcp:resources/list", kind: "MCP method", verdict: Verdict::Gated, note: "Lists open documents and the command catalogue (`pdfcraft://document`, `pdfcraft://commands`); reveals document names.", owner: "BUILD-008" },
    Route { id: "control:ui.command", kind: "UI control method", verdict: Verdict::Gated, note: "Runs any registry command in the running app, including deferred ones.", owner: "BUILD-005" },
    Route { id: "control:ui.open", kind: "UI control method", verdict: Verdict::Gated, note: "Opens any file path in the running app.", owner: "BUILD-005" },
    Route { id: "control:ui.key/type/click/drag/move/focus/set", kind: "UI control methods", verdict: Verdict::Gated, note: "Injects input events and sets values; reaches whatever the visible UI reaches, including commands the profile denies.", owner: "BUILD-005" },
    Route { id: "control:ui.screenshot/inspect/state/commands", kind: "UI control methods", verdict: Verdict::Gated, note: "Reads what is on screen, including document content and file names.", owner: "BUILD-005" },
    Route { id: "control:ui.dock/dock.reset/resize/quit", kind: "UI control methods", verdict: Verdict::Core, note: "Window layout and lifecycle only.", owner: "BUILD-005" },
    Route { id: "app:file-association-open", kind: "OS entry point", verdict: Verdict::Core, note: "Explorer Open with, double click, drag and drop, recent files and the single-instance hand-off open files as tabs.", owner: "BUILD-006" },
    Route { id: "app:shell-verb-edit", kind: "OS entry point", verdict: Verdict::Core, note: "\"Edit with PdfCraft\" on PDF files opens each selected PDF.", owner: "BUILD-006" },
    Route { id: "app:shell-verb-create-pdf", kind: "OS entry point", verdict: Verdict::Deferred, note: "The \"Create PDF\" verb on image files (png, jpg, tif, gif, bmp, jp2 ...) runs `--create-images`, a creation flow outside the v1 list.", owner: "BUILD-011" },
    Route { id: "app:cli-option-flags", kind: "OS entry point", verdict: Verdict::Gated, note: "Any `--name value` on the app command line is applied as a setting override.", owner: "BUILD-008" },
    Route { id: "app:update-check", kind: "OS entry point", verdict: Verdict::Excluded, note: "Help > Check for updates (`help.check_updates`) asks api.github.com for the latest release of the upstream project (`storytold/pdfcraft`). It is started by the user, but `command_run` and `ui.command` reach it too; BDF has no update endpoint, and the network stays off by default (identity migration, BUILD-003 deliverables 4 and 5).", owner: "BUILD-003" },
    Route { id: "app:ui-menus-toolbars-palette", kind: "UI", verdict: Verdict::Gated, note: "Menus, the toolbar, the command palette and the tool panel build from the registry and catalogue and show deferred items; hiding a button does not deny the command (`ui.command`, `command_run`, shortcuts reach it).", owner: "BUILD-011" },
];

/// The inventory entry for a tool.
pub fn tool(name: &str) -> Option<&'static ToolCap> {
    TOOLS.iter().find(|t| t.name == name)
}

/// The inventory entry for a command id.
pub fn command(id: &str) -> Option<&'static CommandCap> {
    COMMANDS.iter().find(|c| c.id == id)
}

fn deny(what: &str, why: impl std::fmt::Display) -> Decision {
    Decision::Deny(format!("{what} is not allowed in the desktop-core profile: {why}"))
}

/// May this command id run in the desktop-core profile? Unlisted ids are denied.
pub fn check_command(id: &str) -> Decision {
    match command(id) {
        None => deny(&format!("command `{id}`"), "it is not in the capability inventory (default deny)"),
        Some(c) if c.verdict == Verdict::Core => Decision::Allow,
        Some(c) => deny(&format!("command `{id}`"), c.why),
    }
}

fn is_unset(v: &Value) -> bool {
    matches!(v, Value::Null | Value::Bool(false))
}

/// The string values of an enumerated argument (one string, or an array of strings); `None` when the
/// argument is anything else, which no enumerated parameter accepts.
fn enum_values(v: &Value) -> Option<Vec<&str>> {
    match v {
        Value::String(s) => Some(vec![s.as_str()]),
        Value::Array(items) => items.iter().map(Value::as_str).collect(),
        _ => None,
    }
}

/// May this tool be called with these arguments in the desktop-core profile?
///
/// A tool that is not listed, a deferred or excluded tool, a parameter that is not listed for a core
/// tool, a refused parameter and an excluded or unknown enumerated mode are all denied. `command_run`
/// and `command_batch` are allowed only when every command they name is core.
pub fn check_tool_call(name: &str, args: &Value) -> Decision {
    let what = format!("tool `{name}`");
    let Some(cap) = tool(name) else {
        return deny(&what, "it is not in the capability inventory (default deny)");
    };
    match cap.verdict {
        Verdict::Deferred | Verdict::Excluded => return deny(&what, cap.why),
        Verdict::Gated => return check_dispatcher(name, args),
        Verdict::Core => {}
    }
    match args {
        Value::Null => check_core_args(cap, &what, &serde_json::Map::new()),
        Value::Object(o) => check_core_args(cap, &what, o),
        _ => deny(&what, "its arguments must be an object"),
    }
}

/// The parameter and mode rules of a core tool. Modes are judged as they will run: a missing or null
/// argument selects the rule's default, which must be core like any explicit value.
fn check_core_args(cap: &ToolCap, what: &str, obj: &serde_json::Map<String, Value>) -> Decision {
    for (key, value) in obj {
        if cap.denied_params.contains(&key.as_str()) {
            if is_unset(value) {
                continue;
            }
            return deny(what, format!("parameter `{key}` is excluded from v1"));
        }
        if !cap.params.contains(&key.as_str()) {
            return deny(what, format!("parameter `{key}` is not in the inventory (default deny)"));
        }
    }
    for rule in cap.enums {
        let given = obj.get(rule.param).filter(|v| !v.is_null());
        let (values, source) = match given {
            Some(value) => match enum_values(value) {
                Some(values) => (values, ""),
                None => return deny(what, format!("parameter `{}` must be a string or an array of strings", rule.param)),
            },
            None => match rule.default {
                Some(default) => (vec![default], " (the default, used because it was omitted or null)"),
                None => continue,
            },
        };
        for v in values {
            if !rule.core.contains(&v) {
                let kind = if rule.excluded.contains(&v) { "an excluded mode" } else { "an unknown mode (default deny)" };
                return deny(what, format!("`{}` = `{v}`{source} is {kind}", rule.param));
            }
        }
    }
    Decision::Allow
}

fn check_dispatcher(name: &str, args: &Value) -> Decision {
    let what = format!("tool `{name}`");
    let step = |v: &Value| -> Decision {
        match v.get("id").and_then(Value::as_str) {
            Some(id) => check_command(id),
            None => deny(&what, "a step names no command id"),
        }
    };
    match name {
        "command_run" => step(args),
        "command_batch" => match args.get("steps").and_then(Value::as_array) {
            Some(steps) if !steps.is_empty() => steps.iter().map(step).find(|d| !d.is_allowed()).unwrap_or(Decision::Allow),
            _ => deny(&what, "it needs at least one step"),
        },
        _ => deny(&what, "it is gated but has no dispatcher rule (default deny)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::{BTreeMap, BTreeSet};

    fn names<T>(items: &[T], f: impl Fn(&T) -> &'static str) -> Vec<&'static str> {
        items.iter().map(f).collect()
    }

    fn no_duplicates(what: &str, list: &[&str]) {
        let set: BTreeSet<_> = list.iter().collect();
        assert_eq!(set.len(), list.len(), "{what} lists an entry twice");
    }

    /// Every command id the product can name: the registry plus the commands the tool catalogue lists.
    fn live_command_ids() -> BTreeSet<&'static str> {
        let mut ids: BTreeSet<&'static str> = pdfcraft_engine::commands::COMMANDS.iter().map(|c| c.id).collect();
        for group in pdfcraft_engine::catalog::TOOL_GROUPS {
            for section in group.sections {
                for item in section.items {
                    if !item.command.is_empty() {
                        ids.insert(item.command);
                    }
                }
            }
        }
        ids
    }

    #[test]
    fn every_tool_is_classified_once_and_no_entry_is_stale() {
        let live: BTreeSet<&str> = crate::tools::tools().iter().map(|t| t.name).collect();
        let listed = names(TOOLS, |t| t.name);
        no_duplicates("TOOLS", &listed);
        let listed: BTreeSet<&str> = listed.into_iter().collect();
        let unclassified: Vec<_> = live.difference(&listed).collect();
        let stale: Vec<_> = listed.difference(&live).collect();
        assert!(unclassified.is_empty(), "tools missing from the capability inventory (classify them; they are denied until then): {unclassified:?}");
        assert!(stale.is_empty(), "inventory entries for tools that no longer exist: {stale:?}");
    }

    #[test]
    fn every_command_and_catalogue_command_is_classified_once_and_no_entry_is_stale() {
        let live = live_command_ids();
        let listed = names(COMMANDS, |c| c.id);
        no_duplicates("COMMANDS", &listed);
        let listed: BTreeSet<&str> = listed.into_iter().collect();
        let unclassified: Vec<_> = live.difference(&listed).collect();
        let stale: Vec<_> = listed.difference(&live).collect();
        assert!(unclassified.is_empty(), "commands missing from the capability inventory: {unclassified:?}");
        assert!(stale.is_empty(), "inventory entries for commands that no longer exist: {stale:?}");
    }

    #[test]
    fn entries_say_why_and_core_entries_name_their_requirement() {
        for t in TOOLS {
            assert!(!t.why.trim().is_empty(), "{} has no reason", t.name);
            let wants_req = matches!(t.verdict, Verdict::Core | Verdict::Gated);
            assert_eq!(!t.requirement.is_empty(), wants_req, "{}: only core and gated tools name a requirement", t.name);
        }
        for c in COMMANDS {
            if c.verdict != Verdict::Core {
                assert!(!c.why.trim().is_empty(), "{} has no reason", c.id);
            }
            assert_eq!(!c.requirement.is_empty(), c.verdict == Verdict::Core, "{}: only core commands name a requirement", c.id);
        }
        for r in ROUTES {
            assert!(!r.note.trim().is_empty() && r.owner.starts_with("BUILD-"), "{}: a route needs a note and an owner", r.id);
        }
        no_duplicates("ROUTES", &names(ROUTES, |r| r.id));
    }

    #[test]
    fn a_tool_and_the_command_it_automates_agree() {
        let mut bad = Vec::new();
        for def in crate::tools::tools() {
            let (Some(cap), Some(id)) = (tool(def.name), def.command) else { continue };
            let command_core = command(id).is_some_and(|c| c.verdict == Verdict::Core);
            let tool_core = cap.verdict == Verdict::Core;
            if tool_core != command_core {
                bad.push(format!("{} ({:?}) automates {id} ({:?})", def.name, cap.verdict, command(id).map(|c| c.verdict)));
            }
        }
        assert!(
            bad.is_empty(),
            "a core tool must automate a core command and the reverse; otherwise one route bypasses the other:\n{}",
            bad.join("\n")
        );
    }

    fn schema_enums(def: &crate::tools::ToolDef) -> BTreeMap<String, BTreeSet<String>> {
        let mut out = BTreeMap::new();
        let props = def.input_schema.get("properties").and_then(Value::as_object).cloned().unwrap_or_default();
        for (k, v) in props {
            let direct = v.get("enum").and_then(Value::as_array);
            let nested = v.get("items").and_then(|i| i.get("enum")).and_then(Value::as_array);
            if let Some(values) = direct.or(nested) {
                out.insert(k, values.iter().filter_map(|s| s.as_str().map(str::to_string)).collect());
            }
        }
        out
    }

    #[test]
    fn core_tool_parameters_and_modes_match_their_schemas_exactly() {
        for def in crate::tools::tools() {
            let Some(cap) = tool(def.name) else { continue };
            if cap.verdict != Verdict::Core {
                assert!(
                    cap.params.is_empty() && cap.denied_params.is_empty() && cap.enums.is_empty(),
                    "{}: only core tools carry parameter rules",
                    def.name
                );
                continue;
            }
            let props: BTreeSet<String> =
                def.input_schema.get("properties").and_then(Value::as_object).map(|p| p.keys().cloned().collect()).unwrap_or_default();
            let ruled: BTreeSet<String> = cap.params.iter().chain(cap.denied_params).map(|s| s.to_string()).collect();
            assert_eq!(
                props, ruled,
                "{}: every parameter must be allowed or refused in the inventory (a new one is denied until classified)",
                def.name
            );
            assert!(cap.params.iter().all(|p| !cap.denied_params.contains(p)), "{}: a parameter is both allowed and refused", def.name);
            let live = schema_enums(&def);
            let listed: BTreeSet<String> = cap.enums.iter().map(|r| r.param.to_string()).collect();
            assert_eq!(live.keys().cloned().collect::<BTreeSet<_>>(), listed, "{}: every enumerated parameter needs a mode rule", def.name);
            for rule in cap.enums {
                let split: BTreeSet<String> = rule.core.iter().chain(rule.excluded).map(|s| s.to_string()).collect();
                assert_eq!(
                    live.get(rule.param),
                    Some(&split),
                    "{}.{}: core plus excluded modes must be exactly the schema's values",
                    def.name,
                    rule.param
                );
                assert!(rule.core.iter().all(|v| !rule.excluded.contains(v)), "{}.{}: a mode is both core and excluded", def.name, rule.param);
            }
        }
    }

    #[test]
    fn the_core_requirements_are_covered_and_the_gaps_are_recorded() {
        let covered: BTreeSet<&str> = TOOLS.iter().filter(|t| t.verdict == Verdict::Core).map(|t| t.requirement).collect();
        for r in ["CORE-01", "CORE-02", "CORE-03", "CORE-04", "CORE-05", "CORE-06", "CORE-07", "CORE-08", "CORE-10"] {
            assert!(covered.contains(r), "no core tool serves {r}");
        }
        for (r, why) in GAPS {
            assert!(!covered.contains(r) && !why.is_empty(), "{r} is recorded as a gap but has a core tool");
        }
        for r in ["CORE-01", "CORE-02", "CORE-03", "CORE-04", "CORE-05", "CORE-06", "CORE-07", "CORE-08"] {
            assert!(COMMANDS.iter().any(|c| c.verdict == Verdict::Core && c.requirement == r), "no core command serves {r}");
        }
    }

    #[test]
    fn unknown_capabilities_are_denied() {
        assert!(!check_tool_call("no_such_tool", &json!({})).is_allowed());
        assert!(!check_command("no.such_command").is_allowed());
        assert!(!check_tool_call("doc_info", &json!({ "doc": 1, "invented": true })).is_allowed(), "an unlisted parameter");
        assert!(!check_tool_call("doc_info", &json!("not an object")).is_allowed());
        assert!(check_tool_call("doc_info", &json!({ "doc": 1 })).is_allowed());
        assert!(check_tool_call("printers", &Value::Null).is_allowed());
    }

    #[test]
    fn deferred_and_excluded_tools_and_commands_are_denied_with_their_reason() {
        for (name, needle) in [("text_edit", "deferred"), ("redact_apply", "redaction"), ("js_run", "scripts"), ("link_open", "external")] {
            let Decision::Deny(why) = check_tool_call(name, &json!({})) else { panic!("{name} must be denied") };
            assert!(why.to_lowercase().contains(needle), "{name}: {why}");
        }
        assert!(!check_command("redact.apply").is_allowed());
        assert!(!check_command("help.check_updates").is_allowed());
        assert!(check_command("file.save").is_allowed());
    }

    #[test]
    fn excluded_parameters_and_modes_of_a_core_tool_are_denied() {
        // Irreversible flattening on save, and extract-and-delete, are not v1 modes; false is harmless.
        assert!(!check_tool_call("doc_save", &json!({ "doc": 1, "flatten_fill_sign": true })).is_allowed());
        assert!(check_tool_call("doc_save", &json!({ "doc": 1, "flatten_fill_sign": false, "full": true })).is_allowed());
        assert!(!check_tool_call("page_extract", &json!({ "doc": 1, "pages": [1], "delete": true })).is_allowed());
        assert!(!check_tool_call("doc_split", &json!({ "doc": 1, "bookmarks": true })).is_allowed());
        // Annotation kinds.
        assert!(check_tool_call("comment_add", &json!({ "doc": 1, "page": 1, "type": "highlight" })).is_allowed());
        assert!(!check_tool_call("comment_add", &json!({ "doc": 1, "page": 1, "type": "stamp" })).is_allowed());
        assert!(!check_tool_call("comment_add", &json!({ "doc": 1, "page": 1, "type": "attachment" })).is_allowed());
        assert!(!check_tool_call("comment_add", &json!({ "doc": 1, "page": 1, "type": "arrow", "endings": ["None", "Slash"] })).is_allowed());
        assert!(check_tool_call("comment_add", &json!({ "doc": 1, "page": 1, "type": "arrow", "endings": ["None", "OpenArrow"] })).is_allowed());
        assert!(!check_tool_call("comment_add", &json!({ "doc": 1, "page": 1, "type": 7 })).is_allowed(), "a non-string mode");
        // Printing: actual size and fit only; large sheets are never silently scaled by a poster or shrink mode.
        assert!(check_tool_call("doc_print", &json!({ "doc": 1, "layout": "actual", "copies": 2 })).is_allowed());
        assert!(!check_tool_call("doc_print", &json!({ "doc": 1, "layout": "poster" })).is_allowed());
        assert!(!check_tool_call("doc_print", &json!({ "doc": 1, "layout": "unheard-of" })).is_allowed(), "an unknown mode");
        assert!(!check_tool_call("doc_combine", &json!({ "paths": ["a.pdf"], "field_conflicts": "merge" })).is_allowed());
    }

    // ---- R3 (Codex review): a mode's default is an effective mode ----

    static SYNTHETIC: ToolCap = ToolCap {
        name: "synthetic",
        verdict: Verdict::Core,
        requirement: "CORE-07",
        why: "a tool whose default mode is excluded",
        params: &["doc", "mode"],
        denied_params: &[],
        enums: &[EnumRule { param: "mode", default: Some("risky"), core: &["safe"], excluded: &["risky"] }],
    };

    fn synthetic(args: Value) -> Decision {
        check_core_args(&SYNTHETIC, "tool `synthetic`", args.as_object().expect("an object"))
    }

    #[test]
    fn a_default_that_is_an_excluded_mode_denies_the_omitted_null_and_explicit_forms_alike() {
        let omitted = synthetic(json!({ "doc": 1 }));
        let null = synthetic(json!({ "doc": 1, "mode": null }));
        let explicit = synthetic(json!({ "doc": 1, "mode": "risky" }));
        assert!(!omitted.is_allowed() && !null.is_allowed() && !explicit.is_allowed(), "{omitted:?} {null:?} {explicit:?}");
        let Decision::Deny(why) = &omitted else { panic!("omitted must be denied") };
        assert!(why.contains("the default"), "the refusal says the default was the reason: {why}");
        assert!(synthetic(json!({ "doc": 1, "mode": "safe" })).is_allowed(), "an explicit core mode overrides the default");
    }

    #[test]
    fn every_defaulted_mode_is_a_core_mode_and_reads_the_same_omitted_null_or_explicit() {
        for cap in TOOLS.iter().filter(|t| t.verdict == Verdict::Core) {
            assert!(
                check_tool_call(cap.name, &json!({})).is_allowed(),
                "{}: a call that omits every mode must be allowed, so every default must be core",
                cap.name
            );
            for rule in cap.enums {
                let Some(default) = rule.default else { continue };
                assert!(
                    rule.core.contains(&default),
                    "{}.{}: the default `{default}` must be a core mode (decide it explicitly)",
                    cap.name,
                    rule.param
                );
                let omitted = check_tool_call(cap.name, &json!({}));
                let null = check_tool_call(cap.name, &json!({ rule.param: null }));
                let explicit = check_tool_call(cap.name, &json!({ rule.param: default }));
                assert_eq!(
                    (&omitted, &null),
                    (&explicit, &explicit),
                    "{}.{}: omitted, null and explicit-default forms must agree",
                    cap.name,
                    rule.param
                );
            }
        }
    }

    /// The review's counterexample: the print default selects `document-and-markups`.
    #[test]
    fn the_print_default_is_judged_like_the_explicit_value_it_selects() {
        let omitted = check_tool_call("doc_print", &json!({ "doc": 1, "path": "print.pdf" }));
        let explicit = check_tool_call("doc_print", &json!({ "doc": 1, "path": "print.pdf", "comments_forms": "document-and-markups" }));
        let null = check_tool_call("doc_print", &json!({ "doc": 1, "path": "print.pdf", "comments_forms": null }));
        assert!(omitted.is_allowed() && explicit.is_allowed() && null.is_allowed(), "printing what is on screen, comments included, is core");
        for excluded in ["document-and-stamps", "form-fields-only"] {
            assert!(!check_tool_call("doc_print", &json!({ "doc": 1, "comments_forms": excluded })).is_allowed(), "{excluded} stays excluded");
        }
    }

    fn call(a: &mut crate::Automation, tool: &str, args: Value) -> Value {
        match a.call(tool, &args) {
            Ok(mut content) => match content.remove(0) {
                crate::Content::Json(v) => v,
                other => panic!("{tool}: {other:?}"),
            },
            Err(e) => panic!("{tool} {args}: {e}"),
        }
    }

    /// Two pages, 200 x 300 points, so the page is portrait.
    fn two_pages() -> Vec<u8> {
        let objs = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 /MediaBox [0 0 200 300] >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /Contents 4 0 R /Resources << >> >>".to_string(),
            "<< /Length 0 >>\nstream\n\nendstream".to_string(),
            "<< /Type /Page /Parent 2 0 R /Contents 4 0 R /Resources << >> >>".to_string(),
        ];
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
        for o in offsets {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
        out
    }

    /// The constants the inventory names are what the tools do: with the parameter omitted, null or set to
    /// the declared default the real tool gives the same result, and a different value gives a different one.
    #[test]
    fn the_declared_defaults_are_what_the_tools_actually_do() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let mut a = crate::Automation::new();
        let id = a.session.open("two.pdf", None, std::sync::Arc::new(two_pages()), None).expect("open").0;
        call(&mut a, "comment_add", json!({ "doc": id, "page": 1, "type": "note", "at": [40, 40], "contents": "markup" }));
        let print = |a: &mut crate::Automation, name: &str, extra: Value| -> (Value, Vec<u8>) {
            let path = dir.path().join(format!("{name}.pdf"));
            let mut args = json!({ "doc": id, "path": path.to_string_lossy() });
            if let (Some(base), Some(more)) = (args.as_object_mut(), extra.as_object()) {
                base.extend(more.clone());
            }
            let result = call(a, "doc_print", args);
            let bytes = std::fs::read(&path).expect("the printed file");
            (json!({ "sheets": result["sheets"], "pages": result["pages"] }), bytes)
        };
        for rule in tool("doc_print").expect("listed").enums {
            let Some(default) = rule.default else { continue };
            let omitted = print(&mut a, "omitted", json!({}));
            let null = print(&mut a, "null", json!({ rule.param: null }));
            let explicit = print(&mut a, "explicit", json!({ rule.param: default }));
            assert_eq!(omitted, null, "doc_print.{}: null must equal omitted", rule.param);
            assert_eq!(omitted, explicit, "doc_print.{}: the declared default `{default}` must be what omitting it does", rule.param);
        }
        // The default prints the comment; `document` does not: the declared markup default is not a no-op.
        let default = print(&mut a, "default", json!({}));
        let document_only = print(&mut a, "document_only", json!({ "comments_forms": "document" }));
        assert_ne!(default.1, document_only.1, "printing markups must change the output of a page that has a comment");
        let odd = print(&mut a, "odd", json!({ "subset": "odd" }));
        assert_eq!(odd.0["pages"], json!(1), "subset=odd prints one of the two pages");
        assert_eq!(default.0["pages"], json!(2), "the default subset prints every page");

        // page_rotate: omitted, null and `all` rotate the same pages; `landscape` matches none of a portrait document.
        let rotate = |a: &mut crate::Automation, extra: Value| -> Result<Value, String> {
            let mut args = json!({ "doc": id, "degrees": 90, "pages": [1] });
            if let (Some(base), Some(more)) = (args.as_object_mut(), extra.as_object()) {
                base.extend(more.clone());
            }
            a.call("page_rotate", &args)
                .map(|mut c| match c.remove(0) {
                    crate::Content::Json(v) => v,
                    _ => Value::Null,
                })
                .map_err(|e| e.to_string())
        };
        for rule in tool("page_rotate").expect("listed").enums {
            let Some(default) = rule.default else { continue };
            let omitted = rotate(&mut a, json!({})).expect("rotates");
            let null = rotate(&mut a, json!({ rule.param: null })).expect("rotates");
            let explicit = rotate(&mut a, json!({ rule.param: default })).expect("rotates");
            assert_eq!(omitted["rotated"], null["rotated"], "page_rotate.{}", rule.param);
            assert_eq!(omitted["rotated"], explicit["rotated"], "page_rotate.{}: `{default}` is the default", rule.param);
        }
        assert!(rotate(&mut a, json!({ "orientation": "landscape" })).is_err(), "a different mode gives a different result");
    }

    #[test]
    fn the_command_dispatchers_cannot_reach_a_non_core_command() {
        assert!(check_tool_call("command_run", &json!({ "id": "file.save" })).is_allowed());
        assert!(!check_tool_call("command_run", &json!({ "id": "redact.apply" })).is_allowed());
        assert!(!check_tool_call("command_run", &json!({ "id": "not.a_command" })).is_allowed());
        assert!(!check_tool_call("command_run", &json!({})).is_allowed());
        let ok = json!({ "steps": [{ "id": "page.rotate" }, { "id": "file.save" }] });
        assert!(check_tool_call("command_batch", &ok).is_allowed());
        let one_bad = json!({ "steps": [{ "id": "page.rotate" }, { "id": "sign.digital" }, { "id": "file.save" }] });
        assert!(!check_tool_call("command_batch", &one_bad).is_allowed(), "one excluded step denies the batch");
        assert!(!check_tool_call("command_batch", &json!({ "steps": [] })).is_allowed());
        assert!(!check_tool_call("command_batch", &json!({})).is_allowed());
    }
}
