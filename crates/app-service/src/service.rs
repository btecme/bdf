//! The service: privately owned document sessions behind typed operations.

use std::collections::HashMap;
use std::hash::{BuildHasher, Hasher};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use pdfcraft_engine::{DocId, Document, Edit, EditError, OpenError, SaveToken, Saved, Session};
use pdfcraft_platform::confinement::{pin_existing, write_confined_with};
use pdfcraft_platform::file_identity::{
    Authority, Conflict, FileVersion, conflict_of, parse_fingerprint, read_regular_with_version, read_with_version_limited, write_guarded,
};
use pdfcraft_platform::persistence::{PersistError, Steps};
use pdfcraft_platform::staging::{StagingName, staging_suffixes};

use crate::effects::Effect;
use crate::errors::{ConflictKind, ErrorCode, OperationError, Result};
use crate::operations::{OLDEST_SCHEMA, OpenSource, Operation, OperationId, OperationRequest, Overwrite, PageSelection, SCHEMA_VERSION, SaveTarget};
use crate::results::{DocumentInfo, History, OperationResult, Outcome, PageSummary, SaveReport};
use crate::version::{DocumentKey, DocumentVersion, SavedSnapshot, SessionId};

/// The largest file `document.open` reads. An interim admission limit, checked before the file is read; the budgets for
/// a whole session belong to the work on bounded jobs.
pub const MAX_OPEN_BYTES: u64 = 512 * 1024 * 1024;

/// How many planned saves may be waiting to be finished at once. A plan that is never finished (its worker died)
/// would otherwise stay registered for ever; [`Service::abandon_save`] drops one, and this bounds the rest.
pub const MAX_SAVES_IN_FLIGHT: usize = 16;

/// What a request is made on behalf of. Every caller is the local user who started this process until the policy
/// layer adds identities and grants; nothing here authorizes anything, and the request id only correlates.
#[derive(Clone, Debug)]
pub struct RequestContext {
    pub request_id: u64,
}

impl RequestContext {
    pub fn local(request_id: u64) -> Self {
        Self { request_id }
    }
}

/// The schema versions this service accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SchemaRange {
    pub oldest: u32,
    pub newest: u32,
}

/// The one owner of the documents of a session. The engine session is private: nothing outside this crate can reach a
/// mutable document, so every change passes the checks in [`Service::execute`].
pub struct Service {
    session: Session,
    id: SessionId,
    /// The version of each document's file as this session last read or wrote it, which is all a save may replace.
    files: HashMap<u64, FileVersion>,
    /// When set, every file the service reads or writes is a relative path below this folder.
    root: Option<PathBuf>,
    /// Saves that were planned and not yet finished or abandoned, by plan number, with the document each is for.
    pending: HashMap<u64, u64>,
    /// The order saves of one document were planned in, and the newest one whose result has been applied.
    orders: HashMap<u64, SaveOrder>,
    /// The last plan number handed out.
    last_plan: u64,
    /// The largest file an open reads (at most [`MAX_OPEN_BYTES`]).
    open_limit: u64,
}

#[derive(Clone, Copy, Debug, Default)]
struct SaveOrder {
    issued: u64,
    applied: u64,
}

impl Default for Service {
    fn default() -> Self {
        Self::new()
    }
}

impl Service {
    pub fn new() -> Self {
        Self {
            session: Session::new(),
            id: SessionId(random_id()),
            files: HashMap::new(),
            root: None,
            pending: HashMap::new(),
            orders: HashMap::new(),
            last_plan: 0,
            open_limit: MAX_OPEN_BYTES,
        }
    }

    /// A service whose files all lie below `root`: paths must be relative to it and may not climb out, the folder of
    /// each save is pinned and revalidated at the commit, and a link on the way is refused. (Resolving absolute paths
    /// and `..` against a root stays with the adapters until they move onto this service.)
    pub fn confined_to(root: &Path) -> Result<Self> {
        let root = root.canonicalize().map_err(|e| OperationError::new(ErrorCode::IoFailure, format!("{}: {e}", root.display())))?;
        if !root.is_dir() {
            return Err(OperationError::invalid(format!("{}: the root must be a folder", root.display())));
        }
        Ok(Self { root: Some(root), ..Self::new() })
    }

