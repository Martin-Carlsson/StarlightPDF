//! Comparison.

use alloc::sync::Arc;

use crate::{ErrorCode, PdfError};

/// How to compare two documents.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum CompareMethod {
    /// By objects and structure.
    Structure,
    /// By text.
    #[default]
    Text,
    /// By rendered pixels.
    Visual,
}

/// Options for [`Document::compare`](crate::document::Document::compare).
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct CompareOptions {
    /// How to compare.
    pub method: CompareMethod,
    /// For `Visual`, pixels per inch; `None`: 72.
    pub resolution: Option<u32>,
}

/// A difference.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Difference {
    /// Height in points.
    pub height: f64,
    /// The kind of difference.
    pub kind: DifferenceKind,
    /// The page index in the other document.
    pub other_page: Option<u32>,
    /// The page index in this document.
    pub page: Option<u32>,
    /// The text or object that differs.
    pub text: String,
    /// Width in points.
    pub width: f64,
    /// Left edge in points.
    pub x: f64,
    /// Top edge in points.
    pub y: f64,
}

/// The kind of a difference.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DifferenceKind {
    /// Changed.
    Changed,
    /// Only in this document.
    Deleted,
    /// Only in the other document.
    Inserted,
}

/// Differences.
#[derive(Clone, Debug)]
pub struct Differences {
    /// Reference to the shared state.
    _shared: Arc<()>,
}

impl Differences {
    /// The next difference, or `None` after the last.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::CapabilityMissing`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn next(&self) -> Result<Option<Difference>, PdfError> {
        Err(PdfError::new(
            ErrorCode::NotImplemented,
            "Differences::next",
        ))
    }
}
