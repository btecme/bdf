//! The operations the service offers, as typed requests and as descriptors.
//!
//! A descriptor is the one place that says what an operation is: its stable id, what it needs, whether it reads,
//! changes or persists, and whether this release has it. Tool definitions and the availability of UI commands are
//! meant to be derived from it, so that what is offered and what is refused cannot drift apart. The service never
//! depends on an adapter's tool type; adapters depend on this.

use std::path::PathBuf;
use std::sync::Arc;

use crate::version::DocumentKey;

/// The request/response schema this build speaks, and the oldest it still accepts.
pub const SCHEMA_VERSION: u32 = 1;
pub const OLDEST_SCHEMA: u32 = 1;

/// A stable operation id. The spelling ([`OperationId::name`]) is the contract; the variants are not ordered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OperationId {
    DocumentOpen,
    DocumentQuery,
    PageRotate,
    EditUndo,
    EditRedo,
    EditHistory,
    DocumentSave,
    DocumentClose,
}

impl OperationId {
    pub const ALL: [OperationId; 8] = [
        OperationId::DocumentOpen,
        OperationId::DocumentQuery,
        OperationId::PageRotate,
        OperationId::EditUndo,
        OperationId::EditRedo,
        OperationId::EditHistory,
        OperationId::DocumentSave,
        OperationId::DocumentClose,
    ];

    pub fn name(self) -> &'static str {
        self.descriptor().name
    }

    pub fn parse(name: &str) -> Option<OperationId> {
        OperationId::ALL.into_iter().find(|id| id.name() == name)
    }

    pub fn descriptor(self) -> &'static OperationDescriptor {
        // The table is indexed by declaration order; the test `every_operation_has_one_descriptor_in_order` keeps
        // it honest, and the lookup falls back to a search rather than indexing.
        DESCRIPTORS.iter().find(|d| d.id == self).unwrap_or(&DESCRIPTORS[0])
    }
}

/// What a caller must hold to use an operation. The first version of the list; the policy layer decides who holds what.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Capability {
    DocumentRead,
    DocumentEditPages,
    DocumentSave,
    /// Closing a document drops its in-memory state, unsaved changes included, so it is not part of reading.
    DocumentClose,
    /// Undo and redo need the capability of the edit they revert, which depends on the step: the policy resolves it
    /// from the document's history (the capability recorded when each change was made) and the actor must hold that one.
    /// It is never a grant a client can hold: undo is not a way to get an effect the client was not granted.
    RevertedEdit,
}

impl Capability {
    /// Every capability a grant can hold. `RevertedEdit` is not among them: it is resolved per history entry.
    pub const GRANTABLE: [Capability; 4] =
        [Capability::DocumentRead, Capability::DocumentEditPages, Capability::DocumentSave, Capability::DocumentClose];

    pub fn name(self) -> &'static str {
        match self {
            Capability::DocumentRead => "document.read",
            Capability::DocumentEditPages => "document.edit.pages",
            Capability::DocumentSave => "document.save",
            Capability::DocumentClose => "document.close",
            Capability::RevertedEdit => "document.edit.reverted",
        }
    }

    /// The grantable capability named `name`.
    pub fn parse(name: &str) -> Option<Capability> {
        Capability::GRANTABLE.into_iter().find(|c| c.name() == name)
    }
}

/// What an operation does to the world.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// Reads documents; changes nothing.
    Read,
    /// Changes the document in memory, undoably, and names the revision it was made against.
    Mutate,
    /// Writes outside memory (a file).
    Persist,
    /// Opens or closes a document in the session.
    Lifecycle,
}

/// Whether this release has the operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Release {
    /// In the v1 desktop core.
    Core,
    /// Not in v1; planned behind its own qualification. A core-only release refuses it even when the name is known.
    Deferred,
    /// Not in v1 and not planned, or unsafe by default.
    Excluded,
}

#[derive(Clone, Debug)]
pub struct OperationDescriptor {
    pub id: OperationId,
    pub name: &'static str,
    pub summary: &'static str,
    pub capability: Capability,
    pub access: Access,
    pub release: Release,
    /// The request names a document.
    pub needs_document: bool,
    /// The request names the content revision it was made against (every operation that changes or discards content).
    pub needs_revision: bool,
}

