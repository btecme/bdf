//! An MCP server over the application service, for a client that presents a grant (BUILD-009).
//!
//! The other server in this crate ([`crate::mcp`]) runs the automation tools on an engine session of its own, with no
//! notion of who is asking. This one holds a [`Service`] and nothing else: every tool call becomes one typed
//! [`OperationRequest`], so the policy, the revision checks, the confinement and the audit record of the service apply
//! to every call and there is no second path to a document.
//!
//! * **Without a valid grant** the server answers `initialize`, `ping` and lists one tool, `bdf_help`, which says what
//!   this build is and which operations it has. It lists no files and returns no document content (the specification's
//!   rule for an MCP invocation that has no grant); every other tool call is refused.
//! * **With a grant** the tools are the service's operations, under the names the automation tools already use where
//!   there is one: `doc_open`, `doc_info`, `doc_list`, `link_list`, `bookmark_list`, `text_extract`, `text_find`,
//!   `page_rotate`, `edit_undo`, `edit_redo`, `edit_history`, `doc_save` and `doc_close`. What the grant allows is the policy's decision, not this module's: a read-only grant lists the same
//!   tools and is refused at the call.
//! * A tool that changes or discards content names the revision it was made against (`revision`, from the last reply).
//!
//! Opt-in like the other server: it runs when a person launches it and stops when its input closes.

use std::io::{BufRead, Write};
use std::path::PathBuf;

use pdfcraft_app_service::{
    ActorContext, BookmarkList, ConflictKind, DEFAULT_BOOKMARK_PAGE, DEFAULT_FIND_MATCHES, DESCRIPTORS, DocumentInfo, DocumentKey, Effect, ErrorCode,
    FindReport, History, LinkList, LinkTarget, OpenSource, Operation, OperationError, OperationId, OperationRequest, OperationResult, Outcome,
    Overwrite, PageSelection, RequestContext, SaveReport, SaveTarget, Service, TextPages,
};
use serde_json::{Map, Value, json};

use crate::mcp::{
    DEFAULT_MAX_MESSAGE, INVALID_PARAMS, INVALID_REQUEST, METHOD_NOT_FOUND, PARSE_ERROR, PROTOCOL_VERSIONS, check_keys, error, serve_lines,
};

use crate::{BookmarkView, nest_rows};

/// The one tool a caller without a grant can use.
const HELP: &str = "bdf_help";

const INSTRUCTIONS_WITH_GRANT: &str = "BDF opens and edits PDFs for the person who granted this client access. Open a file with doc_open to get a \
document id and its revision. Page numbers are 1-based. Every tool that changes or discards content takes `revision`, the revision it was made \
against; a reply that says `conflict` means the document moved on, so read it again (doc_info) first. Edits are undoable (edit_undo) and stay in \
memory until doc_save. What this client may do is limited by its grant; a refusal says why.";
const INSTRUCTIONS_WITHOUT_GRANT: &str = "This server was started without a valid grant, so it serves no documents and lists no files. Call bdf_help for \
what this build can do. Ask the person to issue a grant (`pdfcraft-cli grant issue`) and start the server with it.";

pub struct ClientMcpServer {
    service: Service,
    /// Who is asking, from [`pdfcraft_app_service::GrantStore::authenticate`]; `None` when no valid grant was presented.
    actor: Option<ActorContext>,
    next_request: u64,
    max_message: usize,
}

impl ClientMcpServer {
    pub fn new(service: Service, actor: Option<ActorContext>) -> Self {
        Self { service, actor, next_request: 0, max_message: DEFAULT_MAX_MESSAGE }
    }

    /// The longest message line read, in bytes ([`DEFAULT_MAX_MESSAGE`] unless set here).
    pub fn with_max_message(mut self, bytes: usize) -> Self {
        self.max_message = bytes;
        self
    }

    /// Whether a valid grant was presented.
    pub fn is_authorized(&self) -> bool {
        self.actor.is_some()
    }

    /// Serve until `input` reaches end of file, with the same bound on a message as the other MCP server.
    pub fn serve(&mut self, input: impl BufRead, output: impl Write) -> std::io::Result<()> {
        let max = self.max_message;
        serve_lines(input, output, max, |line| self.handle_line(line))
    }

