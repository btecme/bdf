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
//!   there is one: `doc_open`, `doc_info`, `page_rotate`, `edit_undo`, `edit_redo`, `edit_history`, `doc_save` and
//!   `doc_close`. What the grant allows is the policy's decision, not this module's: a read-only grant lists the same
//!   tools and is refused at the call.
//! * A tool that changes or discards content names the revision it was made against (`revision`, from the last reply).
//!
//! Opt-in like the other server: it runs when a person launches it and stops when its input closes.

use std::io::{BufRead, Write};
use std::path::PathBuf;

use pdfcraft_app_service::{
    ActorContext, ConflictKind, DESCRIPTORS, DocumentInfo, DocumentKey, Effect, History, OpenSource, Operation, OperationError, OperationRequest,
    OperationResult, Outcome, Overwrite, PageSelection, RequestContext, SaveReport, SaveTarget, Service,
};
use serde_json::{Map, Value, json};

use crate::mcp::{
    DEFAULT_MAX_MESSAGE, INVALID_PARAMS, INVALID_REQUEST, METHOD_NOT_FOUND, PARSE_ERROR, PROTOCOL_VERSIONS, check_keys, error, serve_lines,
};

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
                let args = match params.get("arguments") {
                    None | Some(Value::Null) => Map::new(),
                    Some(Value::Object(o)) => o.clone(),
                    Some(_) => return Err((INVALID_PARAMS, "arguments must be an object".into())),
                };
                self.call(name, &args)
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

    fn call(&mut self, name: &str, args: &Map<String, Value>) -> Result<Value, (i64, String)> {
        if name == HELP {
            check_keys(name, &Value::Object(args.clone()), &[]).map_err(|e| (INVALID_PARAMS, e))?;
            return Ok(self.help());
        }
        let Some(spec) = SPECS.iter().find(|s| s.name == name) else {
            return Err((INVALID_PARAMS, format!("unknown tool {name:?}")));
        };
        if self.actor.is_none() {
            return Ok(refusal("not_authorized", "this server was started without a valid grant; call bdf_help, or ask the person to issue a grant"));
        }
        let allowed: Vec<&str> = spec.args.iter().map(|a| a.0).collect();
        check_keys(name, &Value::Object(args.clone()), &allowed).map_err(|e| (INVALID_PARAMS, e))?;
        let (document, operation) = match (spec.build)(&Args { map: args, session: self.service.session_id() }) {
            Ok(pair) => pair,
            Err(message) => return Ok(refusal("invalid_argument", &message)),
        };
        Ok(self.run(document, operation))
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
    title: &'static str,
    description: &'static str,
    /// Argument name, JSON type, and whether it is required.
    args: &'static [(&'static str, &'static str, bool)],
    read_only: bool,
    destructive: bool,
    build: Build,
}

static SPECS: [Spec; 8] = [
    Spec {
        name: "doc_open",
        title: "Open a PDF",
        description: "Open a PDF file and return its document id, revision, pages and whether it can be edited.",
        args: &[("path", "string", true), ("password", "string", false)],
        read_only: true,
        destructive: false,
        build: |a| Ok((None, Operation::Open { source: OpenSource::File(PathBuf::from(a.string("path")?)), password: a.opt_string("password")? })),
    },
    Spec {
        name: "doc_info",
        title: "Describe a document",
        description: "Pages (1-based, as displayed), whether the document can be edited and its current revision.",
        args: &[("doc", "integer", true)],
        read_only: true,
        destructive: false,
        build: |a| Ok((Some(a.doc()?), Operation::Query)),
    },
    Spec {
        name: "page_rotate",
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
        title: "Undo",
        description: "Undo the last change. Needs the capability of the change it undoes.",
        args: &[("doc", "integer", true), ("revision", "integer", true)],
        read_only: false,
        destructive: false,
        build: |a| Ok((Some(a.doc()?), Operation::Undo { expected_revision: a.revision()? })),
    },
    Spec {
        name: "edit_redo",
        title: "Redo",
        description: "Redo the last undone change. Needs the capability of the change it redoes.",
        args: &[("doc", "integer", true), ("revision", "integer", true)],
        read_only: false,
        destructive: false,
        build: |a| Ok((Some(a.doc()?), Operation::Redo { expected_revision: a.revision()? })),
    },
    Spec {
        name: "edit_history",
        title: "What undo and redo would do",
        description: "The next undo and redo steps and whether anything is unsaved.",
        args: &[("doc", "integer", true)],
        read_only: true,
        destructive: false,
        build: |a| Ok((Some(a.doc()?), Operation::History)),
    },
    Spec {
        name: "doc_save",
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