    /// Read no more than `bytes` from a file when opening (never more than [`MAX_OPEN_BYTES`]).
    pub fn with_open_limit(mut self, bytes: u64) -> Self {
        self.open_limit = bytes.min(MAX_OPEN_BYTES);
        self
    }

    /// Fix the clock the engine stamps edits with (deterministic tests).
    pub fn with_clock(mut self, clock: fn() -> i64) -> Self {
        self.session = std::mem::take(&mut self.session).with_clock(clock);
        self
    }

    pub fn session_id(&self) -> SessionId {
        self.id
    }

    pub fn schema(&self) -> SchemaRange {
        SchemaRange { oldest: OLDEST_SCHEMA, newest: SCHEMA_VERSION }
    }

    /// The open documents.
    pub fn documents(&self) -> Vec<DocumentKey> {
        self.session.docs().iter().map(|d| self.key_of(d.id)).collect()
    }

    /// A read-only view of one document. It cannot change it.
    pub fn document(&self, key: DocumentKey) -> Result<DocumentView<'_>> {
        let doc = self.get(key)?;
        Ok(DocumentView { service: self, doc })
    }

    /// Whether `key` is still open and still at `revision`: what a worker that finished late asks before it publishes
    /// anything, so a result for a document that has since changed or closed is dropped instead of applied.
    pub fn is_current(&self, key: DocumentKey, revision: u64) -> bool {
        self.get(key).is_ok_and(|d| d.content_revision() == revision)
    }

    /// Run one operation.
    ///
    /// The checks come in a fixed order and each one that fails leaves the bytes, the history and the files exactly as
    /// they were: the schema version, the document, the revision of an operation that changes or discards content, the
    /// arguments, and only then the operation.
    pub fn execute(&mut self, _context: &RequestContext, request: OperationRequest) -> Result<OperationResult> {
        negotiate(request.schema_version)?;
        let id = request.operation.id();
        let descriptor = id.descriptor();
        let doc_id = match (descriptor.needs_document, request.document) {
            (false, _) => None,
            (true, None) => return Err(OperationError::invalid(format!("{} needs a document", descriptor.name))),
            (true, Some(key)) => Some(self.get(key)?.id),
        };
        if let (Some(doc_id), Some(expected)) = (doc_id, request.operation.expected_revision()) {
            self.check_revision(doc_id, expected)?;
        }
        match request.operation {
            Operation::Open { source, password } => self.open(source, password.as_deref()),
            Operation::Query => self.query(self.need(doc_id)?),
            Operation::Rotate { pages, degrees, .. } => self.rotate(self.need(doc_id)?, pages, degrees),
            Operation::Undo { .. } => self.step(self.need(doc_id)?, Session::undo),
            Operation::Redo { .. } => self.step(self.need(doc_id)?, Session::redo),
            Operation::History => self.history(self.need(doc_id)?),
            Operation::Save { expected_revision, target, overwrite, full } => {
                let key = self.key_of(self.need(doc_id)?);
                let prepared = self.prepare_save(key, expected_revision, target, overwrite, full)?;
                let written = match prepared.write() {
                    Ok(written) => written,
                    Err(e) => {
                        self.abandon_save(&prepared);
                        return Err(e);
                    }
                };
                self.finish_save(&written)
            }
            Operation::Close { discard_changes, .. } => self.close(self.need(doc_id)?, discard_changes),
        }
    }

    // ---- saving, in two halves ----

    /// First half of a save: take the bytes of the content the request was made against (`expected_revision`, checked
    /// here as well as in [`Service::execute`], so that a caller who splits the save cannot skip it), work out what the
    /// target may replace, and return a plan that can be written on any thread. Nothing is written and nothing is marked
    /// saved.
    pub fn prepare_save(
        &mut self,
        key: DocumentKey,
        expected_revision: u64,
        target: SaveTarget,
        overwrite: Overwrite,
        full: Option<bool>,
    ) -> Result<PreparedSave> {
        let doc = self.get(key)?;
        self.check_revision(doc.id, expected_revision)?;
        let path = match target {
            SaveTarget::Path(p) => self.storage_path(&p)?,
            SaveTarget::Tracked => match doc.path.as_deref() {
                Some(stored) => self.storage_path(Path::new(stored))?,
                None => return Err(OperationError::invalid("the document has never been saved: pass a path")),
            },
        };
        let same_file = doc.path.as_deref().is_some_and(|p| self.storage_path(Path::new(p)).is_ok_and(|stored| stored == path));
        let authority = match overwrite {
            Overwrite::Version(text) => {
                let hash =
                    parse_fingerprint(&text).ok_or_else(|| OperationError::invalid("the version to overwrite must be a 64-digit hex fingerprint"))?;
                OwnedAuthority::ReplaceConfirmed(hash)
            }
            Overwrite::Absent => OwnedAuthority::CreateNew,
            Overwrite::Tracked => match self.files.get(&doc.id.0).filter(|_| same_file) {
                Some(version) => OwnedAuthority::ReplaceTracked(version.clone()),
                None => OwnedAuthority::CreateNew,
            },
        };
        // Saving to a new file is a full rewrite unless asked otherwise, like Save As.
        let full = full.unwrap_or(!same_file);
        let revision = doc.content_revision();
        let token = doc.save_token();
        let bytes = if full { self.session.save_full_bytes(doc.id) } else { self.session.save_bytes(doc.id) }.map_err(edit_error)?;
        let incremental = !full && bytes.starts_with(&doc.saved_bytes());
        let (doc_id, root) = (doc.id, self.root.clone());
        if self.pending.len() >= MAX_SAVES_IN_FLIGHT {
            return Err(OperationError::new(ErrorCode::ResourceLimit, "too many saves are waiting to be finished"));
        }
        self.last_plan = self.last_plan.saturating_add(1);
        let plan = self.last_plan;
        let order = self.orders.entry(doc_id.0).or_default();
        order.issued = order.issued.saturating_add(1);
        let seq = order.issued;
        self.pending.insert(plan, doc_id.0);
        Ok(PreparedSave { service: self.id, plan, seq, key, revision, token, bytes, target: path, authority, root, incremental })
    }

    /// Give up a planned save that will not be finished (its write failed or its worker is gone).
    pub fn abandon_save(&mut self, plan: &PreparedSave) {
        if plan.service == self.id {
            self.pending.remove(&plan.plan);
        }
    }

    /// Second half: publish a write that happened.
    ///
    /// The completion carries its own plan, so it can only finish the save it belongs to. It is refused, with nothing
    /// changed here, when that plan belongs to another service or has already been finished or abandoned; the refusal
    /// still names the file the write left on disk (`committed`), because that stands.
    ///
    /// What the document learns is exactly this: the file holds the content the plan was taken from. If the document
    /// was edited, or undone, meanwhile it is unsaved (the file does not hold what it holds now), and undoing or
    /// redoing back to what was written is clean again. A document closed meanwhile is left alone and the file stands.
    /// A plan that was overtaken, because a save planned after it has already been applied, changes nothing at all: it
    /// must not give the document back an older destination, saved state or tracked file version.
    pub fn finish_save(&mut self, written: &WrittenSave) -> Result<OperationResult> {
        let prepared = &written.plan;
        let effect = written.effect();
        let refuse = |why: &str| OperationError { committed: vec![effect.clone()], ..OperationError::invalid(why) };
        if prepared.service != self.id {
            return Err(refuse("this save was planned by another service"));
        }
        let Some(doc_number) = self.pending.remove(&prepared.plan) else {
            return Err(refuse("this save was already finished or abandoned"));
        };
        let file = file_name(&prepared.target);
        let mut report = SaveReport {
            file,
            bytes: written.version.len,
            incremental: prepared.incremental,
            fingerprint: written.version.fingerprint(),
            edited_since: false,
            superseded: false,
            document_open: false,
        };
        let key = prepared.key;
        let Ok(doc_id) = self.get(key).map(|d| d.id) else {
            // Closed (or never ours): the file stands, nothing else is touched.
            self.orders.remove(&doc_number);
            return Ok(OperationResult { outcome: Outcome::Saved(report), version: None, warnings: Vec::new(), effects: vec![effect] });
        };
        report.document_open = true;
        let applied = self.orders.get(&doc_number).map_or(0, |o| o.applied);
        if prepared.seq < applied {
            // A later save of this document has been applied already. This one wrote its file, and that is all it did.
            report.superseded = true;
            let version = self.get(key).ok().map(version_of);
            return Ok(OperationResult { outcome: Outcome::Saved(report), version, warnings: Vec::new(), effects: vec![effect] });
        }
        self.orders.entry(doc_number).or_default().applied = prepared.seq;
        self.files.insert(doc_id.0, written.version.clone());
        let path = prepared.target.to_string_lossy().into_owned();
        match self.session.mark_saved_at(doc_id, prepared.bytes.clone(), Some(path), prepared.token) {
            Ok(Saved::Rebased) => {}
            Ok(Saved::EditedSince) => report.edited_since = true,
            Err(e) => {
                let mut error = edit_error(e);
                error.code = ErrorCode::InternalFailure;
                error.message = format!(
                    "the file was written, but the document could not be refreshed from it ({}); your changes are still open and unsaved, and the file on disk holds the saved version",
                    error.message
                );
                error.committed = vec![effect];
                return Err(error);
            }
        }
        let version = self.get(key).ok().map(version_of);
        Ok(OperationResult { outcome: Outcome::Saved(report), version, warnings: Vec::new(), effects: vec![effect] })
    }

    // ---- the operations ----

    fn open(&mut self, source: OpenSource, password: Option<&str>) -> Result<OperationResult> {
        let (name, path, bytes, version) = match source {
            OpenSource::Bytes { name, bytes } => (name, None, bytes, None),
            OpenSource::File(requested) => {
                let path = self.storage_path(&requested)?;
                // The limit is applied to the handle that is read. Confined: the file itself may not be a link either, which
                // would carry the read out of the root.
                let read = |p: &Path| -> std::io::Result<(Vec<u8>, FileVersion)> {
                    if self.root.is_some() { read_regular_with_version(p, self.open_limit) } else { read_with_version_limited(p, self.open_limit) }
                };
                let pinned = match (&self.root, path.parent()) {
                    (Some(root), Some(dir)) => Some(pin_existing(root, dir).map_err(|e| io_error(&path, &e))?),
                    _ => None,
                };
                let (bytes, version) = read(&path).map_err(|e| io_error(&path, &e))?;
                drop(pinned);
                (file_name(&path), Some(path.to_string_lossy().into_owned()), Arc::new(bytes), Some(version))
            }
        };
        let doc_id = self.session.open(name, path, bytes, password).map_err(open_error)?;
        if let Some(version) = version {
            self.files.insert(doc_id.0, version);
        }
        let doc = self.session.get(doc_id).ok_or_else(|| OperationError::internal("the opened document is missing"))?;
        let key = self.key_of(doc_id);
        Ok(OperationResult {
            outcome: Outcome::Opened { document: key, info: info_of(key, doc) },
            version: Some(version_of(doc)),
            warnings: doc.info.warnings.clone(),
            effects: Vec::new(),
        })
    }

    fn query(&self, id: DocId) -> Result<OperationResult> {
        let doc = self.session.get(id).ok_or_else(OperationError::not_found)?;
        Ok(OperationResult {
            outcome: Outcome::Info(info_of(self.key_of(id), doc)),
            version: Some(version_of(doc)),
            warnings: Vec::new(),
            effects: Vec::new(),
        })
    }

    fn rotate(&mut self, id: DocId, pages: PageSelection, degrees: i64) -> Result<OperationResult> {
        if degrees % 90 != 0 {
            return Err(OperationError::invalid("degrees must be a multiple of 90"));
        }
        let count = self.session.get(id).ok_or_else(OperationError::not_found)?.info.pages.len();
        let indices = match pages {
            PageSelection::All if count == 0 => return Err(OperationError::invalid("the document has no pages")),
            PageSelection::All => (0..count).collect(),
            PageSelection::Numbers(list) if list.is_empty() => return Err(OperationError::invalid("pages must list at least one page")),
            PageSelection::Numbers(list) => list.iter().map(|p| zero_based(*p, count)).collect::<Result<Vec<_>>>()?,
        };
        let affected = indices.len();
        self.session.apply(id, Edit::RotatePages { pages: indices, degrees }).map_err(edit_error)?;
        self.changed(id, Some(affected))
    }

    fn step(&mut self, id: DocId, run: fn(&mut Session, DocId) -> std::result::Result<pdfcraft_engine::Label, EditError>) -> Result<OperationResult> {
        let label = run(&mut self.session, id).map_err(edit_error)?;
        let doc = self.session.get(id).ok_or_else(OperationError::not_found)?;
        Ok(OperationResult {
            outcome: Outcome::Changed { label: label.to_string(), pages_affected: None },
            version: Some(version_of(doc)),
            warnings: Vec::new(),
            effects: Vec::new(),
        })
    }

    fn changed(&self, id: DocId, affected: Option<usize>) -> Result<OperationResult> {
        let doc = self.session.get(id).ok_or_else(OperationError::not_found)?;
        let label = doc.undo_label().map(ToString::to_string).unwrap_or_default();
        Ok(OperationResult {
            outcome: Outcome::Changed { label, pages_affected: affected },
            version: Some(version_of(doc)),
            warnings: Vec::new(),
            effects: Vec::new(),
        })
    }

    fn history(&self, id: DocId) -> Result<OperationResult> {
        let doc = self.session.get(id).ok_or_else(OperationError::not_found)?;
        let version = version_of(doc);
        let history = History {
            next_undo: doc.undo_label().map(ToString::to_string),
            next_redo: doc.redo_label().map(ToString::to_string),
            unsaved: version.dirty(),
        };
        Ok(OperationResult { outcome: Outcome::History(history), version: Some(version), warnings: Vec::new(), effects: Vec::new() })
    }

    fn close(&mut self, id: DocId, discard_changes: bool) -> Result<OperationResult> {
        let doc = self.session.get(id).ok_or_else(OperationError::not_found)?;
        if version_of(doc).dirty() && !discard_changes {
            return Err(OperationError::conflict(
                ConflictKind::UnsavedChanges,
                "the document has unsaved changes: save it, or close it with discard_changes to drop them",
            ));
        }
        self.session.close(id);
        self.files.remove(&id.0);
        self.orders.remove(&id.0);
        Ok(OperationResult { outcome: Outcome::Closed, version: None, warnings: Vec::new(), effects: vec![Effect::DocumentClosed] })
    }

    // ---- helpers ----

    fn key_of(&self, id: DocId) -> DocumentKey {
        DocumentKey { session: self.id, document: id.0 }
    }

    fn get(&self, key: DocumentKey) -> Result<&Document> {
        if key.session != self.id {
            return Err(OperationError::not_found());
        }
        self.session.get(DocId(key.document)).ok_or_else(OperationError::not_found)
    }

    fn need(&self, id: Option<DocId>) -> Result<DocId> {
        id.ok_or_else(|| OperationError::internal("an operation that needs a document reached its handler without one"))
    }

    fn check_revision(&self, id: DocId, expected: u64) -> Result<()> {
        let current = self.session.get(id).ok_or_else(OperationError::not_found)?.content_revision();
        if current == expected {
            return Ok(());
        }
        Err(OperationError::conflict(
            ConflictKind::StaleRevision { expected, current },
            format!(
                "the document changed since revision {expected} and is now at revision {current}: read it again and repeat the request against the current revision"
            ),
        ))
    }

    /// A path as the service will touch it. Without a root it is the path given. With one it must be below the root,
    /// written relative to it, with no `..` and no drive or root of its own.
    fn storage_path(&self, path: &Path) -> Result<PathBuf> {
        let Some(root) = &self.root else { return Ok(path.to_path_buf()) };
        let outside = || OperationError::invalid(format!("{} is outside the allowed directory {}", path.display(), root.display()));
        // A path the service itself stored (the document's own) is already below the root.
        let relative = path.strip_prefix(root).unwrap_or(path);
        if relative.as_os_str().is_empty() || !relative.components().all(|c| matches!(c, Component::Normal(_))) {
            return Err(outside());
        }
        Ok(root.join(relative))
    }
}