    pub fn handle_line(&mut self, line: &str) -> Option<String> {
        let reply = match serde_json::from_str::<Value>(line) {
            Ok(msg) => self.handle(&msg),
            Err(e) => Some(error(Value::Null, PARSE_ERROR, &format!("parse error: {e}"))),
        };
        reply.map(|r| r.to_string())
    }

    /// Handle one parsed message. Notifications (no `id`) get no reply.
    pub fn handle(&mut self, msg: &Value) -> Option<Value> {
        let Some(obj) = msg.as_object() else { return Some(error(Value::Null, INVALID_REQUEST, "expected a JSON-RPC request object")) };
        let id = obj.get("id").cloned();
        let Some(method) = obj.get("method").and_then(Value::as_str) else {
            return id.map(|id| error(id, INVALID_REQUEST, "missing method"));
        };
        let params = obj.get("params").cloned().unwrap_or(Value::Null);
        let id = id?;
        let result =
            if method == "tools/call" { crate::mcp::guarded_tool(|| self.dispatch(method, &params)) } else { self.dispatch(method, &params) };
        Some(match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err((code, message)) => error(id, code, &message),
        })
    }

    fn dispatch(&mut self, method: &str, params: &Value) -> Result<Value, (i64, String)> {
        match method {
            "initialize" => {
                let asked = params.get("protocolVersion").and_then(Value::as_str);
                let version = asked.filter(|v| PROTOCOL_VERSIONS.contains(v)).unwrap_or(PROTOCOL_VERSIONS[0]);
                Ok(json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "bdf", "title": "BDF", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": if self.actor.is_some() { INSTRUCTIONS_WITH_GRANT } else { INSTRUCTIONS_WITHOUT_GRANT },
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": self.tool_list() })),
            "tools/call" => {
                let name = params.get("name").and_then(Value::as_str).ok_or((INVALID_PARAMS, "tools/call needs a tool name".to_string()))?;
                self.call(name, params.get("arguments"))
            }
            "resources/list" => Ok(json!({ "resources": [] })),
            "resources/templates/list" => Ok(json!({ "resourceTemplates": [] })),
            other => Err((METHOD_NOT_FOUND, format!("method not found: {other}"))),
        }
    }

    fn tool_list(&self) -> Vec<Value> {
        let mut tools = vec![tool(
            HELP,
            "What this build can do",
            "Version of this build and the operations it has. Needs no grant and returns no document content.",
            &[],
            true,
            false,
        )];
        if self.actor.is_some() {
            tools.extend(document_tools());
        }
        tools
    }

    fn call(&mut self, name: &str, arguments: Option<&Value>) -> Result<Value, (i64, String)> {
        let args = match arguments {
            None | Some(Value::Null) => Some(Map::new()),
            Some(Value::Object(o)) => Some(o.clone()),
            Some(_) => None,
        };
        if name == HELP {
            let args = args.ok_or((INVALID_PARAMS, "arguments must be an object".to_string()))?;
            check_keys(name, &Value::Object(args), &[]).map_err(|e| (INVALID_PARAMS, e))?;
            return Ok(self.help());
        }
        let Some(spec) = SPECS.iter().find(|s| s.name == name) else {
            return Err((INVALID_PARAMS, format!("unknown tool {name:?}")));
        };
        if self.actor.is_none() {
            return Ok(refusal("not_authorized", "this server was started without a valid grant; call bdf_help, or ask the person to issue a grant"));
        }
        // From here a client with a grant is asking for a document operation, so whatever is wrong with the request is
        // recorded, as every other outcome is.
        let Some(args) = args else {
            return self.refuse_protocol(spec, "arguments must be an object".into());
        };
        let allowed: Vec<&str> = spec.args.iter().map(|a| a.0).collect();
        if let Err(message) = check_keys(name, &Value::Object(args.clone()), &allowed) {
            return self.refuse_protocol(spec, message);
        }
        let (document, operation) = match (spec.build)(&Args { map: &args, session: self.service.session_id() }) {
            Ok(pair) => pair,
            Err(message) => return Ok(failure(&self.reject(spec, message))),
        };
        Ok(self.run(document, operation))
    }

    /// Record a request for `spec` that was refused for its arguments, and return the error to show: the service's, so a
    /// log that cannot be written stops the next call, as it does for a call that reached the service.
    fn reject(&mut self, spec: &Spec, message: String) -> OperationError {
        let Some(actor) = self.actor.clone() else { return OperationError::invalid(message) };
        self.next_request = self.next_request.saturating_add(1);
        self.service.reject(&RequestContext::new(self.next_request, actor), spec.operation, OperationError::invalid(message))
    }

    /// A malformed request is a JSON-RPC error; it is recorded all the same, and shown as the service's refusal instead if
    /// the service is not serving clients.
    fn refuse_protocol(&mut self, spec: &Spec, message: String) -> Result<Value, (i64, String)> {
        let error = self.reject(spec, message);
        if error.code == ErrorCode::InvalidArgument { Err((INVALID_PARAMS, error.message)) } else { Ok(failure(&error)) }
    }

    fn run(&mut self, document: Option<DocumentKey>, operation: Operation) -> Value {
        let Some(actor) = self.actor.clone() else { return refusal("not_authorized", "no grant") };
        self.next_request = self.next_request.saturating_add(1);
        let ctx = RequestContext::new(self.next_request, actor);
        match self.service.execute(&ctx, OperationRequest::new(document, operation)) {
            Ok(result) => success(&result),
            Err(e) => failure(&e),
        }
    }

    fn help(&self) -> Value {
        let operations: Vec<Value> = DESCRIPTORS.iter().map(|d| json!({ "name": d.name, "summary": d.summary })).collect();
        let body = json!({
            "server": "bdf",
            "version": env!("CARGO_PKG_VERSION"),
            "authorized": self.actor.is_some(),
            "operations": operations,
        });
        result_text(&body, false)
    }
}

// ---- tools --------------------------------------------------------------------------------------------------------

type Build = fn(&Args<'_>) -> Result<(Option<DocumentKey>, Operation), String>;

struct Spec {
    name: &'static str,
    /// The service operation the tool is, for the record of a call that is refused before it becomes one.
    operation: OperationId,
    title: &'static str,
    description: &'static str,
    /// Argument name, JSON type, and whether it is required.
    args: &'static [(&'static str, &'static str, bool)],
    read_only: bool,
    destructive: bool,
    build: Build,
}

static SPECS: [Spec; 13] = [
    Spec {
        name: "doc_open",
        operation: OperationId::DocumentOpen,
        title: "Open a PDF",
        description: "Open a PDF file and return its document id, revision, pages and whether it can be edited.",
        args: &[("path", "string", true), ("password", "string", false)],
        read_only: true,
        destructive: false,
        build: |a| Ok((None, Operation::Open { source: OpenSource::File(PathBuf::from(a.string("path")?)), password: a.opt_string("password")? })),
    },
    Spec {
        name: "doc_info",
        operation: OperationId::DocumentQuery,
        title: "Describe a document",
        description: "Pages (1-based, as displayed), whether the document can be edited and its current revision.",
        args: &[("doc", "integer", true)],
        read_only: true,
        destructive: false,
        build: |a| Ok((Some(a.doc()?), Operation::Query)),
    },
    Spec {
        name: "page_rotate",
        operation: OperationId::PageRotate,
        title: "Rotate pages",
        description: "Rotate pages by a multiple of 90 degrees (positive is clockwise): the listed pages, or all. Undoable.",
        args: &[("doc", "integer", true), ("revision", "integer", true), ("pages", "array", false), ("degrees", "integer", true)],
        read_only: false,
        destructive: false,
        build: |a| {
            let pages = match a.opt_int_list("pages")? {
                Some(list) => PageSelection::Numbers(list),
                None => PageSelection::All,
            };
            Ok((Some(a.doc()?), Operation::Rotate { expected_revision: a.revision()?, pages, degrees: a.int("degrees")? }))
        },
    },
    Spec {
        name: "edit_undo",
        operation: OperationId::EditUndo,
        title: "Undo",
        description: "Undo the last change. Needs the capability of the change it undoes.",
        args: &[("doc", "integer", true), ("revision", "integer", true)],
        read_only: false,
        destructive: false,
        build: |a| Ok((Some(a.doc()?), Operation::Undo { expected_revision: a.revision()? })),
    },
    Spec {
        name: "edit_redo",
        operation: OperationId::EditRedo,
        title: "Redo",
        description: "Redo the last undone change. Needs the capability of the change it redoes.",
        args: &[("doc", "integer", true), ("revision", "integer", true)],
        read_only: false,
        destructive: false,
        build: |a| Ok((Some(a.doc()?), Operation::Redo { expected_revision: a.revision()? })),
    },
    Spec {
        name: "edit_history",
        operation: OperationId::EditHistory,
        title: "What undo and redo would do",
        description: "The next undo and redo steps and whether anything is unsaved.",
        args: &[("doc", "integer", true)],
        read_only: true,
        destructive: false,
        build: |a| Ok((Some(a.doc()?), Operation::History)),
    },
    Spec {
        name: "doc_save",
        operation: OperationId::DocumentSave,
        title: "Save",
        description: "Write the document to its file, or to `path`. It replaces only the version of the file this session read or wrote: \
`replace_version` names another (a 64-digit fingerprint you have inspected), `create_only` requires that no file exists.",
        args: &[
            ("doc", "integer", true),
            ("revision", "integer", true),
            ("path", "string", false),
            ("replace_version", "string", false),
            ("create_only", "boolean", false),
            ("full", "boolean", false),
        ],
        read_only: false,
        destructive: true,
        build: |a| {
            let overwrite = match (a.opt_string("replace_version")?, a.opt_bool("create_only")?.unwrap_or(false)) {
                (Some(_), true) => return Err("replace_version and create_only cannot be used together".into()),
                (Some(fingerprint), false) => Overwrite::Version(fingerprint),
                (None, true) => Overwrite::Absent,
                (None, false) => Overwrite::Tracked,
            };
            let target = match a.opt_string("path")? {
                Some(p) => SaveTarget::Path(PathBuf::from(p)),
                None => SaveTarget::Tracked,
            };
            Ok((Some(a.doc()?), Operation::Save { expected_revision: a.revision()?, target, overwrite, full: a.opt_bool("full")? }))
        },
    },
    Spec {
        name: "doc_close",
        operation: OperationId::DocumentClose,
        title: "Close a document",
        description: "Close a document. It refuses to drop unsaved changes unless `discard_changes` is true.",
        args: &[("doc", "integer", true), ("revision", "integer", true), ("discard_changes", "boolean", false)],
        read_only: false,
        destructive: true,
        build: |a| {
            Ok((
                Some(a.doc()?),
                Operation::Close { expected_revision: a.revision()?, discard_changes: a.opt_bool("discard_changes")?.unwrap_or(false) },
            ))
        },
    },
    Spec {
        name: "doc_list",
        operation: OperationId::DocumentList,
        title: "List open documents",
        description: "The open documents this client may read, each with its id, pages and whether it can be edited.",
        args: &[],
        read_only: true,
        destructive: false,
        build: |_| Ok((None, Operation::List)),
    },
    Spec {
        name: "link_list",
        operation: OperationId::DocumentLinks,
        title: "List links",
        description: "Every link: page, 1-based index, rect (points from the top-left of the page), and where it goes (url, to_page or other).",
        args: &[("doc", "integer", true)],
        read_only: true,
        destructive: false,
        build: |a| Ok((Some(a.doc()?), Operation::Links)),
    },
    Spec {
        name: "bookmark_list",
        operation: OperationId::DocumentBookmarks,
        title: "List bookmarks",
        description: "One page of the bookmarks, nested as the outline is. `next` is the offset of the following page, when there is one.",
        args: &[("doc", "integer", true), ("offset", "integer", false), ("limit", "integer", false)],
        read_only: true,
        destructive: false,
        build: |a| {
            let offset = a.opt_uint("offset")?.unwrap_or(0);
            let limit = a.opt_uint("limit")?.unwrap_or(DEFAULT_BOOKMARK_PAGE as u64);
            // A value too large for this machine is as out of range as one the service would refuse.
            let offset = usize::try_from(offset).map_err(|_| "offset is too large".to_string())?;
            let limit = usize::try_from(limit).unwrap_or(usize::MAX);
            Ok((Some(a.doc()?), Operation::Bookmarks { offset, limit }))
        },
    },
    Spec {
        name: "text_extract",
        operation: OperationId::DocumentText,
        title: "Extract text",
        description: "The text of the listed pages, or all, in reading order. With rect (left, top, right, bottom in points from the top-left of the page), only the text inside it, one row per visual line. A long answer stops at a page boundary and says truncated and how many pages it left out.",
        args: &[("doc", "integer", true), ("pages", "array", false), ("rect", "array", false)],
        read_only: true,
        destructive: false,
        build: |a| {
            let pages = match a.opt_int_list("pages")? {
                Some(list) => PageSelection::Numbers(list),
                None => PageSelection::All,
            };
            Ok((Some(a.doc()?), Operation::TextExtract { pages, rect: a.opt_rect("rect")? }))
        },
    },
    Spec {
        name: "text_find",
        operation: OperationId::DocumentFind,
        title: "Find text",
        description: "Find a phrase (case-insensitive, whitespace-normalised): matches in page order with page, text and line rectangles in points (origin top-left). `next` ({start_page, skip}) says where to go on when the search stopped before the end; complete is false then, and after a page that could not be read (warnings).",
        args: &[
            ("doc", "integer", true),
            ("query", "string", true),
            ("limit", "integer", false),
            ("start_page", "integer", false),
            ("skip", "integer", false),
        ],
        read_only: true,
        destructive: false,
        build: |a| {
            let limit = usize::try_from(a.opt_uint("limit")?.unwrap_or(DEFAULT_FIND_MATCHES as u64)).unwrap_or(usize::MAX);
            let skip = usize::try_from(a.opt_uint("skip")?.unwrap_or(0)).unwrap_or(usize::MAX);
            let start_page = a.opt_int("start_page")?.unwrap_or(1);
            Ok((Some(a.doc()?), Operation::TextFind { query: a.string("query")?, limit, start_page, skip }))
        },
    },
];

fn document_tools() -> Vec<Value> {
    SPECS.iter().map(|s| tool(s.name, s.title, s.description, s.args, s.read_only, s.destructive)).collect()
}

fn tool(name: &str, title: &str, description: &str, args: &[(&str, &str, bool)], read_only: bool, destructive: bool) -> Value {
    let mut properties = Map::new();
    let mut required = Vec::new();
    for (key, kind, needed) in args {
        let property = if *kind == "array" { json!({ "type": "array", "items": { "type": "integer" } }) } else { json!({ "type": kind }) };
        properties.insert((*key).to_owned(), property);
        if *needed {
            required.push(json!(key));
        }
    }
    json!({
        "name": name,
        "title": title,
        "description": description,
        "inputSchema": { "type": "object", "properties": properties, "required": required, "additionalProperties": false },
        "annotations": { "title": title, "readOnlyHint": read_only, "destructiveHint": destructive, "idempotentHint": read_only, "openWorldHint": false },
    })
}

/// A tool's arguments, read strictly: a value of the wrong type is an error, never coerced.
struct Args<'a> {
    map: &'a Map<String, Value>,
    session: pdfcraft_app_service::SessionId,
}

impl Args<'_> {
    fn get(&self, key: &str) -> Option<&Value> {
        self.map.get(key).filter(|v| !v.is_null())
    }

    fn string(&self, key: &str) -> Result<String, String> {
        self.opt_string(key)?.ok_or_else(|| format!("{key} is required"))
    }

    fn opt_string(&self, key: &str) -> Result<Option<String>, String> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(format!("{key} must be a string")),
        }
    }

    fn opt_bool(&self, key: &str) -> Result<Option<bool>, String> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::Bool(b)) => Ok(Some(*b)),
            Some(_) => Err(format!("{key} must be true or false")),
        }
    }

    fn int(&self, key: &str) -> Result<i64, String> {
        match self.get(key) {
            None => Err(format!("{key} is required")),
            Some(v) => v.as_i64().ok_or_else(|| format!("{key} must be a whole number")),
        }
    }

    fn uint(&self, key: &str) -> Result<u64, String> {
        match self.get(key) {
            None => Err(format!("{key} is required")),
            Some(v) => v.as_u64().ok_or_else(|| format!("{key} must be a whole number, 0 or more")),
        }
    }

    fn opt_uint(&self, key: &str) -> Result<Option<u64>, String> {
        match self.get(key) {
            None => Ok(None),
            Some(v) => v.as_u64().map(Some).ok_or_else(|| format!("{key} must be a whole number, 0 or more")),
        }
    }

    fn opt_int(&self, key: &str) -> Result<Option<i64>, String> {
        match self.get(key) {
            None => Ok(None),
            Some(v) => v.as_i64().map(Some).ok_or_else(|| format!("{key} must be a whole number")),
        }
    }

    fn opt_rect(&self, key: &str) -> Result<Option<[f64; 4]>, String> {
        let wrong = || format!("{key} must be four numbers: left, top, right, bottom");
        match self.get(key) {
            None => Ok(None),
            Some(Value::Array(items)) if items.len() == 4 => {
                let mut rect = [0.0; 4];
                for (slot, item) in rect.iter_mut().zip(items) {
                    *slot = item.as_f64().ok_or_else(wrong)?;
                }
                Ok(Some(rect))
            }
            Some(_) => Err(wrong()),
        }
    }

    fn opt_int_list(&self, key: &str) -> Result<Option<Vec<i64>>, String> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::Array(items)) => items
                .iter()
                .map(|v| v.as_i64().ok_or_else(|| format!("{key} must be a list of whole numbers")))
                .collect::<Result<Vec<_>, _>>()
                .map(Some),
            Some(_) => Err(format!("{key} must be a list of page numbers")),
        }
    }

    fn doc(&self) -> Result<DocumentKey, String> {
        Ok(DocumentKey { session: self.session, document: self.uint("doc")? })
    }

    fn revision(&self) -> Result<u64, String> {
        self.uint("revision")
    }
}

