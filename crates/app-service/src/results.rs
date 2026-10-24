//! What an operation returns.

use crate::effects::Effect;
use crate::version::{DocumentKey, DocumentVersion};

#[derive(Clone, Debug, PartialEq)]
pub struct OperationResult {
    pub outcome: Outcome,
    /// The document's version after the operation; `None` when no document remains (close) or none was involved.
    pub version: Option<DocumentVersion>,
    /// What was repaired or approximated, as the engine reported it.
    pub warnings: Vec<String>,
    /// What the operation did outside memory. Empty for everything that only reads or edits.
    pub effects: Vec<Effect>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Opened {
        document: DocumentKey,
        info: DocumentInfo,
    },
    Info(DocumentInfo),
    /// An edit, undo or redo was applied. `label` is the history wording ("Rotate pages").
    Changed {
        label: String,
        pages_affected: Option<usize>,
    },
    History(History),
    Saved(SaveReport),
    Closed,
}

/// A document as a caller may see it. Pages are numbered from 1 and sized in points as displayed (after rotation).
#[derive(Clone, Debug, PartialEq)]
pub struct DocumentInfo {
    pub document: DocumentKey,
    /// The file name, not the path.
    pub name: String,
    pub pages: Vec<PageSummary>,
    pub editable: bool,
    /// Why it cannot be edited, when it cannot.
    pub read_only_reason: Option<String>,
    pub signed: bool,
    pub encrypted: bool,
    pub has_file: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PageSummary {
    /// 1-based.
    pub number: usize,
    pub width: f32,
    pub height: f32,
    /// Clockwise degrees: 0, 90, 180 or 270.
    pub rotation: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct History {
    pub next_undo: Option<String>,
    pub next_redo: Option<String>,
    pub unsaved: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveReport {
    pub file: String,
    pub bytes: u64,
    pub incremental: bool,
    /// The fingerprint of what is on disk now.
    pub fingerprint: String,
    /// The document was edited between taking the bytes and finishing the save: those edits are kept and the
    /// document is still unsaved.
    pub edited_since: bool,
    /// The document was closed (or replaced) before the save finished; the file is written and nothing else changed.
    pub document_open: bool,
}