/// A save that has been planned and can be written without the service: the bytes of one revision, the file they go to
/// and what they may replace. It holds no reference to the service, so a worker can own it.
#[derive(Clone)]
pub struct PreparedSave {
    /// The service that planned it, this plan's number, and its place among the saves planned for the document.
    service: SessionId,
    plan: u64,
    seq: u64,
    key: DocumentKey,
    revision: u64,
    token: SaveToken,
    bytes: Arc<Vec<u8>>,
    target: PathBuf,
    authority: OwnedAuthority,
    root: Option<PathBuf>,
    incremental: bool,
}

// Not derived: the bytes are the whole document, and a debug print must not put its content in a log.
impl std::fmt::Debug for PreparedSave {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedSave")
            .field("key", &self.key)
            .field("revision", &self.revision)
            .field("file", &file_name(&self.target))
            .field("bytes", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

/// What a write left on disk, together with the plan that wrote it: the two cannot be separated, so a completion can only
/// finish the save it belongs to. Handed to [`Service::finish_save`].
#[derive(Debug)]
pub struct WrittenSave {
    plan: PreparedSave,
    version: FileVersion,
}

impl WrittenSave {
    pub fn document(&self) -> DocumentKey {
        self.plan.key
    }

    /// The effect the write had, which stands whatever happens to the completion.
    fn effect(&self) -> Effect {
        Effect::FileWritten { file: file_name(&self.plan.target), bytes: self.version.len, fingerprint: self.version.fingerprint() }
    }
}

impl PreparedSave {
    /// The content revision these bytes are of.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn document(&self) -> DocumentKey {
        self.key
    }

    /// Write the bytes. The target is replaced only if it is still what the plan said it may replace, checked at the
    /// moment of replacement; otherwise nothing is written and the error is a conflict that names what is there.
    pub fn write(&self) -> Result<WrittenSave> {
        if self.target.is_dir() {
            return Err(OperationError::invalid(format!("{}: is a folder, not a file", file_name(&self.target))));
        }
        let authority = self.authority.borrow();
        let result = match &self.root {
            Some(root) => {
                write_confined_with(root, &self.target, &self.bytes, StagingName::SuffixThenTag, Some(authority), staging_suffixes(), &Steps::real())
            }
            None => {
                if let Some(dir) = self.target.parent().filter(|p| !p.as_os_str().is_empty()) {
                    std::fs::create_dir_all(dir).map_err(|e| io_error(dir, &e))?;
                }
                write_guarded(&self.target, &self.bytes, StagingName::SuffixThenTag, authority)
            }
        };
        result.map_err(|e| persist_error(&self.target, &e))?;
        Ok(WrittenSave { plan: self.clone(), version: FileVersion::of_bytes(&self.bytes, None) })
    }
}

#[derive(Debug, Clone)]
enum OwnedAuthority {
    CreateNew,
    ReplaceTracked(FileVersion),
    ReplaceConfirmed([u8; 32]),
}

impl OwnedAuthority {
    fn borrow(&self) -> Authority<'_> {
        match self {
            OwnedAuthority::CreateNew => Authority::CreateNew,
            OwnedAuthority::ReplaceTracked(version) => Authority::ReplaceTracked(version),
            OwnedAuthority::ReplaceConfirmed(hash) => Authority::ReplaceConfirmed(hash),
        }
    }
}