// ---- results ------------------------------------------------------------------------------------------------------

fn result_text(body: &Value, is_error: bool) -> Value {
    let text = serde_json::to_string_pretty(body).unwrap_or_default();
    json!({ "content": [{ "type": "text", "text": text }], "structuredContent": body, "isError": is_error })
}

/// A refusal that did not reach the service: the request was not well formed, or there is no grant.
fn refusal(code: &str, message: &str) -> Value {
    result_text(&json!({ "error": { "code": code, "message": message, "retryable": false } }), true)
}

fn failure(e: &OperationError) -> Value {
    let mut body = json!({ "code": e.code.as_str(), "message": e.message, "retryable": e.retryable });
    if let Some(reason) = e.reason {
        body["reason"] = json!(reason);
    }
    if let Some(conflict) = &e.conflict {
        body["conflict"] = conflict_json(conflict);
    }
    if !e.committed.is_empty() {
        body["committed"] = json!(e.committed.iter().map(effect_json).collect::<Vec<_>>());
    }
    result_text(&json!({ "error": body }), true)
}

fn conflict_json(c: &ConflictKind) -> Value {
    match c {
        ConflictKind::StaleRevision { expected, current } => json!({ "kind": "stale_revision", "expected": expected, "current": current }),
        ConflictKind::FileExists { disk } => json!({ "kind": "file_exists", "disk": disk }),
        ConflictKind::FileChanged { disk } => json!({ "kind": "file_changed", "disk": disk }),
        ConflictKind::FileGone => json!({ "kind": "file_gone" }),
        ConflictKind::UnsavedChanges => json!({ "kind": "unsaved_changes" }),
    }
}

