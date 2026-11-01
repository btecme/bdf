//! Grants: what a delegated client may do, for how long, until when it is revoked (BUILD-008).
//!
//! A grant names one client, the operating-system account that delegated it, the capabilities it holds (each limited to
//! some documents), the folders it may read files from and write files into, and an expiry. **A new grant is read-only**:
//! [`GrantRequest::read_only`] is `document.read` on the open documents, no folders at all, and a short life. Anything
//! more is asked for by name, in the request.
//!
//! The client proves it holds a grant with a secret, shown once when the grant is issued and never stored: the store keeps
//! only its SHA-256, and compares it in constant time. The store is a private file (only the current user can read it,
//! checked before anything is written into it) of bounded size; a file that is damaged or too large is an error and is not
//! quietly replaced.
//!
//! A [`GrantStore`] is a cheap handle onto shared state, so a grant revoked through one handle (a command, the desktop's
//! settings) is seen by every service and every running operation that holds another: that is what lets revocation stop
//! work that is already queued or running.

use std::collections::HashSet;
use std::fmt;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::actors::{ActorContext, ClientId, GrantId, OsPrincipal};
use crate::operations::Capability;
use crate::version::{DocumentKey, SessionId};

/// Longest and shortest time a grant may live, in seconds. A grant is never open-ended.
pub const MAX_TTL_SECONDS: i64 = 30 * 24 * 3600;
pub const MIN_TTL_SECONDS: i64 = 60;
/// What a grant gets when its request does not say: eight hours.
pub const DEFAULT_TTL_SECONDS: i64 = 8 * 3600;
/// The most grants the store keeps (revoked and expired ones included until pruned), and the most scopes and folders one grant lists.
pub const MAX_GRANTS: usize = 64;
const MAX_SCOPES: usize = 16;
const MAX_FOLDERS: usize = 16;
const MAX_PATH_BYTES: usize = 1024;
/// The largest grant file read.
const MAX_FILE_BYTES: u64 = 256 * 1024;

/// Which documents a capability applies to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocumentScope {
    /// Every open document, in every session. This is the deliberately broad scope; a grant that must be narrower names
    /// documents.
    All,
    /// These documents only, each named by its **full identity**: the session and the number within it. A document number
    /// alone is not an identity (every session numbers its first document 1), so a grant limited to document 1 of one
    /// window cannot be used on document 1 of another. A session lasts as long as the service that made it, so a grant
    /// that names documents is good for that run and, once the session is gone, authorizes nothing: it never turns into
    /// an all-sessions grant.
    Only(Vec<DocumentKey>),
}

impl DocumentScope {
    pub fn contains(&self, document: &DocumentKey) -> bool {
        match self {
            DocumentScope::All => true,
            DocumentScope::Only(list) => list.contains(document),
        }
    }
}

/// One capability a grant holds, and over which documents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scope {
    pub capability: Capability,
    pub documents: DocumentScope,
}

/// The folders a grant may read files from and write files into. Empty means none: a document grant does not authorize any
/// file; a folder is a separate permission.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileAccess {
    pub read_under: Vec<PathBuf>,
    pub write_under: Vec<PathBuf>,
}

impl FileAccess {
    pub fn allows_read(&self, path: &Path) -> bool {
        within(&self.read_under, path)
    }

    pub fn allows_write(&self, path: &Path) -> bool {
        within(&self.write_under, path)
    }

    /// The granted folder that holds `path`, by name: the deepest one when several do. The name is only the first
    /// question; the folder that answers it is then what the file is *confined* to, so a link inside it cannot carry the
    /// access anywhere else.
    pub fn granting_root(&self, write: bool, path: &Path) -> Option<&Path> {
        let roots = if write { &self.write_under } else { &self.read_under };
        if !within(roots, path) {
            return None;
        }
        roots.iter().filter(|root| path.starts_with(root)).max_by_key(|root| root.components().count()).map(PathBuf::as_path)
    }
}

/// Whether `path` is below one of `roots`, by name, with no `..` anywhere in it.
fn within(roots: &[PathBuf], path: &Path) -> bool {
    !path.components().any(|c| matches!(c, Component::ParentDir)) && roots.iter().any(|root| path.starts_with(root))
}

/// What is asked for when issuing a grant.
#[derive(Clone, Debug)]
pub struct GrantRequest {
    pub client: ClientId,
    pub scopes: Vec<Scope>,
    pub files: FileAccess,
    pub ttl_seconds: i64,
}

impl GrantRequest {
    /// The default for a new client: read the open documents, touch no file, for eight hours.
    pub fn read_only(client: ClientId) -> Self {
        Self {
            client,
            scopes: vec![Scope { capability: Capability::DocumentRead, documents: DocumentScope::All }],
            files: FileAccess::default(),
            ttl_seconds: DEFAULT_TTL_SECONDS,
        }
    }
}

/// A grant as the policy sees it. The secret's hash is not part of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrantTerms {
    pub id: GrantId,
    pub client: ClientId,
    pub scopes: Vec<Scope>,
    pub files: FileAccess,
    pub issued_at: i64,
    pub expires_at: i64,
}

impl GrantTerms {
    /// The scope for `capability`, if the grant holds it.
    pub fn scope_for(&self, capability: Capability) -> Option<&Scope> {
        self.scopes.iter().find(|s| s.capability == capability)
    }
}

/// A grant that was just issued, with the secret the client needs: the only time it exists in readable form.
pub struct IssuedGrant {
    pub terms: GrantTerms,
    secret: String,
}

