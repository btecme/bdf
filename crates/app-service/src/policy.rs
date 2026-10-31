//! What an actor may do (BUILD-008): the intersection of who is asking, what they were granted, what the document
//! allows and what this release has.
//!
//! [`Policy::decide`] is the only place that says yes. A request is allowed when **all** of these hold, and refused with
//! the first that does not:
//!
//! 1. **The release has the operation.** The operation's descriptor must be `Core` in a core-only release. An operation
//!    that is deferred or excluded is refused even though its name is known, and an operation with no descriptor at all
//!    ([`Policy::decide_named`]) is refused: what was not discovered and classified is denied.
//! 2. **The actor is the person, or holds a live grant.** The person at the keyboard (the operating-system user) passes
//!    this step. A client passes it only while its grant exists, is not revoked, has not expired and was delegated by
//!    this operating-system account.
//! 3. **The grant holds the capability for this document.** Reading, editing pages, saving and closing are separate
//!    capabilities, each limited to some documents. A new grant holds only `document.read`.
//! 4. **Undo and redo are not a way round 3.** They need the capability of the change they revert, recorded when the
//!    change was made; a client that cannot make a change cannot undo or redo one either.
//! 5. **A file needs a folder.** Opening a file needs a folder the grant may read from, and saving needs one it may write
//!    into. A document grant alone does not authorize any file, and a path with `..` in it never does.
//!
//! The PDF's own permission bits are a further, independent intersection and stay with the engine, which enforces them
//! when a change is applied; the service reports that refusal as `not_authorized` too.
//!
//! The decision is made when a request arrives. For work that is queued or running, [`Policy::watch`] hands back something
//! that asks again immediately before each effect and before data is handed back, so a revocation stops it.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::actors::ActorContext;
use crate::grants::{GrantProblem, GrantStore, GrantTerms};
use crate::operations::{Capability, OperationDescriptor, OperationId, Release};
use crate::version::DocumentKey;

/// What this release contains. v1 is core-only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReleaseProfile {
    /// Only operations whose descriptor says `Core`.
    Core,
}

impl ReleaseProfile {
    pub fn allows(self, release: Release) -> bool {
        match self {
            ReleaseProfile::Core => release == Release::Core,
        }
    }
}

/// Why a request was refused. The spelling is stable: it is what the audit log records.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DenyReason {
    /// The operation is not in this release (deferred or excluded).
    ReleaseProfile,
    /// The operation is not one this service has classified.
    UnknownOperation,
    GrantUnknown,
    GrantRevoked,
    GrantExpired,
    GrantWrongAccount,
    /// The grant does not hold the capability the operation needs.
    CapabilityNotGranted,
    /// The grant holds it, but not for this document.
    DocumentOutOfScope,
    /// A file is involved and no granted folder contains it.
    FolderNotGranted,
    /// Undo or redo of a change the actor could not have made, or whose origin is not known.
    HistoryNotPermitted,
    /// The audit log could not be written, and a client's requests are not served without one.
    AuditUnavailable,
    /// The document's own permissions refuse the change.
    DocumentRights,
}

impl DenyReason {
    pub fn as_str(self) -> &'static str {
        match self {
            DenyReason::ReleaseProfile => "release_profile",
            DenyReason::UnknownOperation => "unknown_operation",
            DenyReason::GrantUnknown => "grant_unknown",
            DenyReason::GrantRevoked => "grant_revoked",
            DenyReason::GrantExpired => "grant_expired",
            DenyReason::GrantWrongAccount => "grant_wrong_account",
            DenyReason::CapabilityNotGranted => "capability_not_granted",
            DenyReason::DocumentOutOfScope => "document_out_of_scope",
            DenyReason::FolderNotGranted => "folder_not_granted",
            DenyReason::HistoryNotPermitted => "history_not_permitted",
            DenyReason::AuditUnavailable => "audit_unavailable",
            DenyReason::DocumentRights => "document_rights",
        }
    }

    fn of_problem(problem: GrantProblem) -> Self {
        match problem {
            GrantProblem::Unknown => DenyReason::GrantUnknown,
            GrantProblem::Revoked => DenyReason::GrantRevoked,
            GrantProblem::Expired => DenyReason::GrantExpired,
            GrantProblem::WrongAccount => DenyReason::GrantWrongAccount,
        }
    }
}