fn effect_json(e: &Effect) -> Value {
    match e {
        Effect::FileWritten { file, bytes, fingerprint } => {
            json!({ "effect": "file_written", "file": file, "bytes": bytes, "fingerprint": fingerprint })
        }
        Effect::DocumentClosed => json!({ "effect": "document_closed" }),
    }
}

fn success(result: &OperationResult) -> Value {
    let mut body = match &result.outcome {
        Outcome::Opened { info, .. } | Outcome::Info(info) => info_json(info),
        Outcome::Changed { label, pages_affected } => json!({ "changed": label, "pages_affected": pages_affected }),
        Outcome::History(h) => history_json(h),
        Outcome::Saved(report) => saved_json(report),
        Outcome::Closed => json!({ "closed": true }),
        Outcome::Documents(documents) => json!({ "documents": documents.iter().map(info_json).collect::<Vec<_>>() }),
        Outcome::Links(list) => links_json(list),
        Outcome::Bookmarks(list) => bookmarks_json(list),
        Outcome::Text(text) => text_json(text),
        Outcome::Found(report) => found_json(report),
    };
    if let Some(version) = &result.version {
        body["revision"] = json!(version.content_revision);
    }
    if !result.warnings.is_empty() {
        body["warnings"] = json!(result.warnings);
    }
    if !result.effects.is_empty() {
        body["effects"] = json!(result.effects.iter().map(effect_json).collect::<Vec<_>>());
    }
    result_text(&body, false)
}