impl IssuedGrant {
    /// The secret to hand to the client. It is not stored and cannot be shown again.
    pub fn secret(&self) -> &str {
        &self.secret
    }
}

impl fmt::Debug for IssuedGrant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IssuedGrant").field("terms", &self.terms).field("secret", &"<hidden>").finish()
    }
}

/// Why a grant could not be issued, revoked or stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GrantError {
    /// The request asks for something that is not allowed.
    Invalid(String),
    /// The store is full.
    Full,
    NotFound,
    /// The grant file could not be read or written, or is not private.
    Storage(String),
    /// The grant file is damaged or not in a form this version reads.
    Corrupt(String),
}

impl fmt::Display for GrantError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GrantError::Invalid(why) => write!(f, "{why}"),
            GrantError::Full => write!(f, "the grant store is full ({MAX_GRANTS} grants): revoke and prune some"),
            GrantError::NotFound => write!(f, "no such grant"),
            GrantError::Storage(why) => write!(f, "the grant file could not be used: {why}"),
            GrantError::Corrupt(why) => write!(f, "the grant file is damaged: {why}"),
        }
    }
}

impl std::error::Error for GrantError {}

/// A client that could not be authenticated. One answer for every reason (no such client, wrong secret, revoked, expired,
/// another account), so it says nothing about which clients exist.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthError;

impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("authentication failed")
    }
}

impl std::error::Error for AuthError {}

/// Why an actor's grant does not hold right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrantProblem {
    Unknown,
    Revoked,
    Expired,
    /// The grant belongs to a different operating-system account than the actor.
    WrongAccount,
}

impl GrantProblem {
    pub fn as_str(self) -> &'static str {
        match self {
            GrantProblem::Unknown => "grant_unknown",
            GrantProblem::Revoked => "grant_revoked",
            GrantProblem::Expired => "grant_expired",
            GrantProblem::WrongAccount => "grant_wrong_account",
        }
    }
}

#[derive(Clone, Debug)]
struct Grant {
    terms: GrantTerms,
    delegating: OsPrincipal,
    revoked: bool,
    secret_sha256: [u8; 32],
}

struct State {
    grants: Vec<Grant>,
    path: Option<PathBuf>,
    clock: Clock,
    owner: OsPrincipal,
    /// What the grant file looked like when it was last read or written by this store.
    stamp: Option<Stamp>,
    /// The grant file changed and could not be read (damaged, not private, deleted): no grant holds until it can be.
    unreadable: bool,
}

/// Enough of a file to notice that another process changed it: its length and modification time. Revoking a grant flips
/// `false` to `true`, which changes the length, so a revocation is never missed even on a volume with coarse times.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stamp {
    len: u64,
    modified: Option<std::time::SystemTime>,
}

fn stamp_of(path: &Path) -> Option<Stamp> {
    let meta = std::fs::symlink_metadata(path).ok().filter(std::fs::Metadata::is_file)?;
    Some(Stamp { len: meta.len(), modified: meta.modified().ok() })
}

impl State {
    /// Read the grant file again if another process changed it since this store last looked. A file that cannot be read
    /// makes every grant fail until it can be (a revocation must not be lost to a file that was damaged or deleted).
    fn refresh(&mut self) {
        let Some(path) = self.path.clone() else { return };
        if stamp_of(&path) == self.stamp {
            // Unchanged since it was last read, whether that read worked or not.
            return;
        }
        self.reload();
    }

    /// Read the grant file again, whatever the stamp says. A change is made on what the file holds *now*, so a writer
    /// never trusts a stamp: two files of the same length and time are not the same file.
    fn reload(&mut self) {
        let Some(path) = self.path.clone() else { return };
        let now = stamp_of(&path);
        match load(&path, &self.owner) {
            Ok(grants) => {
                self.grants = grants;
                self.unreadable = false;
            }
            Err(_) => self.unreadable = true,
        }
        self.stamp = now;
    }
}

/// The right to change the grant file, held across processes: a lock on a file beside it, released when this is dropped
/// (or when the process ends, however it ends). A change is read, made and written while it is held, so two processes
/// that change the grants at once take turns instead of one writing over the other's change (which would bring back a
/// grant the other had just revoked).
struct FileLock {
    _held: crate::file_lock::FileLock,
}

/// How long a change waits for another process that is changing the grants. A change takes milliseconds.
const LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

impl FileLock {
    fn acquire(grants: &Path) -> Result<Self, GrantError> {
        use crate::file_lock::LockError;
        let mut name = grants.as_os_str().to_owned();
        name.push(".lock");
        match crate::file_lock::FileLock::acquire(&PathBuf::from(name), LOCK_WAIT) {
            Ok(held) => Ok(Self { _held: held }),
            Err(LockError::Busy) => Err(GrantError::Storage("another process is changing the grants; try again".into())),
            Err(LockError::Io(e)) => Err(GrantError::Storage(e.to_string())),
        }
    }
}

impl State {
    /// A store whose file cannot be read does not write over it.
    fn writable(&self) -> Result<(), GrantError> {
        if self.unreadable {
            return Err(GrantError::Storage("the grant file changed and can't be read; it was left alone".into()));
        }
        Ok(())
    }
}

/// The grants in the private file at `path`; none if there is no file yet.
fn load(path: &Path, owner: &OsPrincipal) -> Result<Vec<Grant>, GrantError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => {
            pdfcraft_platform::private_files::verify_private(path).map_err(|e| GrantError::Storage(format!("not private: {e}")))?;
            let (bytes, _) =
                pdfcraft_platform::file_identity::read_regular_with_version(path, MAX_FILE_BYTES).map_err(|e| GrantError::Storage(e.to_string()))?;
            parse_file(&bytes, owner)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(GrantError::Storage(e.to_string())),
    }
}

