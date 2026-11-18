//! Standards: validation, tagging and output intents.

use alloc::sync::Arc;

use crate::objects::ObjectId;
use crate::{ErrorCode, PdfError};

/// A deviation from a standard.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Deviation {
    /// Standard and clause, e.g. `ISO 32000-2:2020, 7.5.4`.
    pub clause: String,
    /// What deviates.
    pub message: String,
    /// The object number, when known.
    pub object_number: Option<u32>,
    /// The byte offset in the source, when known.
    pub offset: Option<u64>,
    /// The page index, when known.
    pub page: Option<u32>,
}

/// Deviations.
#[derive(Clone, Debug)]
pub struct Deviations {
    /// Reference to the shared state.
    _shared: Arc<()>,
}

impl Deviations {
    /// The next deviation, or `None` after the last.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn next(&self) -> Result<Option<Deviation>, PdfError> {
        Err(PdfError::new(ErrorCode::NotImplemented, "Deviations::next"))
    }
}

/// A structure element.
#[derive(Clone, Debug)]
pub struct Element {
    /// Reference to the shared state.
    _shared: Arc<()>,
}

impl Element {
    /// The element's properties.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn info(&self) -> Result<ElementInfo, PdfError> {
        Err(PdfError::new(ErrorCode::NotImplemented, "Element::info"))
    }

    /// Sets the alternate description.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn set_alt_text(&self, options: &SetAltTextOptions) -> Result<(), PdfError> {
        let _: &SetAltTextOptions = options;
        Err(PdfError::new(
            ErrorCode::NotImplemented,
            "Element::set_alt_text",
        ))
    }
}

/// A structure element's properties.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ElementInfo {
    /// The alternate description.
    pub alt_text: String,
    /// 0 at the root.
    pub depth: u32,
    /// Its structure type, such as `P`, `H1` or `Figure`.
    pub kind: String,
    /// The page index, when on one page.
    pub page: Option<u32>,
}

/// Structure elements.
#[derive(Clone, Debug)]
pub struct Elements {
    /// Reference to the shared state.
    _shared: Arc<()>,
}

impl Elements {
    /// The next element, or `None` after the last.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn next(&self) -> Result<Option<Element>, PdfError> {
        Err(PdfError::new(ErrorCode::NotImplemented, "Elements::next"))
    }
}

/// An output intent (ISO 32000-2:2020, 14.11.5).
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct OutputIntent {
    /// Output condition identifier, such as `FOGRA39`.
    pub condition: String,
    /// A description of the condition.
    pub description: String,
    /// The ICC profile's stream, for [`Document::stream_data`](crate::document::Document::stream_data).
    pub profile: ObjectId,
}

/// Options for [`Element::set_alt_text`].
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct SetAltTextOptions {
    /// Empty removes it.
    pub alt_text: String,
}

/// Options for [`Document::set_language`](crate::document::Document::set_language).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct SetLanguageOptions {
    /// BCP 47; empty removes it.
    pub language: String,
}

/// Options for [`Document::set_output_intent`](crate::document::Document::set_output_intent).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct SetOutputIntentOptions {
    /// Output condition identifier, such as `FOGRA39`.
    pub condition: String,
    /// A description of the condition.
    pub description: String,
}

/// A standard a document can conform to.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum Standard {
    /// `ZUGFeRD` and Factur-X e-invoices.
    FacturX,
    /// ISO 32000, the version the file declares, PDF 2.0 included.
    #[default]
    Pdf,
    /// PDF/A-1a.
    PdfA1A,
    /// PDF/A-1b.
    PdfA1B,
    /// PDF/A-2a.
    PdfA2A,
    /// PDF/A-2b.
    PdfA2B,
    /// PDF/A-2u.
    PdfA2U,
    /// PDF/A-3a.
    PdfA3A,
    /// PDF/A-3b.
    PdfA3B,
    /// PDF/A-3u.
    PdfA3U,
    /// PDF/A-4.
    PdfA4,
    /// PDF/A-4e.
    PdfA4E,
    /// PDF/A-4f.
    PdfA4F,
    /// PDF/E-1.
    PdfE1,
    /// PDF/UA-1.
    PdfUa1,
    /// PDF/UA-2.
    PdfUa2,
    /// PDF/VT.
    PdfVt,
    /// PDF/X-1a.
    PdfX1A,
    /// PDF/X-3.
    PdfX3,
    /// PDF/X-4.
    PdfX4,
    /// PDF/X-6.
    PdfX6,
}

/// Options for [`Document::tag`](crate::document::Document::tag).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct TagOptions {
    /// Let an existing structure tree be replaced.
    pub allow_loss: bool,
    /// The document's language, BCP 47.
    pub language: String,
}

/// Options for [`Document::validate`](crate::document::Document::validate).
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct ValidateOptions {
    /// Also report images below this many pixels per inch, for preflight.
    pub min_resolution: Option<u32>,
    /// The standard to check against.
    pub standard: Standard,
}
