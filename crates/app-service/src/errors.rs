//! The errors every surface sees. A code is part of the contract: adapters map it, they do not parse messages.

use crate::effects::Effect;

/// Why an operation did not complete. The set is closed so that adapters can handle every case; a new code is a
/// contract change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    /// The request does not say something the service can do: a page below 1, degrees that are not a multiple of 90.
    InvalidArgument,
    /// The caller may not do this. Raised by the policy layer; nothing in this crate raises it yet.
    NotAuthorized,
    /// There is no such document (never opened, already closed, or opened in another session).
    NotFound,
    /// The operation exists but not for this document or in this build.
    UnsupportedFeature,
    PasswordRequired,
    WrongPassword,
    /// The world is not as the request assumed: the document changed since the revision the request names, or the
    /// file on disk is not the version the save may replace. Nothing was changed; read the current state and decide.
    Conflict,
    /// A size or count limit.
    ResourceLimit,
    Canceled,
    /// The filesystem refused or failed.
    IoFailure,
    WorkerFailure,
    /// A defect in this program. The document and its history are as they were.
    InternalFailure,
    /// The request speaks a schema version this service does not (any more). It is not in the first list of codes: a
    /// client has to tell "upgrade me" from "your request is wrong", and the answer carries the supported range.
    UnsupportedSchema,
}

impl ErrorCode {
    /// The stable spelling used on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::InvalidArgument => "invalid_argument",
            ErrorCode::NotAuthorized => "not_authorized",
            ErrorCode::NotFound => "not_found",
            ErrorCode::UnsupportedFeature => "unsupported_feature",
            ErrorCode::PasswordRequired => "password_required",
            ErrorCode::WrongPassword => "wrong_password",
            ErrorCode::Conflict => "conflict",
            ErrorCode::ResourceLimit => "resource_limit",
            ErrorCode::Canceled => "canceled",
            ErrorCode::IoFailure => "io_failure",
            ErrorCode::WorkerFailure => "worker_failure",
            ErrorCode::InternalFailure => "internal_failure",
            ErrorCode::UnsupportedSchema => "unsupported_schema",
        }
    }
}

/// What a conflict was about, so a caller can resolve it rather than guess.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConflictKind {
    /// The request was made against `expected`, and the document is at `current`.
    StaleRevision { expected: u64, current: u64 },
    /// The save would create a file and one is there (`disk` is its fingerprint).
    FileExists { disk: String },
    /// The file is no longer the version the session read or wrote.
    FileChanged { disk: String },
    /// The file was deleted.
    FileGone,
    /// Closing would drop changes that nobody asked to discard.
    UnsavedChanges,
}

/// A failed operation.
///
/// `committed` is what happened before the failure and stays done. It is empty for every failure that stops before
/// any effect: the document, its history and the files are as they were. It is not empty when a file was written and
/// the step after it failed; the message then must not suggest that repeating the request starts from nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationError {
    pub code: ErrorCode,
    pub message: String,
    /// Whether the same request may succeed if repeated unchanged.
    pub retryable: bool,
    pub conflict: Option<ConflictKind>,
    pub committed: Vec<Effect>,
    /// For a refusal by the policy: the stable reason (`grant_revoked`, `capability_not_granted`, ...). The audit log records it.
    pub reason: Option<&'static str>,
}

impl OperationError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), retryable: false, conflict: None, committed: Vec::new(), reason: None }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidArgument, message)
    }

    pub(crate) fn not_found() -> Self {
        Self::new(ErrorCode::NotFound, "no such document")
    }

    pub(crate) fn conflict(kind: ConflictKind, message: impl Into<String>) -> Self {
        Self { conflict: Some(kind), ..Self::new(ErrorCode::Conflict, message) }
    }

    pub(crate) fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InternalFailure, message)
    }
}

impl std::fmt::Display for OperationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for OperationError {}

pub type Result<T> = std::result::Result<T, OperationError>;
