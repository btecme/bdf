//! pdfcraft-app-service — the application service behind every PdfCraft surface (BUILD-007).
//!
//! The desktop, the command line and the MCP server will each be an adapter over this crate: they translate their own
//! shapes (widgets, arguments, JSON-RPC) into the typed requests of [`operations`] and the results and errors of
//! [`results`] and [`errors`] back out. The engine session lives inside [`Service`] and is not reachable from outside it,
//! so every change to a document passes the same checks, in the same order, whoever asked.
//!
//! What this first slice contains: open, query, rotate, undo, redo, history, save and close. What it does not contain
//! yet (and no adapter uses it yet): identities and grants, jobs and cancellation, audit, the rest of the tool
//! catalogue. See `README.md` for the contract and for what is deliberately left to later work.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod effects;
pub mod errors;
pub mod operations;
pub mod results;
mod service;
pub mod version;

pub use effects::Effect;
pub use errors::{ConflictKind, ErrorCode, OperationError, Result};
pub use operations::{
    Access, Capability, DESCRIPTORS, OLDEST_SCHEMA, OpenSource, Operation, OperationDescriptor, OperationId, OperationRequest, Overwrite,
    PageSelection, Release, SCHEMA_VERSION, SaveTarget,
};
pub use results::{DocumentInfo, History, OperationResult, Outcome, PageSummary, SaveReport};
pub use service::{DocumentView, MAX_OPEN_BYTES, MAX_SAVES_IN_FLIGHT, PreparedSave, RequestContext, SchemaRange, Service, WrittenSave};
pub use version::{DocumentKey, DocumentVersion, SavedSnapshot, SessionId};