/// A refusal: the reason, and a sentence for the caller. The sentence names no file, folder or document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Denial {
    pub reason: DenyReason,
    pub message: String,
}

impl Denial {
    pub(crate) fn new(reason: DenyReason, message: &str) -> Self {
        Self { reason, message: message.to_owned() }
    }
}

impl fmt::Display for Denial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.message, self.reason.as_str())
    }
}

impl std::error::Error for Denial {}

/// What a file is wanted for.
#[derive(Clone, Copy, Debug)]
pub enum FileUse<'a> {
    Read(&'a Path),
    Write(&'a Path),
}

/// What the request itself says, apart from who made it: the facts the decision needs.
#[derive(Clone, Copy, Debug, Default)]
pub struct Facts<'a> {
    /// The document the operation is about, by its full identity (session and number). Never the number alone.
    pub document: Option<DocumentKey>,
    /// The file the operation reads or writes, already resolved.
    pub file: Option<FileUse<'a>>,
    /// For undo and redo: the capability of the change they would revert, from the document's history; `None` when the
    /// history does not say (which a client is refused).
    pub reverts: Option<Capability>,
}

/// The policy a service enforces.
#[derive(Clone)]
pub struct Policy {
    profile: ReleaseProfile,
    grants: Option<GrantStore>,
}

impl Policy {
    /// Only the person at the keyboard: no client can be authorized, because there is no grant store to hold a grant.
    pub fn local() -> Self {
        Self { profile: ReleaseProfile::Core, grants: None }
    }

    /// The person, and the clients the grants in `store` authorize.
    pub fn with_grants(store: GrantStore) -> Self {
        Self { profile: ReleaseProfile::Core, grants: Some(store) }
    }

    pub fn profile(&self) -> ReleaseProfile {
        self.profile
    }

    /// Decide a request for the operation `descriptor`.
    pub fn decide(&self, actor: &ActorContext, descriptor: &OperationDescriptor, facts: &Facts) -> Result<(), Denial> {
        if !self.profile.allows(descriptor.release) {
            return Err(Denial::new(DenyReason::ReleaseProfile, "that operation is not available in this release"));
        }
        if !actor.is_agent() {
            return Ok(());
        }
        let terms = self.terms_of(actor)?;
        let capability = match descriptor.capability {
            // The change being reverted must be known, and must be a change: reading changes nothing, so a history that
            // says a step needed only `document.read` is not one the history can have produced.
            Capability::RevertedEdit => facts
                .reverts
                .filter(|c| *c != Capability::DocumentRead && *c != Capability::RevertedEdit)
                .ok_or_else(|| Denial::new(DenyReason::HistoryNotPermitted, "that change can't be undone or redone by this client"))?,
            other => other,
        };
        let Some(scope) = terms.scope_for(capability) else {
            let reason =
                if descriptor.capability == Capability::RevertedEdit { DenyReason::HistoryNotPermitted } else { DenyReason::CapabilityNotGranted };
            return Err(Denial::new(reason, "this client has not been granted that"));
        };
        if descriptor.needs_document {
            match facts.document {
                Some(document) if scope.documents.contains(&document) => {}
                _ => return Err(Denial::new(DenyReason::DocumentOutOfScope, "this client has not been granted that document")),
            }
        }
        match facts.file {
            None => Ok(()),
            Some(FileUse::Read(path)) if terms.files.allows_read(path) => Ok(()),
            Some(FileUse::Write(path)) if terms.files.allows_write(path) => Ok(()),
            Some(_) => Err(Denial::new(DenyReason::FolderNotGranted, "this client has not been granted that folder")),
        }
    }