fn info_json(info: &DocumentInfo) -> Value {
    json!({
        "doc": info.document.document,
        "name": info.name,
        "pages": info.pages.iter().map(|p| json!({ "number": p.number, "width": p.width, "height": p.height, "rotation": p.rotation })).collect::<Vec<_>>(),
        "editable": info.editable,
        "read_only_reason": info.read_only_reason,
        "signed": info.signed,
        "encrypted": info.encrypted,
        "has_file": info.has_file,
    })
}

fn links_json(list: &LinkList) -> Value {
    let links: Vec<Value> = list
        .links
        .iter()
        .map(|l| {
            let mut link = json!({ "page": l.page, "index": l.index, "rect": l.rect, "visible": l.visible });
            match &l.target {
                LinkTarget::Page(page) => link["to_page"] = json!(page),
                LinkTarget::Uri(url) => link["url"] = json!(url),
                LinkTarget::Other(other) => link["other"] = json!(other),
            }
            link
        })
        .collect();
    let mut body = json!({ "count": links.len(), "links": links });
    if list.total > links.len() {
        body["total"] = json!(list.total);
        body["truncated"] = json!(true);
    }
    body
}

fn bookmarks_json(list: &BookmarkList) -> Value {
    let rows = list.rows.iter().map(|r| BookmarkView { path: r.path.clone(), title: &r.title, page: r.page, open: r.open });
    json!({ "bookmarks": nest_rows(rows), "next": list.next, "truncated": list.truncated })
}

