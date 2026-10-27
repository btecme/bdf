//! The service: privately owned document sessions behind typed operations, a policy and an audit log.

use std::collections::HashMap;
use std::hash::{BuildHasher, Hasher};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use pdfcraft_engine::{DocId, Document, Edit, EditError, OpenError, SaveToken, Saved, Session, UNDO_LIMIT};
use pdfcraft_platform::confinement::{pin_existing, write_confined_gated};
use pdfcraft_platform::file_identity::{
    Authority, Conflict, FileVersion, conflict_of, parse_fingerprint, read_regular_with_version, read_with_version_limited, write_guarded_gated,
};
use pdfcraft_platform::persistence::PersistError;
use pdfcraft_platform::staging::StagingName;

use crate::actors::ActorContext;
use crate::audit::{AuditEffect, AuditEntry, AuditOutcome, AuditRecord, AuditSink, MemoryAudit};
use crate::effects::Effect;
use crate::errors::{ConflictKind, ErrorCode, OperationError, Result};
use crate::grants::system_now;
use crate::operations::{
    Access, Capability, OLDEST_SCHEMA, OpenSource, Operation, OperationId, OperationRequest, Overwrite, PageSelection, SCHEMA_VERSION, SaveTarget,
};
use crate::policy::{Denial, DenyReason, Facts, FileUse, Policy, Watch};
use crate::results::{DocumentInfo, History, OperationResult, Outcome, PageSummary, SaveReport};
use crate::version::{DocumentKey, DocumentVersion, SavedSnapshot, SessionId};

/// The largest file `document.open` reads. An interim admission limit, checked before the file is read; the budgets for
/// a whole session belong to the work on bounded jobs.
pub const MAX_OPEN_BYTES: u64 = 512 * 1024 * 1024;

/// How many planned saves may be waiting to be finished at once. A plan that is never finished (its worker died)
/// would otherwise stay registered for ever; [`Service::abandon_save`] drops one, and this bounds the rest.
pub const MAX_SAVES_IN_FLIGHT: usize = 16;

/// How many audit records the default in-memory log keeps.
pub const DEFAULT_AUDIT_RECORDS: usize = 1024;

/// What a request is made on behalf of: its number (which only correlates) and who is asking. The actor is not something
/// a caller can write: see [`ActorContext`].
#[derive(Clone, Debug)]
pub struct RequestContext {
    pub request_id: u64,
    actor: ActorContext,
}

impl RequestContext {
    pub fn new(request_id: u64, actor: ActorContext) -> Self {
        Self { request_id, actor }
    }

    pub fn actor(&self) -> &ActorContext {
        &self.actor
    }
}

/// Whether the scripts inside a document may run. Scripts are code from the document, so they are off unless someone
/// has decided to trust them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScriptTrust {
    /// Document scripts never run. The default.
    #[default]
    Disabled,
    /// Document scripts run, as they do in the desktop application today.
    Enabled,
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
    /// Who may do what.
    policy: Policy,
    /// Where the record of what was done goes.
    audit: Box<dyn AuditSink>,
    /// The audit log could not be written; clients are not served until a log is attached again.
    audit_failed: bool,
    /// For each document, the capability each undoable change needed, in step with the engine's undo and redo stacks.
    journals: HashMap<u64, Journal>,
    /// Seconds since the Unix epoch, for audit records.
    clock: fn() -> i64,
    /// Test seams, each run at the moment its name says, so a test can withdraw a grant exactly there.
    #[cfg(test)]
    hooks: Hooks,
}

/// Where a test may act between two steps of one operation.
#[cfg(test)]
#[derive(Default)]
struct Hooks {
    /// After the policy said yes and before the operation's effect: the queue.
    before_effect: Option<Box<dyn Fn()>>,
    /// After the operation has its result and before the actor is asked once more whether it may receive it.
    before_delivery: Option<Box<dyn Fn()>>,
    /// Inside the commit of a file write, after staging and before the replacement (given to each plan it makes).
    at_commit: Option<std::sync::Arc<dyn Fn() + Send + Sync>>,
}

#[derive(Clone, Copy, Debug, Default)]
struct SaveOrder {
    issued: u64,
    applied: u64,
}

/// The capability of each change that undo and redo can reach, newest last.
#[derive(Debug, Default)]
struct Journal {
    undo: Vec<Capability>,
    redo: Vec<Capability>,
}