    /// The folder of the actor's grant that holds the file, for an agent: what its reads and writes are confined to, so
    /// that a link below it cannot reach what the grant never named. `None` for the person (who is not confined by a
    /// grant) and for a file no granted folder holds.
    pub fn granted_root(&self, actor: &ActorContext, file: FileUse<'_>) -> Option<PathBuf> {
        if !actor.is_agent() {
            return None;
        }
        let terms = self.terms_of(actor).ok()?;
        let (write, path) = match file {
            FileUse::Read(path) => (false, path),
            FileUse::Write(path) => (true, path),
        };
        terms.files.granting_root(write, path).map(Path::to_path_buf)
    }

    /// Decide a request for an operation known only by name. A name with no descriptor is refused.
    pub fn decide_named(&self, actor: &ActorContext, name: &str, facts: &Facts) -> Result<(), Denial> {
        match OperationId::parse(name) {
            Some(id) => self.decide(actor, id.descriptor(), facts),
            None => Err(Denial::new(DenyReason::UnknownOperation, "that operation is not known to this service")),
        }
    }

    /// Whether the actor is still entitled to act at all: its grant exists, is not revoked, has not expired.
    pub fn recheck(&self, actor: &ActorContext) -> Result<(), Denial> {
        if !actor.is_agent() {
            return Ok(());
        }
        self.terms_of(actor).map(|_| ())
    }

    /// Something that asks [`Policy::recheck`] again later, from another thread, without the policy: what a queued or
    /// running operation carries.
    pub fn watch(&self, actor: &ActorContext) -> Watch {
        Watch { grants: self.grants.clone(), actor: actor.clone() }
    }

    fn terms_of(&self, actor: &ActorContext) -> Result<GrantTerms, Denial> {
        let Some(store) = &self.grants else {
            return Err(Denial::new(DenyReason::GrantUnknown, "this service holds no grants"));
        };
        store.check(actor).map_err(|problem| Denial::new(DenyReason::of_problem(problem), "this client's grant does not hold"))
    }
}

/// A standing question: is this actor still entitled to act? Cheap to ask, safe to carry to another thread.
#[derive(Clone)]
pub struct Watch {
    grants: Option<GrantStore>,
    actor: ActorContext,
}