/// Where the store reads the time from.
type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// The grants of one operating-system user. Clone it to share it.
#[derive(Clone)]
pub struct GrantStore {
    state: Arc<Mutex<State>>,
}

/// Seconds since the Unix epoch. The browser build has no clock to read (asking would panic), so there it is 0.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn system_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok().and_then(|d| i64::try_from(d.as_secs()).ok()).unwrap_or(0)
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn system_now() -> i64 {
    0
}

impl GrantStore {
    /// A store that lives in memory only, for `owner`.
    pub fn in_memory(owner: OsPrincipal) -> Self {
        Self {
            state: Arc::new(Mutex::new(State { grants: Vec::new(), path: None, clock: Arc::new(system_now), owner, stamp: None, unreadable: false })),
        }
    }

    /// The store kept in the private file `path`, for `owner` (created empty if it does not exist). A file that is not
    /// private to the current user, too large or damaged is an error: a grant file is never silently replaced.
    pub fn open(path: &Path, owner: OsPrincipal) -> Result<Self, GrantError> {
        let grants = load(path, &owner)?;
        let state = State { grants, path: Some(path.to_path_buf()), clock: Arc::new(system_now), owner, stamp: stamp_of(path), unreadable: false };
        let store = Self { state: Arc::new(Mutex::new(state)) };
        // Creating the file is a change like any other: a second process starting at the same moment must not replace a
        // grant the first has already issued with an empty list.
        store.transact(|state| if path.exists() { Ok(()) } else { persist(state) })?;
        Ok(store)
    }

