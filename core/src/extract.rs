//! Content extraction.

use alloc::sync::Arc;

use crate::layers::Layer;
use crate::{Error, ErrorCode};

/// A word, line, paragraph or column.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Block {
    /// Heading level, 0 for none.
    pub heading: u8,
    /// Height in points.
    pub height: f64,
    /// The text.
    pub text: String,
    /// Width in points.
    pub width: f64,
    /// Left edge in points.
    pub x: f64,
    /// Top edge in points.
    pub y: f64,
}

/// The unit of a block.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum BlockUnit {
    /// Columns.
    Column,
    /// Lines.
    Line,
    /// Paragraphs.
    #[default]
    Paragraph,
    /// Words.
    Word,
}

/// Layout blocks.
#[derive(Clone, Debug)]
pub struct Blocks {
    /// Reference to the shared state.
    _shared: Arc<()>,
}

impl Blocks {
    /// The next block, or `None` after the last.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn next(&self) -> Result<Option<Block>, Error> {
        Err(Error::new(ErrorCode::NotImplemented, "Blocks::next"))
    }
}

/// Options for [`Page::blocks`](crate::page::Page::blocks).
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct BlocksOptions {
    /// The unit of each block.
    pub unit: BlockUnit,
}

/// Options for [`Document::convert_to`](crate::document::Document::convert_to).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct ConvertToOptions {
    /// The format to write.
    pub format: OutputFormat,
    /// Empty: all pages.
    pub pages: Vec<u32>,
}

/// A match, one per line it spans.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Hit {
    /// Height in points.
    pub height: f64,
    /// The page index.
    pub page: u32,
    /// The matched text.
    pub text: String,
    /// Width in points.
    pub width: f64,
    /// Left edge in points.
    pub x: f64,
    /// Top edge in points.
    pub y: f64,
}

/// Search hits.
#[derive(Clone, Debug)]
pub struct Hits {
    /// Reference to the shared state.
    _shared: Arc<()>,
}

impl Hits {
    /// The next hit, or `None` after the last.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn next(&self) -> Result<Option<Hit>, Error> {
        Err(Error::new(ErrorCode::NotImplemented, "Hits::next"))
    }
}

/// A format to convert a document to.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum OutputFormat {
    /// Word (DOCX).
    Docx,
    /// HTML.
    Html,
    /// JSON.
    Json,
    /// Markdown.
    Markdown,
    /// Plain text.
    #[default]
    Text,
    /// Excel (XLSX).
    Xlsx,
    /// XML.
    Xml,
}

/// Options for [`Document::recognize`](crate::document::Document::recognize).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct RecognizeOptions {
    /// BCP 47 tags; empty: `en`.
    pub languages: Vec<String>,
    /// Empty: all pages.
    pub pages: Vec<u32>,
    /// Pixels per inch to recognize at; `None`: 300.
    pub resolution: Option<u32>,
}

/// Options for [`Document::search`](crate::document::Document::search).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct SearchOptions {
    /// Match case.
    pub case_sensitive: bool,
    /// Empty: all pages.
    pub pages: Vec<u32>,
    /// The text to find.
    pub query: String,
    /// `query` is a regular expression.
    pub regex: bool,
    /// Match whole words only.
    pub whole_words: bool,
}

/// A table.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Table {
    /// Row after row.
    pub cells: Vec<String>,
    /// The number of columns.
    pub columns: u32,
    /// Height in points.
    pub height: f64,
    /// The number of rows.
    pub rows: u32,
    /// Width in points.
    pub width: f64,
    /// Left edge in points.
    pub x: f64,
    /// Top edge in points.
    pub y: f64,
}

/// Tables.
#[derive(Clone, Debug)]
pub struct Tables {
    /// Reference to the shared state.
    _shared: Arc<()>,
}

impl Tables {
    /// The next table, or `None` after the last.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn next(&self) -> Result<Option<Table>, Error> {
        Err(Error::new(ErrorCode::NotImplemented, "Tables::next"))
    }
}

/// Options for [`Page::tables`](crate::page::Page::tables).
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct TablesOptions;

/// Options for [`Page::text`](crate::page::Page::text).
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct TextOptions {
    /// Reconstruct reading order instead of content order.
    pub reading_order: bool,
}

/// Text in one font and color.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct TextSpan {
    /// A [color](crate::drawing#colors).
    pub color: String,
    /// The [font name](crate::fonts#font-names).
    pub font: String,
    /// Font size in points.
    pub font_size: f64,
    /// Height in points.
    pub height: f64,
    /// The layer it is drawn on, or `None`.
    pub layer: Option<Layer>,
    /// The opacity.
    pub opacity: f64,
    /// The text.
    pub text: String,
    /// Width in points.
    pub width: f64,
    /// Left edge in points.
    pub x: f64,
    /// Top edge in points.
    pub y: f64,
}

/// Text spans.
#[derive(Clone, Debug)]
pub struct TextSpans {
    /// Reference to the shared state.
    _shared: Arc<()>,
}

impl TextSpans {
    /// The next span, or `None` after the last.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn next(&self) -> Result<Option<TextSpan>, Error> {
        Err(Error::new(ErrorCode::NotImplemented, "TextSpans::next"))
    }
}
