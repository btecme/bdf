//! pdfcraft-app-service — the application service behind every PdfCraft surface (BUILD-007).
//!
//! The desktop, the command line and the MCP server will each be an adapter over this crate: they translate their own
//! shapes (widgets, arguments, JSON-RPC) into the typed requests of [`operations`] and the results and errors of
//! [`results`] and [`errors`] back out. The engine session lives inside [`Service`] and is not reachable from outside it,
//! so every change to a document passes the same checks, in the same order, whoever asked.
//!
//! What this first slice contains: open, query, rotate, undo, redo, history, save and close, and who may do each: actors
//! that a caller cannot write, grants, a policy, revocation checked before an effect and at the commit of a write, and an
//! audit log. What it does not contain yet (and no adapter uses it yet): jobs and cancellation, the application part of
//! grants (where they live, issuing them, handing a secret over), the rest of the tool catalogue. See `README.md` for the
//! contract and for what is deliberately left to later work.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod actors;
pub mod audit;
pub mod effects;
pub mod errors;
mod file_lock;
pub mod grants;
pub mod operations;
pub mod policy;
pub mod results;
mod service;
#[cfg(test)]
mod testutil;
pub mod version;

pub use actors::{ActorContext, ClientId, GrantId, OsPrincipal, PrincipalKind};
pub use audit::{AuditEffect, AuditEntry, AuditOutcome, AuditRecord, AuditSink, FileAudit, MemoryAudit};
pub use effects::Effect;
pub use errors::{ConflictKind, ErrorCode, OperationError, Result};
pub use grants::{AuthError, DocumentScope, FileAccess, GrantError, GrantRequest, GrantStore, GrantTerms, IssuedGrant, Scope};
pub use operations::{
    Access, Capability, DEFAULT_BOOKMARK_PAGE, DEFAULT_FIND_MATCHES, DESCRIPTORS, MAX_BOOKMARK_PAGE, MAX_FIND_MATCHES, MAX_FIND_PAGES,
    MAX_QUERY_CHARS, MAX_TEXT_BYTES, MAX_TEXT_PAGE_LIST, OLDEST_SCHEMA, OpenSource, Operation, OperationDescriptor, OperationId, OperationRequest,
    Overwrite, PageSelection, Release, SCHEMA_VERSION, SaveTarget,
};
pub use policy::{Denial, DenyReason, Facts, FileUse, Policy, ReleaseProfile, Watch};
pub use results::{
    BookmarkEntry, BookmarkList, DocumentInfo, FindNext, FindReport, FoundMatch, History, LinkList, LinkSummary, LinkTarget, MAX_FIND_WARNINGS,
    MAX_LINKS, MAX_WARNING_BYTES, OperationResult, Outcome, PageSummary, PageTextEntry, PageWarning, SaveReport, TextPages,
};
pub use service::{DocumentView, MAX_OPEN_BYTES, MAX_SAVES_IN_FLIGHT, PreparedSave, RequestContext, SchemaRange, ScriptTrust, Service, WrittenSave};
pub use version::{DocumentKey, DocumentVersion, SavedSnapshot, SessionId};