pub static DESCRIPTORS: [OperationDescriptor; 8] = [
    OperationDescriptor {
        id: OperationId::DocumentOpen,
        name: "document.open",
        summary: "Open a PDF from a file or from bytes.",
        capability: Capability::DocumentRead,
        access: Access::Lifecycle,
        release: Release::Core,
        needs_document: false,
        needs_revision: false,
    },
    OperationDescriptor {
        id: OperationId::DocumentQuery,
        name: "document.query",
        summary: "Describe an open document: its pages, whether it can be edited and what undo would do.",
        capability: Capability::DocumentRead,
        access: Access::Read,
        release: Release::Core,
        needs_document: true,
        needs_revision: false,
    },
    OperationDescriptor {
        id: OperationId::PageRotate,
        name: "page.rotate",
        summary: "Rotate pages by a multiple of 90 degrees (positive is clockwise). Undoable.",
        capability: Capability::DocumentEditPages,
        access: Access::Mutate,
        release: Release::Core,
        needs_document: true,
        needs_revision: true,
    },
    OperationDescriptor {
        id: OperationId::EditUndo,
        name: "edit.undo",
        summary: "Undo the last change.",
        capability: Capability::RevertedEdit,
        access: Access::Mutate,
        release: Release::Core,
        needs_document: true,
        needs_revision: true,
    },
    OperationDescriptor {
        id: OperationId::EditRedo,
        name: "edit.redo",
        summary: "Redo the last undone change.",
        capability: Capability::RevertedEdit,
        access: Access::Mutate,
        release: Release::Core,
        needs_document: true,
        needs_revision: true,
    },
    OperationDescriptor {
        id: OperationId::EditHistory,
        name: "edit.history",
        summary: "What undo and redo would do next, and whether anything is unsaved.",
        capability: Capability::DocumentRead,
        access: Access::Read,
        release: Release::Core,
        needs_document: true,
        needs_revision: false,
    },
    OperationDescriptor {
        id: OperationId::DocumentSave,
        name: "document.save",
        summary: "Write the document to its file or to another path, replacing only a version it is entitled to.",
        capability: Capability::DocumentSave,
        access: Access::Persist,
        release: Release::Core,
        needs_document: true,
        needs_revision: true,
    },
    OperationDescriptor {
        id: OperationId::DocumentClose,
        name: "document.close",
        summary: "Close a document, refusing to drop unsaved changes unless told to.",
        capability: Capability::DocumentClose,
        access: Access::Lifecycle,
        release: Release::Core,
        needs_document: true,
        needs_revision: true,
    },
];

/// One request. `schema_version` is checked before anything else.
#[derive(Clone, Debug)]
pub struct OperationRequest {
    pub schema_version: u32,
    /// The document the operation is about; `None` for operations that name none.
    pub document: Option<DocumentKey>,
    pub operation: Operation,
}

impl OperationRequest {
    pub fn new(document: Option<DocumentKey>, operation: Operation) -> Self {
        Self { schema_version: SCHEMA_VERSION, document, operation }
    }
}

/// The typed arguments of each operation. An operation that changes or discards content carries the revision it was
/// made against as a field, so it cannot be built without one.
#[derive(Clone, Debug)]
pub enum Operation {
    Open { source: OpenSource, password: Option<String> },
    Query,
    Rotate { expected_revision: u64, pages: PageSelection, degrees: i64 },
    Undo { expected_revision: u64 },
    Redo { expected_revision: u64 },
    History,
    Save { expected_revision: u64, target: SaveTarget, overwrite: Overwrite, full: Option<bool> },
    Close { expected_revision: u64, discard_changes: bool },
}

impl Operation {
    pub fn id(&self) -> OperationId {
        match self {
            Operation::Open { .. } => OperationId::DocumentOpen,
            Operation::Query => OperationId::DocumentQuery,
            Operation::Rotate { .. } => OperationId::PageRotate,
            Operation::Undo { .. } => OperationId::EditUndo,
            Operation::Redo { .. } => OperationId::EditRedo,
            Operation::History => OperationId::EditHistory,
            Operation::Save { .. } => OperationId::DocumentSave,
            Operation::Close { .. } => OperationId::DocumentClose,
        }
    }

    /// The revision a changing operation was made against.
    pub fn expected_revision(&self) -> Option<u64> {
        match self {
            Operation::Rotate { expected_revision, .. }
            | Operation::Undo { expected_revision }
            | Operation::Redo { expected_revision }
            | Operation::Save { expected_revision, .. }
            | Operation::Close { expected_revision, .. } => Some(*expected_revision),
            Operation::Open { .. } | Operation::Query | Operation::History => None,
        }
    }
}

#[derive(Clone, Debug)]
pub enum OpenSource {
    File(PathBuf),
    Bytes { name: String, bytes: Arc<Vec<u8>> },
}

/// Pages as an external caller numbers them: from 1. They are checked and converted to the engine's 0-based indices
/// once, inside the service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PageSelection {
    All,
    Numbers(Vec<i64>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveTarget {
    /// The file the document was opened from or last saved to.
    Tracked,
    Path(PathBuf),
}

/// What a save may replace beyond the version it last read or wrote. There is no "overwrite whatever is there".
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Overwrite {
    /// Replace only the tracked version (or create the file if the target is a new path).
    Tracked,
    /// Replace the file whose content has this fingerprint (64 hex digits), which the caller has inspected.
    Version(String),
    /// Create the file anew: it must not exist now.
    Absent,
}
