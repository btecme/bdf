//! What an operation did outside the document's memory.

/// A change that is already made and is not undone by a later failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Bytes were written to a file. `file` is its name (the service does not hand out full paths) and `fingerprint`
    /// the SHA-256 of what was written.
    FileWritten { file: String, bytes: u64, fingerprint: String },
    /// The document was closed.
    DocumentClosed,
}
