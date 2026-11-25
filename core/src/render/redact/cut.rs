//! Paths and shadings a mark meets (ISO 32000-2:2020, 8.5.3, 8.7.4.5).

use crate::PdfError;
use crate::render::job::Job;

/// A path whose box meets a mark: not cut yet, so [`crate::ErrorCode::Unsupported`].
pub(super) fn path(job: &Job<'_>) -> Result<Option<String>, PdfError> {
    Err(job.unsupported())
}

/// A shading painted where a mark may be: not clipped yet, so [`crate::ErrorCode::Unsupported`].
pub(super) fn shading(job: &Job<'_>) -> Result<Option<String>, PdfError> {
    Err(job.unsupported())
}
