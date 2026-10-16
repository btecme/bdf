//! The default of each enumerated parameter that a core tool accepts, in one place.
//!
//! An omitted (or null) parameter selects its default, so the default is an *effective mode* and the
//! desktop-core profile has to judge it like an explicit value (`capability::check_tool_call`). The
//! tools read these constants and the capability inventory names them, so the mode a call runs in and
//! the mode the profile judged cannot drift apart. Changing a value here changes behaviour and the
//! profile's verdict together.

/// `doc_print` `subset`: print all pages, not only the odd or even ones.
pub(crate) const PRINT_SUBSET: &str = "all";
/// `doc_print` `layout`: scale pages to fit the paper.
pub(crate) const PRINT_LAYOUT: &str = "fit";
/// `doc_print` `orientation`: choose per page.
pub(crate) const PRINT_ORIENTATION: &str = "auto";
/// `doc_print` `comments_forms`: print the document with its comments and markups, as it is seen.
pub(crate) const PRINT_COMMENTS_FORMS: &str = "document-and-markups";
/// `doc_print` `duplex`: single-sided.
pub(crate) const PRINT_DUPLEX: &str = "off";
/// `page_rotate` `subset`: every page of the selection, not only odd or even ones.
pub(crate) const ROTATE_SUBSET: &str = "all";
/// `page_rotate` `orientation`: pages of any orientation.
pub(crate) const ROTATE_ORIENTATION: &str = "all";
/// `doc_combine` `field_conflicts`: rename a form field whose name is already used.
pub(crate) const COMBINE_FIELD_CONFLICTS: &str = "rename";
