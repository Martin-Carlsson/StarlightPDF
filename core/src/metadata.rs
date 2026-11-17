//! The document information dictionary and XMP metadata.

/// The document information dictionary (ISO 32000-2:2020, 14.3.3).
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct InfoDictionary {
    /// The author.
    pub author: String,
    /// The creation [date](crate#dates).
    pub created: String,
    /// The application that created the original.
    pub creator: String,
    /// Keywords.
    pub keywords: String,
    /// The last modification [date](crate#dates).
    pub modified: String,
    /// The application that wrote the PDF.
    pub producer: String,
    /// The subject.
    pub subject: String,
    /// The title.
    pub title: String,
}

/// Options for [`Document::set_info_dictionary`](crate::document::Document::set_info_dictionary); an empty field
/// removes the entry.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct SetInfoDictionaryOptions {
    /// The author.
    pub author: String,
    /// The creation [date](crate#dates).
    pub created: String,
    /// The application that created the original.
    pub creator: String,
    /// Keywords.
    pub keywords: String,
    /// The last modification [date](crate#dates).
    pub modified: String,
    /// The application that wrote the PDF.
    pub producer: String,
    /// The subject.
    pub subject: String,
    /// The title.
    pub title: String,
}

/// Options for [`Document::set_xmp`](crate::document::Document::set_xmp).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct SetXmpOptions {
    /// The XMP packet; empty removes it.
    pub xmp: String,
}
