//! What structure says of content a mark touches (ISO 32000-2:2020, 14.6, 14.7, 14.9.3 to
//! 14.9.5).

use crate::PdfError;
use crate::render::job::Job;

/// An annotation removed that structure refers to: not handled yet, so
/// [`crate::ErrorCode::Unsupported`].
pub(super) fn annotation(job: &Job<'_>) -> Result<(), PdfError> {
    Err(job.unsupported())
}

/// Content a mark touches within marked content with `ActualText`, `Alt`, `E` or `MCID`: not
/// handled yet, so [`crate::ErrorCode::Unsupported`].
pub(super) fn touched(job: &Job<'_>) -> Result<(), PdfError> {
    Err(job.unsupported())
}