impl Default for Service {
    fn default() -> Self {
        Self::new()
    }
}

impl Service {
    /// A service for the person at the keyboard: no client can be authorized (it holds no grants), document scripts are
    /// off, and the audit log is the last records in memory. Use [`Service::with_policy`] to let clients in.
    pub fn new() -> Self {
        let mut session = Session::new();
        session.set_javascript(false);
        Self {
            session,
            id: SessionId(random_id()),
            files: HashMap::new(),
            root: None,
            pending: HashMap::new(),
            orders: HashMap::new(),
            last_plan: 0,
            open_limit: MAX_OPEN_BYTES,
            policy: Policy::local(),
            audit: Box::new(MemoryAudit::new(DEFAULT_AUDIT_RECORDS)),
            audit_failed: false,
            journals: HashMap::new(),
            clock: system_now,
            #[cfg(test)]
            hooks: Hooks::default(),
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

    /// Fix the clock the engine stamps edits with and the audit records are dated by (deterministic tests).
    pub fn with_clock(mut self, clock: fn() -> i64) -> Self {
        self.session = std::mem::take(&mut self.session).with_clock(clock);
        self.clock = clock;
        self
    }

    /// Enforce `policy`: the clients its grants authorize, and the release it serves.
    pub fn with_policy(mut self, policy: Policy) -> Self {
        self.policy = policy;
        self
    }

    /// Record what is done in `sink` instead of in memory. A service with a new sink starts serving clients again.
    pub fn with_audit(mut self, sink: Box<dyn AuditSink>) -> Self {
        self.audit = sink;
        self.audit_failed = false;
        self
    }

    /// Decide whether the scripts inside documents run. They do not unless this says so.
    pub fn with_script_trust(mut self, trust: ScriptTrust) -> Self {
        self.session.set_javascript(trust == ScriptTrust::Enabled);
        self
    }

    pub fn scripts_enabled(&self) -> bool {
        self.session.javascript()
    }

    /// The most recent audit records the sink keeps in memory.
    pub fn recent_audit(&self) -> Vec<AuditRecord> {
        self.audit.recent()
    }

    /// Whether the audit log has failed, so that clients are refused.
    pub fn audit_failed(&self) -> bool {
        self.audit_failed
    }

    pub fn session_id(&self) -> SessionId {
        self.id
    }

    pub fn schema(&self) -> SchemaRange {
        SchemaRange { oldest: OLDEST_SCHEMA, newest: SCHEMA_VERSION }
    }

    /// The open documents `actor` may read.
    pub fn documents(&self, actor: &ActorContext) -> Vec<DocumentKey> {
        self.session
            .docs()
            .iter()
            .map(|d| self.key_of(d.id))
            .filter(|key| {
                self.policy
                    .decide(actor, OperationId::DocumentQuery.descriptor(), &Facts { document: Some(key.document), ..Facts::default() })
                    .is_ok()
            })
            .collect()
    }

    /// A read-only view of one document, for an actor that may read it. It cannot change it.
    pub fn document(&self, actor: &ActorContext, key: DocumentKey) -> Result<DocumentView<'_>> {
        self.policy
            .decide(actor, OperationId::DocumentQuery.descriptor(), &Facts { document: Some(key.document), ..Facts::default() })
            .map_err(|d| denied(&d))?;
        let doc = self.get(key)?;
        Ok(DocumentView { service: self, doc })
    }

    /// Whether `key` is still open and still at `revision`: what a worker that finished late asks before it publishes
    /// anything, so a result for a document that has since changed or closed is dropped instead of applied. It says
    /// nothing else about the document.
    pub fn is_current(&self, key: DocumentKey, revision: u64) -> bool {
        self.get(key).is_ok_and(|d| d.content_revision() == revision)
    }

    /// Run one operation, and record it.
    ///
    /// The checks come in a fixed order and each one that fails leaves the bytes, the history and the files exactly as
    /// they were: the schema version, the policy (who is asking, what they were granted, what this release has), the
    /// document, the revision of an operation that changes or discards content, the arguments, and only then the
    /// operation. Before the result is handed back the actor is asked once more whether it still may receive it, so a
    /// grant revoked while the operation ran delivers nothing. Every outcome, refusals included, is one audit record.
    pub fn execute(&mut self, context: &RequestContext, request: OperationRequest) -> Result<OperationResult> {
        let name = request.operation.id().name();
        let document = request.document.filter(|k| k.session == self.id).map(|k| k.document);
        let before = document.and_then(|n| self.session.get(DocId(n))).map(|d| d.content_revision());
        let result = self.run(context, request);
        self.audit_result(context.actor(), context.request_id, name, document, before, &result);
        result
    }

    fn run(&mut self, context: &RequestContext, request: OperationRequest) -> Result<OperationResult> {
        let actor = context.actor();
        negotiate(request.schema_version)?;
        let id = request.operation.id();
        let descriptor = id.descriptor();
        if actor.is_agent() && self.audit_failed {
            return Err(denied(&Denial {
                reason: DenyReason::AuditUnavailable,
                message: "this service can't record what it does, so it is not serving clients".into(),
            }));
        }
        let file = self.file_for(&request)?;
        let facts = Facts {
            document: request.document.filter(|k| k.session == self.id).map(|k| k.document),
            file: file.as_ref().map(|(path, write)| if *write { FileUse::Write(path) } else { FileUse::Read(path) }),
            reverts: self.reverts_for(&request),
        };
        self.policy.decide(actor, descriptor, &facts).map_err(|d| denied(&d))?;
        let doc_id = match (descriptor.needs_document, request.document) {
            (false, _) => None,
            (true, None) => return Err(OperationError::invalid(format!("{} needs a document", descriptor.name))),
            (true, Some(key)) => Some(self.get(key)?.id),
        };
        if let (Some(doc_id), Some(expected)) = (doc_id, request.operation.expected_revision()) {
            self.check_revision(doc_id, expected)?;
        }
        #[cfg(test)]
        if let Some(hook) = &self.hooks.before_effect {
            hook();
        }
        if descriptor.access != Access::Read {
            // Immediately before the effect: a grant withdrawn while the request waited makes nothing happen.
            self.policy.recheck(actor).map_err(|d| denied(&d))?;
        }
        let result = match request.operation {
            Operation::Open { source, password } => self.open(source, password.as_deref())?,
            Operation::Query => self.query(self.need(doc_id)?)?,
            Operation::Rotate { pages, degrees, .. } => self.rotate(self.need(doc_id)?, pages, degrees)?,
            Operation::Undo { .. } => self.step(self.need(doc_id)?, Session::undo, true)?,
            Operation::Redo { .. } => self.step(self.need(doc_id)?, Session::redo, false)?,
            Operation::History => self.history(self.need(doc_id)?)?,
            Operation::Save { expected_revision, target, overwrite, full } => {
                let key = self.key_of(self.need(doc_id)?);
                let prepared = self.prepare_save_inner(actor, context.request_id, key, expected_revision, target, overwrite, full)?;
                let written = match prepared.write() {
                    Ok(written) => written,
                    Err(e) => {
                        self.abandon_save(&prepared);
                        return Err(e);
                    }
                };
                self.finish_save_inner(&written)?
            }
            Operation::Close { discard_changes, .. } => self.close(self.need(doc_id)?, discard_changes)?,
        };
        #[cfg(test)]
        if let Some(hook) = &self.hooks.before_delivery {
            hook();
        }
        if let Err(denial) = self.policy.recheck(actor) {
            // Revoked while the operation ran: nothing is handed back. What was already done stays done and is said so.
            let mut error = denied(&denial);
            error.committed = result.effects;
            return Err(error);
        }
        Ok(result)
    }

    /// The file a request reads or writes, resolved the way the service will touch it, and whether it is written.
    fn file_for(&self, request: &OperationRequest) -> Result<Option<(PathBuf, bool)>> {
        Ok(match &request.operation {
            Operation::Open { source: OpenSource::File(path), .. } => Some((self.storage_path(path)?, false)),
            Operation::Save { target: SaveTarget::Path(path), .. } => Some((self.storage_path(path)?, true)),
            Operation::Save { target: SaveTarget::Tracked, .. } => request
                .document
                .and_then(|key| self.get(key).ok())
                .and_then(|doc| doc.path.as_deref())
                .and_then(|stored| self.storage_path(Path::new(stored)).ok())
                .map(|path| (path, true)),
            _ => None,
        })
    }

    /// For undo and redo: the capability of the change they would revert, from the document's history.
    fn reverts_for(&self, request: &OperationRequest) -> Option<Capability> {
        let journal = self.journals.get(&request.document.filter(|k| k.session == self.id)?.document)?;
        match request.operation {
            Operation::Undo { .. } => journal.undo.last().copied(),
            Operation::Redo { .. } => journal.redo.last().copied(),
            _ => None,
        }
    }

    // ---- saving, in two halves ----

    /// First half of a save: take the bytes of the content the request was made against (`expected_revision`, checked
    /// here as well as in [`Service::execute`], so that a caller who splits the save cannot skip it), work out what the
    /// target may replace, and return a plan that can be written on any thread. The policy is asked here (the actor must
    /// hold `document.save` for the document and a folder for the file) and again at the moment the write commits.
    /// Nothing is written and nothing is marked saved.
    pub fn prepare_save(
        &mut self,
        context: &RequestContext,
        key: DocumentKey,
        expected_revision: u64,
        target: SaveTarget,
        overwrite: Overwrite,
        full: Option<bool>,
    ) -> Result<PreparedSave> {
        let result = self.prepare_save_inner(context.actor(), context.request_id, key, expected_revision, target, overwrite, full);
        if let Err(e) = &result {
            // A plan that is made is recorded when it is finished; one that is refused is recorded now.
            let failed: Result<OperationResult> = Err(e.clone());
            self.audit_result(context.actor(), context.request_id, "document.save", Some(key.document), None, &failed);
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_save_inner(
        &mut self,
        actor: &ActorContext,
        request_id: u64,
        key: DocumentKey,
        expected_revision: u64,
        target: SaveTarget,
        overwrite: Overwrite,
        full: Option<bool>,
    ) -> Result<PreparedSave> {
        let descriptor = OperationId::DocumentSave.descriptor();
        // Before anything about the document is revealed: may this actor save it at all?
        self.policy.decide(actor, descriptor, &Facts { document: Some(key.document), ..Facts::default() }).map_err(|d| denied(&d))?;
        let doc = self.get(key)?;
        self.check_revision(doc.id, expected_revision)?;
        let path = match target {
            SaveTarget::Path(p) => self.storage_path(&p)?,
            SaveTarget::Tracked => match doc.path.as_deref() {
                Some(stored) => self.storage_path(Path::new(stored))?,
                None => return Err(OperationError::invalid("the document has never been saved: pass a path")),
            },
        };
        self.policy
            .decide(actor, descriptor, &Facts { document: Some(key.document), file: Some(FileUse::Write(&path)), ..Facts::default() })
            .map_err(|d| denied(&d))?;
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
        Ok(PreparedSave {
            service: self.id,
            plan,
            seq,
            key,
            revision,
            token,
            bytes,
            target: path,
            authority,
            root,
            incremental,
            actor: actor.clone(),
            request_id,
            watch: self.policy.watch(actor),
            #[cfg(test)]
            at_commit: self.hooks.at_commit.clone(),
        })
    }

    /// Give up a planned save that will not be finished (its write failed or its worker is gone).
    pub fn abandon_save(&mut self, plan: &PreparedSave) {
        if plan.service == self.id {
            self.pending.remove(&plan.plan);
        }
    }

    /// Second half: publish a write that happened, and record it.
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
    ///
    /// If the actor's grant was revoked while the save was queued or running, the document is still brought into line
    /// with what is on disk, but the result is not handed back: the error says `not_authorized` and lists the write.
    pub fn finish_save(&mut self, written: &WrittenSave) -> Result<OperationResult> {
        let plan = &written.plan;
        let before = self.get(plan.key).ok().map(|d| d.content_revision());
        let mut result = self.finish_save_inner(written);
        if let Ok(done) = &result
            && let Err(denial) = plan.watch.check()
        {
            let mut error = denied(&denial);
            error.committed = done.effects.clone();
            result = Err(error);
        }
        self.audit_result(&plan.actor, plan.request_id, "document.save", Some(plan.key.document), before, &result);
        result
    }

    fn finish_save_inner(&mut self, written: &WrittenSave) -> Result<OperationResult> {
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
        // The change is made: remember what it took to make it, so that undoing it takes the same.
        let journal = self.journals.entry(id.0).or_default();
        journal.undo.push(Capability::DocumentEditPages);
        journal.redo.clear();
        if journal.undo.len() > UNDO_LIMIT {
            journal.undo.remove(0);
        }
        self.changed(id, Some(affected))
    }

    fn step(
        &mut self,
        id: DocId,
        run: fn(&mut Session, DocId) -> std::result::Result<pdfcraft_engine::Label, EditError>,
        undoing: bool,
    ) -> Result<OperationResult> {
        let label = run(&mut self.session, id).map_err(edit_error)?;
        let journal = self.journals.entry(id.0).or_default();
        if undoing {
            if let Some(capability) = journal.undo.pop() {
                journal.redo.push(capability);
            }
        } else if let Some(capability) = journal.redo.pop() {
            journal.undo.push(capability);
        }
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
        self.journals.remove(&id.0);
        Ok(OperationResult { outcome: Outcome::Closed, version: None, warnings: Vec::new(), effects: vec![Effect::DocumentClosed] })
    }

    // ---- the audit log ----

    /// One record for one outcome. A log that cannot be written stops the service serving clients; it never stops the
    /// person from working, and it never changes the result that is returned.
    fn audit_result(
        &mut self,
        actor: &ActorContext,
        request_id: u64,
        operation: &str,
        document: Option<u64>,
        revision_before: Option<u64>,
        result: &Result<OperationResult>,
    ) {
        let (outcome, effects, after, warnings) = match result {
            Ok(done) => (AuditOutcome::Ok, audit_effects(&done.effects), done.version.map(|v| v.content_revision), done.warnings.len()),
            Err(error) => {
                let why = error.reason.unwrap_or(error.code.as_str()).to_owned();
                let outcome = if error.code == ErrorCode::NotAuthorized { AuditOutcome::Denied(why) } else { AuditOutcome::Failed(why) };
                (outcome, audit_effects(&error.committed), None, 0)
            }
        };
        let revision_after = after.or_else(|| document.and_then(|n| self.session.get(DocId(n))).map(|d| d.content_revision()));
        let entry = AuditEntry {
            at: (self.clock)(),
            actor: actor.kind(),
            client: actor.client().map(|c| c.as_str().to_owned()),
            grant: actor.grant().map(|g| g.as_str().to_owned()),
            request_id,
            operation: operation.to_owned(),
            document,
            revision_before,
            revision_after,
            outcome,
            effects,
            warnings,
        };
        if self.audit.record(entry).is_err() {
            self.audit_failed = true;
        }
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
    /// written relative to it, with no `..` and no drive or root of its own. The refusal names the path the caller gave
    /// and not the root, which is not the caller's to learn.
    fn storage_path(&self, path: &Path) -> Result<PathBuf> {
        let Some(root) = &self.root else { return Ok(path.to_path_buf()) };
        let outside = || OperationError::invalid(format!("{} is outside the allowed directory", path.display()));
        // A path the service itself stored (the document's own) is already below the root.
        let relative = path.strip_prefix(root).unwrap_or(path);
        if relative.as_os_str().is_empty() || !relative.components().all(|c| matches!(c, Component::Normal(_))) {
            return Err(outside());
        }
        Ok(root.join(relative))
    }
}

/// A save that has been planned and can be written without the service: the bytes of one revision, the file they go to
/// and what they may replace. It holds no reference to the service, so a worker can own it. It remembers who asked, and
/// asks again at the moment of replacement whether that actor still may.
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
    actor: ActorContext,
    request_id: u64,
    watch: Watch,
    #[cfg(test)]
    at_commit: Option<std::sync::Arc<dyn Fn() + Send + Sync>>,
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

/// The grant behind a save was withdrawn before the file was replaced.
#[derive(Debug)]
struct Revoked(DenyReason);

impl std::fmt::Display for Revoked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the grant was withdrawn ({})", self.0.as_str())
    }
}

impl std::error::Error for Revoked {}

impl PreparedSave {
    /// The content revision these bytes are of.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn document(&self) -> DocumentKey {
        self.key
    }

    /// Write the bytes. The actor is asked whether it still may, before anything is staged and again at the moment the new
    /// bytes would replace the file, so a grant revoked while the save was queued or running writes nothing. The target is
    /// replaced only if it is still what the plan said it may replace, checked at the same moment; otherwise nothing is
    /// written and the error is a conflict that names what is there.
    pub fn write(&self) -> Result<WrittenSave> {
        self.watch.check().map_err(|d| denied(&d))?;
        if self.target.is_dir() {
            return Err(OperationError::invalid(format!("{}: is a folder, not a file", file_name(&self.target))));
        }
        let authority = self.authority.borrow();
        let gate = || {
            #[cfg(test)]
            if let Some(hook) = &self.at_commit {
                hook();
            }
            self.watch.check().map_err(|d| std::io::Error::new(std::io::ErrorKind::PermissionDenied, Revoked(d.reason)))
        };
        let result = match &self.root {
            Some(root) => write_confined_gated(root, &self.target, &self.bytes, StagingName::SuffixThenTag, Some(authority), &gate),
            None => {
                if let Some(dir) = self.target.parent().filter(|p| !p.as_os_str().is_empty()) {
                    std::fs::create_dir_all(dir).map_err(|e| io_error(dir, &e))?;
                }
                write_guarded_gated(&self.target, &self.bytes, StagingName::SuffixThenTag, authority, &gate)
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

/// A refusal by the policy, as the error every surface sees. The reason's stable spelling is kept for the audit log.
fn denied(denial: &Denial) -> OperationError {
    OperationError { reason: Some(denial.reason.as_str()), ..OperationError::new(ErrorCode::NotAuthorized, denial.message.clone()) }
}

fn audit_effects(effects: &[Effect]) -> Vec<AuditEffect> {
    effects
        .iter()
        .map(|effect| match effect {
            Effect::FileWritten { file, bytes, .. } => AuditEffect { kind: "file_written".into(), file: Some(file.clone()), bytes: Some(*bytes) },
            Effect::DocumentClosed => AuditEffect { kind: "document_closed".into(), file: None, bytes: None },
        })
        .collect()
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
    if let Some(revoked) = e.source.get_ref().and_then(|inner| inner.downcast_ref::<Revoked>()) {
        return OperationError {
            reason: Some(revoked.0.as_str()),
            ..OperationError::new(ErrorCode::NotAuthorized, format!("{name}: not written: the grant that asked for this save was withdrawn"))
        };
    }
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
    let (code, reason) = match &e {
        EditError::NoDocument => (ErrorCode::NotFound, None),
        EditError::NotPermitted(_) => (ErrorCode::NotAuthorized, Some(DenyReason::DocumentRights.as_str())),
        EditError::ReadOnly(_) | EditError::SignedRewrite(_) | EditError::Signed => (ErrorCode::UnsupportedFeature, None),
        EditError::WrongPassword => (ErrorCode::WrongPassword, None),
        EditError::Cancelled => (ErrorCode::Canceled, None),
        EditError::Write(_) | EditError::Reopen(_) => (ErrorCode::InternalFailure, None),
        _ => (ErrorCode::InvalidArgument, None),
    };
    OperationError { reason, ..OperationError::new(code, e.to_string()) }
}

/// A number that differs between sessions, from the hasher's random keys (they differ per process and per call, and
/// need no clock, which is not available everywhere this crate compiles).
fn random_id() -> u64 {
    std::collections::hash_map::RandomState::new().build_hasher().finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actors::{ClientId, GrantId, OsPrincipal};
    use crate::audit::AuditOutcome;
    use crate::grants::{DocumentScope, FileAccess, GrantRequest, GrantStore, Scope};
    use crate::testutil::{fixture, private_dir};

    fn owner() -> OsPrincipal {
        OsPrincipal::current().expect("a user")
    }

    fn me() -> ActorContext {
        ActorContext::local_user().expect("a user")
    }

    /// A grant for `name` holding `capabilities` over every document, and the files under `folder` for reading and writing.
    fn agent(grants: &GrantStore, name: &str, capabilities: &[Capability], folder: Option<&Path>) -> ActorContext {
        let files = folder.map(|f| FileAccess { read_under: vec![f.to_path_buf()], write_under: vec![f.to_path_buf()] }).unwrap_or_default();
        let request = GrantRequest {
            client: ClientId::parse(name).expect("a name"),
            scopes: capabilities.iter().map(|c| Scope { capability: *c, documents: DocumentScope::All }).collect(),
            files,
            ttl_seconds: 3600,
        };
        let issued = grants.issue(request).expect("issues");
        grants.authenticate(name, issued.secret()).expect("authenticates")
    }

    fn open_bytes(service: &mut Service) -> DocumentKey {
        let source = OpenSource::Bytes { name: "doc.pdf".into(), bytes: Arc::new(fixture(2)) };
        let done =
            service.execute(&RequestContext::new(1, me()), OperationRequest::new(None, Operation::Open { source, password: None })).expect("opens");
        match done.outcome {
            Outcome::Opened { document, .. } => document,
            other => panic!("opened, got {other:?}"),
        }
    }

    fn rotate_request(key: DocumentKey, revision: u64) -> OperationRequest {
        OperationRequest::new(Some(key), Operation::Rotate { expected_revision: revision, pages: PageSelection::Numbers(vec![1]), degrees: 90 })
    }

    fn rotations(service: &Service, key: DocumentKey) -> Vec<u16> {
        service.document(&me(), key).expect("open").info().pages.iter().map(|p| p.rotation).collect()
    }

    fn revoke_when_asked(grants: &GrantStore, actor: &ActorContext) -> Box<dyn Fn()> {
        let (grants, id): (GrantStore, GrantId) = (grants.clone(), actor.grant().expect("a grant").clone());
        Box::new(move || grants.revoke(&id).expect("revokes"))
    }

    #[test]
    fn a_grant_withdrawn_while_a_request_waited_makes_nothing_happen() {
        let grants = GrantStore::in_memory(owner());
        let editor = agent(&grants, "editor", &[Capability::DocumentRead, Capability::DocumentEditPages], None);
        let mut service = Service::new().with_policy(Policy::with_grants(grants.clone()));
        let key = open_bytes(&mut service);
        let before = service.document(&me(), key).expect("open").version();
        service.hooks.before_effect = Some(revoke_when_asked(&grants, &editor));

        let error = service.execute(&RequestContext::new(2, editor), rotate_request(key, before.content_revision)).expect_err("refused");
        assert_eq!((error.code, error.reason), (ErrorCode::NotAuthorized, Some("grant_revoked")));
        assert_eq!(service.document(&me(), key).expect("open").version(), before, "the document is exactly as it was");
        assert_eq!(rotations(&service, key), [0, 0]);
        let last = service.recent_audit().pop().expect("a record");
        assert_eq!(last.entry.outcome, AuditOutcome::Denied("grant_revoked".into()));
        assert_eq!((last.entry.revision_before, last.entry.revision_after), (Some(before.content_revision), Some(before.content_revision)));
    }

    #[test]
    fn a_grant_withdrawn_while_work_ran_hands_nothing_back() {
        let grants = GrantStore::in_memory(owner());
        let reader = agent(&grants, "reader", &[Capability::DocumentRead], None);
        let mut service = Service::new().with_policy(Policy::with_grants(grants.clone()));
        let key = open_bytes(&mut service);
        service.hooks.before_delivery = Some(revoke_when_asked(&grants, &reader));
        let query = OperationRequest::new(Some(key), Operation::Query);
        let error = service.execute(&RequestContext::new(2, reader), query).expect_err("not delivered");
        assert_eq!((error.code, error.reason), (ErrorCode::NotAuthorized, Some("grant_revoked")));
        assert!(!error.message.contains("doc.pdf"), "the refusal does not carry what was asked for: {}", error.message);
    }

    #[test]
    fn a_save_queued_or_running_when_its_grant_is_withdrawn_writes_nothing() {
        let dir = private_dir("revoked-save");
        let path = dir.join("doc.pdf");
        std::fs::write(&path, fixture(2)).expect("writes");
        let original = std::fs::read(&path).expect("reads");
        let all = [Capability::DocumentRead, Capability::DocumentSave];
        let grants = GrantStore::in_memory(owner());
        let mut service = Service::new().with_policy(Policy::with_grants(grants.clone()));
        let human = RequestContext::new(1, me());
        let opened = service
            .execute(&human, OperationRequest::new(None, Operation::Open { source: OpenSource::File(path.clone()), password: None }))
            .expect("opens");
        let key = match opened.outcome {
            Outcome::Opened { document, .. } => document,
            other => panic!("opened, got {other:?}"),
        };
        let v0 = opened.version.expect("version");
        let v1 = service.execute(&human, rotate_request(key, v0.content_revision)).expect("a human edit").version.expect("version");

        // Queued: planned, then the grant is withdrawn before the worker starts.
        let queued = agent(&grants, "queued", &all, Some(&dir));
        // Into a folder that does not exist yet, so that "nothing was staged or created" can be seen.
        let into_new_folder = SaveTarget::Path(dir.join("not-yet").join("copy.pdf"));
        let plan = service
            .prepare_save(&RequestContext::new(2, queued.clone()), key, v1.content_revision, into_new_folder, Overwrite::Absent, None)
            .expect("plans");
        grants.revoke(queued.grant().expect("a grant")).expect("revokes");
        let error = plan.write().expect_err("not written");
        assert_eq!((error.code, error.reason), (ErrorCode::NotAuthorized, Some("grant_revoked")));
        assert_eq!(std::fs::read(&path).expect("reads"), original);
        assert!(!dir.join("not-yet").exists(), "a withdrawn grant does not even get its folder made");
        service.abandon_save(&plan);

        // Running: the grant is withdrawn inside the commit, after the bytes are staged and before they replace the file.
        let running = agent(&grants, "running", &all, Some(&dir));
        service.hooks.at_commit = Some(Arc::new(revoke_when_asked_sync(&grants, &running)));
        let plan = service
            .prepare_save(&RequestContext::new(3, running), key, v1.content_revision, SaveTarget::Tracked, Overwrite::Tracked, None)
            .expect("plans");
        let error = plan.write().expect_err("not written");
        assert_eq!((error.code, error.reason), (ErrorCode::NotAuthorized, Some("grant_revoked")));
        assert_eq!(std::fs::read(&path).expect("reads"), original, "the file was never replaced");
        let leftovers: Vec<_> = std::fs::read_dir(&dir).expect("lists").filter_map(|e| e.ok()).map(|e| e.file_name()).collect();
        assert_eq!(leftovers.len(), 1, "no staging file is left behind: {leftovers:?}");
        service.abandon_save(&plan);
        assert!(service.document(&me(), key).expect("open").version().dirty(), "the edit is still unsaved");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn revoke_when_asked_sync(grants: &GrantStore, actor: &ActorContext) -> impl Fn() + Send + Sync + use<> {
        let (grants, id): (GrantStore, GrantId) = (grants.clone(), actor.grant().expect("a grant").clone());
        move || grants.revoke(&id).expect("revokes")
    }

    #[test]
    fn a_save_whose_grant_is_withdrawn_after_the_write_reports_the_write_and_hands_nothing_back() {
        let dir = private_dir("revoked-after");
        let path = dir.join("doc.pdf");
        std::fs::write(&path, fixture(2)).expect("writes");
        let grants = GrantStore::in_memory(owner());
        let mut service = Service::new().with_policy(Policy::with_grants(grants.clone()));
        let human = RequestContext::new(1, me());
        let opened = service
            .execute(&human, OperationRequest::new(None, Operation::Open { source: OpenSource::File(path.clone()), password: None }))
            .expect("opens");
        let key = match opened.outcome {
            Outcome::Opened { document, .. } => document,
            other => panic!("opened, got {other:?}"),
        };
        let v1 =
            service.execute(&human, rotate_request(key, opened.version.expect("version").content_revision)).expect("edit").version.expect("version");
        let saver = agent(&grants, "saver", &[Capability::DocumentRead, Capability::DocumentSave], Some(&dir));
        let plan = service
            .prepare_save(&RequestContext::new(2, saver.clone()), key, v1.content_revision, SaveTarget::Tracked, Overwrite::Tracked, None)
            .expect("plans");
        let written = plan.write().expect("writes");
        grants.revoke(saver.grant().expect("a grant")).expect("revokes");

        let error = service.finish_save(&written).expect_err("not handed back");
        assert_eq!((error.code, error.reason), (ErrorCode::NotAuthorized, Some("grant_revoked")));
        assert!(matches!(error.committed.as_slice(), [Effect::FileWritten { file, .. }] if file == "doc.pdf"), "{:?}", error.committed);
        // The file is on disk, so the document is brought into line with it: clean, and its next save is not a conflict.
        assert!(!service.document(&me(), key).expect("open").version().dirty());
        let last = service.recent_audit().pop().expect("a record");
        assert_eq!(last.entry.outcome, AuditOutcome::Denied("grant_revoked".into()));
        assert_eq!(last.entry.effects.len(), 1, "the audit record names the write that happened");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
