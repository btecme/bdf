//! Which state of a document a request, a result or a worker saw.

/// A session. Two services never share one, and a reference from another is refused, so a document number from an
/// earlier run or another window cannot reach the wrong document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SessionId(pub u64);

/// A document in a session: `(session, document)`. The document number alone is not an identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DocumentKey {
    pub session: SessionId,
    pub document: u64,
}

/// The three things "the version of a document" has to keep apart.
///
/// * `content_revision` moves when what the document contains changes (an edit, undo, redo, revert, reload) and at no
///   other time. A request that changes a document names the revision it was made against.
/// * `render_generation` moves for every change a cache has to notice, including saves and display-only changes. It
///   says nothing about content and no request is checked against it.
/// * `saved` says whether the content is the content last written to a file, by identity of the content and not by
///   comparing revision numbers: undoing back to the saved content is at the saved content again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DocumentVersion {
    pub content_revision: u64,
    pub render_generation: u64,
    pub saved: SavedSnapshot,
}

/// How the working content relates to what is in a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SavedSnapshot {
    /// The identity of the working content within the session.
    pub content: u64,
    /// The identity of the content last opened from or written to a file, if there is one.
    pub saved_content: Option<u64>,
}

impl SavedSnapshot {
    /// There is nothing unsaved.
    pub fn at_saved_content(&self) -> bool {
        self.saved_content == Some(self.content)
    }
}

impl DocumentVersion {
    /// `true` when closing would lose changes.
    pub fn dirty(&self) -> bool {
        !self.saved.at_saved_content()
    }
}