    /// Use `clock` (seconds since the Unix epoch) instead of the system's, for deterministic tests.
    pub fn with_clock(self, clock: impl Fn() -> i64 + Send + Sync + 'static) -> Self {
        self.lock().clock = Arc::new(clock);
        self
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        // A panic while the lock was held cannot leave the grants in a state that grants more than they did.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Change the grants: take turns with every other process that changes them, read the file as it is now, let `change`
    /// make its change (and write it), and note what the file looks like afterwards. The in-process lock is taken
    /// first, then the file's, always in that order.
    fn transact<T>(&self, change: impl FnOnce(&mut State) -> Result<T, GrantError>) -> Result<T, GrantError> {
        let mut state = self.lock();
        let _held = match state.path.as_deref() {
            Some(path) => Some(FileLock::acquire(path)?),
            None => None,
        };
        state.reload();
        state.writable()?;
        let result = change(&mut state);
        state.stamp = state.path.as_deref().and_then(stamp_of);
        result
    }

    /// Issue a grant for the request, returning it with its secret.
    pub fn issue(&self, request: GrantRequest) -> Result<IssuedGrant, GrantError> {
        validate(&request)?;
        self.transact(|state| Self::issue_in(state, request))
    }

    fn issue_in(state: &mut State, request: GrantRequest) -> Result<IssuedGrant, GrantError> {
        if state.grants.len() >= MAX_GRANTS {
            return Err(GrantError::Full);
        }
        let secret_bytes: [u8; 32] = random_bytes().map_err(|e| GrantError::Storage(format!("no randomness: {e}")))?;
        let id_bytes: [u8; 8] = random_bytes().map_err(|e| GrantError::Storage(format!("no randomness: {e}")))?;
        let secret = format!("bdfg_{}", hex(&secret_bytes));
        let now = (state.clock)();
        let terms = GrantTerms {
            id: GrantId::from_random(id_bytes),
            client: request.client,
            scopes: request.scopes,
            files: request.files,
            issued_at: now,
            expires_at: now.saturating_add(request.ttl_seconds),
        };
        let grant =
            Grant { terms: terms.clone(), delegating: state.owner.clone(), revoked: false, secret_sha256: Sha256::digest(secret.as_bytes()).into() };
        state.grants.push(grant);
        if let Err(e) = persist(state) {
            state.grants.pop();
            return Err(e);
        }
        Ok(IssuedGrant { terms, secret })
    }

    /// Withdraw a grant. Every service and every operation holding this store sees it at once.
    pub fn revoke(&self, id: &GrantId) -> Result<(), GrantError> {
        self.transact(|state| {
            let grant = state.grants.iter_mut().find(|g| &g.terms.id == id).ok_or(GrantError::NotFound)?;
            if grant.revoked {
                return Ok(());
            }
            grant.revoked = true;
            // The revocation holds even if it could not be written: it stays in memory for this run, and the failure is reported.
            persist(state)
        })
    }

    /// Remove grants that are revoked or expired, freeing room.
    pub fn prune(&self) -> Result<usize, GrantError> {
        self.transact(|state| {
            let now = (state.clock)();
            let before = state.grants.len();
            let kept: Vec<Grant> = state.grants.iter().filter(|g| !g.revoked && g.terms.expires_at > now).cloned().collect();
            let removed = before.saturating_sub(kept.len());
            state.grants = kept;
            persist(state)?;
            Ok(removed)
        })
    }

    /// Every grant, without its secret hash.
    pub fn list(&self) -> Vec<(GrantTerms, bool)> {
        let mut state = self.lock();
        state.refresh();
        state.grants.iter().map(|g| (g.terms.clone(), g.revoked)).collect()
    }

    /// Turn a client's name and secret into an agent actor, if they match a grant that is not revoked or expired and that
    /// this operating-system user delegated. Every failure is the same [`AuthError`].
    pub fn authenticate(&self, client: &str, secret: &str) -> Result<ActorContext, AuthError> {
        let presented: [u8; 32] = Sha256::digest(secret.as_bytes()).into();
        let client = ClientId::parse(client).ok();
        let mut state = self.lock();
        state.refresh();
        if state.unreadable {
            return Err(AuthError);
        }
        let now = (state.clock)();
        // Every grant is compared so that the time taken does not say which client names exist.
        let mut found: Option<&Grant> = None;
        for grant in &state.grants {
            let same_secret = constant_time_eq(&grant.secret_sha256, &presented);
            let live = !grant.revoked && grant.terms.expires_at > now && grant.delegating == state.owner;
            if same_secret && live && client.as_ref() == Some(&grant.terms.client) && found.is_none() {
                found = Some(grant);
            }
        }
        let grant = found.ok_or(AuthError)?;
        Ok(ActorContext::agent(state.owner.clone(), grant.terms.client.clone(), grant.terms.id.clone()))
    }

    /// The terms of the actor's grant, if it holds right now. Called when a request arrives, again immediately before each
    /// effect, and again before data is handed back.
    pub fn check(&self, actor: &ActorContext) -> Result<GrantTerms, GrantProblem> {
        let (Some(id), Some(client)) = (actor.grant(), actor.client()) else { return Err(GrantProblem::Unknown) };
        let mut state = self.lock();
        state.refresh();
        if state.unreadable {
            return Err(GrantProblem::Unknown);
        }
        let grant = state.grants.iter().find(|g| &g.terms.id == id && &g.terms.client == client).ok_or(GrantProblem::Unknown)?;
        if grant.revoked {
            return Err(GrantProblem::Revoked);
        }
        if grant.terms.expires_at <= (state.clock)() {
            return Err(GrantProblem::Expired);
        }
        if &grant.delegating != actor.os() {
            return Err(GrantProblem::WrongAccount);
        }
        Ok(grant.terms.clone())
    }
}

fn validate(request: &GrantRequest) -> Result<(), GrantError> {
    let invalid = |why: &str| Err(GrantError::Invalid(why.to_owned()));
    if !(MIN_TTL_SECONDS..=MAX_TTL_SECONDS).contains(&request.ttl_seconds) {
        return invalid("a grant lives between a minute and thirty days");
    }
    if request.scopes.is_empty() || request.scopes.len() > MAX_SCOPES {
        return invalid("a grant holds between one and sixteen capabilities");
    }
    let mut seen = HashSet::new();
    for scope in &request.scopes {
        if !seen.insert(scope.capability) {
            return invalid("a capability is listed twice");
        }
        if let DocumentScope::Only(list) = &scope.documents
            && (list.is_empty() || list.len() > 64)
        {
            return invalid("a document list names between one and sixty-four documents");
        }
    }
    for (what, list) in [("read", &request.files.read_under), ("write", &request.files.write_under)] {
        if list.len() > MAX_FOLDERS {
            return invalid("too many folders");
        }
        for folder in list {
            let text = folder.to_str().ok_or_else(|| GrantError::Invalid("a folder name is not valid text".into()))?;
            if text.is_empty() || text.len() > MAX_PATH_BYTES || text.contains('\0') || !folder.is_absolute() {
                return Err(GrantError::Invalid(format!("a {what} folder must be an absolute path of reasonable length")));
            }
            if folder.components().any(|c| matches!(c, Component::ParentDir)) {
                return Err(GrantError::Invalid(format!("a {what} folder may not contain '..'")));
            }
        }
    }
    Ok(())
}

fn persist(state: &State) -> Result<(), GrantError> {
    let Some(path) = &state.path else { return Ok(()) };
    let grants: Vec<Value> = state.grants.iter().map(grant_json).collect();
    let bytes = serde_json::to_vec_pretty(&json!({ "version": 1, "grants": grants })).map_err(|e| GrantError::Storage(e.to_string()))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_FILE_BYTES {
        return Err(GrantError::Full);
    }
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| GrantError::Storage(e.to_string()))?;
    }
    pdfcraft_platform::private_files::write_private_file(path, &bytes, pdfcraft_platform::staging::StagingName::SuffixThenTag)
        .map_err(|e| GrantError::Storage(e.to_string()))
}

fn grant_json(g: &Grant) -> Value {
    let scopes: Vec<Value> = g
        .terms
        .scopes
        .iter()
        .map(|s| {
            let documents = match &s.documents {
                DocumentScope::All => json!("all"),
                DocumentScope::Only(list) => {
                    json!(list.iter().map(|k| json!({ "session": k.session.0, "document": k.document })).collect::<Vec<_>>())
                }
            };
            json!({ "capability": s.capability.name(), "documents": documents })
        })
        .collect();
    let paths = |list: &[PathBuf]| -> Vec<String> { list.iter().filter_map(|p| p.to_str().map(str::to_owned)).collect() };
    json!({
        "id": g.terms.id.as_str(),
        "client": g.terms.client.as_str(),
        "delegating": g.delegating.as_str(),
        "scopes": scopes,
        "files": { "read": paths(&g.terms.files.read_under), "write": paths(&g.terms.files.write_under) },
        "issued": g.terms.issued_at,
        "expires": g.terms.expires_at,
        "revoked": g.revoked,
        "secret_sha256": hex(&g.secret_sha256),
    })
}

