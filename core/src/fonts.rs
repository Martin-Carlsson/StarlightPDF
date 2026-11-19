//! Fonts.
//!
//! # Font names
//!
//! A font name is a standard 14 font's, such as `Helvetica`, or the PostScript name, [`FontInfo::base_font`], of a
//! font in the document, such as one from [`Document::add_font`](crate::document::Document::add_font).

use alloc::sync::Arc;

use crate::output::Sink;
use crate::{ErrorCode, PdfError};

/// Options for [`Document::add_font`](crate::document::Document::add_font).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct AddFontOptions {
    /// Embed the whole font instead of the glyphs used.
    pub whole: bool,
}

/// A font the document uses.
#[derive(Clone, Debug)]
pub struct Font {
    /// Reference to the shared state.
    _shared: Arc<()>,
}

impl Font {
    /// The font's properties.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn info(&self) -> Result<FontInfo, PdfError> {
        Err(PdfError::new(ErrorCode::NotImplemented, "Font::info"))
    }

    /// Writes the embedded font program.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn write_to(&self, sink: &Sink, options: &WriteFontOptions) -> Result<(), PdfError> {
        let _: &Sink = sink;
        let _: &WriteFontOptions = options;
        Err(PdfError::new(ErrorCode::NotImplemented, "Font::write_to"))
    }
}

/// A font program format to write.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum FontFormat {
    /// Wrapped as an OpenType file.
    OpenType,
    /// The embedded program as stored.
    #[default]
    Original,
}

/// A font's properties.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct FontInfo {
    /// The PostScript name (`BaseFont`).
    pub base_font: String,
    /// The program is embedded.
    pub embedded: bool,
    /// The kind.
    pub kind: FontKind,
    /// Only the glyphs used are embedded.
    pub subset: bool,
}

/// The kind of a font (ISO 32000-2:2020, 9.5).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum FontKind {
    /// TrueType.
    TrueType,
    /// Composite.
    Type0,
    /// Type 1, including multiple master.
    Type1,
    /// Glyphs defined by content streams.
    Type3,
}

/// The fonts a document uses, one at a time.
#[derive(Clone, Debug)]
pub struct Fonts {
    /// Reference to the shared state.
    _shared: Arc<()>,
}

impl Fonts {
    /// The next font, or `None` after the last.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn next(&self) -> Result<Option<Font>, PdfError> {
        Err(PdfError::new(ErrorCode::NotImplemented, "Fonts::next"))
    }
}

/// Options for [`Font::write_to`].
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct WriteFontOptions {
    /// The format.
    pub format: FontFormat,
}
