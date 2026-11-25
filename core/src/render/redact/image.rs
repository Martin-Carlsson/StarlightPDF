//! Images a mark meets (ISO 32000-2:2020, 8.9).

use crate::PdfError;
use crate::render::job::Job;

/// An image `XObject` a mark meets: not removed yet, so [`crate::ErrorCode::Unsupported`].
pub(super) fn painted(job: &Job<'_>) -> Result<Option<String>, PdfError> {
    Err(job.unsupported())
}

/// An inline image a mark meets: not removed yet, so [`crate::ErrorCode::Unsupported`].
pub(super) fn inline(job: &Job<'_>) -> Result<Option<String>, PdfError> {
    Err(job.unsupported())
}
