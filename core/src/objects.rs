//! The object layer: raw objects and streams as PDF syntax.
//!
//! Objects are exchanged as PDF syntax text (ISO 32000-2:2020, 7.3), such as `<< /Type /Page /Rotate 90 >>`;
//! non-ASCII string bytes are written as hex strings.

use alloc::sync::Arc;

use crate::{Error, ErrorCode};

/// Options for [`Document::add_object`](crate::document::Document::add_object).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct AddObjectOptions {
    /// The object as PDF syntax.
    pub syntax: String,
}

/// An indirect object's number and generation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub struct ObjectId {
    /// The generation number.
    pub generation: u16,
    /// The object number.
    pub number: u32,
}

/// Options for [`Document::object`](crate::document::Document::object).
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct ObjectOptions {
    /// The object.
    pub id: ObjectId,
}

/// Options for [`Document::set_object`](crate::document::Document::set_object).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct SetObjectOptions {
    /// The object.
    pub id: ObjectId,
    /// The new object as PDF syntax; for a stream, its dictionary.
    pub syntax: String,
}

/// Options for [`Document::set_stream_data`](crate::document::Document::set_stream_data).
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct SetStreamDataOptions {
    /// Compress with `FlateDecode`; otherwise store as given, matching the dictionary's `Filter`.
    pub compress: bool,
    /// The stream.
    pub id: ObjectId,
}

/// Options for [`Document::stream_data`](crate::document::Document::stream_data).
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct StreamDataOptions {
    /// Undo the stream's filters; otherwise write the stored bytes.
    pub decode: bool,
    /// The stream.
    pub id: ObjectId,
}

/// Cross-reference entries.
#[derive(Clone, Debug)]
pub struct XrefEntries {
    /// Reference to the shared state.
    _shared: Arc<()>,
}

impl XrefEntries {
    /// The next entry, or `None` after the last.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn next(&self) -> Result<Option<XrefEntry>, Error> {
        Err(Error::new(ErrorCode::NotImplemented, "XrefEntries::next"))
    }
}

/// A cross-reference entry (ISO 32000-2:2020, 7.5.4 and 7.5.8).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct XrefEntry {
    /// The generation number.
    pub generation: u16,
    /// Index in its object stream, for [`XrefKind::Compressed`].
    pub index: u32,
    /// The kind of entry.
    pub kind: XrefKind,
    /// The object number.
    pub number: u32,
    /// Byte offset, for [`XrefKind::InUse`].
    pub offset: u64,
    /// The containing object stream's number, for [`XrefKind::Compressed`].
    pub stream: u32,
}

/// The kind of a cross-reference entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum XrefKind {
    /// In an object stream.
    Compressed,
    /// Free.
    Free,
    /// In use, at a byte offset.
    InUse,
}