fn text_json(text: &TextPages) -> Value {
    let pages: Vec<Value> = text.pages.iter().map(|p| json!({ "page": p.page, "text": p.text })).collect();
    let mut body = json!({ "pages": pages });
    if text.omitted_pages > 0 {
        body["truncated"] = json!(true);
        body["omitted_pages"] = json!(text.omitted_pages);
    }
    body
}

fn found_json(report: &FindReport) -> Value {
    let matches: Vec<Value> = report.matches.iter().map(|m| json!({ "page": m.page, "text": m.text, "rects": m.rects })).collect();
    let warnings: Vec<Value> =
        report.warnings.iter().map(|w| json!({ "page": w.page, "error": w.error, "error_truncated": w.error_truncated })).collect();
    let mut body = json!({
        "query": report.query,
        "count": matches.len(),
        "complete": report.next.is_none() && report.warning_count == 0,
        "matches": matches,
        "warnings_truncated": report.warning_count > report.warnings.len(),
        "warning_count": report.warning_count,
        "warnings": warnings,
    });
    if let Some(next) = report.next {
        body["next"] = json!({ "start_page": next.start_page, "skip": next.skip });
    }
    body
}

fn history_json(h: &History) -> Value {
    json!({ "next_undo": h.next_undo, "next_redo": h.next_redo, "unsaved": h.unsaved })
}

fn saved_json(r: &SaveReport) -> Value {
    json!({
        "saved": r.file,
        "bytes": r.bytes,
        "incremental": r.incremental,
        "fingerprint": r.fingerprint,
        "edited_since": r.edited_since,
        "superseded": r.superseded,
        "document_open": r.document_open,
    })
}
