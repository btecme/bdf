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
    /// The documents the caller may read.
    Documents(Vec<DocumentInfo>),
    Links(LinkList),
    Bookmarks(BookmarkList),
    Text(TextPages),
    Found(FindReport),
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
    /// A save planned after this one had already been applied, so this one only wrote its file: the document's
    /// destination, saved content and tracked file version are those of the later save.
    pub superseded: bool,
    /// The document was closed (or replaced) before the save finished; the file is written and nothing else changed.
    pub document_open: bool,
}

/// The links of a document. At most [`MAX_LINKS`] are listed; `total` says how many there are.
#[derive(Clone, Debug, PartialEq)]
pub struct LinkList {
    pub links: Vec<LinkSummary>,
    pub total: usize,
}

/// The most links one `document.links` request lists. Past it the list says how many were left out.
pub const MAX_LINKS: usize = 10_000;

#[derive(Clone, Debug, PartialEq)]
pub struct LinkSummary {
    /// 1-based.
    pub page: usize,
    /// 1-based position among the page's annotations, as the link tools take it.
    pub index: usize,
    /// Points from the top-left of the displayed page: left, top, right, bottom.
    pub rect: [f32; 4],
    pub visible: bool,
    pub target: LinkTarget,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkTarget {
    /// A page of this document, 1-based.
    Page(usize),
    Uri(String),
    /// Something else (a named destination, a script, a file): described, not followed.
    Other(String),
}

/// One page of bookmarks, in outline order, flat: the nesting is in each row's `path`.
#[derive(Clone, Debug, PartialEq)]
pub struct BookmarkList {
    pub rows: Vec<BookmarkEntry>,
    /// Where the next page starts, or `None` at the end.
    pub next: Option<usize>,
    /// The structure could not be read for editing, so the listing is the viewer's shorter copy.
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BookmarkEntry {
    /// 1-based child positions from the top level.
    pub path: Vec<usize>,
    pub title: String,
    /// 1-based page it goes to.
    pub page: Option<usize>,
    pub open: bool,
}

/// The text of pages, in the order they were asked for. When the answer would pass `MAX_TEXT_BYTES` the pages that
/// did not fit are left out, from the one that crossed it on, and `omitted_pages` counts them; the first page is
/// always there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextPages {
    pub pages: Vec<PageTextEntry>,
    pub omitted_pages: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageTextEntry {
    /// 1-based.
    pub page: usize,
    pub text: String,
}

/// What a search found. It stopped early when `next` is set: `next` is where to ask again.
#[derive(Clone, Debug, PartialEq)]
pub struct FindReport {
    pub query: String,
    pub matches: Vec<FoundMatch>,
    pub next: Option<FindNext>,
    /// Pages that could not be read (at most [`MAX_FIND_WARNINGS`]); `warning_count` counts them all. The matches on
    /// the other pages are kept.
    pub warnings: Vec<PageWarning>,
    pub warning_count: usize,
}

/// The most failed pages a search lists, and the longest message it keeps for one, in bytes.
pub const MAX_FIND_WARNINGS: usize = 100;
pub const MAX_WARNING_BYTES: usize = 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct FoundMatch {
    /// 1-based.
    pub page: usize,
    pub text: String,
    /// One rectangle per line the match covers: left, top, right, bottom in points from the top-left of the page.
    pub rects: Vec<[f32; 4]>,
}

/// Where a search that stopped goes on: `start_page` (1-based) and the matches to skip on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FindNext {
    pub start_page: usize,
    pub skip: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageWarning {
    /// 1-based.
    pub page: usize,
    pub error: String,
    pub error_truncated: bool,
}