/// A document as the rest of the program may see it: read-only, and only what the contract names.
pub struct DocumentView<'a> {
    service: &'a Service,
    doc: &'a Document,
}

impl DocumentView<'_> {
    pub fn key(&self) -> DocumentKey {
        self.service.key_of(self.doc.id)
    }

    pub fn name(&self) -> &str {
        &self.doc.name
    }

    pub fn page_count(&self) -> usize {
        self.doc.info.pages.len()
    }

    pub fn version(&self) -> DocumentVersion {
        version_of(self.doc)
    }

    pub fn info(&self) -> DocumentInfo {
        info_of(self.key(), self.doc)
    }

    pub fn next_undo(&self) -> Option<String> {
        self.doc.undo_label().map(ToString::to_string)
    }

    pub fn next_redo(&self) -> Option<String> {
        self.doc.redo_label().map(ToString::to_string)
    }
}

fn negotiate(version: u32) -> Result<()> {
    if (OLDEST_SCHEMA..=SCHEMA_VERSION).contains(&version) {
        return Ok(());
    }
    let advice = if version < OLDEST_SCHEMA {
        "this client is older than the service supports; update the client"
    } else {
        "this client is newer than the service; update PdfCraft, or ask with a supported schema"
    };
    Err(OperationError::new(
        ErrorCode::UnsupportedSchema,
        format!("schema version {version} is not supported (supported: {OLDEST_SCHEMA} to {SCHEMA_VERSION}); {advice}"),
    ))
}

