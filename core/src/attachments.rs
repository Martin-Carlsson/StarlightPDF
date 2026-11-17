//! Embedded files and collections.

use alloc::sync::Arc;

use crate::output::Sink;
use crate::{Error, ErrorCode};

/// Options for [`Document::add_attachment`](crate::document::Document::add_attachment).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct AddAttachmentOptions {
    /// When the file was created, a [date](crate#dates).
    pub created: String,
    /// The description.
    pub description: String,
    /// The media type, such as `text/csv`.
    pub media_type: String,
    /// When the file last changed, a [date](crate#dates).
    pub modified: String,
    /// The file name.
    pub name: String,
    /// Also associate the file with the document; `None`: embed only.
    pub relationship: Option<Relationship>,
}

/// An embedded file.
#[derive(Clone, Debug)]
pub struct Attachment {
    /// Reference to the shared state.
    _shared: Arc<()>,
}

impl Attachment {
    /// Removes the file.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn delete(&self, options: &DeleteAttachmentOptions) -> Result<(), Error> {
        let _: &DeleteAttachmentOptions = options;
        Err(Error::new(ErrorCode::NotImplemented, "Attachment::delete"))
    }

    /// The file's properties.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn info(&self) -> Result<AttachmentInfo, Error> {
        Err(Error::new(ErrorCode::NotImplemented, "Attachment::info"))
    }

    /// Writes the file's contents.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn write_to(&self, sink: &Sink, options: &WriteAttachmentOptions) -> Result<(), Error> {
        let _: &Sink = sink;
        let _: &WriteAttachmentOptions = options;
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Attachment::write_to",
        ))
    }
}

/// An embedded file's properties.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct AttachmentInfo {
    /// When the file was created, a [date](crate#dates).
    pub created: String,
    /// The description.
    pub description: String,
    /// The media type.
    pub media_type: String,
    /// When the file last changed, a [date](crate#dates).
    pub modified: String,
    /// The file name.
    pub name: String,
    /// How the file relates to the document, when associated.
    pub relationship: Option<Relationship>,
    /// The size in bytes.
    pub size: u64,
}

/// A document's embedded files.
#[derive(Clone, Debug)]
pub struct Attachments {
    /// Reference to the shared state.
    _shared: Arc<()>,
}

impl Attachments {
    /// The next file, or `None` after the last.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn next(&self) -> Result<Option<Attachment>, Error> {
        Err(Error::new(ErrorCode::NotImplemented, "Attachments::next"))
    }
}

/// A collection's settings.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Collection {
    /// The file shown first.
    pub initial: String,
    /// `None`: not a collection.
    pub view: Option<CollectionView>,
}

/// How a collection lists its files.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CollectionView {
    /// A detailed list.
    Details,
    /// Only the file shown first.
    Hidden,
    /// Small icons.
    Icons,
    /// Large tiles.
    Tiles,
}

/// Options for [`Attachment::delete`].
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct DeleteAttachmentOptions;

/// How an associated file relates to the document (ISO 32000-2:2020, 14.13.2).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Relationship {
    /// An alternative representation.
    Alternative,
    /// Data the content was derived from or shows.
    Data,
    /// The original the content was made from (`/Source`).
    Original,
    /// A supplement.
    Supplement,
    /// Not specified.
    Unspecified,
}

/// Options for [`Document::set_collection`](crate::document::Document::set_collection).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct SetCollectionOptions {
    /// The file shown first.
    pub initial: String,
    /// `None`: not a collection.
    pub view: Option<CollectionView>,
}

/// Options for [`Attachment::write_to`].
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct WriteAttachmentOptions;