fn parse_file(bytes: &[u8], owner: &OsPrincipal) -> Result<Vec<Grant>, GrantError> {
    let corrupt = |why: &str| GrantError::Corrupt(why.to_owned());
    let root: Value = serde_json::from_slice(bytes).map_err(|e| GrantError::Corrupt(e.to_string()))?;
    if root.get("version").and_then(Value::as_u64) != Some(1) {
        return Err(corrupt("an unknown version"));
    }
    let list = root.get("grants").and_then(Value::as_array).ok_or_else(|| corrupt("no grant list"))?;
    if list.len() > MAX_GRANTS {
        return Err(corrupt("too many grants"));
    }
    let mut out = Vec::new();
    for item in list {
        out.push(parse_grant(item).ok_or_else(|| corrupt("a grant is malformed"))?);
    }
    // Grants delegated by another account are not this user's: they are dropped, never honoured.
    out.retain(|g| &g.delegating == owner);
    Ok(out)
}

fn parse_grant(item: &Value) -> Option<Grant> {
    let id = GrantId::parse(item.get("id")?.as_str()?)?;
    let client = ClientId::parse(item.get("client")?.as_str()?).ok()?;
    let delegating = OsPrincipal::from_stored(item.get("delegating")?.as_str()?)?;
    let mut scopes = Vec::new();
    for s in item.get("scopes")?.as_array()? {
        let capability = Capability::parse(s.get("capability")?.as_str()?)?;
        let documents = match s.get("documents")? {
            Value::String(text) if text == "all" => DocumentScope::All,
            Value::Array(list) => DocumentScope::Only(
                list.iter()
                    .map(|k| Some(DocumentKey { session: SessionId(k.get("session")?.as_u64()?), document: k.get("document")?.as_u64()? }))
                    .collect::<Option<Vec<_>>>()?,
            ),
            _ => return None,
        };
        scopes.push(Scope { capability, documents });
    }
    let folders =
        |key: &str| -> Option<Vec<PathBuf>> { item.get("files")?.get(key)?.as_array()?.iter().map(|v| v.as_str().map(PathBuf::from)).collect() };
    let files = FileAccess { read_under: folders("read")?, write_under: folders("write")? };
    let secret = unhex32(item.get("secret_sha256")?.as_str()?)?;
    Some(Grant {
        terms: GrantTerms { id, client, scopes, files, issued_at: item.get("issued")?.as_i64()?, expires_at: item.get("expires")?.as_i64()? },
        delegating,
        revoked: item.get("revoked")?.as_bool()?,
        secret_sha256: secret,
    })
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex32(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 || !text.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(text.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

fn constant_time_eq(a: &[u8; 32], b: &[u8; 32]) -> bool {
    a.iter().zip(b.iter()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn random_bytes<const N: usize>() -> io::Result<[u8; N]> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).map_err(|e| io::Error::other(e.to_string()))?;
    Ok(bytes)
}

/// No source of randomness is available here (the web build has no grants to issue).
#[cfg(target_arch = "wasm32")]
pub(crate) fn random_bytes<const N: usize>() -> io::Result<[u8; N]> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "no source of randomness on this platform"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{make_shared, private_dir};
    use std::sync::atomic::{AtomicI64, Ordering};

    const START: i64 = 1_000_000;

    /// A clock that belongs to one test: tests run in parallel, so a clock they share would be moved under each other.
    #[derive(Clone)]
    struct TestClock(Arc<AtomicI64>);

    impl TestClock {
        fn new() -> Self {
            Self(Arc::new(AtomicI64::new(START)))
        }

        fn set(&self, now: i64) {
            self.0.store(now, Ordering::SeqCst);
        }

        fn reading(&self) -> impl Fn() -> i64 + Send + Sync + use<> {
            let now = self.0.clone();
            move || now.load(Ordering::SeqCst)
        }
    }

    /// A new grant lives no longer than a working day, checked when the crate is built.
    const _: () = assert!(DEFAULT_TTL_SECONDS <= 8 * 3600);

    fn owner() -> OsPrincipal {
        OsPrincipal::current().expect("a user")
    }

    fn store() -> (GrantStore, TestClock) {
        let clock = TestClock::new();
        (GrantStore::in_memory(owner()).with_clock(clock.reading()), clock)
    }

    fn client(name: &str) -> ClientId {
        ClientId::parse(name).expect("a client name")
    }

    #[test]
    fn a_new_grant_is_read_only_with_no_folders_and_a_short_life() {
        let request = GrantRequest::read_only(client("reader"));
        assert_eq!(request.scopes, [Scope { capability: Capability::DocumentRead, documents: DocumentScope::All }]);
        assert_eq!(request.files, FileAccess::default());
        assert_eq!(request.ttl_seconds, DEFAULT_TTL_SECONDS);
    }

    #[test]
    fn the_secret_opens_its_own_grant_and_nothing_else_does_and_every_failure_looks_the_same() {
        let (grants, _) = store();
        let issued = grants.issue(GrantRequest::read_only(client("reader"))).expect("issues");
        let other = grants.issue(GrantRequest::read_only(client("other"))).expect("issues");
        assert!(issued.secret().starts_with("bdfg_") && issued.secret().len() == 5 + 64);
        assert_ne!(issued.secret(), other.secret());
        assert!(!format!("{issued:?}").contains(issued.secret()), "a debug print hides the secret");

        let actor = grants.authenticate("reader", issued.secret()).expect("authenticates");
        assert_eq!((actor.client().map(ClientId::as_str), actor.grant()), (Some("reader"), Some(&issued.terms.id)));
        assert!(actor.is_agent());
        // Wrong secret, another client's secret, an unknown or malformed name, an empty secret: one answer.
        for (name, secret) in [
            ("reader", "bdfg_wrong"),
            ("reader", other.secret()),
            ("other", issued.secret()),
            ("nobody", issued.secret()),
            ("Reader", issued.secret()),
            ("", issued.secret()),
            ("reader", ""),
            ("reader", &issued.secret().to_uppercase()),
        ] {
            assert_eq!(grants.authenticate(name, secret).expect_err("refused"), AuthError, "{name} / {secret}");
        }
    }

    #[test]
    fn a_revoked_or_expired_grant_no_longer_authenticates_or_holds() {
        let (grants, clock) = store();
        let issued = grants.issue(GrantRequest { ttl_seconds: 3600, ..GrantRequest::read_only(client("short")) }).expect("issues");
        let actor = grants.authenticate("short", issued.secret()).expect("authenticates");
        assert!(grants.check(&actor).is_ok());
        // Expiry: the clock moves past it.
        clock.set(START + 3600);
        assert_eq!(grants.check(&actor).unwrap_err(), GrantProblem::Expired);
        assert_eq!(grants.authenticate("short", issued.secret()).unwrap_err(), AuthError);

        // Revocation: seen at once through any handle.
        clock.set(START);
        let handle = grants.clone();
        assert!(grants.check(&actor).is_ok());
        handle.revoke(&issued.terms.id).expect("revokes");
        assert_eq!(grants.check(&actor).unwrap_err(), GrantProblem::Revoked);
        assert_eq!(grants.authenticate("short", issued.secret()).unwrap_err(), AuthError);
        handle.revoke(&issued.terms.id).expect("revoking twice is fine");
        assert_eq!(handle.revoke(&GrantId::parse("g-0000000000000000").expect("an id")).unwrap_err(), GrantError::NotFound);
    }

    #[test]
    fn a_grant_of_another_account_does_not_hold_for_this_one() {
        let (grants, _) = store();
        let issued = grants.issue(GrantRequest::read_only(client("reader"))).expect("issues");
        let stranger = OsPrincipal::from_stored("S-1-5-21-1-2-3-9999").expect("an account");
        let actor = ActorContext::agent(stranger, client("reader"), issued.terms.id.clone());
        assert_eq!(grants.check(&actor).unwrap_err(), GrantProblem::WrongAccount);
        // And a human context has no grant at all.
        assert_eq!(grants.check(&ActorContext::local_user().expect("a user")).unwrap_err(), GrantProblem::Unknown);
    }

    #[test]
    fn requests_are_validated() {
        let (grants, _) = store();
        let ok = || GrantRequest::read_only(client("c"));
        let refuse = |request: GrantRequest, what: &str| {
            assert!(matches!(grants.issue(request), Err(GrantError::Invalid(_))), "{what}");
        };
        refuse(GrantRequest { ttl_seconds: 0, ..ok() }, "no life");
        refuse(GrantRequest { ttl_seconds: MIN_TTL_SECONDS - 1, ..ok() }, "too short");
        refuse(GrantRequest { ttl_seconds: MAX_TTL_SECONDS + 1, ..ok() }, "too long");
        refuse(GrantRequest { ttl_seconds: -5, ..ok() }, "negative");
        refuse(GrantRequest { scopes: Vec::new(), ..ok() }, "nothing granted");
        refuse(
            GrantRequest {
                scopes: vec![
                    Scope { capability: Capability::DocumentRead, documents: DocumentScope::All },
                    Scope {
                        capability: Capability::DocumentRead,
                        documents: DocumentScope::Only(vec![DocumentKey { session: SessionId(1), document: 1 }]),
                    },
                ],
                ..ok()
            },
            "a capability twice",
        );
        refuse(
            GrantRequest { scopes: vec![Scope { capability: Capability::DocumentRead, documents: DocumentScope::Only(Vec::new()) }], ..ok() },
            "no documents",
        );
        let relative = FileAccess { read_under: vec![PathBuf::from("relative/folder")], write_under: Vec::new() };
        refuse(GrantRequest { files: relative, ..ok() }, "a relative folder");
        let absolute = std::env::temp_dir();
        let climbing = FileAccess { read_under: Vec::new(), write_under: vec![absolute.join("..").join("elsewhere")] };
        refuse(GrantRequest { files: climbing, ..ok() }, "a folder that climbs");
        let many = FileAccess { read_under: (0..=MAX_FOLDERS).map(|i| absolute.join(format!("f{i}"))).collect(), write_under: Vec::new() };
        refuse(GrantRequest { files: many, ..ok() }, "too many folders");
        assert!(grants.list().is_empty(), "a refused request stored nothing");
        grants.issue(GrantRequest { files: FileAccess { read_under: vec![absolute], write_under: Vec::new() }, ..ok() }).expect("a good one");
    }

    #[test]
    fn the_store_is_bounded_and_prune_makes_room() {
        let (grants, clock) = store();
        let mut ids = Vec::new();
        for n in 0..MAX_GRANTS {
            ids.push(grants.issue(GrantRequest::read_only(client(&format!("c{n}")))).expect("issues").terms.id);
        }
        assert_eq!(grants.issue(GrantRequest::read_only(client("one-more"))).unwrap_err(), GrantError::Full);
        grants.revoke(&ids[0]).expect("revokes");
        assert_eq!(grants.prune().expect("prunes"), 1);
        grants.issue(GrantRequest::read_only(client("one-more"))).expect("room again");
        clock.set(START + DEFAULT_TTL_SECONDS + 1);
        assert_eq!(grants.prune().expect("prunes"), MAX_GRANTS, "expired grants go too");
    }

    #[test]
    fn grants_persist_in_a_private_file_that_holds_no_secret() {
        let dir = private_dir("grants-persist");
        let path = dir.join("grants.json");
        let clock = TestClock::new();
        let first = GrantStore::open(&path, owner()).expect("creates").with_clock(clock.reading());
        let issued = first.issue(GrantRequest::read_only(client("kept"))).expect("issues");
        let text = std::fs::read_to_string(&path).expect("reads");
        assert!(!text.contains(issued.secret()) && !text.contains(&issued.secret()[5..]), "only a hash is stored: {text}");
        pdfcraft_platform::private_files::verify_private(&path).expect("the file is private");

        // Another process opens it, and the secret still works; a revocation is kept too.
        let second = GrantStore::open(&path, owner()).expect("reopens").with_clock(clock.reading());
        let actor = second.authenticate("kept", issued.secret()).expect("authenticates");
        second.revoke(&issued.terms.id).expect("revokes");
        let third = GrantStore::open(&path, owner()).expect("reopens").with_clock(clock.reading());
        assert_eq!(third.check(&actor).unwrap_err(), GrantProblem::Revoked, "a revocation outlives the run");
        assert_eq!(third.authenticate("kept", issued.secret()).unwrap_err(), AuthError);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_write_leaves_the_store_as_it_was() {
        let dir = private_dir("grants-fail");
        let path = dir.join("grants.json");
        let clock = TestClock::new();
        let grants = GrantStore::open(&path, owner()).expect("creates").with_clock(clock.reading());
        grants.issue(GrantRequest::read_only(client("first"))).expect("issues");
        // The file's place is taken by a folder: it cannot be replaced.
        std::fs::remove_file(&path).expect("removes");
        std::fs::create_dir(&path).expect("makes a folder");
        let error = grants.issue(GrantRequest::read_only(client("second"))).unwrap_err();
        assert!(matches!(error, GrantError::Storage(_)), "{error}");
        assert_eq!(grants.list().len(), 1, "the grant that could not be stored was not kept");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_damaged_oversized_or_non_private_file_is_an_error_and_is_not_replaced() {
        let dir = private_dir("grants-bad");
        let path = dir.join("grants.json");
        for (what, content) in [
            ("not json", "garbage".to_owned()),
            ("a wrong version", r#"{"version":2,"grants":[]}"#.to_owned()),
            ("no list", r#"{"version":1}"#.to_owned()),
            ("a malformed grant", r#"{"version":1,"grants":[{"id":"x"}]}"#.to_owned()),
            ("too big", format!("{{\"version\":1,\"grants\":[],\"pad\":\"{}\"}}", "x".repeat(300 * 1024))),
        ] {
            std::fs::write(&path, &content).expect("writes");
            let error = match GrantStore::open(&path, owner()) {
                Err(e) => e,
                Ok(_) => panic!("{what}: opened"),
            };
            assert!(matches!(error, GrantError::Corrupt(_) | GrantError::Storage(_)), "{what}: {error}");
            assert_eq!(std::fs::read_to_string(&path).expect("reads"), content, "{what}: the file was left alone");
        }
        let _ = std::fs::remove_dir_all(&dir);

        // A file other accounts can read is refused outright, before it is parsed.
        let shared = private_dir("grants-shared");
        make_shared(&shared);
        let path = shared.join("grants.json");
        std::fs::write(&path, r#"{"version":1,"grants":[]}"#).expect("writes");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("opens the file");
        }
        let error = match GrantStore::open(&path, owner()) {
            Err(e) => e,
            Ok(_) => panic!("a shared file was opened"),
        };
        assert!(matches!(&error, GrantError::Storage(why) if why.contains("not private")), "{error}");
        let _ = std::fs::remove_dir_all(&shared);
    }

    #[test]
    fn grants_delegated_by_another_account_are_dropped_not_honoured() {
        let dir = private_dir("grants-foreign");
        let path = dir.join("grants.json");
        let clock = TestClock::new();
        let store = GrantStore::open(&path, owner()).expect("creates").with_clock(clock.reading());
        let issued = store.issue(GrantRequest::read_only(client("mine"))).expect("issues");
        let text = std::fs::read_to_string(&path).expect("reads");
        let mine = owner();
        let edited = text.replace(mine.as_str(), "S-1-5-21-1-2-3-9999");
        assert_ne!(text, edited);
        std::fs::write(&path, edited).expect("writes");
        let reopened = GrantStore::open(&path, owner()).expect("opens").with_clock(clock.reading());
        assert!(reopened.list().is_empty(), "a grant that someone else delegated is not this user's");
        assert_eq!(reopened.authenticate("mine", issued.secret()).unwrap_err(), AuthError);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hex_and_comparison_helpers_are_exact() {
        assert_eq!(hex(&[0, 1, 0xab, 0xff]), "0001abff");
        assert_eq!(unhex32(&"ab".repeat(32)), Some([0xab; 32]));
        for bad in ["", "ab", &"zz".repeat(32), &"ab".repeat(33), &"é".repeat(32)] {
            assert_eq!(unhex32(bad), None, "{bad:?}");
        }
        let a = [7u8; 32];
        let mut b = a;
        assert!(constant_time_eq(&a, &b));
        b[31] ^= 1;
        assert!(!constant_time_eq(&a, &b));
    }

    #[test]
    fn a_revocation_made_by_another_process_is_seen_by_a_running_one() {
        let dir = private_dir("grants-processes");
        let path = dir.join("grants.json");
        let clock = TestClock::new();
        // Two stores on one file stand for two processes: a running service, and a command that manages grants.
        let running = GrantStore::open(&path, owner()).expect("opens").with_clock(clock.reading());
        let manager = GrantStore::open(&path, owner()).expect("opens").with_clock(clock.reading());

        // A grant issued by the other process can be authenticated by the running one without restarting it.
        let issued = manager.issue(GrantRequest::read_only(client("later"))).expect("issues");
        let actor = running.authenticate("later", issued.secret()).expect("the running process sees the new grant");
        assert!(running.check(&actor).is_ok());

        // And its revocation is seen the next time anything asks.
        manager.revoke(&issued.terms.id).expect("revokes");
        assert_eq!(running.check(&actor).unwrap_err(), GrantProblem::Revoked);
        assert_eq!(running.authenticate("later", issued.secret()).unwrap_err(), AuthError);
        assert!(running.list().iter().any(|(terms, revoked)| terms.id == issued.terms.id && *revoked));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_change_is_made_on_what_the_file_holds_now_even_when_it_looks_unchanged() {
        let dir = private_dir("grants-stale-stamp");
        let path = dir.join("grants.json");
        let clock = TestClock::new();
        let store = GrantStore::open(&path, owner()).expect("opens").with_clock(clock.reading());
        let issued = store.issue(GrantRequest::read_only(client("aaaa"))).expect("issues");
        let before = std::fs::metadata(&path).expect("metadata");

        // Another process rewrites the file with a same-length change and puts the modification time back: by length and
        // time the file is exactly as this store last saw it.
        let edited = std::fs::read_to_string(&path).expect("reads").replace("\"client\": \"aaaa\"", "\"client\": \"bbbb\"");
        std::fs::write(&path, &edited).expect("edits");
        let file = std::fs::OpenOptions::new().write(true).open(&path).expect("opens the file");
        file.set_modified(before.modified().expect("a time")).expect("restores the time");
        drop(file);
        assert_eq!(std::fs::metadata(&path).expect("metadata").len(), before.len());

        // A change made now is made on the file's content, so the other process's edit is not written over.
        store.revoke(&issued.terms.id).expect("revokes");
        let now = std::fs::read_to_string(&path).expect("reads");
        assert!(now.contains("\"client\": \"bbbb\""), "the other process's change was kept: {now}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_grant_file_that_stops_being_readable_fails_closed_and_is_left_alone() {
        let dir = private_dir("grants-fail-closed");
        let path = dir.join("grants.json");
        let clock = TestClock::new();
        let store = GrantStore::open(&path, owner()).expect("opens").with_clock(clock.reading());
        let issued = store.issue(GrantRequest::read_only(client("careful"))).expect("issues");
        let actor = store.authenticate("careful", issued.secret()).expect("authenticates");
        let good = std::fs::read(&path).expect("reads");

        // Damaged: nothing holds, nothing is written over it, and it recovers when the file does.
        std::fs::write(&path, b"{ not json").expect("damages");
        assert_eq!(store.check(&actor).unwrap_err(), GrantProblem::Unknown);
        assert_eq!(store.authenticate("careful", issued.secret()).unwrap_err(), AuthError);
        assert!(matches!(store.issue(GrantRequest::read_only(client("other"))), Err(GrantError::Storage(_))));
        assert!(matches!(store.revoke(&issued.terms.id), Err(GrantError::Storage(_))));
        assert_eq!(std::fs::read(&path).expect("reads"), b"{ not json", "the damaged file was not written over");
        std::fs::write(&path, &good).expect("restores");
        assert!(store.check(&actor).is_ok(), "it recovers when the file is readable again");

        // Deleted: the grants are gone, so nothing holds.
        std::fs::remove_file(&path).expect("deletes");
        assert_eq!(store.check(&actor).unwrap_err(), GrantProblem::Unknown);
        assert_eq!(store.authenticate("careful", issued.secret()).unwrap_err(), AuthError);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_grant_file_that_becomes_readable_by_other_accounts_fails_closed() {
        let dir = private_dir("grants-opened-up");
        let path = dir.join("grants.json");
        let clock = TestClock::new();
        let store = GrantStore::open(&path, owner()).expect("opens").with_clock(clock.reading());
        let issued = store.issue(GrantRequest::read_only(client("exposed"))).expect("issues");
        let actor = store.authenticate("exposed", issued.secret()).expect("authenticates");
        let text = std::fs::read(&path).expect("reads");
        // The file is replaced by a copy that other accounts can read (a folder that hands its files to everyone).
        make_shared(&dir);
        std::fs::remove_file(&path).expect("removes");
        std::fs::write(&path, &text).expect("rewrites");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("opens the file");
        }
        assert_eq!(store.check(&actor).unwrap_err(), GrantProblem::Unknown, "a grant file anyone could have edited is not trusted");
        assert_eq!(store.authenticate("exposed", issued.secret()).unwrap_err(), AuthError);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_folder_that_grants_a_file_is_the_deepest_one_that_holds_it_and_read_and_write_are_separate() {
        let files =
            FileAccess { read_under: vec![PathBuf::from("/data"), PathBuf::from("/data/work")], write_under: vec![PathBuf::from("/data/work/out")] };
        assert_eq!(files.granting_root(false, Path::new("/data/work/a.pdf")), Some(Path::new("/data/work")));
        assert_eq!(files.granting_root(false, Path::new("/data/b.pdf")), Some(Path::new("/data")));
        assert_eq!(files.granting_root(true, Path::new("/data/work/out/c.pdf")), Some(Path::new("/data/work/out")));
        assert_eq!(files.granting_root(true, Path::new("/data/work/a.pdf")), None, "reading does not grant writing");
        assert_eq!(files.granting_root(false, Path::new("/elsewhere/a.pdf")), None);
        assert_eq!(files.granting_root(false, Path::new("/data/work/../../etc/a.pdf")), None, "no climbing");
    }
}