fn version_of(doc: &Document) -> DocumentVersion {
    DocumentVersion {
        content_revision: doc.content_revision(),
        render_generation: doc.edit_generation(),
        saved: SavedSnapshot { content: doc.content_id(), saved_content: doc.saved_content_id() },
    }
}

fn info_of(key: DocumentKey, doc: &Document) -> DocumentInfo {
    DocumentInfo {
        document: key,
        name: doc.name.clone(),
        pages: doc
            .info
            .pages
            .iter()
            .enumerate()
            .map(|(i, p)| PageSummary { number: i.saturating_add(1), width: p.width, height: p.height, rotation: p.rotation })
            .collect(),
        editable: doc.editable(),
        read_only_reason: doc.read_only_reason.clone(),
        signed: doc.is_signed(),
        encrypted: doc.info.encrypted,
        has_file: doc.path.is_some(),
    }
}

/// An external page number (from 1) → the engine's index, checked once.
fn zero_based(page: i64, count: usize) -> Result<usize> {
    if page < 1 {
        return Err(OperationError::invalid(format!("page numbers start at 1 (got {page})")));
    }
    match usize::try_from(page - 1) {
        Ok(index) if index < count => Ok(index),
        _ => Err(OperationError::invalid(format!("page {page} is out of range: the document has {count} pages"))),
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// A filesystem error that names the file, never the folders above it.
fn io_error(path: &Path, e: &std::io::Error) -> OperationError {
    let code = if e.kind() == std::io::ErrorKind::FileTooLarge { ErrorCode::ResourceLimit } else { ErrorCode::IoFailure };
    OperationError::new(code, format!("{}: {e}", file_name(path)))
}

fn persist_error(target: &Path, e: &PersistError) -> OperationError {
    let name = file_name(target);
    let Some(conflict) = conflict_of(e) else { return OperationError::new(ErrorCode::IoFailure, format!("{name}: {e}")) };
    let (kind, advice) = match conflict {
        Conflict::Exists { actual } => (
            ConflictKind::FileExists { disk: actual.fingerprint() },
            "to replace the file that is there, repeat the request with that version as the one to overwrite; to keep both, save to another path",
        ),
        Conflict::Changed { actual, .. } => (
            ConflictKind::FileChanged { disk: actual.fingerprint() },
            "to replace the newer version on disk, repeat the request with that version as the one to overwrite; to keep both, save to another path",
        ),
        Conflict::Gone { .. } => (ConflictKind::FileGone, "to create the file again, repeat the request with the file marked absent"),
    };
    OperationError::conflict(kind, format!("{name}: save refused: {conflict}. {advice}."))
}

fn open_error(e: OpenError) -> OperationError {
    match e {
        OpenError::NeedsPassword => OperationError::new(ErrorCode::PasswordRequired, e.to_string()),
        OpenError::WrongPassword => OperationError::new(ErrorCode::WrongPassword, e.to_string()),
        OpenError::Unsupported(_) => OperationError::new(ErrorCode::UnsupportedFeature, e.to_string()),
        OpenError::Invalid(_) => OperationError::invalid(e.to_string()),
    }
}

fn edit_error(e: EditError) -> OperationError {
    let code = match &e {
        EditError::NoDocument => ErrorCode::NotFound,
        EditError::NotPermitted(_) => ErrorCode::NotAuthorized,
        EditError::ReadOnly(_) | EditError::SignedRewrite(_) | EditError::Signed => ErrorCode::UnsupportedFeature,
        EditError::WrongPassword => ErrorCode::WrongPassword,
        EditError::Cancelled => ErrorCode::Canceled,
        EditError::Write(_) | EditError::Reopen(_) => ErrorCode::InternalFailure,
        _ => ErrorCode::InvalidArgument,
    };
    OperationError::new(code, e.to_string())
}

/// A number that differs between sessions, from the hasher's random keys (they differ per process and per call, and
/// need no clock, which is not available everywhere this crate compiles).
fn random_id() -> u64 {
    std::collections::hash_map::RandomState::new().build_hasher().finish()
}

/// Every operation id has a descriptor: a compile-time reminder to add one when adding the other.
const _: () = {
    let _ = OperationId::ALL;
};
