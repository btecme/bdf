# pdfcraft-app-service

The application service behind every PdfCraft surface. The desktop, the command line and the MCP server are meant to be adapters over this crate: they turn their own shapes (widgets, arguments, JSON-RPC) into the typed requests below and the results and errors back out, and none of them keeps its own document session, save logic or policy. This is BUILD-007 of the BDF plan ([work items](../../docs/bdf-work-items.toml), finding F08).

- **Layer:** L7, headless. It depends on `engine` and `platform`, never on a UI toolkit, and never on `automation`. The only same-layer edges `xtask layers` allows into or out of it are `app-service → platform`, `automation → app-service` and `ui-egui → app-service`.
- **Status:** the first slice, used by tests only. Nothing in the desktop, the CLI or the MCP server calls it yet; moving them over is BUILD-009. **It authorizes nothing**: every caller is the local user who started the process until BUILD-008 adds identities, grants and audit.

## Contract

```rust
let mut service = Service::new();                       // or Service::confined_to(root)
let ctx = RequestContext::local(1);
let opened = service.execute(&ctx, OperationRequest::new(None, Operation::Open { source, password: None }))?;
// opened.outcome == Outcome::Opened { document: DocumentKey, info }, opened.version == Some(DocumentVersion)
service.execute(&ctx, OperationRequest::new(Some(key), Operation::Rotate {
    expected_revision: version.content_revision, pages: PageSelection::Numbers(vec![1]), degrees: 90 }))?;
```

* **Operations** ([`operations.rs`](src/operations.rs)): `document.open`, `document.query`, `page.rotate`, `edit.undo`, `edit.redo`, `edit.history`, `document.save`, `document.close`. Each has a descriptor (stable id, summary, capability, access class, release status, whether it needs a document and a revision). Tool definitions and UI command availability are meant to derive from the descriptors; the service never depends on an adapter's tool type.
* **The session is private.** `Service` owns the engine session and exposes no `&mut Session`. Callers get `DocumentView`, which can only read. Every change passes `Service::execute`, which checks, in this order and leaving everything as it was on failure: the schema version, the document (`(session, number)`, never the number alone), the revision of an operation that changes or discards content, the arguments, and then the operation.
* **Revisions.** A document has three things that used to be one: the *content revision* (moves with edit, undo, redo, revert and reload and with nothing else; no value repeats), the *render generation* (cache invalidation, moves for saves and display changes too) and the *saved snapshot* (an identity of content, so that undoing back to the saved content is clean again even though the revision moved on). `dirty` is derived from the snapshot, never from comparing revisions. An operation that changes or discards content, or writes a file, carries the revision it was made against as a field of its typed request, so it cannot be built without one; a stale one is a `conflict` and changes nothing.
* **Errors** ([`errors.rs`](src/errors.rs)): a closed set of codes (`invalid_argument`, `not_authorized`, `not_found`, `unsupported_feature`, `password_required`, `wrong_password`, `conflict`, `resource_limit`, `canceled`, `io_failure`, `worker_failure`, `internal_failure`, plus `unsupported_schema`, added because a client has to tell "upgrade me" from "your request is wrong"). A failure that stops before any effect has an empty `committed` list; one that happens after a file was written lists the write, and its message says so.
* **Schema.** `SCHEMA_VERSION` is 1 and `OLDEST_SCHEMA` is 1. A request outside the range is refused first, with the supported range in the message. There is nothing to migrate yet; the range is where migrations will be declared.
* **Pages** are 1-based at the boundary and converted to the engine's 0-based indices once, in the service. Messages for the shared failures match the automation tools' (`page numbers start at 1 (got 0)`, `page 3 is out of range: the document has 2 pages`, `degrees must be a multiple of 90`); `crates/automation/tests/service_parity.rs` holds the two to the same wording and the same result.
* **Saving** is two halves so the file I/O can run elsewhere: `prepare_save` takes the bytes of one revision and works out what the target may replace (`CreateNew`, the tracked version, or a confirmed fingerprint; never "whatever is there"), `PreparedSave::write` writes with the check made at the moment of replacement, and `finish_save` publishes. A result that arrives for a document that has been edited meanwhile keeps the newer edits and leaves the document unsaved; one for a document that has been closed leaves the file and nothing else; `is_current` lets any worker ask before it publishes. A save whose file was written but whose document could not be refreshed from it reports `internal_failure` with the write in `committed`.
* **Files.** `Service::confined_to(root)` accepts only root-relative paths without `..`, pins the destination folder (Windows) and revalidates it at the commit, as the platform crate does for the tools. Opening reads at most `MAX_OPEN_BYTES` (checked before the read) and records the file's version for the save.

## Not here yet

Local principals, grants, release policy and audit (BUILD-008); moving the desktop, CLI and MCP onto the service and deleting their duplicate orchestration (BUILD-009); jobs, cancellation, deadlines and aggregate memory budgets (BUILD-010); absolute-path and `..` resolution against a root (still in `automation`); the other tools of the catalogue; rendering through the facade.