impl Watch {
    pub fn check(&self) -> Result<(), Denial> {
        if !self.actor.is_agent() {
            return Ok(());
        }
        let Some(store) = &self.grants else {
            return Err(Denial::new(DenyReason::GrantUnknown, "this service holds no grants"));
        };
        store.check(&self.actor).map(|_| ()).map_err(|problem| Denial::new(DenyReason::of_problem(problem), "this client's grant does not hold"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actors::{ClientId, OsPrincipal};
    use crate::grants::{DocumentScope, FileAccess, GrantRequest, Scope};
    use crate::operations::{Access, DESCRIPTORS};
    use crate::version::SessionId;
    use std::path::PathBuf;

    fn clock() -> i64 {
        1_000_000
    }

    fn store() -> GrantStore {
        GrantStore::in_memory(OsPrincipal::current().expect("a user")).with_clock(clock)
    }

    fn agent(store: &GrantStore, request: GrantRequest) -> ActorContext {
        let client = request.client.as_str().to_owned();
        let issued = store.issue(request).expect("issues");
        store.authenticate(&client, issued.secret()).expect("authenticates")
    }

    fn descriptor(id: OperationId) -> &'static OperationDescriptor {
        id.descriptor()
    }

    fn key(document: u64) -> DocumentKey {
        DocumentKey { session: SessionId(1), document }
    }

    fn facts_for(document: u64) -> Facts<'static> {
        Facts { document: Some(key(document)), ..Facts::default() }
    }

    fn client(name: &str) -> ClientId {
        ClientId::parse(name).expect("a client name")
    }

    #[test]
    fn the_person_may_do_every_core_operation() {
        let policy = Policy::local();
        let me = ActorContext::local_user().expect("a user");
        for d in &DESCRIPTORS {
            assert!(policy.decide(&me, d, &facts_for(1)).is_ok(), "{}", d.name);
        }
    }

    #[test]
    fn a_new_grant_reads_and_does_nothing_else() {
        let grants = store();
        let policy = Policy::with_grants(grants.clone());
        let reader = agent(&grants, GrantRequest::read_only(client("reader")));
        let mut allowed = Vec::new();
        for d in &DESCRIPTORS {
            let facts = Facts { reverts: Some(Capability::DocumentEditPages), ..facts_for(1) };
            if policy.decide(&reader, d, &facts).is_ok() {
                allowed.push(d.name);
            }
        }
        // Reading: open (no file, so bytes only), query, history. Everything that changes, writes, closes, undoes is refused.
        assert_eq!(allowed, ["document.open", "document.query", "edit.history"]);
        for d in DESCRIPTORS.iter().filter(|d| matches!(d.access, Access::Mutate | Access::Persist) || d.name == "document.close") {
            let denial = policy.decide(&reader, d, &Facts { reverts: Some(Capability::DocumentRead), ..facts_for(1) }).expect_err(d.name);
            assert!(matches!(denial.reason, DenyReason::CapabilityNotGranted | DenyReason::HistoryNotPermitted), "{}: {denial}", d.name);
        }
    }

    #[test]
    fn undo_and_redo_need_the_capability_of_the_change_they_revert() {
        let grants = store();
        let policy = Policy::with_grants(grants.clone());
        let editor = agent(
            &grants,
            GrantRequest {
                scopes: vec![
                    Scope { capability: Capability::DocumentRead, documents: DocumentScope::All },
                    Scope { capability: Capability::DocumentEditPages, documents: DocumentScope::All },
                ],
                ..GrantRequest::read_only(client("editor"))
            },
        );
        let reader = agent(&grants, GrantRequest::read_only(client("reader")));
        let undo = descriptor(OperationId::EditUndo);
        let makes_page_edits = Facts { reverts: Some(Capability::DocumentEditPages), ..facts_for(1) };
        assert!(policy.decide(&editor, undo, &makes_page_edits).is_ok());
        assert_eq!(
            policy.decide(&reader, undo, &makes_page_edits).unwrap_err().reason,
            DenyReason::HistoryNotPermitted,
            "a reader may not undo an edit"
        );
        // A change whose origin the history does not say, or one that needs a capability the editor lacks, is refused.
        let unknown = facts_for(1);
        assert_eq!(policy.decide(&editor, undo, &unknown).unwrap_err().reason, DenyReason::HistoryNotPermitted);
        let needs_save = Facts { reverts: Some(Capability::DocumentSave), ..facts_for(1) };
        assert_eq!(policy.decide(&editor, undo, &needs_save).unwrap_err().reason, DenyReason::HistoryNotPermitted);
    }

    #[test]
    fn a_capability_is_limited_to_its_documents() {
        let grants = store();
        let policy = Policy::with_grants(grants.clone());
        let narrow = agent(
            &grants,
            GrantRequest {
                scopes: vec![Scope { capability: Capability::DocumentRead, documents: DocumentScope::Only(vec![key(2)]) }],
                ..GrantRequest::read_only(client("narrow"))
            },
        );
        let query = descriptor(OperationId::DocumentQuery);
        assert!(policy.decide(&narrow, query, &facts_for(2)).is_ok());
        assert_eq!(policy.decide(&narrow, query, &facts_for(3)).unwrap_err().reason, DenyReason::DocumentOutOfScope);
        assert_eq!(
            policy.decide(&narrow, query, &Facts::default()).unwrap_err().reason,
            DenyReason::DocumentOutOfScope,
            "no document named is not 'all of them'"
        );
    }

    #[test]
    fn a_file_needs_a_granted_folder_and_never_a_dotdot() {
        let grants = store();
        let policy = Policy::with_grants(grants.clone());
        let root = std::env::temp_dir().join("bdf-policy-root");
        let granted = agent(
            &grants,
            GrantRequest {
                scopes: vec![
                    Scope { capability: Capability::DocumentRead, documents: DocumentScope::All },
                    Scope { capability: Capability::DocumentSave, documents: DocumentScope::All },
                ],
                files: FileAccess { read_under: vec![root.clone()], write_under: vec![root.join("out")] },
                ..GrantRequest::read_only(client("filer"))
            },
        );
        let open = descriptor(OperationId::DocumentOpen);
        let save = descriptor(OperationId::DocumentSave);
        let inside = root.join("a.pdf");
        let outside = std::env::temp_dir().join("elsewhere").join("a.pdf");
        let sneaky = root.join("..").join("elsewhere").join("a.pdf");
        let with = |file| Facts { file: Some(file), ..facts_for(1) };
        assert!(policy.decide(&granted, open, &with(FileUse::Read(&inside))).is_ok());
        assert_eq!(policy.decide(&granted, open, &with(FileUse::Read(&outside))).unwrap_err().reason, DenyReason::FolderNotGranted);
        assert_eq!(
            policy.decide(&granted, open, &with(FileUse::Read(&sneaky))).unwrap_err().reason,
            DenyReason::FolderNotGranted,
            "a path that climbs out is not inside"
        );
        // Reading a folder is not writing into it.
        assert_eq!(policy.decide(&granted, save, &with(FileUse::Write(&inside))).unwrap_err().reason, DenyReason::FolderNotGranted);
        let out: PathBuf = root.join("out").join("b.pdf");
        assert!(policy.decide(&granted, save, &with(FileUse::Write(&out))).is_ok());
    }

    #[test]
    fn a_revoked_expired_or_foreign_grant_is_refused_and_so_is_a_service_with_no_grants() {
        let grants = store();
        let policy = Policy::with_grants(grants.clone());
        let query = descriptor(OperationId::DocumentQuery);
        let actor = agent(&grants, GrantRequest::read_only(client("soon-gone")));
        assert!(policy.decide(&actor, query, &facts_for(1)).is_ok());
        let watch = policy.watch(&actor);
        assert!(watch.check().is_ok());
        grants.revoke(actor.grant().expect("a grant")).expect("revokes");
        assert_eq!(policy.decide(&actor, query, &facts_for(1)).unwrap_err().reason, DenyReason::GrantRevoked);
        assert_eq!(watch.check().unwrap_err().reason, DenyReason::GrantRevoked, "a watch taken earlier sees the revocation");
        assert_eq!(policy.recheck(&actor).unwrap_err().reason, DenyReason::GrantRevoked);
        // An agent made by another store is unknown to a policy that does not hold its grant.
        let other = store();
        let stranger = agent(&other, GrantRequest::read_only(client("stranger")));
        assert_eq!(policy.decide(&stranger, query, &facts_for(1)).unwrap_err().reason, DenyReason::GrantUnknown);
        assert_eq!(Policy::local().decide(&stranger, query, &facts_for(1)).unwrap_err().reason, DenyReason::GrantUnknown);
        assert_eq!(Policy::local().recheck(&stranger).unwrap_err().reason, DenyReason::GrantUnknown);
    }

    #[test]
    fn a_core_only_release_refuses_deferred_and_unknown_operations_even_with_known_names() {
        let grants = store();
        let policy = Policy::with_grants(grants.clone());
        let me = ActorContext::local_user().expect("a user");
        let everything = agent(
            &grants,
            GrantRequest {
                scopes: Capability::GRANTABLE.iter().map(|c| Scope { capability: *c, documents: DocumentScope::All }).collect(),
                ..GrantRequest::read_only(client("everything"))
            },
        );
        for release in [Release::Deferred, Release::Excluded] {
            let synthetic = OperationDescriptor { release, ..DESCRIPTORS[0].clone() };
            for who in [&me, &everything] {
                let denial = policy.decide(who, &synthetic, &Facts::default()).unwrap_err();
                assert_eq!(denial.reason, DenyReason::ReleaseProfile, "{release:?}: the person and a fully granted client are both refused");
            }
        }
        for name in ["page.flatten", "document.export", "", "document.open ", "DOCUMENT.OPEN", "document.open\u{0}"] {
            for who in [&me, &everything] {
                assert_eq!(policy.decide_named(who, name, &Facts::default()).unwrap_err().reason, DenyReason::UnknownOperation, "{name:?}");
            }
        }
        assert!(policy.decide_named(&me, "document.open", &Facts::default()).is_ok());
    }
}
